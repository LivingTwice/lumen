//! IA en ligne, facultative : à la demande de l'apprenant et avec sa propre clé,
//! un fournisseur en ligne remplace Qwen3.5 sur ce Mac pour la traduction des
//! mots et des phrases, Simplifier et le chat. Whisper, Qwen3-ASR et la voix
//! restent sur ce Mac.
//!
//! Tous les fournisseurs proposés parlent le format « Chat Completions »
//! d'OpenAI (DeepSeek, Gemini, Mistral, OpenAI, Claude, OpenRouter, ou un
//! serveur compatible comme Ollama ou LM Studio) : une seule façon d'écrire la
//! demande et de lire la réponse au fil de l'eau. Ils ne diffèrent que par leur
//! façon de régler la réflexion du modèle (`Dialect`).
//!
//! Ce qui part en ligne : la phrase et le mot touchés (avec l'indice du
//! dictionnaire), le texte à simplifier, et pour le chat la question, la
//! conversation et la leçon jointe. Jamais le profil de l'apprenant.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::ai::{Engine, Output, Piece, Priority};
use crate::i18n::t;

/// Façon dont un fournisseur règle la réflexion (le reste du format est commun).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// `reasoning_effort` ; `max_completion_tokens` ; pas de température (modèles qui raisonnent)
    OpenAi,
    /// `thinking: {type}` et `reasoning_effort` (low, high, max)
    DeepSeek,
    /// `reasoning_effort` (la réflexion des modèles 3 ne se coupe pas : « minimal ») ; pas de température
    Gemini,
    /// `thinking: {type, budget_tokens}` ; aucune réflexion par défaut
    Anthropic,
    /// rien à régler
    Mistral,
    /// `reasoning: {enabled | effort}`
    OpenRouter,
    /// serveur compatible choisi par l'apprenant
    Other,
}

#[derive(Debug)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub base: &'static str,
    pub dialect: Dialect,
    /// réglage qui garde la clé (celle de Gemini sert aussi aux podcasts)
    pub key_setting: &'static str,
    /// modèle pris si la liste du fournisseur ne dit rien
    pub fallback: &'static str,
}

pub const PROVIDERS: [Provider; 7] = [
    Provider { id: "deepseek", name: "DeepSeek", base: "https://api.deepseek.com", dialect: Dialect::DeepSeek, key_setting: "online_key_deepseek", fallback: "deepseek-flash" },
    Provider {
        id: "gemini",
        name: "Gemini",
        base: "https://generativelanguage.googleapis.com/v1beta/openai",
        dialect: Dialect::Gemini,
        key_setting: "gemini_key",
        fallback: "gemini-2.5-flash",
    },
    Provider { id: "mistral", name: "Mistral", base: "https://api.mistral.ai/v1", dialect: Dialect::Mistral, key_setting: "online_key_mistral", fallback: "mistral-small-latest" },
    Provider { id: "openai", name: "OpenAI", base: "https://api.openai.com/v1", dialect: Dialect::OpenAi, key_setting: "online_key_openai", fallback: "gpt-5-mini" },
    Provider { id: "anthropic", name: "Claude", base: "https://api.anthropic.com/v1", dialect: Dialect::Anthropic, key_setting: "online_key_anthropic", fallback: "claude-haiku-4-5" },
    Provider { id: "openrouter", name: "OpenRouter", base: "https://openrouter.ai/api/v1", dialect: Dialect::OpenRouter, key_setting: "online_key_openrouter", fallback: "deepseek/deepseek-chat" },
    // l'adresse est celle que l'apprenant indique (`online_url`)
    Provider { id: "custom", name: "", base: "", dialect: Dialect::Other, key_setting: "online_key_custom", fallback: "" },
];

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Ce que l'IA en ligne peut prendre en charge, au choix de l'apprenant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// sens des mots en contexte, traduction des phrases
    Words,
    /// chat et Simplifier
    Chat,
}

/// Un fournisseur prêt à répondre.
#[derive(Clone, Debug)]
pub struct Config {
    pub provider: &'static Provider,
    /// nom montré dans les messages (l'hôte, pour un serveur choisi)
    pub name: String,
    pub base: String,
    pub key: String,
    pub model: String,
}

impl Config {
    /// Identifiant de la réponse dans le cache des traductions.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.id, self.model)
    }
}

/// Adresse d'un serveur compatible, telle que l'apprenant l'a collée :
/// « http://localhost:11434 » → « http://localhost:11434/v1 ».
pub fn normalize_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix("/chat/completions").unwrap_or(url).trim_end_matches('/');
    let parsed = reqwest::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())?;
    Some(if parsed.path().trim_matches('/').is_empty() { format!("{url}/v1") } else { url.to_string() })
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| match u.port() {
            Some(p) => format!("{h}:{p}"),
            None => h.to_string(),
        }))
        .unwrap_or_else(|| url.to_string())
}

/// L'IA en ligne choisie pour ce rôle (réglages `online_*`) : `None` si ce rôle
/// reste sur ce Mac ; une erreur si elle est choisie mais incomplète.
pub fn config_from(get: impl Fn(&str) -> Option<String>, role: Role) -> Option<Result<Config, String>> {
    let set = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if set("online_on").as_deref() != Some("1") {
        return None;
    }
    let flag = match role {
        Role::Words => "online_words",
        Role::Chat => "online_chat",
    };
    if set(flag).as_deref() == Some("0") {
        return None;
    }
    let p = set("online_provider").and_then(|id| find(&id)).unwrap_or(&PROVIDERS[0]);
    let custom = p.dialect == Dialect::Other;
    let key = set(p.key_setting).unwrap_or_default();
    let (base, name) = if custom {
        match set("online_url").as_deref().and_then(normalize_url) {
            Some(u) => (u.clone(), host_of(&u)),
            None => return Some(Err(t("Indiquez l'adresse de votre serveur dans Réglages › IA.", "Enter your server's address in Settings › AI.").into())),
        }
    } else {
        (p.base.to_string(), p.name.to_string())
    };
    // un serveur sur ce Mac (Ollama, LM Studio) n'a souvent pas de clé
    if key.is_empty() && !custom {
        return Some(Err(tr!("Ajoutez votre clé {name} dans Réglages › IA.", "Add your {name} key in Settings › AI.")));
    }
    let model = set(&format!("online_model_{}", p.id)).unwrap_or_else(|| p.fallback.to_string());
    if model.is_empty() {
        return Some(Err(t("Choisissez un modèle dans Réglages › IA.", "Choose a model in Settings › AI.").into()));
    }
    Some(Ok(Config { provider: p, name, base, key, model }))
}

pub fn config(c: &rusqlite::Connection, role: Role) -> Option<Result<Config, String>> {
    config_from(|k| crate::db::setting(c, k), role)
}

// ---------- la demande ----------

pub struct Ask<'a> {
    pub messages: &'a [(&'a str, String)],
    /// longueur attendue de la réponse (une marge est ajoutée : la réflexion compte souvent dedans)
    pub max_tokens: usize,
    /// effort de réflexion (« low », « medium », « high ») ; `None` : réponse directe
    pub think: Option<&'a str>,
    /// traduction : la réponse la plus probable (température 0, là où elle est permise)
    pub exact: bool,
}

/// Paramètres que tous les modèles n'acceptent pas : refusés (erreur 400), on
/// les retire un à un, et on retient pour ce modèle ce qui a marché.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    /// réglage de la réflexion, longueur, température
    Full,
    /// autre réglage de la réflexion (« minimal » au lieu de « none »…)
    Alt,
    /// longueur et température seulement
    NoThinking,
    /// le modèle et les messages, rien d'autre
    Bare,
}

impl Level {
    fn next(self, d: Dialect, think: bool) -> Level {
        match self {
            Level::Full if !think && matches!(d, Dialect::OpenAi | Dialect::Gemini) => Level::Alt,
            Level::Full | Level::Alt => Level::NoThinking,
            _ => Level::Bare,
        }
    }
}

/// Jetons de réflexion permis selon l'effort (comme `ai::think_budget`, en plus large).
fn think_tokens(effort: &str) -> usize {
    match effort {
        "low" => 1024,
        "high" => 12_000,
        _ => 4096,
    }
}

fn body(cfg: &Config, ask: &Ask, level: Level) -> Value {
    let d = cfg.provider.dialect;
    let messages: Vec<Value> = ask.messages.iter().map(|(role, content)| json!({ "role": role, "content": content })).collect();
    let mut b = json!({ "model": cfg.model, "messages": messages, "stream": true });
    if level == Level::Bare {
        return b;
    }
    let budget = ask.think.map(think_tokens).unwrap_or(0);
    let cap = ask.max_tokens.max(512) * 2 + budget;
    b[if d == Dialect::OpenAi { "max_completion_tokens" } else { "max_tokens" }] = json!(cap);
    // les modèles d'OpenAI qui raisonnent refusent toute autre température ; Google
    // déconseille de toucher à celle de Gemini 3 (il tourne en rond)
    if ask.exact && !matches!(d, Dialect::OpenAi | Dialect::Gemini) {
        b["temperature"] = json!(0);
    }
    match (level, d, ask.think) {
        (Level::Full, Dialect::OpenAi, None) => b["reasoning_effort"] = json!("none"),
        (Level::Alt, Dialect::OpenAi, None) => b["reasoning_effort"] = json!("minimal"),
        (Level::Full, Dialect::Gemini, None) => b["reasoning_effort"] = json!("minimal"),
        (Level::Alt, Dialect::Gemini, None) => b["reasoning_effort"] = json!("low"),
        (Level::Full, Dialect::OpenAi | Dialect::Gemini, Some(e)) => b["reasoning_effort"] = json!(e),
        (Level::Full, Dialect::DeepSeek, None) => b["thinking"] = json!({ "type": "disabled" }),
        (Level::Full, Dialect::DeepSeek, Some(e)) => {
            b["thinking"] = json!({ "type": "enabled" });
            b["reasoning_effort"] = json!(match e {
                "low" => "low",
                "high" => "max",
                _ => "high",
            });
        }
        (Level::Full, Dialect::Anthropic, Some(_)) => b["thinking"] = json!({ "type": "enabled", "budget_tokens": budget }),
        (Level::Full, Dialect::OpenRouter, None) => b["reasoning"] = json!({ "enabled": false }),
        (Level::Full, Dialect::OpenRouter, Some(e)) => b["reasoning"] = json!({ "effort": e }),
        _ => {}
    }
    b
}

fn client() -> Result<reqwest::Client> {
    // un seul client : la connexion reste ouverte d'un mot touché au suivant (réponse plus rapide)
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = C.get() {
        return Ok(c.clone());
    }
    let c = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // silence de plus d'une minute et demie au milieu d'une réponse : abandon
        .read_timeout(Duration::from_secs(90))
        .pool_idle_timeout(Duration::from_secs(120))
        .build()?;
    Ok(C.get_or_init(|| c).clone())
}

fn request(c: &reqwest::Client, cfg: &Config, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    let mut r = c.request(method, format!("{}/{path}", cfg.base.trim_end_matches('/'))).header("content-type", "application/json");
    if !cfg.key.is_empty() {
        r = r.bearer_auth(&cfg.key);
    }
    match cfg.provider.dialect {
        // la liste des modèles de Claude ne connaît que cet en-tête
        Dialect::Anthropic => r.header("x-api-key", &cfg.key).header("anthropic-version", "2023-06-01"),
        // présentation facultative demandée par OpenRouter
        Dialect::OpenRouter => r.header("HTTP-Referer", "https://github.com/LivingTwice/lumen").header("X-Title", "Lumen"),
        _ => r,
    }
}

/// Modèles déjà essayés : le niveau de paramètres qu'ils acceptent.
fn levels() -> &'static Mutex<HashMap<String, Level>> {
    static L: OnceLock<Mutex<HashMap<String, Level>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------- arrêt ----------

/// Arrêt demandé : `Ok(true)` par l'apprenant (ce qui est écrit est gardé),
/// `Err("interrompu")` par une requête plus récente (un autre mot touché).
pub fn halter<'a>(engine: &'a Engine, p: &'a Priority) -> impl Fn() -> Result<bool> + 'a {
    move || match p {
        Priority::Interactive(id) if !engine.is_current(*id) => Err(anyhow!("interrompu")),
        Priority::Stoppable(flag) => Ok(flag.load(Ordering::Relaxed)),
        _ => Ok(false),
    }
}

/// Attend `fut` en surveillant l'arrêt (`None` : arrêtée par l'apprenant).
async fn until<F: Future>(fut: F, halted: &impl Fn() -> Result<bool>) -> Result<Option<F::Output>> {
    // vérifié à chaque appel : les morceaux d'une réponse arrivent souvent sans pause
    if halted()? {
        return Ok(None);
    }
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return Ok(Some(r)),
            _ = tokio::time::sleep(Duration::from_millis(150)) => {
                if halted()? {
                    return Ok(None);
                }
            }
        }
    }
}

// ---------- erreurs ----------

/// Fournisseur injoignable (pas de connexion, délai dépassé) : la traduction
/// peut alors se replier sur le modèle de ce Mac.
#[derive(Debug)]
pub struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

pub fn is_unreachable(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Unreachable>().is_some()
}

fn unreachable(cfg: &Config, e: &reqwest::Error) -> anyhow::Error {
    let name = &cfg.name;
    anyhow::Error::new(Unreachable(if e.is_timeout() {
        tr!("{name} a mis trop de temps à répondre. Réessayez.", "{name} took too long to answer. Try again.")
    } else {
        tr!("{name} est injoignable. Vérifiez la connexion à Internet.", "{name} can't be reached. Check the Internet connection.")
    }))
}

/// Message d'erreur du fournisseur (formats d'OpenAI, de Google, de Mistral…).
fn message_of(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let v = if v.is_array() { v[0].clone() } else { v };
    v["error"]["message"]
        .as_str()
        .or(v["error"].as_str())
        .or(v["message"].as_str())
        .or(v["detail"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(200).collect())
        .trim()
        .to_string()
}

fn key_refused(body: &str) -> bool {
    let b = body.to_lowercase();
    ["api_key_invalid", "api key not valid", "invalid api key", "incorrect api key", "invalid x-api-key", "authentication_error", "invalid_api_key"].iter().any(|x| b.contains(x))
}

fn credit_out(body: &str) -> bool {
    let b = body.to_lowercase();
    ["insufficient balance", "insufficient_quota", "credit balance", "insufficient credits", "payment required"].iter().any(|x| b.contains(x))
}

fn quota_out(body: &str) -> bool {
    let b = body.to_lowercase();
    b.contains("perday") || b.contains("per day") || b.contains("resource_exhausted") || b.contains("exceeded your current quota")
}

/// Une erreur du fournisseur, dite pour l'apprenant.
fn api_error(cfg: &Config, status: u16, body: &str) -> anyhow::Error {
    let name = &cfg.name;
    let model = &cfg.model;
    let msg = message_of(body);
    anyhow!(if credit_out(body) || status == 402 {
        tr!(
            "Le crédit de votre compte {name} est épuisé. Rechargez-le sur le site de {name}, ou passez à l'IA de ce Mac (Réglages › IA).",
            "Your {name} account has run out of credit. Top it up on the {name} website, or switch to this Mac's AI (Settings › AI)."
        )
    } else if status == 401 || status == 403 || key_refused(body) {
        if msg.contains("location is not supported") {
            tr!("{name} n'est pas proposé dans le pays d'où part la demande.", "{name} isn't offered in the country the request comes from.")
        } else {
            tr!("{name} refuse cette clé. Vérifiez-la, puis collez-la à nouveau (Réglages › IA).", "{name} refuses this key. Check it, then paste it again (Settings › AI).")
        }
    } else if status == 404 {
        tr!(
            "{name} ne connaît pas le modèle « {model} ». Choisissez-en un autre dans Réglages › IA.",
            "{name} doesn't know the model “{model}”. Choose another one in Settings › AI."
        )
    } else if status == 429 && quota_out(body) {
        tr!(
            "Le quota de {name} est épuisé pour le moment. Réessayez plus tard, ou choisissez un autre modèle dans Réglages › IA.",
            "Your {name} quota is used up for now. Try again later, or choose another model in Settings › AI."
        )
    } else if status == 429 {
        tr!("{name} reçoit trop de demandes. Réessayez dans un instant.", "{name} is getting too many requests. Try again in a moment.")
    } else if status >= 500 {
        tr!("{name} a un souci de son côté. Réessayez dans un instant.", "{name} is having trouble on its side. Try again in a moment.")
    } else {
        tr!("{name} a répondu : {msg}", "{name} answered: {msg}")
    })
}

fn retry_after(res: &reqwest::Response) -> Option<Duration> {
    res.headers().get("retry-after")?.to_str().ok()?.trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(Duration::from_secs_f64)
}

// ---------- la réponse, au fil de l'eau ----------

/// Ce qu'apporte une ligne du flux (« data: {…} »).
#[derive(Debug, Default, PartialEq)]
struct Delta {
    thought: String,
    answer: String,
    done: bool,
    error: Option<String>,
}

fn parse_line(line: &str) -> Option<Delta> {
    let data = line.trim().strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(Delta { done: true, ..Default::default() });
    }
    let v: Value = serde_json::from_str(data).ok()?;
    if !v["error"].is_null() {
        return Some(Delta { error: Some(message_of(data)), ..Default::default() });
    }
    let delta = &v["choices"][0]["delta"];
    // réflexion : `reasoning_content` (DeepSeek, LM Studio), `reasoning` (OpenRouter, Ollama)
    let thought = delta["reasoning_content"].as_str().or(delta["reasoning"].as_str()).unwrap_or("").to_string();
    let answer = delta["content"].as_str().unwrap_or("").to_string();
    Some(Delta { thought, answer, ..Default::default() })
}

/// Génération complète : réflexion facultative puis réponse, au fil de l'eau
/// (même forme que `Engine::run`).
pub async fn run(cfg: &Config, ask: Ask<'_>, halted: impl Fn() -> Result<bool>, mut on_piece: impl FnMut(Piece) -> bool) -> Result<Output> {
    let c = client()?;
    let d = cfg.provider.dialect;
    let memo = format!("{}|{}|{}", cfg.base, cfg.model, ask.think.is_some());
    let mut level = levels().lock().get(&memo).copied().unwrap_or(Level::Full);
    let first = level;
    let mut out = Output { thought: String::new(), answer: String::new(), stopped: false };
    let mut attempt = 0u32;
    let res = loop {
        let payload = serde_json::to_vec(&body(cfg, &ask, level))?;
        let send = request(&c, cfg, reqwest::Method::POST, "chat/completions").body(payload).send();
        let res = match until(send, &halted).await? {
            None => {
                out.stopped = true;
                return Ok(out);
            }
            Some(r) => r,
        };
        let res = match res {
            Ok(r) => r,
            Err(e) if attempt < 1 && (e.is_connect() || e.is_timeout()) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(800)).await;
                continue;
            }
            Err(e) => return Err(unreachable(cfg, &e)),
        };
        let status = res.status().as_u16();
        if status == 200 {
            break res;
        }
        let wait = retry_after(&res);
        let text = res.text().await.unwrap_or_default();
        // paramètre refusé par ce modèle : on recommence avec moins
        if status == 400 && level != Level::Bare && !key_refused(&text) && !credit_out(&text) {
            level = level.next(d, ask.think.is_some());
            continue;
        }
        // trop de demandes à la fois, serveur débordé : une ou deux attentes courtes
        let busy = (status == 429 && !quota_out(&text) && !credit_out(&text)) || matches!(status, 500 | 502 | 503 | 504 | 529);
        let pause = wait.unwrap_or(Duration::from_millis(1500 << attempt));
        if busy && attempt < 2 && pause <= Duration::from_secs(8) {
            attempt += 1;
            if until(tokio::time::sleep(pause), &halted).await?.is_none() {
                out.stopped = true;
                return Ok(out);
            }
            continue;
        }
        return Err(api_error(cfg, status, &text));
    };
    if level != first {
        levels().lock().insert(memo, level);
    }

    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    'read: loop {
        let next = match until(stream.next(), &halted).await? {
            None => {
                out.stopped = true;
                break;
            }
            Some(n) => n,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| unreachable(cfg, &e))?;
        buf.extend_from_slice(&chunk);
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            let Some(delta) = parse_line(&String::from_utf8_lossy(&line)) else { continue };
            if let Some(e) = delta.error {
                if out.answer.is_empty() {
                    let name = &cfg.name;
                    return Err(anyhow!(tr!("{name} a répondu : {e}", "{name} answered: {e}")));
                }
                // réponse déjà commencée : on garde ce qui est écrit
                break 'read;
            }
            if delta.done {
                break 'read;
            }
            if !delta.thought.is_empty() && out.answer.is_empty() {
                out.thought.push_str(&delta.thought);
                if !on_piece(Piece::Thought(&delta.thought)) {
                    break 'read;
                }
            }
            if !delta.answer.is_empty() && !crate::ai::answer_piece(&mut out, &delta.answer, &mut on_piece) {
                break 'read;
            }
        }
    }
    out.thought = out.thought.trim().to_string();
    Ok(out)
}

/// Réponse simple, sans réflexion (traduction, réécriture).
pub async fn generate(cfg: &Config, messages: &[(&str, String)], max_tokens: usize, halted: impl Fn() -> Result<bool>, mut on_piece: impl FnMut(&str) -> bool) -> Result<String> {
    let ask = Ask { messages, max_tokens, think: None, exact: true };
    let out = run(cfg, ask, halted, |p| match p {
        Piece::Answer(t) => on_piece(t),
        Piece::Thought(_) => true,
    })
    .await?;
    Ok(crate::ai::clean(&out.answer))
}

// ---------- vérification de la clé, choix du modèle ----------

/// Résultat de la vérification (miroir de `OnlineCheck` dans types.ts).
#[derive(Serialize, Debug)]
pub struct Check {
    /// modèles de conversation proposés par le fournisseur
    pub models: Vec<String>,
    /// le modèle retenu (celui demandé s'il existe, sinon le conseillé)
    pub model: String,
    /// temps d'une petite réponse, en millisecondes
    pub ms: u64,
}

/// Modèles qui ne conversent pas (voix, images, recherche, vecteurs…).
const NOT_CHAT: [&str; 22] = [
    "embed", "tts", "audio", "realtime", "transcribe", "whisper", "dall-e", "image", "moderation", "search", "-live", "aqa", "imagen", "veo", "computer-use", "babbage", "davinci", "ocr", "guard", "lyria", "codex", "sora",
];

fn chat_models(v: &Value) -> Vec<String> {
    let list = v["data"].as_array().or(v["models"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
    let mut ids: Vec<String> = list
        .iter()
        // Mistral dit ce que chaque modèle sait faire
        .filter(|m| m["capabilities"]["completion_chat"].as_bool() != Some(false))
        .filter_map(|m| m["id"].as_str().or(m["name"].as_str()))
        .map(|s| s.trim_start_matches("models/").to_string())
        .filter(|s| {
            let l = s.to_lowercase();
            !NOT_CHAT.iter().any(|x| l.contains(x))
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// « gpt-5.4-mini » → ((5, 4), « mini ») ; « gemini-3.8-flash » → ((3, 8), « flash »).
fn version<'a>(name: &'a str, prefix: &str) -> Option<((u32, u32), &'a str)> {
    let rest = name.strip_prefix(prefix)?;
    let (v, tail) = rest.split_once('-')?;
    let mut it = v.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = match it.next() {
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    Some(((major, minor), tail))
}

/// Le plus récent des modèles « <préfixe><version>-<suite> » (sans date ni aperçu).
fn newest(ids: &[String], prefix: &str, tail: &str) -> Option<String> {
    ids.iter().filter_map(|id| version(id, prefix).filter(|(_, t)| *t == tail).map(|(v, _)| (v, id))).max().map(|(_, id)| id.clone())
}

/// Le modèle conseillé : rapide et bon marché, assez fin pour la traduction et le chat.
fn pick(p: &Provider, ids: &[String]) -> Option<String> {
    let first_of = |names: &[&str]| names.iter().find(|n| ids.iter().any(|x| x == *n)).map(|n| n.to_string());
    let chosen = match p.dialect {
        Dialect::DeepSeek => first_of(&["deepseek-flash", "deepseek-chat"]).or_else(|| ids.iter().find(|x| x.contains("flash")).cloned()),
        Dialect::Gemini => newest(ids, "gemini-", "flash").or_else(|| newest(ids, "gemini-", "flash-lite")),
        Dialect::OpenAi => newest(ids, "gpt-", "mini").or_else(|| first_of(&["gpt-4.1-mini", "gpt-4o-mini"])),
        // la liste de Claude commence par les plus récents
        Dialect::Anthropic => ids.iter().find(|x| x.contains("haiku")).cloned().or_else(|| {
            let mut v = ids.to_vec();
            v.sort();
            v.into_iter().find(|x| x.contains("sonnet"))
        }),
        Dialect::Mistral => first_of(&["mistral-small-latest", "mistral-medium-latest", "mistral-large-latest"]),
        Dialect::OpenRouter => first_of(&["deepseek/deepseek-flash", "deepseek/deepseek-chat", "google/gemini-2.5-flash"]),
        Dialect::Other => None,
    };
    chosen.or_else(|| ids.first().cloned())
}

/// Liste des modèles du fournisseur (vérifie la clé du même coup).
async fn list_models(c: &reqwest::Client, cfg: &Config) -> Result<Vec<String>> {
    let d = cfg.provider.dialect;
    if d == Dialect::OpenRouter {
        // la liste d'OpenRouter est publique : la clé se vérifie à part
        let r = request(c, cfg, reqwest::Method::GET, "key").send().await.map_err(|e| unreachable(cfg, &e))?;
        let status = r.status().as_u16();
        if status != 200 {
            let text = r.text().await.unwrap_or_default();
            return Err(api_error(cfg, status, &text));
        }
    }
    let path = if d == Dialect::Anthropic { "models?limit=100" } else { "models" };
    let r = request(c, cfg, reqwest::Method::GET, path).send().await.map_err(|e| unreachable(cfg, &e))?;
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    if status != 200 {
        // un serveur compatible ne sait pas toujours lister ses modèles : on les tapera
        if d == Dialect::Other && !matches!(status, 401 | 403) {
            return Ok(Vec::new());
        }
        return Err(api_error(cfg, status, &text));
    }
    Ok(chat_models(&serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Vérifie une clé : liste des modèles, puis une toute petite question au
/// modèle retenu (le compte a du crédit, le modèle répond, et combien de temps il met).
pub async fn check(provider: &str, key: &str, url: &str, model: &str) -> Result<Check> {
    let p = find(provider).ok_or_else(|| anyhow!(t("Fournisseur inconnu.", "Unknown provider.")))?;
    let key = key.trim().to_string();
    let (base, name) = if p.dialect == Dialect::Other {
        let u = normalize_url(url).ok_or_else(|| {
            anyhow!(t(
                "Cette adresse n'est pas valable. Exemple : http://localhost:11434/v1 pour Ollama.",
                "This address isn't valid. Example: http://localhost:11434/v1 for Ollama."
            ))
        })?;
        let host = host_of(&u);
        (u, host)
    } else {
        if key.is_empty() {
            return Err(anyhow!(tr!("Collez d'abord votre clé {}.", "Paste your {} key first.", p.name)));
        }
        (p.base.to_string(), p.name.to_string())
    };
    let mut cfg = Config { provider: p, name, base, key, model: String::new() };
    let c = client()?;
    let models = list_models(&c, &cfg).await?;
    let model = model.trim();
    cfg.model = if !model.is_empty() && (models.is_empty() || models.iter().any(|m| m == model)) {
        model.to_string()
    } else {
        pick(p, &models).unwrap_or_else(|| p.fallback.to_string())
    };
    if cfg.model.is_empty() {
        return Err(anyhow!(t("Indiquez le nom du modèle à utiliser.", "Enter the name of the model to use.")));
    }
    let start = Instant::now();
    let messages = [("user", "Reply with the single word: OK".to_string())];
    run(&cfg, Ask { messages: &messages, max_tokens: 16, think: None, exact: true }, || Ok(false), |_| true).await?;
    Ok(Check { models, model: cfg.model, ms: start.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str, model: &str) -> Config {
        let p = find(id).unwrap();
        Config { provider: p, name: p.name.to_string(), base: p.base.to_string(), key: "k".into(), model: model.into() }
    }

    fn ask<'a>(messages: &'a [(&'a str, String)], think: Option<&'a str>, exact: bool) -> Ask<'a> {
        Ask { messages, max_tokens: 72, think, exact }
    }

    #[test]
    fn settings_choose_the_provider_and_role() {
        fn get(s: &HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
            let s = s.clone();
            move |k| s.get(k).map(|v| v.to_string())
        }
        let mut s: HashMap<&'static str, &'static str> = HashMap::new();
        assert!(config_from(get(&s), Role::Words).is_none(), "sur ce Mac par défaut");
        s.insert("online_on", "1");
        let e = config_from(get(&s), Role::Words).unwrap().unwrap_err();
        assert!(e.contains("DeepSeek"), "DeepSeek par défaut, clé demandée : {e}");
        s.insert("online_key_deepseek", " sk-123 ");
        let c = config_from(get(&s), Role::Words).unwrap().unwrap();
        assert_eq!((c.key.as_str(), c.model.as_str(), c.id().as_str()), ("sk-123", "deepseek-flash", "deepseek:deepseek-flash"));
        // la traduction peut rester sur ce Mac, le chat partir en ligne
        s.insert("online_words", "0");
        assert!(config_from(get(&s), Role::Words).is_none());
        assert!(config_from(get(&s), Role::Chat).unwrap().is_ok());
        // Gemini reprend la clé des podcasts
        s.insert("online_provider", "gemini");
        s.insert("gemini_key", "AIza");
        s.insert("online_model_gemini", "gemini-3.8-flash");
        let c = config_from(get(&s), Role::Chat).unwrap().unwrap();
        assert_eq!((c.key.as_str(), c.model.as_str()), ("AIza", "gemini-3.8-flash"));
        // serveur choisi : sans clé, nommé par son hôte
        s.insert("online_provider", "custom");
        assert!(config_from(get(&s), Role::Chat).unwrap().is_err(), "adresse demandée");
        s.insert("online_url", "http://localhost:11434");
        s.insert("online_model_custom", "qwen3:8b");
        let c = config_from(get(&s), Role::Chat).unwrap().unwrap();
        assert_eq!((c.base.as_str(), c.name.as_str(), c.key.as_str()), ("http://localhost:11434/v1", "localhost:11434", ""));
    }

    #[test]
    fn urls_are_normalized() {
        assert_eq!(normalize_url("http://localhost:1234/v1/").as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(normalize_url(" https://api.groq.com/openai/v1/chat/completions ").as_deref(), Some("https://api.groq.com/openai/v1"));
        assert_eq!(normalize_url("http://127.0.0.1:11434").as_deref(), Some("http://127.0.0.1:11434/v1"));
        assert_eq!(normalize_url("localhost:11434"), None);
        assert_eq!(normalize_url("ftp://x.org/v1"), None);
    }

    #[test]
    fn bodies_follow_each_dialect() {
        let m = [("user", "ciao".to_string())];
        let b = body(&cfg("deepseek", "deepseek-flash"), &ask(&m, None, true), Level::Full);
        assert_eq!(b["thinking"]["type"], "disabled");
        assert_eq!(b["temperature"], 0);
        assert_eq!(b["stream"], true);
        assert!(b["max_tokens"].as_u64().unwrap() >= 72);
        let b = body(&cfg("deepseek", "deepseek-flash"), &ask(&m, Some("high"), false), Level::Full);
        assert_eq!((b["thinking"]["type"].as_str(), b["reasoning_effort"].as_str()), (Some("enabled"), Some("max")));
        assert!(b["temperature"].is_null());

        let b = body(&cfg("openai", "gpt-5.4-mini"), &ask(&m, None, true), Level::Full);
        assert_eq!(b["reasoning_effort"], "none");
        assert!(b["temperature"].is_null() && b["max_tokens"].is_null() && b["max_completion_tokens"].is_u64());
        let b = body(&cfg("openai", "gpt-5.4-mini"), &ask(&m, None, true), Level::Alt);
        assert_eq!(b["reasoning_effort"], "minimal");

        let b = body(&cfg("gemini", "gemini-3.8-flash"), &ask(&m, None, true), Level::Full);
        assert_eq!(b["reasoning_effort"], "minimal");
        assert!(b["temperature"].is_null());

        let b = body(&cfg("anthropic", "claude-haiku-4-5"), &ask(&m, Some("medium"), false), Level::Full);
        assert_eq!(b["thinking"]["budget_tokens"], 4096);
        assert!(b["max_tokens"].as_u64().unwrap() > 4096, "la réponse a sa place après la réflexion");
        let b = body(&cfg("anthropic", "claude-haiku-4-5"), &ask(&m, None, true), Level::Full);
        assert!(b["thinking"].is_null());
        assert_eq!(b["temperature"], 0);

        let b = body(&cfg("openrouter", "deepseek/deepseek-chat"), &ask(&m, None, true), Level::Full);
        assert_eq!(b["reasoning"]["enabled"], false);

        // en repli : sans réglage de la réflexion, puis le strict minimum
        let b = body(&cfg("deepseek", "deepseek-flash"), &ask(&m, None, true), Level::NoThinking);
        assert!(b["thinking"].is_null() && b["temperature"] == 0);
        let b = body(&cfg("deepseek", "deepseek-flash"), &ask(&m, None, true), Level::Bare);
        assert_eq!(b.as_object().unwrap().len(), 3, "modèle, messages, flux : {b}");
    }

    #[test]
    fn refused_parameters_are_dropped_one_by_one() {
        assert_eq!(Level::Full.next(Dialect::OpenAi, false), Level::Alt);
        assert_eq!(Level::Full.next(Dialect::OpenAi, true), Level::NoThinking);
        assert_eq!(Level::Full.next(Dialect::DeepSeek, false), Level::NoThinking);
        assert_eq!(Level::Alt.next(Dialect::Gemini, false), Level::NoThinking);
        assert_eq!(Level::NoThinking.next(Dialect::Gemini, false), Level::Bare);
    }

    #[test]
    fn stream_lines_are_read() {
        let d = parse_line(r#"data: {"choices":[{"delta":{"reasoning_content":"Hmm"}}]}"#).unwrap();
        assert_eq!((d.thought.as_str(), d.answer.as_str()), ("Hmm", ""));
        let d = parse_line(r#"data:{"choices":[{"delta":{"content":"Sens : lisait"}}]}"#).unwrap();
        assert_eq!(d.answer, "Sens : lisait");
        let d = parse_line(r#"data: {"choices":[{"delta":{"reasoning":"Let me","content":null}}]}"#).unwrap();
        assert_eq!(d.thought, "Let me");
        assert!(parse_line("data: [DONE]").unwrap().done);
        assert_eq!(parse_line(r#"data: {"error":{"message":"overloaded"}}"#).unwrap().error.as_deref(), Some("overloaded"));
        assert!(parse_line(": OPENROUTER PROCESSING").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn models_are_listed_and_picked() {
        let v = json!({ "data": [
            { "id": "gpt-4.1-mini" }, { "id": "gpt-5.4-mini" }, { "id": "gpt-5.4-mini-2026-03-01" }, { "id": "gpt-5.2-mini" },
            { "id": "gpt-5.4" }, { "id": "text-embedding-3-small" }, { "id": "gpt-4o-mini-tts" }, { "id": "gpt-realtime" }
        ]});
        let ids = chat_models(&v);
        assert!(!ids.iter().any(|x| x.contains("embed") || x.contains("tts") || x.contains("realtime")));
        assert_eq!(pick(find("openai").unwrap(), &ids).as_deref(), Some("gpt-5.4-mini"));

        let v = json!({ "object": "list", "data": [
            { "id": "models/gemini-2.5-flash" }, { "id": "models/gemini-3.8-flash" }, { "id": "models/gemini-3.8-flash-lite" },
            { "id": "models/gemini-3.9-flash-preview" }, { "id": "models/gemini-embedding-001" }, { "id": "models/gemini-3.8-pro" }
        ]});
        let ids = chat_models(&v);
        assert!(ids.contains(&"gemini-3.8-flash".to_string()) && !ids.iter().any(|x| x.contains("embedding")));
        assert_eq!(pick(find("gemini").unwrap(), &ids).as_deref(), Some("gemini-3.8-flash"));

        let ids = chat_models(&json!({ "data": [{ "id": "deepseek-v4-pro" }, { "id": "deepseek-flash" }] }));
        assert_eq!(pick(find("deepseek").unwrap(), &ids).as_deref(), Some("deepseek-flash"));
        let ids = vec!["claude-opus-4-8".to_string(), "claude-haiku-4-5-20251001".into(), "claude-sonnet-4-6".into()];
        assert_eq!(pick(find("anthropic").unwrap(), &ids).as_deref(), Some("claude-haiku-4-5-20251001"));
        let v = json!({ "data": [
            { "id": "mistral-small-latest", "capabilities": { "completion_chat": true } },
            { "id": "mistral-embed", "capabilities": { "completion_chat": false } },
            { "id": "codestral-embed-2505", "capabilities": { "completion_chat": false } }
        ]});
        assert_eq!(chat_models(&v), vec!["mistral-small-latest"]);
        assert_eq!(pick(find("custom").unwrap(), &["llama3.2".to_string()]).as_deref(), Some("llama3.2"));
        assert_eq!(pick(find("custom").unwrap(), &[]), None);
    }

    #[test]
    fn errors_say_what_to_do() {
        let c = cfg("deepseek", "deepseek-flash");
        assert!(api_error(&c, 402, r#"{"error":{"message":"Insufficient Balance"}}"#).to_string().contains("crédit"));
        assert!(api_error(&c, 401, r#"{"error":{"message":"Authentication Fails"}}"#).to_string().contains("refuse cette clé"));
        let g = cfg("gemini", "gemini-3.8-flash");
        assert!(api_error(&g, 400, r#"[{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}]"#).to_string().contains("refuse cette clé"));
        assert!(api_error(&g, 429, r#"{"error":{"message":"Quota exceeded for metric: generate_content_free_tier_requests, limit: 20, model: gemini-3.8-flash","status":"RESOURCE_EXHAUSTED","details":[{"quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]}}"#).to_string().contains("quota"));
        assert!(api_error(&c, 404, "{}").to_string().contains("deepseek-flash"));
        assert!(api_error(&c, 503, "").to_string().contains("souci"));
        let a = cfg("anthropic", "claude-haiku-4-5");
        assert!(api_error(&a, 400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#).to_string().contains("crédit"));
        assert_eq!(message_of(r#"{"detail":"Not found"}"#), "Not found");
    }

    /// Un faux fournisseur sur ce Mac : il refuse `reasoning_effort` (comme un
    /// ancien modèle), répond au fil de l'eau, et refuse la clé « mauvaise ».
    #[tokio::test]
    async fn streams_and_drops_refused_parameters() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                counter.fetch_add(1, Ordering::SeqCst);
                // la demande entière : en-têtes, puis le corps annoncé
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let reply = if req.contains("Bearer mauvaise") {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":{\"message\":\"Incorrect API key provided\"}}".to_string()
                } else if req.contains("reasoning_effort") {
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":{\"message\":\"Unsupported parameter: 'reasoning_effort'\"}}".to_string()
                } else {
                    let mut r = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_string();
                    for chunk in [
                        r#"{"choices":[{"delta":{"role":"assistant","reasoning_content":"Hmm"}}]}"#,
                        r#"{"choices":[{"delta":{"content":"\nSens : li"}}]}"#,
                        r#"{"choices":[{"delta":{"content":"sait"}}]}"#,
                    ] {
                        r += &format!("data: {chunk}\n\n");
                    }
                    r += ": commentaire\n\ndata: [DONE]\n\n";
                    r
                };
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let mut cfg = Config { provider: find("openai").unwrap(), name: "Test".into(), base: format!("http://{addr}/v1"), key: "k".into(), model: "gpt-ancien".into() };
        let m = [("user", "ciao".to_string())];
        let mut pieces = Vec::new();
        let out = run(&cfg, ask(&m, None, true), || Ok(false), |p| {
            pieces.push(match p {
                Piece::Thought(t) => format!("T:{t}"),
                Piece::Answer(t) => format!("A:{t}"),
            });
            true
        })
        .await
        .unwrap();
        assert_eq!((out.thought.as_str(), out.answer.as_str(), out.stopped), ("Hmm", "Sens : lisait", false));
        assert_eq!(pieces, ["T:Hmm", "A:Sens : li", "A:sait"]);
        // « none », puis « minimal », puis sans réglage de la réflexion
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        // retenu pour ce modèle : une seule demande la fois suivante
        let raw = generate(&cfg, &m, 72, || Ok(false), |_| true).await.unwrap();
        assert_eq!(raw, "Sens : lisait");
        assert_eq!(hits.load(Ordering::SeqCst), 4);
        // arrêt demandé avant la réponse : rien n'est écrit, rien n'est perdu
        let out = run(&cfg, ask(&m, None, true), || Ok(true), |_| true).await.unwrap();
        assert!(out.stopped && out.answer.is_empty());
        // requête dépassée par une plus récente
        let e = run(&cfg, ask(&m, None, true), || Err(anyhow!("interrompu")), |_| true).await.err().unwrap();
        assert_eq!(e.to_string(), "interrompu");
        // clé refusée : un message pour l'apprenant
        cfg.key = "mauvaise".into();
        let e = generate(&cfg, &m, 72, || Ok(false), |_| true).await.unwrap_err();
        assert!(e.to_string().contains("refuse cette clé") && !is_unreachable(&e), "{e}");
        // serveur éteint : injoignable (la traduction se replie alors sur ce Mac)
        cfg.base = "http://127.0.0.1:9/v1".into();
        let e = generate(&cfg, &m, 72, || Ok(false), |_| true).await.unwrap_err();
        assert!(is_unreachable(&e), "{e}");
    }

    /// Vrai fournisseur : `LUMEN_ONLINE="deepseek:sk-…"` (ou `gemini:AIza…`,
    /// `custom:http://localhost:11434/v1`, `LUMEN_ONLINE_MODEL` pour un autre modèle).
    #[tokio::test]
    #[ignore]
    async fn online_live() {
        let spec = std::env::var("LUMEN_ONLINE").expect("LUMEN_ONLINE=fournisseur:clé");
        let (id, secret) = spec.split_once(':').unwrap();
        let (key, url) = if id == "custom" { ("", secret) } else { (secret, "") };
        let model = std::env::var("LUMEN_ONLINE_MODEL").unwrap_or_default();
        let chk = check(id, key, url, &model).await.unwrap();
        println!("{} modèles, retenu : {} ({} ms)", chk.models.len(), chk.model, chk.ms);
        let p = find(id).unwrap();
        let base = if id == "custom" { normalize_url(url).unwrap() } else { p.base.to_string() };
        let cfg = Config { provider: p, name: p.name.into(), base, key: key.into(), model: chk.model };
        for (native, lang, word, sentence) in [
            ("fr", "it", "pesca", "Ho mangiato una pesca matura al mercato."),
            ("fr", "de", "aufgegeben", "Nach drei Versuchen hat er endlich aufgegeben."),
            ("en", "es", "embarazada", "Mi hermana está embarazada de tres meses."),
        ] {
            let start = Instant::now();
            let raw = generate(&cfg, &crate::ai::word_messages(native, lang, word, sentence, ""), 72, || Ok(false), |_| true).await.unwrap();
            println!("{word} → {:?} ({} ms)", crate::ai::parse_word_answer(&raw), start.elapsed().as_millis());
        }
        let messages = crate::ai::chat_messages("fr", "it", &crate::ai::Learner::knows(1200), None, &[], "Quelle différence entre « ho mangiato » et « mangiavo » ?", &[]);
        for think in [None, Some("low")] {
            let start = Instant::now();
            let (mut thought, mut first) = (0usize, None);
            let out = run(&cfg, Ask { messages: &messages, max_tokens: 1600, think, exact: false }, || Ok(false), |p| {
                match p {
                    Piece::Thought(t) => thought += t.len(),
                    Piece::Answer(_) => {
                        first.get_or_insert(start.elapsed().as_millis());
                    }
                }
                true
            })
            .await
            .unwrap();
            println!("chat (réflexion {think:?}) : {} octets pensés, premier mot à {:?} ms, {} octets en {} ms\n{}\n", thought, first, out.answer.len(), start.elapsed().as_millis(), out.answer);
        }
    }
}
