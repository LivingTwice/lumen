//! Synchronisation entre appareils, par la sauvegarde : chaque appareil (Mac, PC,
//! iPhone, iPad) sauvegarde dans son propre dossier, et ajoute à sa base ce qu'a
//! changé l'autre, sans rien écraser (backup.rs choisit quand et quoi fusionner).
//!
//! Règles, pour que deux appareils qui fusionnent l'un l'autre arrivent au même état :
//! - un mot : le plus récemment modifié l'emporte (`updated_at`) ; un mot remis à
//!   « nouveau » d'un côté disparaît de l'autre (traces de suppression, `tombstones`) ;
//! - une leçon se reconnaît à son identifiant (`uid`, le même partout) : une leçon
//!   nouvelle arrive avec son son et sa couverture s'ils sont dans la sauvegarde ;
//!   l'avancement le plus récent l'emporte (page, mot, seconde, « Lu ») ; une leçon
//!   supprimée d'un côté après sa dernière lecture disparaît de l'autre ;
//! - l'activité d'un jour : le plus grand des deux pour chaque compteur (rien n'est
//!   compté deux fois) ;
//! - une playlist (même nom, même langue) reçoit les leçons qui lui manquent.
//! Les conversations du chat et les réglages restent propres à chaque appareil.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db;

/// Ce qu'a apporté une fusion.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// mots ajoutés ou mis à jour
    pub terms: u64,
    /// mots connus de plus (toutes langues)
    pub known: i64,
    /// leçons arrivées
    pub lessons: u64,
    /// leçons dont l'avancement a été repris
    pub progress: u64,
    /// leçons et mots supprimés ici parce qu'ils l'ont été là-bas
    pub removed: u64,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.terms == 0 && self.lessons == 0 && self.progress == 0 && self.removed == 0
    }
}

/// Une leçon de l'autre appareil.
struct Other {
    id: i64,
    uid: String,
    lang: String,
    title: String,
    collection: String,
    kind: String,
    source: String,
    text: String,
    media_path: Option<String>,
    timings: Option<String>,
    hue: i64,
    word_count: i64,
    page: i64,
    completed: i64,
    created_at: i64,
    opened_at: Option<i64>,
    video_path: Option<String>,
    ext_id: Option<String>,
    position: f64,
    anchor: i64,
    duration: f64,
    cover_path: Option<String>,
    timing_v: i64,
}

/// Dernière activité connue d'une leçon : sa dernière lecture, sinon sa création.
fn touched(opened: Option<i64>, created: i64) -> i64 {
    opened.unwrap_or(created)
}

fn known_total(c: &Connection) -> Result<i64> {
    Ok(c.query_row("SELECT COUNT(*) FROM terms WHERE status=?1", [db::STATUS_KNOWN], |r| r.get(0))?)
}

/// Ajoute à `live` ce qu'apporte `other` (base d'un autre appareil, déjà ouverte une
/// fois par `db::open` : à jour, avec ses identifiants). `media(nom)` rapatrie un
/// fichier de la sauvegarde (son, image, couverture) et renvoie son chemin ici.
/// Renvoie aussi les fichiers des leçons supprimées ici, à effacer du disque.
pub fn merge(live: &mut Connection, other: &Path, media: &dyn Fn(&str) -> Option<String>) -> Result<(Report, Vec<String>)> {
    db::ensure_uids(live)?;
    live.execute("ATTACH DATABASE ?1 AS o", [other.to_string_lossy()])?;
    let res = merge_attached(live, media);
    let _ = live.execute("DETACH DATABASE o", []);
    res
}

fn merge_attached(live: &mut Connection, media: &dyn Fn(&str) -> Option<String>) -> Result<(Report, Vec<String>)> {
    let mut report = Report::default();
    let known_before = known_total(live)?;
    let tx = live.transaction()?;

    // ---------- mots ----------
    // supprimés là-bas après leur dernière modification ici
    report.removed += tx.execute(
        "DELETE FROM main.terms WHERE EXISTS (SELECT 1 FROM o.tombstones t
           WHERE t.kind='term' AND t.key = main.terms.lang || '|' || main.terms.term AND t.at > main.terms.updated_at)",
        [],
    )? as u64;
    // nouveaux ou plus récents là-bas, sauf supprimés ici depuis
    report.terms += tx.execute(
        "INSERT INTO main.terms(lang,term,status,translation,note,lemma,context,created_at,updated_at)
         SELECT x.lang, x.term, x.status, x.translation, x.note, x.lemma, x.context, x.created_at, x.updated_at FROM o.terms x
         WHERE NOT EXISTS (SELECT 1 FROM main.tombstones t WHERE t.kind='term' AND t.key = x.lang || '|' || x.term AND t.at >= x.updated_at)
         ON CONFLICT(lang,term) DO UPDATE SET
           status=excluded.status,
           translation=CASE WHEN excluded.translation<>'' THEN excluded.translation ELSE terms.translation END,
           note=CASE WHEN excluded.note<>'' THEN excluded.note ELSE terms.note END,
           lemma=CASE WHEN excluded.lemma<>'' THEN excluded.lemma ELSE terms.lemma END,
           context=CASE WHEN terms.context='' THEN excluded.context ELSE terms.context END,
           created_at=min(terms.created_at, excluded.created_at),
           updated_at=excluded.updated_at
         WHERE excluded.updated_at > terms.updated_at",
        [],
    )? as u64;

    // ---------- activité : le plus grand des deux, jour par jour ----------
    tx.execute(
        "INSERT INTO main.activity(day,lang,words_read,known_added,lingqs,listen_secs,learn_secs,goal_met)
         SELECT day,lang,words_read,known_added,lingqs,listen_secs,learn_secs,goal_met FROM o.activity WHERE 1
         ON CONFLICT(day,lang) DO UPDATE SET
           words_read=max(activity.words_read, excluded.words_read),
           known_added=max(activity.known_added, excluded.known_added),
           lingqs=max(activity.lingqs, excluded.lingqs),
           listen_secs=max(activity.listen_secs, excluded.listen_secs),
           learn_secs=max(activity.learn_secs, excluded.learn_secs),
           goal_met=max(activity.goal_met, excluded.goal_met)",
        [],
    )?;

    // ---------- leçons ----------
    let mut here: HashMap<String, (i64, Option<i64>, i64, i64)> = HashMap::new();
    {
        let mut st = tx.prepare("SELECT uid, id, opened_at, created_at, timing_v FROM main.lessons WHERE uid IS NOT NULL")?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))? {
            let (u, v) = r?;
            here.insert(u, v);
        }
    }
    let gone_here: HashMap<String, i64> = tx
        .prepare("SELECT key, at FROM main.tombstones WHERE kind='lesson'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let gone_there: HashMap<String, i64> = tx
        .prepare("SELECT key, at FROM o.tombstones WHERE kind='lesson'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let others: Vec<Other> = tx
        .prepare(
            "SELECT id, uid, lang, title, collection, kind, source, text, media_path, timings, hue, word_count, page, completed,
                    created_at, opened_at, video_path, ext_id, position, anchor, duration, cover_path, timing_v
             FROM o.lessons WHERE uid IS NOT NULL AND uid<>''",
        )?
        .query_map([], |r| {
            Ok(Other {
                id: r.get(0)?,
                uid: r.get(1)?,
                lang: r.get(2)?,
                title: r.get(3)?,
                collection: r.get(4)?,
                kind: r.get(5)?,
                source: r.get(6)?,
                text: r.get(7)?,
                media_path: r.get(8)?,
                timings: r.get(9)?,
                hue: r.get(10)?,
                word_count: r.get(11)?,
                page: r.get(12)?,
                completed: r.get(13)?,
                created_at: r.get(14)?,
                opened_at: r.get(15)?,
                video_path: r.get(16)?,
                ext_id: r.get(17)?,
                position: r.get(18)?,
                anchor: r.get(19)?,
                duration: r.get(20)?,
                cover_path: r.get(21)?,
                timing_v: r.get(22)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    // leçon de l'autre appareil (son identifiant là-bas) → leçon d'ici
    let mut mapped: HashMap<i64, i64> = HashMap::new();
    let bring = |p: &Option<String>| -> Option<String> {
        let p = p.as_deref()?;
        let name = p.rsplit(['/', '\\']).next().unwrap_or_default();
        if name.is_empty() {
            return None;
        }
        media(name)
    };
    for o in &others {
        if let Some(&(id, opened, created, timing_v)) = here.get(&o.uid) {
            mapped.insert(o.id, id);
            // l'avancement le plus récent l'emporte
            if touched(o.opened_at, o.created_at) > touched(opened, created) && o.opened_at.is_some() {
                tx.execute(
                    "UPDATE main.lessons SET page=?1, anchor=?2, position=?3, completed=?4, opened_at=?5, duration=max(duration, ?6) WHERE id=?7",
                    params![o.page, o.anchor, o.position, o.completed, o.opened_at, o.duration, id],
                )?;
                report.progress += 1;
            }
            // texte recalé ou ponctué là-bas (« Recaler la lanterne », « Ponctuer le texte »)
            if o.timing_v > timing_v && o.timings.is_some() {
                tx.execute(
                    "UPDATE main.lessons SET text=?1, timings=?2, timing_v=?3, word_count=?4 WHERE id=?5",
                    params![o.text, o.timings, o.timing_v, o.word_count, id],
                )?;
            }
            continue;
        }
        // supprimée ici après sa dernière lecture là-bas
        if gone_here.get(&o.uid).is_some_and(|&at| at >= touched(o.opened_at, o.created_at)) {
            continue;
        }
        tx.execute(
            "INSERT INTO main.lessons(lang,title,collection,kind,source,text,media_path,timings,hue,word_count,page,completed,
                created_at,opened_at,video_path,ext_id,position,anchor,duration,cover_path,timing_v,uid)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            params![
                o.lang,
                o.title,
                o.collection,
                o.kind,
                o.source,
                o.text,
                bring(&o.media_path),
                o.timings,
                o.hue,
                o.word_count,
                o.page,
                o.completed,
                o.created_at,
                o.opened_at,
                bring(&o.video_path),
                o.ext_id,
                o.position,
                o.anchor,
                o.duration,
                bring(&o.cover_path),
                o.timing_v,
                o.uid
            ],
        )?;
        mapped.insert(o.id, tx.last_insert_rowid());
        report.lessons += 1;
    }
    // supprimées là-bas après leur dernière lecture ici
    let mut files = Vec::new();
    for (uid, &(id, opened, created, _)) in &here {
        if gone_there.get(uid).is_some_and(|&at| at > touched(opened, created)) {
            files.extend(db::lesson_delete(&tx, id)?);
            report.removed += 1;
        }
    }

    // ---------- playlists et livres ----------
    let mut lists: HashMap<(String, String, String), i64> = HashMap::new();
    {
        let mut st = tx.prepare("SELECT id, lang, kind, name FROM main.playlists")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))? {
            let (id, lang, kind, name) = r?;
            lists.insert((lang, kind, name), id);
        }
    }
    let theirs: Vec<(i64, String, String, String, i64, i64)> = tx
        .prepare("SELECT id, lang, kind, name, looping, created_at FROM o.playlists")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (pid, lang, kind, name, looping, created) in theirs {
        let items: Vec<i64> = tx
            .prepare("SELECT lesson_id FROM o.playlist_items WHERE playlist_id=?1 ORDER BY position")?
            .query_map([pid], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?
            .into_iter()
            .filter_map(|id| mapped.get(&id).copied())
            .collect();
        if items.is_empty() {
            continue;
        }
        let key = (lang.clone(), kind.clone(), name.clone());
        let local = match lists.get(&key) {
            Some(&id) => id,
            None => {
                tx.execute(
                    "INSERT INTO main.playlists(lang,name,current_id,created_at,kind,looping) VALUES(?1,?2,NULL,?3,?4,?5)",
                    params![lang, name, created, kind, looping],
                )?;
                let id = tx.last_insert_rowid();
                lists.insert(key, id);
                id
            }
        };
        let present: HashSet<i64> =
            tx.prepare("SELECT lesson_id FROM main.playlist_items WHERE playlist_id=?1")?.query_map([local], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut next: i64 = tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM main.playlist_items WHERE playlist_id=?1", [local], |r| r.get(0))?;
        for lesson in items {
            if !present.contains(&lesson) {
                tx.execute("INSERT OR IGNORE INTO main.playlist_items(playlist_id, lesson_id, position) VALUES(?1,?2,?3)", params![local, lesson, next])?;
                next += 1;
            }
        }
    }

    // ---------- traces de suppression : les deux appareils les gardent ----------
    tx.execute(
        "INSERT INTO main.tombstones(kind,key,at) SELECT kind,key,at FROM o.tombstones WHERE 1
         ON CONFLICT(kind,key) DO UPDATE SET at=max(tombstones.at, excluded.at)",
        [],
    )?;
    files.sort();
    files.dedup();
    files.retain(|f| !db::file_used(&tx, f));
    tx.commit()?;
    report.known = known_total(live)? - known_before;
    Ok((report, files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    fn base(name: &str) -> (std::path::PathBuf, Connection) {
        let p = std::env::temp_dir().join(format!("lumen-sync-{name}-{}.db", std::process::id()));
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", p.display()));
        }
        let c = db::open(&p).unwrap();
        (p, c)
    }

    fn term(c: &Connection, lang: &str, t: &str, status: i64, at: i64) {
        c.execute(
            "INSERT INTO terms(lang,term,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)
             ON CONFLICT(lang,term) DO UPDATE SET status=excluded.status, updated_at=excluded.updated_at",
            params![lang, t, status, at],
        )
        .unwrap();
    }

    fn lesson(c: &Connection, title: &str, created: i64, opened: Option<i64>, page: i64) -> i64 {
        c.execute(
            "INSERT INTO lessons(lang,title,text,created_at,opened_at,page,media_path,uid) VALUES('it',?1,'Il faro è alto.',?2,?3,?4,?5,?6)",
            params![title, created, opened, page, format!("/Users/ulysse/media/{title}.m4a"), db::lesson_uid("it", created, title, None)],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn two_devices_meet_halfway() {
        let (mac_p, mut mac) = base("mac");
        let (phone_p, mut phone) = base("phone");
        // le même point de départ (l'iPhone a restauré la sauvegarde du Mac)
        for c in [&mac, &phone] {
            term(c, "it", "faro", 1, 100);
            term(c, "it", "alto", 2, 100);
            term(c, "it", "mare", 1, 100);
            lesson(c, "Il faro", 50, Some(100), 0);
            lesson(c, "Il mare", 60, Some(90), 0);
            c.execute("INSERT INTO activity(day,lang,words_read,learn_secs) VALUES('2026-10-17','it',300,600)", []).unwrap();
        }
        // sur le Mac : « faro » appris, une leçon nouvelle, « Il mare » supprimée
        term(&mac, "it", "faro", 4, 200);
        lesson(&mac, "La luna", 210, Some(220), 3);
        let mare = mac.query_row("SELECT id FROM lessons WHERE title='Il mare'", [], |r| r.get::<_, i64>(0)).unwrap();
        db::lesson_delete(&mac, mare).unwrap();
        mac.execute("UPDATE activity SET words_read=500 WHERE day='2026-10-17'", []).unwrap();
        // sur l'iPhone, plus tard : « alto » connu, « mare » remis à nouveau, « Il faro » lu jusqu'à la page 2
        term(&phone, "it", "alto", 4, 300);
        db::term_set(&phone, &serde_json::from_value(serde_json::json!({ "lang": "it", "term": "mare", "status": 0 })).unwrap()).unwrap();
        phone.execute("UPDATE lessons SET page=2, opened_at=?1 WHERE title='Il faro'", [db::now()]).unwrap();
        phone.execute("UPDATE activity SET learn_secs=900 WHERE day='2026-10-17'", []).unwrap();

        let media = |name: &str| Some(format!("/var/mobile/media/{name}"));
        // l'iPhone ajoute ce qu'a fait le Mac, puis le Mac ce qu'a fait l'iPhone
        let (r, _) = merge(&mut phone, &mac_p, &media).unwrap();
        assert_eq!((r.terms, r.lessons, r.removed), (1, 1, 1));
        assert_eq!(r.known, 1);
        let (r2, _) = merge(&mut mac, &phone_p, &media).unwrap();
        assert!(r2.terms >= 1);
        // les deux appareils arrivent au même état
        for c in [&mac, &phone] {
            let st = |t: &str| c.query_row("SELECT status FROM terms WHERE term=?1", [t], |r| r.get::<_, i64>(0)).optional().unwrap();
            assert_eq!((st("faro"), st("alto"), st("mare")), (Some(4), Some(4), None));
            let titles: Vec<String> = c.prepare("SELECT title FROM lessons ORDER BY title").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
            assert_eq!(titles, ["Il faro", "La luna"]);
            let page: i64 = c.query_row("SELECT page FROM lessons WHERE title='Il faro'", [], |r| r.get(0)).unwrap();
            assert_eq!(page, 2);
            let (w, l): (i64, i64) = c.query_row("SELECT words_read, learn_secs FROM activity", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!((w, l), (500, 900));
        }
        // le son de la leçon arrivée vient du dossier de la sauvegarde
        let p: String = phone.query_row("SELECT media_path FROM lessons WHERE title='La luna'", [], |r| r.get(0)).unwrap();
        assert_eq!(p, "/var/mobile/media/La luna.m4a");
        // fusionner de nouveau n'apporte plus rien
        let (r3, _) = merge(&mut phone, &mac_p, &media).unwrap();
        assert!(r3.is_empty(), "{r3:?}");
        for p in [mac_p, phone_p] {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Sur de vraies données, en copie (rien n'est écrit ailleurs que dans un dossier jetable) :
    /// un appareil neuf reçoit une sauvegarde, puis une base reçoit sa propre sauvegarde.
    /// `LUMEN_SYNC_SNAPSHOT=…/Progression.lumen LUMEN_SYNC_DB=…/lumen.db cargo test --lib sync_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sync_live() {
        let snap = std::env::var("LUMEN_SYNC_SNAPSHOT").expect("LUMEN_SYNC_SNAPSHOT");
        let dir = std::env::temp_dir().join(format!("lumen-sync-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("autre.db");
        let mut dec = flate2::read::GzDecoder::new(std::fs::File::open(&snap).unwrap());
        std::io::copy(&mut dec, &mut std::fs::File::create(&other).unwrap()).unwrap();
        let o = db::open(&other).unwrap();
        let count = |c: &Connection, sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        let (ol, ot, ok) = (count(&o, "SELECT COUNT(*) FROM lessons"), count(&o, "SELECT COUNT(*) FROM terms"), count(&o, "SELECT COUNT(*) FROM terms WHERE status=4"));
        drop(o);
        println!("sauvegarde : {ol} leçons, {ot} mots dont {ok} connus");
        // un appareil neuf
        let fresh_p = dir.join("neuf.db");
        let mut fresh = db::open(&fresh_p).unwrap();
        let started = std::time::Instant::now();
        let (r, _) = merge(&mut fresh, &other, &|_| None).unwrap();
        println!("appareil neuf : {r:?} en {:.2} s", started.elapsed().as_secs_f64());
        assert_eq!(count(&fresh, "SELECT COUNT(*) FROM lessons"), ol);
        assert_eq!(count(&fresh, "SELECT COUNT(*) FROM terms"), ot);
        let (r2, _) = merge(&mut fresh, &other, &|_| None).unwrap();
        assert!(r2.is_empty(), "deuxième fusion : {r2:?}");
        // une base qui reçoit sa propre sauvegarde (copie)
        if let Ok(live) = std::env::var("LUMEN_SYNC_DB") {
            let copy = dir.join("copie.db");
            let src = Connection::open(&live).unwrap();
            src.execute("VACUUM INTO ?1", [copy.to_string_lossy()]).unwrap();
            drop(src);
            let mut c = db::open(&copy).unwrap();
            let before = (count(&c, "SELECT COUNT(*) FROM lessons"), count(&c, "SELECT COUNT(*) FROM terms"));
            let started = std::time::Instant::now();
            let (r, files) = merge(&mut c, &other, &|_| None).unwrap();
            println!("base actuelle : {r:?}, {} fichiers à effacer, en {:.2} s", files.len(), started.elapsed().as_secs_f64());
            let after = (count(&c, "SELECT COUNT(*) FROM lessons"), count(&c, "SELECT COUNT(*) FROM terms"));
            println!("leçons et mots : {before:?} → {after:?}");
            assert!(after.0 >= before.0 - r.removed as i64);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
