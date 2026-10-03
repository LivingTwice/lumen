//! Import depuis LingQ avec la clé API personnelle de l'utilisateur :
//! mots connus, mots ignorés, LingQ (traductions, notes, contexte) et
//! leçons de tous ses cours (texte, audio, horodatages).
//!
//! L'API de LingQ n'est pas documentée à jour : les champs sont lus avec
//! prudence. Les mots connus et ignorés passent par la v2, le reste par la v3.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::db::{self, ImportedTerm, NewLesson};
use crate::media;
use crate::text;

const API: &str = "https://www.lingq.com/api";
/// Langues que Lumen sait étudier.
const LANGS: &[&str] = crate::text::LANGS;

/// Clé refusée : arrête tout l'import (les autres erreurs ne touchent
/// qu'une leçon).
#[derive(Debug)]
struct BadKey;

impl std::fmt::Display for BadKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LingQ refuse cette clé API. Vérifiez-la sur lingq.com puis collez-la à nouveau.")
    }
}

impl std::error::Error for BadKey {}

enum Fetched {
    Ok(Value),
    /// 404 : la ressource n'existe pas (ou plus)
    Missing,
    /// 403 : contenu réservé (leçon premium, cours privé…)
    Denied,
}

struct Client {
    http: reqwest::Client,
    key: String,
}

impl Client {
    fn new(key: &str) -> Result<Self> {
        let key = key.trim().trim_start_matches("Token ").trim().to_string();
        if key.is_empty() {
            bail!("Collez d'abord votre clé API LingQ.");
        }
        let http = reqwest::Client::builder()
            .user_agent("Lumen (lecteur pour apprendre les langues)")
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self { http, key })
    }

    /// GET sur l'API. Patiente et réessaie si LingQ est surchargé ou si une
    /// leçon est en cours de préparation de son côté.
    async fn get(&self, path: &str) -> Result<Fetched> {
        const OFFLINE: &str = "LingQ est injoignable. Vérifiez votre connexion à Internet.";
        let url = format!("{API}/{path}");
        let mut wait = 2;
        let mut offline = false;
        for attempt in 0..5 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                wait *= 2;
            }
            let resp = match self
                .http
                .get(&url)
                .header("Authorization", format!("Token {}", self.key))
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) if e.is_timeout() || e.is_connect() => {
                    offline = true;
                    continue;
                }
                Err(_) => bail!(OFFLINE),
            };
            offline = false;
            let status = resp.status().as_u16();
            let body = resp.bytes().await.unwrap_or_default();
            match status {
                200..=299 => {
                    return serde_json::from_slice(&body)
                        .map(Fetched::Ok)
                        .map_err(|_| anyhow!("LingQ a renvoyé une réponse illisible."))
                }
                401 => return Err(BadKey.into()),
                403 => return Ok(Fetched::Denied),
                404 => return Ok(Fetched::Missing),
                409 | 423 | 429 | 500..=599 => continue,
                _ if String::from_utf8_lossy(&body).contains("locked") => continue,
                _ => bail!("LingQ a répondu par une erreur ({status})."),
            }
        }
        if offline {
            bail!(OFFLINE);
        }
        bail!("LingQ est surchargé pour le moment. Réessayez dans quelques minutes.")
    }

    /// Parcourt une liste paginée de LingQ (`results`, `next`, `count`).
    /// `on_page` reçoit les éléments et le total annoncé.
    async fn each_page(
        &self,
        path: &str,
        page_size: u32,
        cancel: &AtomicBool,
        mut on_page: impl FnMut(&[Value], Option<u64>) -> Result<()>,
    ) -> Result<()> {
        let sep = if path.contains('?') { '&' } else { '?' };
        for page in 1..=5000u32 {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let v = match self.get(&format!("{path}{sep}page={page}&page_size={page_size}")).await? {
                Fetched::Ok(v) => v,
                _ => break,
            };
            // quelques points d'accès renvoient directement un tableau
            if let Some(all) = v.as_array() {
                on_page(all, Some(all.len() as u64))?;
                break;
            }
            let results = v.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            on_page(results, v.get("count").and_then(Value::as_u64))?;
            if results.is_empty() || v.get("next").map_or(true, Value::is_null) {
                break;
            }
        }
        Ok(())
    }

    /// Nombre total d'éléments d'une liste paginée.
    async fn count(&self, path: &str) -> Result<i64> {
        let sep = if path.contains('?') { '&' } else { '?' };
        Ok(match self.get(&format!("{path}{sep}page=1&page_size=1")).await? {
            Fetched::Ok(v) => v.get("count").and_then(Value::as_i64).unwrap_or(0),
            _ => 0,
        })
    }

    /// Tous les cours de l'utilisateur dans une langue : ceux qu'il a créés
    /// ou importés, et ceux qu'il suit (étagère « Continuer » de LingQ).
    async fn courses(&self, lang: &str) -> Result<Vec<Course>> {
        let never = AtomicBool::new(false);
        let mut out: Vec<Course> = Vec::new();
        let mut add = |items: &[Value]| {
            for it in items {
                let Some(id) = it.get("id").or_else(|| it.get("pk")).and_then(Value::as_i64) else { continue };
                if out.iter().any(|c| c.id == id) {
                    continue;
                }
                let lessons = int_field(it, &["lessonsCount", "lessons_count"]).unwrap_or(-1);
                out.push(Course { id, title: str_field(it, &["title"]).unwrap_or_else(|| "Cours LingQ".into()), lessons });
            }
        };
        self.each_page(&format!("v3/{lang}/collections/my/"), 100, &never, |items, _| {
            add(items);
            Ok(())
        })
        .await?;
        self.each_page(&format!("v3/{lang}/search/?shelf=my_lessons&type=collection&sortBy=recentlyOpened"), 100, &never, |items, _| {
            add(items);
            Ok(())
        })
        .await?;
        // nombre de leçons manquant : compteurs groupés
        let missing: Vec<i64> = out.iter().filter(|c| c.lessons < 0).map(|c| c.id).collect();
        for chunk in missing.chunks(40) {
            let q: Vec<String> = chunk.iter().map(|id| format!("collection={id}")).collect();
            if let Fetched::Ok(v) = self.get(&format!("v3/{lang}/collections/counters/?{}", q.join("&"))).await? {
                for c in out.iter_mut().filter(|c| c.lessons < 0) {
                    if let Some(n) = v.get(c.id.to_string()).and_then(|x| int_field(x, &["lessonsCount", "lessons_count"])) {
                        c.lessons = n;
                    }
                }
            }
        }
        for c in &mut out {
            c.lessons = c.lessons.max(0);
        }
        Ok(out)
    }

    /// Télécharge l'audio d'une leçon dans la médiathèque de Lumen.
    async fn download_audio(&self, url: &str, data_dir: &Path) -> Result<PathBuf> {
        let url = if url.starts_with('/') { format!("https://www.lingq.com{url}") } else { url.to_string() };
        let ext = url
            .split(['?', '#'])
            .next()
            .and_then(|p| p.rsplit_once('.'))
            .map(|(_, e)| e.to_lowercase())
            .filter(|e| ["mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac", "mp4", "webm"].contains(&e.as_str()))
            .unwrap_or_else(|| "mp3".into());
        let dir = media::media_dir(data_dir);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.lingq.{ext}", media::new_stem()));
        // la clé n'est envoyée qu'à LingQ, jamais au serveur qui héberge l'audio
        let mut req = self.http.get(&url);
        if url.starts_with("https://www.lingq.com/") {
            req = req.header("Authorization", format!("Token {}", self.key));
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            bail!("audio indisponible ({})", resp.status());
        }
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = resp.bytes_stream();
        let res: Result<()> = async {
            while let Some(chunk) = stream.next().await {
                file.write_all(&chunk?).await?;
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if let Err(e) = res {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e);
        }
        Ok(path)
    }
}

fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn int_field(v: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)).or_else(|| x.as_str()?.trim().parse().ok()))
    })
}

fn is_http(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

// ---------- analyse du compte ----------

#[derive(Serialize, Clone, Debug)]
pub struct Course {
    pub id: i64,
    pub title: String,
    pub lessons: i64,
}

#[derive(Serialize, Debug)]
pub struct LangSummary {
    pub lang: String,
    pub known_words: i64,
    pub lingqs: i64,
    pub courses: Vec<Course>,
    pub lessons: i64,
}

/// Vérifie la clé et résume ce que contient le compte, langue par langue.
pub async fn scan(key: &str) -> Result<Vec<LangSummary>> {
    let c = Client::new(key)?;
    // mots connus de chaque langue (et première vérification de la clé)
    let mut known: HashMap<String, i64> = HashMap::new();
    if let Fetched::Ok(v) = c.get("v2/languages/").await? {
        let list = v.as_array().or_else(|| v.get("results").and_then(Value::as_array)).cloned().unwrap_or_default();
        for l in &list {
            if let (Some(code), Some(n)) = (str_field(l, &["code"]), int_field(l, &["knownWords", "known_words"])) {
                known.insert(code, n);
            }
        }
    }
    let mut out = Vec::new();
    // les langues du compte quand LingQ les donne (moins d'appels), sinon toutes
    let candidates: Vec<&str> = if known.is_empty() { LANGS.to_vec() } else { LANGS.iter().copied().filter(|l| known.contains_key(*l)).collect() };
    for lang in candidates {
        let known_words = match known.get(lang) {
            Some(n) => *n,
            None => c.count(&format!("v2/{lang}/known-words/")).await?,
        };
        let lingqs = c.count(&format!("v3/{lang}/cards/")).await?;
        let courses = c.courses(lang).await?;
        if known_words == 0 && lingqs == 0 && courses.is_empty() {
            continue;
        }
        let lessons = courses.iter().map(|c| c.lessons).sum();
        out.push(LangSummary { lang: lang.to_string(), known_words, lingqs, courses, lessons });
    }
    Ok(out)
}

// ---------- import ----------

#[derive(Deserialize, Debug)]
pub struct ImportPlan {
    pub langs: Vec<String>,
    pub vocab: bool,
    pub lessons: bool,
    pub audio: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LingqEvent {
    /// étape : « known », « ignored », « cards » ou « lessons »
    Stage { lang: String, stage: String },
    Progress { done: u64, total: u64 },
    Lesson { title: String, course: String },
}

#[derive(Serialize, Default, Debug)]
pub struct Report {
    /// termes ajoutés ou complétés
    pub words: i64,
    pub lessons: i64,
    /// leçons déjà présentes dans Lumen
    pub skipped: i64,
    /// leçons inaccessibles (réservées, vides…)
    pub failed: i64,
    pub audio_failed: i64,
    pub cancelled: bool,
}

/// LingQ : 0, 1, 2 = niveaux 1 à 3 ; 3 = niveau 4 (appris) ou ✓ (connu,
/// `extended_status` 3). Lumen n'a que trois niveaux d'apprentissage : le
/// niveau 4 de LingQ devient « connu ».
fn card_status(status: i64) -> i64 {
    match status {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => db::STATUS_KNOWN,
    }
}

fn card_term(card: &Value) -> Option<ImportedTerm> {
    let term = str_field(card, &["term"])?;
    let mut hints: Vec<String> = Vec::new();
    if let Some(list) = card.get("hints").and_then(Value::as_array) {
        // les indications en français d'abord
        let mut sorted: Vec<&Value> = list.iter().collect();
        sorted.sort_by_key(|h| h.get("locale").and_then(Value::as_str) != Some("fr"));
        for h in sorted {
            if let Some(t) = str_field(h, &["text"]) {
                if !hints.contains(&t) && hints.len() < 3 {
                    hints.push(t);
                }
            }
        }
    }
    let context = str_field(card, &["fragment"])
        .map(|f| f.trim_matches(|c: char| c == '.' || c == '…' || c.is_whitespace()).to_string())
        .unwrap_or_default();
    Some(ImportedTerm {
        term,
        status: card_status(int_field(card, &["status"]).unwrap_or(0)),
        translation: hints.join(" ; "),
        note: str_field(card, &["notes"]).unwrap_or_default(),
        context,
    })
}

/// Mot d'une liste de mots connus ou ignorés (simple chaîne ou objet).
fn word_term(v: &Value, status: i64) -> Option<ImportedTerm> {
    let term = match v {
        Value::String(s) => s.trim().to_string(),
        // format exact non documenté : à défaut des noms attendus, le premier texte qui ressemble à un mot
        _ => str_field(v, &["term", "text", "word"]).or_else(|| {
            v.as_object()?.values().filter_map(Value::as_str).find(|s| !s.is_empty() && s.chars().count() < 60 && !s.contains('/')).map(str::to_string)
        })?,
    };
    Some(ImportedTerm { term, status, translation: String::new(), note: String::new(), context: String::new() })
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// LingQ ne donne qu'un horodatage par phrase : on répartit sa durée sur
/// ses mots, au prorata de leur longueur, pour que la lanterne suive l'audio.
pub(crate) fn spread_words(sentence: &str, lang: &str, base: usize, t0: f64, t1: f64, out: &mut Vec<[f64; 4]>) {
    let words: Vec<text::Token> = text::tokenize(sentence, lang).into_iter().filter(|t| t.w).collect();
    let weight = |t: &text::Token| (t.e - t.s) as f64 + 1.0;
    let total: f64 = words.iter().map(weight).sum();
    if total <= 0.0 {
        return;
    }
    let mut at = t0;
    for w in &words {
        let d = (t1 - t0) * weight(w) / total;
        out.push([(base + w.s) as f64, (base + w.e) as f64, round2(at), round2(at + d)]);
        at += d;
    }
}

fn simplified(s: &str) -> String {
    text::normalize(s).trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Texte de la leçon (paragraphes séparés par une ligne vide) et
/// horodatages par mot au format de Lumen, à partir de `tokenizedText`.
fn lesson_text(lesson: &Value, lang: &str) -> (String, Option<String>) {
    let title = simplified(&str_field(lesson, &["title"]).unwrap_or_default());
    let mut text = String::new();
    let mut len16 = 0usize;
    let mut timings: Vec<[f64; 4]> = Vec::new();
    let paras = lesson.get("tokenizedText").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    for (pi, para) in paras.iter().enumerate() {
        let sentences: Vec<(String, Option<(f64, f64)>)> = para
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|s| {
                let t = s.get("text")?.as_str()?.trim();
                let ts = s.get("timestamp").and_then(Value::as_array).and_then(|a| {
                    let (t0, t1) = (a.first()?.as_f64()?, a.get(1)?.as_f64()?);
                    (t1 > t0).then_some((t0, t1))
                });
                (!t.is_empty()).then(|| (t.to_string(), ts))
            })
            .collect();
        if sentences.is_empty() {
            continue;
        }
        // le premier paragraphe répète le titre de la leçon
        if pi == 0 && !title.is_empty() {
            let joined: Vec<&str> = sentences.iter().map(|(s, _)| s.as_str()).collect();
            if simplified(&joined.join(" ")) == title {
                continue;
            }
        }
        if !text.is_empty() {
            text.push_str("\n\n");
            len16 += 2;
        }
        for (si, (s, ts)) in sentences.iter().enumerate() {
            if si > 0 {
                text.push(' ');
                len16 += 1;
            }
            let base = len16;
            text.push_str(s);
            len16 += s.encode_utf16().count();
            if let Some((t0, t1)) = ts {
                spread_words(s, lang, base, *t0, *t1, &mut timings);
            }
        }
    }
    let timings = (!timings.is_empty()).then(|| serde_json::to_string(&timings).unwrap_or_else(|_| "[]".into()));
    (text, timings)
}

/// Repli quand `tokenizedText` est absent : la liste des phrases.
async fn sentences_text(c: &Client, lang: &str, id: i64) -> Result<String> {
    let Fetched::Ok(v) = c.get(&format!("v3/{lang}/lessons/{id}/sentences/")).await? else { return Ok(String::new()) };
    let list = v.as_array().or_else(|| v.get("results").and_then(Value::as_array)).map(Vec::as_slice).unwrap_or(&[]);
    let parts: Vec<String> = list.iter().filter_map(|s| str_field(s, &["text"])).collect();
    Ok(parts.join(" "))
}

/// Importe tout ce qui est demandé. S'arrête proprement si `cancel` passe
/// à vrai (le rapport indique alors ce qui a déjà été importé).
pub async fn import(
    db: &Mutex<Connection>,
    data_dir: &Path,
    key: &str,
    plan: &ImportPlan,
    cancel: &AtomicBool,
    on_event: impl Fn(LingqEvent),
) -> Result<Report> {
    let c = Client::new(key)?;
    let mut report = Report::default();
    let stage = |lang: &str, s: &str| on_event(LingqEvent::Stage { lang: lang.to_string(), stage: s.to_string() });

    for lang in plan.langs.iter().filter(|l| LANGS.contains(&l.as_str())) {
        if plan.vocab {
            // mots connus, puis ignorés, puis LingQ (qui apportent traductions et notes)
            let lists: [(&str, String, u32); 3] = [
                ("known", format!("v2/{lang}/known-words/"), 1000),
                ("ignored", format!("v2/{lang}/ignored-words/"), 1000),
                ("cards", format!("v3/{lang}/cards/"), 500),
            ];
            for (name, path, size) in lists {
                stage(lang, name);
                let mut done = 0u64;
                c.each_page(&path, size, cancel, |items, count| {
                    let terms: Vec<ImportedTerm> = items
                        .iter()
                        .filter_map(|v| match name {
                            "known" => word_term(v, db::STATUS_KNOWN),
                            "ignored" => word_term(v, db::STATUS_IGNORED),
                            _ => card_term(v),
                        })
                        .collect();
                    report.words += db::terms_import(&mut db.lock(), lang, &terms)?;
                    done += items.len() as u64;
                    on_event(LingqEvent::Progress { done, total: count.unwrap_or(done).max(done) });
                    Ok(())
                })
                .await?;
            }
        }

        if plan.lessons && !cancel.load(Ordering::Relaxed) {
            stage(lang, "lessons");
            // liste complète d'abord, pour annoncer le total
            let mut todo: Vec<(i64, String)> = Vec::new();
            for course in c.courses(lang).await? {
                c.each_page(&format!("v3/{lang}/collections/{}/lessons/", course.id), 200, cancel, |items, _| {
                    for it in items {
                        if let Some(id) = it.get("id").and_then(Value::as_i64) {
                            if !todo.iter().any(|(x, _)| *x == id) {
                                todo.push((id, course.title.clone()));
                            }
                        }
                    }
                    Ok(())
                })
                .await?;
            }
            let total = todo.len() as u64;
            for (i, (id, course)) in todo.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                on_event(LingqEvent::Progress { done: i as u64, total });
                let ext = format!("lingq:{id}");
                let exists = db::lesson_by_ext(&db.lock(), &ext)?.is_some();
                if exists {
                    report.skipped += 1;
                    continue;
                }
                match import_lesson(&c, db, data_dir, lang, *id, course, &ext, plan.audio).await {
                    Ok((title, audio_ok)) => {
                        report.lessons += 1;
                        if !audio_ok {
                            report.audio_failed += 1;
                        }
                        on_event(LingqEvent::Lesson { title, course: course.clone() });
                    }
                    Err(e) if e.is::<BadKey>() => return Err(e),
                    Err(_) => report.failed += 1,
                }
                // LingQ limite le rythme des requêtes
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            on_event(LingqEvent::Progress { done: total, total });
        }
    }
    report.cancelled = cancel.load(Ordering::Relaxed);
    Ok(report)
}

/// Importe une leçon. Renvoie son titre, et faux si son audio n'a pas pu
/// être téléchargé.
async fn import_lesson(
    c: &Client,
    db: &Mutex<Connection>,
    data_dir: &Path,
    lang: &str,
    id: i64,
    course: &str,
    ext: &str,
    with_audio: bool,
) -> Result<(String, bool)> {
    let lesson = match c.get(&format!("v3/{lang}/lessons/{id}/")).await? {
        Fetched::Ok(v) => v,
        Fetched::Missing => bail!("leçon introuvable"),
        Fetched::Denied => bail!("leçon réservée"),
    };
    let title = str_field(&lesson, &["title"]).unwrap_or_else(|| "Leçon LingQ".into());
    let (mut text, mut timings) = lesson_text(&lesson, lang);
    if text.trim().is_empty() {
        text = sentences_text(c, lang, id).await?;
        timings = None;
    }
    if text.trim().is_empty() {
        bail!("leçon vide");
    }

    let audio_url = str_field(&lesson, &["audioUrl", "audio"]);
    let mut audio_ok = true;
    let mut media_path: Option<String> = None;
    if with_audio {
        if let Some(url) = audio_url.as_deref() {
            match c.download_audio(url, data_dir).await {
                Ok(p) => media_path = Some(p.display().to_string()),
                Err(_) => audio_ok = false,
            }
        }
    }
    if media_path.is_none() {
        timings = None;
    }

    // une vidéo YouTube d'origine reste téléchargeable depuis le lecteur
    let video = str_field(&lesson, &["videoUrl"]).filter(|u| is_http(u));
    let original = str_field(&lesson, &["originalUrl"]).filter(|u| is_http(u));
    let is_youtube = |u: &String| u.contains("youtube.com") || u.contains("youtu.be");
    let video = video.or_else(|| original.clone().filter(is_youtube));
    let kind = match (&media_path, &video) {
        (Some(_), Some(_)) => "video",
        (Some(_), None) => "audio",
        _ => "text",
    };
    let source = video.or(original).unwrap_or_default();
    let completed = lesson.get("completed").and_then(Value::as_bool).unwrap_or(false);

    let new = NewLesson {
        lang: lang.to_string(),
        title: title.clone(),
        collection: str_field(&lesson, &["collectionTitle"]).unwrap_or_else(|| course.to_string()),
        kind: kind.to_string(),
        source,
        text,
        media_path: media_path.clone(),
        timings,
        video_path: None,
    };
    let saved = {
        let conn = db.lock();
        db::lesson_create(&conn, &new).and_then(|lid| db::lesson_set_ext(&conn, lid, ext, completed))
    };
    if let Err(e) = saved {
        if let Some(p) = media_path {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    Ok((title, audio_ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_mapping() {
        let card = json!({
            "term": "Abetting", "status": 1, "extended_status": null, "notes": "droit",
            "fragment": "...the party's abetting the hijacking...",
            "hints": [{"locale": "ru", "text": "соучастие"}, {"locale": "fr", "text": "encourageant"}]
        });
        let t = card_term(&card).unwrap();
        assert_eq!(t.status, 2);
        assert_eq!(t.translation, "encourageant ; соучастие");
        assert_eq!(t.context, "the party's abetting the hijacking");
        assert_eq!(t.note, "droit");
        assert_eq!(card_term(&json!({"term": "x", "status": 3, "extended_status": 3})).unwrap().status, 4);
        assert_eq!(word_term(&json!("Hola"), 4).unwrap().term, "Hola");
        assert_eq!(word_term(&json!({"term": "casa"}), 5).unwrap().status, 5);
    }

    #[test]
    fn lesson_text_and_timings() {
        let lesson = json!({
            "title": "Le phare",
            "tokenizedText": [
                [{"text": "Le phare.", "timestamp": [0.0, 1.0]}],
                [{"text": "The cat sat.", "timestamp": [1.0, 2.5]}, {"text": "It slept.", "timestamp": [2.5, 3.5]}],
                [{"text": "Bye!", "timestamp": [0.0, null]}]
            ]
        });
        let (text, timings) = lesson_text(&lesson, "en");
        assert_eq!(text, "The cat sat. It slept.\n\nBye!");
        let t: Vec<[f64; 4]> = serde_json::from_str(&timings.unwrap()).unwrap();
        // « cat » : positions 4 à 7, dans la première phrase
        assert_eq!((t[1][0], t[1][1]), (4.0, 7.0));
        assert!(t[1][2] > 1.0 && t[1][3] < 2.5);
        // « It » commence la deuxième phrase à 2,5 s
        assert_eq!((t[3][0], t[3][2]), (13.0, 2.5));
        // « Bye » n'a pas d'horodatage complet
        assert_eq!(t.len(), 5);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Essai réel sur un compte (ignoré par défaut), sans toucher à la
    /// base de Lumen : LUMEN_LINGQ_KEY=… cargo test --lib lingq_live -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn lingq_live() {
        let Ok(key) = std::env::var("LUMEN_LINGQ_KEY") else { return };
        let account = scan(&key).await.unwrap();
        for a in &account {
            println!("[{}] {} mots connus, {} LingQ, {} cours, {} leçons", a.lang, a.known_words, a.lingqs, a.courses.len(), a.lessons);
        }
        let c = Client::new(&key).unwrap();
        let Some(a) = account.first() else { return };
        for path in [format!("v2/{}/known-words/", a.lang), format!("v2/{}/ignored-words/", a.lang), format!("v3/{}/cards/", a.lang)] {
            if let Fetched::Ok(v) = c.get(&format!("{path}?page=1&page_size=2")).await.unwrap() {
                let s = v.to_string();
                println!("{path} -> {}", s.chars().take(600).collect::<String>());
            }
        }
        // première leçon d'un cours, importée dans une base jetable
        let Some(course) = account.iter().flat_map(|a| a.courses.iter().map(move |c| (a.lang.clone(), c))).next() else { return };
        let never = AtomicBool::new(false);
        let mut first = None;
        c.each_page(&format!("v3/{}/collections/{}/lessons/", course.0, course.1.id), 5, &never, |items, _| {
            first = first.or_else(|| items.first().and_then(|v| v.get("id")).and_then(Value::as_i64));
            Ok(())
        })
        .await
        .unwrap();
        let Some(id) = first else { return };
        let dir = std::env::temp_dir().join(format!("lumen-lingq-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Mutex::new(db::open(&dir.join("lumen.db")).unwrap());
        let (title, audio_ok) = import_lesson(&c, &db, &dir, &course.0, id, &course.1.title, "lingq:test", true).await.unwrap();
        let lesson = db::lesson_get(&db.lock(), db::lesson_by_ext(&db.lock(), "lingq:test").unwrap().unwrap()).unwrap();
        println!(
            "leçon « {title} » ({}) : {} mots, audio {}, horodatages {}\n{}",
            lesson.kind,
            lesson.word_count,
            if audio_ok && lesson.media_path.is_some() { "oui" } else { "non" },
            lesson.timings.as_deref().map_or(0, |t| t.matches('[').count().saturating_sub(1)),
            lesson.text.chars().take(400).collect::<String>()
        );
        assert!(lesson.word_count > 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
