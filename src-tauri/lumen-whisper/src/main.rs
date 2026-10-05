//! lumen-whisper : transcription locale avec horodatage par mot.
//!
//! Lancé par Lumen comme processus séparé (whisper.cpp et llama.cpp embarquent
//! chacun leur propre copie de ggml, qu'on ne peut pas lier dans le même binaire).
//!
//! Usage : lumen-whisper <modèle.bin> <fichier audio/vidéo> <code langue> [--pcm <sortie.f32>]
//! `--pcm` : enregistre aussi le son décodé (mono, 16 kHz, f32 petit-boutiste),
//! que Lumen confie ensuite à Qwen3-ASR sans avoir à décoder le fichier.
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
use serde::Serialize;
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

// ---------- minutage précis des mots ----------
//
// Les horodatages classiques de whisper.cpp font commencer chaque mot à la fin
// du précédent : ils arrivent en moyenne 0,4 s trop tôt, silences compris.
// On utilise donc l'alignement DTW sur l'attention du modèle (un instant par
// fragment de mot), puis on cale chaque début sur l'attaque réelle du son.

/// Têtes d'attention de référence pour l'alignement, selon le modèle.
fn dtw_preset(model: &str) -> Option<DtwModelPreset> {
    let name = Path::new(model).file_name()?.to_str()?.to_lowercase();
    let en = name.contains(".en");
    Some(if name.contains("large-v3-turbo") {
        DtwModelPreset::LargeV3Turbo
    } else if name.contains("large-v3") {
        DtwModelPreset::LargeV3
    } else if name.contains("large-v2") {
        DtwModelPreset::LargeV2
    } else if name.contains("large") {
        DtwModelPreset::LargeV1
    } else if name.contains("medium") {
        if en { DtwModelPreset::MediumEn } else { DtwModelPreset::Medium }
    } else if name.contains("small") {
        if en { DtwModelPreset::SmallEn } else { DtwModelPreset::Small }
    } else if name.contains("base") {
        if en { DtwModelPreset::BaseEn } else { DtwModelPreset::Base }
    } else if name.contains("tiny") {
        if en { DtwModelPreset::TinyEn } else { DtwModelPreset::Tiny }
    } else {
        return None;
    })
}

/// Énergie du son par tranches de 10 ms, et seuil voix / silence adapté à
/// l'enregistrement (bruit de fond, musique).
struct Envelope {
    db: Vec<f32>,
    thr: f32,
}

const HOP: f64 = 0.01;

impl Envelope {
    fn new(audio: &[f32]) -> Self {
        let hop = (TARGET_RATE as f64 * HOP) as usize;
        let win = hop * 5 / 2;
        let db: Vec<f32> = (0..audio.len() / hop)
            .map(|i| {
                let a = &audio[i * hop..(i * hop + win).min(audio.len())];
                let rms = (a.iter().map(|x| x * x).sum::<f32>() / a.len().max(1) as f32).sqrt();
                20.0 * (rms + 1e-5).log10()
            })
            .collect();
        let mut sorted = db.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f64| sorted.get(((sorted.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(-100.0);
        let (floor, loud) = (pct(0.1), pct(0.9));
        // relatif à la voix : un silence numérique parfait ne doit pas faire
        // passer le moindre souffle pour une syllabe
        let thr = (floor + 0.3 * (loud - floor)).max(loud - 35.0).max(floor + 6.0);
        Self { db, thr }
    }

    fn above(&self, t: f64, frames: usize) -> bool {
        let i = (t / HOP).max(0.0) as usize;
        (i..i + frames).all(|k| self.db.get(k).is_some_and(|d| *d > self.thr))
    }

    fn below(&self, t: f64, frames: usize) -> bool {
        let i = (t / HOP).max(0.0) as usize;
        (i..i + frames).all(|k| self.db.get(k).is_none_or(|d| *d <= self.thr))
    }

    /// Voix présente à cet instant (deux tranches de suite au-dessus du seuil).
    fn voiced(&self, t: f64) -> bool {
        self.above(t, 2)
    }

    /// Attaque : au moins 30 ms de voix à partir d'ici.
    fn attack(&self, t: f64) -> bool {
        self.above(t, 3)
    }

    /// Vraie pause : au moins 50 ms de silence à partir d'ici.
    fn gap(&self, t: f64) -> bool {
        self.below(t, 5)
    }
}

/// Avance moyenne de l'alignement DTW dans la parole continue (mesurée sur
/// des enregistrements de référence en anglais et en espagnol : 80 à 100 ms).
const DTW_LEAD: f64 = 0.09;

/// Cale les débuts de mots sur l'attaque réelle de la voix, sans jamais
/// inverser l'ordre des mots, puis fixe chaque fin avant le silence suivant.
/// En entrée, `t0` est l'estimation du début et `t1` celle de la fin.
fn refine(words: &mut [Word], env: &Envelope, duration: f64) {
    let n = words.len();
    let first_voice = |from: f64, to: f64| {
        let mut f = from;
        while f < to {
            if env.attack(f) {
                return Some(f);
            }
            f += HOP;
        }
        None
    };
    for k in 0..n {
        let lo = if k > 0 { words[k - 1].t0 + 0.04 } else { 0.0 };
        let s = words[k].t0.max(lo);
        // un mot ne commence pas après sa propre fin estimée
        let own_end = words[k].t1.max(s + HOP);
        let t = if !env.voiced(s) {
            // estimation dans un silence (souvent après une pause) : on avance jusqu'à l'attaque
            first_voice(s, own_end.min(s + 2.0).max(s + 0.3)).unwrap_or(s)
        } else {
            // dans la voix : une micro-pause juste devant marque l'attaque,
            // sinon on corrige l'avance moyenne de l'alignement
            let ahead = (s + DTW_LEAD + 0.06).min(own_end);
            let mut f = s;
            let mut found = None;
            while f < ahead {
                if env.gap(f) {
                    found = first_voice(f, own_end);
                    break;
                }
                f += HOP;
            }
            found.unwrap_or_else(|| (s + DTW_LEAD).min(own_end - HOP).max(s))
        };
        words[k].t0 = (t * 100.0).round() / 100.0;
    }
    for k in 0..n {
        let next = if k + 1 < n { words[k + 1].t0 } else { duration };
        // fin : dernière voix avant un silence d'au moins 60 ms, sinon le mot suivant
        let mut end = next;
        let mut f = words[k].t0 + HOP;
        let mut quiet = 0.0;
        while f < next {
            if env.voiced(f) {
                quiet = 0.0;
            } else {
                quiet += HOP;
                if quiet >= 0.06 {
                    end = f - quiet + HOP;
                    break;
                }
            }
            f += HOP;
        }
        words[k].t1 = ((end.max(words[k].t0 + 0.05).min(next.max(words[k].t0))) * 100.0).round() / 100.0;
    }
}

/// Fils de calcul (8 au plus). Sous Windows, où Whisper calcule sur le
/// processeur, un par cœur physique : l'hyperthreading le ralentit.
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

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err(anyhow!("usage : lumen-whisper <modèle> <média> <langue>"));
    }
    let (model, media, lang) = (&args[1], Path::new(&args[2]), args[3].clone());
    let pcm_out = args.iter().position(|a| a == "--pcm").and_then(|i| args.get(i + 1));

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
    let mut cparams = WhisperContextParameters::default();
    let preset = dtw_preset(model);
    let use_dtw = preset.is_some();
    if let Some(model_preset) = preset {
        // l'alignement DTW exige l'attention classique
        cparams.flash_attn(false);
        cparams.dtw_parameters(DtwParameters { mode: DtwMode::ModelPreset { model_preset }, ..Default::default() });
    }
    let ctx = WhisperContext::new_with_params(model, cparams).map_err(|e| anyhow!("modèle Whisper illisible : {e:?}"))?;
    let eot = ctx.token_eot();
    let mut state = ctx.create_state().map_err(|e| anyhow!("{e:?}"))?;

    let threads = threads();
    let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 });
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

    // mots reconstitués à partir des fragments (un mot commence par une espace) ;
    // les octets sont assemblés avant décodage, pour le cyrillique notamment
    let mut words: Vec<Word> = Vec::new();
    let mut cur: Option<(Vec<u8>, f64)> = None;
    // `end` : instant DTW du dernier fragment du mot (sa fin estimée)
    let flush = |cur: &mut Option<(Vec<u8>, f64)>, words: &mut Vec<Word>, end: Option<f64>| {
        if let Some((bytes, t0)) = cur.take() {
            let w = String::from_utf8_lossy(&bytes).to_string();
            let t = w.trim();
            if !t.is_empty() && !(t.starts_with('[') && t.ends_with(']')) {
                words.push(Word { w, t0, t1: end.unwrap_or(t0).max(t0) });
            }
        }
    };
    for seg in state.as_iter() {
        // l'instant DTW d'un fragment marque sa fin : un mot commence là où
        // finit le fragment précédent (en début de segment, l'estimation
        // classique, recalée ensuite sur l'attaque du son)
        let mut prev_end: Option<f64> = None;
        for i in 0..seg.n_tokens() {
            let Some(tok) = seg.get_token(i) else { continue };
            if tok.token_id() >= eot {
                continue; // jetons spéciaux (horodatages, fin de texte)
            }
            let Ok(bytes) = tok.to_bytes() else { continue };
            let data = tok.token_data();
            let legacy = data.t0 as f64 / 100.0;
            if bytes.first() == Some(&b' ') || cur.is_none() {
                flush(&mut cur, &mut words, prev_end);
                let start = if use_dtw { prev_end.unwrap_or(legacy) } else { legacy };
                cur = Some((bytes.to_vec(), start.max(0.0)));
            } else if let Some((b, _)) = cur.as_mut() {
                b.extend_from_slice(bytes);
            }
            if use_dtw && data.t_dtw >= 0 {
                prev_end = Some(data.t_dtw as f64 / 100.0);
            }
        }
        flush(&mut cur, &mut words, prev_end);
    }
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
