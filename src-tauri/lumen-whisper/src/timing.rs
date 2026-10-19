//! Minutage des mots de Whisper : ce que partagent `lumen-whisper` (programme à part,
//! Mac et Windows) et l'app iPhone et iPad, où Whisper tourne dans l'app
//! (src/whisper.rs, qui inclut ce fichier par `#[path]`).
#![allow(dead_code)]

use std::path::Path;

use serde::Serialize;

pub const TARGET_RATE: u32 = 16_000;

#[derive(Serialize)]
pub struct Word {
    pub w: String,
    pub t0: f64,
    pub t1: f64,
}


/// Rééchantillonnage vers 16 kHz : filtre passe-bas (sinc fenêtré, Blackman)
/// appliqué à la volée, puis interpolation linéaire.
pub fn resample(input: &[f32], from: u32) -> Vec<f32> {
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

/// Têtes d'attention de référence pour l'alignement, selon le modèle
/// (« large-v3-turbo », « small.en »…), d'après le nom de son fichier.
pub fn dtw_name(model: &str) -> Option<&'static str> {
    let name = Path::new(model).file_name()?.to_str()?.to_lowercase();
    let en = name.contains(".en");
    Some(if name.contains("large-v3-turbo") {
        "large-v3-turbo"
    } else if name.contains("large-v3") {
        "large-v3"
    } else if name.contains("large-v2") {
        "large-v2"
    } else if name.contains("large") {
        "large-v1"
    } else if name.contains("medium") {
        if en { "medium.en" } else { "medium" }
    } else if name.contains("small") {
        if en { "small.en" } else { "small" }
    } else if name.contains("base") {
        if en { "base.en" } else { "base" }
    } else if name.contains("tiny") {
        if en { "tiny.en" } else { "tiny" }
    } else {
        return None;
    })
}

/// Énergie du son par tranches de 10 ms, et seuil voix / silence adapté à
/// l'enregistrement (bruit de fond, musique).
pub struct Envelope {
    db: Vec<f32>,
    thr: f32,
}

const HOP: f64 = 0.01;

impl Envelope {
    pub fn new(audio: &[f32]) -> Self {
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
pub fn refine(words: &mut [Word], env: &Envelope, duration: f64) {
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

/// Un fragment de mot (jeton) tel que Whisper le donne, horodatages en centièmes de seconde.
pub struct Tok {
    pub id: i32,
    pub bytes: Vec<u8>,
    /// début estimé classique
    pub t0: i64,
    /// instant de l'alignement DTW (fin du fragment), négatif s'il manque
    pub t_dtw: i64,
}

/// Mots reconstitués à partir des fragments de chaque segment (un mot commence par
/// une espace) ; les octets sont assemblés avant décodage, pour le cyrillique notamment.
/// `eot` : premier jeton spécial (horodatages, fin de texte), écarté.
pub fn assemble(segments: &[Vec<Tok>], eot: i32, use_dtw: bool) -> Vec<Word> {
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
    for seg in segments {
        // l'instant DTW d'un fragment marque sa fin : un mot commence là où
        // finit le fragment précédent (en début de segment, l'estimation
        // classique, recalée ensuite sur l'attaque du son)
        let mut prev_end: Option<f64> = None;
        for tok in seg {
            if tok.id >= eot {
                continue;
            }
            let legacy = tok.t0 as f64 / 100.0;
            if tok.bytes.first() == Some(&b' ') || cur.is_none() {
                flush(&mut cur, &mut words, prev_end);
                let start = if use_dtw { prev_end.unwrap_or(legacy) } else { legacy };
                cur = Some((tok.bytes.clone(), start.max(0.0)));
            } else if let Some((b, _)) = cur.as_mut() {
                b.extend_from_slice(&tok.bytes);
            }
            if use_dtw && tok.t_dtw >= 0 {
                prev_end = Some(tok.t_dtw as f64 / 100.0);
            }
        }
        flush(&mut cur, &mut words, prev_end);
    }
    words
}
