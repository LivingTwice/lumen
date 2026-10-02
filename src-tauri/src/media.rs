//! Import audio et vidéo : copie dans la bibliothèque, transcription locale
//! par le processus `lumen-whisper`, construction du texte et des horodatages.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Deserialize, Debug)]
pub struct WWord {
    pub w: String,
    pub t0: f64,
    pub t1: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ImportEvent {
    Stage { stage: String },
    Progress { value: f64 },
}

pub fn media_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("media")
}

pub fn sidecar_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| anyhow!("dossier de l'application introuvable"))?;
    let name = if cfg!(windows) { "lumen-whisper.exe" } else { "lumen-whisper" };
    let p = dir.join(name);
    if p.exists() {
        return Ok(p);
    }
    Err(anyhow!("le composant de transcription est absent de l'application"))
}

/// Copie le fichier dans la bibliothèque de Lumen et renvoie son nouveau chemin.
pub fn store_media(data_dir: &Path, src: &Path) -> Result<PathBuf> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin").to_lowercase();
    let name = format!("{}.{}", chrono::Utc::now().format("%Y%m%d-%H%M%S%3f"), ext);
    let dst = dir.join(name);
    std::fs::copy(src, &dst)?;
    Ok(dst)
}

pub async fn transcribe(
    model: &Path,
    media: &Path,
    lang: &str,
    mut on_event: impl FnMut(ImportEvent),
) -> Result<Vec<WWord>> {
    let bin = sidecar_path()?;
    let mut child = Command::new(bin)
        .arg(model)
        .arg(media)
        .arg(lang)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut result: Option<Vec<WWord>> = None;
    let mut error: Option<String> = None;
    while let Some(line) = lines.next_line().await? {
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v["type"].as_str() {
            Some("stage") => on_event(ImportEvent::Stage { stage: v["stage"].as_str().unwrap_or("").to_string() }),
            Some("progress") => on_event(ImportEvent::Progress { value: v["value"].as_f64().unwrap_or(0.0) }),
            Some("done") => {
                result = Some(serde_json::from_value(v["words"].clone()).unwrap_or_default());
            }
            Some("error") => error = Some(v["message"].as_str().unwrap_or("erreur inconnue").to_string()),
            _ => {}
        }
    }
    let _ = child.wait().await;
    if let Some(e) = error {
        return Err(anyhow!(e));
    }
    result.ok_or_else(|| anyhow!("la transcription s'est arrêtée sans résultat"))
}

/// Assemble le texte et les horodatages par mot : [[début, fin, t0, t1], …]
/// (positions en unités UTF-16 dans le texte final).
pub fn build_transcript(words: &[WWord]) -> (String, String) {
    let mut text = String::new();
    let mut len16 = 0usize;
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let mut prev_end = 0.0f64;
    for (i, w) in words.iter().enumerate() {
        let raw = w.w.replace('\n', " ");
        let word = raw.trim();
        if word.is_empty() {
            continue;
        }
        let glue_left = raw.starts_with(' ') || i == 0;
        let ends_sentence = text.trim_end().ends_with(['.', '!', '?', '…']);
        if !text.is_empty() {
            if w.t0 - prev_end > 1.6 && ends_sentence {
                text.push_str("\n\n");
                len16 += 2;
            } else if glue_left {
                text.push(' ');
                len16 += 1;
            }
        }
        let s = len16;
        text.push_str(word);
        len16 += word.encode_utf16().count();
        timings.push([s as f64, len16 as f64, (w.t0 * 100.0).round() / 100.0, (w.t1 * 100.0).round() / 100.0]);
        prev_end = w.t1;
    }
    (text, serde_json::to_string(&timings).unwrap_or_else(|_| "[]".into()))
}

/// Lance yt-dlp et renvoie les lignes de sortie utiles. La progression
/// (« LUMEN 42.0% ») est transmise au fur et à mesure.
async fn run_ytdlp(
    data_dir: &Path,
    ytdlp: &Path,
    args: &[String],
    cookies_browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<Vec<String>> {
    let mut cmd = Command::new(ytdlp);
    cmd.args(["--no-playlist", "--newline", "--progress", "--no-colors"]);
    cmd.args(["--progress-template", "download:LUMEN %(progress._percent_str)s"]);
    if let Some(js) = crate::tools::js_runtime_arg(data_dir) {
        cmd.args(["--js-runtimes", &js]);
    }
    if let Some(b) = cookies_browser {
        cmd.args(["--cookies-from-browser", b]);
    }
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut last = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            last.push(l);
            if last.len() > 30 {
                last.remove(0);
            }
        }
        last
    });
    let mut out = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(rest) = line.trim().strip_prefix("LUMEN") {
            if let Ok(v) = rest.trim().trim_end_matches('%').trim().parse::<f64>() {
                on_progress(v);
            }
        } else if !line.trim().is_empty() {
            out.push(line.trim().to_string());
        }
    }
    let status = child.wait().await?;
    let errs = err_task.await.unwrap_or_default();
    if !status.success() {
        let msg = errs.iter().rev().find(|l| l.contains("ERROR")).cloned().unwrap_or_else(|| errs.last().cloned().unwrap_or_default());
        return Err(anyhow!("{}", msg.trim_start_matches("ERROR: ").trim()));
    }
    Ok(out)
}

fn needs_cookies(e: &anyhow::Error) -> bool {
    let m = e.to_string().to_lowercase();
    m.contains("sign in") || m.contains("cookies") || m.contains("not a bot") || m.contains("age")
}

async fn run_with_fallback(
    data_dir: &Path,
    ytdlp: &Path,
    args: &[String],
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<Vec<String>> {
    match run_ytdlp(data_dir, ytdlp, args, None, on_progress).await {
        Ok(v) => Ok(v),
        Err(e) if needs_cookies(&e) => match browser {
            Some(b) => run_ytdlp(data_dir, ytdlp, args, Some(b), on_progress).await,
            None => Err(anyhow!(
                "YouTube demande une vérification pour cette vidéo. Choisissez votre navigateur dans Réglages › YouTube pour que Lumen utilise votre session. ({e})"
            )),
        },
        Err(e) => Err(e),
    }
}

/// Télécharge la piste audio d'une vidéo (YouTube et la plupart des sites
/// vidéo). Renvoie le fichier et le titre.
pub async fn yt_audio(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    stem: &str,
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<(PathBuf, String)> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let template = dir.join(format!("{stem}.audio.%(ext)s"));
    let args: Vec<String> = vec![
        "-f".into(),
        "ba[ext=m4a]/ba[acodec^=mp4a]/ba[ext=mp3]/b[ext=mp4]".into(),
        "--no-simulate".into(),
        "--print".into(),
        "TITLE:%(title)s".into(),
        "--print".into(),
        "after_move:FILE:%(filepath)s".into(),
        "-o".into(),
        template.display().to_string(),
        url.into(),
    ];
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, on_progress).await?;
    let title = out.iter().find_map(|l| l.strip_prefix("TITLE:")).unwrap_or("Vidéo").to_string();
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!("fichier audio introuvable"))?;
    Ok((PathBuf::from(file), title))
}

/// Télécharge l'image de la vidéo (sans le son), en H.264 jusqu'en 1080p :
/// lisible partout sur Mac, synchronisée avec la piste audio.
pub async fn yt_video(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    stem: &str,
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<PathBuf> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let template = dir.join(format!("{stem}.video.%(ext)s"));
    let args: Vec<String> = vec![
        "-f".into(),
        "bv*[vcodec^=avc1][height<=1080][ext=mp4]/bv*[vcodec^=avc1][height<=1080]/bv*[ext=mp4][height<=1080]/b[ext=mp4]".into(),
        "--no-simulate".into(),
        "--print".into(),
        "after_move:FILE:%(filepath)s".into(),
        "-o".into(),
        template.display().to_string(),
        url.into(),
    ];
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, on_progress).await?;
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!("fichier vidéo introuvable"))?;
    Ok(PathBuf::from(file))
}

pub fn new_stem() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S%3f").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_offsets() {
        let words = vec![
            WWord { w: " Hello".into(), t0: 0.0, t1: 0.4 },
            WWord { w: " world.".into(), t0: 0.4, t1: 0.9 },
            WWord { w: " Next".into(), t0: 3.0, t1: 3.4 },
        ];
        let (text, timings) = build_transcript(&words);
        assert_eq!(text, "Hello world.\n\nNext");
        let t: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        assert_eq!(t[2][0] as usize, 14);
    }
}
