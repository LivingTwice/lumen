//! Import audio et vidéo : copie dans la bibliothèque, transcription locale
//! par le processus `lumen-whisper`, construction du texte et des horodatages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::text;

#[derive(Deserialize, Debug)]
pub struct WWord {
    pub w: String,
    pub t0: f64,
    pub t1: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ImportEvent {
    Stage { stage: String },
    Progress { value: f64 },
}

pub fn media_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("media")
}

pub fn sidecar_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| anyhow!(crate::i18n::t("dossier de l'application introuvable", "application folder not found")))?;
    let name = if cfg!(windows) { "lumen-whisper.exe" } else { "lumen-whisper" };
    let p = dir.join(name);
    if p.exists() {
        return Ok(p);
    }
    Err(anyhow!(crate::i18n::t("le composant de transcription est absent de l'application", "the transcription component is missing from the app")))
}

/// Durée d'un son ou d'une vidéo, lue dans le fichier ; à défaut (Windows : MP3,
/// WebM), la fin du dernier mot minuté, un peu courte mais juste à la minute.
pub fn duration_of(path: &Path, last_word_end: Option<f64>) -> Option<f64> {
    crate::compress::duration(path).or(last_word_end).filter(|d| d.is_finite() && *d > 0.0).map(|d| (d * 10.0).round() / 10.0)
}

/// Copie le fichier dans la bibliothèque de Lumen et renvoie son nouveau chemin.
pub fn store_media(data_dir: &Path, src: &Path) -> Result<PathBuf> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin").to_lowercase();
    let name = format!("{}.{}", chrono::Utc::now().format("%Y%m%d-%H%M%S%3f"), ext);
    let dst = dir.join(name);
    std::fs::copy(src, &dst)?;
    Ok(dst)
}

/// Transcrit avec Whisper (processus `lumen-whisper`). `pcm_out` : y enregistre
/// aussi le son décodé (mono, 16 kHz, f32), pour Qwen3-ASR.
pub async fn transcribe(
    model: &Path,
    media: &Path,
    lang: &str,
    pcm_out: Option<&Path>,
    on_event: impl FnMut(ImportEvent),
) -> Result<Vec<WWord>> {
    // iPhone et iPad : Whisper dans l'app (whisper.rs), le son décodé par AVFoundation
    #[cfg(target_os = "ios")]
    return transcribe_inapp(model, media, lang, pcm_out, on_event).await;
    #[cfg(not(target_os = "ios"))]
    return transcribe_sidecar(model, media, lang, pcm_out, on_event).await;
}

/// Whisper par le programme à part `lumen-whisper` (Mac et Windows).
#[cfg(not(target_os = "ios"))]
async fn transcribe_sidecar(model: &Path, media: &Path, lang: &str, pcm_out: Option<&Path>, mut on_event: impl FnMut(ImportEvent)) -> Result<Vec<WWord>> {
    let bin = sidecar_path()?;
    crate::ai::cpu_check()?;
    let mut cmd = crate::proc::tokio_command(bin);
    cmd.arg(model).arg(media).arg(lang);
    if let Some(p) = pcm_out {
        cmd.arg("--pcm").arg(p);
    }
    // Windows : le processeur seul, si l'apprenant l'a choisi (Réglages › IA)
    if !crate::ai::gpu_wanted() {
        cmd.arg("--cpu");
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!(crate::i18n::t("sortie indisponible", "output unavailable")))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut result: Option<Vec<WWord>> = None;
    let mut error: Option<String> = None;
    while let Some(line) = lines.next_line().await? {
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v["type"].as_str() {
            Some("stage") => on_event(ImportEvent::Stage { stage: v["stage"].as_str().unwrap_or("").to_string() }),
            Some("progress") => on_event(ImportEvent::Progress { value: v["value"].as_f64().unwrap_or(0.0) }),
            Some("done") => {
                result = Some(serde_json::from_value(v["words"].clone()).unwrap_or_default());
            }
            Some("error") => error = Some(v["message"].as_str().unwrap_or(crate::i18n::t("erreur inconnue", "unknown error")).to_string()),
            _ => {}
        }
    }
    let _ = child.wait().await;
    if let Some(e) = error {
        return Err(anyhow!(e));
    }
    result.ok_or_else(|| anyhow!(crate::i18n::t("la transcription s'est arrêtée sans résultat", "the transcription stopped without a result")))
}

/// Whisper dans l'app (iPhone et iPad) : mêmes étapes et même résultat que `lumen-whisper`.
#[cfg(target_os = "ios")]
async fn transcribe_inapp(model: &Path, media: &Path, lang: &str, pcm_out: Option<&Path>, mut on_event: impl FnMut(ImportEvent)) -> Result<Vec<WWord>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    on_event(ImportEvent::Stage { stage: "decode".into() });
    let src = media.to_path_buf();
    let audio = tokio::task::spawn_blocking(move || crate::compress::pcm_mono(&src, 16_000.0)).await??;
    if audio.is_empty() {
        return Err(anyhow!(crate::i18n::t("le fichier ne contient pas de son exploitable", "the file contains no usable sound")));
    }
    if let Some(out) = pcm_out {
        let bytes: Vec<u8> = audio.iter().flat_map(|x| x.to_le_bytes()).collect();
        tokio::fs::write(out, bytes).await?;
    }
    on_event(ImportEvent::Stage { stage: "model".into() });
    on_event(ImportEvent::Stage { stage: "transcribe".into() });
    // le travail s'arrête si l'import est abandonné (la tâche qui attend disparaît)
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = Stop(cancel.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let (model, lang) = (model.to_path_buf(), lang.to_string());
    let mut job = tokio::task::spawn_blocking(move || crate::whisper::transcribe(&model, &audio, &lang, crate::ai::gpu_wanted(), &cancel, |p| {
        let _ = tx.send(p);
    }));
    let words = loop {
        tokio::select! {
            Some(p) = rx.recv() => on_event(ImportEvent::Progress { value: p }),
            r = &mut job => break r??,
        }
    };
    Ok(words.into_iter().map(|w| WWord { w: w.w, t0: w.t0, t1: w.t1 }).collect())
}

/// Assemble le texte et les horodatages par mot : [[début, fin, t0, t1], …]
/// (positions en unités UTF-16 dans le texte final).
pub fn build_transcript(words: &[WWord]) -> (String, String) {
    let mut text = String::new();
    let mut len16 = 0usize;
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let mut prev_end = 0.0f64;
    for (i, w) in words.iter().enumerate() {
        let raw = w.w.replace('\n', " ");
        let word = raw.trim();
        if word.is_empty() {
            continue;
        }
        let glue_left = raw.starts_with(' ') || i == 0;
        let ends_sentence = text.trim_end().ends_with(['.', '!', '?', '…', '。', '！', '？']);
        if !text.is_empty() {
            if w.t0 - prev_end > 1.6 && ends_sentence {
                text.push_str("\n\n");
                len16 += 2;
            } else if glue_left {
                text.push(' ');
                len16 += 1;
            }
        }
        let s = len16;
        text.push_str(word);
        len16 += word.encode_utf16().count();
        timings.push([s as f64, len16 as f64, (w.t0 * 100.0).round() / 100.0, (w.t1 * 100.0).round() / 100.0]);
        prev_end = w.t1;
    }
    (text, serde_json::to_string(&timings).unwrap_or_else(|_| "[]".into()))
}

/// Qui écrit le texte d'une transcription, plus juste que Whisper (qui garde
/// le minutage des mots).
pub enum Writer {
    /// Qwen3-ASR par llama.cpp : modèle et partie audio
    Qwen { model: PathBuf, mmproj: PathBuf },
    /// Parakeet par le moteur sherpa-onnx : dossier du modèle
    Parakeet { data_dir: PathBuf, model: PathBuf },
}

impl Writer {
    fn name(&self) -> &'static str {
        match self {
            Writer::Qwen { .. } => "Qwen3-ASR",
            Writer::Parakeet { .. } => "Parakeet",
        }
    }
}

/// Texte et horodatages d'une leçon à partir d'un son. Whisper repère chaque
/// mot dans le temps (et décode le son) ; si un `writer` est fourni (Qwen3-ASR
/// ou Parakeet), il écrit le texte, plus juste, sur lequel le minutage de
/// Whisper est recalé. Le texte est ensuite aéré comme sur LingQ ; avec `voices`
/// (speakers.rs), chaque réplique d'une conversation y a son paragraphe.
/// `reference` : un texte déjà écrit (article, transcription officielle d'une
/// page), gardé tel quel si le son le suit ; sinon, transcription comme d'habitude.
#[allow(clippy::too_many_arguments)]
pub async fn lesson_transcript(
    engine: &crate::ai::Engine,
    whisper: &Path,
    writer: Option<Writer>,
    voices: Option<crate::speakers::Model>,
    media: &Path,
    lang: &str,
    reference: Option<&str>,
    mut on_event: impl FnMut(ImportEvent),
) -> Result<(String, String)> {
    let pcm = (writer.is_some() || voices.is_some()).then(|| std::env::temp_dir().join(format!("lumen-{}.pcm", new_stem())));
    let heard = transcribe(whisper, media, lang, pcm.as_deref(), |e| {
        // avec Qwen3-ASR ou Parakeet, Whisper ne fait plus que repérer les mots dans le temps
        let e = match e {
            ImportEvent::Stage { stage } if stage == "transcribe" && writer.is_some() => ImportEvent::Stage { stage: "timing".into() },
            e => e,
        };
        on_event(e);
    })
    .await;
    let words = match heard {
        Ok(w) => collapse_word_loops(w, lang),
        Err(e) => {
            if let Some(p) = &pcm {
                let _ = std::fs::remove_file(p);
            }
            return Err(e);
        }
    };
    if let Some(r) = reference {
        let (r_timings, found) = align_timings(r, lang, &words);
        // même garde-fou que pour Qwen3-ASR : la moitié des mots au moins doit s'entendre
        if found >= 0.5 {
            if let Some(p) = &pcm {
                let _ = std::fs::remove_file(p);
            }
            return Ok((r.to_string(), r_timings));
        }
        eprintln!("texte de la page écarté : {:.0} % des mots retrouvés", found * 100.0);
    }
    // le son décodé par Whisper : pour le modèle qui écrit et pour reconnaître les voix
    let samples = pcm.as_ref().and_then(|p| {
        let s = read_pcm(p);
        let _ = std::fs::remove_file(p);
        s.map_err(|e| eprintln!("son décodé illisible : {e}")).ok()
    });
    Ok(lesson_text(engine, writer.as_ref(), voices.as_ref(), samples.as_deref(), &words, lang, Cut::Sentences, on_event).await)
}

/// Le texte d'une leçon et ses horodatages, d'après les mots entendus par Whisper
/// et le son décodé (`samples`, s'il y a un modèle qui écrit ou qui reconnaît les
/// voix) : la suite de `lesson_transcript`, une fois Whisper passé.
#[allow(clippy::too_many_arguments)]
async fn lesson_text(
    engine: &crate::ai::Engine,
    writer: Option<&Writer>,
    voices: Option<&crate::speakers::Model>,
    samples: Option<&[f32]>,
    words: &[WWord],
    lang: &str,
    cut: Cut,
    mut on_event: impl FnMut(ImportEvent),
) -> (String, String) {
    let (mut text, mut timings) = build_transcript(words);
    if lang == "zh" {
        text = full_width(&text);
    }
    if text::unspaced(lang) {
        // les fragments de Whisper ne sont pas des mots : un instant par mot de la leçon
        let (t, found) = align_timings(&text, lang, words);
        if found > 0.0 {
            timings = t;
        }
    }
    let mut turns = Vec::new();
    if let Some(samples) = samples {
        // les voix s'écoutent pendant que le texte s'écrit (sur iPhone, après : speakers.rs)
        let listening = voices.and_then(|v| crate::speakers::start(v, samples).map_err(|e| eprintln!("interlocuteurs : {e}")).ok());
        if let Some(writer) = writer {
            on_event(ImportEvent::Stage { stage: "text".into() });
            let written = written_text(engine, writer, samples, words, lang, cut, |p| on_event(ImportEvent::Progress { value: p })).await;
            match written {
                Ok(t) if !t.trim().is_empty() => {
                    // Qwen3-ASR oublie parfois de ponctuer la parole spontanée : Whisper comble
                    let t = borrow_punctuation(&collapse_loops(&t, lang), lang, words);
                    let (t_timings, found) = align_timings(&t, lang, words);
                    // garde-fou : un texte qui ne ressemble pas à ce qu'a entendu Whisper est écarté
                    if found >= 0.5 {
                        text = t;
                        timings = t_timings;
                    } else {
                        eprintln!("{} écarté : {:.0} % des mots retrouvés", writer.name(), found * 100.0);
                    }
                }
                Ok(_) => {}
                // Whisper seul plutôt qu'un échec : la leçon se crée quand même
                Err(e) => eprintln!("{} indisponible : {e}", writer.name()),
            }
        }
        if let Some(listening) = listening {
            on_event(ImportEvent::Stage { stage: "voices".into() });
            // sans les voix, la leçon se crée quand même : une phrase par paragraphe, comme avant
            match listening.finish(samples, |p| on_event(ImportEvent::Progress { value: p })).await {
                Ok(t) => turns = t,
                Err(e) => eprintln!("interlocuteurs : {e}"),
            }
        }
    }
    let openings = crate::speakers::openings(&text, lang, &timings, &turns);
    airy_turns(&text, lang, &timings, &openings)
}

/// Où couper le son confié au modèle qui écrit.
#[derive(Clone, Copy, PartialEq)]
pub enum Cut {
    /// aux fins de phrase entendues par Whisper, jamais dans un mot
    Sentences,
    /// au silence le plus net (l'ancienne découpe, gardée pour comparer)
    #[cfg_attr(not(test), allow(dead_code))]
    Silence,
}

/// Le texte d'un son par le modèle qui écrit : le son est coupé en morceaux (aux
/// fins de phrase de Whisper), chacun est écrit à part, puis les jointures sont
/// reprises (`join_pieces`).
async fn written_text(
    engine: &crate::ai::Engine,
    writer: &Writer,
    samples: &[f32],
    words: &[WWord],
    lang: &str,
    cut: Cut,
    mut on_progress: impl FnMut(f64),
) -> Result<String> {
    let target = match writer {
        Writer::Qwen { .. } => crate::asr::CHUNK_SECS,
        Writer::Parakeet { .. } => crate::parakeet::CHUNK_SECS,
    };
    let pieces = match cut {
        Cut::Sentences => split_at_words(samples, words, target),
        Cut::Silence => crate::asr::split_on_silence(samples, target).into_iter().map(|(a, b)| Piece { a, b, end: None, next: None }).collect(),
    };
    let ranges: Vec<(usize, usize)> = pieces.iter().map(|p| (p.a, p.b)).collect();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let texts = match writer {
        Writer::Qwen { model, mmproj } => tokio::task::block_in_place(|| crate::asr::transcribe(engine, model, mmproj, samples, &ranges, lang, &stop, &mut on_progress))?,
        Writer::Parakeet { data_dir, model } => crate::parakeet::transcribe(data_dir, model, samples, &ranges, &stop, &mut on_progress).await?,
    };
    Ok(join_pieces(&texts, &pieces, lang))
}

// ---------- découpe du son pour le modèle qui écrit ----------

/// Un morceau de son confié au modèle qui écrit, et ce que Whisper entend à sa fin.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// échantillons [a, b)
    pub a: usize,
    pub b: usize,
    /// ponctuation de Whisper après le dernier mot du morceau (vide : la phrase
    /// continue dans le morceau suivant) ; `None` sans repère (coupure dans un silence)
    pub end: Option<String>,
    /// premier mot du morceau suivant, tel que Whisper l'écrit
    pub next: Option<String>,
}

/// Fin de phrase dans la ponctuation d'un mot de Whisper.
fn full_stop(p: &str) -> bool {
    p.contains(['.', '!', '?', '…', '。', '！', '？', '।', '؟'])
}

/// Mot entendu par Whisper, avec la ponctuation qui le suit (en japonais et en
/// chinois, elle arrive en fragments à part).
struct Spoken {
    word: String,
    t0: f64,
    t1: f64,
    punct: String,
}

fn spoken(words: &[WWord]) -> Vec<Spoken> {
    let mut out: Vec<Spoken> = Vec::new();
    for w in words {
        let t = w.w.trim();
        if t.chars().any(char::is_alphanumeric) {
            out.push(Spoken { word: t.trim_end_matches(PUNCT).to_string(), t0: w.t0, t1: w.t1.max(w.t0), punct: trailing_punct(t).to_string() });
        } else if let Some(last) = out.last_mut() {
            last.punct.push_str(t);
        }
    }
    out
}

/// Découpe le son en morceaux d'environ `target` secondes (de la moitié à 5 s de
/// plus), chaque coupure tombant entre deux mots entendus par Whisper : de
/// préférence à une fin de phrase, sinon à la plus longue pause. Le modèle qui écrit
/// rend chaque morceau comme un texte fermé (un point, puis une majuscule) : coupé
/// au silence le plus net, une coupure sur six tombait au milieu d'une phrase, une
/// sur dix dans un mot. Sans mot à cet endroit (musique, silence), le silence le
/// plus net, comme avant. Les morceaux se suivent sans trou ni chevauchement.
pub fn split_at_words(pcm: &[f32], words: &[WWord], target: f64) -> Vec<Piece> {
    use crate::asr::{quietest, RATE};
    let rate = RATE as f64;
    let total = pcm.len();
    let at = |t: f64| ((t.max(0.0) * rate) as usize).min(total);
    let s = spoken(words);
    let (min_len, max_len) = (target * 0.5, target + 5.0);
    let mut out = Vec::new();
    let mut start = 0usize;
    // premier mot qui peut encore finir un morceau
    let mut k = 0usize;
    while (total - start) as f64 / rate > max_len {
        let (lo, hi) = (start as f64 / rate + min_len, start as f64 / rate + max_len);
        let mut best: Option<(f64, usize)> = None;
        for i in k..s.len().saturating_sub(1) {
            // l'intervalle entre le mot i et le suivant
            let (g0, g1) = (s[i].t1, s[i + 1].t0.max(s[i].t1));
            if g1 < lo {
                continue;
            }
            if g0 > hi {
                break;
            }
            let p = &s[i].punct;
            let mark = if full_stop(p) { 3.0 } else if !p.is_empty() { 0.5 } else { 0.0 };
            // à égalité, le morceau le plus long : plus de contexte pour le modèle
            let late = 0.25 * (((g0 + g1) / 2.0).clamp(lo, hi) - lo) / (hi - lo);
            let score = mark + (g1 - g0).min(1.5) + late;
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, i));
            }
        }
        let (boundary, end, next) = match best {
            Some((_, i)) => {
                let (g0, g1) = (s[i].t1.max(lo), s[i + 1].t0.max(s[i].t1).min(hi));
                let cut = if g1 - g0 >= 0.1 {
                    // une vraie pause : son moment le plus calme
                    quietest(pcm, at(g0), at(g1), RATE / 10)
                } else {
                    // parole continue : le creux le plus proche de la frontière des deux mots
                    let c = ((g0 + g1) / 2.0).clamp(lo, hi);
                    quietest(pcm, at(c - 0.12), at(c + 0.12), RATE / 25)
                };
                k = i + 1;
                (cut, Some(s[i].punct.clone()), Some(s[i + 1].word.clone()))
            }
            None => {
                let c = start as f64 / rate + target;
                (quietest(pcm, at(c - 5.0).max(start), at(c + 5.0), RATE / 10), None, None)
            }
        };
        let boundary = boundary.clamp(start + 1, total);
        out.push(Piece { a: start, b: boundary, end, next });
        start = boundary;
    }
    if start < total {
        out.push(Piece { a: start, b: total, end: None, next: None });
    }
    out
}

/// Premier mot d'un texte : sa place (octets) et lui-même, sans ce qui le précède
/// (guillemet, tiret, « ¿ ») ni la ponctuation qui le suit.
fn first_word(text: &str) -> Option<(usize, &str)> {
    let a = text.find(char::is_alphanumeric)?;
    let len = text[a..].find(|c: char| c.is_whitespace()).unwrap_or(text.len() - a);
    Some((a, text[a..a + len].trim_end_matches(PUNCT)))
}

/// Met bout à bout les textes des morceaux. Le modèle écrit chaque morceau comme un
/// texte fermé : un point à la fin, une majuscule au début, même quand la phrase
/// continue. Aux jointures, c'est Whisper, qui a entendu la phrase d'un seul tenant,
/// qui dit si elle se termine : fin de phrase entendue, le point et la majuscule y
/// sont ; phrase qui continue (Whisper écrit le mot suivant, le même, en minuscule),
/// le point ajouté s'en va (ou devient la virgule de Whisper) et la majuscule aussi.
/// Dans le doute (mots différents, majuscule chez Whisper), rien ne change.
pub fn join_pieces(texts: &[String], pieces: &[Piece], lang: &str) -> String {
    let unspaced = text::unspaced(lang);
    let mut out = String::new();
    // ce que Whisper entend à la fin du dernier morceau écrit
    let mut seam: Option<(&str, &str)> = None;
    for (t, piece) in texts.iter().zip(pieces) {
        let t = t.trim();
        let here = piece.end.as_deref().zip(piece.next.as_deref());
        if t.is_empty() {
            // rien d'écrit pour ce morceau : la jointure n'a plus de repère
            seam = None;
            continue;
        }
        let mut t = t.to_string();
        if let (false, Some((punct, next))) = (out.is_empty(), seam) {
            let ended = full_stop(trailing_punct(&out));
            let first = first_word(&t).map(|(a, w)| (a, w.to_string()));
            if full_stop(punct) {
                if !ended {
                    let kept = out.trim_end_matches(PUNCT).len();
                    out.truncate(kept);
                    out.push_str(punct);
                }
                if let Some((a, w)) = &first {
                    t.replace_range(*a..*a + w.len(), &capitalized(w, lang));
                }
            } else if let Some((a, w)) = &first {
                let same = if unspaced {
                    w.chars().next() == next.chars().next()
                } else {
                    text::normalize_for(w, lang) == text::normalize_for(next, lang)
                };
                let c = w.chars().next().unwrap_or(' ');
                // Whisper écrit la suite en minuscule : la phrase continue (sans casse : même mot suffit)
                let goes_on = same && next.chars().next().is_some_and(|n| n.is_lowercase() || !n.is_uppercase());
                let dot = out.ends_with(['.', '。']) && !out.ends_with("..");
                if goes_on && (dot || !ended) {
                    if dot {
                        out.pop();
                        out.push_str(punct);
                    }
                    let low: Vec<char> = c.to_lowercase().collect();
                    if c.is_uppercase() && low.len() == 1 && !(lang == "tr" && c == 'I') {
                        t.replace_range(*a..*a + c.len_utf8(), &low[0].to_string());
                    }
                }
            }
        }
        if !out.is_empty() && !unspaced {
            out.push(' ');
        }
        out.push_str(&t);
        seam = here;
    }
    out
}

/// Chinois : Whisper ponctue à l'occidentale (« , » « ? ») ; après un idéogramme, la
/// ponctuation pleine chasse (« ， » « ？ »), de même longueur en UTF-16 : les positions
/// des mots ne bougent pas.
fn full_width(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut prev = ' ';
    for c in text.chars() {
        let w = match c {
            ',' if text::is_han(prev) => '，',
            '?' if text::is_han(prev) => '？',
            '!' if text::is_han(prev) => '！',
            ':' if text::is_han(prev) => '：',
            ';' if text::is_han(prev) => '；',
            _ => c,
        };
        out.push(w);
        prev = c;
    }
    out
}

/// Son décodé par `lumen-whisper --pcm` (mono, 16 kHz, f32 petit-boutiste).
pub fn read_pcm(path: &Path) -> Result<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

// ---------- mise en page aérée, comme LingQ ----------

/// Une phrase d'au plus deux mots (« Sì. ») rejoint la suivante.
const PARA_MIN_WORDS: usize = 3;
/// Au-delà, une phrase (parole spontanée peu ponctuée) est coupée en morceaux.
const PARA_MAX_WORDS: usize = 55;
/// Taille minimale d'un morceau de phrase coupée.
const PARA_MIN_PART: usize = 6;

/// Met une phrase par paragraphe, comme LingQ : le texte respire. Les phrases
/// d'un ou deux mots rejoignent la suivante ; les très longues sont coupées à
/// leur plus longue pause (à défaut après une virgule). Les horodatages
/// [[début, fin, t0, t1], …] suivent le texte (positions UTF-16).
pub fn airy(text: &str, lang: &str, timings: &str) -> (String, String) {
    airy_turns(text, lang, timings, &[])
}

/// La mise en page aérée d'une conversation. `openings` : les mots (position UTF-16)
/// qui ouvrent une réplique, quand une autre voix prend la parole
/// (`speakers::openings`). Un paragraphe commence toujours là, même au milieu d'une
/// phrase (parole coupée), et une réponse courte (« Sì. ») ne rejoint jamais la
/// phrase d'un autre.
pub fn airy_turns(text: &str, lang: &str, timings: &str, openings: &[usize]) -> (String, String) {
    let times: Vec<[f64; 4]> = serde_json::from_str(timings).unwrap_or_default();
    let words: Vec<text::Token> = text::tokenize(text, lang).into_iter().filter(|t| t.w).collect();
    let n = words.len();
    if n < 2 {
        return (text.trim().to_string(), timings.to_string());
    }
    let byte_at = byte_positions(text);
    let byte = |p: usize| byte_at[p.min(byte_at.len() - 1)];
    let when: HashMap<usize, (f64, f64)> = times.iter().map(|t| (t[0] as usize, (t[2], t[3]))).collect();
    let wt: Vec<Option<(f64, f64)>> = words.iter().map(|w| when.get(&w.s).copied()).collect();
    // ce qui sépare le mot k du suivant : espaces et ponctuation
    let gap = |k: usize| &text[byte(words[k].e)..byte(words[k + 1].s)];
    // fin de phrase après le mot k : où elle s'arrête dans ce qui le sépare du suivant
    let stop = |k: usize| sentence_stop(gap(k), &text[byte(words[k + 1].s)..byte(words[k + 1].e)]);

    // le mot k ouvre une réplique : une autre voix prend la parole
    let opens: std::collections::HashSet<usize> = (1..n).filter(|&k| openings.contains(&words[k].s)).collect();

    // 1. phrases (un saut de paragraphe existant compte aussi, et un changement de voix)
    let mut sentences: Vec<(usize, usize)> = Vec::new();
    let mut a = 0;
    for k in 0..n - 1 {
        let g = gap(k);
        if stop(k).is_some() || g.contains('\n') || opens.contains(&(k + 1)) {
            sentences.push((a, k));
            a = k + 1;
        }
    }
    sentences.push((a, n - 1));
    // 2. les phrases trop courtes rejoignent la suivante (la dernière, la précédente),
    // jamais celle d'une autre voix
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut carry: Option<usize> = None;
    for (a, b) in sentences {
        if let Some(c) = carry.filter(|_| opens.contains(&a)) {
            merged.push((c, a - 1));
            carry = None;
        }
        let a = carry.take().unwrap_or(a);
        if b + 1 - a < PARA_MIN_WORDS {
            carry = Some(a);
        } else {
            merged.push((a, b));
        }
    }
    if let Some(a) = carry {
        match merged.last_mut().filter(|_| !opens.contains(&a)) {
            Some(last) => last.1 = n - 1,
            None => merged.push((a, n - 1)),
        }
    }
    // 3. les phrases trop longues sont coupées
    let score = |k: usize| {
        let pause = match (wt[k], wt[k + 1]) {
            (Some(x), Some(y)) => (y.0 - x.1).max(0.0),
            _ => 0.0,
        };
        let comma = if gap(k).contains([',', ';', ':', '–', '—', '，', '、', '；', '：']) { 0.25 } else { 0.0 };
        pause + comma
    };
    let mut paras: Vec<(usize, usize)> = Vec::new();
    let mut todo: Vec<(usize, usize)> = merged.into_iter().rev().collect();
    while let Some((a, b)) = todo.pop() {
        if b + 1 - a <= PARA_MAX_WORDS {
            paras.push((a, b));
            continue;
        }
        // la meilleure coupure ; à égalité, la plus proche du milieu
        let mid = (a + b) as f64 / 2.0;
        let k = (a + PARA_MIN_PART - 1..=b - PARA_MIN_PART)
            .max_by(|&x, &y| (score(x), -(x as f64 - mid).abs()).partial_cmp(&(score(y), -(y as f64 - mid).abs())).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((a + b) / 2);
        todo.push((k + 1, b));
        todo.push((a, k));
    }
    let breaks: std::collections::HashSet<usize> = paras.iter().map(|p| p.1).filter(|&b| b + 1 < n).collect();

    // 4. le texte, recomposé, et la nouvelle place de chaque mot
    let mut out = Utf16Text::default();
    out.push(text[..byte(words[0].s)].trim_start());
    let mut place: HashMap<usize, (usize, usize)> = HashMap::new();
    for k in 0..n {
        let start = out.len16;
        out.push(&text[byte(words[k].s)..byte(words[k].e)]);
        place.insert(words[k].s, (start, out.len16));
        if k + 1 == n {
            break;
        }
        let g = gap(k);
        if breaks.contains(&k) {
            // ce qui termine la phrase reste avec elle (ponctuation, et un nombre : « Formula 1. »),
            // saut de paragraphe, puis ce qui ouvre la phrase suivante
            let rest = stop(k).map_or(0, |i| i);
            match g[rest..].find(char::is_whitespace).map(|i| i + rest) {
                Some(i) => {
                    out.push(&g[..i]);
                    out.push("\n\n");
                    out.push(g[i..].trim_start());
                }
                None => {
                    out.push(g);
                    out.push("\n\n");
                }
            }
        } else if g.contains('\n') {
            // ancien saut de paragraphe, devenu inutile : une simple espace (aucune en
            // japonais et en chinois)
            let sep = if text::unspaced(lang) { "" } else { " " };
            let joined = g.split_whitespace().collect::<Vec<_>>().join(sep);
            out.push(&joined);
            if g.ends_with(char::is_whitespace) {
                out.push(sep);
            }
        } else {
            out.push(g);
        }
    }
    out.push(text[byte(words[n - 1].e)..].trim_end());
    let moved: Vec<[f64; 4]> = times
        .iter()
        .filter_map(|t| place.get(&(t[0] as usize)).map(|&(s, e)| [s as f64, e as f64, t[2], t[3]]))
        .collect();
    (out.text, serde_json::to_string(&moved).unwrap_or_else(|_| "[]".into()))
}

/// Fin de phrase dans ce qui sépare deux mots : la position (en octets) juste après
/// elle, guillemet fermant compris. Le point d'un nombre (« 7.000 », « 3.5 ») n'en
/// est pas une ; des points de suspension devant une minuscule non plus (hésitation :
/// « Io… penso »). `next` : le mot qui suit.
pub(crate) fn sentence_stop(g: &str, next: &str) -> Option<usize> {
    let cs: Vec<(usize, char)> = g.char_indices().collect();
    let goes_on = next.chars().next().is_some_and(char::is_lowercase);
    let mut stop = None;
    for (n, &(i, c)) in cs.iter().enumerate() {
        if !matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            continue;
        }
        let prev = n.checked_sub(1).map(|p| cs[p].1);
        let after = cs.get(n + 1).map(|x| x.1);
        let digit = |x: Option<char>| x.is_some_and(|d| d.is_ascii_digit());
        if c == '.' && digit(prev) && digit(after) {
            continue;
        }
        let dots = c == '…' || (c == '.' && (prev == Some('.') || after == Some('.')));
        if dots && goes_on {
            continue;
        }
        stop = Some(i + c.len_utf8());
    }
    let mut stop = stop?;
    while let Some(c) = g[stop..].chars().next().filter(|c| "»”’)]」』".contains(*c)) {
        stop += c.len_utf8();
    }
    Some(stop)
}

/// Texte en construction, avec sa longueur en unités UTF-16 (celles de l'interface).
#[derive(Default)]
struct Utf16Text {
    text: String,
    len16: usize,
}

impl Utf16Text {
    fn push(&mut self, s: &str) {
        self.text.push_str(s);
        self.len16 += s.encode_utf16().count();
    }
}

/// Position en octets de chaque unité UTF-16 du texte (et de sa fin).
pub(crate) fn byte_positions(text: &str) -> Vec<usize> {
    let mut byte_at: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        byte_at.extend(std::iter::repeat_n(i, c.len_utf16()));
    }
    byte_at.push(text.len());
    byte_at
}

// ---------- boucles ----------

/// Répétitions en boucle d'une transcription (« oh oh oh… » des centaines de
/// fois sur une musique de fin) : une suite de 1 à 8 mots redite au moins 5 fois
/// de suite (8 pour un seul mot). Renvoie les intervalles [début, fin) à
/// retirer : tout sauf la première occurrence.
fn loops(keys: &[String]) -> Vec<(usize, usize)> {
    let n = keys.len();
    let mut out = Vec::new();
    let mut i = 0;
    'scan: while i < n {
        for k in 1..=8 {
            let need = if k == 1 { 8 } else { 5 };
            if i + k * need > n {
                continue;
            }
            let mut r = 1;
            while i + (r + 1) * k <= n && keys[i + r * k..i + (r + 1) * k] == keys[i..i + k] {
                r += 1;
            }
            if r >= need {
                out.push((i + k, i + r * k));
                i += r * k;
                continue 'scan;
            }
        }
        i += 1;
    }
    out
}

/// Le texte sans ses boucles (voir `loops`) : de la fin de la première
/// occurrence à la fin de la dernière, tout disparaît.
pub fn collapse_loops(text: &str, lang: &str) -> String {
    let toks: Vec<text::Token> = text::tokenize(text, lang).into_iter().filter(|t| t.w).collect();
    let cut = loops(&toks.iter().map(|t| t.k.clone()).collect::<Vec<_>>());
    if cut.is_empty() {
        return text.to_string();
    }
    let byte_at = byte_positions(text);
    let byte = |p: usize| byte_at[p.min(byte_at.len() - 1)];
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for (a, b) in cut {
        out.push_str(&text[from..byte(toks[a - 1].e)]);
        from = byte(toks[b - 1].e);
    }
    out.push_str(&text[from..]);
    out
}

/// Les mots entendus par Whisper, sans leurs boucles.
pub fn collapse_word_loops(words: Vec<WWord>, lang: &str) -> Vec<WWord> {
    let keys: Vec<String> = words.iter().map(|w| text::word_keys(w.w.trim(), lang).join(" ")).collect();
    let cut = loops(&keys);
    if cut.is_empty() {
        return words;
    }
    words.into_iter().enumerate().filter(|(i, _)| !cut.iter().any(|&(a, b)| (a..b).contains(i))).map(|(_, w)| w).collect()
}

// ---------- ponctuation oubliée ----------

/// Au-delà, une suite de mots sans fin de phrase est une ponctuation oubliée :
/// Qwen3-ASR écrit parfois la parole spontanée sans un point pendant des
/// minutes (509 mots d'affilée sur une conversation d'Easy Italian), là où
/// Whisper ponctue (38 mots au plus pour la même vidéo).
pub const RUN_ON_WORDS: usize = 40;

/// Ponctuation que Whisper prête à la transcription.
const PUNCT: &[char] = &['.', ',', ';', ':', '!', '?', '…', '。', '，', '、', '！', '？', '।', '؟', '،'];

/// Ponctuation de fin de phrase (sans le retour à la ligne de `text::ends_sentence`).
fn closes_sentence(g: &str) -> bool {
    g.contains(['.', '!', '?', '…', ';', '。', '！', '？', '।', '؟'])
}

/// Saut de paragraphe : la mise en page aérée en met au milieu des phrases trop longues.
fn paragraph_break(g: &str) -> bool {
    g.matches('\n').count() >= 2
}

/// Ce qui sépare deux mots termine-t-il une suite ? Une fin de phrase, ou un
/// simple retour à la ligne (vers d'une chanson, titre) ; un saut de paragraphe, non.
fn ends_run(g: &str) -> bool {
    closes_sentence(g) || g.matches('\n').count() == 1
}

/// Plus longue suite de mots sans fin de phrase (miroir de `longestRun` dans tokenize.ts).
pub fn longest_run(text: &str, lang: &str) -> usize {
    let (mut best, mut run, mut gap) = (0, 0, String::new());
    for t in text::tokenize(text, lang) {
        if t.w {
            if run > 0 && ends_run(&gap) {
                best = best.max(run);
                run = 0;
            }
            run += 1;
            gap.clear();
        } else {
            gap.push_str(&t.t);
        }
    }
    best.max(run)
}

/// Ponctuation qui termine un mot de Whisper (« facile. » → « . »).
fn trailing_punct(w: &str) -> &str {
    let w = w.trim_end();
    &w[w.trim_end_matches(PUNCT).len()..]
}

/// Le mot avec sa majuscule, si sa clé reste la même (le vocabulaire n'en voit
/// rien) ; en turc, « i » se garde de devenir un « I » sans point.
fn capitalized(w: &str, lang: &str) -> String {
    let mut cs = w.chars();
    let Some(c) = cs.next() else { return String::new() };
    if !c.is_lowercase() || (lang == "tr" && c == 'i') {
        return w.to_string();
    }
    let up: String = c.to_uppercase().chain(cs).collect();
    if text::normalize_for(&up, lang) == text::normalize_for(w, lang) {
        up
    } else {
        w.to_string()
    }
}

/// Reprend la ponctuation de Whisper dans les longs passages qu'une
/// transcription a laissés sans fin de phrase : aux mêmes mots (alignement de
/// `align_keys`), ses points, virgules et points d'interrogation, et la
/// majuscule de la phrase qui suit. Les mots et leurs clés ne changent pas ;
/// ailleurs, le texte reste tel quel. Dans ces passages, un saut de paragraphe
/// (mise en page aérée d'une phrase sans fin) redevient une espace.
pub fn borrow_punctuation(text: &str, lang: &str, words: &[WWord]) -> String {
    let toks: Vec<text::Token> = text::tokenize(text, lang).into_iter().filter(|t| t.w).collect();
    let n = toks.len();
    if n < 2 {
        return text.to_string();
    }
    let byte_at = byte_positions(text);
    let byte = |p: usize| byte_at[p.min(byte_at.len() - 1)];
    // ce qui suit le mot k : jusqu'au mot suivant, ou la fin du texte
    let gap = |k: usize| if k + 1 < n { &text[byte(toks[k].e)..byte(toks[k + 1].s)] } else { &text[byte(toks[k].e)..] };
    let mut long = vec![false; n];
    let mut a = 0;
    for k in 0..n {
        if k + 1 == n || ends_run(gap(k)) {
            if k + 1 - a > RUN_ON_WORDS {
                long[a..=k].fill(true);
            }
            a = k + 1;
        }
    }
    if !long.contains(&true) {
        return text.to_string();
    }
    let heard = heard_keys(words, lang);
    let ka: Vec<String> = toks.iter().map(|t| t.k.clone()).collect();
    let kb: Vec<String> = heard.iter().map(|h| h.k.clone()).collect();
    let mut punct: Vec<&str> = vec![""; n];
    for (i, j) in align_keys(&ka, &kb) {
        if long[i] {
            punct[i] = &heard[j].punct;
        }
    }
    // saut de paragraphe au milieu d'une phrase : une simple espace
    let spaced = |g: &str| {
        if !paragraph_break(g) || closes_sentence(g) {
            return g.to_string();
        }
        let mut s = String::new();
        for c in g.chars() {
            if !c.is_whitespace() {
                s.push(c);
            } else if !s.ends_with(' ') {
                s.push(' ');
            }
        }
        s
    };
    let mut out = String::with_capacity(text.len() + n * 2);
    out.push_str(&text[..byte(toks[0].s)]);
    let mut capital = false;
    for k in 0..n {
        // une phrase commence : après la ponctuation reprise, ou en tête d'un passage
        // (Qwen3-ASR met parfois un point sans majuscule entre deux morceaux)
        capital |= long[k] && (k == 0 || closes_sentence(gap(k - 1)));
        let w = &text[byte(toks[k].s)..byte(toks[k].e)];
        out.push_str(&if capital { capitalized(w, lang) } else { w.to_string() });
        capital = false;
        let (g, p) = (gap(k), punct[k]);
        if !long[k] {
            out.push_str(g);
        } else if !p.is_empty() && g.trim().is_empty() {
            out.push_str(p);
            out.push_str(&spaced(g));
            capital = closes_sentence(p);
        } else if closes_sentence(p) && matches!(g.trim(), "," | "，" | "،") {
            // une virgule là où Whisper entend la fin d'une phrase
            out.push_str(p);
            out.push_str(&spaced(g.trim_start_matches([',', '，', '،'])));
            capital = true;
        } else {
            out.push_str(&spaced(g));
        }
    }
    out
}

// ---------- recalage sur un texte existant ----------

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Mot entendu, redécoupé comme le texte de Lumen.
struct Heard {
    k: String,
    t0: f64,
    t1: f64,
    /// ponctuation qui suit le mot de Whisper (portée par sa dernière partie)
    punct: String,
}

/// Mots entendus, redécoupés comme le texte de Lumen (élisions comprises),
/// la durée d'un mot coupé étant répartie au prorata de la longueur.
fn heard_keys(words: &[WWord], lang: &str) -> Vec<Heard> {
    if text::unspaced(lang) {
        return heard_unspaced(words, lang);
    }
    let mut out = Vec::new();
    for w in words {
        let parts: Vec<text::Token> = text::tokenize(w.w.trim(), lang).into_iter().filter(|t| t.w).collect();
        let total: usize = parts.iter().map(|p| p.e - p.s).sum();
        let mut at = w.t0;
        for (i, p) in parts.iter().enumerate() {
            let d = if total > 0 { (w.t1 - w.t0).max(0.0) * (p.e - p.s) as f64 / total as f64 } else { 0.0 };
            let punct = if i + 1 == parts.len() { trailing_punct(&w.w).to_string() } else { String::new() };
            out.push(Heard { k: p.k.clone(), t0: at, t1: at + d, punct });
            at += d;
        }
    }
    out
}

/// Japonais, chinois : rien ne sépare les mots dans ce qu'entend Whisper (un fragment
/// peut finir au milieu d'un mot). Tout est mis bout à bout, puis redécoupé d'un seul
/// tenant comme le texte de la leçon (jieba voit la phrase entière) ; chaque caractère
/// garde l'instant de son fragment, réparti au prorata.
fn heard_unspaced(words: &[WWord], lang: &str) -> Vec<Heard> {
    let mut all = String::new();
    // instants de chaque unité UTF-16 de `all`
    let mut at: Vec<(f64, f64)> = Vec::new();
    for w in words {
        // la ponctuation ne prend pas de temps
        let n = w.w.chars().filter(|c| c.is_alphanumeric()).count().max(1);
        let d = (w.t1 - w.t0).max(0.0) / n as f64;
        let mut k = 0usize;
        for c in w.w.chars() {
            let span = (w.t0 + d * k as f64, w.t0 + d * (k + 1) as f64);
            if c.is_alphanumeric() {
                k += 1;
            }
            all.push(c);
            at.extend(std::iter::repeat_n(span, c.len_utf16()));
        }
    }
    let toks = text::tokenize(&all, lang);
    let byte_at = byte_positions(&all);
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.w {
            continue;
        }
        // ponctuation qui suit, jusqu'au mot suivant
        let next = toks[i + 1..].iter().find(|x| x.w).map_or(all.len(), |x| byte_at[x.s]);
        let gap = &all[byte_at[t.e]..next];
        out.push(Heard { k: t.k.clone(), t0: at[t.s].0, t1: at[t.e - 1].1, punct: trailing_punct(gap).to_string() });
    }
    out
}

/// Plus longue sous-suite croissante (en j) d'ancres triées par i.
fn longest_increasing(v: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; v.len()];
    for (k, &(_, j)) in v.iter().enumerate() {
        let pos = tails.partition_point(|&t| v[t].1 < j);
        if pos > 0 {
            prev[k] = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(k);
        } else {
            tails[pos] = k;
        }
    }
    let mut out = Vec::new();
    let mut k = tails.last().copied();
    while let Some(x) = k {
        out.push(v[x]);
        k = (prev[x] != usize::MAX).then_some(prev[x]);
    }
    out.reverse();
    out
}

/// Plus longue sous-suite commune entre a[a0..a1] et b[b0..b1].
fn common_into(a: &[String], b: &[String], (a0, a1): (usize, usize), (b0, b1): (usize, usize), out: &mut Vec<(usize, usize)>) {
    let (n, m) = (a1.saturating_sub(a0), b1.saturating_sub(b0));
    // trop long sans ancre : ces mots seront interpolés
    if n == 0 || m == 0 || n * m > 4_000_000 {
        return;
    }
    let at = |i: usize, j: usize| i * (m + 1) + j;
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if a[a0 + i] == b[b0 + j] { dp[at(i + 1, j + 1)] + 1 } else { dp[at(i + 1, j)].max(dp[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[a0 + i] == b[b0 + j] {
            out.push((a0 + i, b0 + j));
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// Couples (i, j) croissants de mots identiques entre le texte (a) et ce qui
/// est entendu (b) : des ancres sûres d'abord (suites de trois mots présentes
/// une seule fois de chaque côté), puis l'alignement fin entre deux ancres.
pub fn align_keys(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    fn grams(v: &[String]) -> HashMap<(&str, &str, &str), (u32, usize)> {
        let mut m = HashMap::new();
        for i in 0..v.len().saturating_sub(2) {
            let e = m.entry((v[i].as_str(), v[i + 1].as_str(), v[i + 2].as_str())).or_insert((0, i));
            e.0 += 1;
        }
        m
    }
    let (ga, gb) = (grams(a), grams(b));
    let mut anchors: Vec<(usize, usize)> = ga
        .iter()
        .filter_map(|(k, &(n, i))| {
            let &(nb, j) = gb.get(k)?;
            (n == 1 && nb == 1).then_some((i, j))
        })
        .collect();
    anchors.sort_unstable();
    let mut out = Vec::new();
    let (mut pa, mut pb) = (0, 0);
    for (i, j) in longest_increasing(&anchors) {
        if i < pa || j < pb {
            continue; // chevauche l'ancre précédente
        }
        common_into(a, b, (pa, i), (pb, j), &mut out);
        out.extend((0..3).map(|k| (i + k, j + k)));
        pa = i + 3;
        pb = j + 3;
    }
    common_into(a, b, (pa, a.len()), (pb, b.len()), &mut out);
    out
}

/// Recale les horodatages d'un texte existant sur les mots entendus : le
/// texte ne change pas, seuls les instants sont recalculés. Renvoie
/// [[début, fin, t0, t1], …] (positions UTF-16) et la part de mots retrouvés.
pub fn align_timings(text_in: &str, lang: &str, words: &[WWord]) -> (String, f64) {
    let toks: Vec<text::Token> = text::tokenize(text_in, lang).into_iter().filter(|t| t.w).collect();
    let heard = heard_keys(words, lang);
    let a: Vec<String> = toks.iter().map(|t| t.k.clone()).collect();
    let b: Vec<String> = heard.iter().map(|h| h.k.clone()).collect();
    let pairs = align_keys(&a, &b);
    let mut times: Vec<Option<(f64, f64)>> = vec![None; toks.len()];
    for &(i, j) in &pairs {
        times[i] = Some((heard[j].t0, heard[j].t1));
    }
    // mots non retrouvés entre deux mots retrouvés : temps réparti au prorata de la longueur
    let mut i = 0;
    while i < toks.len() {
        if times[i].is_some() {
            i += 1;
            continue;
        }
        let start = i;
        while i < toks.len() && times[i].is_none() {
            i += 1;
        }
        let (Some(Some(before)), Some(Some(after))) = (start.checked_sub(1).map(|k| times[k]), times.get(i).copied()) else {
            continue; // début ou fin sans repère : pas de lanterne
        };
        let from = before.1.max(before.0);
        let to = after.0.max(from);
        let total: usize = toks[start..i].iter().map(|t| t.e - t.s + 1).sum();
        let mut at = from;
        for k in start..i {
            let d = (to - from) * (toks[k].e - toks[k].s + 1) as f64 / total as f64;
            times[k] = Some((at, at + d));
            at += d;
        }
    }
    let mut out: Vec<[f64; 4]> = Vec::new();
    let mut last = 0.0f64;
    for (t, tm) in toks.iter().zip(&times) {
        if let Some((t0, t1)) = tm {
            let t0 = t0.max(last);
            last = t0;
            out.push([t.s as f64, t.e as f64, round2(t0), round2(t1.max(t0))]);
        }
    }
    let found = if toks.is_empty() { 0.0 } else { pairs.len() as f64 / toks.len() as f64 };
    (serde_json::to_string(&out).unwrap_or_else(|_| "[]".into()), found)
}

/// Lance yt-dlp et renvoie les lignes de sortie utiles. La progression
/// (« LUMEN 42.0% ») est transmise au fur et à mesure.
async fn run_ytdlp(
    data_dir: &Path,
    ytdlp: &Path,
    args: &[String],
    cookies_browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<Vec<String>> {
    // iPhone et iPad : pas d'autre programme ; Lumen parle lui-même à YouTube (ytnative.rs)
    if crate::ytnative::is_on() {
        let _ = (data_dir, ytdlp, cookies_browser);
        return crate::ytnative::run(args, on_progress).await;
    }
    let mut cmd = crate::proc::tokio_command(ytdlp);
    cmd.args(["--no-playlist", "--newline", "--progress", "--no-colors"]);
    cmd.args(["--progress-template", "download:LUMEN %(progress._percent_str)s"]);
    if let Some(js) = crate::tools::js_runtime_arg(data_dir) {
        cmd.args(["--js-runtimes", &js]);
    }
    if let Some(b) = cookies_browser {
        cmd.args(["--cookies-from-browser", b]);
    }
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!(crate::i18n::t("sortie indisponible", "output unavailable")))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!(crate::i18n::t("sortie indisponible", "output unavailable")))?;
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut last = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            last.push(l);
            if last.len() > 30 {
                last.remove(0);
            }
        }
        last
    });
    let mut out = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(rest) = line.trim().strip_prefix("LUMEN") {
            if let Ok(v) = rest.trim().trim_end_matches('%').trim().parse::<f64>() {
                on_progress(v);
            }
        } else if !line.trim().is_empty() {
            out.push(line.trim().to_string());
        }
    }
    let status = child.wait().await?;
    let errs = err_task.await.unwrap_or_default();
    if !status.success() {
        let msg = errs.iter().rev().find(|l| l.contains("ERROR")).cloned().unwrap_or_else(|| errs.last().cloned().unwrap_or_default());
        return Err(anyhow!("{}", msg.trim_start_matches("ERROR: ").trim()));
    }
    Ok(out)
}

fn needs_cookies(e: &anyhow::Error) -> bool {
    let m = e.to_string().to_lowercase();
    m.contains("sign in") || m.contains("logged-in") || m.contains("cookies") || m.contains("not a bot") || m.contains("age")
}

async fn run_with_fallback(
    data_dir: &Path,
    ytdlp: &Path,
    args: &[String],
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<Vec<String>> {
    match run_ytdlp(data_dir, ytdlp, args, None, on_progress).await {
        Ok(v) => Ok(v),
        Err(e) if needs_cookies(&e) => match browser {
            Some(b) => run_ytdlp(data_dir, ytdlp, args, Some(b), on_progress).await,
            None => Err(anyhow!(crate::tr!(
                "Le site demande de se connecter pour cette vidéo. Choisissez votre navigateur dans Réglages › Vidéos en ligne pour que Lumen utilise votre session. ({e})",
                "The site asks you to sign in for this video. Choose your browser in Settings › Online videos so that Lumen can use your session. ({e})"
            ))),
        },
        Err(e) => Err(e),
    }
}

/// Ce que yt-dlp sait d'une adresse, sans rien télécharger. Une liste
/// (playlist, chaîne, podcast) n'est pas dépliée : ses éléments sont seulement nommés,
/// 2 000 au plus (`link::MAX_YTDLP` ; une chaîne de 2 000 vidéos se lit en 20 s).
pub async fn yt_info(data_dir: &Path, ytdlp: &Path, url: &str, browser: Option<&str>) -> Result<serde_json::Value> {
    let args: Vec<String> = vec!["-J".into(), "--flat-playlist".into(), "--playlist-items".into(), "1:2000".into(), url.into()];
    let mut quiet = |_p: f64| {};
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, &mut quiet).await?;
    let json = out
        .iter()
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!(crate::i18n::t("rien à télécharger à cette adresse", "nothing to download at this address")))?;
    Ok(serde_json::from_str(json)?)
}

/// Langue de l'interface de YouTube (`youtube:lang`) : le chinois s'y dit « zh-CN »
/// (yt-dlp refuse « zh »).
fn yt_hl(lang: &str) -> &str {
    if lang == "zh" {
        "zh-CN"
    } else {
        lang
    }
}

/// Les `max` dernières vidéos d'une chaîne (son onglet « Vidéos » : ni Shorts
/// ni directs), sans rien télécharger. `lang` : titres dans cette langue (ceux de
/// la chaîne, que YouTube traduirait sinon en anglais) ; sans elle, dates
/// approximatives (« 3 days ago », que yt-dlp ne lit qu'en anglais).
pub async fn yt_latest(data_dir: &Path, ytdlp: &Path, url: &str, max: usize, lang: Option<&str>) -> Result<serde_json::Value> {
    let mut args: Vec<String> = vec!["-J".into(), "--flat-playlist".into(), "--playlist-items".into(), format!("1:{max}")];
    match lang {
        Some(l) => args.extend(["--extractor-args".into(), format!("youtube:lang={}", yt_hl(l))]),
        None => args.extend(["--extractor-args".into(), "youtubetab:approximate_date".into()]),
    }
    args.push(url.into());
    let mut quiet = |_p: f64| {};
    let out = run_ytdlp(data_dir, ytdlp, &args, None, &mut quiet).await?;
    let json = out
        .iter()
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!(crate::i18n::t("chaîne vide", "empty channel")))?;
    Ok(serde_json::from_str(json)?)
}

/// Les éléments `from` à `to` d'une liste (résultats de recherche, playlist),
/// sans rien télécharger. `lang` : titres dans cette langue plutôt que traduits.
pub async fn yt_flat(data_dir: &Path, ytdlp: &Path, url: &str, from: usize, to: usize, lang: Option<&str>) -> Result<serde_json::Value> {
    let mut args: Vec<String> = vec!["-J".into(), "--flat-playlist".into(), "--playlist-items".into(), format!("{from}:{to}")];
    if let Some(l) = lang {
        args.extend(["--extractor-args".into(), format!("youtube:lang={}", yt_hl(l))]);
    }
    args.push(url.into());
    let mut quiet = |_p: f64| {};
    let out = run_ytdlp(data_dir, ytdlp, &args, None, &mut quiet).await?;
    let json = out.iter().find(|l| l.starts_with('{')).ok_or_else(|| anyhow!(crate::i18n::t("aucun résultat", "no results")))?;
    Ok(serde_json::from_str(json)?)
}

/// Adresses de lecture directe d'une vidéo (ou du seul son), pour la regarder
/// avant d'en faire une leçon : image H.264 et son AAC séparés (que la WebView lit
/// telles quelles), sinon un flux qui porte les deux (HLS compris).
/// L'image de YouTube se prend en HLS sur Mac : WebKit n'y lit pas ses fichiers
/// DASH d'image seule (la vidéo restait figée, le son seul jouait). Le son, lui,
/// se lit très bien en DASH.
pub async fn yt_stream(data_dir: &Path, ytdlp: &Path, url: &str, audio_only: bool, browser: Option<&str>) -> Result<serde_json::Value> {
    let format = if audio_only {
        "ba[ext=m4a][protocol=https]/ba[protocol=https]/ba/b"
    } else if cfg!(any(windows, target_os = "linux")) {
        // WebView2 (Windows) lit l'image DASH de YouTube, pas toujours le HLS ; WebKitGTK
        // (Linux) lit la DASH et pas le HLS du tout (hls.js ne peut lire le manifeste de
        // YouTube, qui refuse la lecture croisée) : la DASH pour tous les deux
        "bv*[vcodec^=avc1][height<=720][protocol=https]+ba[ext=m4a][protocol=https]/b[vcodec^=avc1][acodec!=none][protocol=https]/bv*[vcodec^=avc1][height<=720][protocol^=m3u8]+ba[ext=m4a][protocol=https]/b[protocol=https]/b[protocol^=m3u8]/b"
    } else {
        "bv*[vcodec^=avc1][height<=720][protocol^=m3u8]+ba[ext=m4a][protocol=https]/bv*[vcodec^=avc1][height<=720][protocol=https]+ba[ext=m4a][protocol=https]/b[vcodec^=avc1][acodec!=none][protocol=https]/b[protocol^=m3u8]/b[protocol=https]/b"
    };
    let args: Vec<String> = vec!["-J".into(), "-f".into(), format.into(), url.into()];
    let mut quiet = |_p: f64| {};
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, &mut quiet).await?;
    let json = out.iter().find(|l| l.starts_with('{')).ok_or_else(|| anyhow!(crate::i18n::t("vidéo illisible", "unreadable video")))?;
    Ok(serde_json::from_str(json)?)
}

/// Télécharge la piste audio d'une vidéo (YouTube et la plupart des sites
/// vidéo). Renvoie le fichier et le titre.
pub async fn yt_audio(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    stem: &str,
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<(PathBuf, String)> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let template = dir.join(format!("{stem}.audio.%(ext)s"));
    let args: Vec<String> = vec![
        "-f".into(),
        "ba[ext=m4a]/ba[acodec^=mp4a]/ba[ext=mp3]/b[ext=mp4]".into(),
        "--no-simulate".into(),
        "--print".into(),
        "TITLE:%(title)s".into(),
        "--print".into(),
        "after_move:FILE:%(filepath)s".into(),
        "-o".into(),
        template.display().to_string(),
        url.into(),
    ];
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, on_progress).await?;
    let title = out.iter().find_map(|l| l.strip_prefix("TITLE:")).unwrap_or(crate::i18n::t("Vidéo", "Video")).to_string();
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!(crate::i18n::t("fichier audio introuvable", "audio file not found")))?;
    Ok((PathBuf::from(file), title))
}

/// Choix de l'image à télécharger (`-f` puis `-S` de yt-dlp) : la plus légère
/// de celles que le moteur de la fenêtre lit (`codecs` : « av1 », « vp9 » ; le
/// H.264 toujours), à la meilleure définition jusqu'à `max_side` (côté court :
/// une vidéo en hauteur compte par sa largeur), 30 images/s de préférence, sans
/// HDR. Mesuré sur des vidéos d'apprenants : en 1080p, le VP9 de YouTube pèse
/// 53 à 68 % du H.264 et l'AV1 43 à 70 %, sans rien perdre (même original).
pub fn video_format(codecs: &[String], max_side: u32) -> (String, String) {
    let mut accepted = vec!["avc1"];
    if codecs.iter().any(|c| c == "vp9") {
        accepted.push("vp0?9");
    }
    if codecs.iter().any(|c| c == "av1") {
        accepted.push("av01");
    }
    let re = accepted.join("|");
    let format = format!("bv*[vcodec~='^({re})'][dynamic_range=SDR]/bv*[vcodec~='^({re})']/bv*[vcodec^=avc1]/bv*[ext=mp4]/b[ext=mp4]/bv*/b");
    // à définition égale, le fichier le plus petit ; en https plutôt qu'en HLS (plus lent)
    let sort = format!("res:{max_side},fps:30,proto,+size,+br");
    (format, sort)
}

/// L'image que `yt_video` choisirait, sans rien télécharger : son codec et son
/// poids annoncé (0 s'il est inconnu).
pub async fn yt_video_choice(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    browser: Option<&str>,
    codecs: &[String],
    max_side: u32,
) -> Result<(String, u64)> {
    let (format, sort) = video_format(codecs, max_side);
    let args: Vec<String> = vec!["-J".into(), "-f".into(), format, "-S".into(), sort, url.into()];
    let mut quiet = |_p: f64| {};
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, &mut quiet).await?;
    let json = out.iter().find(|l| l.starts_with('{')).ok_or_else(|| anyhow!(crate::i18n::t("vidéo illisible", "unreadable video")))?;
    let v: serde_json::Value = serde_json::from_str(json)?;
    let codec = v["vcodec"].as_str().unwrap_or("").to_string();
    let size = v["filesize"].as_u64().or_else(|| v["filesize_approx"].as_u64()).unwrap_or(0);
    Ok((codec, size))
}

/// Télécharge l'image de la vidéo (sans le son), synchronisée avec la piste
/// audio : voir `video_format` (`codecs`, `max_side`).
pub async fn yt_video(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    stem: &str,
    browser: Option<&str>,
    codecs: &[String],
    max_side: u32,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<PathBuf> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let template = dir.join(format!("{stem}.video.%(ext)s"));
    let (format, sort) = video_format(codecs, max_side);
    let args: Vec<String> = vec![
        "-f".into(),
        format,
        "-S".into(),
        sort,
        "--no-simulate".into(),
        "--print".into(),
        "after_move:FILE:%(filepath)s".into(),
        "-o".into(),
        template.display().to_string(),
        url.into(),
    ];
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, on_progress).await?;
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!(crate::i18n::t("fichier vidéo introuvable", "video file not found")))?;
    Ok(PathBuf::from(file))
}

pub fn new_stem() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S%3f").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ww(w: &str, t0: f64, t1: f64) -> WWord {
        WWord { w: w.into(), t0, t1 }
    }

    #[test]
    fn chinese_heard_as_words() {
        // les fragments de Whisper coupent les mots ailleurs que jieba : tout est retrouvé
        let text = "我们是学生。我们很好。";
        let heard = [ww("我", 0.0, 0.2), ww("们是", 0.2, 0.6), ww("学", 0.6, 0.8), ww("生。", 0.8, 1.0), ww("我们", 1.5, 1.9), ww("很好。", 1.9, 2.5)];
        let (timings, found) = align_timings(text, "zh", &heard);
        assert_eq!(found, 1.0);
        let t: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        // 我们 · 是 · 学生 · 我们 · 很 · 好 : un instant par mot de la leçon
        assert_eq!(t.len(), 6);
        assert_eq!(t[0], [0.0, 2.0, 0.0, 0.4]);
        assert_eq!(t[2], [3.0, 5.0, 0.6, 1.0]);
        // Whisper seul : le texte assemblé, sans espaces ; sa ponctuation en pleine chasse
        let (built, _) = build_transcript(&heard);
        assert_eq!(built, text);
        assert_eq!(full_width("是谁写的呢? 3,5 ok, 好!"), "是谁写的呢？ 3,5 ok, 好！");
    }

    #[test]
    fn realign_keeps_text_and_fills_gaps() {
        // titre absent de l'audio, mot mal entendu (« vacanze »), élision italienne
        let text = "Giustino in vacanza\n\nGiustino vuole andare in vacanza. Prende dell'acqua e parte.";
        let words = vec![
            ww(" Giustino", 2.0, 2.6), ww(" vuole", 2.6, 2.9), ww(" andare", 2.9, 3.3), ww(" in", 3.3, 3.4),
            ww(" vacanze.", 3.4, 4.0), ww(" Prende", 5.0, 5.4), ww(" dell'acqua", 5.4, 6.0), ww(" e", 6.0, 6.1), ww(" parte.", 6.1, 6.6),
        ];
        let (json, found) = align_timings(text, "it", &words);
        let t: Vec<[f64; 4]> = serde_json::from_str(&json).unwrap();
        let at = |w: &str| {
            let s = text.rfind(w).unwrap() as f64;
            t.iter().find(|x| x[0] == s).map(|x| x[2])
        };
        assert!(found > 0.6); // 9 mots sur 13, titre compris
        // le titre n'a pas d'horodatage, le second « Giustino » oui
        assert_eq!(t[0][0], text.rfind("Giustino").unwrap() as f64);
        assert_eq!(at("vuole"), Some(2.6));
        // « vacanza » non retrouvé : placé entre « in » et « Prende »
        let v = at("vacanza.").unwrap();
        assert!(v >= 3.4 && v <= 5.0);
        // « dell'acqua » entendu en un mot, coupé en deux comme dans le texte
        assert_eq!(at("dell'"), Some(5.4));
        assert!(at("acqua").unwrap() > 5.4);
        // ordre croissant
        assert!(t.windows(2).all(|p| p[1][2] >= p[0][2]));
    }

    #[test]
    fn anchors_survive_long_insertions() {
        let a: Vec<String> = (0..300).map(|i| format!("w{i}")).collect();
        // l'audio saute 40 mots et en ajoute 25 qui ne sont pas dans le texte
        let mut b: Vec<String> = a[..100].to_vec();
        b.extend((0..25).map(|i| format!("x{i}")));
        b.extend(a[140..].iter().cloned());
        let p = align_keys(&a, &b);
        assert_eq!(p.len(), 260);
        assert!(p.windows(2).all(|w| w[1].0 > w[0].0 && w[1].1 > w[0].1));
    }

    /// Essai réel (ignoré par défaut) : LUMEN_TEST_TEXT=texte.txt LUMEN_TEST_WORDS=mots.json
    /// LUMEN_TEST_LANG=it cargo test --lib realign_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn realign_live() {
        let (Ok(t), Ok(w)) = (std::env::var("LUMEN_TEST_TEXT"), std::env::var("LUMEN_TEST_WORDS")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "en".into());
        let text = std::fs::read_to_string(t).unwrap();
        let words: Vec<WWord> = serde_json::from_str(&std::fs::read_to_string(w).unwrap()).unwrap();
        let (json, found) = align_timings(&text, &lang, &words);
        let t: Vec<[f64; 4]> = serde_json::from_str(&json).unwrap();
        let u16: Vec<u16> = text.encode_utf16().collect();
        println!("{:.0} % des mots retrouvés, {} mots minutés", found * 100.0, t.len());
        for x in t.iter().take(24) {
            println!("{:>7.2} s  {}", x[2], String::from_utf16_lossy(&u16[x[0] as usize..x[1] as usize]));
        }
    }

    /// Chaque horodatage retombe sur le même mot après la mise en page.
    fn same_words(before: &str, bt: &str, after: &str, at: &str) {
        let (b, a): (Vec<[f64; 4]>, Vec<[f64; 4]>) = (serde_json::from_str(bt).unwrap(), serde_json::from_str(at).unwrap());
        assert_eq!(b.len(), a.len());
        let (b16, a16): (Vec<u16>, Vec<u16>) = (before.encode_utf16().collect(), after.encode_utf16().collect());
        for (x, y) in b.iter().zip(&a) {
            assert_eq!(String::from_utf16_lossy(&b16[x[0] as usize..x[1] as usize]), String::from_utf16_lossy(&a16[y[0] as usize..y[1] as usize]));
            assert_eq!((x[2], x[3]), (y[2], y[3]));
        }
    }

    /// Horodatages d'un texte : un mot toutes les 0,3 s, plus les pauses données (après le mot k).
    fn timed(text: &str, lang: &str, pauses: &[(usize, f64)]) -> String {
        let mut t = 0.0;
        let out: Vec<[f64; 4]> = text::tokenize(text, lang)
            .into_iter()
            .filter(|w| w.w)
            .enumerate()
            .map(|(k, w)| {
                let x = [w.s as f64, w.e as f64, t, t + 0.25];
                t += 0.3 + pauses.iter().find(|p| p.0 == k).map(|p| p.1).unwrap_or(0.0);
                x
            })
            .collect();
        serde_json::to_string(&out).unwrap()
    }

    #[test]
    fn airy_like_lingq() {
        let text = "Andrea ha una nuova fidanzata. La sua fidanzata si chiama «Sara»! Sì. Andrea vuole prepararle la cena, poi va al negozio.\n\nFine.";
        let tm = timed(text, "it", &[]);
        let (out, ot) = airy(text, "it", &tm);
        assert_eq!(out, "Andrea ha una nuova fidanzata.\n\nLa sua fidanzata si chiama «Sara»!\n\nSì. Andrea vuole prepararle la cena, poi va al negozio. Fine.");
        same_words(text, &tm, &out, &ot);
        // déjà aéré : rien ne bouge
        let (again, _) = airy(&out, "it", &ot);
        assert_eq!(again, out);
    }

    #[test]
    fn airy_cuts_long_spoken_runs_at_pauses() {
        // 90 mots sans ponctuation, une vraie pause après le 50e
        let text = (0..90).map(|i| format!("parola{i}")).collect::<Vec<_>>().join(" ");
        let tm = timed(&text, "it", &[(49, 0.9)]);
        let (out, ot) = airy(&text, "it", &tm);
        let paras: Vec<&str> = out.split("\n\n").collect();
        assert!(paras.iter().all(|p| p.split_whitespace().count() <= PARA_MAX_WORDS), "{paras:?}");
        assert!(paras.iter().all(|p| p.split_whitespace().count() >= PARA_MIN_PART));
        assert!(paras.iter().any(|p| p.ends_with("parola49")), "{paras:?}");
        same_words(&text, &tm, &out, &ot);
    }

    #[test]
    fn airy_without_spaces_and_untimed_words() {
        let text = "猫が好きです。犬も好きです。鳥はとても好きです。";
        let tm = timed(text, "ja", &[]);
        let (out, ot) = airy(text, "ja", &tm);
        assert_eq!(out, "猫が好きです。\n\n犬も好きです。\n\n鳥はとても好きです。");
        same_words(text, &tm, &out, &ot);
        // horodatages incomplets (début et fin non retrouvés) : la mise en page se fait quand même
        let text = "Prima frase qui. Seconda frase qui. Terza frase qui.";
        let all: Vec<[f64; 4]> = serde_json::from_str(&timed(text, "it", &[])).unwrap();
        let some = serde_json::to_string(&all[2..7]).unwrap();
        let (out, ot) = airy(text, "it", &some);
        assert_eq!(out, "Prima frase qui.\n\nSeconda frase qui.\n\nTerza frase qui.");
        same_words(text, &some, &out, &ot);
    }

    #[test]
    fn whisper_punctuates_run_on_transcripts() {
        // Qwen3-ASR : 48 mots sans un point, mieux entendus (« due tre ») ; Whisper ponctue
        let base = "ne fumavo due tre o quattro al massimo ed è stato più facile per chi fuma un pacchetto capisco che possa diventare difficile due nazioni possono avere vizi diversi penso di sì poi l'uomo è uomo in ogni caso uomini e donne però il luogo influenza il vizio che si può sviluppare";
        let heard = "Ne fumavo tre o quattro al massimo, ed è stato più facile. Per chi fuma un pacchetto capisco che possa diventare difficile. Due nazioni possono avere vizi diversi? Penso di sì, poi l'uomo è uomo in ogni caso, uomini e donne, però il luogo influenza il vizio che si può sviluppare.";
        let words: Vec<WWord> = heard.split(' ').enumerate().map(|(i, w)| ww(&format!(" {w}"), i as f64 * 0.3, i as f64 * 0.3 + 0.25)).collect();
        assert!(longest_run(base, "it") > RUN_ON_WORDS);
        let out = borrow_punctuation(base, "it", &words);
        assert_eq!(
            out,
            "Ne fumavo due tre o quattro al massimo, ed è stato più facile. Per chi fuma un pacchetto capisco che possa diventare difficile. Due nazioni possono avere vizi diversi? Penso di sì, poi l'uomo è uomo in ogni caso, uomini e donne, però il luogo influenza il vizio che si può sviluppare."
        );
        // mêmes mots, mêmes clés : le vocabulaire n'y voit rien
        assert_eq!(text::word_keys(&out, "it"), text::word_keys(base, "it"));
        assert_eq!(longest_run(&out, "it"), 24);
        // texte déjà ponctué, ou suite courte : rien ne change
        assert_eq!(borrow_punctuation(heard, "it", &words), heard);
        let short = "penso di sì poi l'uomo è uomo";
        assert_eq!(borrow_punctuation(short, "it", &words), short);
    }

    #[test]
    fn run_on_lessons_are_repaired_and_songs_left_alone() {
        // leçon déjà aérée : sauts de paragraphe au milieu des phrases, une virgule à la place d'un point
        let text = (0..30).map(|i| format!("parola{i}")).collect::<Vec<_>>().join(" ")
            + ",\n\n"
            + &(30..60).map(|i| format!("parola{i}")).collect::<Vec<_>>().join(" ");
        let heard: Vec<WWord> = (0..60).map(|i| ww(&format!(" parola{i}{}", if i == 29 { "." } else if i == 59 { "!" } else { "" }), i as f64, i as f64 + 0.5)).collect();
        assert_eq!(longest_run(&text, "it"), 60);
        let out = borrow_punctuation(&text, "it", &heard);
        assert!(out.starts_with("Parola0 ") && out.contains("parola29. Parola30") && out.ends_with("parola59!"), "{out}");
        // un point sans majuscule (entre deux morceaux de Qwen3-ASR), une apostrophe avant un saut de paragraphe
        let text = format!("Fine. {}\n\n{}", (0..30).map(|i| format!("parola{i}")).collect::<Vec<_>>().join(" "), (30..60).map(|i| format!("parola{i}")).collect::<Vec<_>>().join(" ").replacen("parola40", "po'\n\nparola40", 1));
        let out = borrow_punctuation(&text, "it", &[]);
        assert!(out.starts_with("Fine. Parola0 parola1") && out.contains("po' parola40") && !out.contains('\n'), "{out}");
        // une chanson : un vers par ligne, sans ponctuation ; chaque retour à la ligne termine une suite
        let song = (0..12).map(|v| (0..6).map(|i| format!("mot{v}x{i}")).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        assert_eq!(longest_run(&song, "it"), 6);
        // japonais, sans espaces
        let ja = "今日はとても暑いですね".repeat(6);
        let jw: Vec<WWord> = (0..6).map(|i| ww(&format!("今日はとても暑いですね{}", if i == 2 { "。" } else { "" }), i as f64, i as f64 + 0.9)).collect();
        let out = borrow_punctuation(&ja, "ja", &jw);
        assert_eq!(out.matches('。').count(), 1);
        assert_eq!(text::word_keys(&out, "ja"), text::word_keys(&ja, "ja"));
    }

    #[test]
    fn transcription_loops_are_collapsed() {
        let text = format!("Scrivicelo nei commenti. Ciao!{} Oh oh", " oh".repeat(300));
        assert_eq!(collapse_loops(&text, "it"), "Scrivicelo nei commenti. Ciao! oh");
        let text = "Ciao a tutti. E poi e poi e poi e poi e poi e poi, basta.";
        assert_eq!(collapse_loops(text, "it"), "Ciao a tutti. E poi, basta.");
        // la parole ordinaire ne bouge pas : « sì » redit, refrain redit trois fois
        let speech = "Sì, sì, sì, sì. Vieni qui, vieni qui, vieni qui!";
        assert_eq!(collapse_loops(speech, "it"), speech);
        let words: Vec<WWord> = (0..20).map(|i| ww(if i < 2 { " Grazie." } else { " oh" }, i as f64, i as f64 + 0.5)).collect();
        let kept = collapse_word_loops(words, "it");
        assert_eq!(kept.iter().map(|w| w.w.as_str()).collect::<String>(), " Grazie. Grazie. oh");
    }

    /// Ponctuation reprise sur une vraie transcription : LUMEN_TEST_TEXT=texte.txt
    /// LUMEN_TEST_WORDS=mots.json (sortie `done` de lumen-whisper) LUMEN_TEST_LANG=it
    /// cargo test --lib punctuation_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn punctuation_live() {
        let (Ok(t), Ok(w)) = (std::env::var("LUMEN_TEST_TEXT"), std::env::var("LUMEN_TEST_WORDS")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "en".into());
        let text = std::fs::read_to_string(t).unwrap();
        let words: Vec<WWord> = serde_json::from_str(&std::fs::read_to_string(w).unwrap()).unwrap();
        // comme « Recaler la lanterne » sur une leçon déjà importée
        let words = collapse_word_loops(words, &lang);
        let unlooped = collapse_loops(&text, &lang);
        let fixed = borrow_punctuation(&unlooped, &lang, &words);
        assert_eq!(text::word_keys(&fixed, &lang), text::word_keys(&unlooped, &lang));
        println!("{} mots, {} sans les boucles", text::word_count(&text, &lang), text::word_count(&unlooped, &lang));
        let (tm, found) = align_timings(&fixed, &lang, &words);
        let (out, _) = airy(&fixed, &lang, &tm);
        let sizes: Vec<usize> = out.split("\n\n").map(|p| p.split_whitespace().count()).collect();
        println!(
            "{:.0} % retrouvés · plus longue suite sans point : {} → {} mots · {} paragraphes, {:.1} mots en moyenne",
            found * 100.0,
            longest_run(&text, &lang),
            longest_run(&out, &lang),
            sizes.len(),
            sizes.iter().sum::<usize>() as f64 / sizes.len() as f64
        );
        println!("{}", out.split("\n\n").take(30).collect::<Vec<_>>().join("\n\n"));
        if let Ok(path) = std::env::var("LUMEN_AIRY_OUT") {
            std::fs::write(path, &out).unwrap();
        }
    }

    /// Mise en page réelle : LUMEN_TEST_TEXT=texte.txt LUMEN_TEST_WORDS=mots.json LUMEN_TEST_LANG=it
    /// cargo test --lib airy_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn airy_live() {
        let (Ok(t), Ok(w)) = (std::env::var("LUMEN_TEST_TEXT"), std::env::var("LUMEN_TEST_WORDS")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "en".into());
        let text = std::fs::read_to_string(t).unwrap();
        let words: Vec<WWord> = serde_json::from_str(&std::fs::read_to_string(w).unwrap()).unwrap();
        let (tm, found) = align_timings(&text, &lang, &words);
        let (out, ot) = airy(&text, &lang, &tm);
        same_words(&text, &tm, &out, &ot);
        let sizes: Vec<usize> = out.split("\n\n").map(|p| p.split_whitespace().count()).collect();
        println!("{:.0} % retrouvés · {} paragraphes · {} à {} mots, {:.1} en moyenne", found * 100.0, sizes.len(), sizes.iter().min().unwrap(), sizes.iter().max().unwrap(), sizes.iter().sum::<usize>() as f64 / sizes.len() as f64);
        println!("{}", out.split("\n\n").take(40).collect::<Vec<_>>().join("\n\n"));
        if let Ok(path) = std::env::var("LUMEN_AIRY_OUT") {
            std::fs::write(path, &out).unwrap();
        }
    }

    /// Texte d'une page gardé si le son le suit, transcription sinon (Whisper seul) :
    /// LUMEN_ASR_MODEL=ggml-….bin LUMEN_TEST_AUDIO=son.mp3 LUMEN_TEST_LANG=fr cargo test --lib reference_live -- --ignored --nocapture
    /// (copier d'abord binaries/lumen-whisper-aarch64-apple-darwin en target/debug/deps/lumen-whisper)
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn reference_live() {
        let (Ok(whisper), Ok(audio)) = (std::env::var("LUMEN_ASR_MODEL"), std::env::var("LUMEN_TEST_AUDIO")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "fr".into());
        let engine = crate::ai::Engine::new();
        // un texte sans rapport avec le son : Lumen transcrit
        let other = "Ogni mattina, Marta saliva le scale strette del vecchio faro. Dall'alto, il mare sembrava infinito e calmo.";
        let (heard, _) = lesson_transcript(&engine, Path::new(&whisper), None, None, Path::new(&audio), &lang, Some(other), |_| {}).await.unwrap();
        assert_ne!(heard, other);
        // le texte de la page : un titre en plus, un mot sur dix différent, autres paragraphes
        let words: Vec<&str> = heard.split_whitespace().collect();
        let body: Vec<String> = words.iter().enumerate().map(|(i, w)| if i % 10 == 5 { "autrement".to_string() } else { w.to_string() }).collect();
        let page = format!("Le journal du jour\n\n{}\n\n{}", body[..body.len() / 2].join(" "), body[body.len() / 2..].join(" "));
        let (text, timings) = lesson_transcript(&engine, Path::new(&whisper), None, None, Path::new(&audio), &lang, Some(&page), |_| {}).await.unwrap();
        let tm: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        println!("transcription : {} mots ; texte de la page gardé : {} ; {} mots minutés, du {:.1} s au {:.1} s", words.len(), text == page, tm.len(), tm[0][2], tm.last().unwrap()[2]);
        assert_eq!(text, page);
        assert!(tm.len() > words.len() * 8 / 10);
    }

    /// Import réel de bout en bout (Whisper puis Qwen3-ASR ou Parakeet, recalage, mise en page) :
    /// LUMEN_ASR_MODEL=ggml-….bin LUMEN_ASR_DIR=dossier de Qwen3-ASR (ou LUMEN_PARAKEET_DIR=dossier
    /// de Parakeet) LUMEN_TEST_AUDIO=son.mp3 LUMEN_TEST_LANG=it [LUMEN_AIRY_OUT=texte.txt]
    /// cargo test --release --lib transcript_live -- --ignored --nocapture
    /// (copier d'abord binaries/lumen-whisper-aarch64-apple-darwin en target/release/deps/lumen-whisper)
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn transcript_live() {
        let (Ok(whisper), Ok(audio)) = (std::env::var("LUMEN_ASR_MODEL"), std::env::var("LUMEN_TEST_AUDIO")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let fine = if let Ok(model) = std::env::var("LUMEN_PARAKEET_DIR") {
            // le moteur dans un dossier jetable
            let data = std::env::temp_dir().join("lumen-parakeet-live");
            std::fs::create_dir_all(&data).unwrap();
            crate::sherpa::ensure(&data, crate::sherpa::Tool::Asr, Default::default(), |_, _| {}).await.unwrap();
            Some(Writer::Parakeet { data_dir: data, model: PathBuf::from(model) })
        } else {
            std::env::var("LUMEN_ASR_DIR").ok().map(|d| {
                let files: Vec<PathBuf> = std::fs::read_dir(d).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
                let pick = |mm: bool| files.iter().find(|p| {
                    let n = p.file_name().unwrap().to_string_lossy();
                    n.ends_with(".gguf") && n.starts_with("mmproj") == mm
                }).unwrap().clone();
                Writer::Qwen { model: pick(false), mmproj: pick(true) }
            })
        };
        let engine = crate::ai::Engine::new();
        let t = std::time::Instant::now();
        let mut stages = Vec::new();
        // les interlocuteurs : LUMEN_VOICES_DIR=dossier des deux modèles, LUMEN_DATA=dossier de données qui a le moteur
        let voices = std::env::var("LUMEN_VOICES_DIR").ok().map(|d| crate::speakers::Model {
            data_dir: std::env::var("LUMEN_DATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("lumen-parakeet-live")),
            voices: Path::new(&d).join("nemo_en_titanet_small.onnx"),
            turns: Path::new(&d).join("pyannote-segmentation-3-0.onnx"),
        });
        let (text, timings) = lesson_transcript(&engine, Path::new(&whisper), fine, voices, Path::new(&audio), &lang, None, |e| {
            if let ImportEvent::Stage { stage } = e {
                stages.push(stage);
            }
        })
        .await
        .unwrap();
        let tm: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        let paras = text.split("\n\n").count();
        println!("{:.0} s · étapes {stages:?} · {} mots minutés · {paras} paragraphes\n{}", t.elapsed().as_secs_f64(), tm.len(), text.split("\n\n").take(12).collect::<Vec<_>>().join("\n\n"));
        if let Ok(out) = std::env::var("LUMEN_AIRY_OUT") {
            std::fs::write(out, &text).unwrap();
        }
        assert!(paras > 3 && !tm.is_empty());
    }

    #[test]
    fn numbers_stay_with_their_sentence() {
        // « 1. » finit la phrase et reste avec elle ; le point de « 7.000 » n'est pas une fin
        let text = "Lui vinceva nel campionato di Formula 1. Fantastico, davvero. Raccoglie circa 7.000 euro in un anno. Sono il 99%. Alcuni la definiscono così.";
        let tm = timed(text, "it", &[]);
        let (out, ot) = airy(text, "it", &tm);
        assert_eq!(out, "Lui vinceva nel campionato di Formula 1.\n\nFantastico, davvero. Raccoglie circa 7.000 euro in un anno.\n\nSono il 99%. Alcuni la definiscono così.");
        same_words(text, &tm, &out, &ot);
        // hésitation : des points de suspension devant une minuscule ne coupent pas
        let text = "Io… penso che sia giusto così. Poi… Vediamo che cosa succede.";
        let (out, _) = airy(text, "it", &timed(text, "it", &[]));
        assert_eq!(out, "Io… penso che sia giusto così.\n\nPoi… Vediamo che cosa succede.");
        assert_eq!(sentence_stop(". ", "Poi"), Some(1));
        assert_eq!(sentence_stop(" 3.5 ", "volte"), None);
        assert_eq!(sentence_stop("!» ", "E"), Some(3));
        assert_eq!(sentence_stop("... ", "e"), None);
    }

    #[test]
    fn each_reply_has_its_paragraph() {
        let text = "Andiamo al mare domani? Sì. Va bene. Allora preparo la macchina stasera. Non ha nulla a che fare con confermi che non ha nulla a che fare con l'Italia. Ecco.";
        let tm = timed(text, "it", &[]);
        let at = |w: &str| text[..text.find(w).unwrap()].encode_utf16().count();
        // sans les voix : la réponse courte rejoint la phrase qui suit, celle d'un autre
        let (plain, _) = airy(text, "it", &tm);
        assert!(plain.contains("domani?\n\nSì. Va bene.\n\nAllora") && plain.contains("fare con confermi che"));
        // « Sì. » est d'une autre voix, « Va bene. Allora… » revient à la première ; la parole
        // coupée après « con » ouvre un paragraphe au milieu de la phrase
        let (out, ot) = airy_turns(text, "it", &tm, &[at("Sì"), at("Va bene"), at("confermi"), at("Ecco")]);
        assert_eq!(
            out,
            "Andiamo al mare domani?\n\nSì.\n\nVa bene. Allora preparo la macchina stasera.\n\nNon ha nulla a che fare con\n\nconfermi che non ha nulla a che fare con l'Italia.\n\nEcco."
        );
        same_words(text, &tm, &out, &ot);
        // déjà mis en page : rien ne bouge
        let again = |w: &str| out[..out.find(w).unwrap()].encode_utf16().count();
        let (twice, _) = airy_turns(&out, "it", &ot, &[again("Sì"), again("Va bene"), again("confermi"), again("Ecco")]);
        assert_eq!(twice, out);
    }

    /// Son d'essai : de la « voix » partout, sauf dans les silences donnés (début, durée).
    fn voice(secs: f64, silences: &[(f64, f64)]) -> Vec<f32> {
        let rate = crate::asr::RATE as f64;
        let mut pcm: Vec<f32> = (0..(secs * rate) as usize).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        for &(a, d) in silences {
            pcm[(a * rate) as usize..((a + d) * rate) as usize].iter_mut().for_each(|x| *x = 0.0);
        }
        pcm
    }

    #[test]
    fn pieces_end_where_sentences_end() {
        let rate = crate::asr::RATE as f64;
        // un mot toutes les 0,5 s ; fins de phrase à 9 s (courte pause) et à 27 s, un grand
        // silence à 14 s au milieu d'une phrase : l'ancienne découpe y coupait
        let mut words = Vec::new();
        for i in 0..80 {
            let t0 = i as f64 * 0.5;
            let (w, t1) = match i {
                17 => (" fine.".to_string(), t0 + 0.3),
                27 => (" con".to_string(), t0 + 0.1),
                53 => (" basta?".to_string(), t0 + 0.3),
                _ => (format!(" parola{i}"), t0 + 0.5),
            };
            words.push(ww(&w, t0, t1));
        }
        let pcm = voice(40.0, &[(8.8, 0.2), (13.6, 0.4), (26.8, 0.2)]);
        let old = crate::asr::split_on_silence(&pcm, 15.0);
        assert!((13.6..14.0).contains(&(old[0].1 as f64 / rate)));
        let parts = split_at_words(&pcm, &words, 15.0);
        assert_eq!(parts.len(), 3);
        assert!(parts.windows(2).all(|w| w[0].b == w[1].a) && parts[0].a == 0 && parts[2].b == pcm.len());
        // après « fine. » puis après « basta? », dans leurs pauses
        assert!((8.8..9.0).contains(&(parts[0].b as f64 / rate)), "{}", parts[0].b as f64 / rate);
        assert!((26.8..27.0).contains(&(parts[1].b as f64 / rate)));
        assert_eq!((parts[0].end.as_deref(), parts[0].next.as_deref()), (Some("."), Some("parola18")));
        assert_eq!(parts[1].end.as_deref(), Some("?"));
        assert_eq!(parts[2].end, None);
        // aucune fin de phrase à portée : la plus longue pause, et Whisper dit que la phrase continue
        let plain: Vec<WWord> = (0..80).map(|i| ww(&format!(" parola{i}"), i as f64 * 0.5, i as f64 * 0.5 + if i == 27 { 0.1 } else { 0.5 })).collect();
        let parts = split_at_words(&pcm, &plain, 15.0);
        assert!((13.6..14.0).contains(&(parts[0].b as f64 / rate)));
        assert_eq!((parts[0].end.as_deref(), parts[0].next.as_deref()), (Some(""), Some("parola28")));
        // rien d'entendu (musique) : le silence le plus net, sans repère
        let parts = split_at_words(&pcm, &[], 15.0);
        assert_eq!(parts.iter().map(|p| (p.a, p.b)).collect::<Vec<_>>(), old);
        assert!(parts.iter().all(|p| p.end.is_none()));
        // un son court : un seul morceau
        assert_eq!(split_at_words(&pcm[..16_000 * 12], &words, 15.0).len(), 1);
    }

    #[test]
    fn seams_follow_what_whisper_heard() {
        let piece = |end: Option<&str>, next: Option<&str>| Piece { a: 0, b: 1, end: end.map(String::from), next: next.map(String::from) };
        let join = |a: &str, b: &str, end: Option<&str>, next: Option<&str>, lang: &str| join_pieces(&[a.to_string(), b.to_string()], &[piece(end, next), piece(None, None)], lang);
        // la phrase continue : le point et la majuscule ajoutés par le modèle s'en vont
        assert_eq!(join("Inizio a cercare di.", "Portarmi alcuni pezzi.", Some(""), Some("portarmi"), "it"), "Inizio a cercare di portarmi alcuni pezzi.");
        assert_eq!(join("Va bene.", "Ma non oggi.", Some(","), Some("ma"), "it"), "Va bene, ma non oggi.");
        // nom propre chez Whisper, mot différent, « ? » du modèle : dans le doute, rien ne change
        assert_eq!(join("Un altro Kimi.", "Raikkonen ha vinto.", Some(""), Some("Raikkonen"), "it"), "Un altro Kimi. Raikkonen ha vinto.");
        assert_eq!(join("Non so che.", "Capo della Morra.", Some(""), Some("gioco"), "it"), "Non so che. Capo della Morra.");
        assert_eq!(join("Chi è?", "Questo Cesare.", Some(""), Some("questo"), "it"), "Chi è? Questo Cesare.");
        // fin de phrase entendue : le point et la majuscule y sont
        assert_eq!(join("Ci vediamo di là", "allora ciao.", Some("."), Some("Allora"), "it"), "Ci vediamo di là. Allora ciao.");
        assert_eq!(join("Come stai,", "bene.", Some("?"), Some("Bene"), "it"), "Come stai? Bene.");
        assert_eq!(join("Tutto bene!", "Sì.", Some("."), Some("Sì"), "it"), "Tutto bene! Sì.");
        // sans repère (silence, musique) : simplement bout à bout
        assert_eq!(join("Uno.", "Due.", None, None, "it"), "Uno. Due.");
        // un morceau vide ne laisse pas de repère
        let texts = ["Uno di.".to_string(), String::new(), "Due.".to_string()];
        assert_eq!(join_pieces(&texts, &[piece(Some(""), Some("due")), piece(Some(""), Some("due")), piece(None, None)], "it"), "Uno di. Due.");
        // turc : « I » ne devient pas « i » ; japonais : sans espace
        assert_eq!(join("Bu bir.", "Irmak var.", Some(""), Some("irmak"), "tr"), "Bu bir Irmak var.");
        assert_eq!(join("今日は。", "暑いですね。", Some(""), Some("暑"), "ja"), "今日は暑いですね。");
    }

    /// Le texte d'un vrai son par le modèle qui écrit, d'après des mots de Whisper déjà
    /// calculés (sortie `done` de lumen-whisper) : compare l'ancienne découpe (silences)
    /// et la nouvelle (fins de phrase). LUMEN_TEST_AUDIO=son.wav (f32, 16 kHz, mono)
    /// LUMEN_TEST_WORDS=mots.json LUMEN_PARAKEET_DIR=… (ou LUMEN_ASR_DIR=…) LUMEN_TEST_LANG=it
    /// [LUMEN_DATA=dossier de données qui a déjà le moteur] [LUMEN_CUT=silence] [LUMEN_AIRY_OUT=texte.txt]
    /// cargo test --release --lib writer_live -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn writer_live() {
        let (Ok(audio), Ok(w)) = (std::env::var("LUMEN_TEST_AUDIO"), std::env::var("LUMEN_TEST_WORDS")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let writer = if let Ok(model) = std::env::var("LUMEN_PARAKEET_DIR") {
            let data = std::env::var("LUMEN_DATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("lumen-parakeet-live"));
            std::fs::create_dir_all(&data).unwrap();
            crate::sherpa::ensure(&data, crate::sherpa::Tool::Asr, Default::default(), |_, _| {}).await.unwrap();
            Writer::Parakeet { data_dir: data, model: PathBuf::from(model) }
        } else {
            let files: Vec<PathBuf> = std::fs::read_dir(std::env::var("LUMEN_ASR_DIR").unwrap()).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
            let pick = |mm: bool| files.iter().find(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.ends_with(".gguf") && n.starts_with("mmproj") == mm
            }).unwrap().clone();
            Writer::Qwen { model: pick(false), mmproj: pick(true) }
        };
        let words = collapse_word_loops(read_words(&w), &lang);
        let samples = read_wav(&audio);
        let cut = if std::env::var("LUMEN_CUT").is_ok_and(|c| c == "silence") { Cut::Silence } else { Cut::Sentences };
        // les interlocuteurs : LUMEN_VOICES_DIR=dossier des deux modèles (le moteur de LUMEN_DATA doit avoir l'outil)
        let voices = std::env::var("LUMEN_VOICES_DIR").ok().map(|d| crate::speakers::Model {
            data_dir: std::env::var("LUMEN_DATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("lumen-parakeet-live")),
            voices: Path::new(&d).join("nemo_en_titanet_small.onnx"),
            turns: Path::new(&d).join("pyannote-segmentation-3-0.onnx"),
        });
        let engine = crate::ai::Engine::new();
        let t = std::time::Instant::now();
        let mut stages = Vec::new();
        let (out, tm) = lesson_text(&engine, Some(&writer), voices.as_ref(), Some(&samples), &words, &lang, cut, |e| {
            if let ImportEvent::Stage { stage } = e {
                stages.push(stage);
            }
        })
        .await;
        let timed: Vec<[f64; 4]> = serde_json::from_str(&tm).unwrap();
        println!("{:.0} s · étapes {stages:?} · {} mots, {} minutés · {} paragraphes", t.elapsed().as_secs_f64(), text::word_count(&out, &lang), timed.len(), out.split("\n\n").count());
        if let Ok(path) = std::env::var("LUMEN_AIRY_OUT") {
            std::fs::write(path, &out).unwrap();
        }
    }

    /// Mots de Whisper : un tableau JSON, ou les lignes de `lumen-whisper` (celle de `done`).
    fn read_words(path: &str) -> Vec<WWord> {
        let raw = std::fs::read_to_string(path).unwrap();
        if let Ok(w) = serde_json::from_str::<Vec<WWord>>(&raw) {
            return w;
        }
        let done = raw.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).find(|v| v["type"] == "done").unwrap();
        serde_json::from_value(done["words"].clone()).unwrap()
    }

    /// WAV en f32, mono, 16 kHz (afconvert -f WAVE -d LEF32@16000 -c 1).
    fn read_wav(path: &str) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if &b[i..i + 4] == b"data" {
                return b[i + 8..(i + 8 + len).min(b.len())].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("pas de données dans {path}");
    }

    #[test]
    fn transcript_offsets() {
        let words = vec![
            WWord { w: " Hello".into(), t0: 0.0, t1: 0.4 },
            WWord { w: " world.".into(), t0: 0.4, t1: 0.9 },
            WWord { w: " Next".into(), t0: 3.0, t1: 3.4 },
        ];
        let (text, timings) = build_transcript(&words);
        assert_eq!(text, "Hello world.\n\nNext");
        let t: Vec<[f64; 4]> = serde_json::from_str(&timings).unwrap();
        assert_eq!(t[2][0] as usize, 14);
    }
}
