//! Dictionnaires hors ligne (Wiktionnaire, CC BY-SA), dans la langue de l'interface.
//! Définitions en français (Wiktionnaire français) : livrées compressées avec
//! l'application, décompressées au premier usage de chaque langue. Définitions en
//! anglais (Wiktionnaire anglais) : téléchargées à la demande depuis les versions
//! publiées de Lumen, pour ne pas alourdir l'application de 74 Mo.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::text::normalize;

const DICT_VERSION: &str = "1";

/// Langues des dictionnaires à définitions anglaises (pas d'anglais : un
/// anglophone ne l'étudie pas ; le français en plus).
pub const EN_LANGS: &[&str] = &["it", "es", "de", "pt", "ru", "fr"];
/// Les fichiers `<langue>.db.gz` (tools/build_dicts.py --en), publiés une fois
/// pour toutes dans cette version de `lumen-releases` (jamais marquée « latest »).
const EN_URL: &str = "https://github.com/LivingTwice/lumen-releases/releases/download/dictionaries-en-1";
const EN_VERSION: &str = "1";

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
    /// dictionnaire en cours de téléchargement (interface en anglais)
    pub pending: bool,
}

pub struct Dicts {
    resource_dir: PathBuf,
    data_dir: PathBuf,
    /// connexions ouvertes, par « langue des définitions:langue étudiée »
    open: Mutex<HashMap<String, Connection>>,
    /// dictionnaires anglais en cours de téléchargement
    fetching: Mutex<HashSet<String>>,
}

impl Dicts {
    pub fn new(resource_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self { resource_dir, data_dir, open: Mutex::new(HashMap::new()), fetching: Mutex::new(HashSet::new()) }
    }

    /// Dictionnaire prêt pour cette langue, dans la langue de l'interface.
    pub fn available(&self, lang: &str) -> bool {
        self.available_in(crate::i18n::native(), lang)
    }

    pub fn available_in(&self, native: &str, lang: &str) -> bool {
        if native == "en" {
            self.en_db(lang).exists()
        } else {
            self.resource_dir.join(format!("{lang}.db.gz")).exists()
        }
    }

    /// Langues qui ont un dictionnaire dans la langue de l'interface (prêt ou à télécharger).
    pub fn langs(&self) -> Vec<String> {
        if crate::i18n::en() {
            return EN_LANGS.iter().map(|s| s.to_string()).collect();
        }
        ["en", "es", "it", "de", "pt", "ru"].iter().filter(|l| self.available_in("fr", l)).map(|s| s.to_string()).collect()
    }

    /// Dictionnaire anglais à télécharger (interface en anglais, pas encore là).
    pub fn missing(&self, lang: &str) -> bool {
        crate::i18n::en() && EN_LANGS.contains(&lang) && !self.en_db(lang).exists()
    }

    fn en_db(&self, lang: &str) -> PathBuf {
        self.data_dir.join(format!("{lang}-en-v{EN_VERSION}.db"))
    }

    /// Télécharge et décompresse le dictionnaire anglais d'une langue (un seul
    /// téléchargement à la fois par langue ; reprise après une coupure).
    pub async fn fetch_en(&self, lang: &str) -> Result<()> {
        self.fetch_en_from(EN_URL, lang).await
    }

    async fn fetch_en_from(&self, base: &str, lang: &str) -> Result<()> {
        if !EN_LANGS.contains(&lang) || self.en_db(lang).exists() || !self.fetching.lock().insert(lang.to_string()) {
            return Ok(());
        }
        let res = self.fetch_en_inner(base, lang).await;
        self.fetching.lock().remove(lang);
        res
    }

    async fn fetch_en_inner(&self, base: &str, lang: &str) -> Result<()> {
        tokio::fs::create_dir_all(&self.data_dir).await?;
        let part = self.data_dir.join(format!("{lang}-en.db.gz.part"));
        let url = format!("{base}/{lang}.db.gz");
        crate::models::fetch_resumable(&url, &part, 0, 0, Arc::new(AtomicBool::new(false)), &mut |_: crate::models::DownloadEvent| {}).await?;
        // décompressé à côté, puis mis en place d'un coup : un fichier présent est complet
        let target = self.en_db(lang);
        let tmp = target.with_extension("tmp");
        let (src, out) = (part.clone(), tmp.clone());
        let unpacked = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut dec = GzDecoder::new(File::open(&src)?);
            let mut file = File::create(&out)?;
            std::io::copy(&mut dec, &mut file)?;
            Ok(())
        })
        .await?;
        let _ = std::fs::remove_file(&part);
        if let Err(e) = unpacked {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, &target)?;
        Ok(())
    }

    fn ensure(&self, native: &str, lang: &str) -> Result<PathBuf> {
        if native == "en" {
            let p = self.en_db(lang);
            return if p.exists() { Ok(p) } else { Err(anyhow!("dictionary not downloaded yet")) };
        }
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

    fn with_conn<T>(&self, native: &str, lang: &str, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let id = format!("{native}:{lang}");
        let mut map = self.open.lock();
        if !map.contains_key(&id) {
            let path = self.ensure(native, lang)?;
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            map.insert(id.clone(), conn);
        }
        f(map.get(&id).unwrap())
    }

    /// Prépare le dictionnaire en arrière-plan (évite l'attente au premier clic).
    pub fn warm(&self, lang: &str) {
        let _ = self.with_conn(crate::i18n::native(), lang, |_| Ok(()));
    }

    /// Recherche dans le dictionnaire de la langue de l'interface.
    pub fn lookup(&self, lang: &str, surface: &str) -> Result<DictResult> {
        self.lookup_in(crate::i18n::native(), lang, surface)
    }

    pub fn lookup_in(&self, native: &str, lang: &str, surface: &str) -> Result<DictResult> {
        let key = normalize(surface);
        if key.is_empty() {
            return Ok(DictResult::default());
        }
        self.with_conn(native, lang, |c| {
            let mut res = DictResult::default();
            res.entries = entries_by(c, "k", &key)?;
            let mut st = c.prepare_cached("SELECT lemma, note FROM forms WHERE k=?1 LIMIT 4")?;
            let forms: Vec<(String, String)> = st
                .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            // une forme décrite d'abord (« first-person singular imperfect indicative of andare »)
            let described = forms.iter().find(|(l, n)| normalize(l) != key && !n.is_empty());
            if let Some((lemma, note)) = described.or_else(|| forms.iter().find(|(l, _)| normalize(l) != key)).cloned() {
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

    /// Téléchargement réel d'un dictionnaire anglais (portugais, le plus léger), depuis
    /// les versions publiées ou un autre serveur :
    /// LUMEN_DICT_BASE=http://localhost:8765 cargo test --lib dict_download_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dict_download_live() {
        let base = std::env::var("LUMEN_DICT_BASE").unwrap_or_else(|_| EN_URL.to_string());
        let tmp = std::env::temp_dir().join(format!("lumen-dict-dl-{}", std::process::id()));
        let d = Dicts::new(PathBuf::from("/nonexistent"), tmp.clone());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let t = std::time::Instant::now();
        rt.block_on(d.fetch_en_from(&base, "pt")).unwrap();
        println!("téléchargé et décompressé en {:?}", t.elapsed());
        assert!(d.available_in("en", "pt") && !tmp.join("pt-en.db.gz.part").exists());
        let r = d.lookup_in("en", "pt", "farol").unwrap();
        println!("{r:?}");
        assert!(r.entries.iter().any(|e| e.glosses.iter().any(|g| g.contains("lighthouse"))));
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Dictionnaires anglais construits sur ce Mac (tools/build_dicts.py --en), s'ils sont là.
    fn en_dicts(tag: &str) -> Option<(Dicts, PathBuf)> {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/dicts-en");
        if !src.join("it.db.gz").exists() {
            return None;
        }
        let tmp = std::env::temp_dir().join(format!("lumen-dict-en-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        for lang in ["it", "fr", "ru"] {
            let mut dec = GzDecoder::new(File::open(src.join(format!("{lang}.db.gz"))).unwrap());
            let mut out = File::create(tmp.join(format!("{lang}-en-v{EN_VERSION}.db"))).unwrap();
            std::io::copy(&mut dec, &mut out).unwrap();
        }
        Some((Dicts::new(PathBuf::from("/nonexistent"), tmp.clone()), tmp))
    }

    #[test]
    fn lookup_english_dictionaries() {
        let Some((d, tmp)) = en_dicts("lookup") else { return };
        assert!(d.available_in("en", "it") && !d.available_in("en", "de") && !d.available_in("fr", "it"));
        let r = d.lookup_in("en", "it", "andavo").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("andare"));
        assert!(r.form_note.as_deref().unwrap().contains("imperfect indicative of andare"), "{r:?}");
        assert_eq!(r.entries[0].glosses[0], "to go");
        let r = d.lookup_in("en", "it", "faro").unwrap();
        assert!(r.entries.iter().any(|e| e.glosses.iter().any(|g| g.contains("lighthouse"))), "{r:?}");
        // russe : forme de base sans accent tonique, description sans translittération
        let r = d.lookup_in("en", "ru", "читала").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("читать"));
        assert!(!r.form_note.as_deref().unwrap_or("").contains("čit"), "{r:?}");
        let r = d.lookup_in("en", "fr", "phare").unwrap();
        assert!(r.entries.iter().any(|e| e.glosses.iter().any(|g| g.contains("lighthouse"))));
        let _ = std::fs::remove_dir_all(tmp);
    }
}
