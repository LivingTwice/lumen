//! Catalogue des modèles d'IA locaux et téléchargement (avec reprise).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

/// Second fichier d'un modèle, téléchargé avec lui (partie audio de Qwen3-ASR).
#[derive(Clone, Debug)]
pub struct Companion {
    pub url: &'static str,
    pub file: &'static str,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct ModelInfo {
    pub id: &'static str,
    /// "llm" (traduction, explications), "asr" (transcription et minutage des
    /// mots), "asrtext" (texte des transcriptions, plus juste) ou "tts" (voix)
    pub kind: &'static str,
    pub name: &'static str,
    pub detail: &'static str,
    pub size: u64,
    pub url: &'static str,
    /// fichier du modèle, ou dossier une fois l'archive (.tar.bz2) décompressée
    pub file: &'static str,
    pub ram_gb: u32,
    /// second fichier, compris dans `size`
    #[serde(skip)]
    pub companion: Option<Companion>,
}

pub const CATALOG: &[ModelInfo] = &[
    ModelInfo {
        id: "qwen3.5-0.8b",
        kind: "llm",
        name: "Qwen3.5 0.8B",
        detail: "Très rapide, pour les Mac avec 8 Go de mémoire",
        size: 533_000_000,
        url: "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_K_M.gguf",
        file: "Qwen3.5-0.8B-Q4_K_M.gguf",
        ram_gb: 8,
        companion: None,
    },
    ModelInfo {
        id: "qwen3.5-2b",
        kind: "llm",
        name: "Qwen3.5 2B",
        detail: "L'équilibre idéal entre qualité et vitesse",
        size: 1_281_000_000,
        url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf",
        file: "Qwen3.5-2B-Q4_K_M.gguf",
        ram_gb: 8,
        companion: None,
    },
    ModelInfo {
        id: "qwen3.5-4b",
        kind: "llm",
        name: "Qwen3.5 4B",
        detail: "Les traductions les plus fines, à partir de 16 Go",
        size: 2_741_000_000,
        url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
        file: "Qwen3.5-4B-Q4_K_M.gguf",
        ram_gb: 16,
        companion: None,
    },
    ModelInfo {
        id: "whisper-small",
        kind: "asr",
        name: "Whisper Small",
        detail: "Transcription légère et rapide",
        size: 190_000_000,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
        file: "ggml-small-q5_1.bin",
        ram_gb: 8,
        companion: None,
    },
    ModelInfo {
        id: "whisper-turbo",
        kind: "asr",
        name: "Whisper Large v3 Turbo",
        detail: "Transcription et minutage des mots, très précis, 99 langues",
        size: 574_000_000,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        file: "ggml-large-v3-turbo-q5_0.bin",
        ram_gb: 8,
        companion: None,
    },
    ModelInfo {
        id: "qwen3-asr-1.7b",
        kind: "asrtext",
        name: "Qwen3-ASR 1.7B",
        detail: "Texte des transcriptions plus juste que Whisper, sans phrase sautée, dans 23 langues ; Whisper garde le minutage des mots",
        // modèle (2 165 Mo) + partie audio (356 Mo), téléchargés ensemble
        size: 2_165_034_944 + 355_709_344,
        url: "https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF/resolve/main/Qwen3-ASR-1.7B-Q8_0.gguf",
        file: "Qwen3-ASR-1.7B-Q8_0.gguf",
        ram_gb: 16,
        companion: Some(Companion {
            url: "https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF/resolve/main/mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
            file: "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
            size: 355_709_344,
        }),
    },
    ModelInfo {
        id: "supertonic-3",
        kind: "tts",
        name: "Supertonic 3",
        detail: "Voix naturelle pour les mots, les expressions et l'audio des leçons, dans les 31 langues",
        // modèle (128,8 Mo) + moteur sherpa-onnx (20,3 Mo), téléchargés ensemble
        size: 128_774_318 + crate::voice::ENGINE_SIZE,
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2",
        file: "sherpa-onnx-supertonic-3-tts-int8-2026-05-11",
        ram_gb: 8,
        companion: None,
    },
];

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

pub fn path_of(data_dir: &Path, m: &ModelInfo) -> PathBuf {
    models_dir(data_dir).join(m.file)
}

/// Fichier partiel d'un téléchargement interrompu (reprise possible).
pub fn part_of(data_dir: &Path, m: &ModelInfo) -> PathBuf {
    path_of(data_dir, m).with_extension("part")
}

fn is_archive(m: &ModelInfo) -> bool {
    m.url.ends_with(".tar.bz2")
}

/// Chemin du second fichier d'un modèle, s'il en a un.
pub fn companion_path(data_dir: &Path, m: &ModelInfo) -> Option<PathBuf> {
    m.companion.as_ref().map(|c| models_dir(data_dir).join(c.file))
}

pub fn installed(data_dir: &Path, m: &ModelInfo) -> bool {
    let p = path_of(data_dir, m);
    if m.kind == "tts" {
        return p.is_dir() && crate::voice::engine_ready(data_dir);
    }
    p.exists() && companion_path(data_dir, m).is_none_or(|c| c.exists())
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DownloadEvent {
    Progress { received: u64, total: u64, speed: f64 },
    Done,
}

pub async fn download(
    data_dir: &Path,
    m: &ModelInfo,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(DownloadEvent),
) -> Result<()> {
    let dir = models_dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    if installed(data_dir, m) {
        on_event(DownloadEvent::Done);
        return Ok(());
    }
    // la voix a besoin de son moteur : téléchargé d'abord, compté dans la progression
    let mut base = 0u64;
    if m.kind == "tts" {
        crate::voice::ensure_engine(data_dir, cancel.clone(), |received, _| {
            on_event(DownloadEvent::Progress { received, total: m.size, speed: 0.0 });
        })
        .await?;
        base = crate::voice::ENGINE_SIZE;
    }
    // second fichier d'abord (le plus petit), compté dans la progression
    if let (Some(c), Some(cpath)) = (&m.companion, companion_path(data_dir, m)) {
        if !cpath.exists() {
            let cpart = cpath.with_extension("part");
            // la barre de progression porte sur les deux fichiers
            let mut whole = |e: DownloadEvent| {
                on_event(match e {
                    DownloadEvent::Progress { received, speed, .. } => DownloadEvent::Progress { received, total: m.size, speed },
                    e => e,
                })
            };
            fetch_resumable(c.url, &cpart, 0, m.size, cancel.clone(), &mut whole).await?;
            tokio::fs::rename(&cpart, &cpath).await?;
        }
        base = c.size;
    }
    let final_path = path_of(data_dir, m);
    let part = part_of(data_dir, m);
    if !(is_archive(m) && final_path.is_dir()) {
        fetch_resumable(m.url, &part, base, m.size, cancel, &mut on_event).await?;
    }
    if is_archive(m) {
        extract_archive(&part, &dir, &final_path).await?;
        let _ = tokio::fs::remove_file(&part).await;
    } else {
        tokio::fs::rename(&part, &final_path).await?;
    }
    on_event(DownloadEvent::Done);
    Ok(())
}

/// Télécharge `url` dans `part`, en reprenant là où un essai précédent s'était
/// arrêté. `base` : octets déjà comptés avant ce fichier (progression globale).
pub async fn fetch_resumable(
    url: &str,
    part: &Path,
    base: u64,
    expected: u64,
    cancel: Arc<AtomicBool>,
    on_event: &mut impl FnMut(DownloadEvent),
) -> Result<()> {
    let mut start: u64 = tokio::fs::metadata(part).await.map(|md| md.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .user_agent("Lumen/0.1 (+language reader)")
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    let mut req = client.get(url);
    if start > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={start}-"));
    }
    let resp = req.send().await?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(()); // déjà complet
    }
    if !(status.is_success()) {
        return Err(anyhow!("téléchargement refusé ({status})"));
    }
    if start > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
        start = 0; // le serveur ne gère pas la reprise : on recommence
    }
    let total = resp.content_length().map(|l| l + start + base).unwrap_or(expected);
    let mut file = if start > 0 {
        tokio::fs::OpenOptions::new().append(true).open(part).await?
    } else {
        tokio::fs::File::create(part).await?
    };
    let mut received = start + base;
    let mut last = Instant::now();
    let mut window_bytes = 0u64;
    let mut speed = 0.0f64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            file.flush().await?;
            return Err(anyhow!("annulé"));
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        window_bytes += chunk.len() as u64;
        let el = last.elapsed();
        if el >= Duration::from_millis(250) {
            let inst = window_bytes as f64 / el.as_secs_f64();
            speed = if speed == 0.0 { inst } else { speed * 0.7 + inst * 0.3 };
            on_event(DownloadEvent::Progress { received, total, speed });
            last = Instant::now();
            window_bytes = 0;
        }
    }
    file.flush().await?;
    drop(file);
    if received < total.saturating_sub(1024) {
        return Err(anyhow!("téléchargement incomplet, réessayez pour reprendre"));
    }
    on_event(DownloadEvent::Progress { received, total, speed });
    Ok(())
}

/// Décompresse une archive .tar.bz2 dans un dossier provisoire, puis la met en
/// place d'un coup : un dossier présent est toujours un dossier complet.
async fn extract_archive(archive: &Path, dir: &Path, final_path: &Path) -> Result<()> {
    let tmp = dir.join(".extraction");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    let out = tokio::process::Command::new("tar").arg("-xjf").arg(archive).arg("-C").arg(&tmp).output().await?;
    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let _ = tokio::fs::remove_file(archive).await;
        return Err(anyhow!("archive illisible, réessayez le téléchargement"));
    }
    let name = final_path.file_name().ok_or_else(|| anyhow!("chemin invalide"))?;
    let inner = tmp.join(name);
    let src = if inner.is_dir() { inner } else { tmp.clone() };
    let _ = tokio::fs::remove_dir_all(final_path).await;
    tokio::fs::rename(&src, final_path).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(())
}
