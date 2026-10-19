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


/// L'appareil : identifiant du modèle (« iPhone17,1 », « Mac15,9 »), mémoire totale,
/// mémoire que le système laisse encore à Lumen (iPhone et iPad), et sa chaleur.
#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct DeviceInfo {
    pub machine: String,
    /// octets
    pub memory: u64,
    /// octets ; 0 si inconnue (Mac, PC)
    pub available: u64,
    /// "nominal", "fair", "serious", "critical" (vide : inconnue)
    pub thermal: String,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use std::ffi::{c_char, c_void, CStr, CString};

    extern "C" {
        fn sysctlbyname(name: *const c_char, oldp: *mut c_void, oldlenp: *mut usize, newp: *mut c_void, newlen: usize) -> i32;
    }

    /// Valeur brute d'une clé de `sysctl`.
    fn raw(name: &str) -> Option<Vec<u8>> {
        let n = CString::new(name).ok()?;
        let mut len: usize = 0;
        // SAFETY : la taille d'abord, puis la valeur dans un tampon de cette taille
        unsafe {
            if sysctlbyname(n.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            if sysctlbyname(n.as_ptr(), buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
            buf.truncate(len);
            Some(buf)
        }
    }

    pub fn string(name: &str) -> String {
        raw(name).and_then(|b| CStr::from_bytes_until_nul(&b).ok().map(|c| c.to_string_lossy().into_owned())).unwrap_or_default()
    }

    pub fn u64_of(name: &str) -> u64 {
        raw(name).filter(|b| b.len() == 8).map(|b| u64::from_ne_bytes(b[..8].try_into().unwrap())).unwrap_or(0)
    }

    pub fn thermal() -> String {
        use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
        match NSProcessInfo::processInfo().thermalState() {
            NSProcessInfoThermalState::Nominal => "nominal",
            NSProcessInfoThermalState::Fair => "fair",
            NSProcessInfoThermalState::Serious => "serious",
            NSProcessInfoThermalState::Critical => "critical",
            _ => "",
        }
        .into()
    }
}

/// iPhone et iPad : l'appareil chauffe (état « sérieux » ou « critique » d'iOS). Les
/// calculs prennent alors moitié moins de cœurs : plus lents, mais l'iPhone ne
/// bride pas tout l'appareil. Toujours faux sur Mac et PC.
pub fn hot() -> bool {
    #[cfg(target_os = "ios")]
    {
        matches!(apple::thermal().as_str(), "serious" | "critical")
    }
    #[cfg(not(target_os = "ios"))]
    {
        false
    }
}

pub fn device_info() -> DeviceInfo {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        // iPhone et iPad : « hw.machine » (« iPhone17,1 ») ; Mac : « hw.model » (« Mac15,9 »)
        let machine = if cfg!(target_os = "ios") { apple::string("hw.machine") } else { apple::string("hw.model") };
        #[cfg(target_os = "ios")]
        let available = crate::ios::available_memory();
        #[cfg(not(target_os = "ios"))]
        let available = 0;
        DeviceInfo { machine, memory: apple::u64_of("hw.memsize"), available, thermal: apple::thermal() }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    DeviceInfo::default()
}
