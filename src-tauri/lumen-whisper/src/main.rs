//! lumen-whisper : transcription locale avec horodatage par mot.
//!
//! Lancé par Lumen comme processus séparé (whisper.cpp et llama.cpp embarquent
//! chacun leur propre copie de ggml, qu'on ne peut pas lier dans le même binaire).
//!
//! Usage : lumen-whisper <modèle.bin> <fichier audio/vidéo> <code langue> [--pcm <sortie.f32>] [--cpu]
//! `--pcm` : enregistre aussi le son décodé (mono, 16 kHz, f32 petit-boutiste),
//! que Lumen confie ensuite à Qwen3-ASR sans avoir à décoder le fichier.
//! `--cpu` : le processeur seul (choisi dans Lumen, sous Windows) ; sinon la carte
//! graphique (Metal sur Mac, Vulkan sous Windows) si elle est là et a la place.
//! Sortie (stdout, une ligne JSON par événement) :
//!   {"type":"stage","stage":"decode"}
//!   {"type":"progress","value":42}
//!   {"type":"done","duration":123.4,"words":[{"w":" Hello","t0":0.12,"t1":0.48}, ...]}
//!   {"type":"error","message":"..."}
//!
//! Minutage des mots : alignement DTW de whisper.cpp, puis calage sur l'attaque
//! réelle de la voix (écart moyen mesuré : 30 à 45 ms, contre 0,4 s avant).

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

mod timing;
use timing::{assemble, dtw_name, refine, resample, Envelope, Tok, TARGET_RATE};

// Windows : Vulkan chargé seulement s'il est là (le même fichier que l'app)
#[cfg(windows)]
#[path = "../../src/vulkan.rs"]
mod vulkan;

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

/// Fils de calcul (8 au plus). Sous Windows, un par cœur physique :
/// l'hyperthreading ralentit ce qui se calcule sur le processeur.
fn threads() -> i32 {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    #[cfg(windows)]
    let n = physical_cores().unwrap_or(n);
    n.min(8) as i32
}

#[cfg(windows)]
fn physical_cores() -> Option<usize> {
    use windows_sys::Win32::System::SystemInformation::{GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION};
    let size = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    let mut len = 0u32;
    unsafe {
        GetLogicalProcessorInformation(std::ptr::null_mut(), &mut len);
        if (len as usize) < size {
            return None;
        }
        let mut buf: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> = vec![std::mem::zeroed(); len as usize / size + 1];
        if GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) == 0 {
            return None;
        }
        let n = (len as usize / size).min(buf.len());
        let cores = buf[..n].iter().filter(|i| i.Relationship == RelationProcessorCore).count();
        (cores > 0).then_some(cores)
    }
}

/// Windows : la carte graphique dédiée plutôt que celle intégrée au processeur.
/// whisper.cpp prend la première que Vulkan présente, souvent l'intégrée sur un
/// portable qui a les deux. Rang parmi les cartes graphiques.
#[cfg(windows)]
fn gpu_device() -> i32 {
    use whisper_rs_sys::{
        ggml_backend_dev_count, ggml_backend_dev_get, ggml_backend_dev_type, ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU as GPU,
        ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU as IGPU,
    };
    let kinds: Vec<_> = unsafe { (0..ggml_backend_dev_count()).map(|i| ggml_backend_dev_type(ggml_backend_dev_get(i))).collect() };
    kinds.into_iter().filter(|k| *k == GPU || *k == IGPU).position(|k| k == GPU).unwrap_or(0) as i32
}

/// Têtes d'attention de référence pour l'alignement, selon le modèle (`timing::dtw_name`).
fn dtw_preset(model: &str) -> Option<DtwModelPreset> {
    Some(match dtw_name(model)? {
        "large-v3-turbo" => DtwModelPreset::LargeV3Turbo,
        "large-v3" => DtwModelPreset::LargeV3,
        "large-v2" => DtwModelPreset::LargeV2,
        "large-v1" => DtwModelPreset::LargeV1,
        "medium.en" => DtwModelPreset::MediumEn,
        "medium" => DtwModelPreset::Medium,
        "small.en" => DtwModelPreset::SmallEn,
        "small" => DtwModelPreset::Small,
        "base.en" => DtwModelPreset::BaseEn,
        "base" => DtwModelPreset::Base,
        "tiny.en" => DtwModelPreset::TinyEn,
        _ => DtwModelPreset::Tiny,
    })
}

/// Le modèle et son état de travail, sur la carte graphique ou sur le processeur.
fn load(model: &str, gpu: bool) -> Result<(WhisperContext, whisper_rs::WhisperState)> {
    let mut cparams = WhisperContextParameters::default();
    cparams.use_gpu(gpu);
    #[cfg(windows)]
    if gpu {
        cparams.gpu_device(gpu_device());
    }
    if std::env::var("LUMEN_W_FA").is_ok() { cparams.flash_attn(true); } else // ESSAI
    if let Some(model_preset) = dtw_preset(model) {
        // l'alignement DTW exige l'attention classique
        cparams.flash_attn(false);
        cparams.dtw_parameters(DtwParameters { mode: DtwMode::ModelPreset { model_preset }, ..Default::default() });
    }
    let ctx = WhisperContext::new_with_params(model, cparams).map_err(|e| anyhow!("modèle Whisper illisible : {e:?}"))?;
    let state = ctx.create_state().map_err(|e| anyhow!("{e:?}"))?;
    Ok((ctx, state))
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err(anyhow!("usage : lumen-whisper <modèle> <média> <langue>"));
    }
    let (model, media, lang) = (&args[1], Path::new(&args[2]), args[3].clone());
    let pcm_out = args.iter().position(|a| a == "--pcm").and_then(|i| args.get(i + 1));
    let gpu = !args.iter().any(|a| a == "--cpu");

    emit(serde_json::json!({"type":"stage","stage":"decode"}));
    let (raw, rate) = decode(media)?;
    if raw.is_empty() {
        return Err(anyhow!("le fichier ne contient pas de son exploitable"));
    }
    let audio = resample(&raw, rate);
    drop(raw);
    let duration = audio.len() as f64 / TARGET_RATE as f64;
    if let Some(out) = pcm_out {
        let bytes: Vec<u8> = audio.iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(out, bytes).with_context(|| format!("impossible d'écrire {out}"))?;
    }

    emit(serde_json::json!({"type":"stage","stage":"model","duration":duration}));
    let use_dtw = dtw_preset(model).is_some();
    // Windows : une carte graphique sans la place pour Whisper laisse faire le processeur
    let (ctx, mut state) = match load(model, gpu) {
        Err(_) if gpu && cfg!(windows) => load(model, false)?,
        r => r?,
    };
    let eot = ctx.token_eot();

    let threads = threads();
    // ESSAI (provisoire)
    let strategy = match std::env::var("LUMEN_W_BEAM").ok().and_then(|v| v.parse::<i32>().ok()) {
        Some(0) => SamplingStrategy::Greedy { best_of: 1 },
        Some(b) => SamplingStrategy::BeamSearch { beam_size: b, patience: -1.0 },
        None => SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 },
    };
    let mut params = FullParams::new(strategy);
    params.set_n_threads(threads);
    params.set_language(Some(lang.as_str()));
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(true);
    params.set_suppress_blank(true);
    params.set_progress_callback_safe(|p: i32| {
        emit(serde_json::json!({"type":"progress","value":p}));
    });

    emit(serde_json::json!({"type":"stage","stage":"transcribe","duration":duration}));
    state.full(params, &audio).map_err(|e| anyhow!("transcription échouée : {e:?}"))?;
    if std::env::var("LUMEN_W_TIMINGS").is_ok() {
        ctx.print_timings();
    }

    // fragments de chaque segment, puis mots (timing::assemble)
    let segments: Vec<Vec<Tok>> = state
        .as_iter()
        .map(|seg| {
            (0..seg.n_tokens())
                .filter_map(|i| {
                    let tok = seg.get_token(i)?;
                    let bytes = tok.to_bytes().ok()?.to_vec();
                    let data = tok.token_data();
                    Some(Tok { id: tok.token_id(), bytes, t0: data.t0, t_dtw: data.t_dtw })
                })
                .collect()
        })
        .collect();
    let mut words = assemble(&segments, eot, use_dtw);
    refine(&mut words, &Envelope::new(&audio), duration);
    emit(serde_json::json!({"type":"done","duration":duration,"words":words}));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        emit(serde_json::json!({"type":"error","message":format!("{e:#}")}));
        std::process::exit(1);
    }
}
