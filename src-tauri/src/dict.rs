//! Dictionnaires hors ligne (Wiktionnaire, CC BY-SA), livrés compressés avec
//! l'application et décompressés au premier usage de chaque langue.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::text::normalize;

const DICT_VERSION: &str = "1";

#[derive(Serialize, Clone, Debug)]
pub struct DictEntry {
    pub word: String,
    pub pos: String,
    pub ipa: String,
    pub glosses: Vec<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct DictResult {
    pub entries: Vec<DictEntry>,
    /// forme de base quand le mot cherché est une forme fléchie
    pub lemma: Option<String>,
    /// description grammaticale de la forme (« Prétérit de choose. »)
    pub form_note: Option<String>,
}

pub struct Dicts {
    resource_dir: PathBuf,
    data_dir: PathBuf,
    open: Mutex<HashMap<String, Connection>>,
}

impl Dicts {
    pub fn new(resource_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self { resource_dir, data_dir, open: Mutex::new(HashMap::new()) }
    }

    pub fn available(&self, lang: &str) -> bool {
        self.resource_dir.join(format!("{lang}.db.gz")).exists()
    }

    fn ensure(&self, lang: &str) -> Result<PathBuf> {
        let target = self.data_dir.join(format!("{lang}-v{DICT_VERSION}.db"));
        if target.exists() {
            return Ok(target);
        }
        let src = self.resource_dir.join(format!("{lang}.db.gz"));
        if !src.exists() {
            return Err(anyhow!("pas de dictionnaire pour « {lang} »"));
        }
        std::fs::create_dir_all(&self.data_dir)?;
        let tmp = target.with_extension("tmp");
        {
            let mut dec = GzDecoder::new(File::open(&src)?);
            let mut out = File::create(&tmp)?;
            std::io::copy(&mut dec, &mut out)?;
        }
        std::fs::rename(&tmp, &target)?;
        Ok(target)
    }

    fn with_conn<T>(&self, lang: &str, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut map = self.open.lock();
        if !map.contains_key(lang) {
            let path = self.ensure(lang)?;
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            map.insert(lang.to_string(), conn);
        }
        f(map.get(lang).unwrap())
    }

    /// Prépare le dictionnaire en arrière-plan (évite l'attente au premier clic).
    pub fn warm(&self, lang: &str) {
        let _ = self.with_conn(lang, |_| Ok(()));
    }

    pub fn lookup(&self, lang: &str, surface: &str) -> Result<DictResult> {
        let key = normalize(surface);
        if key.is_empty() {
            return Ok(DictResult::default());
        }
        self.with_conn(lang, |c| {
            let mut res = DictResult::default();
            res.entries = entries_by(c, "k", &key)?;
            let mut st = c.prepare_cached("SELECT lemma, note FROM forms WHERE k=?1 LIMIT 4")?;
            let forms: Vec<(String, String)> = st
                .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            if let Some((lemma, note)) = forms.iter().find(|(l, _)| normalize(l) != key).cloned() {
                if res.entries.is_empty() || res.entries.iter().all(|e| e.pos.starts_with("Forme")) {
                    let mut lemma_entries = entries_by(c, "word", &lemma)?;
                    if lemma_entries.is_empty() {
                        lemma_entries = entries_by(c, "k", &normalize(&lemma))?;
                    }
                    res.entries.extend(lemma_entries);
                }
                res.lemma = Some(lemma);
                if !note.is_empty() {
                    res.form_note = Some(note);
                }
            }
            res.entries.truncate(4);
            Ok(res)
        })
    }
}

fn entries_by(c: &Connection, col: &str, val: &str) -> Result<Vec<DictEntry>> {
    let sql = format!("SELECT word,pos,ipa,gloss FROM entries WHERE {col}=?1 ORDER BY rank LIMIT 3");
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map([val], |r| {
        let g: String = r.get(3)?;
        Ok(DictEntry {
            word: r.get(0)?,
            pos: r.get(1)?,
            ipa: r.get(2)?,
            glosses: g.split('\u{241e}').map(|s| s.trim_end_matches('.').to_string()).collect(),
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn dict_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("dicts")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_real_dictionaries() {
        let res = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/dicts");
        let tmp = std::env::temp_dir().join(format!("lumen-dict-test-{}", std::process::id()));
        let d = Dicts::new(res, tmp.clone());
        let r = d.lookup("en", "chose").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("choose"));
        assert!(!r.entries.is_empty(), "choose doit avoir une entrée");
        let r = d.lookup("de", "Häuser").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("Haus"));
        let r = d.lookup("es", "faro").unwrap();
        assert_eq!(r.entries[0].glosses[0], "Phare");
        let r = d.lookup("ru", "дом").unwrap();
        assert!(!r.entries.is_empty());
        let r = d.lookup("it", "andavo").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("andare"));
        println!("{:?}", d.lookup("en", "chose").unwrap());
        let _ = std::fs::remove_dir_all(tmp);
    }
}
