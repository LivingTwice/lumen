//! Catalogue des modèles d'IA locaux et téléchargement (avec reprise).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

#[derive(Serialize, Clone, Debug)]
pub struct ModelInfo {
    pub id: &'static str,
    /// "llm" (traduction, explications) ou "asr" (transcription)
    pub kind: &'static str,
    pub name: &'static str,
    pub detail: &'static str,
    pub size: u64,
    pub url: &'static str,
    pub file: &'static str,
    pub ram_gb: u32,
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
    },
    ModelInfo {
        id: "whisper-turbo",
        kind: "asr",
        name: "Whisper Large v3 Turbo",
        detail: "Transcription très précise, 99 langues",
        size: 574_000_000,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        file: "ggml-large-v3-turbo-q5_0.bin",
        ram_gb: 8,
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

pub fn installed(data_dir: &Path, m: &ModelInfo) -> bool {
    path_of(data_dir, m).exists()
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
    let final_path = path_of(data_dir, m);
    if final_path.exists() {
        on_event(DownloadEvent::Done);
        return Ok(());
    }
    let part = final_path.with_extension("part");
    let mut start: u64 = tokio::fs::metadata(&part).await.map(|md| md.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .user_agent("Lumen/0.1 (+language reader)")
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    let mut req = client.get(m.url);
    if start > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={start}-"));
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !(status.is_success()) {
        return Err(anyhow!("téléchargement refusé ({status})"));
    }
    if start > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
        start = 0; // le serveur ne gère pas la reprise : on recommence
    }
    let total = resp.content_length().map(|l| l + start).unwrap_or(m.size);
    let mut file = if start > 0 {
        tokio::fs::OpenOptions::new().append(true).open(&part).await?
    } else {
        tokio::fs::File::create(&part).await?
    };
    let mut received = start;
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
    tokio::fs::rename(&part, &final_path).await?;
    on_event(DownloadEvent::Progress { received, total, speed });
    on_event(DownloadEvent::Done);
    Ok(())
}
