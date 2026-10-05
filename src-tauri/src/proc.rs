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
