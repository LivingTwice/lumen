//! Dictionnaires hors ligne (Wiktionnaire, JMdict), dans la langue de l'interface.
//! Définitions en français de l'anglais, de l'italien, de l'allemand, du portugais, du russe
//! et de l'espagnol : livrées compressées avec l'application, décompressées au premier usage.
//! Tous les autres (définitions en anglais, et en français pour les 24 autres langues) :
//! téléchargés à la demande depuis les versions publiées de Lumen, pour ne pas alourdir
//! l'application. Le format 2 (tools/build_dicts.py --v2) a une clé de recherche propre à
//! chaque langue et range à part les descriptions des formes fléchies.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::text::{normalize, normalize_for};

const DICT_VERSION: &str = "1";

/// Définitions en français livrées avec l'application.
const BUNDLED_FR: &[&str] = &["en", "es", "it", "de", "pt", "ru"];
/// Les 24 autres langues de Lumen : dictionnaires du format 2, en français et en anglais.
pub const V2_LANGS: &[&str] = &[
    "nl", "sv", "da", "fi", "et", "lv", "lt", "pl", "cs", "sk", "sl", "hr", "hu", "ro", "bg", "uk", "el", "tr", "ar", "hi", "id", "vi", "ko", "ja",
];

/// Dictionnaires téléchargés à la demande, publiés une fois pour toutes dans des versions
/// du dépôt `lumen` qui ne doivent jamais devenir « latest ». Les Lumen jusqu'à 0.7.0 les
/// lisent dans `lumen-releases`, qui garde les mêmes versions.
struct Remote {
    /// langue des définitions
    native: &'static str,
    /// version du dépôt lumen qui porte les fichiers
    tag: &'static str,
    /// numéro dans le nom du fichier décompressé : le changer fait retélécharger
    version: &'static str,
    /// fichiers nommés `<native>-<langue>.db.gz` (sinon `<langue>.db.gz`)
    prefixed: bool,
    langs: &'static [&'static str],
}

const REMOTE: &[Remote] = &[
    // définitions anglaises des six premières langues (pas d'anglais, le français en plus)
    Remote { native: "en", tag: "dictionaries-en-1", version: "1", prefixed: false, langs: &["it", "es", "de", "pt", "ru", "fr"] },
    Remote { native: "en", tag: "dictionaries-2", version: "2", prefixed: true, langs: V2_LANGS },
    Remote { native: "fr", tag: "dictionaries-2", version: "2", prefixed: true, langs: V2_LANGS },
];
const RELEASES: &str = "https://github.com/LivingTwice/lumen/releases/download";

fn remote(native: &str, lang: &str) -> Option<&'static Remote> {
    REMOTE.iter().find(|r| r.native == native && r.langs.contains(&lang))
}

impl Remote {
    fn file(&self, lang: &str) -> String {
        if self.prefixed {
            format!("{}-{lang}.db.gz", self.native)
        } else {
            format!("{lang}.db.gz")
        }
    }
}

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
    /// description grammaticale de la forme (« Prétérit de choose. », « génitif pluriel »)
    pub form_note: Option<String>,
    /// dictionnaire en cours de téléchargement
    pub pending: bool,
}

impl DictResult {
    fn found(&self) -> bool {
        !self.entries.is_empty()
    }
}

/// Clé de recherche d'un mot (miroir de `key2` dans tools/build_dicts.py) : celle du
/// vocabulaire, plus, en turc, le « İ » mis en minuscule sans son point en trop ; en
/// arabe, sans voyelles brèves ni hamza sur l'alif (les textes les écrivent rarement) ;
/// en lituanien, croate et slovène, sans les accents de ton des manuels (« niẽko », « kȕća »).
pub fn key(s: &str, lang: &str) -> String {
    let k = normalize_for(s, lang);
    match lang {
        "tr" => k.replace("i\u{307}", "i"),
        "lt" | "hr" | "sl" => strip_tones(&k, lang),
        "ar" => k
            .chars()
            .filter(|c| !matches!(c, '\u{64b}'..='\u{65f}' | '\u{670}' | '\u{640}'))
            .map(|c| if matches!(c, 'أ' | 'إ' | 'آ' | 'ٱ') { 'ا' } else { c })
            .collect(),
        _ => k,
    }
}

/// Accents de ton retirés : tonique lituanien, tons croates (« ć » reste) et slovènes.
fn strip_tones(s: &str, lang: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let tones: &[char] = match lang {
        "lt" => &['\u{300}', '\u{301}', '\u{303}'],
        "hr" => &['\u{300}', '\u{302}', '\u{304}', '\u{30f}', '\u{311}'],
        _ => &['\u{300}', '\u{301}', '\u{302}', '\u{304}', '\u{30f}', '\u{311}', '\u{323}', '\u{327}', '\u{328}'],
    };
    let mut base = ' ';
    let mut out = String::with_capacity(s.len());
    for c in s.nfd() {
        if is_combining_mark(c) {
            if tones.contains(&c) || (c == '\u{301}' && lang == "hr" && base != 'c') || (c == '\u{307}' && lang == "lt" && base == 'i') {
                continue;
            }
        } else {
            base = c;
        }
        out.push(c);
    }
    out.nfc().collect()
}

#[derive(Serialize, Clone, Debug)]
pub struct DictStatus {
    /// un dictionnaire existe pour cette langue, dans la langue de l'interface
    pub exists: bool,
    /// il est sur ce Mac
    pub ready: bool,
    /// il se télécharge
    pub downloading: bool,
    /// il est livré avec l'application
    pub bundled: bool,
}

struct Dict {
    conn: Connection,
    /// format 2 : clé propre à la langue, descriptions des formes dans `notes`
    v2: bool,
}

pub struct Dicts {
    resource_dir: PathBuf,
    data_dir: PathBuf,
    /// connexions ouvertes, par « langue des définitions:langue étudiée »
    open: Mutex<HashMap<String, Dict>>,
    /// dictionnaires en cours de téléchargement (« langue des définitions:langue étudiée »)
    fetching: Mutex<HashSet<String>>,
}

impl Dicts {
    pub fn new(resource_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self { resource_dir, data_dir, open: Mutex::new(HashMap::new()), fetching: Mutex::new(HashSet::new()) }
    }

    fn bundled(&self, native: &str, lang: &str) -> bool {
        native == "fr" && BUNDLED_FR.contains(&lang)
    }

    /// Dictionnaire prêt pour cette langue, dans la langue de l'interface.
    pub fn available(&self, lang: &str) -> bool {
        self.available_in(crate::i18n::native(), lang)
    }

    pub fn available_in(&self, native: &str, lang: &str) -> bool {
        if self.bundled(native, lang) {
            return self.resource_dir.join(format!("{lang}.db.gz")).exists();
        }
        remote(native, lang).is_some_and(|r| self.remote_db(r, lang).exists())
    }

    /// Langues qui ont un dictionnaire dans la langue de l'interface (prêt ou à télécharger).
    pub fn langs(&self) -> Vec<String> {
        let native = crate::i18n::native();
        crate::text::LANGS
            .iter()
            .filter(|l| if self.bundled(native, l) { self.available_in(native, l) } else { remote(native, l).is_some() })
            .map(|s| s.to_string())
            .collect()
    }

    /// État du dictionnaire d'une langue, dans la langue de l'interface.
    pub fn status(&self, lang: &str) -> DictStatus {
        let native = crate::i18n::native();
        let bundled = self.bundled(native, lang);
        let id = format!("{native}:{lang}");
        DictStatus {
            exists: if bundled { self.available_in(native, lang) } else { remote(native, lang).is_some() },
            ready: self.available_in(native, lang),
            downloading: self.fetching.lock().contains(&id),
            bundled,
        }
    }

    /// Dictionnaire à télécharger (pas encore là).
    pub fn missing(&self, lang: &str) -> bool {
        let native = crate::i18n::native();
        !self.bundled(native, lang) && remote(native, lang).is_some_and(|r| !self.remote_db(r, lang).exists())
    }

    fn remote_db(&self, r: &Remote, lang: &str) -> PathBuf {
        self.data_dir.join(format!("{lang}-{}-v{}.db", r.native, r.version))
    }

    /// Télécharge et décompresse le dictionnaire d'une langue, dans la langue de l'interface
    /// (un seul téléchargement à la fois par dictionnaire ; reprise après une coupure).
    pub async fn fetch(&self, lang: &str) -> Result<()> {
        self.fetch_from(crate::i18n::native(), lang, None).await
    }

    /// `base` : un autre serveur qui sert les fichiers à sa racine (tests).
    async fn fetch_from(&self, native: &str, lang: &str, base: Option<&str>) -> Result<()> {
        let Some(r) = remote(native, lang) else { return Ok(()) };
        let id = format!("{native}:{lang}");
        if self.remote_db(r, lang).exists() || !self.fetching.lock().insert(id.clone()) {
            return Ok(());
        }
        let url = match base {
            Some(b) => format!("{b}/{}", r.file(lang)),
            None => format!("{RELEASES}/{}/{}", r.tag, r.file(lang)),
        };
        let res = self.fetch_inner(&url, r, lang).await;
        self.fetching.lock().remove(&id);
        res
    }

    async fn fetch_inner(&self, url: &str, r: &Remote, lang: &str) -> Result<()> {
        tokio::fs::create_dir_all(&self.data_dir).await?;
        let part = self.data_dir.join(format!("{lang}-{}.db.gz.part", r.native));
        crate::models::fetch_resumable(url, &part, 0, 0, Arc::new(AtomicBool::new(false)), &mut |_: crate::models::DownloadEvent| {}).await?;
        // décompressé à côté, puis mis en place d'un coup : un fichier présent est complet
        let target = self.remote_db(r, lang);
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
        if !self.bundled(native, lang) {
            let r = remote(native, lang).ok_or_else(|| anyhow!("no dictionary for « {lang} »"))?;
            let p = self.remote_db(r, lang);
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

    fn with_dict<T>(&self, native: &str, lang: &str, f: impl FnOnce(&Dict) -> Result<T>) -> Result<T> {
        let id = format!("{native}:{lang}");
        let mut map = self.open.lock();
        if !map.contains_key(&id) {
            let path = self.ensure(native, lang)?;
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            let v2 = conn.query_row("SELECT value FROM meta WHERE key='format'", [], |r| r.get::<_, String>(0)).is_ok_and(|v| v == "2");
            map.insert(id.clone(), Dict { conn, v2 });
        }
        f(map.get(&id).unwrap())
    }

    /// Prépare le dictionnaire en arrière-plan (évite l'attente au premier clic).
    pub fn warm(&self, lang: &str) {
        let _ = self.with_dict(crate::i18n::native(), lang, |_| Ok(()));
    }

    /// Recherche dans le dictionnaire de la langue de l'interface.
    pub fn lookup(&self, lang: &str, surface: &str) -> Result<DictResult> {
        self.lookup_in(crate::i18n::native(), lang, surface)
    }

    pub fn lookup_in(&self, native: &str, lang: &str, surface: &str) -> Result<DictResult> {
        self.lookup_ctx(native, lang, surface, "")
    }

    /// Recherche d'un mot. `after`, la suite de la phrase, sert aux langues dont les mots
    /// s'étendent sur plusieurs jetons : le japonais (découpé caractère par caractère) et le
    /// vietnamien (syllabe par syllabe). Le plus long mot du dictionnaire qui commence au mot
    /// touché vient en premier, puis le mot lui-même (le kanji, la syllabe).
    pub fn lookup_ctx(&self, native: &str, lang: &str, surface: &str, after: &str) -> Result<DictResult> {
        let surface = surface.trim();
        if surface.is_empty() {
            return Ok(DictResult::default());
        }
        self.with_dict(native, lang, |d| {
            let mut long = DictResult::default();
            if d.v2 {
                for span in longer_spans(lang, surface, after) {
                    let r = d.find_with_variants(native, lang, &span)?;
                    if r.found() {
                        long = r;
                        break;
                    }
                }
            }
            let mut own = d.find_with_variants(native, lang, surface)?;
            if lang == "ja" && d.v2 && surface.chars().count() == 1 {
                // sens et lectures du kanji, juste après le premier mot
                let kanji = d.entries(&d.key(surface, lang), true)?;
                let mut words = std::mem::take(&mut own.entries);
                let rest = words.split_off(words.len().min(1));
                own.entries = [words, kanji, rest].concat();
            }
            if !long.found() {
                own.entries.truncate(4);
                return Ok(own);
            }
            let mut res = long;
            res.entries.truncate(2);
            if lang == "ja" {
                // le mot trouvé, le kanji touché, puis le reste
                let (kanji, words): (Vec<_>, Vec<_>) = own.entries.into_iter().partition(|e| e.pos == "Kanji");
                let more = res.entries.split_off(1);
                res.entries = [res.entries, kanji, more, words].concat();
            } else {
                for e in own.entries {
                    if !res.entries.iter().any(|x| x.word == e.word && x.pos == e.pos) {
                        res.entries.push(e);
                    }
                }
            }
            res.entries.truncate(4);
            Ok(res)
        })
    }
}

impl Dict {
    fn key(&self, s: &str, lang: &str) -> String {
        if self.v2 {
            key(s, lang)
        } else {
            normalize(s)
        }
    }

    /// Entrées d'une clé : les mots, ou (`kanji`) le kanji seul, rangé après eux.
    fn entries(&self, k: &str, kanji: bool) -> Result<Vec<DictEntry>> {
        if kanji {
            let mut st = self.conn.prepare_cached("SELECT word,pos,ipa,gloss FROM entries WHERE k=?1 AND pos='Kanji' LIMIT 1")?;
            let rows = st.query_map([k], entry_row)?;
            return Ok(rows.filter_map(|r| r.ok()).collect());
        }
        let mut e = entries_by(&self.conn, "k", k)?;
        if self.v2 {
            e.retain(|x| x.pos != "Kanji");
        }
        Ok(e)
    }

    /// Formes de base d'une forme fléchie, avec leur description.
    fn forms(&self, k: &str) -> Result<Vec<(String, String)>> {
        let sql = if self.v2 {
            "SELECT f.lemma, IFNULL(n.text, '') FROM forms f LEFT JOIN notes n ON n.id = f.note WHERE f.k = ?1 LIMIT 4"
        } else {
            "SELECT lemma, note FROM forms WHERE k = ?1 LIMIT 4"
        };
        let mut st = self.conn.prepare_cached(sql)?;
        let rows = st.query_map([k], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Le mot tel quel : ses entrées, et sa forme de base s'il est fléchi.
    fn find(&self, lang: &str, word: &str) -> Result<DictResult> {
        let key = self.key(word, lang);
        let mut res = DictResult::default();
        if key.is_empty() {
            return Ok(res);
        }
        res.entries = self.entries(&key, false)?;
        let mut forms: Vec<(String, String)> = self.forms(&key)?.into_iter().filter(|(l, _)| self.key(l, lang) != key).collect();
        // une forme décrite d'abord, et la plus simple : « الكتاب », singulier défini de كتاب
        // (le livre), avant le pluriel masculin défini de كاتب (les écrivains)
        forms.sort_by_key(|(_, n)| (n.is_empty(), n.split_whitespace().count()));
        if let Some((lemma, note)) = forms.first().cloned() {
            // « volt » (hongrois) : le volt, unité, puis « van », être, dont c'est le passé
            if res.entries.len() < 2 || res.entries.iter().all(|e| e.pos.starts_with("Forme")) {
                for (l, _) in forms.iter().take(2) {
                    let mut lemma_entries = entries_by(&self.conn, "word", l)?;
                    if lemma_entries.is_empty() {
                        lemma_entries = self.entries(&self.key(l, lang), false)?;
                    }
                    for e in lemma_entries {
                        if !res.entries.iter().any(|x| x.word == e.word && x.pos == e.pos) {
                            res.entries.push(e);
                        }
                    }
                }
            }
            res.lemma = Some(lemma);
            if !note.is_empty() {
                res.form_note = Some(note);
            }
        }
        Ok(res)
    }

    /// Le mot, puis ce qu'il devient sans ses particules (coréen), ses préfixes et pronoms
    /// attachés (arabe, indonésien), ou sa conjugaison (japonais).
    fn find_with_variants(&self, native: &str, lang: &str, word: &str) -> Result<DictResult> {
        let res = self.find(lang, word)?;
        // arabe sans voyelles : « والكتاب » est une forme de كاتب (et les écrivains), mais
        // d'abord et + le + كتاب (le livre) ; les deux lectures, la plus courante en premier
        if lang == "ar" && self.v2 && res.lemma.is_some() {
            for (stem, _) in variants(lang, word) {
                let r = self.find(lang, &stem)?;
                if r.found() {
                    // « الكتاب » mène encore aux écrivains : on retire aussi l'article
                    if r.entries[0].word == *res.lemma.as_ref().unwrap() {
                        continue;
                    }
                    let mut both = r;
                    both.lemma = both.lemma.or_else(|| Some(both.entries[0].word.clone()));
                    for e in res.entries {
                        if !both.entries.iter().any(|x| x.word == e.word && x.pos == e.pos) {
                            both.entries.push(e);
                        }
                    }
                    return Ok(both);
                }
            }
        }
        if res.found() || !self.v2 {
            return Ok(res);
        }
        if lang == "ja" {
            return self.find_ja(native, word);
        }
        for (stem, note) in variants(lang, word) {
            let mut r = self.find(lang, &stem)?;
            if r.found() {
                if r.lemma.is_none() && stem != word {
                    r.lemma = Some(r.entries[0].word.clone());
                }
                if r.form_note.is_none() {
                    r.form_note = note;
                }
                return Ok(r);
            }
        }
        Ok(res)
    }

    /// Japonais conjugué : la forme du dictionnaire, si sa nature correspond
    /// (見つけなかった → 見つける, verbe ichidan ; négatif · passé).
    fn find_ja(&self, native: &str, word: &str) -> Result<DictResult> {
        // une conjugaison finit toujours par un kana
        if !word.chars().last().is_some_and(is_kana) {
            return Ok(DictResult::default());
        }
        for (cand, class, chain) in ja::deinflect(word) {
            let mut entries: Vec<DictEntry> = self.entries(&key(&cand, "ja"), false)?.into_iter().filter(|e| ja::class_ok(&e.pos, class)).collect();
            // verbe en する : la nature est portée par le nom (勉強した → 勉強)
            if entries.is_empty() && class == ja::VS {
                if let Some(stem) = cand.strip_suffix("する").filter(|s| !s.is_empty()) {
                    entries = self.entries(&key(stem, "ja"), false)?.into_iter().filter(|e| ja::class_ok(&e.pos, class)).collect();
                }
            }
            if !entries.is_empty() {
                return Ok(DictResult { lemma: Some(cand), form_note: Some(ja::describe(&chain, native)), entries, pending: false });
            }
        }
        Ok(DictResult::default())
    }
}

// ---------- lemmes : les formes connues regroupées sous leur mot de base ----------

/// Descriptions de formes qu'on ne suit pas pour regrouper (fautes, graphies anciennes).
const DOUBTFUL: &[&str] = &[
    "misspelling", "alternative", "variant", "abbreviation", "archaic", "obsolete", "dated", "nonstandard", "pre-reform", "eye dialect",
    "variante", "orthographe", "abréviation", "graphie", "archaïque", "vieilli",
];

/// Les dictionnaires d'une langue présents sur ce Mac (dans les deux langues de
/// l'interface), ouverts à part : un long calcul ne bloque pas les recherches de mots.
pub struct Lemmatizer {
    dicts: Vec<Dict>,
    lang: String,
}

impl Dicts {
    /// `None` : aucun dictionnaire de cette langue n'est encore sur ce Mac.
    pub fn lemmatizer(&self, lang: &str) -> Option<Lemmatizer> {
        let ui = crate::i18n::native();
        let other = if ui == "fr" { "en" } else { "fr" };
        let mut dicts = Vec::new();
        for native in [ui, other] {
            let path = if self.bundled(native, lang) {
                self.ensure(native, lang).ok()
            } else {
                remote(native, lang).map(|r| self.remote_db(r, lang)).filter(|p| p.exists())
            };
            let Some(p) = path else { continue };
            let Ok(conn) = Connection::open_with_flags(&p, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { continue };
            let v2 = conn.query_row("SELECT value FROM meta WHERE key='format'", [], |r| r.get::<_, String>(0)).is_ok_and(|v| v == "2");
            dicts.push(Dict { conn, v2 });
        }
        (!dicts.is_empty()).then(|| Lemmatizer { dicts, lang: lang.to_string() })
    }
}

impl Dict {
    /// Le mot est une entrée à part entière (nom, verbe…), pas seulement une forme.
    fn headword(&self, k: &str) -> bool {
        let Ok(mut st) = self.conn.prepare_cached("SELECT pos FROM entries WHERE k=?1 LIMIT 8") else { return false };
        let Ok(rows) = st.query_map([k], |r| r.get::<_, String>(0)) else { return false };
        let found = rows.filter_map(|r| r.ok()).any(|pos| {
            let p = pos.to_lowercase();
            !p.starts_with("forme") && !p.starts_with("form") && p != "kanji"
        });
        found
    }
}

impl Lemmatizer {
    /// Clé du lemme d'un mot connu ; `None` : le mot est inconnu des dictionnaires
    /// (nom propre, mot rare, faute de frappe).
    pub fn lemma(&self, word: &str) -> Option<String> {
        if let Some(l) = self.lemma_of(word) {
            return Some(l);
        }
        // particules et affixes attachés (coréen, arabe, indonésien, négation soudée)
        variants(&self.lang, word).into_iter().find_map(|(stem, _)| self.lemma_of(&stem))
    }

    fn lemma_of(&self, word: &str) -> Option<String> {
        // une entrée à part entière est son propre lemme (« anno », l'année, pas « hanno »)
        if self.dicts.iter().any(|d| {
            let k = d.key(word, &self.lang);
            !k.is_empty() && d.headword(&k)
        }) {
            return Some(key(word, &self.lang));
        }
        for d in &self.dicts {
            let k = d.key(word, &self.lang);
            if k.is_empty() {
                continue;
            }
            let mut forms: Vec<(String, String)> = d
                .forms(&k)
                .unwrap_or_default()
                .into_iter()
                .filter(|(l, n)| d.key(l, &self.lang) != k && !DOUBTFUL.iter().any(|x| n.to_lowercase().contains(x)))
                .collect();
            // une forme décrite, et la plus simple, d'abord (comme pour l'affichage)
            forms.sort_by_key(|(_, n)| (n.is_empty(), n.split_whitespace().count()));
            if let Some((l, _)) = forms.first() {
                return Some(key(l, &self.lang));
            }
        }
        None
    }
}

fn entry_row(r: &rusqlite::Row) -> rusqlite::Result<DictEntry> {
    let g: String = r.get(3)?;
    Ok(DictEntry {
        word: r.get(0)?,
        pos: r.get(1)?,
        ipa: r.get(2)?,
        glosses: g.split('\u{241e}').map(|s| s.trim_end_matches('.').to_string()).collect(),
    })
}

fn entries_by(c: &Connection, col: &str, val: &str) -> Result<Vec<DictEntry>> {
    let sql = format!("SELECT word,pos,ipa,gloss FROM entries WHERE {col}=?1 ORDER BY rank LIMIT 3");
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map([val], entry_row)?;
    let mut out: Vec<DictEntry> = Vec::new();
    // une même entrée sous deux graphies n'apparaît qu'une fois
    for e in rows.filter_map(|r| r.ok()) {
        if !out.iter().any(|x| x.word == e.word && x.pos == e.pos && x.glosses == e.glosses) {
            out.push(e);
        }
    }
    Ok(out)
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309f}' | '\u{30a0}'..='\u{30ff}')
}

/// Débuts de plus en plus courts de « mot touché + suite de la phrase » (du plus long au
/// plus court, le mot seul exclu).
fn longer_spans(lang: &str, surface: &str, after: &str) -> Vec<String> {
    match lang {
        "ja" => {
            let stop = |c: &char| c.is_whitespace() || (!c.is_alphanumeric() && !is_kana(*c) && *c != 'ー' && *c != '々');
            let first: Vec<char> = surface.chars().collect();
            if first.iter().any(stop) {
                return vec![];
            }
            let text: Vec<char> = first.iter().copied().chain(after.chars().take_while(|c| !stop(c))).take(10).collect();
            (first.len() + 1..=text.len()).rev().map(|n| text[..n].iter().collect()).collect()
        }
        "vi" => {
            // trois syllabes de plus au plus, sans franchir une ponctuation
            let mut next: Vec<&str> = Vec::new();
            for w in after.split_whitespace().take(3) {
                let core = w.trim_end_matches(|c: char| !c.is_alphabetic());
                if core.is_empty() || !core.chars().all(char::is_alphabetic) {
                    break;
                }
                next.push(core);
                if core.len() < w.len() {
                    break;
                }
            }
            (1..=next.len()).rev().map(|n| format!("{surface} {}", next[..n].join(" "))).collect()
        }
        _ => vec![],
    }
}

/// Particules coréennes, des plus longues aux plus courtes (학교에서 → 학교).
const KO_PARTICLES: &[&str] = &[
    "에서부터", "으로부터", "에게서는", "이었습니다", "이었어요", "입니다", "이에요", "이었다", "에서는", "에서도", "에게는", "에게도", "한테서", "한테는",
    "으로는", "으로도", "으로서", "으로써", "이라도", "이라는", "이라고", "이지만", "까지는", "까지도", "부터는", "보다는", "로부터", "에게서", "였어요",
    "예요", "이야", "였다", "이다", "이고", "인데", "처럼", "에게", "한테", "께서", "에서", "까지", "부터", "보다", "마다", "밖에", "으로", "이나", "이랑",
    "만큼", "조차", "마저", "에는", "에도", "라도", "라는", "라고", "하고", "와", "과", "은", "는", "이", "가", "을", "를", "에", "의", "도", "로", "만",
    "나", "랑", "께", "요", "야",
];
/// Préfixes arabes attachés (et, alors, avec, comme, pour, l'article, futur).
const AR_PREFIXES: &[&str] = &["وبال", "وكال", "فبال", "وال", "فال", "بال", "كال", "ولل", "فلل", "لل", "ال", "وس", "و", "ف", "ب", "ك", "ل", "س"];
/// Pronoms arabes attachés en fin de mot.
const AR_SUFFIXES: &[&str] = &["كما", "هما", "كم", "كن", "هم", "هن", "ها", "نا", "ني", "ه", "ك", "ي"];
/// Particules indonésiennes attachées en fin de mot.
const ID_SUFFIXES: &[&str] = &["-nya", "nya", "lah", "kah", "pun", "ku", "mu"];

/// Formes sans les particules ou affixes attachés, de la plus probable à la moins probable,
/// avec la description de la décomposition.
fn variants(lang: &str, word: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    match lang {
        "ko" => {
            for p in KO_PARTICLES {
                if let Some(stem) = word.strip_suffix(p).filter(|s| !s.is_empty()) {
                    out.push((stem.to_string(), Some(format!("{stem} + {p}"))));
                    for q in KO_PARTICLES {
                        if let Some(s2) = stem.strip_suffix(q).filter(|s| !s.is_empty()) {
                            out.push((s2.to_string(), Some(format!("{s2} + {q}{p}"))));
                        }
                    }
                }
            }
        }
        "ar" => {
            let w = key(word, "ar");
            let mut cands: Vec<(usize, String)> = Vec::new();
            for p in std::iter::once(&"").chain(AR_PREFIXES) {
                let Some(rest) = w.strip_prefix(p) else { continue };
                for s in std::iter::once(&"").chain(AR_SUFFIXES) {
                    let Some(stem) = rest.strip_suffix(s) else { continue };
                    if (p.is_empty() && s.is_empty()) || stem.chars().count() < 2 {
                        continue;
                    }
                    let cost = p.chars().count() + s.chars().count();
                    cands.push((cost, stem.to_string()));
                    // مدرستي → مدرسة : le tā' marbūṭa redevient visible sans le pronom
                    if !s.is_empty() {
                        if let Some(t) = stem.strip_suffix('ت') {
                            cands.push((cost, format!("{t}ة")));
                        }
                    }
                }
            }
            cands.sort_by_key(|c| c.0);
            out.extend(cands.into_iter().map(|(_, s)| (s, None)));
        }
        "id" => {
            for s in ID_SUFFIXES {
                if let Some(stem) = word.strip_suffix(s).filter(|x| x.chars().count() > 2) {
                    out.push((stem.to_string(), Some(format!("{stem} + -{}", s.trim_start_matches('-')))));
                }
            }
        }
        // négation soudée au verbe : « negaliu » (lituanien), « nevím » (tchèque)
        "lt" | "lv" | "cs" | "sk" => {
            let lower = word.to_lowercase();
            if let Some(stem) = lower.strip_prefix("ne").filter(|s| s.chars().count() > 2) {
                out.push((stem.to_string(), Some(format!("ne + {stem}"))));
            }
        }
        "tr" if word.contains(['I', 'İ']) => {
            // le « I » turc est un « ı » en minuscule
            out.push((word.replace('I', "ı").replace('İ', "i"), None));
        }
        _ => {}
    }
    let mut seen = HashSet::new();
    out.retain(|(s, _)| seen.insert(s.clone()));
    out
}

/// Conjugaison japonaise : règles de déflexion (forme conjuguée → forme du dictionnaire).
mod ja {
    use super::OnceLock;

    pub const V1: u8 = 1; // verbe ichidan
    pub const V5: u8 = 2; // verbe godan
    pub const VS: u8 = 4; // verbe en する
    pub const VK: u8 = 8; // 来る
    pub const ADJ: u8 = 16; // adjectif en い (et ない, たい, qui se conjuguent comme lui)
    const TE: u8 = 32; // forme en て (étape)
    const PAST: u8 = 64; // forme en た (étape)

    /// Noms des transformations, en anglais et en français.
    const NAMES: &[(&str, &str)] = &[
        ("polite", "poli"),
        ("past", "passé"),
        ("negative", "négatif"),
        ("te-form", "forme en -te"),
        ("want to (-tai)", "désir (-tai)"),
        ("potential", "potentiel"),
        ("passive", "passif"),
        ("potential or passive", "potentiel ou passif"),
        ("causative", "causatif"),
        ("volitional", "volitif"),
        ("conditional (-ba)", "conditionnel (-ba)"),
        ("imperative", "impératif"),
        ("progressive (-te iru)", "progressif (-te iru)"),
        ("completion (-te shimau)", "achèvement (-te shimau)"),
        ("in advance (-te oku)", "à l’avance (-te oku)"),
        ("result (-te aru)", "résultat (-te aru)"),
        ("try (-te miru)", "essai (-te miru)"),
        ("favour (-te kureru, -te morau, -te ageru)", "service (-te kureru, -te morau, -te ageru)"),
        ("request (-te kudasai)", "demande (-te kudasai)"),
        ("even if (-te mo)", "même si (-te mo)"),
        ("conditional (-tara)", "conditionnel (-tara)"),
        ("listing (-tari)", "énumération (-tari)"),
        ("adverbial (-ku)", "adverbial (-ku)"),
        ("noun (-sa)", "nom (-sa)"),
        ("seems (-sou)", "air de (-sou)"),
        ("too much (-sugiru)", "excès (-sugiru)"),
        ("while (-nagara)", "simultané (-nagara)"),
        ("command (-nasai)", "ordre (-nasai)"),
        ("direction (-te iku, -te kuru)", "direction (-te iku, -te kuru)"),
        ("-te wa", "-te wa"),
    ];
    const POLITE: u8 = 0;
    const PASTN: u8 = 1;
    const NEG: u8 = 2;
    const TEN: u8 = 3;
    const TAI: u8 = 4;
    const POT: u8 = 5;
    const PASS: u8 = 6;
    const POTPASS: u8 = 7;
    const CAUS: u8 = 8;
    const VOL: u8 = 9;
    const COND: u8 = 10;
    const IMP: u8 = 11;
    const PROG: u8 = 12;
    const SHIMAU: u8 = 13;
    const OKU: u8 = 14;
    const ARU: u8 = 15;
    const MIRU: u8 = 16;
    const FAVOUR: u8 = 17;
    const KUDASAI: u8 = 18;
    const TEMO: u8 = 19;
    const TARA: u8 = 20;
    const TARI: u8 = 21;
    const ADV: u8 = 22;
    const SA: u8 = 23;
    const SOU: u8 = 24;
    const SUGIRU: u8 = 25;
    const NAGARA: u8 = 26;
    const NASAI: u8 = 27;
    const IKU: u8 = 28;
    const TEWA: u8 = 29;

    struct Rule {
        from: String,
        to: String,
        /// natures que la forme conjuguée peut avoir dans une chaîne (0 : seulement le mot lu)
        cond: u8,
        out: u8,
        name: u8,
    }

    /// Godan : terminaison, puis radicaux en a, i, e, o, formes en て et en た.
    const GODAN: &[[&str; 7]] = &[
        ["う", "わ", "い", "え", "お", "って", "った"],
        ["く", "か", "き", "け", "こ", "いて", "いた"],
        ["ぐ", "が", "ぎ", "げ", "ご", "いで", "いだ"],
        ["す", "さ", "し", "せ", "そ", "して", "した"],
        ["つ", "た", "ち", "て", "と", "って", "った"],
        ["ぬ", "な", "に", "ね", "の", "んで", "んだ"],
        ["ぶ", "ば", "び", "べ", "ぼ", "んで", "んだ"],
        ["む", "ま", "み", "め", "も", "んで", "んだ"],
        ["る", "ら", "り", "れ", "ろ", "って", "った"],
    ];

    fn rules() -> &'static [Rule] {
        static R: OnceLock<Vec<Rule>> = OnceLock::new();
        R.get_or_init(build)
    }

    fn build() -> Vec<Rule> {
        let mut r: Vec<Rule> = Vec::new();
        let mut add = |from: &str, to: &str, cond: u8, out: u8, name: u8| r.push(Rule { from: from.into(), to: to.into(), cond, out, name });
        // 行く : って, った (avant les godan en う, つ, る)
        for (te, ta, base) in [("行って", "行った", "行く"), ("いって", "いった", "いく")] {
            add(te, base, TE, V5, TEN);
            add(ta, base, PAST, V5, PASTN);
        }
        // radical en i (ます, たい, ながら, なさい, そう)
        let masu: &[(&str, u8, u8)] = &[
            ("ます", 0, POLITE),
            ("ました", 0, POLITE),
            ("ません", 0, POLITE),
            ("ませんでした", 0, POLITE),
            ("ましょう", 0, POLITE),
            ("まして", 0, POLITE),
            ("たい", ADJ, TAI),
            ("ながら", 0, NAGARA),
            ("なさい", 0, NASAI),
            ("そう", 0, SOU),
        ];
        for &(s, cond, name) in masu {
            add(s, "る", cond, V1, name);
            for g in GODAN {
                add(&format!("{}{s}", g[2]), g[0], cond, V5, name);
            }
            add(&format!("し{s}"), "する", cond, VS, name);
            add(&format!("き{s}"), "くる", cond, VK, name);
            add(&format!("来{s}"), "来る", cond, VK, name);
        }
        // radical en a : négatif, passif, causatif
        for (s, cond, name) in [("ない", ADJ, NEG), ("ず", 0, NEG), ("ずに", 0, NEG), ("ぬ", 0, NEG)] {
            add(s, "る", cond, V1, name);
            for g in GODAN {
                add(&format!("{}{s}", g[1]), g[0], cond, V5, name);
            }
            add(&format!("し{s}"), "する", cond, VS, name);
            add(&format!("せ{s}"), "する", cond, VS, name);
            add(&format!("こ{s}"), "くる", cond, VK, name);
            add(&format!("来{s}"), "来る", cond, VK, name);
        }
        for g in GODAN {
            add(&format!("{}れる", g[1]), g[0], V1, V5, PASS);
            add(&format!("{}せる", g[1]), g[0], V1, V5, CAUS);
            add(&format!("{}せられる", g[1]), g[0], V1, V5, CAUS);
            // radical en e : potentiel, conditionnel, impératif ; en o : volitif
            add(&format!("{}る", g[3]), g[0], V1, V5, POT);
            add(&format!("{}ば", g[3]), g[0], 0, V5, COND);
            add(g[3], g[0], 0, V5, IMP);
            add(&format!("{}う", g[4]), g[0], 0, V5, VOL);
            add(g[5], g[0], TE, V5, TEN);
            add(g[6], g[0], PAST, V5, PASTN);
        }
        for (s, to, cond, out, name) in [
            ("られる", "る", V1, V1, POTPASS),
            ("れる", "る", V1, V1, POT),
            ("させる", "る", V1, V1, CAUS),
            ("させられる", "る", V1, V1, CAUS),
            ("れば", "る", 0, V1, COND),
            ("ろ", "る", 0, V1, IMP),
            ("よ", "る", 0, V1, IMP),
            ("よう", "る", 0, V1, VOL),
            ("て", "る", TE, V1, TEN),
            ("た", "る", PAST, V1, PASTN),
            ("される", "する", V1, VS, PASS),
            ("させる", "する", V1, VS, CAUS),
            ("できる", "する", V1, VS, POT),
            ("すれば", "する", 0, VS, COND),
            ("しろ", "する", 0, VS, IMP),
            ("せよ", "する", 0, VS, IMP),
            ("しよう", "する", 0, VS, VOL),
            ("して", "する", TE, VS, TEN),
            ("した", "する", PAST, VS, PASTN),
            ("こられる", "くる", V1, VK, POTPASS),
            ("来られる", "来る", V1, VK, POTPASS),
            ("こさせる", "くる", V1, VK, CAUS),
            ("来させる", "来る", V1, VK, CAUS),
            ("くれば", "くる", 0, VK, COND),
            ("来れば", "来る", 0, VK, COND),
            ("こい", "くる", 0, VK, IMP),
            ("来い", "来る", 0, VK, IMP),
            ("こよう", "くる", 0, VK, VOL),
            ("来よう", "来る", 0, VK, VOL),
            ("きて", "くる", TE, VK, TEN),
            ("来て", "来る", TE, VK, TEN),
            ("きた", "くる", PAST, VK, PASTN),
            ("来た", "来る", PAST, VK, PASTN),
            // adjectifs en い (et ない, たい)
            ("かった", "い", PAST, ADJ, PASTN),
            ("くて", "い", TE, ADJ, TEN),
            ("くない", "い", ADJ, ADJ, NEG),
            ("ければ", "い", 0, ADJ, COND),
            ("かろう", "い", 0, ADJ, VOL),
            ("く", "い", 0, ADJ, ADV),
            ("さ", "い", 0, ADJ, SA),
            ("そう", "い", 0, ADJ, SOU),
            ("すぎる", "い", V1, ADJ, SUGIRU),
            // auxiliaires après て : la forme en て, puis le verbe
            ("ている", "て", V1, TE, PROG),
            ("でいる", "で", V1, TE, PROG),
            ("てる", "て", V1, TE, PROG),
            ("でる", "で", V1, TE, PROG),
            ("てしまう", "て", V5, TE, SHIMAU),
            ("でしまう", "で", V5, TE, SHIMAU),
            ("ちゃう", "て", V5, TE, SHIMAU),
            ("じゃう", "で", V5, TE, SHIMAU),
            ("ておく", "て", V5, TE, OKU),
            ("でおく", "で", V5, TE, OKU),
            ("とく", "て", V5, TE, OKU),
            ("どく", "で", V5, TE, OKU),
            ("てある", "て", V5, TE, ARU),
            ("である", "で", V5, TE, ARU),
            ("てみる", "て", V1, TE, MIRU),
            ("でみる", "で", V1, TE, MIRU),
            ("てくれる", "て", V1, TE, FAVOUR),
            ("でくれる", "で", V1, TE, FAVOUR),
            ("てあげる", "て", V1, TE, FAVOUR),
            ("であげる", "で", V1, TE, FAVOUR),
            ("てもらう", "て", V5, TE, FAVOUR),
            ("でもらう", "で", V5, TE, FAVOUR),
            ("ていく", "て", V5, TE, IKU),
            ("でいく", "で", V5, TE, IKU),
            ("てくる", "て", VK, TE, IKU),
            ("でくる", "で", VK, TE, IKU),
            ("てください", "て", 0, TE, KUDASAI),
            ("でください", "で", 0, TE, KUDASAI),
            ("ても", "て", 0, TE, TEMO),
            ("でも", "で", 0, TE, TEMO),
            ("ては", "て", 0, TE, TEWA),
            ("では", "で", 0, TE, TEWA),
            ("たら", "た", 0, PAST, TARA),
            ("だら", "だ", 0, PAST, TARA),
            ("たり", "た", 0, PAST, TARI),
            ("だり", "だ", 0, PAST, TARI),
        ] {
            add(s, to, cond, out, name);
        }
        r
    }

    /// Formes du dictionnaire possibles, des plus proches aux plus lointaines : (mot,
    /// nature attendue, transformations depuis le mot lu).
    pub fn deinflect(word: &str) -> Vec<(String, u8, Vec<u8>)> {
        let mut queue: Vec<(String, u8, Vec<u8>)> = vec![(word.to_string(), 0, vec![])];
        let mut seen = std::collections::HashSet::new();
        let mut i = 0;
        while i < queue.len() && queue.len() < 400 {
            let (term, t, chain) = queue[i].clone();
            i += 1;
            if chain.len() >= 6 {
                continue;
            }
            for r in rules() {
                if t != 0 && t & r.cond == 0 {
                    continue;
                }
                let Some(stem) = term.strip_suffix(r.from.as_str()) else { continue };
                let next = format!("{stem}{}", r.to);
                if next.is_empty() || next == term || !seen.insert((next.clone(), r.out)) {
                    continue;
                }
                let mut c = chain.clone();
                c.push(r.name);
                queue.push((next, r.out, c));
            }
        }
        queue.into_iter().skip(1).filter(|(_, t, _)| *t & (TE | PAST) == 0).collect()
    }

    /// La nature d'une entrée de JMdict correspond à la classe attendue.
    pub fn class_ok(pos: &str, class: u8) -> bool {
        let mark = match class {
            V1 => "(ichidan)",
            V5 => "(godan)",
            VS => "(suru)",
            VK => "(kuru)",
            ADJ => "(-i)",
            _ => return false,
        };
        pos.contains(mark)
    }

    /// « potentiel ou passif · négatif · passé » (de la forme du dictionnaire vers le mot lu).
    pub fn describe(chain: &[u8], native: &str) -> String {
        let mut names: Vec<&str> = chain.iter().rev().map(|&n| if native == "en" { NAMES[n as usize].0 } else { NAMES[n as usize].1 }).collect();
        names.dedup();
        names.join(" · ")
    }
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
        let r = d.lookup_in("fr", "en", "chose").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("choose"));
        assert!(!r.entries.is_empty(), "choose doit avoir une entrée");
        let r = d.lookup_in("fr", "de", "Häuser").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("Haus"));
        let r = d.lookup_in("fr", "es", "faro").unwrap();
        assert_eq!(r.entries[0].glosses[0], "Phare");
        let r = d.lookup_in("fr", "ru", "дом").unwrap();
        assert!(!r.entries.is_empty());
        let r = d.lookup_in("fr", "it", "andavo").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("andare"));
        println!("{:?}", d.lookup_in("fr", "en", "chose").unwrap());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn catalogue() {
        // toutes les langues de Lumen ont un dictionnaire dans les deux langues de l'interface
        for lang in crate::text::LANGS {
            let fr = BUNDLED_FR.contains(lang) || remote("fr", lang).is_some();
            let en = remote("en", lang).is_some();
            assert!(fr || *lang == "fr", "{lang} : pas de dictionnaire en français");
            assert!(en || *lang == "en", "{lang} : pas de dictionnaire en anglais");
        }
        assert_eq!(remote("fr", "hu").unwrap().file("hu"), "fr-hu.db.gz");
        assert_eq!(remote("en", "it").unwrap().file("it"), "it.db.gz");
    }

    #[test]
    fn keys_per_language() {
        assert_eq!(key("İstanbul", "tr"), "istanbul");
        assert_eq!(key("Kitabı", "tr"), "kitabı");
        assert_eq!(key("الْكِتَابُ", "ar"), "الكتاب");
        assert_eq!(key("أحمد", "ar"), "احمد");
        assert_eq!(key("Πότε", "el"), "πότε");
        assert_eq!(key("Mà", "vi"), "mà");
        assert_eq!(key("до\u{301}м", "ru"), "дом");
        // accents des manuels retirés, lettres de l'alphabet gardées
        assert_eq!(key("niẽko", "lt"), "nieko");
        assert_eq!(key("vi\u{307}\u{300}sko", "lt"), "visko");
        assert_eq!(key("gėlė", "lt"), "gėlė");
        assert_eq!(key("kȕća", "hr"), "kuća");
        assert_eq!(key("človȅk", "sl"), "človek");
    }

    #[test]
    fn attached_particles() {
        let v: Vec<String> = variants("ko", "학교에서").into_iter().map(|x| x.0).collect();
        assert_eq!(v[0], "학교");
        let v: Vec<String> = variants("ar", "والكتاب").into_iter().map(|x| x.0).collect();
        assert!(v.contains(&"كتاب".to_string()), "{v:?}");
        let v: Vec<String> = variants("ar", "مدرستي").into_iter().map(|x| x.0).collect();
        assert!(v.contains(&"مدرسة".to_string()), "{v:?}");
        let v: Vec<String> = variants("id", "bukunya").into_iter().map(|x| x.0).collect();
        assert_eq!(v[0], "buku");
        assert_eq!(variants("tr", "KITABI")[0].0, "KıTABı");
    }

    #[test]
    fn japanese_deinflection() {
        let has = |w: &str, base: &str, class: u8| ja::deinflect(w).iter().any(|(c, t, _)| c == base && *t == class);
        assert!(has("見つけなかった", "見つける", ja::V1));
        assert!(has("見つけられません", "見つける", ja::V1));
        assert!(has("書いている", "書く", ja::V5));
        assert!(has("読んじゃった", "読む", ja::V5));
        assert!(has("行った", "行く", ja::V5));
        assert!(has("勉強した", "勉強する", ja::VS));
        assert!(has("高くなかった", "高い", ja::ADJ));
        assert!(has("食べたかった", "食べる", ja::V1));
        assert!(has("来ます", "来る", ja::VK));
        // 行った : 行く d'abord (行う est bien plus rare)
        let first_v5 = ja::deinflect("行った").into_iter().find(|(_, t, _)| *t == ja::V5).unwrap();
        assert_eq!(first_v5.0, "行く");
        let (_, _, chain) = ja::deinflect("見つけなかった").into_iter().find(|(c, _, _)| c == "見つける").unwrap();
        assert_eq!(ja::describe(&chain, "en"), "negative · past");
        assert_eq!(ja::describe(&chain, "fr"), "négatif · passé");
    }

    #[test]
    fn spans_follow_the_sentence() {
        assert_eq!(longer_spans("ja", "見", "つけた。それ"), ["見つけた", "見つけ", "見つ"]);
        assert_eq!(longer_spans("vi", "học", "sinh giỏi, nhưng"), ["học sinh giỏi", "học sinh"]);
        assert!(longer_spans("it", "casa", "mia").is_empty());
    }

    /// Les clés de tools/build_dicts.py (`--keys`) et celles de l'app doivent être identiques :
    /// LUMEN_KEYS=/chemin/cles.tsv cargo test --lib keys_match_builder -- --ignored --nocapture
    #[test]
    #[ignore]
    fn keys_match_builder() {
        let path = std::env::var("LUMEN_KEYS").expect("LUMEN_KEYS");
        let text = std::fs::read_to_string(path).unwrap();
        let (mut n, mut bad) = (0, Vec::new());
        for line in text.lines() {
            let mut cols = line.split('\t');
            let (Some(lang), Some(word), Some(k)) = (cols.next(), cols.next(), cols.next()) else { continue };
            n += 1;
            if key(word, lang) != k {
                bad.push(format!("{lang} « {word} » : Python « {k} », Rust « {} »", key(word, lang)));
            }
        }
        println!("{n} clés comparées, {} différences", bad.len());
        assert!(bad.is_empty(), "{}", bad[..bad.len().min(20)].join("\n"));
    }

    /// Téléchargement réel d'un dictionnaire (anglais, portugais : le plus léger), depuis
    /// les versions publiées ou un serveur qui sert les fichiers à sa racine :
    /// LUMEN_DICT_BASE=http://127.0.0.1:8765 cargo test --lib dict_download_live -- --ignored --nocapture
    /// (LUMEN_DICT=fr-hu pour un dictionnaire de la version 2)
    #[test]
    #[ignore]
    fn dict_download_live() {
        let base = std::env::var("LUMEN_DICT_BASE").ok();
        let which = std::env::var("LUMEN_DICT").unwrap_or_else(|_| "en-pt".into());
        let (native, lang) = which.split_once('-').unwrap();
        let tmp = std::env::temp_dir().join(format!("lumen-dict-dl-{}", std::process::id()));
        let d = Dicts::new(PathBuf::from("/nonexistent"), tmp.clone());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let t = std::time::Instant::now();
        rt.block_on(d.fetch_from(native, lang, base.as_deref())).unwrap();
        println!("téléchargé et décompressé en {:?}", t.elapsed());
        assert!(d.available_in(native, lang) && !tmp.join(format!("{lang}-{native}.db.gz.part")).exists());
        let word = match lang {
            "pt" => "farol",
            "hu" => "házak",
            "ja" => "家",
            _ => "a",
        };
        let r = d.lookup_in(native, lang, word).unwrap();
        println!("{r:?}");
        assert!(r.found());
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Dictionnaires construits sur ce Mac (tools/build_dicts.py), s'ils sont là : décompressés
    /// dans un dossier jetable sous le nom qu'ils ont une fois téléchargés.
    fn built(tag: &str, names: &[(&str, &str)]) -> Option<(Dicts, PathBuf)> {
        let tmp = std::env::temp_dir().join(format!("lumen-dict-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        for (native, lang) in names {
            let r = remote(native, lang).unwrap();
            let dir = if r.version == "1" { "resources/dicts-en" } else { "resources/dicts-v2" };
            let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir).join(r.file(lang));
            if !src.exists() {
                let _ = std::fs::remove_dir_all(&tmp);
                return None;
            }
            let mut dec = GzDecoder::new(File::open(src).unwrap());
            let mut out = File::create(tmp.join(format!("{lang}-{native}-v{}.db", r.version))).unwrap();
            std::io::copy(&mut dec, &mut out).unwrap();
        }
        Some((Dicts::new(PathBuf::from("/nonexistent"), tmp.clone()), tmp))
    }

    #[test]
    fn lookup_english_dictionaries() {
        let Some((d, tmp)) = built("en1", &[("en", "it"), ("en", "fr"), ("en", "ru")]) else { return };
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

    /// Format 2 : définitions en français et en anglais des 24 autres langues.
    #[test]
    fn lookup_v2_dictionaries() {
        let names = [("fr", "hu"), ("en", "hu"), ("fr", "ko"), ("en", "ar"), ("fr", "ja"), ("en", "ja"), ("fr", "vi"), ("fr", "el")];
        let Some((d, tmp)) = built("v2", &names) else { return };
        let gloss = |r: &DictResult, s: &str| r.entries.iter().any(|e| e.glosses.iter().any(|g| g.to_lowercase().contains(s)));
        // forme fléchie décrite, dans la langue de l'interface
        let r = d.lookup_in("fr", "hu", "házak").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("ház"));
        assert!(r.form_note.as_deref().unwrap_or("").contains("luriel"), "{r:?}");
        assert!(gloss(&r, "maison"), "{r:?}");
        let r = d.lookup_in("en", "hu", "házak").unwrap();
        assert!(gloss(&r, "house"), "{r:?}");
        // coréen : sans la particule
        let r = d.lookup_in("fr", "ko", "학교에서").unwrap();
        assert!(gloss(&r, "école"), "{r:?}");
        // arabe : sans voyelles brèves ni article
        let r = d.lookup_in("en", "ar", "والكتاب").unwrap();
        assert!(gloss(&r, "book"), "{r:?}");
        // japonais : le mot qui commence au caractère touché, conjugué, puis le kanji
        let r = d.lookup_ctx("fr", "ja", "見", "つけなかった。").unwrap();
        assert_eq!(r.lemma.as_deref(), Some("見つける"), "{r:?}");
        assert!(gloss(&r, "trouver"), "{r:?}");
        assert_eq!(r.entries[1].pos, "Kanji", "{r:?}");
        let r = d.lookup_ctx("en", "ja", "家", "に帰る").unwrap();
        assert!(gloss(&r, "house"), "{r:?}");
        // vietnamien : le mot de deux syllabes d'abord
        let r = d.lookup_ctx("fr", "vi", "học", "sinh giỏi").unwrap();
        assert_eq!(r.entries[0].word, "học sinh", "{r:?}");
        // grec : un texte en capitales n'a pas d'accents
        let r = d.lookup_in("fr", "el", "ΣΠΙΤΙ").unwrap();
        assert!(gloss(&r, "maison"), "{r:?}");
        let _ = std::fs::remove_dir_all(tmp);
    }
}
