//! IA locale : Qwen3.5 exécuté par llama.cpp (Metal sur Mac).
//! Sert à la traduction contextuelle des mots, des phrases, à la
//! réécriture de textes à un niveau plus simple et au chat.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

pub(crate) fn backend() -> Result<&'static LlamaBackend> {
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
    /// conversation : arrêtée seulement par le bouton « Arrêter » (le drapeau),
    /// jamais par un clic sur un mot ; ce qui est déjà écrit est gardé
    Stoppable(Arc<AtomicBool>),
}

/// Choix de chaque jeton.
pub enum Sampling {
    /// toujours le plus probable : traductions stables, reproductibles (et mises en cache)
    Exact,
    /// un peu de variété, comme Qwen le recommande pour converser (et indispensable
    /// en réflexion : le choix systématique du plus probable la fait tourner en rond)
    Natural,
}

pub struct Gen {
    /// longueur maximale de la réponse (réflexion non comprise)
    pub max_tokens: usize,
    /// réflexion autorisée (mode « thinking » de Qwen3.5), avec son budget de jetons
    pub think: Option<usize>,
    pub sampling: Sampling,
    pub priority: Priority,
}

/// Texte produit au fil de l'eau : la réflexion d'abord (si elle est permise), puis la réponse.
pub enum Piece<'a> {
    Thought(&'a str),
    Answer(&'a str),
}

pub struct Output {
    pub thought: String,
    pub answer: String,
    /// arrêtée par l'utilisateur (la réponse peut être partielle)
    pub stopped: bool,
}

/// Budget de réflexion épuisé : on clôt la réflexion comme le préconise Qwen
/// (« thinking budget »), et le modèle répond avec ce qu'il a déjà pensé.
const THINK_STOP: &str = "\n\nConsidering the limited time by the user, I have to give the solution based on the thinking directly now.\n</think>\n\n";

/// Longueur de réflexion selon l'effort choisi. Qwen3.5 n'a pas de réglage
/// d'effort à proprement parler : on borne le nombre de jetons de réflexion.
pub fn think_budget(effort: &str) -> usize {
    match effort {
        "low" => 512,
        "high" => 4096,
        _ => 1536,
    }
}

impl Engine {
    pub fn new() -> Self {
        Self { loaded: Mutex::new(None), run_lock: Mutex::new(()), epoch: AtomicU64::new(0) }
    }

    /// Travail exclusif sur le GPU (un morceau de transcription, par exemple) :
    /// attend la fin de la génération en cours, et la suivante attendra celui-ci.
    pub fn exclusive<T>(&self, f: impl FnOnce() -> T) -> T {
        let _run = self.run_lock.lock();
        f()
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
        let model = LlamaModel::load_from_file(be, path, &params).map_err(|e| anyhow!(crate::tr!("modèle illisible : {e}", "unreadable model: {e}")))?;
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
        let g = Gen { max_tokens, think: None, sampling: Sampling::Exact, priority };
        let out = self.run(model_path, messages, g, |p| match p {
            Piece::Answer(t) => on_piece(t),
            Piece::Thought(_) => true,
        })?;
        Ok(clean(&out.answer))
    }

    /// Génération complète : réflexion facultative (bornée par son budget),
    /// puis réponse. La réflexion et la réponse arrivent séparément dans `on_piece`.
    pub fn run(
        &self,
        model_path: &Path,
        messages: &[(&str, String)],
        g: Gen,
        mut on_piece: impl FnMut(Piece) -> bool,
    ) -> Result<Output> {
        let _run = self.run_lock.lock();
        let mut out = Output { thought: String::new(), answer: String::new(), stopped: false };
        // requête dépassée (interactive) ou arrêtée (conversation) pendant l'attente du moteur
        let halted = |out: &mut Output| -> Result<bool> {
            match &g.priority {
                Priority::Interactive(id) if self.epoch.load(Ordering::SeqCst) != *id => Err(anyhow!("interrompu")),
                Priority::Stoppable(flag) if flag.load(Ordering::Relaxed) => {
                    out.stopped = true;
                    Ok(true)
                }
                _ => Ok(false),
            }
        };
        if halted(&mut out)? {
            return Ok(out);
        }
        let model = self.load(model_path)?;
        let be = backend()?;
        let mut prompt = String::new();
        for (role, content) in messages {
            prompt.push_str(&format!("<|im_start|>{role}\n{content}<|im_end|>\n"));
        }
        // même ouverture que le modèle de conversation de Qwen3.5 (enable_thinking)
        prompt.push_str(if g.think.is_some() { "<|im_start|>assistant\n<think>\n" } else { "<|im_start|>assistant\n<think>\n\n</think>\n\n" });
        let vocab = model.vocab();
        let mut tokens = vocab.tokenize(prompt.as_bytes(), false, true);
        // jeton de début : attendu par MiniCPM (« <s> »), absent chez Qwen
        if vocab.should_add_bos() || std::env::var("LUMEN_FORCE_BOS").is_ok() {
            tokens.insert(0, vocab.bos());
        }
        let stop_tokens = vocab.tokenize(THINK_STOP.as_bytes(), false, true);
        // fin de réflexion : un seul jeton spécial chez Qwen (sinon, repérée dans le texte)
        let think_end = {
            let t = vocab.tokenize(b"</think>", false, true);
            (t.len() == 1).then(|| t[0])
        };
        let budget = g.think.unwrap_or(0);
        let n_prompt = tokens.len();
        let reserve = if g.think.is_some() { budget + stop_tokens.len() } else { 0 };
        let n_ctx = (n_prompt + reserve + g.max_tokens + 16).max(512) as u32;
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
            if halted(&mut out)? {
                return Ok(out);
            }
            batch.clear();
            for (i, t) in part.iter().enumerate() {
                let is_last = pos + i == n_prompt - 1;
                batch.add(*t, (pos + i) as i32, &[0], is_last).map_err(|e| anyhow!("{e}"))?;
            }
            ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
            pos += part.len();
        }

        let mut sampler = match g.sampling {
            Sampling::Exact => LlamaSampler::chain_simple([
                LlamaSampler::penalties(model.n_vocab(), 64, 1.08, 0.0, 0.0),
                LlamaSampler::greedy(),
            ]),
            Sampling::Natural => {
                // réglages de Qwen, un peu resserrés : un petit modèle invente moins
                // quand il s'écarte peu des mots les plus probables
                let (temp, top_p) = if g.think.is_some() { (0.6, 0.95) } else { (0.5, 0.8) };
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7);
                LlamaSampler::chain_simple([
                    LlamaSampler::penalties(model.n_vocab(), 128, 1.0, 0.0, 0.5),
                    // « DRY » : pénalise la reprise de suites entières déjà écrites (les petits
                    // modèles tournent vite en boucle), sans gêner les mots courts et fréquents
                    LlamaSampler::dry(&model, 0.8, 1.75, 2, 1024, ["\n", ":", "\"", "*", "«", "»"]),
                    LlamaSampler::top_k(20),
                    LlamaSampler::top_p(top_p, 1),
                    LlamaSampler::temp(temp),
                    LlamaSampler::dist(seed),
                ])
            }
        };
        let mut thinking = g.think.is_some();
        let mut thought_tokens = 0usize;
        let mut answer_tokens = 0usize;
        let mut pending: Vec<u8> = Vec::new();
        let mut n_cur = n_prompt as i32;
        loop {
            if halted(&mut out)? {
                break;
            }
            if thinking && thought_tokens >= budget {
                // budget épuisé : la réflexion est close d'autorité
                batch.clear();
                for (i, t) in stop_tokens.iter().enumerate() {
                    batch.add(*t, n_cur + i as i32, &[0], i == stop_tokens.len() - 1).map_err(|e| anyhow!("{e}"))?;
                }
                ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
                n_cur += stop_tokens.len() as i32;
                // la réponse repart de zéro : elle peut reprendre les idées de la réflexion
                sampler.reset();
                pending.clear();
                thinking = false;
                continue;
            }
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                if thinking {
                    // fin de tour en pleine réflexion : on la clôt pour obtenir une réponse
                    thought_tokens = budget;
                    continue;
                }
                break;
            }
            if thinking && Some(token) == think_end {
                sampler.reset();
                pending.clear();
                thinking = false;
            } else {
                pending.extend(vocab.token_to_piece(token, false, None));
                // n'émet que des caractères UTF-8 complets
                let valid = match std::str::from_utf8(&pending) {
                    Ok(s) => s.len(),
                    Err(e) => e.valid_up_to(),
                };
                if valid > 0 {
                    let piece = String::from_utf8_lossy(&pending[..valid]).to_string();
                    pending.drain(..valid);
                    if thinking {
                        out.thought.push_str(&piece);
                        if let Some(i) = out.thought.find("</think>") {
                            // balise écrite en toutes lettres : la suite est déjà la réponse
                            let rest = out.thought[i + 8..].to_string();
                            out.thought.truncate(i);
                            sampler.reset();
                            thinking = false;
                            if !answer_piece(&mut out, &rest, &mut on_piece) {
                                break;
                            }
                        } else if !on_piece(Piece::Thought(&piece)) {
                            break;
                        }
                    } else if let Some(i) = piece.find("<|im_end|>") {
                        answer_piece(&mut out, &piece[..i], &mut on_piece);
                        break;
                    } else if !answer_piece(&mut out, &piece, &mut on_piece) {
                        break;
                    }
                }
            }
            if thinking {
                thought_tokens += 1;
            } else {
                answer_tokens += 1;
                if answer_tokens >= g.max_tokens {
                    break;
                }
            }
            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| anyhow!("{e}"))?;
            n_cur += 1;
            ctx.decode(&mut batch).map_err(|e| anyhow!("décodage : {e}"))?;
        }
        out.thought = out.thought.trim().to_string();
        Ok(out)
    }
}

/// Ajoute un morceau de réponse (sans les sauts de ligne qui suivent la
/// réflexion) et le transmet. `false` : arrêt demandé par l'appelant.
fn answer_piece(out: &mut Output, piece: &str, on_piece: &mut impl FnMut(Piece) -> bool) -> bool {
    let piece = if out.answer.is_empty() { piece.trim_start() } else { piece };
    if piece.is_empty() {
        return true;
    }
    out.answer.push_str(piece);
    on_piece(Piece::Answer(piece))
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
        "nl" => "néerlandais",
        "sv" => "suédois",
        "da" => "danois",
        "fi" => "finnois",
        "et" => "estonien",
        "lv" => "letton",
        "lt" => "lituanien",
        "pl" => "polonais",
        "cs" => "tchèque",
        "sk" => "slovaque",
        "sl" => "slovène",
        "hr" => "croate",
        "hu" => "hongrois",
        "ro" => "roumain",
        "bg" => "bulgare",
        "uk" => "ukrainien",
        "el" => "grec",
        "tr" => "turc",
        "ar" => "arabe",
        "hi" => "hindi",
        "id" => "indonésien",
        "vi" => "vietnamien",
        "ko" => "coréen",
        "ja" => "japonais",
        "zh" => "chinois",
        _ => "étranger",
    }
}

/// Nom anglais d'une langue (consignes pour un apprenant anglophone).
pub fn lang_name_en(code: &str) -> &'static str {
    match code {
        "en" => "English",
        "es" => "Spanish",
        "it" => "Italian",
        "de" => "German",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "fr" => "French",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "da" => "Danish",
        "fi" => "Finnish",
        "et" => "Estonian",
        "lv" => "Latvian",
        "lt" => "Lithuanian",
        "pl" => "Polish",
        "cs" => "Czech",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "bg" => "Bulgarian",
        "uk" => "Ukrainian",
        "el" => "Greek",
        "tr" => "Turkish",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "ko" => "Korean",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => "foreign",
    }
}

#[allow(dead_code)]
pub const SYSTEM: &str = "Tu es le moteur de traduction de Lumen, une application d'apprentissage des langues. Tu réponds toujours en français, brièvement et avec exactitude, sans formule de politesse ni commentaire superflu.";

// Les consignes existent pour un apprenant francophone (`native` = "fr") et
// anglophone ("en") ; les versions françaises sont celles qui ont été éprouvées.

const WORD_SYSTEM: &str = "Tu es un dictionnaire bilingue pour apprenants francophones. On te donne une phrase étrangère et un mot ou une expression de cette phrase. Tu réponds sur deux lignes exactement :\nSens : la traduction française du mot dans cette phrase précise, accordée au contexte (1 à 5 mots, jamais le mot original recopié)\nNote : seulement s'il s'agit d'une expression figée, d'un faux ami ou d'un sens inattendu ; sinon écris -";

const WORD_SYSTEM_EN: &str = "You are a bilingual dictionary for English-speaking learners. You are given a foreign sentence and a word or expression from that sentence. You answer in exactly two lines:\nMeaning: the English translation of the word in this precise sentence, matching the context (1 to 5 words, never the original word copied)\nNote: only if it is a fixed expression, a false friend or an unexpected meaning; otherwise write -";

fn word_query(lang: &str, word: &str, sentence: &str, hint: &str) -> String {
    let mut q = format!("Langue : {}\nPhrase : {sentence}\nMot : {word}", lang_name(lang));
    if !hint.is_empty() {
        q.push_str(&format!("\nDictionnaire : {hint}"));
    }
    q
}

fn word_query_en(lang: &str, word: &str, sentence: &str, hint: &str) -> String {
    let mut q = format!("Language: {}\nSentence: {sentence}\nWord: {word}", lang_name_en(lang));
    if !hint.is_empty() {
        q.push_str(&format!("\nDictionary: {hint}"));
    }
    q
}

/// Conversation complète (avec exemples) pour la traduction d'un mot en contexte,
/// dans la langue de l'apprenant.
pub fn word_messages(native: &str, lang: &str, word: &str, sentence: &str, hint: &str) -> Vec<(&'static str, String)> {
    if native == "en" {
        return vec![
            ("system", WORD_SYSTEM_EN.to_string()),
            ("user", word_query_en("fr", "pris", "Elle a pris le dernier train pour Lyon.", "form of prendre: to take")),
            ("assistant", "Meaning: took\nNote: -".into()),
            ("user", word_query_en("es", "embarazada", "Mi hermana está embarazada de tres meses.", "pregnant")),
            ("assistant", "Meaning: pregnant\nNote: false friend, it does not mean \"embarrassed\"".into()),
            ("user", word_query_en("de", "verpasst", "Er hat den letzten Bus verpasst.", "form of verpassen: to miss")),
            ("assistant", "Meaning: missed\nNote: -".into()),
            ("user", word_query_en("it", "in bocca al lupo", "Domani hai l'esame? In bocca al lupo!", "")),
            ("assistant", "Meaning: good luck\nNote: fixed expression, literally \"in the wolf's mouth\"".into()),
            ("user", word_query_en("ru", "читала", "Вчера она читала книгу до полуночи.", "form of читать: to read")),
            ("assistant", "Meaning: was reading\nNote: -".into()),
            ("user", word_query_en(lang, word, sentence, hint)),
        ];
    }
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

pub fn sentence_messages(native: &str, lang: &str, sentence: &str) -> Vec<(&'static str, String)> {
    if native == "en" {
        return vec![
            ("system", "You are a professional translator. You translate into English, naturally and faithfully. You reply only with the English translation, without quotation marks or comments.".to_string()),
            ("user", "Sentence in Spanish: El faro estaba lejos de la casa, pero se veía su luz.".into()),
            ("assistant", "The lighthouse was far from the house, but its light could be seen.".into()),
            ("user", "Sentence in French: J'aurais aimé le savoir avant.".into()),
            ("assistant", "I wish I had known that before.".into()),
            ("user", format!("Sentence in {}: {sentence}", lang_name_en(lang))),
        ];
    }
    vec![
        ("system", "Tu es un traducteur professionnel. Tu traduis vers le français de façon naturelle et fidèle. Tu réponds uniquement par la traduction française, sans guillemets ni commentaire.".to_string()),
        ("user", "Phrase en espagnol : El faro estaba lejos de la casa, pero se veía su luz.".into()),
        ("assistant", "Le phare était loin de la maison, mais on voyait sa lumière.".into()),
        ("user", "Phrase en anglais : I wish I had known that before.".into()),
        ("assistant", "J'aurais aimé le savoir avant.".into()),
        ("user", format!("Phrase en {} : {sentence}", lang_name(lang))),
    ]
}

pub fn simplify_messages(native: &str, lang: &str, level: &str, text: &str) -> Vec<(&'static str, String)> {
    if native == "en" {
        let l = lang_name_en(lang);
        return vec![
            ("system", format!("You rewrite texts in {l} for learners. You write only in {l}.")),
            ("user", format!("Rewrite this text in {l} for a learner at level {level}: short sentences, common vocabulary, same facts in the same order. Write only the rewritten text, with no title, no translation and no comments.\n\n{text}")),
        ];
    }
    let l = lang_name(lang);
    vec![
        ("system", format!("Tu réécris des textes en {l} pour des apprenants. Tu écris uniquement en {l}.")),
        ("user", format!("Réécris ce texte en {l} pour un apprenant de niveau {level} : phrases courtes, vocabulaire courant, mêmes faits et même ordre. Écris uniquement le texte réécrit, sans titre, sans traduction et sans commentaire.\n\n{text}")),
    ]
}

// ---------- chat ----------

const CHAT_SYSTEM: &str = "Tu es Lumen, un professeur de langues expert, patient et chaleureux, intégré à une application qui aide à apprendre les langues en lisant et en écoutant. Tu maîtrises la grammaire, la conjugaison, le vocabulaire, la prononciation, les expressions idiomatiques, les registres de langue et la culture des pays concernés, et tu sais rendre tout cela simple.

Règles :
- C'est l'apprenant qui mène la conversation. Réponds à ce qu'il demande, et seulement à cela : pas d'exercice, de quiz, de liste de mots ni de nouveau sujet qu'il n'a pas demandés.
- Termine ta réponse quand tu as répondu : pas de proposition, de question pour relancer ni d'encouragement à la fin (« Veux-tu que… ? », « N'hésite pas… », « Bravo ! »). S'il veut aller plus loin, il te le dira.
- Réponds en français, sauf si l'apprenant t'écrit dans une autre langue pour s'entraîner : réponds-lui alors dans cette langue, avec des phrases à sa portée, comme un interlocuteur qui suit le sujet qu'il a choisi ; puis, seulement s'il a fait des fautes, corrige-les en une ligne, en français.
- S'il te propose de converser sans choisir de sujet, demande-lui de quoi il veut parler.
- Sois précis et concis : va droit au but, sans formule de politesse superflue.
- Donne des exemples dans la langue étudiée, chacun suivi de sa traduction française.
- Pour un mot ou une expression : son sens dans le contexte, sa forme de base, sa nature grammaticale, puis un ou deux exemples.
- Si tu n'es pas sûr de quelque chose, dis-le au lieu d'inventer.
- Mise en forme sobre : paragraphes courts, listes, **gras** pour les mots clés, tableaux seulement pour les conjugaisons et les déclinaisons.";

/// Rappel ajouté à la question : un petit modèle suit mieux une règle lue
/// juste avant de répondre que la même règle perdue en tête des consignes.
const CHAT_LEAD: &str = "(Réponds seulement à ce que je demande, sans rien me proposer à la fin.)";

const CHAT_SYSTEM_EN: &str = "You are Lumen, an expert, patient and warm language teacher, built into an app that helps people learn languages by reading and listening. You master grammar, conjugation, vocabulary, pronunciation, idioms, registers and the culture of the countries concerned, and you know how to make all of it simple.

Rules:
- The learner leads the conversation. Answer what they ask, and only that: no exercises, quizzes, word lists or new topics they didn't ask for.
- End your answer once you have answered: no offer, follow-up question or encouragement at the end (\"Would you like me to…?\", \"Feel free to…\", \"Great job!\"). If they want to go further, they will tell you.
- Answer in English, unless the learner writes to you in another language to practise: then answer in that language, with sentences they can understand, like someone following the topic they chose; then, only if they made mistakes, correct them in one line, in English.
- If they offer to chat without choosing a topic, ask them what they would like to talk about.
- Be precise and concise: get straight to the point, without unnecessary pleasantries.
- Give examples in the language being studied, each followed by its English translation.
- For a word or expression: its meaning in context, its base form, its part of speech, then one or two examples.
- If you are not sure about something, say so instead of making it up.
- Keep formatting simple: short paragraphs, lists, **bold** for key words, tables only for conjugations and declensions.";

const CHAT_LEAD_EN: &str = "(Answer only what I ask, without offering anything else at the end.)";

/// Ce que le chat sait de l'apprenant : ses mots connus, et son profil (nom,
/// ce qui le motive, centres d'intérêt, accords), lu par `user::learner`.
#[derive(Default, Clone, Debug)]
pub struct Learner {
    pub known: i64,
    pub name: String,
    pub why: String,
    pub interests: Vec<String>,
    /// accords quand le chat s'adresse à lui : féminin, masculin, ou sans préférence
    pub feminine: Option<bool>,
}

#[cfg(test)]
impl Learner {
    /// Un apprenant dont on ne sait que le nombre de mots connus.
    pub fn knows(known: i64) -> Self {
        Learner { known, ..Default::default() }
    }
}

/// Leçon jointe à une conversation.
pub struct LessonContext<'a> {
    pub title: &'a str,
    pub text: &'a str,
    /// extrait seulement : la leçon entière ne tenait pas
    pub partial: bool,
}

/// « l'italien », « le russe » : nom de la langue avec son article.
pub fn lang_with_article(code: &str) -> String {
    let n = lang_name(code);
    // h muet dans « l'hindi », aspiré dans « le hongrois »
    if n.starts_with(['a', 'e', 'é', 'i', 'o', 'u']) || n == "hindi" {
        format!("l'{n}")
    } else {
        format!("le {n}")
    }
}

/// Conversation complète pour le chat : consignes, apprenant, leçon jointe,
/// échanges précédents (réflexions écartées, comme le veut Qwen) et question.
pub fn chat_messages(
    native: &str,
    lang: &str,
    learner: &Learner,
    lesson: Option<&LessonContext>,
    history: &[(String, String)],
    question: &str,
    hints: &[String],
) -> Vec<(&'static str, String)> {
    if native == "en" {
        return chat_messages_en(lang, learner, lesson, history, question, hints);
    }
    let known = learner.known;
    // « elle » si l'apprenante l'a demandé dans son profil, sinon le « il » de « l'apprenant »
    let il = if learner.feminine == Some(true) { "Elle" } else { "Il" };
    let mut system = String::from(CHAT_SYSTEM);
    system.push_str(&format!("\n\n{} est francophone. ", if learner.feminine == Some(true) { "L'apprenante" } else { "L'apprenant" }));
    if known < 50 {
        system.push_str(&format!("{il} débute en {}.", lang_name(lang)));
    } else {
        system.push_str(&format!("{il} étudie {} et connaît déjà environ {known} mots dans cette langue.", lang_with_article(lang)));
    }
    if !learner.name.is_empty() {
        system.push_str(&format!(" {il} s'appelle {} : tu peux l'appeler ainsi de temps en temps, sans en abuser.", learner.name));
    }
    match learner.feminine {
        Some(true) => system.push_str(" Quand tu t'adresses à elle, accorde au féminin, en français comme dans la langue étudiée."),
        Some(false) => system.push_str(" Quand tu t'adresses à lui, accorde au masculin, en français comme dans la langue étudiée."),
        None => {}
    }
    // centres d'intérêt et motivation : seulement quand l'apprenant demande au
    // chat de choisir un sujet ou des exemples ; ressortis à tout propos, ils
    // détournent les réponses vers des sujets qu'il n'a pas choisis
    let mut tastes = Vec::new();
    if !learner.interests.is_empty() {
        tastes.push(format!("ses centres d'intérêt ({})", learner.interests.join(", ")));
    }
    if !learner.why.is_empty() {
        tastes.push(format!("ce qui {} motive (« {} »)", if learner.feminine == Some(true) { "la" } else { "le" }, learner.why));
    }
    if !tastes.is_empty() {
        let si = if learner.feminine == Some(true) { "si elle" } else { "s'il" };
        system.push_str(&format!(" Seulement {si} te demande de choisir un sujet ou des exemples, inspire-toi de {} ; sinon, n'en parle pas.", tastes.join(" et de ")));
    }
    if let Some(l) = lesson {
        let part = if l.partial { " (ce n'est qu'un extrait, autour du passage qu'il lit : la leçon complète est plus longue)" } else { "" };
        system.push_str(&format!(
            "\n\nL'apprenant lit en ce moment la leçon « {} », en {}. Appuie-toi sur ce texte quand il t'interroge dessus{part}.\n\n<leçon>\n{}\n</leçon>",
            l.title.trim(),
            lang_name(lang),
            l.text.trim()
        ));
    }
    let mut out = vec![("system", system)];
    for (role, content) in history {
        out.push((if role == "assistant" { "assistant" } else { "user" }, content.trim().to_string()));
    }
    let mut q = question.trim().to_string();
    let practice = lang != "fr" && !looks_french(&q);
    if practice {
        // l'apprenant écrit dans une autre langue pour s'entraîner : un petit modèle
        // suit mieux une consigne précise, au bon endroit, qu'une règle générale
        q.push_str(&format!(
            "\n\n(Je m'entraîne : réponds-moi en {}, en quelques phrases simples, comme dans une vraie conversation sur le sujet que j'ai choisi. Si mon message contient des fautes, termine par une seule ligne « Corrections : » qui les corrige, en français.)",
            lang_name(lang)
        ));
    }
    if !hints.is_empty() {
        // comme pour le panneau du mot : des repères sûrs évitent les sens inventés
        q.push_str(&format!("\n\n(Dictionnaire, sens possibles à choisir selon le contexte : {})", hints.join(" ; ")));
    }
    if !practice {
        q.push_str("\n\n");
        q.push_str(CHAT_LEAD);
    }
    out.push(("user", q));
    out
}

fn chat_messages_en(
    lang: &str,
    learner: &Learner,
    lesson: Option<&LessonContext>,
    history: &[(String, String)],
    question: &str,
    hints: &[String],
) -> Vec<(&'static str, String)> {
    let name = lang_name_en(lang);
    let known = learner.known;
    let mut system = String::from(CHAT_SYSTEM_EN);
    system.push_str("\n\nThe learner is an English speaker. ");
    if known < 50 {
        system.push_str(&format!("They are a beginner in {name}."));
    } else {
        system.push_str(&format!("They are learning {name} and already know about {known} words in this language."));
    }
    if !learner.name.is_empty() {
        system.push_str(&format!(" Their name is {}: you can call them by it now and then, without overdoing it.", learner.name));
    }
    match learner.feminine {
        Some(true) => system.push_str(&format!(" When addressing them in a language with grammatical gender, such as {name}, use feminine agreement.")),
        Some(false) => system.push_str(&format!(" When addressing them in a language with grammatical gender, such as {name}, use masculine agreement.")),
        None => {}
    }
    let mut tastes = Vec::new();
    if !learner.interests.is_empty() {
        tastes.push(format!("their interests ({})", learner.interests.join(", ")));
    }
    if !learner.why.is_empty() {
        tastes.push(format!("what motivates them (\"{}\")", learner.why));
    }
    if !tastes.is_empty() {
        system.push_str(&format!(" Only if they ask you to choose a topic or examples, draw on {}; otherwise, don't mention them.", tastes.join(" and ")));
    }
    if let Some(l) = lesson {
        let part = if l.partial { " (this is only an excerpt, around the passage they are reading: the full lesson is longer)" } else { "" };
        system.push_str(&format!(
            "\n\nThe learner is currently reading the lesson \"{}\", in {name}. Rely on this text when they ask about it{part}.\n\n<lesson>\n{}\n</lesson>",
            l.title.trim(),
            l.text.trim()
        ));
    }
    let mut out = vec![("system", system)];
    for (role, content) in history {
        out.push((if role == "assistant" { "assistant" } else { "user" }, content.trim().to_string()));
    }
    let mut q = question.trim().to_string();
    let practice = lang != "en" && !looks_english(&q);
    if practice {
        q.push_str(&format!(
            "\n\n(I'm practising my {name}: answer me in {name}, in a few simple sentences, like in a real conversation on the topic I chose. If my message contains mistakes, end with a single line \"Corrections:\" that fixes them, written in English.)"
        ));
    }
    if !hints.is_empty() {
        q.push_str(&format!("\n\n(Dictionary, possible meanings to choose from depending on the context: {})", hints.join("; ")));
    }
    if !practice {
        q.push_str("\n\n");
        q.push_str(CHAT_LEAD_EN);
    }
    out.push(("user", q));
    out
}

/// Repère du dictionnaire pour un mot cité dans le chat, par exemple
/// « andavo : première personne du singulier de l'indicatif imparfait du verbe
/// andare (andare : aller) ». Seules les formes décrites par le dictionnaire
/// mènent à leur forme de base : les autres renvois y sont trop peu sûrs.
pub fn dict_hint(dicts: &crate::dict::Dicts, native: &str, lang: &str, word: &str) -> Option<String> {
    if !dicts.available_in(native, lang) {
        return None;
    }
    let d = dicts.lookup_in(native, lang, word).ok()?;
    let en = native == "en";
    // « andare : aller » en français, « andare: to go » en anglais
    let colon = if en { ": " } else { " : " };
    let short = |g: &str| -> String {
        let g = g.trim().trim_end_matches('.');
        if g.chars().count() <= 70 {
            return g.to_string();
        }
        let cut: String = g.chars().take(70).collect();
        format!("{}…", cut[..cut.rfind([' ', ',']).unwrap_or(cut.len())].trim_end_matches([',', ' ']))
    };
    let glosses = |entries: &[crate::dict::DictEntry]| -> String {
        entries.iter().flat_map(|e| e.glosses.iter().take(2)).take(3).map(|g| short(g).to_lowercase()).collect::<Vec<_>>().join(", ")
    };
    let mut parts: Vec<String> = Vec::new();
    if let (Some(lemma), Some(note)) = (&d.lemma, &d.form_note) {
        let base = dicts.lookup_in(native, lang, lemma).map(|r| glosses(&r.entries)).unwrap_or_default();
        let note = note.trim().trim_end_matches('.');
        let note = note.chars().next().map(|c| c.to_lowercase().collect::<String>() + &note[c.len_utf8()..]).unwrap_or_default();
        parts.push(if base.is_empty() { note } else { format!("{note} ({lemma}{colon}{base})") });
    }
    // sens propres du mot, sauf s'il est donné pour la forme d'un autre mot sans
    // plus de détail : ces sens-là sont souvent ceux d'un homonyme (« saliva », la
    // salive, quand le texte parle de monter) et égareraient le modèle
    let doubtful = d.lemma.is_some() && d.form_note.is_none();
    let own: Vec<crate::dict::DictEntry> = if doubtful { Vec::new() } else { d
        .entries
        .iter()
        .filter(|e| !e.pos.starts_with("Forme") && crate::text::normalize(&e.word) == crate::text::normalize(word))
        .cloned()
        .collect() };
    if !own.is_empty() {
        parts.push(format!("{}{colon}{}", own[0].pos.to_lowercase(), glosses(&own)));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("{word}{colon}{}", parts.join(if en { "; or " } else { " ; ou " })))
}

/// Le message est-il écrit en français ? (Sinon, l'apprenant s'entraîne dans
/// la langue étudiée.) Les citations entre guillemets ne comptent pas.
pub fn looks_french(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "le", "la", "les", "un", "une", "des", "du", "de", "et", "est", "que", "qui", "quoi", "je", "tu", "il", "elle", "nous", "vous",
        "ce", "ça", "pour", "pas", "dans", "sur", "avec", "mot", "mots", "phrase", "dire", "veut", "comment", "pourquoi", "quel", "quelle",
        "explique", "explique-moi", "peux-tu", "merci", "bonjour", "salut", "oui", "non", "c'est", "qu'est-ce", "j'ai", "moi", "mon", "ma",
        "mes", "leçon", "texte", "traduis", "traduction", "donne-moi", "écris-moi", "aussi", "plus", "très", "ou", "où", "au", "aux",
        "cette", "cet", "ces", "quels", "quelles", "quelques", "résume", "résume-moi", "résumé", "niveau", "discutons", "parlons",
        "pose-moi", "dois-je", "peux", "veux", "suis", "sont", "être", "avoir", "faire", "fait", "différence",
    ];
    // les passages cités (mots de la leçon) ne disent rien de la langue du message
    let mut plain = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '«' | '“' => depth += 1,
            '»' | '”' => depth = (depth - 1).max(0),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let words: Vec<String> = plain
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().replace('’', "'"))
        .collect();
    if words.is_empty() {
        return true;
    }
    let hits = words.iter().filter(|w| MARKERS.contains(&w.as_str()) || w.starts_with("l'") || w.starts_with("d'") || w.starts_with("qu'")).count();
    hits >= 2 || hits * 5 >= words.len()
}

/// Le message est-il écrit en anglais ? (Sinon, l'apprenant anglophone
/// s'entraîne dans la langue étudiée.) Mots choisis pour ne pas exister tels
/// quels dans les langues voisines (« in », « me », « no » sont écartés).
pub fn looks_english(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "the", "what", "what's", "how", "why", "does", "mean", "means", "meaning", "word", "words", "sentence", "you", "your", "this",
        "that", "these", "those", "can", "could", "would", "should", "please", "thanks", "thank", "hello", "yes", "explain", "translate",
        "translation", "lesson", "text", "of", "and", "with", "my", "difference", "between", "say", "use", "when", "which", "example",
        "examples", "are", "it's", "i'm", "give", "tell", "about", "grammar", "verb", "tense",
    ];
    let mut plain = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '«' | '“' => depth += 1,
            '»' | '”' => depth = (depth - 1).max(0),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let words: Vec<String> = plain
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().replace('’', "'"))
        .collect();
    if words.is_empty() {
        return true;
    }
    let hits = words.iter().filter(|w| MARKERS.contains(&w.as_str())).count();
    hits >= 2 || hits * 4 >= words.len()
}

/// Mots cités dans une question (entre « », “ ” ou " "), à chercher au
/// dictionnaire. Les citations longues (une phrase entière) sont laissées de côté.
pub fn quoted_words(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (open, close) in [('«', '»'), ('“', '”'), ('"', '"')] {
        let mut rest = question;
        while let Some(a) = rest.find(open) {
            let after = &rest[a + open.len_utf8()..];
            let Some(b) = after.find(close) else { break };
            let quote = after[..b].trim();
            let words: Vec<&str> = quote.split_whitespace().collect();
            if !words.is_empty() && words.len() <= 3 {
                for w in words {
                    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
                    if !w.is_empty() && !out.iter().any(|x| x == w) {
                        out.push(w.to_string());
                    }
                }
            }
            rest = &after[b + close.len_utf8()..];
        }
    }
    out.truncate(4);
    out
}

/// Texte de la leçon confié au chat : entier s'il tient dans `max` octets, sinon
/// un extrait autour du passage lu (`focus`, position UTF-16 comme dans
/// l'interface), coupé entre deux paragraphes ou deux phrases. Les octets sont
/// une bonne mesure de la place prise : un caractère japonais en compte trois.
pub fn lesson_excerpt(text: &str, focus: Option<usize>, max: usize) -> (String, bool) {
    if text.trim().len() <= max {
        return (text.trim().to_string(), false);
    }
    let at = focus.map(|f| byte_at_utf16(text, f)).unwrap_or(0);
    // un tiers avant le passage lu, deux tiers après : on lit vers l'avant
    let end = (at.saturating_sub(max / 3) + max).min(text.len());
    let mut start = end.saturating_sub(max);
    let mut end = end;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let window = &text[start..end];
    // bords où chercher une coupure : un cinquième de l'extrait de chaque côté
    let mut head = window.len() / 5;
    while !window.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = window.len() - head;
    while !window.is_char_boundary(tail) {
        tail += 1;
    }
    // début : après la première fin de paragraphe (ou de phrase) proche du bord
    let from = if start == 0 { 0 } else { cut_after(&window[..head], true).unwrap_or(0) };
    // fin : après la dernière fin de paragraphe (ou de phrase) proche du bord
    let to = if end == text.len() { window.len() } else { cut_after(&window[tail..], false).map(|i| tail + i).unwrap_or(window.len()) };
    (window[from..to].trim().to_string(), true)
}

/// Position (en octets) juste après une fin de paragraphe, à défaut une fin de
/// phrase : la première (`first`) ou la dernière du morceau.
fn cut_after(s: &str, first: bool) -> Option<usize> {
    let find = |pats: &[&str]| -> Option<usize> {
        let hits = pats.iter().filter_map(|p| if first { s.find(p).map(|i| i + p.len()) } else { s.rfind(p).map(|i| i + p.len()) });
        if first { hits.min() } else { hits.max() }
    };
    find(&["\n"]).or_else(|| find(&[". ", "! ", "? ", "。", "！", "？"]))
}

/// Position en octets d'une position UTF-16 (celle des chaînes JavaScript).
fn byte_at_utf16(text: &str, pos16: usize) -> usize {
    let mut n16 = 0;
    for (i, c) in text.char_indices() {
        if n16 >= pos16 {
            return i;
        }
        n16 += c.len_utf16();
    }
    text.len()
}

/// Derniers échanges qui tiennent dans `max` octets, à partir d'une question
/// de l'apprenant.
pub fn recent_history(history: &[(String, String)], max: usize) -> &[(String, String)] {
    let mut used = 0;
    let mut from = history.len();
    while from > 0 {
        used += history[from - 1].1.len();
        if used > max && from < history.len() {
            break;
        }
        from -= 1;
    }
    // une conversation reprend toujours sur une question de l'apprenant
    while from < history.len() && history[from].0 != "user" {
        from += 1;
    }
    &history[from..]
}

/// Titre d'une conversation : le début de la première question.
pub fn chat_title(question: &str) -> String {
    let line = question.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 60 {
        return line;
    }
    let cut: String = line.chars().take(60).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > 30 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

/// Analyse la réponse au format « Sens : … / Note : … ».
pub fn parse_word_answer(raw: &str) -> (String, String) {
    let mut sense = String::new();
    let mut note = String::new();
    for line in raw.lines() {
        let l = line.trim().trim_start_matches(['*', '-', ' ']);
        let lower = l.to_lowercase();
        if lower.starts_with("sens") || lower.starts_with("meaning") {
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
    let lower = note.to_lowercase();
    if note == "-" || note == "–" || lower.starts_with("rien") || lower.starts_with("none") || note.chars().count() < 3 {
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

    #[test]
    fn chat_prompt() {
        assert_eq!(lang_with_article("it"), "l'italien");
        assert_eq!(lang_with_article("ru"), "le russe");
        assert_eq!(lang_with_article("hi"), "l'hindi");
        assert_eq!(lang_with_article("hu"), "le hongrois");
        let lesson = LessonContext { title: "Il faro", text: "Marta sale le scale.", partial: false };
        let history = vec![("user".to_string(), "Ciao !".to_string()), ("assistant".to_string(), "Ciao ! Come stai ?".to_string())];
        let m = chat_messages("fr", "it", &Learner::knows(1200), Some(&lesson), &history, "  Que veut dire « sale » ?  ", &[]);
        assert_eq!(m.iter().map(|(r, _)| *r).collect::<Vec<_>>(), vec!["system", "user", "assistant", "user"]);
        assert!(m[0].1.contains("connaît déjà environ 1200 mots") && m[0].1.contains("l'italien"));
        assert!(m[0].1.contains("« Il faro »") && m[0].1.contains("<leçon>\nMarta sale le scale.\n</leçon>"));
        // l'apprenant mène : le rappel suit la question, la question reste telle quelle
        assert_eq!(m[3].1, format!("Que veut dire « sale » ?\n\n{CHAT_LEAD}"));
        let m = chat_messages("fr", "ru", &Learner::knows(0), None, &[], "Привет", &["привет : bonjour".into()]);
        assert!(m[0].1.contains("Il débute en russe.") && !m[0].1.contains("<leçon>"));
        assert!(m[1].1.starts_with("Привет\n\n(Je m'entraîne : réponds-moi en russe"));
        assert!(m[1].1.ends_with("\n\n(Dictionnaire, sens possibles à choisir selon le contexte : привет : bonjour)") && !m[1].1.contains(CHAT_LEAD));
        assert_eq!(quoted_words("Explique-moi « saliva » dans cette phrase : « Marta saliva le scale strette. »"), vec!["saliva"]);
        assert_eq!(quoted_words("Différence entre « è salita » et “scese” ?"), vec!["è", "salita", "scese"]);
        assert!(quoted_words("Bonjour, ça va ?").is_empty());
        // entraînement dans la langue étudiée : consigne explicite
        assert!(looks_french("Que veut dire « Marta saliva le scale » ?"));
        assert!(looks_french("Explique-moi le passé composé"));
        assert!(looks_french("Merci !"));
        assert!(!looks_french("Ciao ! Io sono andato al mare ieri, e tu ?"));
        assert!(!looks_french("Привет, как дела ?"));
        assert!(!looks_french("Yesterday I goed to the beach with my friends."));
        // les questions proposées par le chat sont bien du français (sinon : « Corrections » sans objet)
        assert!(looks_french("Résume cette leçon en quelques phrases"));
        assert!(looks_french("Discutons en italien, à mon niveau"));
        assert!(looks_french("Quels mots de cette leçon dois-je retenir en priorité ?"));
        assert!(!looks_french("Hay una diferencia entre ser y estar."));
        assert!(!looks_french("These phrases are useful."));
        let m = chat_messages("fr", "it", &Learner::knows(900), None, &[], "Ciao ! Come stai ?", &[]);
        assert!(m[1].1.contains("réponds-moi en italien"));
        let m = chat_messages("fr", "it", &Learner::knows(900), None, &[], "Comment dit-on « bonjour » ?", &[]);
        assert!(!m[1].1.contains("Je m'entraîne"));
        let m = chat_messages("fr", "fr", &Learner::knows(900), None, &[], "Je suis allé à la plage.", &[]);
        assert!(!m[1].1.contains("Je m'entraîne"));
        // profil : nom, accords, ce qui motive, centres d'intérêt
        let me = Learner { known: 900, name: "Léa".into(), why: "parler avec ma famille à Naples".into(), interests: vec!["cuisine".into(), "opéra".into()], feminine: Some(true) };
        let m = chat_messages("fr", "it", &me, None, &[], "Ciao !", &[]);
        assert!(m[0].1.contains("L'apprenante est francophone. Elle étudie l'italien"));
        assert!(m[0].1.contains("Elle s'appelle Léa :") && m[0].1.contains("accorde au féminin"));
        // centres d'intérêt et motivation : seulement quand l'apprenante laisse choisir
        assert!(m[0].1.contains(
            "Seulement si elle te demande de choisir un sujet ou des exemples, inspire-toi de ses centres d'intérêt (cuisine, opéra) et de ce qui la motive (« parler avec ma famille à Naples ») ; sinon, n'en parle pas."
        ));
        let m = chat_messages("en", "it", &me, None, &[], "Ciao!", &[]);
        assert!(m[0].1.contains("Their name is Léa:") && m[0].1.contains("such as Italian, use feminine agreement"));
        assert!(m[0].1.contains(
            "Only if they ask you to choose a topic or examples, draw on their interests (cuisine, opéra) and what motivates them (\"parler avec ma famille à Naples\"); otherwise, don't mention them."
        ));
        // sans profil : rien de plus qu'avant
        let m = chat_messages("fr", "it", &Learner::knows(900), None, &[], "Ciao !", &[]);
        assert!(m[0].1.contains("L'apprenant est francophone. Il étudie"));
        assert!(!m[0].1.contains("s'appelle") && !m[0].1.contains("accorde") && !m[0].1.contains("motive") && !m[0].1.contains("centres d'intérêt"));
    }

    #[test]
    fn chat_prompt_english() {
        let lesson = LessonContext { title: "Il faro", text: "Marta sale le scale.", partial: true };
        let m = chat_messages("en", "it", &Learner::knows(1200), Some(&lesson), &[], "What does « sale » mean here?", &["sale: third-person singular present indicative of salire".into()]);
        assert!(m[0].1.starts_with("You are Lumen") && m[0].1.contains("already know about 1200 words") && m[0].1.contains("Italian"));
        assert!(m[0].1.contains("\"Il faro\"") && m[0].1.contains("<lesson>\nMarta sale le scale.\n</lesson>") && m[0].1.contains("only an excerpt"));
        assert!(!m[1].1.contains("practising") && m[1].1.ends_with(&format!("salire)\n\n{CHAT_LEAD_EN}")));
        let m = chat_messages("en", "it", &Learner::knows(10), None, &[], "Ciao! Io sono andato al mare ieri, e tu?", &[]);
        assert!(m[0].1.contains("beginner in Italian") && m[1].1.contains("answer me in Italian") && m[1].1.contains("written in English"));
        // l'anglais : le français d'un apprenant anglophone, pas une question
        assert!(looks_english("What does « Marta saliva le scale » mean?"));
        assert!(looks_english("Explain the past tense"));
        assert!(looks_english("Thanks!"));
        assert!(!looks_english("Ciao! Io sono andato al mare ieri, e tu?"));
        assert!(!looks_english("Привет, как дела?"));
        assert!(!looks_english("Ich bin in Berlin, und du?"));
        assert!(!looks_english("Hola, me llamo Ana y tengo un perro."));
        assert!(!looks_english("Je suis allé à la plage avec mes amis."));
        let m = chat_messages("en", "en", &Learner::knows(900), None, &[], "I goed to the beach.", &[]);
        assert!(!m[1].1.contains("practising"));
        // mots et phrases
        let w = word_messages("en", "it", "andavo", "Quando ero piccolo andavo al mare.", "form of andare: to go");
        assert!(w[0].1.contains("English-speaking") && w.last().unwrap().1 == "Language: Italian\nSentence: Quando ero piccolo andavo al mare.\nWord: andavo\nDictionary: form of andare: to go");
        assert!(sentence_messages("en", "de", "Guten Morgen.").last().unwrap().1 == "Sentence in German: Guten Morgen.");
        assert!(simplify_messages("en", "es", "A2", "Hola.")[1].1.starts_with("Rewrite this text in Spanish for a learner at level A2"));
        let (s, n) = parse_word_answer("Meaning: was going\nNote: -");
        assert_eq!((s.as_str(), n.as_str()), ("was going", ""));
        let (s, n) = parse_word_answer("Meaning: good luck\nNote: fixed expression");
        assert_eq!((s.as_str(), n.as_str()), ("good luck", "fixed expression"));
    }

    #[test]
    fn excerpt_of_long_lessons() {
        // courte : entière
        assert_eq!(lesson_excerpt("  Un texte court.\n", Some(3), 100), ("Un texte court.".to_string(), false));
        // longue : extrait autour du passage lu, coupé entre deux paragraphes
        let paras: Vec<String> = (0..40).map(|i| format!("Paragraphe numéro {i:02}, avec quelques mots de plus.")).collect();
        let text = paras.join("\n");
        let at = text[..text.find("numéro 30").unwrap()].encode_utf16().count();
        let (ex, partial) = lesson_excerpt(&text, Some(at), 600);
        assert!(partial && ex.len() <= 600);
        assert!(ex.contains("numéro 30") && ex.starts_with("Paragraphe") && ex.ends_with("plus."));
        // le passage lu est vers le début de l'extrait : on lit vers l'avant
        assert!(ex.find("numéro 30").unwrap() < ex.len() / 2);
        // sans position : le début de la leçon
        let (ex, _) = lesson_excerpt(&text, None, 600);
        assert!(ex.starts_with("Paragraphe numéro 00"));
        // écriture sur plusieurs octets : jamais de coupure au milieu d'un caractère
        let jp = "猫が好きです。".repeat(400);
        let pos16 = jp.encode_utf16().count() / 2;
        let (ex, partial) = lesson_excerpt(&jp, Some(pos16), 1000);
        assert!(partial && ex.len() <= 1000 && ex.ends_with('。'));
    }

    #[test]
    fn dictionary_hints() {
        let res = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/dicts");
        let tmp = std::env::temp_dir().join(format!("lumen-hint-test-{}", std::process::id()));
        let d = crate::dict::Dicts::new(res, tmp.clone());
        let h = dict_hint(&d, "fr", "it", "andavo").unwrap();
        assert!(h.starts_with("andavo : première personne du singulier") && h.contains("(andare : "), "{h}");
        let h = dict_hint(&d, "fr", "en", "chose").unwrap();
        assert!(h.contains("choose"), "{h}");
        // renvoi sans détail : aucun indice plutôt qu'un homonyme trompeur
        assert_eq!(dict_hint(&d, "fr", "it", "saliva"), None);
        // homographes (« faro », le phare, et « farò », je ferai) : les deux sens, au modèle de choisir
        let h = dict_hint(&d, "fr", "it", "faro").unwrap();
        assert!(h.contains("futur simple de fare") && h.contains("ou nom commun : phare"), "{h}");
        assert_eq!(dict_hint(&d, "fr", "ja", "猫"), None);
        assert_eq!(dict_hint(&d, "fr", "it", "zzzqx"), None);
        // dictionnaire anglais absent : aucun indice (jamais de définitions françaises)
        assert_eq!(dict_hint(&d, "en", "it", "andavo"), None);
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn history_and_titles() {
        let h = |r: &str, t: &str| (r.to_string(), t.to_string());
        let history = vec![h("user", &"a".repeat(50)), h("assistant", &"b".repeat(50)), h("user", "c"), h("assistant", "d")];
        assert_eq!(recent_history(&history, 1000).len(), 4);
        // trop long : on garde la fin, en reprenant sur une question
        assert_eq!(recent_history(&history, 60), &history[2..]);
        assert!(recent_history(&[], 10).is_empty());
        assert_eq!(chat_title("\n  Que veut dire   « faro » ?\nMerci"), "Que veut dire « faro » ?");
        let long = chat_title("Peux-tu m'expliquer la différence entre le passé composé et l'imparfait en italien ?");
        assert!(long.ends_with('…') && long.chars().count() <= 61, "{long}");
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
                .generate(Path::new(&path), &word_messages("fr", lang, word, sentence, hint), 72, Priority::Background, |_| true)
                .unwrap();
            let (s, n) = parse_word_answer(&raw);
            println!("[{lang}] {word} -> « {s} » | note : {n} ({:?})\n   brut : {raw:?}", t.elapsed());
            assert!(!s.is_empty());
        }
        let out = engine
            .generate(Path::new(&path), &sentence_messages("fr", "en", "If you are reading this, you are not alone."), 120, Priority::Background, |_| true)
            .unwrap();
        println!("phrase -> {out}");
    }

    /// Test réel en anglais (apprenant anglophone) : sens en contexte, phrase et chat.
    /// LUMEN_TEST_MODEL=/chemin/modele.gguf cargo test --release --lib english_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn english_live() {
        let Ok(path) = std::env::var("LUMEN_TEST_MODEL") else { return };
        let engine = Engine::new();
        let cases = [
            ("it", "andavo", "Quando ero piccolo andavo sempre al mare con mio nonno.", "form of andare: to go"),
            ("es", "subía", "Cada mañana, Marta subía las escaleras del viejo faro.", ""),
            ("de", "ging", "Gestern ging ich nach der Arbeit in den Park.", "form of gehen: to go"),
            ("fr", "phare", "Chaque matin, Marthe montait l'étroit escalier du vieux phare.", "lighthouse"),
            ("ru", "нашла", "Однажды она нашла письмо, спрятанное между двумя камнями.", ""),
            ("it", "in bocca al lupo", "Domani hai l'esame? In bocca al lupo!", ""),
        ];
        for (lang, word, sentence, hint) in cases {
            let t = std::time::Instant::now();
            let raw = engine
                .generate(Path::new(&path), &word_messages("en", lang, word, sentence, hint), 72, Priority::Background, |_| true)
                .unwrap();
            let (s, n) = parse_word_answer(&raw);
            println!("[{lang}] {word} -> « {s} » | note : {n} ({:?})\n   brut : {raw:?}", t.elapsed());
            assert!(!s.is_empty());
        }
        let out = engine
            .generate(Path::new(&path), &sentence_messages("en", "it", "Se stai leggendo queste righe, non sei sola."), 120, Priority::Background, |_| true)
            .unwrap();
        println!("phrase -> {out}");
        // repères du dictionnaire anglais (tools/build_dicts.py --en), comme dans l'application
        let tmp = std::env::temp_dir().join("lumen-english-live");
        std::fs::create_dir_all(&tmp).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/dicts-en/it.db.gz");
        if src.exists() && !tmp.join("it-en-v1.db").exists() {
            let mut dec = flate2::read::GzDecoder::new(std::fs::File::open(&src).unwrap());
            std::io::copy(&mut dec, &mut std::fs::File::create(tmp.join("it-en-v1.db")).unwrap()).unwrap();
        }
        let dicts = crate::dict::Dicts::new(Path::new("/nonexistent").to_path_buf(), tmp);
        let flag = Arc::new(AtomicBool::new(false));
        for q in ["What does “saliva” mean in “Marta saliva le scale”?", "Ciao! Ieri sono andato al mare con mio amici."] {
            let hints: Vec<String> = quoted_words(q).iter().filter_map(|w| dict_hint(&dicts, "en", "it", w)).collect();
            println!("\n--- repères : {hints:?}");
            let msgs = chat_messages("en", "it", &Learner::knows(900), None, &[], q, &hints);
            let g = Gen { max_tokens: 400, think: None, sampling: Sampling::Natural, priority: Priority::Stoppable(flag.clone()) };
            let out = engine.run(Path::new(&path), &msgs, g, |_| true).unwrap();
            println!("\n=== {q}\n{}", out.answer);
            assert!(!out.answer.is_empty());
        }
    }

    /// Chat réel, sans puis avec réflexion, et arrêt en cours de route :
    /// LUMEN_TEST_MODEL=/chemin/modele.gguf cargo test --release --lib chat_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn chat_live() {
        let Ok(path) = std::env::var("LUMEN_TEST_MODEL") else { return };
        let engine = Engine::new();
        let lesson = LessonContext {
            title: "Il faro",
            text: "Ogni mattina, Marta saliva le scale strette del vecchio faro. Dalla cima, il mare sembrava infinito e calmo.\n\nUn giorno trovò una lettera nascosta tra due pietre.",
            partial: false,
        };
        // repères du dictionnaire, comme dans l'application
        let dicts = crate::dict::Dicts::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/dicts"), std::env::temp_dir().join("lumen-chat-live-dicts"));
        let hints = |q: &str| -> Vec<String> { quoted_words(q).iter().filter_map(|w| dict_hint(&dicts, "fr", "it", w)).collect() };
        let ask = |q: &str, think: Option<usize>, stop_after: Option<usize>| {
            let flag = Arc::new(AtomicBool::new(false));
            let msgs = chat_messages("fr", "it", &Learner::knows(900), Some(&lesson), &[], q, &hints(q));
            let g = Gen { max_tokens: 700, think, sampling: Sampling::Natural, priority: Priority::Stoppable(flag.clone()) };
            let t = std::time::Instant::now();
            let (mut thought_pieces, mut answer_pieces) = (0usize, 0usize);
            let out = engine
                .run(Path::new(&path), &msgs, g, |p| {
                    match p {
                        Piece::Thought(_) => thought_pieces += 1,
                        Piece::Answer(_) => answer_pieces += 1,
                    }
                    if stop_after.is_some_and(|n| answer_pieces >= n) {
                        flag.store(true, Ordering::Relaxed);
                    }
                    true
                })
                .unwrap();
            println!("\n--- repères : {:?}", hints(q));
            println!(
                "\n=== {q} (réflexion : {think:?}) : {:?}, {thought_pieces} morceaux pensés, {answer_pieces} de réponse, arrêt : {}\n--- réflexion ({} car.) :\n{}\n--- réponse :\n{}",
                t.elapsed(),
                out.stopped,
                out.thought.chars().count(),
                out.thought.chars().take(600).collect::<String>(),
                out.answer
            );
            (out, thought_pieces)
        };
        let (out, pieces) = ask("Explique-moi « saliva » (traduit ici par « montait ») dans cette phrase : « Ogni mattina, Marta saliva le scale strette del vecchio faro. »", None, None);
        assert!(!out.answer.is_empty() && out.thought.is_empty() && pieces == 0 && !out.stopped);
        // réflexion bornée : jamais plus que le budget, et une réponse au bout
        let (out, pieces) = ask("Explique la différence entre « saliva » et « è salita ».", Some(256), None);
        assert!(!out.answer.is_empty() && !out.thought.is_empty() && pieces <= 256, "{pieces}");
        assert!(!out.answer.contains("<think>") && !out.answer.contains("</think>"));
        let (out, _) = ask("Ciao ! Io sono andato al mare ieri, e tu ?", Some(1536), None);
        assert!(!out.answer.is_empty());
        let (out, _) = ask("Ciao ! Ieri io sono andato al mare con mio amici. E tu, cosa hai fatto ?", None, None);
        assert!(!out.answer.is_empty());
        // arrêt demandé : la réponse partielle est gardée
        let (out, _) = ask("Écris-moi une longue histoire en italien sur un phare.", None, Some(12));
        assert!(out.stopped && !out.answer.is_empty());
    }

    /// Dernier paragraphe d'une réponse : une offre ou une question qui relance
    /// l'apprenant (« Veux-tu que je… ? », « Let me know if… ») ?
    fn ends_with_offer(answer: &str) -> bool {
        const OFFERS: &[&str] = &[
            "veux-tu", "voulez-vous", "souhaites-tu", "souhaitez-vous", "aimerais-tu", "aimeriez-vous", "n'hésite", "n'hésitez", "si tu veux",
            "si vous voulez", "si tu le souhaites", "si vous le souhaitez", "je peux aussi", "je peux te", "je peux vous", "dis-moi", "dites-moi",
            "à toi", "à vous", "would you like", "do you want", "let me know", "feel free", "if you want", "if you'd like", "i can also", "shall we",
            "your turn", "want me to",
        ];
        let last = answer.trim().rsplit("\n\n").next().unwrap_or("").to_lowercase().replace('’', "'");
        last.trim_end().ends_with('?') || OFFERS.iter().any(|o| last.contains(o))
    }

    /// L'apprenant mène la conversation : les réponses s'arrêtent à ce qui est
    /// demandé, sans offre ni question pour relancer (compte et affiche les fins) :
    /// LUMEN_TEST_MODEL=/chemin/modele.gguf cargo test --release --lib chat_lead_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn chat_lead_live() {
        let Ok(path) = std::env::var("LUMEN_TEST_MODEL") else { return };
        let engine = Engine::new();
        let lesson = LessonContext {
            title: "Il faro",
            text: "Ogni mattina, Marta saliva le scale strette del vecchio faro. Dalla cima, il mare sembrava infinito e calmo.\n\nUn giorno trovò una lettera nascosta tra due pietre.",
            partial: false,
        };
        let me = Learner { known: 900, name: "Léa".into(), why: "parler avec ma famille à Naples".into(), interests: vec!["cuisine".into()], feminine: Some(true) };
        // questions précises : la réponse s'arrête à ce qui est demandé
        let asks: &[(&str, bool, &str)] = &[
            ("fr", true, "Que veut dire « saliva » dans la première phrase ?"),
            ("fr", false, "Quelle est la différence entre le passato prossimo et l'imperfetto ?"),
            ("fr", false, "Comment dit-on « je suis fatigué » en italien ?"),
            ("fr", true, "Résume cette leçon en quelques phrases"),
            ("fr", false, "Pourquoi dit-on « la mano » alors que le mot finit par -o ?"),
            ("en", false, "How do I say “I'm tired” in Italian?"),
            ("en", true, "What does “trovò” mean?"),
        ];
        // conversation voulue par l'apprenant : répondre comme un interlocuteur reste permis
        let talks: &[(&str, &str)] = &[
            ("fr", "Discutons en italien, à mon niveau"),
            ("fr", "Discutons en italien, à mon niveau, de mon dernier voyage"),
            ("fr", "Ciao ! Oggi ho cucinato una pasta al pomodoro."),
            ("fr", "Ciao ! Ieri sono andata al cinema con mia sorella."),
        ];
        let flag = Arc::new(AtomicBool::new(false));
        let answer = |native: &str, with_lesson: bool, q: &str| {
            let msgs = chat_messages(native, "it", &me, with_lesson.then_some(&lesson), &[], q, &[]);
            let g = Gen { max_tokens: 900, think: None, sampling: Sampling::Natural, priority: Priority::Stoppable(flag.clone()) };
            engine.run(Path::new(&path), &msgs, g, |_| true).unwrap().answer
        };
        let mut offers = 0;
        let mut total = 0;
        for round in 0..2 {
            for (native, with_lesson, q) in asks {
                let a = answer(native, *with_lesson, q);
                let offer = ends_with_offer(&a);
                offers += offer as usize;
                total += 1;
                let last = a.trim().rsplit("\n\n").next().unwrap_or("");
                println!("\n=== [{round}] {q} ({} car.){}\n--- fin : {last}", a.chars().count(), if offer { "  << RELANCE" } else { "" });
            }
        }
        for (native, q) in talks {
            println!("\n=== {q}\n{}", answer(native, false, q));
        }
        println!("\n>>> relances : {offers} sur {total}");
    }

    /// Comparaison de modèles sur les mêmes questions, dans les 31 langues : sens en contexte
    /// (interface en français et en anglais), phrases, simplification, chat, et vitesse.
    /// Sert à choisir les modèles proposés (un fichier de résultats par modèle) :
    /// LUMEN_TEST_MODELS=a.gguf,b.gguf [LUMEN_COMPARE_OUT=dossier] cargo test --release --lib compare_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn compare_live() {
        use std::fmt::Write as _;
        let Ok(paths) = std::env::var("LUMEN_TEST_MODELS") else { return };
        let out_dir = std::env::var("LUMEN_COMPARE_OUT").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("lumen-compare"));
        std::fs::create_dir_all(&out_dir).unwrap();
        // (langue, mot, phrase, repère du dictionnaire, réponses acceptées : débuts de mots)
        let words_fr: &[(&str, &str, &str, &str, &[&str])] = &[
            ("it", "preso", "Ho preso un caffè al bar prima di andare in ufficio.", "", &["pris", "bu"]),
            ("it", "Non vedo l'ora", "Non vedo l'ora di rivederti.", "", &["hâte", "impatien"]),
            ("it", "pianta", "La pianta in salotto ha bisogno di acqua.", "", &["plante"]),
            ("es", "olvidaron", "Se me olvidaron las llaves en casa.", "", &["oubli"]),
            ("es", "constipado", "Estoy constipado desde el lunes.", "", &["enrhum"]),
            ("es", "viene", "Siéntate, que ya viene la comida.", "", &["arrive", "vient"]),
            ("de", "aufgehört", "Er hat mit dem Rauchen aufgehört.", "", &["arrêt", "cessé"]),
            ("de", "abholen", "Kannst du mich morgen vom Bahnhof abholen?", "", &["cherch", "récupér", "prendre"]),
            ("de", "Bank", "Wir saßen auf einer Bank am Fluss.", "banque ; banc", &["banc"]),
            ("pt", "puxou", "Ela puxou a porta com força.", "", &["tir"]),
            ("pt", "saudade", "Fiquei com saudade da minha avó.", "", &["manqu", "nostalg", "regret"]),
            ("ru", "душно", "Он открыл окно, потому что в комнате было душно.", "", &["étouff", "lourd", "irrespirable", "suffoc", "manqu"]),
            ("ru", "успел", "Он не успел на последний автобус.", "", &["raté", "manqué", "eu le temps", "attrap", "arrivé à temps"]),
            ("en", "pick you up", "I'll pick you up at eight.", "", &["cherch", "récupér", "prendre"]),
            ("en", "fair", "It's not fair that she has to work on Sunday.", "", &["juste", "équitable"]),
            ("nl", "boodschappen doen", "We gaan morgen boodschappen doen.", "", &["course", "commission"]),
            ("sv", "ont", "Jag har ont i huvudet.", "", &["mal", "douleur"]),
            ("da", "hyggede", "Vi hyggede os med en kop te.", "", &["agréable", "bon moment", "cosy", "confort", "détend", "plaisir", "amus"]),
            ("fi", "ystävälleni", "Kirjoitin kirjeen ystävälleni eilen.", "", &["ami"]),
            ("et", "poodi", "Ma lähen homme poodi.", "", &["magasin", "boutique"]),
            ("lv", "pilsētā", "Es dzīvoju lielā pilsētā.", "", &["ville"]),
            ("lt", "geriu", "Aš geriu kavą kiekvieną rytą.", "", &["boi", "bu"]),
            ("pl", "zamknąć", "Zapomniałem zamknąć drzwi na klucz.", "", &["ferm", "verrouill"]),
            ("cs", "zubaři", "Zítra musím jít k zubaři.", "", &["dentiste"]),
            ("sk", "bicykel", "Kúpil som si nový bicykel.", "", &["vélo", "bicyclette"]),
            ("sl", "vreme", "Danes je zelo lepo vreme.", "", &["temps", "météo"]),
            ("hr", "pekari", "Kupio sam kruh u pekari.", "", &["boulanger"]),
            ("hu", "moziba", "Holnap elmegyünk a moziba.", "", &["cinéma"]),
            ("ro", "umbrela", "Am uitat umbrela acasă.", "", &["parapluie"]),
            ("bg", "планина", "Утре ще ходим на планина.", "", &["montagne"]),
            ("uk", "читати", "Я дуже люблю читати книжки.", "", &["lire"]),
            ("el", "θάλασσα", "Το καλοκαίρι πηγαίνουμε στη θάλασσα.", "", &["mer", "plage"]),
            ("tr", "kalkmam", "Yarın sabah erken kalkmam lazım.", "", &["lever", "levé"]),
            ("ar", "لأستعير", "ذهبت إلى المكتبة لأستعير كتابا.", "", &["emprunt"]),
            ("hi", "पसंद", "मुझे चाय बहुत पसंद है।", "", &["aim", "plaî", "préf", "goût", "apprécie"]),
            ("id", "makan siang", "Saya sudah makan siang.", "", &["déjeun", "repas de midi"]),
            ("vi", "học", "Tôi đang học tiếng Pháp.", "", &["appren", "étudi"]),
            ("ko", "만날", "저는 내일 친구를 만날 거예요.", "", &["rencontr", "voir", "retrouv"]),
            ("ja", "見に行きました", "昨日、友達と映画を見に行きました。", "", &["allé voir", "allés voir", "suis allé", "sommes allés", "voir"]),
            ("ja", "忘れて", "傘を忘れてしまいました。", "", &["oubli"]),
        ];
        let words_en: &[(&str, &str, &str, &str, &[&str])] = &[
            ("fr", "raté", "Il a raté son train ce matin.", "", &["miss"]),
            ("fr", "tombée dans les pommes", "Je suis tombée dans les pommes à cause de la chaleur.", "", &["faint", "pass"]),
            ("it", "manca", "Mi manca molto la mia famiglia.", "", &["miss"]),
            ("es", "me di cuenta", "Ya me di cuenta del error.", "", &["realiz", "realis", "notic"]),
            ("de", "Lust", "Ich habe keine Lust, heute zu kochen.", "", &["feel like", "desire", "mood", "want", "inclination", "urge"]),
            ("nl", "heb zin in", "Ik heb zin in een ijsje.", "", &["feel like", "fancy", "want", "crav", "in the mood"]),
            ("pl", "Czekam", "Czekam na autobus od godziny.", "", &["wait", "been wait"]),
            ("tr", "okudun", "Bu kitabı okudun mu?", "", &["read"]),
            ("ar", "المساء", "أحب القراءة في المساء.", "", &["evening"]),
            ("vi", "mưa", "Hôm nay trời mưa to quá.", "", &["rain"]),
            ("ko", "고파요", "배가 고파요.", "", &["hungry"]),
            ("ja", "乗り遅れました", "電車に乗り遅れました。", "", &["miss"]),
        ];
        let sentences: &[(&str, &str, &str)] = &[
            ("fr", "it", "Se avessi saputo che venivi, avrei preparato una torta."),
            ("fr", "de", "Obwohl es regnete, sind wir spazieren gegangen."),
            ("fr", "ru", "Мне кажется, что он сегодня не придёт."),
            ("fr", "es", "Me hubiera gustado que me lo dijeras antes."),
            ("fr", "pl", "Gdybym miał więcej czasu, nauczyłbym się grać na pianinie."),
            ("fr", "hu", "Tegnap este sokáig beszélgettünk a barátaimmal."),
            ("fr", "ja", "雨が降っていたので、家で本を読みました。"),
            ("fr", "ko", "주말에 가족과 함께 바다에 갔어요."),
            ("fr", "ar", "لم أتمكن من النوم الليلة الماضية بسبب الضوضاء."),
            ("en", "fr", "Il faut que tu viennes me voir avant de partir."),
            ("en", "it", "Ci vediamo domani, se non piove."),
            ("en", "tr", "Türkçe öğrenmek sandığımdan daha zor."),
        ];
        let simplify: &[(&str, &str, &str, &str)] = &[
            ("fr", "it", "A2", "Nonostante la pioggia incessante che da giorni flagellava la costa, il vecchio guardiano del faro si ostinava a salire ogni sera i centoventi gradini della torre, convinto che, finché la lanterna avesse brillato, nessuna nave si sarebbe smarrita tra gli scogli. I pescatori del villaggio, che lo consideravano un po' eccentrico, cominciarono a capire il valore della sua dedizione soltanto quando, una notte di tempesta, la luce salvò un peschereccio in difficoltà."),
            ("en", "de", "A1", "Obwohl die Stadtverwaltung seit Jahren versprochen hatte, die marode Brücke über den Fluss zu sanieren, begannen die Bauarbeiten erst, nachdem ein Lastwagen beinahe eingebrochen wäre und die örtliche Zeitung tagelang über die Gefahr berichtet hatte."),
        ];
        let lesson = LessonContext {
            title: "Il faro",
            text: "Ogni mattina, Marta saliva le scale strette del vecchio faro. Dalla cima, il mare sembrava infinito e calmo.\n\nUn giorno trovò una lettera nascosta tra due pietre.",
            partial: false,
        };
        // (interface, langue, leçon jointe, question)
        let chats: &[(&str, &str, bool, &str)] = &[
            ("fr", "it", true, "Explique-moi « saliva » dans la première phrase, et pourquoi ce n'est pas « salì »."),
            ("fr", "it", false, "Ciao! Ieri io sono andato al mare con mio amici. E tu, cosa hai fatto?"),
            ("fr", "es", false, "Quelle est la différence entre « ser » et « estar » ? Donne deux exemples."),
            ("fr", "ja", false, "Explique simplement la différence entre は et が."),
            ("en", "de", false, "When do I use “seit” and when “vor” to talk about time?"),
        ];

        for path in paths.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let path = Path::new(path);
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let engine = Engine::new();
            let mut log = String::new();
            let t0 = std::time::Instant::now();
            let model = engine.load(path).unwrap();
            let vocab = model.vocab();
            let think = vocab.tokenize(b"</think>", false, true);
            let end = vocab.tokenize(b"<|im_end|>", false, true);
            let _ = writeln!(
                log,
                "# {name}\nchargement : {:?} · {:.2} Go · début <s> : {} · </think> en {} jeton(s) · <|im_end|> fin de tour : {}",
                t0.elapsed(),
                model.size() as f64 / 1e9,
                vocab.should_add_bos(),
                think.len(),
                end.len() == 1 && vocab.is_eog(end[0]),
            );
            let n_tok = |s: &str| vocab.tokenize(s.as_bytes(), false, false).len();
            let score = |section: &str, cases: &[(&str, &str, &str, &str, &[&str])], native: &str, log: &mut String| {
                let (mut ok, mut total_ms) = (0usize, 0u128);
                let _ = writeln!(log, "\n## {section}");
                for (lang, word, sentence, hint, accept) in cases {
                    let t = std::time::Instant::now();
                    let raw = engine.generate(path, &word_messages(native, lang, word, sentence, hint), 72, Priority::Background, |_| true).unwrap();
                    let ms = t.elapsed().as_millis();
                    total_ms += ms;
                    let (s, n) = parse_word_answer(&raw);
                    let low = s.to_lowercase();
                    let good = accept.iter().any(|a| low.contains(a));
                    ok += good as usize;
                    let _ = writeln!(log, "{} [{lang}] {word} → « {s} » · {n} ({ms} ms)", if good { "✓" } else { "✗" });
                }
                let _ = writeln!(log, "→ {ok}/{} attendus, {} ms en moyenne", cases.len(), total_ms / cases.len() as u128);
                (ok, cases.len())
            };
            let (a, b) = score("Sens en contexte, interface en français", words_fr, "fr", &mut log);
            let (c, d) = score("Sens en contexte, interface en anglais", words_en, "en", &mut log);

            let _ = writeln!(log, "\n## Phrases");
            for (native, lang, s) in sentences {
                let t = std::time::Instant::now();
                let out = engine.generate(path, &sentence_messages(native, lang, s), 160, Priority::Background, |_| true).unwrap();
                let _ = writeln!(log, "[{lang} → {native}] {s}\n   {out} ({} ms)", t.elapsed().as_millis());
            }

            let _ = writeln!(log, "\n## Simplifier");
            for (native, lang, level, text) in simplify {
                let t = std::time::Instant::now();
                let out = engine.generate(path, &simplify_messages(native, lang, level, text), 600, Priority::Background, |_| true).unwrap();
                let _ = writeln!(log, "[{lang} {level}] langue reconnue : {:?} ({} ms)\n{out}", crate::langid::guess(&out), t.elapsed().as_millis());
            }

            let _ = writeln!(log, "\n## Chat");
            let (mut gen_tokens, mut gen_secs) = (0usize, 0f64);
            for (native, lang, with_lesson, q) in chats {
                let flag = Arc::new(AtomicBool::new(false));
                let msgs = chat_messages(native, lang, &Learner::knows(900), with_lesson.then_some(&lesson), &[], q, &[]);
                let n_prompt: usize = msgs.iter().map(|(_, c)| n_tok(c)).sum();
                let g = Gen { max_tokens: 700, think: None, sampling: Sampling::Natural, priority: Priority::Stoppable(flag) };
                let t = std::time::Instant::now();
                let mut first: Option<std::time::Duration> = None;
                let out = engine
                    .run(path, &msgs, g, |_| {
                        first.get_or_insert(t.elapsed());
                        true
                    })
                    .unwrap();
                let all = t.elapsed();
                let first = first.unwrap_or(all);
                let n = n_tok(&out.answer);
                gen_tokens += n;
                gen_secs += (all - first).as_secs_f64();
                let _ = writeln!(
                    log,
                    "\n### [{native}/{lang}] {q}\n(consigne ≈ {n_prompt} jetons, premier mot après {:.2} s, {n} jetons en {:.1} s)\n{}",
                    first.as_secs_f64(),
                    all.as_secs_f64(),
                    out.answer
                );
            }
            let _ = writeln!(
                log,
                "\n## Bilan\nmots : {a}/{b} (français), {c}/{d} (anglais) · écriture : {:.1} jetons/s · durée totale : {:.0} s",
                gen_tokens as f64 / gen_secs.max(0.001),
                t0.elapsed().as_secs_f64()
            );
            println!("{log}");
            std::fs::write(out_dir.join(format!("{name}.md")), &log).unwrap();
        }
    }
}
