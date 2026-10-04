//! Podcasts sur mesure. Gemini (l'IA en ligne de Google, avec la clé
//! personnelle et gratuite de l'apprenant) écrit un podcast dans la langue
//! étudiée, au niveau, sur le sujet et à la durée choisis, puis le dit à une ou
//! deux voix. Lumen assemble le son et garde le texte exact pour la leçon : la
//! lanterne est ensuite calée par Whisper, sur ce Mac.
//!
//! Pourquoi l'API Gemini et pas Gemini Notebook (ex-NotebookLM) : Gemini
//! Notebook n'ouvre aucun accès aux applications des particuliers (seulement
//! une API d'entreprise, sur Google Cloud). L'API Gemini a les mêmes voix, et
//! Lumen écrit lui-même la commande : niveau tenu, mots en apprentissage
//! glissés dans le texte, leçon sans faute de transcription.
//!
//! Ce qui part en ligne : le sujet, les précisions et, si l'apprenant le veut,
//! quelques mots en apprentissage. Rien d'autre.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::i18n::t;

const API: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Ce que l'apprenant demande (miroir de `PodcastRequest` dans types.ts).
#[derive(Deserialize, Debug, Clone)]
pub struct Request {
    pub topic: String,
    /// 1 (A1) à 5 (C1)
    pub level: u8,
    pub minutes: u32,
    /// « talk » (deux animateurs), « story » (un conteur), « debate » (deux avis)
    pub format: String,
    #[serde(default)]
    pub details: String,
    /// glisser dans le texte quelques mots en apprentissage
    #[serde(default)]
    pub use_words: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    pub name: String,
    pub female: bool,
}

/// Une réplique (un paragraphe de la leçon).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub host: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Script {
    pub title: String,
    pub hosts: Vec<Host>,
    pub lines: Vec<Line>,
}

/// Modèles disponibles avec la clé, du meilleur au repli.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Models {
    pub text: Vec<String>,
    pub tts: Vec<String>,
}

pub fn client() -> Result<reqwest::Client> {
    // une réponse longue (texte réfléchi, deux minutes de voix) peut prendre une minute ou deux
    Ok(reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(300)).build()?)
}

// ---------- niveau, durée, format ----------

/// Débit visé (mots par minute) : lent et net au début, celui d'un vrai podcast ensuite.
const WPM: [f64; 5] = [85.0, 100.0, 120.0, 140.0, 155.0];

fn ix(level: u8) -> usize {
    (level.clamp(1, 5) - 1) as usize
}

/// Mots visés pour une durée (caractères en japonais, découpé caractère par
/// caractère ; un mot coréen et une syllabe vietnamienne ne pèsent pas pareil).
pub fn target_units(lang: &str, level: u8, minutes: u32) -> usize {
    let factor = match lang {
        "ja" => 2.1,
        "ko" => 0.8,
        "vi" => 1.3,
        _ => 1.0,
    };
    (WPM[ix(level)] * factor * minutes.clamp(1, 30) as f64).round() as usize
}

const LEVELS: [&str; 5] = [
    "complete beginners (CEFR A1): only very common, concrete words (roughly the 500 most frequent), very short sentences of 4 to 8 words, the present tense, and plenty of natural repetition of the key words. When a harder word is unavoidable, explain it right away with simpler words. No idioms.",
    "elementary learners (CEFR A2): everyday vocabulary (roughly the 1,000 most frequent words), short sentences of 6 to 12 words, mostly the present and a simple past, a few very common expressions, some repetition of the key words. Briefly explain any less common word.",
    "intermediate learners (CEFR B1): clear, connected speech, sentences of up to about 18 words, the usual tenses, common idioms when they are easy to guess from context. Rephrase rarer words in passing.",
    "upper-intermediate learners (CEFR B2): natural, fluent speech with nuance, varied tenses and connectors, colloquial expressions and idioms in moderation.",
    "advanced learners (CEFR C1): native-like speech, rich and precise vocabulary, idioms, humour and cultural references, the texture of a real podcast.",
];

fn format_rules(format: &str) -> &'static str {
    match format {
        "story" => "A story told by a single narrator (exactly one host). Vivid but simple descriptions, a clear beginning, middle and end. Dialogue inside the story stays in the narrator's voice. Each line is a paragraph of 2 to 5 sentences.",
        "debate" => "A friendly debate between two hosts (exactly two, one female and one male) who defend different opinions on the topic with arguments and concrete examples, answer each other respectfully and end on what they agree about. Turns of 1 to 4 sentences.",
        _ => "A warm, lively conversation between two podcast hosts (exactly two, one female and one male) who explore the topic together, like an audio overview: one explains, the other reacts, asks questions, gives examples and sums up. Alternate often; turns of 1 to 4 sentences. Open with a short greeting that announces the topic, close with a short recap and a goodbye.",
    }
}

/// La commande passée à Gemini. En anglais (consignes mieux suivies) ; le podcast
/// lui-même est dans la langue étudiée.
pub fn script_prompt(lang: &str, r: &Request, words: &[String]) -> String {
    let language = crate::ai::lang_name_en(lang);
    let units = target_units(lang, r.level, r.minutes);
    let length = if lang == "ja" { format!("about {units} Japanese characters") } else { format!("about {units} words") };
    let mut p = format!(
        "Write the script of an audio podcast in {language} for {}\n\nTopic: {}\nFormat: {}\nLength: {length} in total (about {} minutes when spoken at the right pace). Do not stop early.\n",
        LEVELS[ix(r.level)],
        r.topic.trim(),
        format_rules(&r.format),
        r.minutes.clamp(1, 30),
    );
    let details = r.details.trim();
    if !details.is_empty() {
        p += &format!("Listener's wishes: {details}\n");
    }
    if !words.is_empty() {
        p += &format!(
            "The listener is learning these {language} words and expressions; work in as many as fit naturally (any form, never forced): {}.\n",
            words.join(", ")
        );
    }
    p += &format!(
        "\nRules:\n\
         - Everything in {language} only (proper nouns aside), even if the topic is given in another language. No translations.\n\
         - Plain spoken text: no markdown, no emojis, no stage directions, no sound effects, no notes in brackets, no speaker names inside the lines.\n\
         - Write numbers, dates and abbreviations in words, as they are spoken.\n\
         - Hosts: first names that are common where {language} is spoken; gender \"female\" or \"male\".\n\
         - Title: short and inviting, in {language}, without quotes.\n\
         - `host` is the index of the speaking host in `hosts`."
    );
    p
}

fn script_schema() -> Value {
    json!({
        "type": "OBJECT",
        "properties": {
            "title": { "type": "STRING" },
            "hosts": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "name": { "type": "STRING" },
                        "gender": { "type": "STRING", "enum": ["female", "male"] }
                    },
                    "required": ["name", "gender"]
                }
            },
            "lines": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "host": { "type": "INTEGER" },
                        "text": { "type": "STRING" }
                    },
                    "required": ["host", "text"]
                }
            }
        },
        "required": ["title", "hosts", "lines"],
        "propertyOrdering": ["title", "hosts", "lines"]
    })
}

// ---------- lecture du script ----------

/// Pictogrammes de décor (le texte de la leçon n'en veut pas).
fn decorative(c: char) -> bool {
    matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F | 0x200D)
}

/// Une réplique propre : sans indications entre crochets, ni gras, ni émojis,
/// ni nom d'animateur en tête.
fn clean_line(s: &str, names: &[String]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth > 0 => {}
            '*' | '#' => {}
            c if decorative(c) => {}
            c => out.push(c),
        }
    }
    let mut line = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((head, rest)) = line.split_once(':') {
        let head = head.trim().to_lowercase();
        if !rest.trim().is_empty() && (head.starts_with("speaker") || names.iter().any(|n| !n.is_empty() && *n == head)) {
            line = rest.trim().to_string();
        }
    }
    line
}

fn unreadable() -> anyhow::Error {
    anyhow!(t("Gemini a répondu dans un format inattendu. Réessayez.", "Gemini answered in an unexpected format. Try again."))
}

/// Le script renvoyé par Gemini (JSON, parfois entouré de ```json … ```).
pub fn parse_script(raw: &str, format: &str) -> Result<Script> {
    let (a, b) = (raw.find('{'), raw.rfind('}'));
    let body = match (a, b) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => return Err(unreadable()),
    };
    let v: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    let want = if format == "story" { 1 } else { 2 };
    let mut hosts: Vec<Host> = v["hosts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|h| Host {
                    name: h["name"].as_str().unwrap_or("").trim().to_string(),
                    female: !h["gender"].as_str().unwrap_or("").eq_ignore_ascii_case("male"),
                })
                .collect()
        })
        .unwrap_or_default();
    hosts.truncate(want);
    while hosts.len() < want {
        let female = hosts.first().map(|h| !h.female).unwrap_or(true);
        hosts.push(Host { name: String::new(), female });
    }
    let names: Vec<String> = hosts.iter().map(|h| h.name.to_lowercase()).collect();
    let lines: Vec<Line> = v["lines"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|l| {
            let host = l["host"].as_u64().or_else(|| l["host"].as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0) as usize;
            let text = clean_line(l["text"].as_str()?, &names);
            (!text.is_empty()).then(|| Line { host: host.min(want - 1), text })
        })
        .collect();
    if lines.is_empty() {
        return Err(anyhow!(t("Gemini n'a pas écrit de podcast. Réessayez, ou changez de sujet.", "Gemini didn't write a podcast. Try again, or change the topic.")));
    }
    let quotes: &[char] = &['"', '\'', '«', '»', '“', '”', '„', '‚', '‘', '’', '「', '」'];
    let mut title = clean_line(v["title"].as_str().unwrap_or(""), &[]).trim_matches(quotes).trim().to_string();
    if title.is_empty() {
        title = lines[0].text.split_whitespace().take(6).collect::<Vec<_>>().join(" ");
    }
    Ok(Script { title, hosts, lines })
}

/// Texte de la leçon (une réplique par paragraphe) et position de chaque
/// réplique dans ce texte (unités UTF-16).
pub fn layout(lines: &[Line]) -> (String, Vec<usize>) {
    let mut text = String::new();
    let mut bases = Vec::with_capacity(lines.len());
    let mut len16 = 0usize;
    for (k, l) in lines.iter().enumerate() {
        if k > 0 {
            text.push_str("\n\n");
            len16 += 2;
        }
        bases.push(len16);
        text.push_str(&l.text);
        len16 += l.text.encode_utf16().count();
    }
    (text, bases)
}

// ---------- voix ----------

/// Voix de Gemini, par genre : chaleureuses et nettes, faciles à suivre.
const FEMALE: [&str; 6] = ["Aoede", "Kore", "Leda", "Sulafat", "Despina", "Erinome"];
const MALE: [&str; 6] = ["Puck", "Charon", "Achird", "Iapetus", "Algieba", "Umbriel"];

/// Une voix par animateur, de son genre, toutes différentes ; elles changent
/// d'un podcast à l'autre (`seed`).
pub fn voices(hosts: &[Host], seed: u64) -> Vec<&'static str> {
    let mut used: Vec<&'static str> = Vec::new();
    for (i, h) in hosts.iter().enumerate() {
        let pool: &[&'static str] = if h.female { &FEMALE } else { &MALE };
        let mut k = (seed as usize).wrapping_add(i * 3) % pool.len();
        while used.contains(&pool[k]) {
            k = (k + 1) % pool.len();
        }
        used.push(pool[k]);
    }
    used
}

fn seed_of(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Au-delà, le texte est enregistré en plusieurs fois : une voix de synthèse
/// reste plus juste sur une minute et demie que sur quinze, et un raté ne
/// coûte qu'un morceau.
const CHUNK_SECS: f64 = 100.0;

/// Morceaux de répliques enregistrés d'un seul tenant.
pub fn chunks(lines: &[Line], lang: &str, level: u8) -> Vec<Range<usize>> {
    let per_sec = target_units(lang, level, 1) as f64 / 60.0;
    let mut out = Vec::new();
    let (mut start, mut secs) = (0usize, 0.0f64);
    for (i, l) in lines.iter().enumerate() {
        let s = crate::text::word_count(&l.text, lang) as f64 / per_sec;
        if i > start && secs + s > CHUNK_SECS {
            out.push(start..i);
            start = i;
            secs = 0.0;
        }
        secs += s;
    }
    if start < lines.len() {
        out.push(start..lines.len());
    }
    out
}

/// Noms des voix dans le texte envoyé à la synthèse (jamais prononcés).
const LABELS: [&str; 2] = ["Speaker1", "Speaker2"];

const PACE: [&str; 5] = [
    "slowly and very clearly, with a short pause after each sentence",
    "slowly and clearly",
    "clearly, at a relaxed pace",
    "at a natural pace",
    "naturally and with energy, like native podcast hosts",
];

/// Requête de synthèse d'un morceau : à deux voix s'il en a deux, sinon à une.
pub fn tts_body(lang: &str, level: u8, lines: &[Line], voices: &[&str]) -> Value {
    let language = crate::ai::lang_name_en(lang);
    let pace = PACE[ix(level)];
    let two = voices.len() >= 2 && lines.iter().any(|l| l.host == 0) && lines.iter().any(|l| l.host > 0);
    let voice = |name: &str| json!({ "prebuiltVoiceConfig": { "voiceName": name } });
    let (text, speech) = if two {
        let mut text = format!("Read this {language} podcast conversation between {} and {} aloud, warmly, {pace}:\n\n", LABELS[0], LABELS[1]);
        for l in lines {
            text += &format!("{}: {}\n", LABELS[l.host.min(1)], l.text);
        }
        let speakers: Vec<Value> = (0..2).map(|k| json!({ "speaker": LABELS[k], "voiceConfig": voice(voices[k]) })).collect();
        (text, json!({ "multiSpeakerVoiceConfig": { "speakerVoiceConfigs": speakers } }))
    } else {
        let host = lines.first().map(|l| l.host).unwrap_or(0).min(voices.len().saturating_sub(1));
        let body = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n\n");
        (format!("Read this {language} text aloud, warmly, {pace}:\n\n{body}"), json!({ "voiceConfig": voice(voices.get(host).copied().unwrap_or(FEMALE[0])) }))
    };
    json!({
        "contents": [{ "role": "user", "parts": [{ "text": text }] }],
        "generationConfig": { "responseModalities": ["AUDIO"], "speechConfig": speech }
    })
}

/// Fréquence annoncée par le type du son (« audio/L16;codec=pcm;rate=24000 »).
fn rate_of(mime: &str) -> Option<u32> {
    mime.split(';').find_map(|p| p.trim().strip_prefix("rate=")?.parse().ok())
}

/// Le son d'une réponse : PCM 16 bits mono brut ou WAV, en base64.
pub fn audio_of(v: &Value) -> Option<(u32, Vec<i16>)> {
    let parts = v["candidates"][0]["content"]["parts"].as_array()?;
    let mut rate = 0u32;
    let mut all: Vec<i16> = Vec::new();
    for p in parts {
        let Some(d) = p.get("inlineData").or_else(|| p.get("inline_data")) else { continue };
        let mime = d.get("mimeType").or_else(|| d.get("mime_type")).and_then(Value::as_str).unwrap_or("");
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d["data"].as_str().unwrap_or("")) else { continue };
        let (r, s) = if bytes.starts_with(b"RIFF") {
            crate::voice::read_pcm(&bytes)?
        } else {
            (rate_of(mime).unwrap_or(24_000), bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
        };
        if rate == 0 {
            rate = r;
        }
        all.extend(if r == rate { s } else { resample(&s, r, rate) });
    }
    (rate > 0 && !all.is_empty()).then_some((rate, all))
}

/// Changement de fréquence linéaire (un morceau venu d'un autre modèle).
fn resample(s: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || s.is_empty() {
        return s.to_vec();
    }
    let n = (s.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * from as f64 / to as f64;
            let k = x.floor() as usize;
            let f = x - k as f64;
            let a = s[k.min(s.len() - 1)] as f64;
            let b = s[(k + 1).min(s.len() - 1)] as f64;
            (a + (b - a) * f).round() as i16
        })
        .collect()
}

/// Les `n` répliques d'un morceau (secondes depuis son début), coupées aux
/// `n - 1` plus longs silences : la voix marque une pause à chaque réplique.
/// `None` s'il n'y a pas assez de silences pour cela.
fn turns(s: &[i16], rate: u32, n: usize) -> Option<Vec<(f64, f64)>> {
    let total = s.len() as f64 / rate.max(1) as f64;
    if n <= 1 {
        return Some(vec![(0.0, total)]);
    }
    let frame = (rate as usize / 50).max(1); // 20 ms
    let energy: Vec<f32> = s.chunks(frame).map(|c| (c.iter().map(|v| (*v as f32).powi(2)).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let peak = energy.iter().cloned().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return None;
    }
    let quiet = peak * 0.05;
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut from: Option<usize> = None;
    for (i, e) in energy.iter().enumerate() {
        if *e < quiet {
            from.get_or_insert(i);
        } else if let Some(a) = from.take() {
            // une vraie pause : au moins 160 ms, pas le silence d'avant la voix
            if a > 0 && i - a >= 8 {
                gaps.push((a, i));
            }
        }
    }
    if gaps.len() < n - 1 {
        return None;
    }
    gaps.sort_by(|x, y| (y.1 - y.0).cmp(&(x.1 - x.0)));
    gaps.truncate(n - 1);
    gaps.sort();
    let sec = |f: usize| (f * frame) as f64 / rate as f64;
    let mut out = Vec::with_capacity(n);
    let mut at = 0.0;
    for (a, b) in gaps {
        out.push((at, sec(a)));
        at = sec(b);
    }
    out.push((at, total));
    Some(out)
}

/// Répartit le temps d'un morceau sur ses répliques, au prorata de leur longueur.
fn share(lines: &[Line], t0: f64, t1: f64) -> Vec<(f64, f64)> {
    let w: Vec<f64> = lines.iter().map(|l| l.text.chars().count() as f64 + 1.0).collect();
    let total: f64 = w.iter().sum::<f64>().max(1.0);
    let mut at = t0;
    w.iter()
        .map(|x| {
            let d = (t1 - t0) * x / total;
            let span = (at, at + d);
            at += d;
            span
        })
        .collect()
}

// ---------- appels à l'API ----------

/// Ce qu'une réponse d'erreur de Google demande de faire.
#[derive(Debug, PartialEq)]
enum Next {
    /// réessayer le même modèle après ce délai
    Wait(Duration),
    /// passer au modèle suivant (`quota` : quota gratuit épuisé)
    Skip { quota: bool },
    /// arrêter : le message s'adresse à l'apprenant
    Stop(String),
}

/// « 17s », « 0.5s » → durée.
fn parse_delay(s: &str) -> Option<Duration> {
    s.trim().strip_suffix('s')?.parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0).map(Duration::from_secs_f64)
}

fn google_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

fn next_step(status: u16, body: &str, attempt: u32) -> Next {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let msg = google_message(body);
    let delay = v["error"]["details"].as_array().and_then(|d| d.iter().find_map(|x| x["retryDelay"].as_str()).and_then(parse_delay));
    match status {
        429 => {
            // quota du jour : inutile d'attendre, un autre modèle a son propre quota
            if body.contains("PerDay") {
                return Next::Skip { quota: true };
            }
            match delay {
                Some(d) if d <= Duration::from_secs(70) && attempt < 3 => Next::Wait(d + Duration::from_secs(1)),
                None if attempt < 2 => Next::Wait(Duration::from_secs(20)),
                _ => Next::Skip { quota: true },
            }
        }
        404 => Next::Skip { quota: false },
        500 | 502 | 503 | 504 if attempt < 2 => Next::Wait(Duration::from_secs(4 << attempt)),
        500 | 502 | 503 | 504 => Next::Skip { quota: false },
        400 if body.contains("API_KEY_INVALID") || msg.contains("API key not valid") => Next::Stop(
            t(
                "Google refuse cette clé Gemini. Vérifiez-la dans Google AI Studio, puis collez-la à nouveau (Réglages › Podcasts).",
                "Google refuses this Gemini key. Check it in Google AI Studio, then paste it again (Settings › Podcasts).",
            )
            .into(),
        ),
        400 if msg.contains("location is not supported") => Next::Stop(
            t("Gemini n'est pas proposé dans le pays d'où part la demande.", "Gemini isn't offered in the country the request comes from.").into(),
        ),
        401 | 403 => Next::Stop(tr!(
            "Google n'autorise pas cette clé à utiliser Gemini ({msg}). Créez une clé dans Google AI Studio.",
            "Google doesn't allow this key to use Gemini ({msg}). Create a key in Google AI Studio."
        )),
        // requête qu'un modèle ne sait pas traiter : un autre le saura peut-être
        _ => Next::Skip { quota: false },
    }
}

fn offline(e: reqwest::Error) -> anyhow::Error {
    if e.is_timeout() {
        anyhow!(t("Gemini a mis trop de temps à répondre. Réessayez.", "Gemini took too long to answer. Try again."))
    } else {
        anyhow!(t("Gemini est injoignable. Vérifiez la connexion à Internet.", "Gemini can't be reached. Check the Internet connection."))
    }
}

/// Appelle `generateContent` sur le premier modèle qui répond : quota épuisé ou
/// modèle absent, on passe au suivant ; serveur surchargé, on réessaie.
async fn generate(c: &reqwest::Client, key: &str, models: &[String], body: &Value) -> Result<Value> {
    let payload = serde_json::to_vec(body)?;
    let mut quota = false;
    let mut last: Option<String> = None;
    for m in models {
        let mut attempt = 0u32;
        loop {
            let res = c
                .post(format!("{API}/models/{m}:generateContent"))
                .header("x-goog-api-key", key)
                .header("content-type", "application/json")
                .body(payload.clone())
                .send()
                .await;
            let res = match res {
                Ok(r) => r,
                Err(e) if attempt < 2 && (e.is_timeout() || e.is_connect()) => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
                Err(e) => return Err(offline(e)),
            };
            let status = res.status().as_u16();
            let text = res.text().await.map_err(offline)?;
            if status == 200 {
                return serde_json::from_str(&text).map_err(|_| unreadable());
            }
            match next_step(status, &text, attempt) {
                Next::Wait(d) => {
                    attempt += 1;
                    tokio::time::sleep(d).await;
                }
                Next::Skip { quota: q } => {
                    quota |= q;
                    last = Some(google_message(&text));
                    break;
                }
                Next::Stop(msg) => return Err(anyhow!(msg)),
            }
        }
    }
    Err(anyhow!(if quota {
        t(
            "Le quota gratuit de Gemini est épuisé pour le moment. Réessayez plus tard (il se renouvelle chaque jour).",
            "Gemini's free quota is used up for now. Try again later (it renews every day).",
        )
        .to_string()
    } else if let Some(m) = last {
        tr!("Gemini a répondu : {m}", "Gemini answered: {m}")
    } else {
        t("Aucun modèle Gemini n'est disponible avec cette clé.", "No Gemini model is available with this key.").to_string()
    }))
}

/// Pourquoi une réponse n'apporte rien (sujet refusé, réponse vide).
fn empty_reason(v: &Value) -> anyhow::Error {
    let reason = v["promptFeedback"]["blockReason"].as_str().or_else(|| v["candidates"][0]["finishReason"].as_str()).unwrap_or("");
    if matches!(reason, "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" | "OTHER") {
        anyhow!(t("Gemini a refusé ce sujet. Essayez-en un autre.", "Gemini declined this topic. Try another one."))
    } else {
        anyhow!(t("Gemini n'a rien répondu. Réessayez dans un instant.", "Gemini gave no answer. Try again in a moment."))
    }
}

fn text_of(v: &Value) -> Option<String> {
    let parts = v["candidates"][0]["content"]["parts"].as_array()?;
    // les pensées du modèle (« thought ») ne font pas partie de la réponse
    let s: String = parts.iter().filter(|p| !p["thought"].as_bool().unwrap_or(false)).filter_map(|p| p["text"].as_str()).collect();
    (!s.trim().is_empty()).then_some(s)
}

/// Version d'un modèle (« gemini-3.8-flash » → (3, 8)) et ce qui suit.
fn version(name: &str) -> Option<((u32, u32), &str)> {
    let rest = name.strip_prefix("gemini-")?;
    let (v, tail) = rest.split_once('-')?;
    let mut it = v.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = match it.next() {
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    Some(((major, minor), tail))
}

/// Les modèles à essayer, du meilleur au repli : pour écrire, les « flash »
/// stables les plus récents ; pour dire, les modèles de voix stables d'abord.
pub fn pick_models(list: &Value) -> Models {
    let names: Vec<&str> = list["models"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|m| m["supportedGenerationMethods"].as_array().map(|a| a.iter().any(|x| x == "generateContent")).unwrap_or(true))
        .filter_map(|m| m["name"].as_str())
        .map(|n| n.trim_start_matches("models/"))
        .collect();
    let mut text: Vec<(bool, std::cmp::Reverse<(u32, u32)>, &str)> = names
        .iter()
        .filter_map(|n| {
            let (v, tail) = version(n)?;
            matches!(tail, "flash" | "flash-lite").then_some((tail == "flash-lite", std::cmp::Reverse(v), *n))
        })
        .collect();
    text.sort();
    let mut tts: Vec<(bool, bool, bool, std::cmp::Reverse<(u32, u32)>, &str)> = names
        .iter()
        .filter_map(|n| {
            let (v, tail) = version(n)?;
            tail.contains("tts").then_some((tail.contains("preview"), tail.contains("pro"), tail.contains("lite"), std::cmp::Reverse(v), *n))
        })
        .collect();
    tts.sort();
    let mut text: Vec<String> = text.into_iter().map(|x| x.2.to_string()).collect();
    let mut tts: Vec<String> = tts.into_iter().map(|x| x.4.to_string()).collect();
    if text.is_empty() {
        text.push("gemini-2.5-flash".into());
    }
    if tts.is_empty() {
        tts.push("gemini-2.5-flash-preview-tts".into());
    }
    Models { text, tts }
}

/// Vérifie la clé et liste les modèles qu'elle ouvre.
pub async fn models(c: &reqwest::Client, key: &str) -> Result<Models> {
    let key = key.trim();
    if key.is_empty() {
        return Err(anyhow!(t("Collez d'abord votre clé Gemini.", "Paste your Gemini key first.")));
    }
    let res = c.get(format!("{API}/models?pageSize=1000")).header("x-goog-api-key", key).send().await.map_err(offline)?;
    let status = res.status().as_u16();
    let body = res.text().await.map_err(offline)?;
    if status != 200 {
        return Err(match next_step(status, &body, 9) {
            Next::Stop(m) => anyhow!(m),
            _ if status == 400 => anyhow!(t(
                "Google refuse cette clé Gemini. Vérifiez-la dans Google AI Studio, puis collez-la à nouveau.",
                "Google refuses this Gemini key. Check it in Google AI Studio, then paste it again."
            )),
            _ => anyhow!(tr!("Gemini a répondu : {}", "Gemini answered: {}", google_message(&body))),
        });
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| unreadable())?;
    Ok(pick_models(&v))
}

// ---------- écriture et enregistrement ----------

/// Gemini écrit le podcast.
pub async fn write(c: &reqwest::Client, key: &str, models: &[String], lang: &str, r: &Request, words: &[String]) -> Result<Script> {
    let body = json!({
        "systemInstruction": { "parts": [{ "text": "You write scripts for audio podcasts that help people learn languages. You follow the requested level, length and format closely, and you always answer with the JSON object requested." }] },
        "contents": [{ "role": "user", "parts": [{ "text": script_prompt(lang, r, words) }] }],
        "generationConfig": { "responseMimeType": "application/json", "responseSchema": script_schema() }
    });
    let v = generate(c, key, models, &body).await?;
    let raw = text_of(&v).ok_or_else(|| empty_reason(&v))?;
    parse_script(&raw, &r.format)
}

pub struct Recording {
    pub path: PathBuf,
    /// texte de la leçon (une réplique par paragraphe)
    pub text: String,
    /// horodatages réplique par réplique (avant le recalage par Whisper)
    pub timings: Vec<[f64; 4]>,
    pub duration: f64,
}

/// Silence entre deux morceaux enregistrés à part.
const GAP_SECS: f64 = 0.5;
/// Morceaux enregistrés en même temps.
const PARALLEL: usize = 2;

/// Gemini dit le podcast, morceau par morceau (deux à la fois) ; Lumen les
/// assemble, égalise le volume et compresse en AAC dans `media/`. La position
/// de chaque réplique (pauses entre les voix) donne celle de ses mots.
pub async fn record(
    c: &reqwest::Client,
    key: &str,
    models: &[String],
    data_dir: &Path,
    lang: &str,
    level: u8,
    script: &Script,
    mut on_progress: impl FnMut(f64),
) -> Result<Recording> {
    use futures_util::StreamExt;
    let (text, bases) = layout(&script.lines);
    let voices = voices(&script.hosts, seed_of(&script.title));
    let parts = chunks(&script.lines, lang, level);
    let total = parts.len();
    let jobs: Vec<_> = parts
        .iter()
        .map(|range| {
            let body = tts_body(lang, level, &script.lines[range.clone()], &voices);
            async move {
                let v = generate(c, key, models, &body).await?;
                audio_of(&v).ok_or_else(|| empty_reason(&v))
            }
        })
        .collect();
    let mut stream = futures_util::stream::iter(jobs).buffered(PARALLEL);
    let mut rate = 0u32;
    let mut all: Vec<i16> = Vec::new();
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let mut done = 0usize;
    while let Some(res) = stream.next().await {
        let (r, s) = res?;
        if rate == 0 {
            rate = r;
        }
        let s = if r == rate { s } else { resample(&s, r, rate) };
        let range = parts[done].clone();
        let lines = &script.lines[range.clone()];
        let ms = |x: f64| (rate as f64 * x / 1000.0) as usize;
        if let Some((first, last, _)) = crate::voice::speech_bounds(&s) {
            let a = first.saturating_sub(ms(20.0));
            let b = (last + ms(120.0)).min(s.len());
            let seg = &s[a..b];
            let start = all.len() as f64 / rate as f64;
            let len = seg.len() as f64 / rate as f64;
            let spans = turns(seg, rate, lines.len()).unwrap_or_else(|| share(lines, 0.0, len));
            for (k, (t0, t1)) in spans.iter().enumerate() {
                crate::lingq::spread_words(&lines[k].text, lang, bases[range.start + k], start + t0, start + t1, &mut timings);
            }
            all.extend_from_slice(seg);
        }
        done += 1;
        if done < total {
            all.extend(std::iter::repeat_n(0i16, (rate as f64 * GAP_SECS) as usize));
        }
        on_progress(done as f64 / total as f64 * 100.0);
    }
    drop(stream);
    if all.is_empty() || rate == 0 {
        return Err(anyhow!(t("Gemini n'a renvoyé aucun son.", "Gemini returned no sound.")));
    }
    crate::voice::level_volume(&mut all);
    let duration = all.len() as f64 / rate as f64;
    let media = crate::media::media_dir(data_dir);
    let work = media.join(format!(".podcast-{}", crate::media::new_stem()));
    tokio::fs::create_dir_all(&work).await?;
    let path = crate::voice::save_m4a(&work, &media, "podcast", rate, &all).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(Recording { path: path?, text, timings, duration })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(format: &str, level: u8, minutes: u32) -> Request {
        Request { topic: "il caffè a Napoli".into(), level, minutes, format: format.into(), details: String::new(), use_words: false }
    }

    #[test]
    fn prompt_carries_level_length_and_words() {
        let mut r = req("talk", 2, 5);
        r.details = "con un po' di umorismo".into();
        let p = script_prompt("it", &r, &["tazzina".into(), "fare la scarpetta".into()]);
        assert!(p.contains("in Italian for elementary learners (CEFR A2)"));
        assert!(p.contains("Topic: il caffè a Napoli"));
        assert!(p.contains("about 500 words"), "{p}");
        assert!(p.contains("Listener's wishes: con un po' di umorismo"));
        assert!(p.contains("tazzina, fare la scarpetta"));
        assert!(p.contains("exactly two"));
        // japonais : en caractères
        assert!(script_prompt("ja", &req("story", 1, 3), &[]).contains("Japanese characters"));
        assert!(script_prompt("de", &req("story", 1, 3), &[]).contains("exactly one host"));
    }

    #[test]
    fn script_is_cleaned() {
        let raw = "```json\n{\"title\": \"«Il caffè» ☕\", \"hosts\": [{\"name\": \"Giulia\", \"gender\": \"female\"}, {\"name\": \"Marco\", \"gender\": \"male\"}, {\"name\": \"Luca\", \"gender\": \"male\"}],\n\"lines\": [{\"host\": 0, \"text\": \"Giulia: Ciao a **tutti**! [ride] Oggi parliamo di caffè.\"}, {\"host\": 2, \"text\": \"Ciao   Giulia.\"}, {\"host\": 1, \"text\": \"  \"}, {\"host\": \"1\", \"text\": \"Speaker2: Mi piace!\"}]}\n```";
        let s = parse_script(raw, "talk").unwrap();
        assert_eq!(s.title, "Il caffè");
        assert_eq!(s.hosts, vec![Host { name: "Giulia".into(), female: true }, Host { name: "Marco".into(), female: false }]);
        assert_eq!(
            s.lines,
            vec![
                Line { host: 0, text: "Ciao a tutti! Oggi parliamo di caffè.".into() },
                Line { host: 1, text: "Ciao Giulia.".into() },
                Line { host: 1, text: "Mi piace!".into() },
            ]
        );
        // un conteur seul : une voix, toutes les répliques à lui
        let s = parse_script(raw, "story").unwrap();
        assert_eq!(s.hosts.len(), 1);
        assert!(s.lines.iter().all(|l| l.host == 0));
        // il manque un animateur : il est ajouté, de l'autre genre
        let s = parse_script("{\"title\":\"\",\"hosts\":[{\"name\":\"Ana\",\"gender\":\"female\"}],\"lines\":[{\"host\":1,\"text\":\"Olá, tudo bem?\"}]}", "debate").unwrap();
        assert_eq!(s.hosts.len(), 2);
        assert!(!s.hosts[1].female);
        assert_eq!(s.title, "Olá, tudo bem?");
        assert!(parse_script("pas de json", "talk").is_err());
        assert!(parse_script("{\"title\":\"x\",\"hosts\":[],\"lines\":[]}", "talk").is_err());
    }

    #[test]
    fn layout_positions_match_the_tokenizer() {
        let lines = vec![Line { host: 0, text: "Ciao! Perché?".into() }, Line { host: 1, text: "Così è.".into() }];
        let (text, bases) = layout(&lines);
        assert_eq!(text, "Ciao! Perché?\n\nCosì è.");
        assert_eq!(bases, vec![0, 15]);
        let mut timings = Vec::new();
        for (k, l) in lines.iter().enumerate() {
            crate::lingq::spread_words(&l.text, "it", bases[k], k as f64, k as f64 + 1.0, &mut timings);
        }
        let words: Vec<(usize, usize)> = crate::text::tokenize(&text, "it").iter().filter(|t| t.w).map(|t| (t.s, t.e)).collect();
        let got: Vec<(usize, usize)> = timings.iter().map(|x| (x[0] as usize, x[1] as usize)).collect();
        assert_eq!(got, words);
    }

    #[test]
    fn chunks_stay_short() {
        // A2 : 100 mots par minute, donc au plus ~166 mots par morceau
        let line = |n: usize| Line { host: n % 2, text: "parola ".repeat(40).trim_end().to_string() };
        let lines: Vec<Line> = (0..10).map(line).collect();
        let parts = chunks(&lines, "it", 2);
        assert_eq!(parts, vec![0..4, 4..8, 8..10]);
        assert_eq!(chunks(&lines[..1], "it", 2), vec![0..1]);
    }

    #[test]
    fn tts_body_uses_one_or_two_voices() {
        let lines = vec![Line { host: 0, text: "Ciao!".into() }, Line { host: 1, text: "Ciao a te.".into() }];
        let b = tts_body("it", 1, &lines, &["Kore", "Puck"]);
        let text = b["contents"][0]["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains("Italian") && text.contains("slowly") && text.ends_with("Speaker1: Ciao!\nSpeaker2: Ciao a te.\n"));
        let cfg = &b["generationConfig"]["speechConfig"]["multiSpeakerVoiceConfig"]["speakerVoiceConfigs"];
        assert_eq!(cfg[1]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"], "Puck");
        assert_eq!(b["generationConfig"]["responseModalities"][0], "AUDIO");
        // un morceau où une seule voix parle
        let b = tts_body("it", 5, &lines[1..], &["Kore", "Puck"]);
        assert_eq!(b["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"], "Puck");
        assert!(b["contents"][0]["parts"][0]["text"].as_str().unwrap().ends_with("Ciao a te."));
    }

    #[test]
    fn voices_follow_gender_and_differ() {
        let hosts = vec![Host { name: "A".into(), female: true }, Host { name: "B".into(), female: true }];
        for seed in 0..12 {
            let v = voices(&hosts, seed);
            assert_ne!(v[0], v[1]);
            assert!(FEMALE.contains(&v[0]) && FEMALE.contains(&v[1]));
        }
        let v = voices(&[Host { name: "M".into(), female: false }], 7);
        assert!(MALE.contains(&v[0]));
    }

    #[test]
    fn audio_is_decoded() {
        let pcm: Vec<u8> = [100i16, -200, 300].iter().flat_map(|v| v.to_le_bytes()).collect();
        let data = base64::engine::general_purpose::STANDARD.encode(&pcm);
        let v = json!({ "candidates": [{ "content": { "parts": [{ "inlineData": { "mimeType": "audio/L16;codec=pcm;rate=24000", "data": data } }] } }] });
        assert_eq!(audio_of(&v), Some((24000, vec![100, -200, 300])));
        let wav = crate::voice::write_wav(16000, &[1, 2]);
        let v = json!({ "candidates": [{ "content": { "parts": [{ "inlineData": { "mimeType": "audio/wav", "data": base64::engine::general_purpose::STANDARD.encode(&wav) } }] } }] });
        assert_eq!(audio_of(&v), Some((16000, vec![1, 2])));
        assert_eq!(audio_of(&json!({ "candidates": [{ "finishReason": "OTHER" }] })), None);
        assert_eq!(resample(&[0, 100], 2, 4), vec![0, 50, 100, 100]);
    }

    #[test]
    fn turns_follow_pauses() {
        let rate = 1000;
        let burst = |n: usize| (0..n).map(|k| if k % 2 == 0 { 8000i16 } else { -8000 }).collect::<Vec<_>>();
        // parole 1 s, pause 0,4 s, parole 0,5 s, pause 0,2 s, parole 0,3 s, petite respiration de 0,1 s, parole 0,2 s
        let mut s = burst(1000);
        s.extend(vec![0; 400]);
        s.extend(burst(500));
        s.extend(vec![0; 200]);
        s.extend(burst(300));
        s.extend(vec![0; 100]);
        s.extend(burst(200));
        let t = turns(&s, rate, 3).unwrap();
        assert_eq!(t.len(), 3);
        assert!((t[0].1 - 1.0).abs() < 0.03 && (t[1].0 - 1.4).abs() < 0.03, "{t:?}");
        assert!((t[1].1 - 1.9).abs() < 0.03 && (t[2].0 - 2.1).abs() < 0.03, "{t:?}");
        assert!((t[2].1 - 2.7).abs() < 0.001);
        // plus de répliques que de pauses : on renonce
        assert!(turns(&s, rate, 4).is_none());
        let p = share(&[Line { host: 0, text: "abc".into() }, Line { host: 1, text: "abcdefg".into() }], 0.0, 12.0);
        assert_eq!(p, vec![(0.0, 4.0), (4.0, 12.0)]);
    }

    #[test]
    fn errors_say_what_to_do() {
        let wait = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"Quota exceeded","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"7s"}]}}"#;
        assert_eq!(next_step(429, wait, 0), Next::Wait(Duration::from_secs(8)));
        assert_eq!(next_step(429, wait, 3), Next::Skip { quota: true });
        let daily = r#"{"error":{"code":429,"details":[{"violations":[{"quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]},{"retryDelay":"30s"}]}}"#;
        assert_eq!(next_step(429, daily, 0), Next::Skip { quota: true });
        assert_eq!(next_step(404, "{}", 0), Next::Skip { quota: false });
        assert_eq!(next_step(503, "{}", 0), Next::Wait(Duration::from_secs(4)));
        assert_eq!(next_step(503, "{}", 2), Next::Skip { quota: false });
        let bad = r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert!(matches!(next_step(400, bad, 0), Next::Stop(_)));
        assert!(matches!(next_step(403, r#"{"error":{"message":"denied"}}"#, 0), Next::Stop(m) if m.contains("denied")));
        assert_eq!(parse_delay("0.5s"), Some(Duration::from_millis(500)));
    }

    #[test]
    fn models_are_ranked() {
        let list = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.8-flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/gemini-3.10-flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.5-flash-lite", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3-flash-preview", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.1-flash-image", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-pro-preview-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.8-flash-lite-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.8-flash-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.1-flash-tts-preview", "supportedGenerationMethods": ["generateContent"] }
        ]});
        let m = pick_models(&list);
        assert_eq!(m.text, ["gemini-3.10-flash", "gemini-3.8-flash", "gemini-2.5-flash", "gemini-3.5-flash-lite"]);
        assert_eq!(m.tts, ["gemini-3.8-flash-tts", "gemini-3.8-flash-lite-tts", "gemini-3.1-flash-tts-preview", "gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"]);
        let none = pick_models(&json!({}));
        assert_eq!((none.text.len(), none.tts.len()), (1, 1));
    }

    #[test]
    fn length_follows_level() {
        assert_eq!(target_units("it", 1, 10), 850);
        assert_eq!(target_units("it", 5, 10), 1550);
        assert_eq!(target_units("ja", 3, 1), 252);
        assert_eq!(target_units("it", 9, 0), 155);
    }
}

/// Test réel (clé Gemini personnelle ; écrit et enregistre un podcast de deux
/// minutes dans un dossier jetable) :
/// `LUMEN_GEMINI_KEY=… cargo test --lib podcast_live -- --ignored --nocapture`
/// (`LUMEN_PODCAST=de:B1:story` pour une autre langue, un autre niveau, un autre format ;
/// `LUMEN_PODCAST_OUT=dossier` pour garder le son et le texte).
#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    #[ignore]
    async fn podcast_live() {
        let key = std::env::var("LUMEN_GEMINI_KEY").expect("LUMEN_GEMINI_KEY");
        let spec = std::env::var("LUMEN_PODCAST").unwrap_or_else(|_| "it:A2:talk".into());
        let mut it = spec.split(':');
        let lang = it.next().unwrap_or("it").to_string();
        let level = it.next().and_then(|l| ["A1", "A2", "B1", "B2", "C1"].iter().position(|x| *x == l)).unwrap_or(1) as u8 + 1;
        let format = it.next().unwrap_or("talk").to_string();
        let c = client().unwrap();
        let t = Instant::now();
        let m = models(&c, &key).await.unwrap();
        println!("modèles : texte {:?}, voix {:?} ({:.1} s)", m.text, m.tts, t.elapsed().as_secs_f32());
        let r = Request { topic: "a morning at the market".into(), level, minutes: 2, format, details: String::new(), use_words: false };
        let t = Instant::now();
        let s = write(&c, &key, &m.text, &lang, &r, &[]).await.unwrap();
        let (text, _) = layout(&s.lines);
        let words = crate::text::word_count(&text, &lang);
        println!(
            "script « {} » en {:.1} s : {} répliques, {words} mots (visé {}), animateurs {:?}, langue reconnue {:?}",
            s.title,
            t.elapsed().as_secs_f32(),
            s.lines.len(),
            target_units(&lang, level, 2),
            s.hosts,
            crate::langid::guess(&text)
        );
        assert!(words > target_units(&lang, level, 2) / 3);
        let data = std::env::temp_dir().join("lumen-podcast-live");
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(data.join("media")).unwrap();
        let t = Instant::now();
        let rec = record(&c, &key, &m.tts, &data, &lang, level, &s, |p| println!("  voix : {p:.0} %")).await.unwrap();
        println!(
            "son de {:.1} s en {:.1} s ({} mots minutés, {:.0} mots par minute), {}",
            rec.duration,
            t.elapsed().as_secs_f32(),
            rec.timings.len(),
            words as f64 / rec.duration * 60.0,
            rec.path.display()
        );
        assert_eq!(rec.timings.len(), words);
        assert!(rec.timings.windows(2).all(|w| w[1][2] >= w[0][2] - 0.01), "horodatages dans l'ordre");
        if let Ok(out) = std::env::var("LUMEN_PODCAST_OUT") {
            let out = PathBuf::from(out);
            std::fs::copy(&rec.path, out.join(rec.path.file_name().unwrap())).unwrap();
            std::fs::write(out.join("podcast.txt"), &rec.text).unwrap();
        }
        let _ = std::fs::remove_dir_all(&data);
    }
}
