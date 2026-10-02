//! IA locale : Qwen3.5 exécuté par llama.cpp (Metal sur Mac).
//! Sert à la traduction contextuelle des mots, des phrases et à la
//! réécriture de textes à un niveau plus simple.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use parking_lot::Mutex;

static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend> {
    if let Some(b) = BACKEND.get() {
        return Ok(b);
    }
    let mut b = LlamaBackend::init().map_err(|e| anyhow!("llama.cpp : {e}"))?;
    b.void_logs();
    let _ = BACKEND.set(b);
    Ok(BACKEND.get().unwrap())
}

pub struct Engine {
    loaded: Mutex<Option<(PathBuf, Arc<LlamaModel>)>>,
    run_lock: Mutex<()>,
    /// incrémenté à chaque requête interactive : une requête plus récente
    /// interrompt la précédente (clics rapides sur plusieurs mots).
    epoch: AtomicU64,
}

pub enum Priority {
    /// interrompue dès qu'une autre requête interactive arrive
    Interactive(u64),
    /// tâche longue (réécriture d'un texte)
    Background,
}

impl Engine {
    pub fn new() -> Self {
        Self { loaded: Mutex::new(None), run_lock: Mutex::new(()), epoch: AtomicU64::new(0) }
    }

    pub fn next_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_loaded(&self, path: &Path) -> bool {
        self.loaded.lock().as_ref().map(|(p, _)| p == path).unwrap_or(false)
    }

    pub fn unload(&self) {
        *self.loaded.lock() = None;
    }

    pub fn load(&self, path: &Path) -> Result<Arc<LlamaModel>> {
        let mut guard = self.loaded.lock();
        if let Some((p, m)) = guard.as_ref() {
            if p == path {
                return Ok(m.clone());
            }
        }
        *guard = None;
        let be = backend()?;
        let params = LlamaModelParams::default().with_n_gpu_layers(999);
        let model = LlamaModel::load_from_file(be, path, &params).map_err(|e| anyhow!("modèle illisible : {e}"))?;
        let model = Arc::new(model);
        *guard = Some((path.to_path_buf(), model.clone()));
        Ok(model)
    }

    /// Génère une réponse (format de conversation ChatML de Qwen, mode
    /// « réflexion » désactivé). `on_piece` reçoit le texte au fil de l'eau ;
    /// s'il renvoie `false`, la génération s'arrête.
    pub fn generate(
        &self,
        model_path: &Path,
        messages: &[(&str, String)],
        max_tokens: usize,
        priority: Priority,
        mut on_piece: impl FnMut(&str) -> bool,
    ) -> Result<String> {
        let _run = self.run_lock.lock();
        if let Priority::Interactive(id) = priority {
            if self.epoch.load(Ordering::SeqCst) != id {
                return Err(anyhow!("interrompu"));
            }
        }
        let model = self.load(model_path)?;
        let be = backend()?;
        let mut prompt = String::new();
        for (role, content) in messages {
            prompt.push_str(&format!("<|im_start|>{role}\n{content}<|im_end|>\n"));
        }
        prompt.push_str("<|im_start|>assistant\n<think>\n\n</think>\n\n");
        let vocab = model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), false, true);
        let n_prompt = tokens.len();
        let n_ctx = (n_prompt + max_tokens + 16).max(512) as u32;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) as i32;
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(n_ctx.min(2048))
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let mut ctx = model.new_context(be, ctx_params).map_err(|e| anyhow!("contexte : {e}"))?;

        let mut batch = LlamaBatch::new(n_ctx as usize, 1);
        // le prompt est envoyé par blocs pour respecter n_batch
        let chunk = n_ctx.min(2048) as usize;
        let mut pos = 0usize;
        for part in tokens.chunks(chunk) {
            batch.clear();
            for (i, t) in part.iter().enumerate() {
                let is_last = pos + i == n_prompt - 1;
                batch.add(*t, (pos + i) as i32, &[0], is_last).map_err(|e| anyhow!("{e}"))?;
            }
            ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
            pos += part.len();
        }

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(model.n_vocab(), 64, 1.08, 0.0, 0.0),
            LlamaSampler::greedy(),
        ]);
        let mut out = String::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut n_cur = n_prompt as i32;
        for _ in 0..max_tokens {
            if let Priority::Interactive(id) = priority {
                if self.epoch.load(Ordering::SeqCst) != id {
                    return Err(anyhow!("interrompu"));
                }
            }
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            pending.extend(vocab.token_to_piece(token, false, None));
            // n'émet que des caractères UTF-8 complets
            let valid = match std::str::from_utf8(&pending) {
                Ok(s) => s.len(),
                Err(e) => e.valid_up_to(),
            };
            if valid > 0 {
                let piece = String::from_utf8_lossy(&pending[..valid]).to_string();
                pending.drain(..valid);
                out.push_str(&piece);
                if out.contains("<|im_end|>") {
                    out = out.replace("<|im_end|>", "");
                    break;
                }
                if !on_piece(&piece) {
                    break;
                }
            }
            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| anyhow!("{e}"))?;
            n_cur += 1;
            ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
        }
        Ok(clean(&out))
    }
}

/// Retire d'éventuels restes de balises de réflexion.
fn clean(s: &str) -> String {
    let mut t = s.to_string();
    if let Some(i) = t.find("</think>") {
        t = t[i + 8..].to_string();
    }
    t.replace("<think>", "").trim().to_string()
}

pub fn lang_name(code: &str) -> &'static str {
    match code {
        "en" => "anglais",
        "es" => "espagnol",
        "it" => "italien",
        "de" => "allemand",
        "pt" => "portugais",
        "ru" => "russe",
        "fr" => "français",
        "ja" => "japonais",
        "zh" => "chinois",
        _ => "étranger",
    }
}

#[allow(dead_code)]
pub const SYSTEM: &str = "Tu es le moteur de traduction de Lumen, une application d'apprentissage des langues. Tu réponds toujours en français, brièvement et avec exactitude, sans formule de politesse ni commentaire superflu.";

const WORD_SYSTEM: &str = "Tu es un dictionnaire bilingue pour apprenants francophones. On te donne une phrase étrangère et un mot ou une expression de cette phrase. Tu réponds sur deux lignes exactement :\nSens : la traduction française du mot dans cette phrase précise, accordée au contexte (1 à 5 mots, jamais le mot original recopié)\nNote : seulement s'il s'agit d'une expression figée, d'un faux ami ou d'un sens inattendu ; sinon écris -";

fn word_query(lang: &str, word: &str, sentence: &str, hint: &str) -> String {
    let mut q = format!("Langue : {}\nPhrase : {sentence}\nMot : {word}", lang_name(lang));
    if !hint.is_empty() {
        q.push_str(&format!("\nDictionnaire : {hint}"));
    }
    q
}

/// Conversation complète (avec exemples) pour la traduction d'un mot en contexte.
pub fn word_messages(lang: &str, word: &str, sentence: &str, hint: &str) -> Vec<(&'static str, String)> {
    vec![
        ("system", WORD_SYSTEM.to_string()),
        ("user", word_query("en", "ran", "She ran to the station to catch the last train.", "forme de run : courir")),
        ("assistant", "Sens : courut\nNote : -".into()),
        ("user", word_query("es", "embarazada", "Mi hermana está embarazada de tres meses.", "enceinte")),
        ("assistant", "Sens : enceinte\nNote : faux ami, ne veut pas dire « embarrassée »".into()),
        ("user", word_query("de", "verpasst", "Er hat den letzten Bus verpasst.", "forme de verpassen : manquer, rater")),
        ("assistant", "Sens : raté\nNote : -".into()),
        ("user", word_query("it", "in bocca al lupo", "Domani hai l'esame? In bocca al lupo!", "")),
        ("assistant", "Sens : bonne chance\nNote : expression figée, littéralement « dans la gueule du loup »".into()),
        ("user", word_query("ru", "читала", "Вчера она читала книгу до полуночи.", "forme de читать : lire")),
        ("assistant", "Sens : lisait\nNote : -".into()),
        ("user", word_query(lang, word, sentence, hint)),
    ]
}

pub fn sentence_messages(lang: &str, sentence: &str) -> Vec<(&'static str, String)> {
    vec![
        ("system", "Tu es un traducteur professionnel. Tu traduis vers le français de façon naturelle et fidèle. Tu réponds uniquement par la traduction française, sans guillemets ni commentaire.".to_string()),
        ("user", "Phrase en espagnol : El faro estaba lejos de la casa, pero se veía su luz.".into()),
        ("assistant", "Le phare était loin de la maison, mais on voyait sa lumière.".into()),
        ("user", "Phrase en anglais : I wish I had known that before.".into()),
        ("assistant", "J'aurais aimé le savoir avant.".into()),
        ("user", format!("Phrase en {} : {sentence}", lang_name(lang))),
    ]
}

pub fn simplify_messages(lang: &str, level: &str, text: &str) -> Vec<(&'static str, String)> {
    let l = lang_name(lang);
    vec![
        ("system", format!("Tu réécris des textes en {l} pour des apprenants. Tu écris uniquement en {l}.")),
        ("user", format!("Réécris ce texte en {l} pour un apprenant de niveau {level} : phrases courtes, vocabulaire courant, mêmes faits et même ordre. Écris uniquement le texte réécrit, sans titre, sans traduction et sans commentaire.\n\n{text}")),
    ]
}

/// Analyse la réponse au format « Sens : … / Note : … ».
pub fn parse_word_answer(raw: &str) -> (String, String) {
    let mut sense = String::new();
    let mut note = String::new();
    for line in raw.lines() {
        let l = line.trim().trim_start_matches(['*', '-', ' ']);
        let lower = l.to_lowercase();
        if lower.starts_with("sens") {
            sense = after_colon(l);
        } else if lower.starts_with("note") {
            note = after_colon(l);
        } else if sense.is_empty() && !l.is_empty() {
            sense = l.to_string();
        }
    }
    let strip = |s: String| {
        let mut t = s.trim().to_string();
        for (a, b) in [('«', '»'), ('"', '"'), ('“', '”'), ('<', '>')] {
            if t.starts_with(a) && t.ends_with(b) && t.chars().count() >= 2 {
                t = t[a.len_utf8()..t.len() - b.len_utf8()].trim().to_string();
            }
        }
        t.trim_end_matches('.').trim().to_string()
    };
    let sense = strip(sense);
    let mut note = strip(note);
    if note == "-" || note == "–" || note.to_lowercase().starts_with("rien") || note.chars().count() < 3 {
        note.clear();
    }
    (sense, note)
}

fn after_colon(s: &str) -> String {
    match s.find([':', '：']) {
        Some(i) => s[i + 1..].trim().to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse() {
        let (s, n) = parse_word_answer("Sens : montait\nNote : imparfait de « subir »");
        assert_eq!(s, "montait");
        assert_eq!(n, "imparfait de « subir »");
        let (s, n) = parse_word_answer("Sens: phare\nNote: -");
        assert_eq!(s, "phare");
        assert_eq!(n, "");
    }
}

#[cfg(test)]
mod live {
    use super::*;
    /// Test réel (ignoré par défaut) : LUMEN_TEST_MODEL=/chemin/modele.gguf cargo test --lib -- --ignored
    #[test]
    #[ignore]
    fn qwen_translates_in_context() {
        let Ok(path) = std::env::var("LUMEN_TEST_MODEL") else { return };
        let engine = Engine::new();
        let cases = [
            ("en", "chose", "She chose the red dress for the party.", "forme de choose : choisir"),
            ("en", "damp", "The paper was damp, but the words could still be read.", "humide ; moite"),
            ("es", "subía", "Cada mañana, Marta subía las escaleras del viejo faro.", ""),
            ("it", "andavo", "Quando ero piccolo andavo sempre al mare con mio nonno.", "forme de andare : aller"),
            ("de", "ging", "Gestern ging ich nach der Arbeit in den Park.", "forme de gehen : aller ; marcher"),
            ("pt", "farol", "Todas as manhãs, Marta subia as escadas do velho farol.", "phare"),
            ("ru", "нашла", "Однажды она нашла письмо, спрятанное между двумя камнями.", ""),
            ("en", "looked out", "She looked out at the sea, as if the answer might come.", ""),
        ];
        for (lang, word, sentence, hint) in cases {
            let t = std::time::Instant::now();
            let raw = engine
                .generate(Path::new(&path), &word_messages(lang, word, sentence, hint), 72, Priority::Background, |_| true)
                .unwrap();
            let (s, n) = parse_word_answer(&raw);
            println!("[{lang}] {word} -> « {s} » | note : {n} ({:?})\n   brut : {raw:?}", t.elapsed());
            assert!(!s.is_empty());
        }
        let out = engine
            .generate(Path::new(&path), &sentence_messages("en", "If you are reading this, you are not alone."), 120, Priority::Background, |_| true)
            .unwrap();
        println!("phrase -> {out}");
    }
}
