//! Base de données locale (SQLite) : leçons, mots, activité, réglages, cache.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::text;

pub const STATUS_KNOWN: i64 = 4;
pub const STATUS_IGNORED: i64 = 5;

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
    // migrations
    let has_video: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('lessons') WHERE name='video_path'")?
        .exists([])?;
    if !has_video {
        conn.execute_batch("ALTER TABLE lessons ADD COLUMN video_path TEXT;")?;
    }
    // identifiant d'origine des leçons importées (ex. « lingq:123 ») pour ne pas les dupliquer
    let has_ext: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('lessons') WHERE name='ext_id'")?
        .exists([])?;
    if !has_ext {
        conn.execute_batch("ALTER TABLE lessons ADD COLUMN ext_id TEXT; CREATE INDEX IF NOT EXISTS lessons_ext ON lessons(ext_id);")?;
    }
    Ok(conn)
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
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
        "SELECT id,lang,title,collection,kind,source,hue,word_count,page,completed,media_path,created_at,opened_at,text
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
        "SELECT id,lang,title,collection,kind,source,text,media_path,timings,hue,word_count,page,completed,video_path FROM lessons WHERE id=?1",
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
    Ok(())
}

pub fn lesson_delete(c: &Connection, id: i64) -> Result<Vec<String>> {
    let files: Option<(Option<String>, Option<String>)> = c
        .query_row("SELECT media_path, video_path FROM lessons WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    c.execute("DELETE FROM lessons WHERE id=?1", [id])?;
    let mut out = Vec::new();
    if let Some((a, v)) = files {
        out.extend(a);
        out.extend(v);
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

/// Leçon déjà importée depuis cette origine ?
pub fn lesson_by_ext(c: &Connection, ext_id: &str) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT id FROM lessons WHERE ext_id=?1", [ext_id], |r| r.get(0)).optional()?)
}

pub fn lesson_set_ext(c: &Connection, id: i64, ext_id: &str, completed: bool) -> Result<()> {
    c.execute("UPDATE lessons SET ext_id=?1, completed=?2 WHERE id=?3", params![ext_id, completed as i64, id])?;
    Ok(())
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
    let term = text::normalize(&u.term);
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
            let k = text::normalize(k);
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
            let term = text::normalize(&it.term);
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

#[derive(Serialize, Debug)]
pub struct DayStat {
    pub day: String,
    pub words_read: i64,
    pub known_added: i64,
    pub lingqs: i64,
    pub listen_secs: i64,
}

#[derive(Serialize, Debug)]
pub struct Stats {
    pub known: i64,
    pub learning: i64,
    pub phrases: i64,
    pub lessons: i64,
    pub words_read_total: i64,
    pub listen_secs_total: i64,
    pub today: DayStat,
    pub days: Vec<DayStat>,
}

pub fn stats(c: &Connection, lang: &str) -> Result<Stats> {
    let known: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND status=4 AND instr(term,' ')=0", [lang], |r| r.get(0))?;
    let learning: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND status BETWEEN 1 AND 3", [lang], |r| r.get(0))?;
    let phrases: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE lang=?1 AND instr(term,' ')>0 AND status<>5", [lang], |r| r.get(0))?;
    let lessons: i64 = c.query_row("SELECT COUNT(*) FROM lessons WHERE lang=?1", [lang], |r| r.get(0))?;
    let (wr, ls): (i64, i64) = c.query_row(
        "SELECT COALESCE(SUM(words_read),0), COALESCE(SUM(listen_secs),0) FROM activity WHERE lang=?1",
        [lang],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut st = c.prepare(
        "SELECT day,words_read,known_added,lingqs,listen_secs FROM activity WHERE lang=?1 AND day>=?2 ORDER BY day",
    )?;
    let since = (chrono::Local::now() - chrono::Duration::days(29)).format("%Y-%m-%d").to_string();
    let found: HashMap<String, DayStat> = st
        .query_map(params![lang, since], |r| {
            Ok(DayStat { day: r.get(0)?, words_read: r.get(1)?, known_added: r.get(2)?, lingqs: r.get(3)?, listen_secs: r.get(4)? })
        })?
        .filter_map(|r| r.ok())
        .map(|d| (d.day.clone(), d))
        .collect();
    let mut days = Vec::new();
    for i in (0..30).rev() {
        let d = (chrono::Local::now() - chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
        days.push(match found.get(&d) {
            Some(x) => DayStat { day: d, words_read: x.words_read, known_added: x.known_added, lingqs: x.lingqs, listen_secs: x.listen_secs },
            None => DayStat { day: d, words_read: 0, known_added: 0, lingqs: 0, listen_secs: 0 },
        });
    }
    let t = days.last().map(|d| DayStat { day: d.day.clone(), words_read: d.words_read, known_added: d.known_added, lingqs: d.lingqs, listen_secs: d.listen_secs }).unwrap();
    Ok(Stats { known, learning, phrases, lessons, words_read_total: wr, listen_secs_total: ls, today: t, days })
}

pub fn activity_add(c: &Connection, lang: &str, words_read: i64, listen_secs: i64) -> Result<()> {
    bump(c, lang, "words_read", words_read)?;
    bump(c, lang, "listen_secs", listen_secs)?;
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
        assert_eq!(s.today.words_read, 6);
        assert_eq!(s.today.lingqs, 1);
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
        assert_eq!(stats(&c, "en").unwrap().today.lingqs, 0);
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
