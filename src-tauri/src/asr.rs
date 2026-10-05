//! Transcription par Qwen3-ASR (llama.cpp, entrée audio « mtmd »), plus juste
//! que Whisper dans 23 des langues de Lumen. Le texte vient de Qwen3-ASR ; le
//! minutage des mots reste celui de Whisper (`lumen-whisper`, calé au mot près),
//! recalé ensuite sur ce texte par `media::align_timings`.

use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::mtmd::{mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;

use crate::ai::{backend, Engine};

/// Fréquence d'échantillonnage attendue (mono, f32).
pub const RATE: usize = 16_000;

/// Durée visée d'un morceau, coupé dans le silence le plus proche : assez long
/// pour le contexte, assez court pour que le chat garde la main entre deux.
pub const CHUNK_SECS: f64 = 30.0;

/// Nom de la langue pour Qwen3-ASR, ou `None` si elle n'est pas prise en charge
/// (Whisper transcrit alors seul).
pub fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "English",
        "it" => "Italian",
        "de" => "German",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "es" => "Spanish",
        "fr" => "French",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "da" => "Danish",
        "fi" => "Finnish",
        "pl" => "Polish",
        "cs" => "Czech",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "el" => "Greek",
        "tr" => "Turkish",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "ko" => "Korean",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => return None,
    })
}

/// Transcrit un son (mono, 16 kHz) dans la langue donnée. Chaque morceau passe
/// sur le GPU à son tour (`Engine::exclusive`) : le chat et la traduction des
/// mots s'intercalent entre deux morceaux.
pub fn transcribe(
    engine: &Engine,
    model_path: &Path,
    mmproj_path: &Path,
    pcm: &[f32],
    lang: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f64),
) -> Result<String> {
    let name = language_name(lang).ok_or_else(|| anyhow!(crate::i18n::t("langue non prise en charge par Qwen3-ASR", "language not supported by Qwen3-ASR")))?;
    let be = backend()?;
    let model = engine.exclusive(|| LlamaModel::load_from_file(be, model_path, &LlamaModelParams::default().with_n_gpu_layers(999)))
        .map_err(|e| anyhow!(crate::tr!("modèle de transcription illisible : {e}", "unreadable transcription model: {e}")))?;
    let threads = crate::ai::threads();
    let params = MtmdContextParams {
        use_gpu: true,
        print_timings: false,
        n_threads: threads,
        media_marker: CString::new(mtmd_default_marker())?,
        image_min_tokens: -1,
        image_max_tokens: -1,
    };
    let mmproj = mmproj_path.to_str().ok_or_else(|| anyhow!(crate::i18n::t("chemin du modèle illisible", "unreadable model path")))?;
    let mctx = engine
        .exclusive(|| MtmdContext::init_from_file(mmproj, &model, &params))
        .map_err(|e| anyhow!(crate::tr!("partie audio du modèle illisible : {e}", "unreadable audio part of the model: {e}")))?;
    let pieces = split_on_silence(pcm, CHUNK_SECS);
    let mut texts: Vec<String> = Vec::new();
    for (a, b) in pieces {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("annulé"));
        }
        let t = engine.exclusive(|| transcribe_piece(&model, &mctx, &pcm[a..b], name, threads))?;
        if !t.is_empty() {
            texts.push(t);
        }
        on_progress(b as f64 / pcm.len().max(1) as f64 * 100.0);
    }
    // japonais et chinois s'écrivent sans espaces
    let sep = if matches!(lang, "ja" | "zh") { "" } else { " " };
    Ok(texts.join(sep))
}

fn transcribe_piece(model: &LlamaModel, mctx: &MtmdContext, audio: &[f32], name: &str, threads: i32) -> Result<String> {
    // un morceau trop court est complété de silence (le modèle attend au moins une demi-seconde)
    let mut padded;
    let audio = if audio.len() < RATE / 2 {
        padded = audio.to_vec();
        padded.resize(RATE / 2, 0.0);
        &padded[..]
    } else {
        audio
    };
    let bitmap = MtmdBitmap::from_audio_data(audio).map_err(|e| anyhow!(crate::tr!("son illisible : {e}", "unreadable sound: {e}")))?;
    // format de Qwen3-ASR ; la langue imposée donne une sortie en texte seul
    let prompt = format!(
        "<|im_start|>system\n<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\nlanguage {name}<asr_text>",
        mtmd_default_marker()
    );
    let chunks = mctx
        .tokenize(MtmdInputText { text: prompt, add_special: false, parse_special: true }, &[&bitmap])
        .map_err(|e| anyhow!("préparation du son : {e}"))?;
    let secs = audio.len() as f64 / RATE as f64;
    // au-delà de ~7 jetons par seconde, le modèle tourne en rond
    let max_new = (secs * 7.0) as usize + 48;
    let n_ctx = (chunks.total_tokens() + max_new + 16).max(512) as u32;
    let be = backend()?;
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx))
        .with_n_batch(n_ctx.min(4096))
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut ctx = model.new_context(be, ctx_params).map_err(|e| anyhow!("contexte : {e}"))?;
    let mut n_past = chunks
        .eval_chunks(mctx, &ctx, 0, 0, n_ctx.min(4096) as i32, true)
        .map_err(|e| anyhow!("écoute du son : {e}"))?;

    let vocab = model.vocab();
    let mut sampler = LlamaSampler::greedy();
    let mut batch = LlamaBatch::new(8, 1);
    let mut bytes: Vec<u8> = Vec::new();
    let mut idx = -1;
    for _ in 0..max_new {
        let token = sampler.sample(&ctx, idx);
        sampler.accept(token);
        if vocab.is_eog(token) {
            break;
        }
        bytes.extend(vocab.token_to_piece(token, false, None));
        if bytes.ends_with(b"<|im_end|>") {
            bytes.truncate(bytes.len() - 10);
            break;
        }
        if bytes.ends_with(b" ") || bytes.ends_with(b".") {
            let text = String::from_utf8_lossy(&bytes).to_string();
            if let Some(cut) = repeated_tail(&text) {
                bytes.truncate(cut);
                break;
            }
        }
        batch.clear();
        batch.add(token, n_past, &[0], true).map_err(|e| anyhow!("{e}"))?;
        n_past += 1;
        ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
        idx = batch.n_tokens() - 1;
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    // au cas où le modèle répéterait l'en-tête de langue
    let text = text.rsplit("<asr_text>").next().unwrap_or("").replace("<|im_end|>", "");
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Boucle en fin de texte : une suite de 1 à 8 mots répétée au moins 4 fois de
/// suite. Renvoie la position (octets) où couper, après la première occurrence.
fn repeated_tail(text: &str) -> Option<usize> {
    let words: Vec<(usize, &str)> = text.split_whitespace().map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w)).collect();
    // « E poi » et « e poi, » sont le même motif
    let keys: Vec<String> = words.iter().map(|(_, w)| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).collect();
    let n = words.len();
    for k in 1..=8 {
        let reps = if k == 1 { 6 } else { 4 };
        if n < k * reps {
            continue;
        }
        let unit = &keys[n - k..];
        let same = (1..reps).all(|r| (0..k).all(|i| keys[n - k * (r + 1) + i] == unit[i]));
        if same {
            // garde une seule occurrence de la suite répétée
            let first = n - k * reps;
            let (pos, w) = words[first + k - 1];
            return Some(pos + w.len());
        }
    }
    None
}

/// Découpe en morceaux d'environ `max_secs`, chaque coupure tombant dans le
/// passage le plus calme (fenêtre de 100 ms) à ± 5 s de la limite, comme le
/// fait Qwen. Les morceaux se suivent sans trou ni chevauchement.
pub fn split_on_silence(pcm: &[f32], max_secs: f64) -> Vec<(usize, usize)> {
    let total = pcm.len();
    let max_len = (max_secs * RATE as f64) as usize;
    let expand = 5 * RATE;
    let win = RATE / 10;
    let mut out = Vec::new();
    let mut start = 0;
    while total - start > max_len {
        let cut = start + max_len;
        let left = cut.saturating_sub(expand).max(start);
        let right = (cut + expand).min(total);
        let mut boundary = cut;
        if right - left > win {
            // somme glissante de l'amplitude : le minimum est le silence le plus net
            let mut sum: f32 = pcm[left..left + win].iter().map(|x| x.abs()).sum();
            let (mut best, mut best_at) = (sum, left);
            for i in left + 1..=right - win {
                sum += pcm[i + win - 1].abs() - pcm[i - 1].abs();
                if sum < best {
                    best = sum;
                    best_at = i;
                }
            }
            boundary = best_at + win / 2;
        }
        let boundary = boundary.clamp(start + 1, total);
        out.push((start, boundary));
        start = boundary;
    }
    if start < total {
        out.push((start, total));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_fall_in_silences() {
        // 70 s de « voix » avec des silences à 28 s et 57 s : les coupures y tombent
        let mut pcm: Vec<f32> = (0..70 * RATE).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        for s in [28.0, 57.0] {
            let a = (s * RATE as f64) as usize;
            pcm[a..a + RATE / 2].iter_mut().for_each(|x| *x = 0.0);
        }
        let parts = split_on_silence(&pcm, 30.0);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].0, 0);
        assert_eq!(parts.last().unwrap().1, pcm.len());
        assert!(parts.windows(2).all(|w| w[0].1 == w[1].0));
        let at = |i: usize| parts[i].1 as f64 / RATE as f64;
        assert!((28.0..28.5).contains(&at(0)) && (57.0..57.5).contains(&at(1)), "{} {}", at(0), at(1));
        assert_eq!(split_on_silence(&pcm[..RATE * 10], 30.0), vec![(0, RATE * 10)]);
    }

    #[test]
    fn loops_are_cut() {
        let t = "Ciao a tutti. E poi e poi e poi e poi ";
        let cut = repeated_tail(t).unwrap();
        assert_eq!(&t[..cut], "Ciao a tutti. E poi");
        assert_eq!(repeated_tail("Andrea va al negozio e prende un cestino. "), None);
        assert_eq!(language_name("it"), Some("Italian"));
        assert_eq!(language_name("uk"), None);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Lit un WAV en f32, mono, 16 kHz (afconvert -f WAVE -d LEF32@16000 -c 1).
    fn read_wav(path: &str) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let id = &b[i..i + 4];
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if id == b"data" {
                return b[i + 8..(i + 8 + len).min(b.len())].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("pas de données dans {path}");
    }

    /// Transcription réelle : LUMEN_ASR_DIR=dossier (Qwen3-ASR-1.7B-Q8_0.gguf et son mmproj)
    /// LUMEN_TEST_AUDIO=a.wav,b.wav LUMEN_TEST_LANG=it [LUMEN_ASR_OUT=dossier]
    /// cargo test --release --lib asr_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn asr_live() {
        let (Ok(dir), Ok(files)) = (std::env::var("LUMEN_ASR_DIR"), std::env::var("LUMEN_TEST_AUDIO")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let dir = Path::new(&dir);
        let model = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.ends_with(".gguf") && !n.starts_with("mmproj")
        });
        let mmproj = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("mmproj"));
        let (model, mmproj) = (model.unwrap(), mmproj.unwrap());
        let engine = Engine::new();
        for f in files.split(',') {
            let pcm = read_wav(f);
            let t = std::time::Instant::now();
            let text = transcribe(&engine, &model, &mmproj, &pcm, &lang, &AtomicBool::new(false), |_| {}).unwrap();
            let secs = pcm.len() as f64 / RATE as f64;
            println!("\n=== {f} : {secs:.0} s d'audio, transcrit en {:.1} s\n{text}", t.elapsed().as_secs_f64());
            if let Ok(out) = std::env::var("LUMEN_ASR_OUT") {
                let name = Path::new(f).file_stem().unwrap().to_string_lossy().to_string();
                std::fs::write(Path::new(&out).join(format!("{name}.qwen.txt")), &text).unwrap();
            }
            assert!(!text.is_empty());
        }
    }
}
