//! Outils externes gérés par Lumen : yt-dlp (téléchargement YouTube) et un
//! petit moteur JavaScript (QuickJS) dont yt-dlp a besoin pour YouTube.
//! On utilise ceux du système s'ils existent, sinon Lumen les télécharge
//! une fois dans son dossier de données et les tient à jour.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

pub fn tools_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Emplacements usuels des outils installés par Homebrew ou à la main
/// (une app lancée depuis le Finder n'hérite pas du PATH du Terminal).
fn system_candidates(name: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        return windows_candidates(name);
    }
    let h = home();
    vec![
        PathBuf::from(format!("/opt/homebrew/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
        PathBuf::from(format!("{h}/.homebrew/bin/{name}")),
        PathBuf::from(format!("{h}/.local/bin/{name}")),
        PathBuf::from(format!("{h}/.deno/bin/{name}")),
        PathBuf::from(format!("/usr/bin/{name}")),
    ]
}

/// Windows : outils installés par winget, Scoop, Chocolatey, l'installateur de
/// Node ou celui de Deno, puis les dossiers du PATH (une app Windows en hérite).
fn windows_candidates(name: &str) -> Vec<PathBuf> {
    let exe = format!("{name}.exe");
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut out = Vec::new();
    if let Some(p) = env("LOCALAPPDATA") {
        out.push(p.join("Microsoft").join("WinGet").join("Links").join(&exe));
    }
    if let Some(h) = env("USERPROFILE") {
        out.push(h.join("scoop").join("shims").join(&exe));
        out.push(h.join(".deno").join("bin").join(&exe));
    }
    if let Some(p) = env("ProgramData") {
        out.push(p.join("chocolatey").join("bin").join(&exe));
    }
    if let Some(p) = env("ProgramFiles") {
        out.push(p.join("nodejs").join(&exe));
    }
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|d| d.join(&exe)));
    }
    out
}

fn ytdlp_asset() -> &'static str {
    if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp_linux"
    }
}

fn qjs_asset() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "qjs-darwin-arm64"
    } else if cfg!(target_os = "macos") {
        "qjs-darwin-x86_64"
    } else if cfg!(windows) {
        "qjs-windows-x86_64.exe"
    } else {
        "qjs-linux-x86_64"
    }
}

fn managed_ytdlp(data_dir: &Path) -> PathBuf {
    tools_dir(data_dir).join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" })
}

fn managed_qjs(data_dir: &Path) -> PathBuf {
    tools_dir(data_dir).join(if cfg!(windows) { "qjs.exe" } else { "qjs" })
}

/// yt-dlp disponible (système ou géré par Lumen).
pub fn find_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    let m = managed_ytdlp(data_dir);
    if m.exists() {
        return Some(m);
    }
    system_candidates("yt-dlp").into_iter().find(|p| p.exists())
}

/// Version majeure de Node (yt-dlp exige Node 22 ou plus).
fn node_major(path: &Path) -> Option<u32> {
    let out = crate::proc::command(path).arg("--version").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout);
    v.trim().trim_start_matches('v').split('.').next()?.parse().ok()
}

/// Argument --js-runtimes à passer à yt-dlp, s'il existe un moteur utilisable.
pub fn js_runtime_arg(data_dir: &Path) -> Option<String> {
    if let Some(d) = system_candidates("deno").into_iter().find(|p| p.exists()) {
        return Some(format!("deno:{}", d.display()));
    }
    for n in system_candidates("node") {
        if n.exists() && node_major(&n).unwrap_or(0) >= 22 {
            return Some(format!("node:{}", n.display()));
        }
    }
    let q = managed_qjs(data_dir);
    if q.exists() {
        return Some(format!("quickjs:{}", q.display()));
    }
    None
}

async fn fetch(url: &str, dest: &Path, mut on_progress: impl FnMut(u64, u64)) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("Lumen/0.1")
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(crate::tr!("téléchargement refusé ({})", "download refused ({})", resp.status())));
    }
    let total = resp.content_length().unwrap_or(0);
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut got = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        got += chunk.len() as u64;
        on_progress(got, total);
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Installe (si besoin) yt-dlp et le moteur JavaScript. Renvoie le chemin de yt-dlp.
pub async fn ensure_youtube_tools(data_dir: &Path, mut on_progress: impl FnMut(f64)) -> Result<PathBuf> {
    tokio::fs::create_dir_all(tools_dir(data_dir)).await?;
    let need_ytdlp = find_ytdlp(data_dir).is_none();
    let need_js = js_runtime_arg(data_dir).is_none();
    if need_js {
        let url = format!("https://github.com/quickjs-ng/quickjs/releases/latest/download/{}", qjs_asset());
        fetch(&url, &managed_qjs(data_dir), |g, t| {
            if t > 0 {
                on_progress((g as f64 / t as f64) * if need_ytdlp { 8.0 } else { 100.0 });
            }
        })
        .await
        .map_err(|e| anyhow!(crate::tr!("moteur JavaScript : {e}", "JavaScript engine: {e}")))?;
    }
    if need_ytdlp {
        let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}", ytdlp_asset());
        let base = if need_js { 8.0 } else { 0.0 };
        fetch(&url, &managed_ytdlp(data_dir), |g, t| {
            if t > 0 {
                on_progress(base + (g as f64 / t as f64) * (100.0 - base));
            }
        })
        .await
        .map_err(|e| anyhow!("yt-dlp : {e}"))?;
    } else {
        maybe_self_update(data_dir).await;
    }
    find_ytdlp(data_dir).ok_or_else(|| anyhow!(crate::i18n::t("yt-dlp est introuvable", "yt-dlp can't be found")))
}

/// YouTube change souvent : on met à jour la copie gérée de yt-dlp au plus
/// une fois par semaine (sans bloquer en cas d'échec).
async fn maybe_self_update(data_dir: &Path) {
    let m = managed_ytdlp(data_dir);
    if !m.exists() {
        return;
    }
    let stamp = tools_dir(data_dir).join(".ytdlp-updated");
    let fresh = std::fs::metadata(&stamp)
        .and_then(|md| md.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d < Duration::from_secs(7 * 24 * 3600))
        .unwrap_or(false);
    if fresh {
        return;
    }
    let _ = tokio::time::timeout(Duration::from_secs(90), crate::proc::tokio_command(&m).arg("-U").output()).await;
    let _ = std::fs::write(stamp, b"");
}

pub fn status(data_dir: &Path) -> (bool, bool) {
    (find_ytdlp(data_dir).is_some(), js_runtime_arg(data_dir).is_some())
}
