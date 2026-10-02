//! lumen-whisper : transcription locale avec horodatage par mot.
//!
//! Lancé par Lumen comme processus séparé (whisper.cpp et llama.cpp embarquent
//! chacun leur propre copie de ggml, qu'on ne peut pas lier dans le même binaire).
//!
//! Usage : lumen-whisper <modèle.bin> <fichier audio/vidéo> <code langue>
//! Sortie (stdout, une ligne JSON par événement) :
//!   {"type":"stage","stage":"decode"}
//!   {"type":"progress","value":42}
//!   {"type":"done","duration":123.4,"words":[{"w":" Hello","t0":0.12,"t1":0.48}, ...]}
//!   {"type":"error","message":"..."}

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const TARGET_RATE: u32 = 16_000;

#[derive(Serialize)]
struct Word {
    w: String,
    t0: f64,
    t1: f64,
}

fn emit(v: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", v);
    let _ = out.flush();
}

/// Décode n'importe quel fichier audio ou vidéo (MP3, AAC/M4A, MP4, WAV, FLAC, OGG…)
/// en échantillons mono f32.
fn decode(path: &Path) -> Result<(Vec<f32>, u32)> {
    let file = File::open(path).with_context(|| format!("impossible d'ouvrir {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("format non reconnu : {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or_else(|| anyhow!("aucune piste audio trouvée"))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| anyhow!("codec non pris en charge : {e}"))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(anyhow!("lecture interrompue : {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let ch = spec.channels.count().max(1);
                let needed = decoded.capacity() as u64;
                if buf.as_ref().map(|b| b.capacity() < decoded.capacity()).unwrap_or(true) {
                    buf = Some(SampleBuffer::<f32>::new(needed, spec));
                }
                let b = buf.as_mut().unwrap();
                b.copy_interleaved_ref(decoded);
                for frame in b.samples().chunks(ch) {
                    mono.push(frame.iter().sum::<f32>() / ch as f32);
                }
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("décodage impossible : {e}")),
        }
    }
    Ok((mono, rate))
}

/// Rééchantillonnage vers 16 kHz : filtre passe-bas (sinc fenêtré, Blackman)
/// appliqué à la volée, puis interpolation linéaire.
fn resample(input: &[f32], from: u32) -> Vec<f32> {
    if from == TARGET_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = TARGET_RATE as f64 / from as f64;
    let len = input.len();
    let out_len = (len as f64 * ratio).floor() as usize;
    let step = 1.0 / ratio;
    if ratio >= 1.0 {
        return (0..out_len)
            .map(|o| {
                let pos = o as f64 * step;
                let i0 = (pos.floor() as usize).min(len - 1);
                let i1 = (i0 + 1).min(len - 1);
                let frac = (pos - i0 as f64) as f32;
                input[i0] + (input[i1] - input[i0]) * frac
            })
            .collect();
    }
    let cutoff = 0.45 * ratio;
    let taps = 16usize;
    let pi = std::f64::consts::PI;
    let mut kernel: Vec<f32> = (0..=2 * taps)
        .map(|i| {
            let n = i as f64 - taps as f64;
            let sinc = if n == 0.0 { 2.0 * cutoff } else { (2.0 * pi * cutoff * n).sin() / (pi * n) };
            let x = i as f64 / (2 * taps) as f64;
            let w = 0.42 - 0.5 * (2.0 * pi * x).cos() + 0.08 * (4.0 * pi * x).cos();
            (sinc * w) as f32
        })
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);
    let filt = |c: isize| -> f32 {
        let mut acc = 0.0f32;
        for (j, k) in kernel.iter().enumerate() {
            let idx = c + j as isize - taps as isize;
            if idx >= 0 && (idx as usize) < len {
                acc += input[idx as usize] * k;
            }
        }
        acc
    };
    (0..out_len)
        .map(|o| {
            let pos = o as f64 * step;
            let i0 = pos.floor() as isize;
            let frac = (pos - i0 as f64) as f32;
            let a = filt(i0);
            let b = filt(i0 + 1);
            a + (b - a) * frac
        })
        .collect()
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err(anyhow!("usage : lumen-whisper <modèle> <média> <langue>"));
    }
    let (model, media, lang) = (&args[1], Path::new(&args[2]), args[3].clone());

    emit(serde_json::json!({"type":"stage","stage":"decode"}));
    let (raw, rate) = decode(media)?;
    if raw.is_empty() {
        return Err(anyhow!("le fichier ne contient pas de son exploitable"));
    }
    let audio = resample(&raw, rate);
    drop(raw);
    let duration = audio.len() as f64 / TARGET_RATE as f64;

    emit(serde_json::json!({"type":"stage","stage":"model","duration":duration}));
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|e| anyhow!("modèle Whisper illisible : {e:?}"))?;
    let mut state = ctx.create_state().map_err(|e| anyhow!("{e:?}"))?;

    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) as i32;
    let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 });
    params.set_n_threads(threads);
    params.set_language(Some(lang.as_str()));
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(true);
    params.set_split_on_word(true);
    params.set_max_len(1);
    params.set_suppress_blank(true);
    params.set_progress_callback_safe(|p: i32| {
        emit(serde_json::json!({"type":"progress","value":p}));
    });

    emit(serde_json::json!({"type":"stage","stage":"transcribe","duration":duration}));
    state.full(params, &audio).map_err(|e| anyhow!("transcription échouée : {e:?}"))?;

    let mut words: Vec<Word> = Vec::new();
    for seg in state.as_iter() {
        let text = match seg.to_str_lossy() {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        if text.trim().is_empty() || text.trim().starts_with('[') && text.trim().ends_with(']') {
            continue;
        }
        let t0 = seg.start_timestamp() as f64 / 100.0;
        let t1 = seg.end_timestamp() as f64 / 100.0;
        words.push(Word { w: text, t0, t1: t1.max(t0) });
    }
    emit(serde_json::json!({"type":"done","duration":duration,"words":words}));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        emit(serde_json::json!({"type":"error","message":format!("{e:#}")}));
        std::process::exit(1);
    }
}
