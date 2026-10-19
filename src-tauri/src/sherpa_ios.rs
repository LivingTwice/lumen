//! iPhone et iPad : la voix naturelle (Supertonic 3) et Parakeet par la bibliothèque
//! sherpa-onnx, dans l'app (iOS interdit les outils `sherpa-onnx-offline-tts` et
//! `sherpa-onnx-offline` qu'emploient le Mac et Windows). Le crate officiel
//! `sherpa-onnx` apporte le framework iOS (onnxruntime compris), que le projet Xcode
//! intègre à l'app (gen/apple/project.yml). Mêmes modèles, mêmes réglages qu'ailleurs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use sherpa_onnx::{GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig};

/// La voix chargée (environ 150 Mo) : la garder rend chaque mot suivant immédiat.
static VOICE: Mutex<Option<(PathBuf, OfflineTts)>> = Mutex::new(None);

fn path(p: PathBuf) -> Option<String> {
    Some(p.to_string_lossy().into_owned())
}

/// Prononce `text` (Supertonic 3) : fréquence et échantillons (mono, f32).
pub fn tts(model: &Path, lang: &str, text: &str, sid: u32, threads: u32) -> Result<(u32, Vec<f32>)> {
    let mut slot = VOICE.lock();
    if slot.as_ref().is_none_or(|(dir, _)| dir != model) {
        *slot = None;
        let f = |name: &str| path(model.join(name));
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                supertonic: OfflineTtsSupertonicModelConfig {
                    duration_predictor: f("duration_predictor.int8.onnx"),
                    text_encoder: f("text_encoder.int8.onnx"),
                    vector_estimator: f("vector_estimator.int8.onnx"),
                    vocoder: f("vocoder.int8.onnx"),
                    tts_json: f("tts.json"),
                    unicode_indexer: f("unicode_indexer.bin"),
                    voice_style: f("voice.bin"),
                },
                num_threads: threads as i32,
                ..Default::default()
            },
            ..Default::default()
        };
        let tts = OfflineTts::create(&config).ok_or_else(|| anyhow!(crate::i18n::t("La voix naturelle n'a pas pu se charger.", "The natural voice couldn't load.")))?;
        *slot = Some((model.to_path_buf(), tts));
    }
    let (_, tts) = slot.as_ref().expect("voix chargée");
    let mut extra = HashMap::new();
    extra.insert("lang".to_string(), serde_json::json!(lang));
    // 8 étapes : la meilleure qualité, comme sur Mac
    let gen = GenerationConfig { sid: sid as i32, num_steps: 8, extra: Some(extra), ..Default::default() };
    let audio = tts
        .generate_with_config(text, &gen, None::<fn(&[f32], f32) -> bool>)
        .ok_or_else(|| anyhow!(crate::i18n::t("La voix n'a pas pu prononcer ce texte.", "The voice couldn't pronounce this text.")))?;
    Ok((audio.sample_rate() as u32, audio.samples().to_vec()))
}

/// Libère la voix (modèle supprimé, mémoire demandée par iOS).
pub fn release_voice() {
    *VOICE.lock() = None;
}

/// Le texte de chaque morceau (mono, 16 kHz), décodés ensemble par Parakeet. Le modèle
/// (près d'1 Go en mémoire) se charge le temps de la transcription.
pub struct Parakeet(OfflineRecognizer);

impl Parakeet {
    pub fn load(model: &Path, threads: i32) -> Result<Parakeet> {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path(model.join("encoder.onnx")),
            decoder: path(model.join("decoder.onnx")),
            joiner: path(model.join("joiner.onnx")),
        };
        config.model_config.tokens = path(model.join("tokens.txt"));
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = threads;
        OfflineRecognizer::create(&config)
            .map(Parakeet)
            .ok_or_else(|| anyhow!(crate::i18n::t("Parakeet n'a pas pu se charger.", "Parakeet couldn't load.")))
    }

    pub fn decode(&self, pieces: &[&[f32]]) -> Vec<String> {
        let streams: Vec<_> = pieces
            .iter()
            .map(|p| {
                let s = self.0.create_stream();
                s.accept_waveform(16_000, p);
                s
            })
            .collect();
        let refs: Vec<_> = streams.iter().collect();
        self.0.decode_multiple_streams(&refs);
        streams.iter().map(|s| s.get_result().map(|r| r.text.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()).collect()
    }
}
