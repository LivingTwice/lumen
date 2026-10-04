//! Base de données locale (SQLite) : leçons, mots, activité, réglages, cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{Datelike, Months, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::text;

pub const STATUS_KNOWN: i64 = 4;
pub const STATUS_IGNORED: i64 = 5;
/// Minutage des mots au nouveau calage précis (0 : ancien Whisper ou LingQ, approximatif).
pub const TIMING_PRECISE: i64 = 2;

/// Fichier de la base dans le dossier de données.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("lumen.db")
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS lessons(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lang TEXT NOT NULL,
            title TEXT NOT NULL,
            collection TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL DEFAULT 'text',
            source TEXT NOT NULL DEFAULT '',
            text TEXT NOT NULL,
            media_path TEXT,
            timings TEXT,
            hue INTEGER NOT NULL DEFAULT 210,
            word_count INTEGER NOT NULL DEFAULT 0,
            page INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            opened_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS lessons_lang ON lessons(lang, opened_at);
        CREATE TABLE IF NOT EXISTS terms(
            lang TEXT NOT NULL,
            term TEXT NOT NULL,
            status INTEGER NOT NULL,
            translation TEXT NOT NULL DEFAULT '',
            note TEXT NOT NULL DEFAULT '',
            lemma TEXT NOT NULL DEFAULT '',
            context TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(lang, term)
        );
        CREATE INDEX IF NOT EXISTS terms_status ON terms(lang, status);
        CREATE TABLE IF NOT EXISTS activity(
            day TEXT NOT NULL,
            lang TEXT NOT NULL,
            words_read INTEGER NOT NULL DEFAULT 0,
            known_added INTEGER NOT NULL DEFAULT 0,
            lingqs INTEGER NOT NULL DEFAULT 0,
            listen_secs INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(day, lang)
        );
        CREATE TABLE IF NOT EXISTS tcache(k TEXT PRIMARY KEY, v TEXT NOT NULL, created_at INTEGER NOT NULL);
        "#,
    )?;
    // migrations (uniquement des ajouts : les bases existantes restent lisibles)
    add_column(&conn, "lessons", "video_path", "TEXT")?;
    // identifiant d'origine des leçons importées (ex. « lingq:123 ») pour ne pas les dupliquer
    add_column(&conn, "lessons", "ext_id", "TEXT")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS lessons_ext ON lessons(ext_id);")?;
    // reprise exacte : seconde atteinte dans l'audio, mot atteint dans le texte
    add_column(&conn, "lessons", "position", "REAL NOT NULL DEFAULT 0")?;
    add_column(&conn, "lessons", "anchor", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(&conn, "lessons", "duration", "REAL NOT NULL DEFAULT 0")?;
    // couverture choisie par l'utilisateur (fichier dans media/)
    add_column(&conn, "lessons", "cover_path", "TEXT")?;
    // qualité du minutage des mots (voir TIMING_PRECISE)
    add_column(&conn, "lessons", "timing_v", "INTEGER NOT NULL DEFAULT 0")?;
    // playlists : leçons d'une langue dans l'ordre choisi (une leçon supprimée en sort d'elle-même)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS playlists(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lang TEXT NOT NULL,
            name TEXT NOT NULL,
            current_id INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playlist_items(
            playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            lesson_id INTEGER NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            PRIMARY KEY(playlist_id, lesson_id)
        );
        CREATE INDEX IF NOT EXISTS playlist_items_lesson ON playlist_items(lesson_id);
        "#,
    )?;
    // chat : conversations (une leçon peut y être jointe) et leurs messages
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS chats(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lang TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            lesson_id INTEGER REFERENCES lessons(id) ON DELETE SET NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS chats_lang ON chats(lang, updated_at);
        CREATE TABLE IF NOT EXISTS chat_messages(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            thought TEXT NOT NULL DEFAULT '',
            thought_secs REAL NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS chat_messages_chat ON chat_messages(chat_id, id);
        "#,
    )?;
    // progrès : temps actif passé dans les leçons, et jours où l'objectif est atteint (série)
    add_column(&conn, "activity", "learn_secs", "INTEGER NOT NULL DEFAULT 0")?;
    if add_column(&conn, "activity", "goal_met", "INTEGER NOT NULL DEFAULT 0")? {
        // avant le temps actif, un jour de lecture ou d'écoute compte dans la série :
        // personne ne perd la série qu'il avait déjà
        conn.execute_batch("UPDATE activity SET goal_met=1 WHERE words_read>0 OR listen_secs>=60;")?;
    }
    Ok(conn)
}

/// Ajoute une colonne si elle manque. Renvoie `true` si elle vient d'être créée.
fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<bool> {
    let exists: bool = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name=?1"))?
        .exists([column])?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))?;
    }
    Ok(!exists)
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn today_date() -> NaiveDate {
    chrono::Local::now().date_naive()
}

// ---------- réglages ----------

pub fn settings_all(c: &Connection) -> Result<HashMap<String, String>> {
    let mut st = c.prepare("SELECT key, value FROM settings")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn setting(c: &Connection, key: &str) -> Option<String> {
    c.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional().ok().flatten()
}

pub fn setting_set(c: &Connection, key: &str, value: &str) -> Result<()> {
    c.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------- leçons ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NewLesson {
    pub lang: String,
    pub title: String,
    #[serde(default)]
    pub collection: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub source: String,
    pub text: String,
    #[serde(default)]
    pub media_path: Option<String>,
    #[serde(default)]
    pub timings: Option<String>,
    #[serde(default)]
    pub video_path: Option<String>,
}
fn default_kind() -> String {
    "text".into()
}

#[derive(Serialize, Clone, Debug)]
pub struct LessonSummary {
    pub id: i64,
    pub lang: String,
    pub title: String,
    pub collection: String,
    pub kind: String,
    pub source: String,
    pub hue: i64,
    pub word_count: i64,
    pub page: i64,
    pub completed: bool,
    pub has_media: bool,
    pub created_at: i64,
    pub opened_at: Option<i64>,
    /// mots uniques jamais vus (ni appris ni connus)
    pub new_words: i64,
    /// part (0-100) des occurrences reconnues (connues ou en apprentissage)
    pub known_pct: i64,
    pub excerpt: String,
    /// seconde atteinte dans l'audio ou la vidéo, et durée totale
    pub position: f64,
    pub duration: f64,
    pub cover_path: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Lesson {
    pub id: i64,
    pub lang: String,
    pub title: String,
    pub collection: String,
    pub kind: String,
    pub source: String,
    pub text: String,
    pub media_path: Option<String>,
    pub timings: Option<String>,
    pub video_path: Option<String>,
    pub hue: i64,
    pub word_count: i64,
    pub page: i64,
    pub completed: bool,
    pub position: f64,
    /// jeton (mot) où la lecture s'était arrêtée
    pub anchor: i64,
    pub duration: f64,
    pub cover_path: Option<String>,
    pub timing_v: i64,
}

fn hue_for(title: &str) -> i64 {
    let mut h: u32 = 2166136261;
    for b in title.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    (h % 360) as i64
}

pub fn lesson_create(c: &Connection, l: &NewLesson) -> Result<i64> {
    let wc = text::word_count(&l.text, &l.lang) as i64;
    c.execute(
        "INSERT INTO lessons(lang,title,collection,kind,source,text,media_path,timings,hue,word_count,created_at,video_path)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![l.lang, l.title, l.collection, l.kind, l.source, l.text, l.media_path, l.timings, hue_for(&l.title), wc, now(), l.video_path],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn status_map(c: &Connection, lang: &str) -> Result<HashMap<String, i64>> {
    let mut st = c.prepare_cached("SELECT term, status FROM terms WHERE lang=?1")?;
    let rows = st.query_map([lang], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn lessons_list(c: &Connection, lang: &str) -> Result<Vec<LessonSummary>> {
    let statuses = status_map(c, lang)?;
    let mut st = c.prepare(
        "SELECT id,lang,title,collection,kind,source,hue,word_count,page,completed,media_path,created_at,opened_at,text,position,duration,cover_path
         FROM lessons WHERE lang=?1 ORDER BY COALESCE(opened_at, created_at) DESC",
    )?;
    let rows = st.query_map([lang], |r| {
        let text: String = r.get(13)?;
        Ok((
            LessonSummary {
                id: r.get(0)?,
                lang: r.get(1)?,
                title: r.get(2)?,
                collection: r.get(3)?,
                kind: r.get(4)?,
                source: r.get(5)?,
                hue: r.get(6)?,
                word_count: r.get(7)?,
                page: r.get(8)?,
                completed: r.get::<_, i64>(9)? != 0,
                has_media: r.get::<_, Option<String>>(10)?.is_some(),
                created_at: r.get(11)?,
                opened_at: r.get(12)?,
                new_words: 0,
                known_pct: 0,
                excerpt: String::new(),
                position: r.get(14)?,
                duration: r.get(15)?,
                cover_path: r.get(16)?,
            },
            text,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (mut s, text) = row?;
        let keys = text::word_keys(&text, &s.lang);
        let mut uniq = std::collections::HashSet::new();
        let mut recognized = 0usize;
        for k in &keys {
            match statuses.get(k) {
                Some(_) => recognized += 1,
                None => {
                    uniq.insert(k.as_str());
                }
            }
        }
        s.new_words = uniq.len() as i64;
        s.known_pct = if keys.is_empty() { 100 } else { (recognized * 100 / keys.len()) as i64 };
        s.excerpt = text.chars().take(220).collect::<String>().replace('\n', " ");
        out.push(s);
    }
    Ok(out)
}

pub fn lesson_get(c: &Connection, id: i64) -> Result<Lesson> {
    let l = c.query_row(
        "SELECT id,lang,title,collection,kind,source,text,media_path,timings,hue,word_count,page,completed,video_path,position,anchor,duration,cover_path,timing_v
         FROM lessons WHERE id=?1",
        [id],
        |r| {
            Ok(Lesson {
                id: r.get(0)?,
                lang: r.get(1)?,
                title: r.get(2)?,
                collection: r.get(3)?,
                kind: r.get(4)?,
                source: r.get(5)?,
                text: r.get(6)?,
                media_path: r.get(7)?,
                timings: r.get(8)?,
                hue: r.get(9)?,
                word_count: r.get(10)?,
                page: r.get(11)?,
                completed: r.get::<_, i64>(12)? != 0,
                video_path: r.get(13)?,
                position: r.get(14)?,
                anchor: r.get(15)?,
                duration: r.get(16)?,
                cover_path: r.get(17)?,
                timing_v: r.get(18)?,
            })
        },
    )?;
    c.execute("UPDATE lessons SET opened_at=?1 WHERE id=?2", params![now(), id])?;
    Ok(l)
}

#[derive(Deserialize, Debug, Default)]
pub struct LessonPatch {
    pub title: Option<String>,
    pub collection: Option<String>,
    pub page: Option<i64>,
    pub completed: Option<bool>,
    pub position: Option<f64>,
    pub anchor: Option<i64>,
    pub duration: Option<f64>,
}

pub fn lesson_update(c: &Connection, id: i64, p: &LessonPatch) -> Result<()> {
    if let Some(t) = &p.title {
        c.execute("UPDATE lessons SET title=?1 WHERE id=?2", params![t, id])?;
    }
    if let Some(t) = &p.collection {
        c.execute("UPDATE lessons SET collection=?1 WHERE id=?2", params![t, id])?;
    }
    if let Some(v) = p.page {
        c.execute("UPDATE lessons SET page=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = p.completed {
        c.execute("UPDATE lessons SET completed=?1 WHERE id=?2", params![v as i64, id])?;
    }
    if let Some(v) = p.position {
        c.execute("UPDATE lessons SET position=?1 WHERE id=?2", params![v.max(0.0), id])?;
    }
    if let Some(v) = p.anchor {
        c.execute("UPDATE lessons SET anchor=?1 WHERE id=?2", params![v.max(0), id])?;
    }
    if let Some(v) = p.duration {
        c.execute("UPDATE lessons SET duration=?1 WHERE id=?2", params![v.max(0.0), id])?;
    }
    Ok(())
}

pub fn lesson_delete(c: &Connection, id: i64) -> Result<Vec<String>> {
    let files: Option<(Option<String>, Option<String>, Option<String>)> = c
        .query_row("SELECT media_path, video_path, cover_path FROM lessons WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    c.execute("DELETE FROM lessons WHERE id=?1", [id])?;
    let mut out = Vec::new();
    if let Some((a, v, cover)) = files {
        out.extend(a);
        out.extend(v);
        out.extend(cover);
    }
    out.dedup();
    Ok(out)
}

pub fn lesson_set_video(c: &Connection, id: i64, path: &str) -> Result<()> {
    c.execute("UPDATE lessons SET video_path=?1 WHERE id=?2", params![path, id])?;
    Ok(())
}

pub fn lesson_source(c: &Connection, id: i64) -> Result<(String, Option<String>)> {
    Ok(c.query_row("SELECT source, video_path FROM lessons WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// Langue, texte et audio d'une leçon (pour recaler la lanterne).
pub fn lesson_media(c: &Connection, id: i64) -> Result<(String, String, Option<String>)> {
    Ok(c.query_row("SELECT lang, text, media_path FROM lessons WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?)
}

/// Audio créé par la voix naturelle : la leçon devient une leçon audio,
/// écoutée depuis le début. Renvoie l'ancien audio créé, à supprimer du disque.
pub fn lesson_set_voice(c: &Connection, id: i64, media: &str, timings: &str, version: i64, duration: f64) -> Result<Option<String>> {
    let old: Option<String> = c.query_row("SELECT media_path FROM lessons WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten();
    c.execute(
        "UPDATE lessons SET media_path=?1, timings=?2, timing_v=?3, duration=?4, position=0 WHERE id=?5",
        params![media, timings, version, duration, id],
    )?;
    Ok(old.filter(|p| p != media && p.contains(".voice.")))
}

pub fn lesson_set_timings(c: &Connection, id: i64, timings: &str, version: i64) -> Result<()> {
    c.execute("UPDATE lessons SET timings=?1, timing_v=?2 WHERE id=?3", params![timings, version, id])?;
    Ok(())
}

/// Change (ou retire) la couverture. Renvoie l'ancienne, à supprimer du disque.
pub fn lesson_set_cover(c: &Connection, id: i64, path: Option<&str>) -> Result<Option<String>> {
    let old: Option<String> = c.query_row("SELECT cover_path FROM lessons WHERE id=?1", [id], |r| r.get(0))?;
    c.execute("UPDATE lessons SET cover_path=?1 WHERE id=?2", params![path, id])?;
    Ok(old)
}

/// (identifiant, source) des leçons d'une langue : Découvrir y reconnaît ce qui est déjà importé.
pub fn lesson_sources(c: &Connection, lang: &str) -> Result<Vec<(i64, String)>> {
    let mut st = c.prepare("SELECT id, source FROM lessons WHERE lang=?1")?;
    let rows = st.query_map([lang], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Leçon déjà importée depuis cette origine ?
pub fn lesson_by_ext(c: &Connection, ext_id: &str) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT id FROM lessons WHERE ext_id=?1", [ext_id], |r| r.get(0)).optional()?)
}

pub fn lesson_set_ext(c: &Connection, id: i64, ext_id: &str, completed: bool) -> Result<()> {
    c.execute("UPDATE lessons SET ext_id=?1, completed=?2 WHERE id=?3", params![ext_id, completed as i64, id])?;
    Ok(())
}

// ---------- playlists ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub lang: String,
    pub name: String,
    /// leçons, dans l'ordre de lecture
    pub lessons: Vec<i64>,
    /// leçon où la lecture de la playlist en est (aucune : elle repart du début)
    pub current: Option<i64>,
    pub created_at: i64,
}

#[derive(Deserialize, Debug, Default)]
pub struct PlaylistPatch {
    pub name: Option<String>,
    pub lessons: Option<Vec<i64>>,
    /// 0 : la playlist repart du début
    pub current: Option<i64>,
}

/// Nom propre, jamais vide, de longueur raisonnable.
fn playlist_name(name: &str) -> String {
    let n: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
    if n.is_empty() { crate::i18n::t("Nouvelle playlist", "New playlist").into() } else { n }
}

pub fn playlists_list(c: &Connection, lang: &str) -> Result<Vec<Playlist>> {
    let mut st = c.prepare("SELECT id,lang,name,current_id,created_at FROM playlists WHERE lang=?1 ORDER BY created_at DESC, id DESC")?;
    let mut out = st
        .query_map([lang], |r| {
            Ok(Playlist { id: r.get(0)?, lang: r.get(1)?, name: r.get(2)?, lessons: Vec::new(), current: r.get(3)?, created_at: r.get(4)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = c.prepare_cached("SELECT lesson_id FROM playlist_items WHERE playlist_id=?1 ORDER BY position")?;
    for p in &mut out {
        p.lessons = items.query_map([p.id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        // la leçon en cours a pu quitter la playlist entre-temps
        p.current = p.current.filter(|id| p.lessons.contains(id));
    }
    Ok(out)
}

/// Remplace les leçons d'une playlist (ajout, retrait et nouvel ordre d'un coup).
/// Les doublons et les leçons d'une autre langue sont écartés.
fn playlist_fill(tx: &rusqlite::Transaction, id: i64, lessons: &[i64]) -> Result<()> {
    tx.execute("DELETE FROM playlist_items WHERE playlist_id=?1", [id])?;
    let mut ins = tx.prepare_cached(
        "INSERT INTO playlist_items(playlist_id,lesson_id,position)
         SELECT ?1, id, ?3 FROM lessons WHERE id=?2 AND lang=(SELECT lang FROM playlists WHERE id=?1)",
    )?;
    let mut seen = std::collections::HashSet::new();
    let mut pos = 0i64;
    for &l in lessons {
        if seen.insert(l) && ins.execute(params![id, l, pos])? > 0 {
            pos += 1;
        }
    }
    Ok(())
}

pub fn playlist_create(c: &mut Connection, lang: &str, name: &str, lessons: &[i64]) -> Result<i64> {
    let tx = c.transaction()?;
    tx.execute("INSERT INTO playlists(lang,name,created_at) VALUES(?1,?2,?3)", params![lang, playlist_name(name), now()])?;
    let id = tx.last_insert_rowid();
    playlist_fill(&tx, id, lessons)?;
    tx.commit()?;
    Ok(id)
}

pub fn playlist_update(c: &mut Connection, id: i64, p: &PlaylistPatch) -> Result<()> {
    let tx = c.transaction()?;
    if let Some(n) = &p.name {
        tx.execute("UPDATE playlists SET name=?1 WHERE id=?2", params![playlist_name(n), id])?;
    }
    if let Some(l) = &p.lessons {
        playlist_fill(&tx, id, l)?;
    }
    if let Some(cur) = p.current {
        tx.execute("UPDATE playlists SET current_id=?1 WHERE id=?2", params![(cur > 0).then_some(cur), id])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn playlist_delete(c: &Connection, id: i64) -> Result<()> {
    c.execute("DELETE FROM playlists WHERE id=?1", [id])?;
    Ok(())
}

// ---------- chat ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChatSummary {
    pub id: i64,
    pub lang: String,
    /// vide tant que la première question n'est pas posée
    pub title: String,
    pub lesson_id: Option<i64>,
    pub lesson_title: Option<String>,
    pub updated_at: i64,
    pub count: i64,
    /// début du dernier message
    pub preview: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub id: i64,
    /// "user" ou "assistant"
    pub role: String,
    pub content: String,
    /// réflexion du modèle avant sa réponse (vide sans réflexion)
    pub thought: String,
    pub thought_secs: f64,
    pub created_at: i64,
}

#[derive(Deserialize, Debug, Default)]
pub struct ChatPatch {
    pub title: Option<String>,
    /// leçon jointe ; 0 la retire
    pub lesson: Option<i64>,
}

const CHAT_SUMMARY: &str = "SELECT c.id, c.lang, c.title, c.lesson_id, l.title, c.updated_at,
        (SELECT COUNT(*) FROM chat_messages m WHERE m.chat_id=c.id),
        COALESCE((SELECT substr(m.content,1,160) FROM chat_messages m WHERE m.chat_id=c.id ORDER BY m.id DESC LIMIT 1),'')
     FROM chats c LEFT JOIN lessons l ON l.id=c.lesson_id";

fn row_chat(r: &rusqlite::Row) -> rusqlite::Result<ChatSummary> {
    let preview: String = r.get(7)?;
    Ok(ChatSummary {
        id: r.get(0)?,
        lang: r.get(1)?,
        title: r.get(2)?,
        lesson_id: r.get(3)?,
        lesson_title: r.get(4)?,
        updated_at: r.get(5)?,
        count: r.get(6)?,
        preview: preview.split_whitespace().collect::<Vec<_>>().join(" ").replace("**", ""),
    })
}

pub fn chats_list(c: &Connection, lang: &str) -> Result<Vec<ChatSummary>> {
    let mut st = c.prepare(&format!("{CHAT_SUMMARY} WHERE c.lang=?1 ORDER BY c.updated_at DESC, c.id DESC"))?;
    let out = st.query_map([lang], row_chat)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

pub fn chat_get(c: &Connection, id: i64) -> Result<ChatSummary> {
    Ok(c.query_row(&format!("{CHAT_SUMMARY} WHERE c.id=?1"), [id], row_chat)?)
}

pub fn chat_messages(c: &Connection, id: i64) -> Result<Vec<ChatMessage>> {
    let mut st = c.prepare("SELECT id,role,content,thought,thought_secs,created_at FROM chat_messages WHERE chat_id=?1 ORDER BY id")?;
    let out = st
        .query_map([id], |r| {
            Ok(ChatMessage { id: r.get(0)?, role: r.get(1)?, content: r.get(2)?, thought: r.get(3)?, thought_secs: r.get(4)?, created_at: r.get(5)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

/// Nouvelle conversation (une leçon d'une autre langue n'y est pas jointe).
pub fn chat_create(c: &Connection, lang: &str, lesson: Option<i64>) -> Result<i64> {
    let lesson = match lesson {
        Some(id) => c.query_row("SELECT id FROM lessons WHERE id=?1 AND lang=?2", params![id, lang], |r| r.get::<_, i64>(0)).optional()?,
        None => None,
    };
    let t = now();
    c.execute("INSERT INTO chats(lang,title,lesson_id,created_at,updated_at) VALUES(?1,'',?2,?3,?3)", params![lang, lesson, t])?;
    Ok(c.last_insert_rowid())
}

pub fn chat_update(c: &Connection, id: i64, p: &ChatPatch) -> Result<()> {
    if let Some(t) = &p.title {
        let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
        c.execute("UPDATE chats SET title=?1 WHERE id=?2", params![t, id])?;
    }
    if let Some(l) = p.lesson {
        c.execute(
            "UPDATE chats SET lesson_id=(SELECT id FROM lessons WHERE id=?1 AND lang=chats.lang) WHERE id=?2",
            params![l, id],
        )?;
    }
    Ok(())
}

pub fn chat_delete(c: &Connection, id: i64) -> Result<()> {
    c.execute("DELETE FROM chats WHERE id=?1", [id])?;
    Ok(())
}

/// Enregistre une question et sa réponse d'un seul coup ; la première question
/// donne son titre à la conversation.
pub fn chat_append(c: &mut Connection, id: i64, question: &str, answer: &str, thought: &str, thought_secs: f64, title: &str) -> Result<(ChatMessage, ChatMessage)> {
    let tx = c.transaction()?;
    let t = now();
    let push = |role: &str, content: &str, thought: &str, secs: f64| -> Result<ChatMessage> {
        tx.execute(
            "INSERT INTO chat_messages(chat_id,role,content,thought,thought_secs,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, role, content, thought, secs, t],
        )?;
        Ok(ChatMessage { id: tx.last_insert_rowid(), role: role.into(), content: content.into(), thought: thought.into(), thought_secs: secs, created_at: t })
    };
    let q = push("user", question, "", 0.0)?;
    let a = push("assistant", answer, thought, thought_secs)?;
    tx.execute("UPDATE chats SET updated_at=?1, title=CASE WHEN title='' THEN ?2 ELSE title END WHERE id=?3", params![t, title, id])?;
    tx.commit()?;
    Ok((q, a))
}

/// Titre et texte d'une leçon, sans la marquer comme ouverte.
pub fn lesson_brief(c: &Connection, id: i64) -> Result<Option<(String, String)>> {
    Ok(c.query_row("SELECT title, text FROM lessons WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
}

/// Mots connus (sans les expressions) : le chat s'adapte au niveau de l'apprenant.
pub fn known_words(c: &Connection, lang: &str) -> Result<i64> {
    Ok(c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND status=4 AND instr(term,' ')=0", [lang], |r| r.get(0))?)
}

// ---------- termes (mots et expressions) ----------

#[derive(Serialize, Clone, Debug)]
pub struct Term {
    pub term: String,
    pub status: i64,
    pub translation: String,
    pub note: String,
    pub lemma: String,
    pub context: String,
    pub updated_at: i64,
}

fn row_term(r: &rusqlite::Row) -> rusqlite::Result<Term> {
    Ok(Term {
        term: r.get(0)?,
        status: r.get(1)?,
        translation: r.get(2)?,
        note: r.get(3)?,
        lemma: r.get(4)?,
        context: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

/// Tous les termes d'une langue utiles à l'affichage d'une leçon.
pub fn terms_for_keys(c: &Connection, lang: &str, keys: &[String]) -> Result<HashMap<String, Term>> {
    let wanted: std::collections::HashSet<&str> = keys.iter().map(|s| s.as_str()).collect();
    let mut st = c.prepare_cached(
        "SELECT term,status,translation,note,lemma,context,updated_at FROM terms WHERE lang=?1",
    )?;
    let rows = st.query_map([lang], row_term)?;
    let mut out = HashMap::new();
    for t in rows.flatten() {
        // les expressions (avec espace) sont toujours renvoyées pour la détection
        if wanted.contains(t.term.as_str()) || t.term.contains(' ') {
            out.insert(t.term.clone(), t);
        }
    }
    Ok(out)
}

#[derive(Deserialize, Debug)]
pub struct TermUpdate {
    pub lang: String,
    pub term: String,
    pub status: i64,
    pub translation: Option<String>,
    pub note: Option<String>,
    pub lemma: Option<String>,
    pub context: Option<String>,
}

fn bump(c: &Connection, lang: &str, col: &str, n: i64) -> Result<()> {
    if n == 0 {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO activity(day,lang,{col}) VALUES(?1,?2,?3)
         ON CONFLICT(day,lang) DO UPDATE SET {col}={col}+excluded.{col}"
    );
    c.execute(&sql, params![today(), lang, n])?;
    Ok(())
}

/// Met à jour (ou crée) un terme. Statut 0 = supprimer (redevient « nouveau »).
pub fn term_set(c: &Connection, u: &TermUpdate) -> Result<()> {
    let term = text::normalize_for(&u.term, &u.lang);
    let prev: Option<i64> = c
        .query_row("SELECT status FROM terms WHERE lang=?1 AND term=?2", params![u.lang, term], |r| r.get(0))
        .optional()?;
    if u.status == 0 {
        c.execute("DELETE FROM terms WHERE lang=?1 AND term=?2", params![u.lang, term])?;
        if prev == Some(STATUS_KNOWN) {
            bump(c, &u.lang, "known_added", -1)?;
        }
        return Ok(());
    }
    let t = now();
    c.execute(
        "INSERT INTO terms(lang,term,status,translation,note,lemma,context,created_at,updated_at)
         VALUES(?1,?2,?3,COALESCE(?4,''),COALESCE(?5,''),COALESCE(?6,''),COALESCE(?7,''),?8,?8)
         ON CONFLICT(lang,term) DO UPDATE SET
            status=excluded.status,
            translation=COALESCE(?4, terms.translation),
            note=COALESCE(?5, terms.note),
            lemma=COALESCE(?6, terms.lemma),
            context=CASE WHEN terms.context='' THEN COALESCE(?7,'') ELSE terms.context END,
            updated_at=excluded.updated_at",
        params![u.lang, term, u.status, u.translation, u.note, u.lemma, u.context, t],
    )?;
    if prev.is_none() && (1..=3).contains(&u.status) {
        bump(c, &u.lang, "lingqs", 1)?;
    }
    if prev != Some(STATUS_KNOWN) && u.status == STATUS_KNOWN {
        bump(c, &u.lang, "known_added", 1)?;
    }
    if prev == Some(STATUS_KNOWN) && u.status != STATUS_KNOWN {
        bump(c, &u.lang, "known_added", -1)?;
    }
    Ok(())
}

/// « Terminer la page » : les mots jamais consultés passent en connu.
pub fn terms_mark_known(c: &mut Connection, lang: &str, keys: &[String], words_read: i64) -> Result<i64> {
    let tx = c.transaction()?;
    let t = now();
    let mut added = 0i64;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR IGNORE INTO terms(lang,term,status,created_at,updated_at) VALUES(?1,?2,4,?3,?3)",
        )?;
        let mut seen = std::collections::HashSet::new();
        for k in keys {
            let k = text::normalize_for(k, lang);
            if k.is_empty() || !seen.insert(k.clone()) {
                continue;
            }
            added += st.execute(params![lang, k, t])? as i64;
        }
    }
    bump(&tx, lang, "known_added", added)?;
    bump(&tx, lang, "words_read", words_read)?;
    tx.commit()?;
    Ok(added)
}

/// Terme venu d'une autre application (LingQ).
pub struct ImportedTerm {
    pub term: String,
    pub status: i64,
    pub translation: String,
    pub note: String,
    pub context: String,
}

/// Statut retenu quand un mot existe déjà : jamais de recul, et un mot
/// ignoré d'un côté ne remplace pas un mot étudié de l'autre.
fn merge_status(current: i64, incoming: i64) -> i64 {
    if current == STATUS_IGNORED || incoming == STATUS_IGNORED {
        return current;
    }
    current.max(incoming)
}

/// Fusionne des termes importés : un mot absent est ajouté, un mot présent
/// garde le statut le plus avancé et ne complète que ses champs vides.
/// L'activité du jour n'est pas touchée (ce ne sont pas des mots appris
/// aujourd'hui). Renvoie le nombre de termes ajoutés ou modifiés.
pub fn terms_import(c: &mut Connection, lang: &str, items: &[ImportedTerm]) -> Result<i64> {
    let tx = c.transaction()?;
    let t = now();
    let mut changed = 0i64;
    {
        let mut get = tx.prepare_cached("SELECT status,translation,note,context FROM terms WHERE lang=?1 AND term=?2")?;
        let mut ins = tx.prepare_cached(
            "INSERT INTO terms(lang,term,status,translation,note,context,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
        )?;
        let mut upd = tx.prepare_cached(
            "UPDATE terms SET status=?3, translation=?4, note=?5, context=?6, updated_at=?7 WHERE lang=?1 AND term=?2",
        )?;
        for it in items {
            let term = text::normalize_for(&it.term, lang);
            if term.is_empty() || term.chars().count() > 120 || !(1..=5).contains(&it.status) {
                continue;
            }
            let prev: Option<(i64, String, String, String)> = get
                .query_row(params![lang, term], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .optional()?;
            match prev {
                None => {
                    changed += ins.execute(params![lang, term, it.status, it.translation, it.note, it.context, t])? as i64;
                }
                Some((status, tr, note, ctx)) => {
                    let keep = |old: &String, new: &String| if old.is_empty() { new.clone() } else { old.clone() };
                    let next = (merge_status(status, it.status), keep(&tr, &it.translation), keep(&note, &it.note), keep(&ctx, &it.context));
                    if next != (status, tr, note, ctx) {
                        upd.execute(params![lang, term, next.0, next.1, next.2, next.3, t])?;
                        changed += 1;
                    }
                }
            }
        }
    }
    tx.commit()?;
    Ok(changed)
}

#[derive(Deserialize, Debug)]
pub struct TermQuery {
    pub lang: String,
    /// "learning" | "known" | "ignored" | "all" | "phrases"
    pub filter: String,
    pub search: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub fn terms_list(c: &Connection, q: &TermQuery) -> Result<(Vec<Term>, i64)> {
    let mut wh = String::from("lang=?1");
    match q.filter.as_str() {
        "learning" => wh.push_str(" AND status BETWEEN 1 AND 3"),
        "known" => wh.push_str(" AND status=4"),
        "ignored" => wh.push_str(" AND status=5"),
        "phrases" => wh.push_str(" AND instr(term,' ')>0"),
        _ => wh.push_str(" AND status<>5"),
    }
    let search = q.search.as_deref().map(text::normalize).unwrap_or_default();
    if !search.is_empty() {
        wh.push_str(" AND (term LIKE ?2 OR translation LIKE ?2)");
    } else {
        wh.push_str(" AND ?2=''");
    }
    let like = if search.is_empty() { String::new() } else { format!("%{search}%") };
    let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM terms WHERE {wh}"), params![q.lang, like], |r| r.get(0))?;
    let mut st = c.prepare(&format!(
        "SELECT term,status,translation,note,lemma,context,updated_at FROM terms WHERE {wh}
         ORDER BY CASE WHEN status BETWEEN 1 AND 3 THEN 0 ELSE 1 END, updated_at DESC LIMIT ?3 OFFSET ?4"
    ))?;
    let rows = st.query_map(params![q.lang, like, q.limit, q.offset], row_term)?;
    Ok((rows.filter_map(|r| r.ok()).collect(), total))
}

// ---------- statistiques ----------

/// Objectif du jour par défaut : minutes de temps actif dans les leçons.
pub const GOAL_MIN_DEFAULT: i64 = 10;

/// Objectif du jour (réglage `daily_goal`, en minutes).
pub fn goal_min(c: &Connection) -> i64 {
    setting(c, "daily_goal").and_then(|v| v.trim().parse().ok()).filter(|m| (1..=600).contains(m)).unwrap_or(GOAL_MIN_DEFAULT)
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct DayStat {
    pub day: String,
    pub words_read: i64,
    pub known_added: i64,
    pub lingqs: i64,
    pub listen_secs: i64,
    /// temps actif passé dans les leçons
    pub learn_secs: i64,
    /// objectif du jour atteint : la journée compte dans la série
    pub goal_met: bool,
}

impl DayStat {
    fn active(&self) -> bool {
        self.learn_secs > 0 || self.words_read > 0 || self.listen_secs > 0 || self.lingqs > 0 || self.known_added > 0
    }
}

/// Activité cumulée sur une période (jour, semaine, mois, tout).
#[derive(Serialize, Debug, Clone, Default)]
pub struct Span {
    /// premier jour de la période
    pub start: String,
    pub words_read: i64,
    pub known_added: i64,
    pub lingqs: i64,
    pub listen_secs: i64,
    pub learn_secs: i64,
    pub active_days: i64,
    pub goal_days: i64,
}

impl Span {
    fn at(start: NaiveDate) -> Span {
        Span { start: start.format("%Y-%m-%d").to_string(), ..Default::default() }
    }
    fn add(&mut self, d: &DayStat) {
        self.words_read += d.words_read;
        self.known_added += d.known_added;
        self.lingqs += d.lingqs;
        self.listen_secs += d.listen_secs;
        self.learn_secs += d.learn_secs;
        self.active_days += d.active() as i64;
        self.goal_days += d.goal_met as i64;
    }
}

#[derive(Serialize, Debug, Default)]
pub struct Periods {
    pub today: Span,
    pub yesterday: Span,
    /// semaine en cours, depuis lundi
    pub week: Span,
    pub last_week: Span,
    /// mois en cours, depuis le 1er
    pub month: Span,
    pub last_month: Span,
    pub total: Span,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Streak {
    /// jours de suite où l'objectif est atteint (aujourd'hui compris s'il l'est déjà)
    pub current: i64,
    pub best: i64,
    pub today_done: bool,
    pub goal_min: i64,
    pub today_secs: i64,
}

#[derive(Serialize, Debug, Default)]
pub struct Records {
    pub words_read: i64,
    pub words_day: String,
    pub learn_secs: i64,
    pub learn_day: String,
}

#[derive(Serialize, Debug)]
pub struct Stats {
    pub known: i64,
    pub learning: i64,
    pub phrases: i64,
    pub lessons: i64,
    /// 26 semaines entières (depuis un lundi) jusqu'à aujourd'hui, jour par jour
    pub days: Vec<DayStat>,
    /// 12 dernières semaines
    pub weeks: Vec<Span>,
    /// mois par mois depuis le début (12 au moins, 36 au plus)
    pub months: Vec<Span>,
    pub periods: Periods,
    pub streak: Streak,
    pub records: Records,
    /// premier jour d'activité
    pub first_day: Option<String>,
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// Activité de la langue, jour par jour (l'objectif d'aujourd'hui relu avec le réglage actuel).
fn activity_days(c: &Connection, lang: &str, today: NaiveDate, goal: i64) -> Result<Vec<(NaiveDate, DayStat)>> {
    let mut st = c.prepare(
        "SELECT day,words_read,known_added,lingqs,listen_secs,learn_secs,goal_met FROM activity WHERE lang=?1 ORDER BY day",
    )?;
    let rows = st.query_map([lang], |r| {
        Ok(DayStat {
            day: r.get(0)?,
            words_read: r.get(1)?,
            known_added: r.get(2)?,
            lingqs: r.get(3)?,
            listen_secs: r.get(4)?,
            learn_secs: r.get(5)?,
            goal_met: r.get::<_, i64>(6)? != 0,
        })
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter_map(|mut d| {
            let date = NaiveDate::parse_from_str(&d.day, "%Y-%m-%d").ok()?;
            // l'objectif a pu baisser depuis la dernière écriture
            if date == today && d.learn_secs >= goal * 60 {
                d.goal_met = true;
            }
            Some((date, d))
        })
        .collect())
}

/// Série en cours et record. La série d'hier tient encore tant que la journée n'est pas finie.
fn streak_of(days: &[(NaiveDate, DayStat)], today: NaiveDate, goal: i64) -> Streak {
    let met: std::collections::BTreeSet<NaiveDate> = days.iter().filter(|(_, d)| d.goal_met).map(|(n, _)| *n).collect();
    let today_done = met.contains(&today);
    let mut current = 0;
    let mut day = if today_done { Some(today) } else { today.pred_opt() };
    while let Some(d) = day.filter(|d| met.contains(d)) {
        current += 1;
        day = d.pred_opt();
    }
    let (mut best, mut run, mut prev) = (0, 0, None::<NaiveDate>);
    for &d in &met {
        run = if prev.and_then(|p| p.succ_opt()) == Some(d) { run + 1 } else { 1 };
        best = best.max(run);
        prev = Some(d);
    }
    let today_secs = days.iter().find(|(n, _)| *n == today).map(|(_, d)| d.learn_secs).unwrap_or(0);
    Streak { current, best: best.max(current), today_done, goal_min: goal, today_secs }
}

pub fn stats(c: &Connection, lang: &str) -> Result<Stats> {
    stats_at(c, lang, today_date())
}

fn stats_at(c: &Connection, lang: &str, today: NaiveDate) -> Result<Stats> {
    let known: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND status=4 AND instr(term,' ')=0", [lang], |r| r.get(0))?;
    let learning: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND status BETWEEN 1 AND 3", [lang], |r| r.get(0))?;
    let phrases: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND instr(term,' ')>0 AND status<>5", [lang], |r| r.get(0))?;
    let lessons: i64 = c.query_row("SELECT COUNT(*) FROM lessons WHERE lang=?1", [lang], |r| r.get(0))?;
    let goal = goal_min(c);
    let all = activity_days(c, lang, today, goal)?;

    // bornes des périodes
    let yesterday = today.pred_opt().unwrap_or(today);
    let week = monday(today);
    let last_week = week - chrono::Duration::days(7);
    let month = first_of_month(today);
    let last_month = month.checked_sub_months(Months::new(1)).unwrap_or(month);
    let mut p = Periods {
        today: Span::at(today),
        yesterday: Span::at(yesterday),
        week: Span::at(week),
        last_week: Span::at(last_week),
        month: Span::at(month),
        last_month: Span::at(last_month),
        total: Span::default(),
    };
    let first = all.iter().find(|(_, d)| d.active()).map(|(n, _)| *n);
    p.total.start = first.map(ymd).unwrap_or_else(|| ymd(today));

    // 12 semaines, et les mois depuis le début (12 au moins, 36 au plus)
    let weeks_from = week - chrono::Duration::days(7 * 11);
    let mut weeks: Vec<Span> = (0..12).map(|i| Span::at(weeks_from + chrono::Duration::days(7 * i))).collect();
    let min_from = month.checked_sub_months(Months::new(11)).unwrap_or(month);
    let max_from = month.checked_sub_months(Months::new(35)).unwrap_or(month);
    let months_from = first.map(first_of_month).unwrap_or(min_from).clamp(max_from, min_from);
    let mut months = Vec::new();
    let mut m = months_from;
    while m <= month {
        months.push(Span::at(m));
        m = match m.checked_add_months(Months::new(1)) {
            Some(n) => n,
            None => break,
        };
    }

    // jour par jour : 26 semaines entières jusqu'à aujourd'hui
    let days_from = week - chrono::Duration::days(7 * 25);
    let found: HashMap<NaiveDate, &DayStat> = all.iter().map(|(n, d)| (*n, d)).collect();
    let mut days = Vec::new();
    let mut d = days_from;
    while d <= today {
        days.push(found.get(&d).map(|x| (*x).clone()).unwrap_or_else(|| DayStat { day: ymd(d), ..Default::default() }));
        d = match d.succ_opt() {
            Some(n) => n,
            None => break,
        };
    }

    let mut records = Records::default();
    for (n, d) in &all {
        let n = *n;
        p.total.add(d);
        if n == today {
            p.today.add(d);
        }
        if n == yesterday {
            p.yesterday.add(d);
        }
        if n >= week && n <= today {
            p.week.add(d);
        }
        if n >= last_week && n < week {
            p.last_week.add(d);
        }
        if n >= month && n <= today {
            p.month.add(d);
        }
        if n >= last_month && n < month {
            p.last_month.add(d);
        }
        if n >= weeks_from && n <= today {
            let i = ((monday(n) - weeks_from).num_days() / 7) as usize;
            if let Some(w) = weeks.get_mut(i) {
                w.add(d);
            }
        }
        if n >= months_from && n <= today {
            let i = ((n.year() - months_from.year()) * 12 + n.month() as i32 - months_from.month() as i32) as usize;
            if let Some(m) = months.get_mut(i) {
                m.add(d);
            }
        }
        if d.words_read > records.words_read {
            records.words_read = d.words_read;
            records.words_day = d.day.clone();
        }
        if d.learn_secs > records.learn_secs {
            records.learn_secs = d.learn_secs;
            records.learn_day = d.day.clone();
        }
    }

    Ok(Stats {
        known,
        learning,
        phrases,
        lessons,
        days,
        weeks,
        months,
        periods: p,
        streak: streak_of(&all, today, goal),
        records,
        first_day: first.map(ymd),
    })
}

/// Série atteinte quand un temps d'apprentissage fait passer l'objectif du jour.
#[derive(Serialize, Debug)]
pub struct GoalReached {
    pub streak: i64,
    pub goal_min: i64,
}

/// Ajoute à l'activité du jour : mots lus, écoute, temps actif dans les leçons.
/// Renvoie la série si ce temps vient de faire atteindre l'objectif du jour.
pub fn activity_add(c: &Connection, lang: &str, words_read: i64, listen_secs: i64, learn_secs: i64) -> Result<Option<GoalReached>> {
    bump(c, lang, "words_read", words_read.max(0))?;
    bump(c, lang, "listen_secs", listen_secs.max(0))?;
    bump(c, lang, "learn_secs", learn_secs.max(0))?;
    if learn_secs <= 0 {
        return Ok(None);
    }
    let goal = goal_min(c);
    let reached = c.execute(
        "UPDATE activity SET goal_met=1 WHERE day=?1 AND lang=?2 AND goal_met=0 AND learn_secs>=?3",
        params![today(), lang, goal * 60],
    )?;
    if reached == 0 {
        return Ok(None);
    }
    let today = today_date();
    let streak = streak_of(&activity_days(c, lang, today, goal)?, today, goal).current;
    Ok(Some(GoalReached { streak, goal_min: goal }))
}

/// L'objectif a changé : une journée déjà au-dessus du nouvel objectif est acquise.
pub fn goal_refresh(c: &Connection) -> Result<()> {
    c.execute(
        "UPDATE activity SET goal_met=1 WHERE day=?1 AND goal_met=0 AND learn_secs>=?2",
        params![today(), goal_min(c) * 60],
    )?;
    Ok(())
}

// ---------- cache de traduction ----------

pub fn cache_get(c: &Connection, k: &str) -> Option<String> {
    c.query_row("SELECT v FROM tcache WHERE k=?1", [k], |r| r.get(0)).optional().ok().flatten()
}

pub fn cache_put(c: &Connection, k: &str, v: &str) {
    let _ = c.execute("INSERT OR REPLACE INTO tcache(k,v,created_at) VALUES(?1,?2,?3)", params![k, v, now()]);
}

pub fn export_csv(c: &Connection, lang: &str) -> Result<String> {
    let mut st = c.prepare(
        "SELECT term,translation,note,context,status FROM terms WHERE lang=?1 AND status BETWEEN 1 AND 4 ORDER BY status, term",
    )?;
    let esc = |s: String| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("terme,traduction,note,contexte,statut\n");
    let rows = st.query_map([lang], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
    })?;
    for (a, b, n, ctx, s) in rows.flatten() {
        out.push_str(&format!("{},{},{},{},{}\n", esc(a), esc(b), esc(n), esc(ctx), s));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lesson_and_terms_flow() {
        let path = std::env::temp_dir().join(format!("lumen-db-test-{}.db", std::process::id()));
        let mut c = open(&path).unwrap();
        let id = lesson_create(&c, &NewLesson {
            lang: "en".into(), title: "T".into(), collection: String::new(), kind: "text".into(),
            source: String::new(), text: "The cat sat. The dog ran.".into(), media_path: None, timings: None, video_path: None,
        }).unwrap();
        let l = lessons_list(&c, "en").unwrap();
        assert_eq!(l[0].new_words, 5);
        term_set(&c, &TermUpdate { lang: "en".into(), term: "Cat".into(), status: 1, translation: Some("chat".into()), note: None, lemma: None, context: Some("The cat sat.".into()) }).unwrap();
        let added = terms_mark_known(&mut c, "en", &["the".into(), "sat".into(), "cat".into()], 6).unwrap();
        assert_eq!(added, 2);
        let s = stats(&c, "en").unwrap();
        assert_eq!(s.known, 2);
        assert_eq!(s.learning, 1);
        assert_eq!(s.periods.today.words_read, 6);
        assert_eq!(s.periods.today.lingqs, 1);
        let l = lessons_list(&c, "en").unwrap();
        assert_eq!(l[0].new_words, 2);
        let lesson = lesson_get(&c, id).unwrap();
        assert_eq!(lesson.word_count, 6);
        let (items, total) = terms_list(&c, &TermQuery { lang: "en".into(), filter: "learning".into(), search: Some("chat".into()), limit: 10, offset: 0 }).unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].translation, "chat");
        assert!(export_csv(&c, "en").unwrap().contains("\"cat\",\"chat\""));
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn progress_periods_and_streak() {
        let path = std::env::temp_dir().join(format!("lumen-db-progress-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = open(&path).unwrap();
        // mercredi 14 octobre 2026
        let today = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let day = |n: i64| ymd(today - chrono::Duration::days(n));
        let put = |n: i64, words: i64, learn: i64, met: bool| {
            c.execute(
                "INSERT INTO activity(day,lang,words_read,learn_secs,listen_secs,goal_met) VALUES(?1,'it',?2,?3,60,?4)",
                params![day(n), words, learn, met as i64],
            )
            .unwrap();
        };
        // série de 3 jours jusqu'à hier (aujourd'hui pas encore atteint), puis un trou,
        // et un ancien record de 5 jours le mois dernier
        put(0, 100, 120, false);
        for n in 1..=3 {
            put(n, 200, 900, true);
        }
        for n in 20..=24 {
            put(n, 50, 700, true);
        }
        let s = stats_at(&c, "it", today).unwrap();
        assert_eq!(s.streak, Streak { current: 3, best: 5, today_done: false, goal_min: 10, today_secs: 120 });
        assert_eq!(s.periods.today.words_read, 100);
        assert_eq!(s.periods.yesterday.learn_secs, 900);
        // semaine depuis lundi 12 : aujourd'hui, mardi, lundi
        assert_eq!((s.periods.week.start.as_str(), s.periods.week.words_read, s.periods.week.active_days), ("2026-10-12", 500, 3));
        assert_eq!(s.periods.last_week.words_read, 200);
        assert_eq!((s.periods.month.start.as_str(), s.periods.month.words_read), ("2026-10-01", 700));
        assert_eq!((s.periods.last_month.start.as_str(), s.periods.last_month.words_read), ("2026-09-01", 250));
        assert_eq!((s.periods.total.words_read, s.periods.total.learn_secs, s.periods.total.goal_days), (950, 120 + 2700 + 3500, 8));
        assert_eq!(s.first_day.as_deref(), Some("2026-09-20"));
        assert_eq!(s.days.len(), 7 * 25 + 3);
        assert_eq!(s.days.first().map(|d| d.day.as_str()), Some("2026-04-20"));
        assert_eq!(s.weeks.len(), 12);
        assert_eq!(s.weeks.last().map(|w| (w.start.as_str(), w.words_read)), Some(("2026-10-12", 500)));
        assert_eq!(s.months.len(), 12);
        assert_eq!(s.months.last().map(|m| (m.start.as_str(), m.words_read)), Some(("2026-10-01", 700)));
        assert_eq!((s.records.words_read, s.records.learn_secs), (200, 900));
        // un objectif plus bas compte tout de suite pour aujourd'hui
        setting_set(&c, "daily_goal", "2").unwrap();
        let s = stats_at(&c, "it", today).unwrap();
        assert_eq!((s.streak.current, s.streak.today_done, s.streak.goal_min), (4, true, 2));
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn goal_reached_once() {
        let path = std::env::temp_dir().join(format!("lumen-db-goal-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = open(&path).unwrap();
        setting_set(&c, "daily_goal", "1").unwrap();
        assert!(activity_add(&c, "de", 0, 0, 30).unwrap().is_none());
        let r = activity_add(&c, "de", 0, 0, 30).unwrap().expect("objectif atteint");
        assert_eq!((r.streak, r.goal_min), (1, 1));
        // une seule annonce par jour
        assert!(activity_add(&c, "de", 0, 0, 30).unwrap().is_none());
        assert!(activity_add(&c, "de", 120, 45, 0).unwrap().is_none());
        let s = stats(&c, "de").unwrap();
        assert_eq!((s.periods.today.learn_secs, s.periods.today.words_read, s.periods.today.listen_secs), (90, 120, 45));
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_activity_keeps_its_streak() {
        let path = std::env::temp_dir().join(format!("lumen-db-legacy-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // base d'avant le temps actif : un jour de lecture, un jour sans rien de notable
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE activity(day TEXT NOT NULL, lang TEXT NOT NULL, words_read INTEGER NOT NULL DEFAULT 0,
                 known_added INTEGER NOT NULL DEFAULT 0, lingqs INTEGER NOT NULL DEFAULT 0,
                 listen_secs INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(day, lang));
                 INSERT INTO activity(day,lang,words_read) VALUES('2026-01-02','it',230);
                 INSERT INTO activity(day,lang,lingqs) VALUES('2026-01-03','it',2);",
            )
            .unwrap();
        }
        let c = open(&path).unwrap();
        let met: Vec<String> = c.prepare("SELECT day FROM activity WHERE goal_met=1").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(met, vec!["2026-01-02".to_string()]);
        // rouvrir ne refait pas la reprise
        c.execute("UPDATE activity SET goal_met=0", []).unwrap();
        drop(c);
        let c = open(&path).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM activity WHERE goal_met=1", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn resume_position_and_cover() {
        let path = std::env::temp_dir().join(format!("lumen-db-resume-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // base d'une ancienne version : ni position, ni couverture
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE lessons(id INTEGER PRIMARY KEY AUTOINCREMENT, lang TEXT NOT NULL, title TEXT NOT NULL,
                 collection TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL DEFAULT 'text', source TEXT NOT NULL DEFAULT '',
                 text TEXT NOT NULL, media_path TEXT, timings TEXT, hue INTEGER NOT NULL DEFAULT 210,
                 word_count INTEGER NOT NULL DEFAULT 0, page INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL, opened_at INTEGER);
                 INSERT INTO lessons(lang,title,text,created_at) VALUES('en','Ancienne','Old text here.',1);",
            )
            .unwrap();
        }
        let c = open(&path).unwrap();
        let l = lesson_get(&c, 1).unwrap();
        assert_eq!((l.position, l.anchor, l.duration, l.cover_path.clone()), (0.0, 0, 0.0, None));
        lesson_update(&c, 1, &LessonPatch { position: Some(754.3), anchor: Some(42), duration: Some(1510.0), page: Some(2), ..Default::default() }).unwrap();
        let l = lesson_get(&c, 1).unwrap();
        assert_eq!((l.position, l.anchor, l.duration, l.page), (754.3, 42, 1510.0, 2));
        assert_eq!(lesson_set_cover(&c, 1, Some("/m/a.cover.jpg")).unwrap(), None);
        assert_eq!(lesson_set_cover(&c, 1, Some("/m/b.cover.jpg")).unwrap().as_deref(), Some("/m/a.cover.jpg"));
        let s = &lessons_list(&c, "en").unwrap()[0];
        assert_eq!((s.position, s.duration, s.cover_path.as_deref()), (754.3, 1510.0, Some("/m/b.cover.jpg")));
        assert!(lesson_delete(&c, 1).unwrap().contains(&"/m/b.cover.jpg".to_string()));
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn playlists_keep_order_and_follow_lessons() {
        let path = std::env::temp_dir().join(format!("lumen-db-playlists-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path).unwrap();
        let mk = |c: &Connection, lang: &str, title: &str| {
            lesson_create(c, &NewLesson {
                lang: lang.into(), title: title.into(), collection: String::new(), kind: "text".into(),
                source: String::new(), text: "Hello there.".into(), media_path: None, timings: None, video_path: None,
            }).unwrap()
        };
        let (a, b, d) = (mk(&c, "en", "A"), mk(&c, "en", "B"), mk(&c, "en", "D"));
        let other = mk(&c, "it", "Altra lingua");
        // doublons et leçon d'une autre langue écartés, nom vide remplacé
        let id = playlist_create(&mut c, "en", "   ", &[b, a, b, other]).unwrap();
        let p = &playlists_list(&c, "en").unwrap()[0];
        assert_eq!((p.name.as_str(), p.lessons.clone(), p.current), ("Nouvelle playlist", vec![b, a], None));
        playlist_update(&mut c, id, &PlaylistPatch { name: Some("  Le  matin ".into()), lessons: Some(vec![a, d, b]), current: Some(d) }).unwrap();
        let p = &playlists_list(&c, "en").unwrap()[0];
        assert_eq!((p.name.as_str(), p.lessons.clone(), p.current), ("Le matin", vec![a, d, b], Some(d)));
        // une leçon supprimée quitte la playlist, et n'y est plus « en cours »
        lesson_delete(&c, d).unwrap();
        let p = &playlists_list(&c, "en").unwrap()[0];
        assert_eq!((p.lessons.clone(), p.current), (vec![a, b], None));
        assert!(playlists_list(&c, "it").unwrap().is_empty());
        playlist_update(&mut c, id, &PlaylistPatch { current: Some(0), ..Default::default() }).unwrap();
        playlist_delete(&c, id).unwrap();
        assert!(playlists_list(&c, "en").unwrap().is_empty());
        let left: i64 = c.query_row("SELECT COUNT(*) FROM playlist_items", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chats_keep_messages_and_follow_lessons() {
        let path = std::env::temp_dir().join(format!("lumen-db-chats-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path).unwrap();
        let mk = |c: &Connection, lang: &str, title: &str| {
            lesson_create(c, &NewLesson {
                lang: lang.into(), title: title.into(), collection: String::new(), kind: "text".into(),
                source: String::new(), text: "Ciao a tutti.".into(), media_path: None, timings: None, video_path: None,
            }).unwrap()
        };
        let it = mk(&c, "it", "Il faro");
        let en = mk(&c, "en", "The lighthouse");
        // une leçon d'une autre langue n'est pas jointe
        let other = chat_create(&c, "it", Some(en)).unwrap();
        assert_eq!(chat_get(&c, other).unwrap().lesson_id, None);
        let id = chat_create(&c, "it", Some(it)).unwrap();
        let s = chat_get(&c, id).unwrap();
        assert_eq!((s.title.as_str(), s.lesson_title.as_deref(), s.count), ("", Some("Il faro"), 0));
        // la première question donne le titre, les suivantes ne le changent pas
        let (q, a) = chat_append(&mut c, id, "Che vuol dire faro ?", "**Faro** : phare.", "Il cherche le sens.", 2.5, "Che vuol dire faro ?").unwrap();
        assert_eq!((q.role.as_str(), a.role.as_str(), a.thought_secs), ("user", "assistant", 2.5));
        chat_append(&mut c, id, "Et mare ?", "La mer.", "", 0.0, "Et mare ?").unwrap();
        let s = chat_get(&c, id).unwrap();
        assert_eq!((s.title.as_str(), s.count, s.preview.as_str()), ("Che vuol dire faro ?", 4, "La mer."));
        let m = chat_messages(&c, id).unwrap();
        assert_eq!(m.iter().map(|x| x.content.as_str()).collect::<Vec<_>>(), vec!["Che vuol dire faro ?", "**Faro** : phare.", "Et mare ?", "La mer."]);
        assert_eq!(m[1].thought, "Il cherche le sens.");
        assert_eq!(chats_list(&c, "it").unwrap().len(), 2);
        assert!(chats_list(&c, "en").unwrap().is_empty());
        // renommer, retirer puis rejoindre la leçon
        chat_update(&c, id, &ChatPatch { title: Some("  Le   phare ".into()), lesson: Some(0) }).unwrap();
        let s = chat_get(&c, id).unwrap();
        assert_eq!((s.title.as_str(), s.lesson_id), ("Le phare", None));
        chat_update(&c, id, &ChatPatch { lesson: Some(it), ..Default::default() }).unwrap();
        assert_eq!(chat_get(&c, id).unwrap().lesson_id, Some(it));
        // leçon supprimée : la conversation reste, sans leçon
        lesson_delete(&c, it).unwrap();
        let s = chat_get(&c, id).unwrap();
        assert_eq!((s.lesson_id, s.count), (None, 4));
        // conversation supprimée : ses messages aussi
        chat_delete(&c, id).unwrap();
        let left: i64 = c.query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        assert_eq!(known_words(&c, "it").unwrap(), 0);
        drop(c);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn import_never_goes_backwards() {
        let path = std::env::temp_dir().join(format!("lumen-db-import-{}.db", std::process::id()));
        let mut c = open(&path).unwrap();
        let it = |term: &str, status: i64, tr: &str| ImportedTerm {
            term: term.into(), status, translation: tr.into(), note: String::new(), context: String::new(),
        };
        term_set(&c, &TermUpdate { lang: "en".into(), term: "cat".into(), status: 4, translation: None, note: None, lemma: None, context: None }).unwrap();
        term_set(&c, &TermUpdate { lang: "en".into(), term: "dog".into(), status: 5, translation: None, note: None, lemma: None, context: None }).unwrap();
        let n = terms_import(&mut c, "en", &[it("Cat", 2, "chat"), it("dog", 1, "chien"), it("Bird", 3, "oiseau"), it("the", 4, ""), it("", 4, "")]).unwrap();
        assert_eq!(n, 4);
        let all = terms_for_keys(&c, "en", &["cat".into(), "dog".into(), "bird".into(), "the".into()]).unwrap();
        assert_eq!((all["cat"].status, all["cat"].translation.as_str()), (4, "chat"));
        assert_eq!(all["dog"].status, 5);
        assert_eq!(all["bird"].status, 3);
        // un nouvel import identique ne change rien, et l'activité du jour reste vide
        assert_eq!(terms_import(&mut c, "en", &[it("bird", 3, "oiseau")]).unwrap(), 0);
        assert_eq!(stats(&c, "en").unwrap().periods.today.lingqs, 0);
        // leçon importée retrouvée par son origine
        let id = lesson_create(&c, &NewLesson {
            lang: "en".into(), title: "L".into(), collection: "C".into(), kind: "text".into(),
            source: String::new(), text: "Hello there.".into(), media_path: None, timings: None, video_path: None,
        }).unwrap();
        lesson_set_ext(&c, id, "lingq:42", true).unwrap();
        assert_eq!(lesson_by_ext(&c, "lingq:42").unwrap(), Some(id));
        assert_eq!(lesson_by_ext(&c, "lingq:43").unwrap(), None);
        drop(c);
        let _ = std::fs::remove_file(path);
    }
}
