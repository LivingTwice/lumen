//! Moteur sherpa-onnx (outils officiels et onnxruntime), partagé par la voix
//! naturelle (Supertonic 3), le texte des transcriptions (Parakeet) et la
//! reconnaissance des interlocuteurs (speakers.rs).
//!
//! Téléchargé à part (~20 Mo), comme les modèles : l'app ne grossit pas, et le
//! « hardened runtime » de macOS laisse lancer un exécutable téléchargé, pas
//! charger une bibliothèque. Une seule archive pour tous les outils.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::models::{self, DownloadEvent};

const VERSION: &str = "1.13.8";
/// Archive du moteur pour ce système : macOS (puce Apple), Windows ou Linux
/// (x64, bibliothèque C++ intégrée : rien à installer à côté).
const FLAVOR: &str = if cfg!(windows) {
    "win-x64-shared-MT-Release"
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "linux-x64-shared"
} else {
    "osx-arm64-shared"
};
/// Taille de cette archive, pour la progression (iPhone et iPad : rien à télécharger,
/// la bibliothèque est dans l'app, sherpa_ios.rs).
pub const SIZE: u64 = if cfg!(target_os = "ios") {
    0
} else if cfg!(windows) {
    24_805_859
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    28_156_791
} else {
    20_314_448
};
/// Bibliothèques que chargent les outils (sous Windows, à côté d'eux) ; la
/// seconde, facultative, peut manquer.
const LIBS: &[&str] = if cfg!(windows) {
    &["bin/onnxruntime.dll", "bin/onnxruntime_providers_shared.dll"]
} else if cfg!(target_os = "linux") {
    &["lib/libonnxruntime.so"]
} else {
    &["lib/libonnxruntime.dylib"]
};

/// Les outils du moteur dont Lumen se sert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// synthèse vocale (la voix naturelle)
    Tts,
    /// reconnaissance de la parole (Parakeet)
    Asr,
    /// qui parle quand (les interlocuteurs d'une conversation)
    Speakers,
}

const TOOLS: [Tool; 3] = [Tool::Tts, Tool::Asr, Tool::Speakers];

impl Tool {
    /// Chemin de l'outil dans l'archive.
    fn file(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::Tts, false) => "bin/sherpa-onnx-offline-tts",
            (Tool::Tts, true) => "bin/sherpa-onnx-offline-tts.exe",
            (Tool::Asr, false) => "bin/sherpa-onnx-offline",
            (Tool::Asr, true) => "bin/sherpa-onnx-offline.exe",
            (Tool::Speakers, false) => "bin/sherpa-onnx-offline-speaker-diarization",
            (Tool::Speakers, true) => "bin/sherpa-onnx-offline-speaker-diarization.exe",
        }
    }
}

fn name() -> String {
    format!("sherpa-onnx-v{VERSION}-{FLAVOR}")
}

pub fn dir(data_dir: &Path) -> PathBuf {
    crate::tools::tools_dir(data_dir).join(name())
}

/// Un fichier du moteur (« bin/… », « lib/… »).
fn file(data_dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(dir(data_dir), |p, part| p.join(part))
}

/// L'outil à lancer.
pub fn bin(data_dir: &Path, tool: Tool) -> PathBuf {
    file(data_dir, tool.file())
}

/// La commande qui lance un outil du moteur (la voix, Parakeet, les interlocuteurs).
/// iPhone et iPad lancent la bibliothèque dans l'app (sherpa_ios.rs), jamais un outil.
#[cfg(not(target_os = "ios"))]
pub fn command(data_dir: &Path, tool: Tool) -> tokio::process::Command {
    #[cfg(target_os = "linux")]
    {
        loader_command(data_dir, tool).unwrap_or_else(|| crate::proc::tokio_command(bin(data_dir, tool)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        crate::proc::tokio_command(bin(data_dir, tool))
    }
}

/// Linux : le moteur téléchargé attend l'éditeur de liens du système
/// (/lib64/ld-linux…), qui n'existe pas sous NixOS. On le lance avec celui de
/// Lumen et les bibliothèques déjà chargées par l'app (la libc, libstdc++…) :
/// le moteur trouve ainsi tout ce qu'il lui faut, sur toute distribution.
/// (Retourne None si l'ELF de Lumen ne se laisse pas lire : lancement direct.)
#[cfg(target_os = "linux")]
fn loader_command(data_dir: &Path, tool: Tool) -> Option<tokio::process::Command> {
    let exe = std::fs::read(std::fs::read_link("/proc/self/exe").ok()?).ok()?;
    let interp = elf_interpreter(&exe)?;
    // les bibliothèques du moteur : la libc et ses sœurs viennent du dossier de
    // l'éditeur de liens lui-même (une seule libc, celle qui l'accompagne —
    // /proc/self/maps peut contenir une seconde libc chargée en privé par une
    // bibliothèque de l'app, qui mélangée à l'autre écrase la pile du moteur) ;
    // libstdc++ et libgcc_s, de la première chargée par l'app.
    let mut dirs: Vec<String> = vec![
        dir(data_dir).join("lib").display().to_string(),
        Path::new(&interp).parent().map(|d| d.display().to_string())?,
    ];
    let mut have_cxx = false;
    let mut have_gcc = false;
    for line in std::fs::read_to_string("/proc/self/maps").ok()?.lines() {
        let Some((_, path)) = line.rsplit_once(' ') else { continue };
        let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) else { continue };
        let Some(dir) = Path::new(path).parent().map(|d| d.display().to_string()) else { continue };
        if !have_cxx && name.starts_with("libstdc++.so") {
            have_cxx = true;
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        } else if !have_gcc && name.starts_with("libgcc_s.so") {
            have_gcc = true;
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if have_cxx && have_gcc {
            break;
        }
    }
    let mut cmd = crate::proc::tokio_command(&interp);
    cmd.arg("--library-path").arg(dirs.join(":")).arg(bin(data_dir, tool));
    Some(cmd)
}

/// Lit l'éditeur de liens d'un exécutable ELF64 (segment PT_INTERP) : c'est
/// lui qui charge le binaire et ses bibliothèques. Sous Linux sans paquet
/// (NixOS), le moteur téléchargé attend celui du système, absent : Lumen lui
/// prête le sien.
#[cfg(target_os = "linux")]
fn elf_interpreter(exe: &[u8]) -> Option<String> {
    if exe.first()? != &0x7f || exe.get(1)? != &b'E' {
        return None;
    }
    let u16at = |i: usize| -> Option<u16> {
        let b = exe.get(i..i + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    };
    let u64at = |i: usize| -> Option<u64> {
        let b = exe.get(i..i + 8)?;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    };
    let phoff = u64at(0x20)? as usize;
    let phsize = u16at(0x36)? as usize;
    let phnum = u16at(0x38)? as usize;
    for i in 0..phnum {
        let p = phoff + i * phsize;
        let kind = u32::from_le_bytes([*exe.get(p)?, *exe.get(p + 1)?, *exe.get(p + 2)?, *exe.get(p + 3)?]);
        if kind == 3 {
            let off = u64at(p + 8)? as usize;
            let end = exe[off..].iter().position(|b| *b == 0)? + off;
            return std::str::from_utf8(&exe[off..end]).ok().map(String::from);
        }
    }
    None
}

pub fn ready(data_dir: &Path, tool: Tool) -> bool {
    // iPhone et iPad : le moteur est dans l'app
    cfg!(target_os = "ios") || (bin(data_dir, tool).exists() && file(data_dir, LIBS[0]).exists())
}

/// Supprime le moteur, quand plus aucun modèle ne s'en sert.
pub fn remove(data_dir: &Path) {
    // iPhone et iPad : rien à supprimer, mais la voix chargée se libère
    #[cfg(target_os = "ios")]
    crate::sherpa_ios::release_voice();
    let _ = std::fs::remove_dir_all(dir(data_dir));
}

/// La voix et Parakeet peuvent demander le moteur en même temps : un seul téléchargement.
static FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Télécharge le moteur si l'outil manque (un moteur installé avant l'arrivée d'un
/// outil le reprend), et n'en garde que les outils dont Lumen se sert et
/// leurs bibliothèques (la mise en page bin/ et lib/ est celle qu'attendent les outils).
pub async fn ensure(data_dir: &Path, tool: Tool, cancel: Arc<AtomicBool>, mut on_progress: impl FnMut(u64, u64)) -> Result<()> {
    if ready(data_dir, tool) {
        return Ok(());
    }
    let _turn = FETCH.lock().await;
    if ready(data_dir, tool) {
        return Ok(());
    }
    if !cfg!(any(all(target_os = "macos", target_arch = "aarch64"), all(windows, target_arch = "x86_64"), all(target_os = "linux", target_arch = "x86_64"))) {
        return Err(anyhow!(match tool {
            Tool::Tts => crate::i18n::t("La voix naturelle n'est pas encore disponible sur cet ordinateur.", "The natural voice isn't available on this computer yet."),
            Tool::Asr => crate::i18n::t("Parakeet n'est pas encore disponible sur cet appareil.", "Parakeet isn't available on this device yet."),
            Tool::Speakers => crate::i18n::t("La reconnaissance des interlocuteurs n'est pas encore disponible sur cet appareil.", "Speaker recognition isn't available on this device yet."),
        }));
    }
    let tools = crate::tools::tools_dir(data_dir);
    tokio::fs::create_dir_all(&tools).await?;
    let name = name();
    let url = format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/v{VERSION}/{name}.tar.bz2");
    let part = tools.join(format!("{name}.part"));
    let mut forward = |e: DownloadEvent| {
        if let DownloadEvent::Progress { received, total, .. } = e {
            on_progress(received, total);
        }
    };
    models::fetch_resumable(&url, &part, 0, SIZE, cancel, &mut forward).await?;
    let rels: Vec<&str> = TOOLS.iter().map(|t| t.file()).chain(LIBS.iter().copied()).collect();
    let wanted: Vec<String> = rels.iter().map(|f| format!("{name}/{f}")).collect();
    // décompressé à part, puis mis en place fichier par fichier : un outil
    // présent est toujours un outil complet (l'autre continue de servir)
    let tmp = tools.join(".sherpa-extraction");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    // macOS : le tar du système ; ailleurs, décompressé ici (le tar de Windows 10
    // ne lit pas toujours le bzip2, et NixOS n'a pas de tar système pour l'app)
    #[cfg(target_os = "macos")]
    let ok = tokio::process::Command::new("tar").arg("-xjf").arg(&part).arg("-C").arg(&tmp).args(&wanted).output().await?.status.success();
    #[cfg(not(target_os = "macos"))]
    let ok = {
        let (archive, dest) = (part.clone(), tmp.clone());
        tokio::task::spawn_blocking(move || models::unpack_tar(&archive, &dest, Some(&wanted))).await?.is_ok()
    };
    let _ = tokio::fs::remove_file(&part).await;
    if ok {
        let from = tmp.join(&name);
        for rel in &rels {
            let src = rel.split('/').fold(from.clone(), |p, part| p.join(part));
            let dest = file(data_dir, rel);
            // bibliothèque facultative absente ; ou fichier déjà là, de la même version
            // (Windows refuse de remplacer une bibliothèque que la voix est en train d'employer)
            if !src.exists() || dest.exists() {
                continue;
            }
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::rename(&src, &dest).await?;
            // les outils lisent leur ligne de commande (texte à dire, chemins des fichiers) en UTF-8
            #[cfg(windows)]
            if dest.extension().is_some_and(|e| e == "exe") {
                let _ = crate::win::utf8_manifest(&dest);
            }
        }
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    if !ok || !ready(data_dir, tool) {
        return Err(anyhow!(match tool {
            Tool::Tts => crate::i18n::t("le moteur de voix n'a pas pu être installé, réessayez", "the voice engine couldn't be installed, try again"),
            Tool::Asr => crate::i18n::t("le moteur de Parakeet n'a pas pu être installé, réessayez", "Parakeet's engine couldn't be installed, try again"),
            Tool::Speakers => crate::i18n::t("le moteur qui reconnaît les interlocuteurs n'a pas pu être installé, réessayez", "the speaker recognition engine couldn't be installed, try again"),
        }));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Un ELF64 minimal avec un segment PT_INTERP : l'en-tête (64 octets),
    /// un programme d'en-tête (56 octets) de type 3, et le chemin.
    fn elf_with_interpreter(path: &str) -> Vec<u8> {
        let mut e = vec![0u8; 64];
        e[0] = 0x7f;
        e[1] = b'E';
        e[2] = b'L';
        e[3] = b'F';
        e[4] = 2; // ELF64
        e[5] = 1; // little endian
        e[0x20..0x28].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        e[0x36..0x38].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        e[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let mut p = vec![0u8; 56];
        p[0..4].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        let path_off = 64 + 56;
        p[8..16].copy_from_slice(&(path_off as u64).to_le_bytes()); // p_offset
        p[32..40].copy_from_slice(&(path.len() as u64 + 1).to_le_bytes()); // p_filesz
        let mut out = e;
        out.extend(p);
        out.extend_from_slice(path.as_bytes());
        out.push(0);
        out
    }

    #[test]
    fn elf_interpreter_is_found() {
        let elf = elf_with_interpreter("/nix/store/…-glibc/lib/ld-linux-x86-64.so.2");
        assert_eq!(elf_interpreter(&elf).as_deref(), Some("/nix/store/…-glibc/lib/ld-linux-x86-64.so.2"));
    }

    #[test]
    fn elf_interpreter_rejects_non_elf() {
        assert!(elf_interpreter(b"pas un elf").is_none());
        assert!(elf_interpreter(b"\x7fEL\x00").is_none()); // magie incomplète
        assert!(elf_interpreter(&[]).is_none());
    }

    /// Chaque outil du moteur passe par l'éditeur de liens de Lumen (ici, celui
    /// du programme de test), jamais lancé seul : ses bibliothèques viennent
    /// avec lui (le moteur attend celles d'un système que NixOS n'a pas).
    #[test]
    fn every_tool_is_launched_by_lumens_loader() {
        let data = Path::new("/partout/lumen-sans-moteur");
        let exe = std::fs::read(std::fs::read_link("/proc/self/exe").unwrap()).unwrap();
        let Some(interp) = elf_interpreter(&exe) else {
            eprintln!("sans éditeur de liens : essai sans objet");
            return;
        };
        let text = |p: PathBuf| p.display().to_string();
        // les bibliothèques données à l'outil : celles du moteur d'abord, la libc de l'éditeur de liens
        let engine_libs = text(dir(data).join("lib"));
        let system_libs = text(Path::new(&interp).parent().unwrap().to_path_buf());
        for tool in TOOLS {
            let launched = command(data, tool);
            let cmd = launched.as_std();
            assert_eq!(cmd.get_program().to_str(), Some(interp.as_str()), "{tool:?}");
            let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            assert_eq!(args.first().map(String::as_str), Some("--library-path"), "{tool:?}");
            let libs: Vec<&str> = args[1].split(':').collect();
            assert_eq!(libs.first().copied(), Some(engine_libs.as_str()), "{tool:?}");
            assert!(libs.contains(&system_libs.as_str()), "{tool:?} : {}", args[1]);
            let tool_path = text(bin(data, tool));
            assert_eq!(args.last().map(String::as_str), Some(tool_path.as_str()), "{tool:?}");
        }
    }
}
