//! Import audio et vidéo : copie dans la bibliothèque, transcription locale
//! par le processus `lumen-whisper`, construction du texte et des horodatages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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
    let dir = exe.parent().ok_or_else(|| anyhow!("dossier de l'application introuvable"))?;
    let name = if cfg!(windows) { "lumen-whisper.exe" } else { "lumen-whisper" };
    let p = dir.join(name);
    if p.exists() {
        return Ok(p);
    }
    Err(anyhow!("le composant de transcription est absent de l'application"))
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
    mut on_event: impl FnMut(ImportEvent),
) -> Result<Vec<WWord>> {
    let bin = sidecar_path()?;
    let mut cmd = Command::new(bin);
    cmd.arg(model).arg(media).arg(lang);
    if let Some(p) = pcm_out {
        cmd.arg("--pcm").arg(p);
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
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
            Some("error") => error = Some(v["message"].as_str().unwrap_or("erreur inconnue").to_string()),
            _ => {}
        }
    }
    let _ = child.wait().await;
    if let Some(e) = error {
        return Err(anyhow!(e));
    }
    result.ok_or_else(|| anyhow!("la transcription s'est arrêtée sans résultat"))
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
        let ends_sentence = text.trim_end().ends_with(['.', '!', '?', '…']);
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

/// Texte et horodatages d'une leçon à partir d'un son. Whisper repère chaque
/// mot dans le temps (et décode le son) ; si Qwen3-ASR est fourni (`fine` :
/// modèle et partie audio), il écrit le texte, plus juste, sur lequel le
/// minutage de Whisper est recalé. Le texte est ensuite aéré comme sur LingQ.
pub async fn lesson_transcript(
    engine: &crate::ai::Engine,
    whisper: &Path,
    fine: Option<(PathBuf, PathBuf)>,
    media: &Path,
    lang: &str,
    mut on_event: impl FnMut(ImportEvent),
) -> Result<(String, String)> {
    let pcm = fine.as_ref().map(|_| std::env::temp_dir().join(format!("lumen-{}.pcm", new_stem())));
    let heard = transcribe(whisper, media, lang, pcm.as_deref(), |e| {
        // avec Qwen3-ASR, Whisper ne fait plus que repérer les mots dans le temps
        let e = match e {
            ImportEvent::Stage { stage } if stage == "transcribe" && fine.is_some() => ImportEvent::Stage { stage: "timing".into() },
            e => e,
        };
        on_event(e);
    })
    .await;
    let words = match heard {
        Ok(w) => w,
        Err(e) => {
            if let Some(p) = &pcm {
                let _ = std::fs::remove_file(p);
            }
            return Err(e);
        }
    };
    let (mut text, mut timings) = build_transcript(&words);
    if let (Some((model, mmproj)), Some(pcm)) = (fine, pcm) {
        on_event(ImportEvent::Stage { stage: "text".into() });
        let written = read_pcm(&pcm).and_then(|samples| {
            tokio::task::block_in_place(|| {
                crate::asr::transcribe(engine, &model, &mmproj, &samples, lang, &std::sync::atomic::AtomicBool::new(false), |p| {
                    on_event(ImportEvent::Progress { value: p });
                })
            })
        });
        let _ = std::fs::remove_file(&pcm);
        match written {
            Ok(t) if !t.trim().is_empty() => {
                let (t_timings, found) = align_timings(&t, lang, &words);
                // garde-fou : un texte qui ne ressemble pas à ce qu'a entendu Whisper est écarté
                if found >= 0.5 {
                    text = t;
                    timings = t_timings;
                } else {
                    eprintln!("Qwen3-ASR écarté : {:.0} % des mots retrouvés", found * 100.0);
                }
            }
            Ok(_) => {}
            // Whisper seul plutôt qu'un échec : la leçon se crée quand même
            Err(e) => eprintln!("Qwen3-ASR indisponible : {e}"),
        }
    }
    Ok(airy(&text, lang, &timings))
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
    let times: Vec<[f64; 4]> = serde_json::from_str(timings).unwrap_or_default();
    let words: Vec<text::Token> = text::tokenize(text, lang).into_iter().filter(|t| t.w).collect();
    let n = words.len();
    if n < 2 {
        return (text.trim().to_string(), timings.to_string());
    }
    // position UTF-16 → position en octets
    let mut byte_at: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        byte_at.extend(std::iter::repeat_n(i, c.len_utf16()));
    }
    byte_at.push(text.len());
    let byte = |p: usize| byte_at[p.min(byte_at.len() - 1)];
    let when: HashMap<usize, (f64, f64)> = times.iter().map(|t| (t[0] as usize, (t[2], t[3]))).collect();
    let wt: Vec<Option<(f64, f64)>> = words.iter().map(|w| when.get(&w.s).copied()).collect();
    // ce qui sépare le mot k du suivant : espaces et ponctuation
    let gap = |k: usize| &text[byte(words[k].e)..byte(words[k + 1].s)];
    let ends_sentence = |g: &str| g.contains(['.', '!', '?', '…', '。', '！', '？']);

    // 1. phrases (un saut de paragraphe existant compte aussi)
    let mut sentences: Vec<(usize, usize)> = Vec::new();
    let mut a = 0;
    for k in 0..n - 1 {
        let g = gap(k);
        if ends_sentence(g) || g.contains('\n') {
            sentences.push((a, k));
            a = k + 1;
        }
    }
    sentences.push((a, n - 1));
    // 2. les phrases trop courtes rejoignent la suivante (la dernière, la précédente)
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut carry: Option<usize> = None;
    for (a, b) in sentences {
        let a = carry.take().unwrap_or(a);
        if b + 1 - a < PARA_MIN_WORDS {
            carry = Some(a);
        } else {
            merged.push((a, b));
        }
    }
    if let Some(a) = carry {
        match merged.last_mut() {
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
        let comma = if gap(k).contains([',', ';', ':', '–', '—']) { 0.25 } else { 0.0 };
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
            // ponctuation collée au mot, saut de paragraphe, puis ce qui ouvre la phrase suivante
            match g.find(char::is_whitespace) {
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
            // ancien saut de paragraphe, devenu inutile : une simple espace
            let joined = g.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push(&joined);
            if g.ends_with(char::is_whitespace) {
                out.push(" ");
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

// ---------- recalage sur un texte existant ----------

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Mots entendus, redécoupés comme le texte de Lumen (élisions comprises),
/// la durée d'un mot coupé étant répartie au prorata de la longueur.
fn heard_keys(words: &[WWord], lang: &str) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    for w in words {
        let parts: Vec<text::Token> = text::tokenize(w.w.trim(), lang).into_iter().filter(|t| t.w).collect();
        let total: usize = parts.iter().map(|p| p.e - p.s).sum();
        let mut at = w.t0;
        for p in &parts {
            let d = if total > 0 { (w.t1 - w.t0).max(0.0) * (p.e - p.s) as f64 / total as f64 } else { 0.0 };
            out.push((p.k.clone(), at, at + d));
            at += d;
        }
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
    let b: Vec<String> = heard.iter().map(|h| h.0.clone()).collect();
    let pairs = align_keys(&a, &b);
    let mut times: Vec<Option<(f64, f64)>> = vec![None; toks.len()];
    for &(i, j) in &pairs {
        times[i] = Some((heard[j].1, heard[j].2));
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
    let mut cmd = Command::new(ytdlp);
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
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("sortie indisponible"))?;
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
    m.contains("sign in") || m.contains("cookies") || m.contains("not a bot") || m.contains("age")
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
            None => Err(anyhow!(
                "YouTube demande une vérification pour cette vidéo. Choisissez votre navigateur dans Réglages › YouTube pour que Lumen utilise votre session. ({e})"
            )),
        },
        Err(e) => Err(e),
    }
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
    let title = out.iter().find_map(|l| l.strip_prefix("TITLE:")).unwrap_or("Vidéo").to_string();
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!("fichier audio introuvable"))?;
    Ok((PathBuf::from(file), title))
}

/// Télécharge l'image de la vidéo (sans le son), en H.264 jusqu'en 1080p :
/// lisible partout sur Mac, synchronisée avec la piste audio.
pub async fn yt_video(
    data_dir: &Path,
    ytdlp: &Path,
    url: &str,
    stem: &str,
    browser: Option<&str>,
    on_progress: &mut (dyn FnMut(f64) + Send),
) -> Result<PathBuf> {
    let dir = media_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let template = dir.join(format!("{stem}.video.%(ext)s"));
    let args: Vec<String> = vec![
        "-f".into(),
        "bv*[vcodec^=avc1][height<=1080][ext=mp4]/bv*[vcodec^=avc1][height<=1080]/bv*[ext=mp4][height<=1080]/b[ext=mp4]".into(),
        "--no-simulate".into(),
        "--print".into(),
        "after_move:FILE:%(filepath)s".into(),
        "-o".into(),
        template.display().to_string(),
        url.into(),
    ];
    let out = run_with_fallback(data_dir, ytdlp, &args, browser, on_progress).await?;
    let file = out.iter().find_map(|l| l.strip_prefix("FILE:")).ok_or_else(|| anyhow!("fichier vidéo introuvable"))?;
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

    /// Import réel de bout en bout (Whisper puis Qwen3-ASR, recalage, mise en page) :
    /// LUMEN_ASR_MODEL=ggml-….bin LUMEN_ASR_DIR=dossier de Qwen3-ASR LUMEN_TEST_AUDIO=son.mp3
    /// LUMEN_TEST_LANG=it [LUMEN_AIRY_OUT=texte.txt] cargo test --release --lib transcript_live -- --ignored --nocapture
    /// (copier d'abord binaries/lumen-whisper-aarch64-apple-darwin en target/release/deps/lumen-whisper)
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn transcript_live() {
        let (Ok(whisper), Ok(audio)) = (std::env::var("LUMEN_ASR_MODEL"), std::env::var("LUMEN_TEST_AUDIO")) else { return };
        let lang = std::env::var("LUMEN_TEST_LANG").unwrap_or_else(|_| "it".into());
        let fine = std::env::var("LUMEN_ASR_DIR").ok().map(|d| {
            let files: Vec<PathBuf> = std::fs::read_dir(d).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
            let pick = |mm: bool| files.iter().find(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.ends_with(".gguf") && n.starts_with("mmproj") == mm
            }).unwrap().clone();
            (pick(false), pick(true))
        });
        let engine = crate::ai::Engine::new();
        let t = std::time::Instant::now();
        let mut stages = Vec::new();
        let (text, timings) = lesson_transcript(&engine, Path::new(&whisper), fine, Path::new(&audio), &lang, |e| {
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
