//! Programmes lancés par Lumen (Whisper, yt-dlp, la voix…). Sous Windows, une
//! app sans console qui lance un programme en ligne de commande ferait surgir
//! une fenêtre noire à chaque fois : on le lance sans fenêtre.

use std::ffi::OsStr;

/// Pas de fenêtre de console pour le programme lancé (CREATE_NO_WINDOW).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `std::process::Command`, sans fenêtre de console sous Windows.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `tokio::process::Command`, sans fenêtre de console sous Windows.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Paquet de l'app à rouvrir en quittant (relance après une mise à jour, Mac).
#[cfg(target_os = "macos")]
static RELAUNCH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Le paquet `.app` qui contient l'exécutable en cours (aucun en développement).
#[cfg(target_os = "macos")]
fn app_bundle() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let ok = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.extension()? == "app";
    ok.then(|| bundle.to_path_buf())
}

/// Relance Lumen (« Redémarrer » après une mise à jour). Sur Mac, Tauri relancerait
/// l'exécutable lui-même : macOS (depuis la version 14) ne laisse pas une app lancée
/// ainsi passer au premier plan, et Lumen s'ouvrait derrière les autres fenêtres.
/// On quitte normalement (dernière sauvegarde comprise) et `relaunch_on_exit` rouvre
/// le paquet par le Finder (LaunchServices), comme un double-clic.
pub fn relaunch(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle() {
        let _ = RELAUNCH.set(bundle);
        app.exit(0);
        return;
    }
    app.request_restart();
}

/// À la sortie, après la dernière sauvegarde : rouvre Lumen si une relance est demandée.
pub fn relaunch_on_exit() {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = RELAUNCH.get() {
        use std::time::{Duration, Instant};
        // -n : une nouvelle instance, celle-ci n'est pas encore partie. On attend que
        // `open` ait lancé Lumen : c'est parce que Lumen est encore l'app active que
        // macOS met la nouvelle au premier plan.
        let opened = command("/usr/bin/open").arg("-n").arg(bundle).spawn().is_ok_and(|mut child| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => return status.success(),
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                    // toujours en cours : la demande est partie, Lumen va s'ouvrir
                    Ok(None) => return true,
                    Err(_) => return false,
                }
            }
        });
        // dernier recours : comme Tauri, l'exécutable lui-même (Lumen s'ouvre, mais derrière)
        if !opened {
            if let Ok(exe) = std::env::current_exe() {
                let _ = command(exe).spawn();
            }
        }
    }
}
