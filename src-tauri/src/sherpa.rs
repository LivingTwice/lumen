//! Moteur sherpa-onnx (outils officiels et onnxruntime), partagé par la voix
//! naturelle (Supertonic 3) et le texte des transcriptions (Parakeet).
//!
//! Téléchargé à part (~20 Mo), comme les modèles : l'app ne grossit pas, et le
//! « hardened runtime » de macOS laisse lancer un exécutable téléchargé, pas
//! charger une bibliothèque. Une seule archive pour les deux outils.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::models::{self, DownloadEvent};

const VERSION: &str = "1.13.8";
/// Archive du moteur pour ce système : macOS (puce Apple) ou Windows (x64,
/// bibliothèque C++ intégrée : rien à installer à côté).
const FLAVOR: &str = if cfg!(windows) { "win-x64-shared-MT-Release" } else { "osx-arm64-shared" };
/// Taille de cette archive, pour la progression (iPhone et iPad : rien à télécharger,
/// la bibliothèque est dans l'app, sherpa_ios.rs).
pub const SIZE: u64 = if cfg!(target_os = "ios") {
    0
} else if cfg!(windows) {
    24_805_859
} else {
    20_314_448
};
/// Bibliothèques que chargent les outils (sous Windows, à côté d'eux) ; la
/// seconde, facultative, peut manquer.
const LIBS: &[&str] = if cfg!(windows) { &["bin/onnxruntime.dll", "bin/onnxruntime_providers_shared.dll"] } else { &["lib/libonnxruntime.dylib"] };

/// Les outils du moteur dont Lumen se sert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// synthèse vocale (la voix naturelle)
    Tts,
    /// reconnaissance de la parole (Parakeet)
    Asr,
}

const TOOLS: [Tool; 2] = [Tool::Tts, Tool::Asr];

impl Tool {
    /// Chemin de l'outil dans l'archive.
    fn file(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::Tts, false) => "bin/sherpa-onnx-offline-tts",
            (Tool::Tts, true) => "bin/sherpa-onnx-offline-tts.exe",
            (Tool::Asr, false) => "bin/sherpa-onnx-offline",
            (Tool::Asr, true) => "bin/sherpa-onnx-offline.exe",
        }
    }
}

fn name() -> String {
    format!("sherpa-onnx-v{VERSION}-{FLAVOR}")
}

fn dir(data_dir: &Path) -> PathBuf {
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

/// Télécharge le moteur si l'outil manque, et n'en garde que les deux outils et
/// leurs bibliothèques (la mise en page bin/ et lib/ est celle qu'attendent les outils).
pub async fn ensure(data_dir: &Path, tool: Tool, cancel: Arc<AtomicBool>, mut on_progress: impl FnMut(u64, u64)) -> Result<()> {
    if ready(data_dir, tool) {
        return Ok(());
    }
    let _turn = FETCH.lock().await;
    if ready(data_dir, tool) {
        return Ok(());
    }
    if !cfg!(any(all(target_os = "macos", target_arch = "aarch64"), all(windows, target_arch = "x86_64"))) {
        return Err(anyhow!(match tool {
            Tool::Tts => crate::i18n::t("La voix naturelle n'est pas encore disponible sur cet ordinateur.", "The natural voice isn't available on this computer yet."),
            Tool::Asr => crate::i18n::t("Parakeet n'est pas encore disponible sur cet appareil.", "Parakeet isn't available on this device yet."),
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
    #[cfg(not(windows))]
    let ok = tokio::process::Command::new("tar").arg("-xjf").arg(&part).arg("-C").arg(&tmp).args(&wanted).output().await?.status.success();
    // Windows : décompressé ici (le tar de Windows 10 ne lit pas toujours le bzip2)
    #[cfg(windows)]
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
        }));
    }
    Ok(())
}
