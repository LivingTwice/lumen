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
        let tokens = vocab.tokenize(prompt.as_bytes(), false, true);
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

// ---------- chat ----------

const CHAT_SYSTEM: &str = "Tu es Lumen, un professeur de langues expert, patient et chaleureux, intégré à une application qui aide à apprendre les langues en lisant et en écoutant. Tu maîtrises la grammaire, la conjugaison, le vocabulaire, la prononciation, les expressions idiomatiques, les registres de langue et la culture des pays concernés, et tu sais rendre tout cela simple.

Règles :
- Réponds en français, sauf si l'apprenant t'écrit dans une autre langue pour s'entraîner : réponds-lui alors dans cette langue, avec des phrases à sa portée, puis signale brièvement ses fautes et leur correction, en français.
- Sois précis et concis : va droit au but, sans formule de politesse superflue.
- Donne des exemples dans la langue étudiée, chacun suivi de sa traduction française.
- Pour un mot ou une expression : son sens dans le contexte, sa forme de base, sa nature grammaticale, puis un ou deux exemples.
- Si tu n'es pas sûr de quelque chose, dis-le au lieu d'inventer.
- Mise en forme sobre : paragraphes courts, listes, **gras** pour les mots clés, tableaux seulement pour les conjugaisons et les déclinaisons.";

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
    lang: &str,
    known: i64,
    lesson: Option<&LessonContext>,
    history: &[(String, String)],
    question: &str,
    hints: &[String],
) -> Vec<(&'static str, String)> {
    let mut system = String::from(CHAT_SYSTEM);
    system.push_str("\n\nL'apprenant est francophone. ");
    if known < 50 {
        system.push_str(&format!("Il débute en {}.", lang_name(lang)));
    } else {
        system.push_str(&format!("Il étudie {} et connaît déjà environ {known} mots dans cette langue.", lang_with_article(lang)));
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
    if lang != "fr" && !looks_french(&q) {
        // l'apprenant écrit dans une autre langue pour s'entraîner : un petit modèle
        // suit mieux une consigne précise, au bon endroit, qu'une règle générale
        q.push_str(&format!(
            "\n\n(Je m'entraîne : réponds-moi en {}, avec des phrases simples, puis ajoute une courte partie « Corrections » en français si j'ai fait des fautes.)",
            lang_name(lang)
        ));
    }
    if !hints.is_empty() {
        // comme pour le panneau du mot : des repères sûrs évitent les sens inventés
        q.push_str(&format!("\n\n(Dictionnaire, sens possibles à choisir selon le contexte : {})", hints.join(" ; ")));
    }
    out.push(("user", q));
    out
}

/// Repère du dictionnaire pour un mot cité dans le chat, par exemple
/// « andavo : première personne du singulier de l'indicatif imparfait du verbe
/// andare (andare : aller) ». Seules les formes décrites par le dictionnaire
/// mènent à leur forme de base : les autres renvois y sont trop peu sûrs.
pub fn dict_hint(dicts: &crate::dict::Dicts, lang: &str, word: &str) -> Option<String> {
    if !dicts.available(lang) {
        return None;
    }
    let d = dicts.lookup(lang, word).ok()?;
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
        let base = dicts.lookup(lang, lemma).map(|r| glosses(&r.entries)).unwrap_or_default();
        let note = note.trim().trim_end_matches('.');
        let note = note.chars().next().map(|c| c.to_lowercase().collect::<String>() + &note[c.len_utf8()..]).unwrap_or_default();
        parts.push(if base.is_empty() { note } else { format!("{note} ({lemma} : {base})") });
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
        parts.push(format!("{} : {}", own[0].pos.to_lowercase(), glosses(&own)));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("{word} : {}", parts.join(" ; ou ")))
}

/// Le message est-il écrit en français ? (Sinon, l'apprenant s'entraîne dans
/// la langue étudiée.) Les citations entre guillemets ne comptent pas.
pub fn looks_french(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "le", "la", "les", "un", "une", "des", "du", "de", "et", "est", "que", "qui", "quoi", "je", "tu", "il", "elle", "nous", "vous",
        "ce", "ça", "pour", "pas", "dans", "sur", "avec", "mot", "mots", "phrase", "dire", "veut", "comment", "pourquoi", "quel", "quelle",
        "explique", "explique-moi", "peux-tu", "merci", "bonjour", "salut", "oui", "non", "c'est", "qu'est-ce", "j'ai", "moi", "mon", "ma",
        "mes", "leçon", "texte", "traduis", "traduction", "donne-moi", "écris-moi", "aussi", "plus", "très", "ou", "où", "au", "aux",
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

    #[test]
    fn chat_prompt() {
        assert_eq!(lang_with_article("it"), "l'italien");
        assert_eq!(lang_with_article("ru"), "le russe");
        assert_eq!(lang_with_article("hi"), "l'hindi");
        assert_eq!(lang_with_article("hu"), "le hongrois");
        let lesson = LessonContext { title: "Il faro", text: "Marta sale le scale.", partial: false };
        let history = vec![("user".to_string(), "Ciao !".to_string()), ("assistant".to_string(), "Ciao ! Come stai ?".to_string())];
        let m = chat_messages("it", 1200, Some(&lesson), &history, "  Que veut dire « sale » ?  ", &[]);
        assert_eq!(m.iter().map(|(r, _)| *r).collect::<Vec<_>>(), vec!["system", "user", "assistant", "user"]);
        assert!(m[0].1.contains("connaît déjà environ 1200 mots") && m[0].1.contains("l'italien"));
        assert!(m[0].1.contains("« Il faro »") && m[0].1.contains("<leçon>\nMarta sale le scale.\n</leçon>"));
        assert_eq!(m[3].1, "Que veut dire « sale » ?");
        let m = chat_messages("ru", 0, None, &[], "Привет", &["привет : bonjour".into()]);
        assert!(m[0].1.contains("Il débute en russe.") && !m[0].1.contains("<leçon>"));
        assert!(m[1].1.starts_with("Привет\n\n(Je m'entraîne : réponds-moi en russe"));
        assert!(m[1].1.ends_with("\n\n(Dictionnaire, sens possibles à choisir selon le contexte : привет : bonjour)"));
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
        let m = chat_messages("it", 900, None, &[], "Ciao ! Come stai ?", &[]);
        assert!(m[1].1.contains("réponds-moi en italien"));
        let m = chat_messages("it", 900, None, &[], "Comment dit-on « bonjour » ?", &[]);
        assert!(!m[1].1.contains("Je m'entraîne"));
        let m = chat_messages("fr", 900, None, &[], "Je suis allé à la plage.", &[]);
        assert!(!m[1].1.contains("Je m'entraîne"));
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
        let h = dict_hint(&d, "it", "andavo").unwrap();
        assert!(h.starts_with("andavo : première personne du singulier") && h.contains("(andare : "), "{h}");
        let h = dict_hint(&d, "en", "chose").unwrap();
        assert!(h.contains("choose"), "{h}");
        // renvoi sans détail : aucun indice plutôt qu'un homonyme trompeur
        assert_eq!(dict_hint(&d, "it", "saliva"), None);
        // homographes (« faro », le phare, et « farò », je ferai) : les deux sens, au modèle de choisir
        let h = dict_hint(&d, "it", "faro").unwrap();
        assert!(h.contains("futur simple de fare") && h.contains("ou nom commun : phare"), "{h}");
        assert_eq!(dict_hint(&d, "ja", "猫"), None);
        assert_eq!(dict_hint(&d, "it", "zzzqx"), None);
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
        let hints = |q: &str| -> Vec<String> { quoted_words(q).iter().filter_map(|w| dict_hint(&dicts, "it", w)).collect() };
        let ask = |q: &str, think: Option<usize>, stop_after: Option<usize>| {
            let flag = Arc::new(AtomicBool::new(false));
            let msgs = chat_messages("it", 900, Some(&lesson), &[], q, &hints(q));
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
}
