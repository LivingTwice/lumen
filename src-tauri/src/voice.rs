//! Voix naturelle pour prononcer un mot ou une expression : Supertonic 3
//! (modèle neuronal, 31 langues dont les six de Lumen), exécuté par l'outil
//! officiel de sherpa-onnx.
//!
//! Le moteur (~30 Mo) et le modèle (~130 Mo) se téléchargent à part, comme les
//! modèles d'IA. Rien ne tourne en permanence : l'outil est lancé à la demande
//! puis s'arrête. Chaque prononciation est gardée en cache (`media/voice`), le
//! même mot revient donc instantanément.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::models::{self, DownloadEvent, ModelInfo};

const ENGINE_VERSION: &str = "1.13.8";
/// Taille de l'archive du moteur (macOS arm64), pour la progression.
pub const ENGINE_SIZE: u64 = 20_314_448;

/// Les langues de Lumen portent le même code chez Supertonic.
use crate::text::LANGS;

/// Locuteur du modèle : 0 à 4 = voix féminines F1 à F5, 5 à 9 = masculines M1 à M5
/// (ordre de voice.bin). « f » et « m » (premiers réglages) valent F1 et M1.
fn speaker(voice: &str) -> u32 {
    match voice {
        "m" => 5,
        v => v.parse::<u32>().ok().filter(|n| *n <= 9).unwrap_or(0),
    }
}

fn engine_name() -> String {
    format!("sherpa-onnx-v{ENGINE_VERSION}-osx-arm64-shared")
}

fn engine_dir(data_dir: &Path) -> PathBuf {
    crate::tools::tools_dir(data_dir).join(engine_name())
}

fn engine_bin(data_dir: &Path) -> PathBuf {
    engine_dir(data_dir).join("bin").join("sherpa-onnx-offline-tts")
}

pub fn engine_ready(data_dir: &Path) -> bool {
    engine_bin(data_dir).exists() && engine_dir(data_dir).join("lib").join("libonnxruntime.dylib").exists()
}

/// Supprime le moteur (avec le modèle de voix).
pub fn remove_engine(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(engine_dir(data_dir));
}

/// Télécharge le moteur s'il manque, et n'en garde que l'outil de synthèse et
/// sa bibliothèque (la mise en page bin/ et lib/ est celle qu'attend l'outil).
pub async fn ensure_engine(data_dir: &Path, cancel: Arc<AtomicBool>, mut on_progress: impl FnMut(u64, u64)) -> Result<()> {
    if engine_ready(data_dir) {
        return Ok(());
    }
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(anyhow!(crate::i18n::t("La voix naturelle n'est disponible que sur Mac pour l'instant.", "The natural voice is only available on Mac for now.")));
    }
    let tools = crate::tools::tools_dir(data_dir);
    tokio::fs::create_dir_all(&tools).await?;
    let name = engine_name();
    let url = format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/v{ENGINE_VERSION}/{name}.tar.bz2");
    let part = tools.join(format!("{name}.part"));
    let mut forward = |e: DownloadEvent| {
        if let DownloadEvent::Progress { received, total, .. } = e {
            on_progress(received, total);
        }
    };
    models::fetch_resumable(&url, &part, 0, ENGINE_SIZE, cancel, &mut forward).await?;
    let out = tokio::process::Command::new("tar")
        .arg("-xjf")
        .arg(&part)
        .arg("-C")
        .arg(&tools)
        .arg(format!("{name}/bin/sherpa-onnx-offline-tts"))
        .arg(format!("{name}/lib/libonnxruntime.dylib"))
        .output()
        .await?;
    let _ = tokio::fs::remove_file(&part).await;
    if !out.status.success() || !engine_ready(data_dir) {
        remove_engine(data_dir);
        return Err(anyhow!(crate::i18n::t("le moteur de voix n'a pas pu être installé, réessayez", "the voice engine couldn't be installed, try again")));
    }
    Ok(())
}

/// Dossier des prononciations en cache (dans `media/`, lisible par l'interface).
fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("media").join("voice")
}

/// Fichier de cache d'une prononciation (même texte, même voix : même fichier).
pub fn cached_path(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(format!("{}\u{1}{}\u{1}{lang}\u{1}{text}", m.id, speaker(voice)));
    cache_dir(data_dir).join(format!("{}.wav", &hex::encode(h.finalize())[..20]))
}

/// Lance le moteur une fois : `text` prononcé dans `out` (WAV PCM 16 bits mono).
async fn run_engine(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str, threads: u32, out: &Path) -> Result<()> {
    let model = models::path_of(data_dir, m);
    let f = |name: &str| model.join(name);
    let mut cmd = tokio::process::Command::new(engine_bin(data_dir));
    cmd.arg(format!("--supertonic-duration-predictor={}", f("duration_predictor.int8.onnx").display()))
        .arg(format!("--supertonic-text-encoder={}", f("text_encoder.int8.onnx").display()))
        .arg(format!("--supertonic-vector-estimator={}", f("vector_estimator.int8.onnx").display()))
        .arg(format!("--supertonic-vocoder={}", f("vocoder.int8.onnx").display()))
        .arg(format!("--supertonic-tts-json={}", f("tts.json").display()))
        .arg(format!("--supertonic-unicode-indexer={}", f("unicode_indexer.bin").display()))
        .arg(format!("--supertonic-voice-style={}", f("voice.bin").display()))
        .arg(format!("--lang={lang}"))
        .arg(format!("--sid={}", speaker(voice)))
        // 8 étapes : la meilleure qualité (5 n'irait que 0,2 s plus vite)
        .arg("--num-steps=8")
        .arg(format!("--num-threads={threads}"))
        .arg(format!("--output-filename={}", out.display()))
        .arg(text)
        .kill_on_drop(true);
    let res = tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| anyhow!(crate::i18n::t("la voix a mis trop de temps à répondre", "the voice took too long to answer")))??;
    if !res.status.success() || !out.exists() {
        let _ = tokio::fs::remove_file(out).await;
        return Err(anyhow!(crate::i18n::t("La voix n'a pas pu prononcer ce texte.", "The voice couldn't pronounce this text.")));
    }
    Ok(())
}

/// Prononce `text` et renvoie le chemin du fichier WAV (déjà en cache, ou créé).
pub async fn say(data_dir: &Path, m: &ModelInfo, lang: &str, text: &str, voice: &str) -> Result<PathBuf> {
    if !LANGS.contains(&lang) {
        return Err(anyhow!(crate::i18n::t("Cette langue n'a pas encore de voix naturelle.", "This language has no natural voice yet.")));
    }
    let dest = cached_path(data_dir, m, lang, text, voice);
    if dest.exists() {
        return Ok(dest);
    }
    tokio::fs::create_dir_all(cache_dir(data_dir)).await?;
    let tmp = dest.with_extension("tmp.wav");
    run_engine(data_dir, m, lang, text, voice, 4, &tmp).await?;
    let raw = tokio::fs::read(&tmp).await?;
    let _ = tokio::fs::remove_file(&tmp).await;
    let clean = polish_wav(&raw).unwrap_or(raw);
    tokio::fs::write(&dest, clean).await?;
    Ok(dest)
}

/// Échantillons d'un WAV PCM 16 bits mono, avec leur fréquence.
fn read_pcm(wav: &[u8]) -> Option<(u32, Vec<i16>)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut i = 12;
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let len = u32::from_le_bytes(wav[i + 4..i + 8].try_into().ok()?) as usize;
        let body = &wav[i + 8..(i + 8 + len).min(wav.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
        }
        i += 8 + len + (len & 1);
    }
    if channels != 1 || bits != 16 || rate == 0 {
        return None;
    }
    Some((rate, data?.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()))
}

fn write_wav(rate: u32, s: &[i16]) -> Vec<u8> {
    let bytes = (s.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for v in s {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Début et fin de la parole (au-dessus de 2 % de la crête), en échantillons.
fn speech_bounds(s: &[i16]) -> Option<(usize, usize, i32)> {
    let peak = s.iter().map(|v| (*v as i32).abs()).max().unwrap_or(0);
    if peak < 200 {
        return None;
    }
    let thr = (peak / 50).max(60);
    let first = s.iter().position(|v| (*v as i32).abs() > thr)?;
    let last = s.iter().rposition(|v| (*v as i32).abs() > thr)?;
    Some((first, last, peak))
}

/// Prépare le son pour une écoute immédiate : retire les silences du début et
/// de la fin (le modèle en ajoute près d'une demi-seconde de chaque côté), égalise
/// le volume et adoucit les bords pour éviter un clic. WAV PCM 16 bits mono.
pub fn polish_wav(wav: &[u8]) -> Option<Vec<u8>> {
    let (rate, s) = read_pcm(wav)?;
    let (first, last, peak) = speech_bounds(&s)?;
    let ms = |x: u32| (rate as usize * x as usize) / 1000;
    let a = first.saturating_sub(ms(30));
    let b = (last + ms(160)).min(s.len());
    let mut s = s[a..b].to_vec();
    let gain = (0.85 * 32767.0 / peak as f32).min(2.5);
    let fade = ms(8).max(1).min(s.len() / 2);
    let n = s.len();
    for (k, v) in s.iter_mut().enumerate() {
        let edge = k.min(n - 1 - k);
        let ramp = if edge < fade { edge as f32 / fade as f32 } else { 1.0 };
        *v = (*v as f32 * gain * ramp).round().clamp(-32768.0, 32767.0) as i16;
    }
    Some(write_wav(rate, &s))
}

// ---------- audio d'une leçon entière ----------

/// Au-delà, une phrase est coupée (aux virgules si possible) : le modèle
/// garde un débit naturel et la lanterne reste précise.
const CHUNK_MAX_WORDS: usize = 40;
/// Phrases prononcées en même temps (chacune lance le moteur).
const PARALLEL: usize = 3;

/// Morceau de texte prononcé d'un seul tenant.
struct Chunk {
    /// texte exact (pour les positions des mots)
    raw: String,
    /// position du début dans le texte de la leçon (unités UTF-16)
    base: usize,
    /// silence après le morceau, en secondes
    pause: f64,
}

fn chunks_of(text: &str, lang: &str) -> Vec<Chunk> {
    let tokens = crate::text::tokenize(text, lang);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (a, b) in crate::text::sentences(&tokens) {
        // coupe les phrases trop longues, de préférence après une virgule
        let mut start = a;
        let mut words = 0usize;
        let mut last_comma: Option<usize> = None;
        for i in a..b {
            if tokens[i].w {
                words += 1;
            } else if tokens[i].t.contains([',', ';', ':', '،', '、', '，']) {
                last_comma = Some(i + 1);
            }
            if words >= CHUNK_MAX_WORDS && i + 1 < b {
                let cut = last_comma.filter(|c| *c > start + 3).unwrap_or(i + 1);
                ranges.push((start, cut));
                start = cut;
                words = tokens[start..=i].iter().filter(|t| t.w).count();
                last_comma = None;
            }
        }
        if tokens[start..b].iter().any(|t| t.w) {
            ranges.push((start, b));
        }
    }
    let mut out = Vec::with_capacity(ranges.len());
    for (k, (a, b)) in ranges.iter().enumerate() {
        let raw: String = tokens[*a..*b].iter().map(|t| t.t.as_str()).collect();
        // pause : plus longue entre deux paragraphes, brève au milieu d'une phrase coupée
        let next_gap = ranges.get(k + 1).map(|(na, _)| tokens[*b..*na].iter().chain(tokens[*na..].iter().take_while(|t| !t.w)).any(|t| t.t.contains('\n')));
        let ends_sentence = tokens[*a..*b].iter().rev().find(|t| !t.t.trim().is_empty()).map(|t| !t.w && crate::text::ends_sentence(&t.t)).unwrap_or(false);
        let pause = match next_gap {
            Some(true) => 0.75,
            Some(false) if ends_sentence => 0.32,
            Some(false) => 0.12,
            None => 0.0,
        };
        out.push(Chunk { raw, base: tokens[*a].s, pause });
    }
    out
}

pub struct LessonAudio {
    pub path: PathBuf,
    /// horodatages par mot, au format de Lumen
    pub timings: Vec<[f64; 4]>,
    pub duration: f64,
}

/// Lit toute une leçon avec la voix naturelle : chaque phrase est prononcée à
/// part (trois à la fois), les morceaux sont assemblés avec de courtes pauses,
/// et la position de chaque phrase dans l'audio donne celle de ses mots.
/// L'audio est compressé en AAC (m4a) dans `media/`.
pub async fn lesson_audio(
    data_dir: &Path,
    m: &'static ModelInfo,
    lang: &str,
    text: &str,
    voice: &str,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<LessonAudio> {
    use futures_util::StreamExt;
    use std::sync::atomic::Ordering;
    if !LANGS.contains(&lang) {
        return Err(anyhow!(crate::i18n::t("Cette langue n'a pas encore de voix naturelle.", "This language has no natural voice yet.")));
    }
    let chunks = chunks_of(text, lang);
    if chunks.is_empty() {
        return Err(anyhow!(crate::i18n::t("Cette leçon n'a pas de texte à lire.", "This lesson has no text to read.")));
    }
    let media = crate::media::media_dir(data_dir);
    let work = media.join(format!(".voice-{}", crate::media::new_stem()));
    tokio::fs::create_dir_all(&work).await?;
    let total = chunks.len();
    // chaque tâche a ses propres copies : elles tournent en parallèle
    let jobs: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let out = work.join(format!("{k}.wav"));
            let spoken = c.raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let (cancel, dir, lang, voice) = (cancel.clone(), data_dir.to_path_buf(), lang.to_string(), voice.to_string());
            async move {
                if cancel.load(Ordering::Relaxed) {
                    return Err(anyhow!("annulé"));
                }
                run_engine(&dir, m, &lang, &spoken, &voice, 2, &out).await?;
                let wav = tokio::fs::read(&out).await?;
                let _ = tokio::fs::remove_file(&out).await;
                read_pcm(&wav).ok_or_else(|| anyhow!(crate::i18n::t("son illisible", "unreadable sound")))
            }
        })
        .collect();
    let mut stream = futures_util::stream::iter(jobs).buffered(PARALLEL);
    let mut rate = 0u32;
    let mut all: Vec<i16> = Vec::new();
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let mut done = 0usize;
    let result: Result<()> = async {
        while let Some(res) = stream.next().await {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("annulé"));
            }
            let (r, s) = res?;
            rate = r;
            let c = &chunks[done];
            let ms = |x: f64| (r as f64 * x / 1000.0) as usize;
            if let Some((first, last, _)) = speech_bounds(&s) {
                let a = first.saturating_sub(ms(20.0));
                let b = (last + ms(80.0)).min(s.len());
                let t0 = all.len() as f64 / r as f64 + (first - a) as f64 / r as f64;
                all.extend_from_slice(&s[a..b]);
                let t1 = all.len() as f64 / r as f64 - (b - last) as f64 / r as f64;
                crate::lingq::spread_words(&c.raw, lang, c.base, t0, t1, &mut timings);
            }
            all.extend(std::iter::repeat_n(0i16, (r as f64 * c.pause) as usize));
            done += 1;
            on_progress(done as f64 / total as f64 * 100.0);
        }
        Ok(())
    }
    .await;
    drop(stream);
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&work).await;
        return Err(e);
    }
    // volume égalisé sur l'ensemble (les phrases gardent leurs nuances entre elles)
    if let Some(peak) = all.iter().map(|v| (*v as i32).abs()).max().filter(|p| *p > 0) {
        let gain = (0.85 * 32767.0 / peak as f32).min(2.5);
        for v in all.iter_mut() {
            *v = (*v as f32 * gain).round().clamp(-32768.0, 32767.0) as i16;
        }
    }
    let duration = all.len() as f64 / rate.max(1) as f64;
    let wav_path = work.join("lecon.wav");
    tokio::fs::write(&wav_path, write_wav(rate, &all)).await?;
    drop(all);
    let stem = crate::media::new_stem();
    let m4a = media.join(format!("{stem}.voice.m4a"));
    // AAC : environ 0,5 Mo par minute au lieu de 5 Mo en WAV
    let conv = tokio::process::Command::new("afconvert")
        .args(["-f", "m4af", "-d", "aac", "-b", "64000"])
        .arg(&wav_path)
        .arg(&m4a)
        .output()
        .await;
    let path = if conv.map(|o| o.status.success()).unwrap_or(false) && m4a.exists() {
        m4a
    } else {
        let wav = media.join(format!("{stem}.voice.wav"));
        tokio::fs::rename(&wav_path, &wav).await?;
        wav
    };
    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(LessonAudio { path, timings, duration })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let bytes = (samples.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&bytes.to_le_bytes());
        for v in samples {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    #[test]
    fn polish_trims_silence_and_levels_volume() {
        let rate = 1000;
        // 400 ms de silence, 200 ms de son, 500 ms de silence
        let mut s = vec![0i16; 400];
        s.extend((0..200).map(|k| if k % 2 == 0 { 12000 } else { -12000 }));
        s.extend(vec![0i16; 500]);
        let out = polish_wav(&wav(&s, rate)).expect("wav valide");
        let n = (out.len() - 44) / 2;
        // 30 ms avant, 200 ms de son, 160 ms après
        assert_eq!(n, 30 + 200 + 159);
        let peak = out[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        // crête ramenée à 85 % du maximum
        assert!((27000..=28500).contains(&peak), "volume égalisé : {peak}");
    }

    #[test]
    fn chunks_follow_sentences_and_paragraphs() {
        let c = chunks_of("One two. Three four?\n\nFive six", "en");
        let raw: Vec<&str> = c.iter().map(|c| c.raw.trim()).collect();
        assert_eq!(raw, ["One two.", "Three four?", "Five six"]);
        assert_eq!(c[0].pause, 0.32);
        assert_eq!(c[1].pause, 0.75);
        assert_eq!(c[2].pause, 0.0);
        // positions UTF-16 du début de chaque morceau
        assert_eq!(c[0].base, 0);
        assert!(c[2].raw.trim_start().starts_with("Five"));
        // phrase très longue : coupée en morceaux d'au plus 40 mots
        let long = (0..95).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ") + ".";
        let parts = chunks_of(&long, "en");
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.raw.split_whitespace().count() <= 41));
    }

    #[test]
    fn polish_leaves_unknown_formats_alone() {
        assert!(polish_wav(b"pas un wav").is_none());
        assert!(polish_wav(&wav(&[0; 100], 1000)).is_none());
    }
}

/// Test réel (télécharge le moteur dans un dossier jetable) :
/// `LUMEN_VOICE_MODEL=/chemin/sherpa-onnx-supertonic-3-tts-int8-2026-05-11 cargo test --lib voice_live -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    #[ignore]
    async fn voice_live() {
        let model_src = PathBuf::from(std::env::var("LUMEN_VOICE_MODEL").expect("LUMEN_VOICE_MODEL"));
        let data = std::env::temp_dir().join("lumen-voice-live");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("models")).unwrap();
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        std::os::unix::fs::symlink(&model_src, models::path_of(&data, m)).unwrap();

        let t = Instant::now();
        ensure_engine(&data, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        println!("moteur installé en {:.1} s", t.elapsed().as_secs_f32());
        assert!(models::installed(&data, m));

        for (lang, text) in [("en", "thoroughly"), ("it", "sfortunatamente"), ("de", "Schifffahrt"), ("pt", "saudade"), ("ru", "счастье"), ("es", "hasta luego")] {
            let t = Instant::now();
            let p = say(&data, m, lang, text, "f").await.unwrap();
            let first = t.elapsed().as_secs_f32();
            let t = Instant::now();
            say(&data, m, lang, text, "f").await.unwrap();
            let again = t.elapsed().as_secs_f32();
            let len = std::fs::metadata(&p).unwrap().len();
            let secs = (len - 44) as f32 / 2.0 / 44100.0;
            println!("{lang} « {text} » : {first:.2} s, puis {:.0} ms en cache, son de {secs:.2} s", again * 1000.0);
            assert!(secs > 0.2 && secs < 3.0);
            if let Ok(out) = std::env::var("LUMEN_VOICE_OUT") {
                std::fs::copy(&p, PathBuf::from(out).join(format!("{lang}.wav"))).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Audio d'une leçon entière, puis recalage par Whisper si `LUMEN_ASR_MODEL` est donné :
    /// `LUMEN_VOICE_MODEL=… LUMEN_ASR_MODEL=…/ggml-large-v3-turbo-q5_0.bin cargo test --lib lesson_audio_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn lesson_audio_live() {
        let model_src = PathBuf::from(std::env::var("LUMEN_VOICE_MODEL").expect("LUMEN_VOICE_MODEL"));
        let data = std::env::temp_dir().join("lumen-voice-lesson");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("models")).unwrap();
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        std::os::unix::fs::symlink(&model_src, models::path_of(&data, m)).unwrap();
        ensure_engine(&data, Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap();
        let text = "Ogni mattina Marta saliva le scale strette del vecchio faro. Dall'alto, il mare sembrava infinito e tranquillo. Le piaceva ascoltare il vento mentre il sole sorgeva lentamente dietro le nuvole.\n\nUn giorno trovò una lettera nascosta tra due pietre. La carta era umida, ma le parole si potevano ancora leggere: «Se stai leggendo queste righe, non sei sola».";
        let t = Instant::now();
        let mut steps = 0;
        let a = lesson_audio(&data, m, "it", text, "3", Arc::new(AtomicBool::new(false)), |_| steps += 1).await.unwrap();
        let words = crate::text::tokenize(text, "it").iter().filter(|t| t.w).count();
        println!("audio de {:.1} s créé en {:.1} s ({} morceaux), {} mots minutés sur {words}, fichier {}", a.duration, t.elapsed().as_secs_f32(), steps, a.timings.len(), a.path.display());
        assert_eq!(a.timings.len(), words);
        assert!(a.timings.windows(2).all(|w| w[1][2] >= w[0][2]), "horodatages dans l'ordre");
        assert!(a.path.extension().unwrap() == "m4a");
        if let Ok(asr) = std::env::var("LUMEN_ASR_MODEL") {
            let heard = crate::media::transcribe(Path::new(&asr), &a.path, "it", None, |_| {}).await.unwrap();
            let (aligned, found) = crate::media::align_timings(text, "it", &heard);
            println!("Whisper retrouve {:.0} % des mots", found * 100.0);
            let precise: Vec<[f64; 4]> = serde_json::from_str(&aligned).unwrap();
            let gap: f64 = a.timings.iter().zip(&precise).map(|(x, y)| (x[2] - y[2]).abs()).sum::<f64>() / words as f64;
            println!("écart moyen entre la répartition et Whisper : {:.0} ms", gap * 1000.0);
            assert!(found > 0.8);
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Téléchargement complet comme dans l'app (moteur + modèle, ~150 Mo) :
    /// `cargo test --lib voice_download_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn voice_download_live() {
        let data = std::env::temp_dir().join("lumen-voice-download");
        let _ = std::fs::remove_dir_all(&data);
        let m = models::CATALOG.iter().find(|m| m.kind == "tts").unwrap();
        let t = Instant::now();
        let mut last = 0u64;
        models::download(&data, m, Arc::new(AtomicBool::new(false)), |e| {
            if let DownloadEvent::Progress { received, total, .. } = e {
                assert!(received >= last && received <= total + 1024, "progression {received}/{total}");
                last = received;
            }
        })
        .await
        .unwrap();
        println!("installé en {:.1} s ({last} octets annoncés sur {})", t.elapsed().as_secs_f32(), m.size);
        assert!(models::installed(&data, m));
        assert!(!models::part_of(&data, m).exists());
        let p = say(&data, m, "it", "buongiorno", "m").await.unwrap();
        assert!(std::fs::metadata(p).unwrap().len() > 10_000);
        let _ = std::fs::remove_dir_all(&data);
    }
}
