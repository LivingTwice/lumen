//! Sauvegarde de la progression dans iCloud Drive, ou dans tout dossier
//! synchronisé choisi par l'utilisateur (Dropbox, Google Drive, clé USB…).
//! Rien ne passe par un serveur de Lumen : macOS envoie lui-même le dossier
//! dans le nuage de l'utilisateur, gratuitement dans la limite de son forfait.
//!
//! ```text
//! Lumen/
//!   <nom du Mac> (<clé>)/     une sauvegarde par Mac et par profil
//!     Progression.lumen       la base SQLite compressée, sans le cache
//!     Infos.json              appareil, date, mots et leçons, médias cités
//!     Historique/             une version par jour (14 jours)
//!   Médias/                   audio, vidéos et couvertures, communs à tous
//! ```
//!
//! La clé réunit le profil (`profile_id`, qui voyage avec la base : un Mac
//! restauré continue le même profil) et l'appareil : une installation neuve
//! n'écrase jamais la sauvegarde d'une autre.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db;
use crate::media::{self, ImportEvent};
use crate::state::AppState;

const ROOT_NAME: &str = "Lumen";
const MEDIA: &str = "Médias";
const HISTORY: &str = "Historique";
const SNAPSHOT: &str = "Progression.lumen";
const MANIFEST: &str = "Infos.json";
/// Forme des fichiers de sauvegarde (à augmenter si elle change).
const FORMAT: i64 = 1;
/// Versions quotidiennes gardées dans l'historique.
const HISTORY_DAYS: usize = 14;
/// Réglages qui ne quittent pas ce Mac : les clés LingQ et Gemini (promis :
/// « elle reste sur ce Mac ») et l'emplacement de la sauvegarde, propre à ce disque.
const LOCAL_ONLY: [&str; 3] = ["lingq_key", "gemini_key", "backup_dir"];
/// Réglages de ce Mac conservés quand on restaure une sauvegarde (la langue de
/// l'interface aussi : celle qu'on vient de choisir à l'accueil reste).
const KEEP_ON_RESTORE: [&str; 8] = ["lingq_key", "gemini_key", "backup_dir", "backup_on", "backup_audio", "backup_video", "backup_snooze", "ui_lang"];
/// Leçons d'accueil : un profil qui n'a qu'elles n'a encore rien à sauvegarder.
/// Leur collection dépend de la langue de l'interface au premier lancement.
const STARTER_COLLECTIONS: [&str; 2] = ["Pour commencer", "Getting started"];
/// Écart minimal entre deux sauvegardes automatiques (secondes).
const AUTO_EVERY: i64 = 10 * 60;

// ---------- état de la session ----------

/// Sauvegardes de cette session. Rien n'est écrit dans la base : une écriture
/// y compterait elle-même comme un changement à sauvegarder.
#[derive(Default)]
pub struct Tracker {
    running: AtomicBool,
    /// `total_changes` de la base à la dernière sauvegarde réussie
    seen: AtomicU64,
    /// dernière tentative (secondes), réussie ou non
    last_try: AtomicI64,
    /// la base a-t-elle été comparée à la dernière sauvegarde depuis le lancement ?
    checked: AtomicBool,
    /// dernière erreur, effacée par une sauvegarde réussie
    error: Mutex<Option<String>>,
}

/// Une seule sauvegarde (ou restauration) à la fois.
struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Tracker {
    fn begin(&self) -> Option<Running<'_>> {
        self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| Running(&self.running))
    }

    /// Attend la fin de la sauvegarde en cours.
    fn wait(&self) -> Running<'_> {
        loop {
            if let Some(g) = self.begin() {
                return g;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

// ---------- types échangés ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Counts {
    pub known: i64,
    pub learning: i64,
    pub phrases: i64,
    pub lessons: i64,
    pub langs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Manifest {
    pub format: i64,
    pub app_version: String,
    pub profile: String,
    pub device: String,
    pub device_name: String,
    pub saved_at: i64,
    /// taille de Progression.lumen
    pub size: u64,
    pub counts: Counts,
    /// fichiers de Médias/ dont cette version a besoin
    pub media: Vec<String>,
    pub media_size: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    /// réglage `backup_on` : "1" active, "0" coupée, "" pas encore choisie
    pub enabled: bool,
    pub decided: bool,
    /// dossier « Lumen » de la sauvegarde (aucun : ni iCloud Drive ni dossier choisi)
    pub dir: Option<String>,
    pub icloud: bool,
    pub icloud_available: bool,
    pub running: bool,
    pub last_at: Option<i64>,
    pub size: u64,
    pub media_size: u64,
    pub media_count: u64,
    pub counts: Counts,
    /// "uploaded" | "uploading" | "waiting" | "error" | "local" | "unknown"
    pub cloud: String,
    pub cloud_error: Option<String>,
    pub error: Option<String>,
    /// audio et vidéos de ce Mac (pour les interrupteurs)
    pub local_audio: u64,
    pub local_video: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Version {
    pub day: String,
    pub saved_at: i64,
    pub known: i64,
    pub lessons: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Info {
    pub key: String,
    pub device_name: String,
    /// sauvegarde de ce Mac et du profil en cours
    pub mine: bool,
    pub this_device: bool,
    pub saved_at: i64,
    pub app_version: String,
    pub size: u64,
    pub media_size: u64,
    pub counts: Counts,
    /// versions des jours précédents, de la plus récente à la plus ancienne
    pub versions: Vec<Version>,
    /// faite par une version plus récente de Lumen
    pub newer: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Restored {
    pub counts: Counts,
    /// médias absents de la sauvegarde (leçons restaurées sans leur audio ou leur vidéo)
    pub missing_media: u64,
}

pub struct Options {
    pub root: PathBuf,
    pub audio: bool,
    pub video: bool,
}

/// Sauvegarde faite, et le média qui n'a peut-être pas pu être copié
/// (la progression, elle, est sauvegardée).
pub type Outcome = (Manifest, Option<io::Error>);

// ---------- emplacements ----------

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// iCloud Drive de ce Mac (aucun : iCloud Drive n'est pas activé).
pub fn icloud_drive() -> Option<PathBuf> {
    let home = home()?;
    let p = if cfg!(target_os = "macos") { home.join("Library/Mobile Documents/com~apple~CloudDocs") } else { home.join("iCloudDrive") };
    p.is_dir().then_some(p)
}

/// Dossier « Lumen » de la sauvegarde : dans le dossier choisi, sinon dans iCloud Drive.
pub fn root(custom: &str) -> Option<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        return Some(if p.file_name().is_some_and(|n| n == ROOT_NAME) { p } else { p.join(ROOT_NAME) });
    }
    icloud_drive().map(|d| d.join(ROOT_NAME))
}

fn short_hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))[..12].to_string()
}

fn hardware_uuid() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("\"IOPlatformUUID\""))?;
    line.split('"').nth(3).filter(|s| !s.is_empty()).map(String::from)
}

/// Identifiant de ce Mac, stable même si les données de Lumen sont effacées
/// (tiré du numéro du matériel, jamais envoyé tel quel).
pub fn device_id(data_dir: &Path) -> String {
    let f = data_dir.join("device-id");
    if let Ok(s) = fs::read_to_string(&f) {
        let s = s.trim();
        if s.len() >= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
            return s.to_string();
        }
    }
    let seed = hardware_uuid().unwrap_or_else(|| format!("{:?}-{}", std::time::SystemTime::now(), std::process::id()));
    let id = short_hash(&format!("lumen-appareil:{seed}"));
    let _ = fs::write(&f, &id);
    id
}

/// Nom du Mac tel qu'il apparaît dans le Finder (« MacBook Pro de … »).
fn device_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        if cfg!(target_os = "macos") {
            if let Ok(o) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| crate::i18n::t("Ce Mac", "This Mac").into())
    })
    .clone()
}

/// Profil de la base, créé à la première sauvegarde.
fn ensure_profile(c: &Connection, device: &str) -> Result<String> {
    if let Some(p) = db::setting(c, "profile_id").filter(|p| p.len() >= 8) {
        return Ok(p);
    }
    let p = short_hash(&format!("lumen-profil:{device}:{:?}:{}", std::time::SystemTime::now(), std::process::id()));
    db::setting_set(c, "profile_id", &p)?;
    Ok(p)
}

fn key_of(profile: &str, device: &str) -> String {
    profile.chars().take(6).chain(device.chars().take(6)).collect()
}

/// Clé d'un dossier de sauvegarde : « MacBook Pro (a1b2c3d4e5f6) » → « a1b2c3d4e5f6 ».
fn folder_key(p: &Path) -> Option<String> {
    let n = p.file_name()?.to_str()?;
    let open = n.rfind(" (")?;
    let key = n.strip_suffix(')')?.get(open + 2..)?;
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric())).then(|| key.to_string())
}

fn folder_name(device_name: &str, key: &str) -> String {
    let clean: String = device_name.chars().map(|c| if matches!(c, '/' | ':' | '\\' | '(' | ')') || c.is_control() { ' ' } else { c }).collect();
    let clean: String = clean.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
    format!("{} ({key})", if clean.is_empty() { "Mac" } else { &clean })
}

/// Dossiers de sauvegarde (un par Mac et par profil).
fn device_folders(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in fs::read_dir(root)?.flatten() {
        let p = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) && folder_key(&p).is_some() {
            out.push(p);
        }
    }
    Ok(out)
}

fn find_folder(root: &Path, key: &str) -> Option<PathBuf> {
    device_folders(root).ok()?.into_iter().find(|p| folder_key(p).as_deref() == Some(key))
}

fn local_day(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn is_day(s: &str) -> bool {
    s.len() == 10 && s.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

// ---------- fichiers ----------

/// Fichier resté dans iCloud sur les anciens macOS (« .nom.icloud »).
fn placeholder(p: &Path) -> PathBuf {
    p.with_file_name(format!(".{}.icloud", file_name(p)))
}

/// Rapatrie un fichier resté dans iCloud. Depuis macOS 14, la simple lecture
/// suffit ; avant, le fichier n'existe que sous forme de « .nom.icloud ».
fn ensure_local(p: &Path, wait: Duration) -> Result<()> {
    if p.exists() {
        return Ok(());
    }
    let ph = placeholder(p);
    if !ph.exists() {
        bail!(crate::tr!("« {} » est introuvable dans la sauvegarde.", "\"{}\" can't be found in the backup.", file_name(p)));
    }
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("/usr/bin/brctl").arg("download").arg(&ph).status();
    }
    let start = Instant::now();
    while start.elapsed() < wait {
        if p.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    bail!(crate::tr!("« {} » n'est pas encore descendu d'iCloud. Vérifiez la connexion à Internet, puis réessayez.", "\"{}\" hasn't come down from iCloud yet. Check the Internet connection, then try again.", file_name(p)))
}

/// Copie ordinaire, avec repli sur une copie octet par octet quand le système
/// refuse le clonage (dossiers gérés par iCloud, disques réseau).
fn copy_file(src: &Path, dst: &Path) -> io::Result<()> {
    if fs::copy(src, dst).is_ok() {
        return Ok(());
    }
    let mut input = fs::File::open(src)?;
    let mut out = BufWriter::new(fs::File::create(dst)?);
    io::copy(&mut input, &mut out)?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()
}

/// Pose `staged` (fichier de travail sur ce Mac) à `dest` d'un seul geste :
/// iCloud ne voit jamais de fichier à moitié écrit.
fn put(staged: &Path, dest: &Path) -> io::Result<()> {
    if fs::rename(staged, dest).is_ok() {
        return Ok(());
    }
    // autre disque (clé USB, dossier réseau) : copie à côté, puis renommage
    let part = dest.with_file_name(format!("{}.nosync", file_name(dest)));
    let res = copy_file(staged, &part).and_then(|_| fs::rename(&part, dest));
    if res.is_err() {
        let _ = fs::remove_file(&part);
    }
    let _ = fs::remove_file(staged);
    res
}

fn write_json<T: Serialize>(dest: &Path, value: &T, scratch: &Path) -> Result<()> {
    let staged = scratch.join(format!("{}.json", short_hash(&dest.display().to_string())));
    fs::write(&staged, serde_json::to_vec_pretty(value)?)?;
    put(&staged, dest)?;
    Ok(())
}

fn read_manifest(p: &Path) -> Option<Manifest> {
    serde_json::from_slice(&fs::read(p).ok()?).ok()
}

fn gzip(src: &Path, dst: &Path) -> io::Result<u64> {
    let mut input = fs::File::open(src)?;
    let mut enc = GzEncoder::new(BufWriter::new(fs::File::create(dst)?), Compression::new(6));
    io::copy(&mut input, &mut enc)?;
    let f = enc.finish()?.into_inner().map_err(|e| e.into_error())?;
    f.sync_all()?;
    f.metadata().map(|m| m.len())
}

fn gunzip(src: &Path, dst: &Path) -> io::Result<()> {
    let mut dec = GzDecoder::new(io::BufReader::new(fs::File::open(src)?));
    let mut out = BufWriter::new(fs::File::create(dst)?);
    io::copy(&mut dec, &mut out)?;
    out.flush()
}

fn remove_db(p: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", p.display()));
    }
}

// ---------- contenu de la base ----------

fn counts(c: &Connection) -> Result<Counts> {
    let one = |sql: &str| -> Result<i64> { Ok(c.query_row(sql, [], |r| r.get(0))?) };
    let langs = db::setting(c, "langs").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    Ok(Counts {
        known: one("SELECT COUNT(*) FROM terms WHERE status=4 AND instr(term,' ')=0")?,
        learning: one("SELECT COUNT(*) FROM terms WHERE status BETWEEN 1 AND 3")?,
        phrases: one("SELECT COUNT(*) FROM terms WHERE instr(term,' ')>0 AND status<>5")?,
        lessons: one("SELECT COUNT(*) FROM lessons")?,
        langs,
    })
}

/// Un profil neuf (leçons d'accueil seules, aucun mot) n'a rien à sauvegarder :
/// une installation fraîche ne crée pas de sauvegarde vide.
fn has_progress(c: &Connection) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM terms) OR EXISTS(SELECT 1 FROM lessons WHERE collection NOT IN (?1, ?2))",
        STARTER_COLLECTIONS,
        |r| r.get(0),
    )?)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Audio,
    Video,
    Cover,
}

/// Fichiers cités par les leçons. Une vidéo qui porte aussi le son compte
/// comme audio : sans elle, la leçon n'aurait plus de son.
fn media_refs(c: &Connection) -> Result<Vec<(PathBuf, Kind)>> {
    let mut st = c.prepare("SELECT media_path, video_path, cover_path FROM lessons")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (audio, video, cover) = row?;
        if let Some(v) = video.filter(|v| Some(v) != audio.as_ref()) {
            out.push((PathBuf::from(v), Kind::Video));
        }
        if let Some(a) = audio {
            out.push((PathBuf::from(a), Kind::Audio));
        }
        if let Some(c) = cover {
            out.push((PathBuf::from(c), Kind::Cover));
        }
    }
    Ok(out)
}

// ---------- sauvegarder ----------

/// Sauvegarde la base et les médias dans `opts.root`. `None` : rien à
/// sauvegarder encore (le dossier est tout de même créé, ce qui fait
/// demander à macOS l'accès à iCloud Drive au moment où l'on active).
pub fn run(data_dir: &Path, opts: &Options) -> Result<Option<Outcome>> {
    fs::create_dir_all(&opts.root)?;
    let device = device_id(data_dir);
    let scratch = data_dir.join("sauvegarde.tmp");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch)?;
    let res = run_in(data_dir, opts, &device, &scratch);
    let _ = fs::remove_dir_all(&scratch);
    res
}

fn run_in(data_dir: &Path, opts: &Options, device: &str, scratch: &Path) -> Result<Option<Outcome>> {
    // 1. copie cohérente de la base par une seconde connexion : l'app continue d'écrire pendant ce temps
    let snap = scratch.join("base.db");
    let profile = {
        let src = Connection::open(db::path(data_dir))?;
        src.busy_timeout(Duration::from_secs(15))?;
        if !has_progress(&src)? {
            return Ok(None);
        }
        let p = ensure_profile(&src, device)?;
        src.execute("VACUUM INTO ?1", [snap.to_string_lossy()])?;
        p
    };
    // 2. sans le cache des traductions ni les réglages propres à ce Mac
    let (counts, refs) = {
        let c = Connection::open(&snap)?;
        c.execute("DELETE FROM tcache", [])?;
        for k in LOCAL_ONLY {
            c.execute("DELETE FROM settings WHERE key=?1", [k])?;
        }
        let out = (counts(&c)?, media_refs(&c)?);
        c.execute_batch("VACUUM;")?;
        out
    };

    // 3. les médias d'abord : une version n'est publiée qu'avec tout ce qu'elle cite
    let media_root = opts.root.join(MEDIA);
    fs::create_dir_all(&media_root)?;
    let local_media = media::media_dir(data_dir);
    let (mut names, mut media_size, mut warning) = (Vec::new(), 0u64, None);
    let mut seen = HashSet::new();
    for (path, kind) in refs {
        let wanted = match kind {
            Kind::Cover => true,
            Kind::Audio => opts.audio,
            Kind::Video => opts.video,
        };
        if !wanted || !path.starts_with(&local_media) || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else { continue };
        let name = file_name(&path);
        if !meta.is_file() || name.is_empty() {
            continue;
        }
        let dest = media_root.join(&name);
        let present = match fs::metadata(&dest) {
            Ok(m) => m.len() == meta.len(),
            Err(_) => placeholder(&dest).exists(),
        };
        if !present {
            let staged = scratch.join(&name);
            if let Err(e) = copy_file(&path, &staged).and_then(|_| put(&staged, &dest)) {
                warning.get_or_insert(e);
                continue;
            }
        }
        names.push(name);
        media_size += meta.len();
    }
    names.sort();

    // 4. la base compressée ; la version d'un autre jour rejoint l'historique
    let key = key_of(&profile, device);
    let folder = match find_folder(&opts.root, &key) {
        Some(f) => f,
        None => {
            let f = opts.root.join(folder_name(&device_name(), &key));
            fs::create_dir_all(&f)?;
            f
        }
    };
    let gz = scratch.join(SNAPSHOT);
    let size = gzip(&snap, &gz)?;
    // médias que ce Mac cessera peut-être de citer
    let mut dropped: Vec<String> = Vec::new();
    if let Some(prev) = read_manifest(&folder.join(MANIFEST)) {
        let day = local_day(prev.saved_at);
        if is_day(&day) && day != local_day(db::now()) && folder.join(SNAPSHOT).exists() {
            let hist = folder.join(HISTORY);
            fs::create_dir_all(&hist)?;
            fs::rename(folder.join(SNAPSHOT), hist.join(format!("{day}.lumen")))?;
            write_json(&hist.join(format!("{day}.json")), &prev, scratch)?;
        } else {
            dropped.extend(prev.media);
        }
    }
    put(&gz, &folder.join(SNAPSHOT))?;
    let manifest = Manifest {
        format: FORMAT,
        app_version: env!("CARGO_PKG_VERSION").into(),
        profile,
        device: device.into(),
        device_name: device_name(),
        saved_at: db::now(),
        size,
        counts,
        media: names,
        media_size,
    };
    write_json(&folder.join(MANIFEST), &manifest, scratch)?;
    dropped.extend(prune_history(&folder));
    // 5. ménage dans les médias
    collect_garbage(&opts.root, &dropped);
    Ok(Some((manifest, warning)))
}

/// Garde les versions quotidiennes les plus récentes ; renvoie les médias
/// cités par les versions effacées.
fn prune_history(folder: &Path) -> Vec<String> {
    let hist = folder.join(HISTORY);
    let mut days: Vec<String> = fs::read_dir(&hist)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = file_name(&e.path());
            let day = n.strip_suffix(".lumen").or_else(|| n.strip_suffix(".json"))?.to_string();
            is_day(&day).then_some(day)
        })
        .collect();
    days.sort_unstable_by(|a, b| b.cmp(a));
    days.dedup();
    let mut dropped = Vec::new();
    for day in days.iter().skip(HISTORY_DAYS) {
        if let Some(m) = read_manifest(&hist.join(format!("{day}.json"))) {
            dropped.extend(m.media);
        }
        let _ = fs::remove_file(hist.join(format!("{day}.lumen")));
        let _ = fs::remove_file(hist.join(format!("{day}.json")));
    }
    dropped
}

fn has_placeholder(dir: &Path) -> bool {
    fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let n = file_name(&e.path());
        n.starts_with('.') && n.ends_with(".icloud")
    })
}

/// Efface de Médias/ les fichiers que ce Mac a cessé de citer et qu'aucune
/// version, de ce Mac ou d'un autre, ne cite plus. Un manifeste illisible
/// (pas encore descendu d'iCloud) suspend le ménage : dans le doute, on garde.
/// Si un autre Mac cite un fichier effacé trop tôt, sa prochaine sauvegarde
/// le recopie.
fn collect_garbage(root: &Path, candidates: &[String]) {
    if candidates.is_empty() {
        return;
    }
    let Ok(folders) = device_folders(root) else { return };
    let mut keep: HashSet<String> = HashSet::new();
    for folder in folders {
        let hist = folder.join(HISTORY);
        if has_placeholder(&folder) || has_placeholder(&hist) {
            return;
        }
        let mut files = Vec::new();
        if folder.join(MANIFEST).exists() {
            files.push(folder.join(MANIFEST));
        } else if folder.join(SNAPSHOT).exists() {
            // sauvegarde en cours d'écriture sur un autre Mac
            return;
        }
        for e in fs::read_dir(&hist).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                files.push(e.path());
            }
        }
        for f in files {
            match read_manifest(&f) {
                Some(m) => keep.extend(m.media),
                None => return,
            }
        }
    }
    for name in candidates {
        let safe = !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']);
        if safe && !keep.contains(name) {
            let _ = fs::remove_file(root.join(MEDIA).join(name));
        }
    }
}

// ---------- retrouver et restaurer ----------

fn versions(folder: &Path) -> Vec<Version> {
    let hist = folder.join(HISTORY);
    let mut out: Vec<Version> = fs::read_dir(&hist)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = file_name(&e.path());
            let day = n.strip_suffix(".json")?;
            if !is_day(day) || !(hist.join(format!("{day}.lumen")).exists() || placeholder(&hist.join(format!("{day}.lumen"))).exists()) {
                return None;
            }
            let m = read_manifest(&e.path())?;
            Some(Version { day: day.to_string(), saved_at: m.saved_at, known: m.counts.known, lessons: m.counts.lessons })
        })
        .collect();
    out.sort_by(|a, b| b.day.cmp(&a.day));
    out
}

/// Sauvegardes trouvées dans `root`, de la plus récente à la plus ancienne.
pub fn list(data_dir: &Path, root: &Path, profile: Option<&str>) -> Result<Vec<Info>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let device = device_id(data_dir);
    let mut out = Vec::new();
    for folder in device_folders(root)? {
        let Some(key) = folder_key(&folder) else { continue };
        let mf = folder.join(MANIFEST);
        if ensure_local(&mf, Duration::from_secs(15)).is_err() {
            continue;
        }
        let Some(m) = read_manifest(&mf) else { continue };
        out.push(Info {
            key,
            mine: profile == Some(m.profile.as_str()) && m.device == device,
            this_device: m.device == device,
            device_name: m.device_name,
            saved_at: m.saved_at,
            app_version: m.app_version,
            size: m.size,
            media_size: m.media_size,
            counts: m.counts,
            versions: versions(&folder),
            newer: m.format > FORMAT,
        });
    }
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    Ok(out)
}

/// Remplace la progression de ce Mac par une sauvegarde (la plus récente,
/// ou celle du jour `day`). La progression d'avant est gardée sur ce Mac
/// (`lumen.avant-restauration.db`).
pub fn restore(live: &Mutex<Connection>, data_dir: &Path, root: &Path, key: &str, day: Option<&str>, on: impl Fn(ImportEvent)) -> Result<Restored> {
    let stage = |s: &str| on(ImportEvent::Stage { stage: s.into() });
    let folder = find_folder(root, key).ok_or_else(|| anyhow!(crate::i18n::t("Cette sauvegarde est introuvable.", "This backup can't be found.")))?;
    let (snap, mf) = match day {
        None => (folder.join(SNAPSHOT), folder.join(MANIFEST)),
        Some(d) if is_day(d) => (folder.join(HISTORY).join(format!("{d}.lumen")), folder.join(HISTORY).join(format!("{d}.json"))),
        Some(_) => bail!(crate::i18n::t("Cette version de la sauvegarde est introuvable.", "This version of the backup can't be found.")),
    };
    stage("download");
    ensure_local(&mf, Duration::from_secs(30))?;
    let manifest = read_manifest(&mf).ok_or_else(|| anyhow!(crate::i18n::t("Les informations de cette sauvegarde sont illisibles.", "The details of this backup are unreadable.")))?;
    if manifest.format > FORMAT {
        bail!(crate::i18n::t("Cette sauvegarde vient d'une version plus récente de Lumen. Mettez Lumen à jour, puis réessayez.", "This backup comes from a newer version of Lumen. Update Lumen, then try again."));
    }
    ensure_local(&snap, Duration::from_secs(600))?;
    let scratch = data_dir.join("restauration.tmp");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch)?;
    let res = restore_in(live, data_dir, root, &snap, &scratch, &on);
    let _ = fs::remove_dir_all(&scratch);
    res
}

fn restore_in(live: &Mutex<Connection>, data_dir: &Path, root: &Path, snap: &Path, scratch: &Path, on: &impl Fn(ImportEvent)) -> Result<Restored> {
    let stage = |s: &str| on(ImportEvent::Stage { stage: s.into() });
    let tmp = scratch.join("base.db");
    gunzip(snap, &tmp).map_err(|e| match e.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof => anyhow!(crate::i18n::t("Le fichier de sauvegarde est abîmé ou incomplet.", "The backup file is damaged or incomplete.")),
        _ => anyhow!(e),
    })?;
    let bad = || anyhow!(crate::i18n::t("Le fichier de sauvegarde est abîmé ou incomplet.", "The backup file is damaged or incomplete."));
    // remise à niveau d'une base plus ancienne (migrations additives)
    let c = db::open(&tmp).map_err(|_| bad())?;
    let check: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|_| bad())?;
    if check != "ok" {
        return Err(bad());
    }

    // médias : chemins de ce Mac, fichiers rapatriés de la sauvegarde
    stage("media");
    let local_media = media::media_dir(data_dir);
    fs::create_dir_all(&local_media)?;
    let cloud_media = root.join(MEDIA);
    let rows: Vec<(i64, [Option<String>; 3])> = c
        .prepare("SELECT id, media_path, video_path, cover_path FROM lessons")?
        .query_map([], |r| Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?])))?
        .collect::<rusqlite::Result<_>>()?;
    let total = rows.iter().flat_map(|(_, p)| p.iter()).filter(|p| p.is_some()).count().max(1);
    let mut done = 0usize;
    let mut found: HashMap<String, Option<String>> = HashMap::new();
    let mut missing = HashSet::new();
    for (id, paths) in &rows {
        let mut fixed: [Option<String>; 3] = [None, None, None];
        for (i, p) in paths.iter().enumerate() {
            let Some(p) = p else { continue };
            let name = file_name(Path::new(p));
            let local = found
                .entry(name.clone())
                .or_insert_with(|| {
                    let dest = local_media.join(&name);
                    if name.is_empty() || dest.is_file() {
                        return (!name.is_empty()).then(|| dest.display().to_string());
                    }
                    let remote = cloud_media.join(&name);
                    ensure_local(&remote, Duration::from_secs(600)).ok()?;
                    let staged = scratch.join(&name);
                    copy_file(&remote, &staged).and_then(|_| fs::rename(&staged, &dest)).ok()?;
                    Some(dest.display().to_string())
                })
                .clone();
            if local.is_none() {
                missing.insert(name);
            }
            fixed[i] = local;
            done += 1;
            on(ImportEvent::Progress { value: done as f64 / total as f64 });
        }
        if &fixed != paths {
            c.execute("UPDATE lessons SET media_path=?1, video_path=?2, cover_path=?3 WHERE id=?4", params![fixed[0], fixed[1], fixed[2], id])?;
        }
    }
    {
        // réglages propres à ce Mac (clé LingQ, emplacement et choix de la sauvegarde)
        let l = live.lock();
        for k in KEEP_ON_RESTORE {
            if let Some(v) = db::setting(&l, k) {
                db::setting_set(&c, k, &v)?;
            }
        }
    }
    let counts = counts(&c)?;
    drop(c);

    stage("apply");
    let mut l = live.lock();
    // filet de sécurité : la progression d'avant reste sur ce Mac
    let safety = data_dir.join("lumen.avant-restauration.db");
    remove_db(&safety);
    l.execute("VACUUM INTO ?1", [safety.to_string_lossy()])?;
    l.flush_prepared_statement_cache();
    l.restore(rusqlite::MAIN_DB, &tmp, None::<fn(rusqlite::backup::Progress)>)?;
    Ok(Restored { counts, missing_media: missing.len() as u64 })
}

// ---------- état iCloud d'un fichier ----------

/// Le fichier est-il arrivé dans iCloud ? (macOS seulement)
#[cfg(target_os = "macos")]
fn cloud_state(path: &Path) -> (String, Option<String>) {
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::AnyObject;
    use objc2_foundation::{
        NSError, NSNumber, NSString, NSURLIsUbiquitousItemKey, NSURLResourceKey, NSURLUbiquitousItemIsUploadedKey,
        NSURLUbiquitousItemIsUploadingKey, NSURLUbiquitousItemUploadingErrorKey, NSURL,
    };
    if !path.exists() {
        return ("unknown".into(), None);
    }
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let get = |key: &NSURLResourceKey| -> Option<Retained<AnyObject>> {
            let mut v: Option<Retained<AnyObject>> = None;
            // SAFETY : la valeur demandée est un objet, rendu sous forme générique
            unsafe { url.getResourceValue_forKey_error(&mut v, key) }.ok()?;
            v
        };
        let flag = |key: &NSURLResourceKey| get(key).and_then(|o| o.downcast::<NSNumber>().ok()).is_some_and(|n| n.boolValue());
        // SAFETY : constantes de Foundation, toujours définies
        let (ubiquitous, error, uploaded, uploading) = unsafe {
            (
                flag(NSURLIsUbiquitousItemKey),
                get(NSURLUbiquitousItemUploadingErrorKey).and_then(|o| o.downcast::<NSError>().ok()),
                flag(NSURLUbiquitousItemIsUploadedKey),
                flag(NSURLUbiquitousItemIsUploadingKey),
            )
        };
        if !ubiquitous {
            ("local".into(), None)
        } else if let Some(e) = error {
            ("error".into(), Some(e.localizedDescription().to_string()))
        } else if uploaded {
            ("uploaded".into(), None)
        } else if uploading {
            ("uploading".into(), None)
        } else {
            ("waiting".into(), None)
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn cloud_state(_path: &Path) -> (String, Option<String>) {
    ("unknown".into(), None)
}

// ---------- branchement sur l'application ----------

struct Prefs {
    on: String,
    dir: String,
    audio: bool,
    video: bool,
    profile: Option<String>,
}

fn prefs(c: &Connection) -> Prefs {
    let get = |k: &str| db::setting(c, k).unwrap_or_default();
    Prefs {
        on: get("backup_on"),
        dir: get("backup_dir"),
        audio: get("backup_audio") != "0",
        video: get("backup_video") == "1",
        profile: db::setting(c, "profile_id").filter(|p| !p.is_empty()),
    }
}

fn no_place() -> String {
    crate::i18n::t(
        "iCloud Drive n'est pas activé sur ce Mac. Activez-le dans Réglages Système › votre nom › iCloud, ou choisissez un autre dossier.",
        "iCloud Drive isn't turned on on this Mac. Turn it on in System Settings › your name › iCloud, or choose another folder.",
    )
    .into()
}

/// Message compréhensible pour les erreurs de fichier les plus courantes.
fn io_message(io: &io::Error, icloud: bool) -> Option<String> {
    if matches!(io.raw_os_error(), Some(28) | Some(112)) {
        return Some(crate::i18n::t("Il n'y a plus assez d'espace disque sur ce Mac.", "There isn't enough disk space left on this Mac.").into());
    }
    match io.kind() {
        io::ErrorKind::PermissionDenied if icloud => Some(
            crate::i18n::t(
                "Lumen n'a pas accès à iCloud Drive. Autorisez-le dans Réglages Système › Confidentialité et sécurité › Fichiers et dossiers, puis réessayez.",
                "Lumen has no access to iCloud Drive. Allow it in System Settings › Privacy & Security › Files and Folders, then try again.",
            )
            .into(),
        ),
        io::ErrorKind::PermissionDenied => Some(crate::i18n::t("Lumen ne peut pas écrire dans ce dossier. Choisissez-en un autre.", "Lumen can't write to this folder. Choose another one.").into()),
        io::ErrorKind::NotFound if !icloud => Some(crate::i18n::t("Le dossier de sauvegarde est introuvable. Le disque est-il branché ?", "The backup folder can't be found. Is the drive connected?").into()),
        _ => None,
    }
}

/// Message d'erreur lisible ; `what` : (« Sauvegarde impossible », « Backup failed »)…
/// Les messages écrits pour l'utilisateur (`bail!`) passent tels quels.
fn friendly(e: &anyhow::Error, icloud: bool, what: (&'static str, &'static str)) -> String {
    let what = crate::i18n::t(what.0, what.1);
    if let Some(io) = e.chain().find_map(|c| c.downcast_ref::<io::Error>()) {
        return io_message(io, icloud).unwrap_or_else(|| crate::tr!("{what} : {io}", "{what}: {io}"));
    }
    if let Some(sql) = e.chain().find_map(|c| c.downcast_ref::<rusqlite::Error>()) {
        return crate::tr!("{what} : {sql}", "{what}: {sql}");
    }
    e.to_string()
}

/// Sauvegarde maintenant (bouton, activation, minuterie ou fermeture).
pub fn run_now(state: &AppState) -> Result<(), String> {
    let _running = state.backup.wait();
    let (p, changes) = {
        let c = state.db.lock();
        (prefs(&c), c.total_changes())
    };
    state.backup.last_try.store(db::now(), Ordering::SeqCst);
    let icloud = p.dir.trim().is_empty();
    let res = match root(&p.dir) {
        None => Err(no_place()),
        Some(root) => run(&state.data_dir, &Options { root, audio: p.audio, video: p.video }).map_err(|e| friendly(&e, icloud, ("Sauvegarde impossible", "Backup failed"))),
    };
    let mut error = state.backup.error.lock();
    match res {
        Ok(out) => {
            state.backup.seen.store(changes, Ordering::SeqCst);
            *error = out.and_then(|(_, w)| w).map(|w| {
                let why = io_message(&w, icloud).unwrap_or_else(|| w.to_string());
                crate::tr!(
                    "La progression est sauvegardée, mais un média n'a pas pu être copié : {why}",
                    "Your progress is backed up, but a media file couldn't be copied: {why}"
                )
            });
            Ok(())
        }
        Err(msg) => {
            *error = Some(msg.clone());
            Err(msg)
        }
    }
}

/// La progression a changé depuis la dernière sauvegarde ?
fn dirty(state: &AppState, changes: u64) -> bool {
    if changes != state.backup.seen.load(Ordering::SeqCst) {
        return true;
    }
    // premier passage : l'app a pu être quittée brutalement avant de sauvegarder
    if state.backup.checked.swap(true, Ordering::SeqCst) {
        return false;
    }
    let last = status(state).last_at.unwrap_or(0);
    let path = db::path(&state.data_dir);
    [path.clone(), PathBuf::from(format!("{}-wal", path.display()))]
        .iter()
        .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .any(|d| d.as_secs() as i64 > last + 5)
}

/// Sauvegarde automatique : au plus toutes les 10 minutes, et seulement si
/// quelque chose a changé.
pub async fn auto_loop(app: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        let h = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let st = h.state::<AppState>();
            let (on, changes) = {
                let c = st.db.lock();
                (db::setting(&c, "backup_on").as_deref() == Some("1"), c.total_changes())
            };
            let waited = db::now() - st.backup.last_try.load(Ordering::SeqCst) >= AUTO_EVERY;
            if on && waited && !st.backup.running.load(Ordering::SeqCst) && dirty(&st, changes) {
                let _ = run_now(&st);
                let _ = h.emit("backup", status(&st));
            }
        })
        .await;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// À la fermeture de Lumen : dernière sauvegarde si quelque chose a changé.
pub fn on_exit(state: &AppState) {
    let (on, changes) = {
        let c = state.db.lock();
        (db::setting(&c, "backup_on").as_deref() == Some("1"), c.total_changes())
    };
    if on && changes != state.backup.seen.load(Ordering::SeqCst) && !state.backup.running.load(Ordering::SeqCst) {
        let _ = run_now(state);
    }
}

/// Taille de l'audio et des vidéos de ce Mac.
fn local_sizes(state: &AppState) -> (u64, u64) {
    let refs = media_refs(&state.db.lock()).unwrap_or_default();
    let local_media = media::media_dir(&state.data_dir);
    let mut seen = HashSet::new();
    let (mut audio, mut video) = (0, 0);
    for (p, kind) in refs {
        if kind == Kind::Cover || !p.starts_with(&local_media) || !seen.insert(p.clone()) {
            continue;
        }
        let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if kind == Kind::Audio {
            audio += size;
        } else {
            video += size;
        }
    }
    (audio, video)
}

pub fn status(state: &AppState) -> Status {
    let p = prefs(&state.db.lock());
    let root = root(&p.dir);
    let (local_audio, local_video) = local_sizes(state);
    let mut s = Status {
        enabled: p.on == "1",
        decided: !p.on.is_empty(),
        dir: root.as_ref().map(|r| r.display().to_string()),
        icloud: p.dir.trim().is_empty() && root.is_some(),
        icloud_available: icloud_drive().is_some(),
        running: state.backup.running.load(Ordering::SeqCst),
        cloud: "unknown".into(),
        error: state.backup.error.lock().clone(),
        local_audio,
        local_video,
        ..Default::default()
    };
    // le dossier n'est lu qu'une fois la sauvegarde acceptée (macOS demande l'accès à iCloud Drive)
    if let (true, Some(root), Some(profile)) = (s.enabled, &root, &p.profile) {
        if let Some(folder) = find_folder(root, &key_of(profile, &device_id(&state.data_dir))) {
            if let Some(m) = read_manifest(&folder.join(MANIFEST)) {
                s.last_at = Some(m.saved_at);
                s.size = m.size;
                s.media_size = m.media_size;
                s.media_count = m.media.len() as u64;
                s.counts = m.counts;
            }
            (s.cloud, s.cloud_error) = cloud_state(&folder.join(SNAPSHOT));
        }
    }
    s
}

pub fn list_for(state: &AppState) -> Result<Vec<Info>, String> {
    let p = prefs(&state.db.lock());
    let root = root(&p.dir).ok_or_else(no_place)?;
    list(&state.data_dir, &root, p.profile.as_deref()).map_err(|e| friendly(&e, p.dir.trim().is_empty(), ("Lecture des sauvegardes impossible", "Couldn't read the backups")))
}

pub fn restore_for(state: &AppState, key: &str, day: Option<&str>, on: impl Fn(ImportEvent)) -> Result<Restored, String> {
    let _running = state.backup.wait();
    let p = prefs(&state.db.lock());
    let root = root(&p.dir).ok_or_else(no_place)?;
    let out = restore(&state.db, &state.data_dir, &root, key, day, on).map_err(|e| friendly(&e, p.dir.trim().is_empty(), ("Restauration impossible", "Restore failed")))?;
    // la progression restaurée rejoint la sauvegarde de ce Mac à la prochaine occasion
    state.backup.seen.store(u64::MAX, Ordering::SeqCst);
    state.backup.last_try.store(0, Ordering::SeqCst);
    *state.backup.error.lock() = None;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lumen-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("media")).unwrap();
        p
    }

    fn lesson(c: &Connection, title: &str, collection: &str, audio: Option<&Path>, video: Option<&Path>, cover: Option<&Path>) -> i64 {
        let id = db::lesson_create(c, &db::NewLesson {
            lang: "it".into(), title: title.into(), collection: collection.into(), kind: "audio".into(),
            source: String::new(), text: "Il faro è alto.".into(),
            media_path: audio.map(|p| p.display().to_string()), timings: None, video_path: video.map(|p| p.display().to_string()),
        })
        .unwrap();
        if let Some(cover) = cover {
            db::lesson_set_cover(c, id, Some(cover.to_str().unwrap())).unwrap();
        }
        id
    }

    fn snapshot_settings(folder: &Path) -> HashMap<String, String> {
        let out = folder.join("lu.db");
        gunzip(&folder.join(SNAPSHOT), &out).unwrap();
        let c = Connection::open(&out).unwrap();
        let s = db::settings_all(&c).unwrap();
        let cache: i64 = c.query_row("SELECT COUNT(*) FROM tcache", [], |r| r.get(0)).unwrap();
        assert_eq!(cache, 0);
        drop(c);
        let _ = fs::remove_file(out);
        s
    }

    #[test]
    fn fresh_profile_is_not_saved() {
        let data = temp("fresh");
        let root = data.join("cloud").join(ROOT_NAME);
        let c = db::open(&db::path(&data)).unwrap();
        lesson(&c, "Il faro", STARTER_COLLECTIONS[0], None, None, None);
        assert!(run(&data, &Options { root: root.clone(), audio: true, video: true }).unwrap().is_none());
        // le dossier existe (accès demandé), mais aucune sauvegarde n'y est
        assert!(root.is_dir());
        assert!(list(&data, &root, None).unwrap().is_empty());
        drop(c);
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn backup_then_restore_on_another_mac() {
        // premier Mac : une leçon audio avec vidéo et couverture, des mots, une clé LingQ
        let a = temp("a");
        fs::write(a.join("device-id"), "aaaaaaaaaaaa").unwrap();
        let root = a.join("cloud").join(ROOT_NAME);
        let (audio, video, cover) = (a.join("media/1.m4a"), a.join("media/1.mp4"), a.join("media/1.cover.jpg"));
        fs::write(&audio, b"audio").unwrap();
        fs::write(&video, b"video!").unwrap();
        fs::write(&cover, b"img").unwrap();
        let mut ca = db::open(&db::path(&a)).unwrap();
        let id = lesson(&ca, "Il faro", "Storie", Some(&audio), Some(&video), Some(&cover));
        db::terms_mark_known(&mut ca, "it", &["faro".into(), "alto".into()], 3).unwrap();
        db::term_set(&ca, &db::TermUpdate { lang: "it".into(), term: "è alto".into(), status: 2, translation: Some("est haut".into()), note: None, lemma: None, context: None }).unwrap();
        db::setting_set(&ca, "lingq_key", "secret").unwrap();
        db::setting_set(&ca, "langs", "it").unwrap();
        db::cache_put(&ca, "k", "v");

        let out = run(&a, &Options { root: root.clone(), audio: true, video: false }).unwrap().unwrap();
        assert!(out.1.is_none());
        let m = out.0;
        assert_eq!((m.counts.known, m.counts.phrases, m.counts.lessons, m.counts.langs.clone()), (2, 1, 1, vec!["it".to_string()]));
        assert_eq!(m.media, vec!["1.cover.jpg".to_string(), "1.m4a".to_string()]);
        assert!(!root.join(MEDIA).join("1.mp4").exists());
        let folder = find_folder(&root, &key_of(&m.profile, "aaaaaaaaaaaa")).unwrap();
        // ni la clé LingQ ni le cache ne quittent ce Mac
        let s = snapshot_settings(&folder);
        assert_eq!((s.get("lingq_key"), s.get("profile_id")), (None, Some(&m.profile)));

        // second Mac : sa propre clé LingQ, sauvegarde coupée
        let b = temp("b");
        fs::write(b.join("device-id"), "bbbbbbbbbbbb").unwrap();
        let cb = db::open(&db::path(&b)).unwrap();
        db::setting_set(&cb, "lingq_key", "autre").unwrap();
        db::setting_set(&cb, "backup_on", "0").unwrap();
        let live = Mutex::new(cb);
        let found = list(&b, &root, None).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].mine && !found[0].this_device && !found[0].newer);
        assert_eq!(found[0].counts.known, 2);

        let events = Mutex::new(Vec::new());
        let r = restore(&live, &b, &root, &found[0].key, None, |e| events.lock().push(e)).unwrap();
        assert_eq!((r.counts.known, r.counts.lessons, r.missing_media), (2, 1, 1));
        assert!(matches!(events.lock().last(), Some(ImportEvent::Stage { stage }) if stage == "apply"));
        let cb = live.lock();
        let l = db::lesson_get(&cb, id).unwrap();
        // audio et couverture rapatriés sur ce Mac, vidéo absente de la sauvegarde
        assert_eq!(l.media_path, Some(b.join("media/1.m4a").display().to_string()));
        assert_eq!(fs::read(b.join("media/1.m4a")).unwrap(), b"audio");
        assert_eq!(l.cover_path, Some(b.join("media/1.cover.jpg").display().to_string()));
        assert_eq!(l.video_path, None);
        assert_eq!(db::stats(&cb, "it").unwrap().phrases, 1);
        assert_eq!(db::setting(&cb, "lingq_key").as_deref(), Some("autre"));
        assert_eq!(db::setting(&cb, "backup_on").as_deref(), Some("0"));
        assert_eq!(db::setting(&cb, "profile_id"), Some(m.profile.clone()));
        assert!(b.join("lumen.avant-restauration.db").exists());
        drop(cb);

        // le second Mac continue le même profil, dans son propre dossier
        let out_b = run(&b, &Options { root: root.clone(), audio: true, video: true }).unwrap().unwrap();
        assert_eq!(out_b.0.profile, m.profile);
        let all = list(&b, &root, Some(&m.profile)).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|i| i.mine && i.this_device));
        drop(ca);
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn history_and_media_cleanup() {
        let a = temp("hist");
        fs::write(a.join("device-id"), "cccccccccccc").unwrap();
        let root = a.join("cloud").join(ROOT_NAME);
        let (one, two) = (a.join("media/1.m4a"), a.join("media/2.m4a"));
        fs::write(&one, b"un").unwrap();
        fs::write(&two, b"deux").unwrap();
        let c = db::open(&db::path(&a)).unwrap();
        let first = lesson(&c, "Uno", "Storie", Some(&one), None, None);
        let opts = Options { root: root.clone(), audio: true, video: false };
        let m = run(&a, &opts).unwrap().unwrap().0;
        let folder = find_folder(&root, &key_of(&m.profile, "cccccccccccc")).unwrap();

        // la sauvegarde d'hier part dans l'historique, avec les médias qu'elle cite
        let mut old = read_manifest(&folder.join(MANIFEST)).unwrap();
        old.saved_at -= 86_400;
        fs::write(folder.join(MANIFEST), serde_json::to_vec(&old).unwrap()).unwrap();
        db::lesson_delete(&c, first).unwrap();
        lesson(&c, "Due", "Storie", Some(&two), None, None);
        run(&a, &opts).unwrap().unwrap();
        let yesterday = local_day(old.saved_at);
        assert!(folder.join(HISTORY).join(format!("{yesterday}.lumen")).exists());
        assert!(root.join(MEDIA).join("1.m4a").exists());
        let v = versions(&folder);
        assert_eq!((v.len(), v[0].day.as_str()), (1, yesterday.as_str()));

        // au-delà de 14 jours, les versions s'effacent, et les médias qu'elles seules citaient
        for d in 1..=15 {
            let day = format!("2020-01-{d:02}");
            fs::write(folder.join(HISTORY).join(format!("{day}.lumen")), b"x").unwrap();
            fs::write(folder.join(HISTORY).join(format!("{day}.json")), serde_json::to_vec(&Manifest::default()).unwrap()).unwrap();
        }
        // le média « 1 » n'est plus cité que par la version d'hier, poussée hors des 14 jours
        let mut stale = read_manifest(&folder.join(HISTORY).join(format!("{yesterday}.json"))).unwrap();
        stale.saved_at = 0;
        fs::write(folder.join(HISTORY).join(format!("{yesterday}.json")), serde_json::to_vec(&stale).unwrap()).unwrap();
        fs::rename(folder.join(HISTORY).join(format!("{yesterday}.json")), folder.join(HISTORY).join("2019-12-31.json")).unwrap();
        fs::rename(folder.join(HISTORY).join(format!("{yesterday}.lumen")), folder.join(HISTORY).join("2019-12-31.lumen")).unwrap();
        run(&a, &opts).unwrap().unwrap();
        assert_eq!(versions(&folder).len(), HISTORY_DAYS);
        assert!(!folder.join(HISTORY).join("2019-12-31.lumen").exists());
        assert!(!root.join(MEDIA).join("1.m4a").exists());
        assert!(root.join(MEDIA).join("2.m4a").exists());
        drop(c);
        let _ = fs::remove_dir_all(a);
    }

    /// Sauvegarde puis restauration d'une copie de vraies données, dans un dossier
    /// jetable (rien n'est écrit dans le dossier d'origine ni dans iCloud) :
    /// `LUMEN_BACKUP_DATA="$HOME/Library/Application Support/app.lumen.reader" cargo test --lib backup_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn backup_live() {
        let src = PathBuf::from(std::env::var("LUMEN_BACKUP_DATA").expect("LUMEN_BACKUP_DATA : dossier de données de Lumen"));
        let a = temp("live-a");
        Connection::open(db::path(&src)).unwrap().execute("VACUUM INTO ?1", [db::path(&a).to_string_lossy()]).unwrap();
        // les médias d'origine, lus à travers un lien, et les chemins de la copie qui y mènent
        let real = media::media_dir(&src);
        fs::remove_dir(a.join("media")).unwrap();
        std::os::unix::fs::symlink(&real, a.join("media")).unwrap();
        {
            let c = Connection::open(db::path(&a)).unwrap();
            for col in ["media_path", "video_path", "cover_path"] {
                c.execute(&format!("UPDATE lessons SET {col}=replace({col}, ?1, ?2)"), params![real.display().to_string(), a.join("media").display().to_string()]).unwrap();
            }
        }
        let root = a.join("nuage").join(ROOT_NAME);
        let t = Instant::now();
        let (m, warning) = run(&a, &Options { root: root.clone(), audio: true, video: false }).unwrap().expect("rien à sauvegarder");
        println!(
            "sauvegarde : {:.2} s, base {:.1} Mo, {} médias ({:.0} Mo), {:?}, avertissement {warning:?}",
            t.elapsed().as_secs_f64(), m.size as f64 / 1e6, m.media.len(), m.media_size as f64 / 1e6, m.counts
        );
        let t = Instant::now();
        assert!(run(&a, &Options { root: root.clone(), audio: true, video: false }).unwrap().is_some());
        println!("seconde sauvegarde (médias déjà là) : {:.2} s", t.elapsed().as_secs_f64());

        let b = temp("live-b");
        fs::write(b.join("device-id"), "bbbbbbbbbbbb").unwrap();
        let live = Mutex::new(db::open(&db::path(&b)).unwrap());
        let found = list(&b, &root, None).unwrap();
        let t = Instant::now();
        let r = restore(&live, &b, &root, &found[0].key, None, |_| {}).unwrap();
        println!("restauration : {:.2} s, {:?}, médias manquants {}", t.elapsed().as_secs_f64(), r.counts, r.missing_media);
        assert_eq!(r.counts, m.counts);
        let c = live.lock();
        let lessons: i64 = c.query_row("SELECT COUNT(*) FROM lessons", [], |x| x.get(0)).unwrap();
        let outside: i64 = c
            .query_row("SELECT COUNT(*) FROM lessons WHERE substr(media_path,1,length(?1))<>?1", [media::media_dir(&b).display().to_string()], |x| x.get(0))
            .unwrap();
        assert_eq!((lessons, outside), (m.counts.lessons, 0));
        drop(c);
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    /// Lecture seule : état d'envoi du premier fichier trouvé dans iCloud Drive.
    #[test]
    #[ignore]
    fn icloud_state_live() {
        let drive = icloud_drive().expect("iCloud Drive n'est pas activé");
        let file = fs::read_dir(&drive).unwrap().flatten().map(|e| e.path()).find(|p| p.is_file()).expect("aucun fichier");
        let (state, err) = cloud_state(&file);
        println!("{} : {state} {err:?}", file.display());
        assert!(["uploaded", "uploading", "waiting", "error"].contains(&state.as_str()));
        assert_eq!(cloud_state(&std::env::temp_dir()).0, "local");
    }

    #[test]
    fn folder_names_and_keys() {
        assert_eq!(folder_name("MacBook Pro de Léa: travail/maison", "abc123def456"), "MacBook Pro de Léa travail maison (abc123def456)");
        assert_eq!(folder_key(Path::new("/x/MacBook (Pro) (abc123def456)")).as_deref(), Some("abc123def456"));
        assert_eq!(folder_key(Path::new("/x/Médias")), None);
        assert_eq!(key_of("0123456789ab", "ba9876543210"), "012345ba9876");
        assert!(is_day("2026-10-03") && !is_day("2026-1-03") && !is_day("../../x"));
        assert_eq!(root("/Users/x/Dropbox").unwrap(), PathBuf::from("/Users/x/Dropbox/Lumen"));
        assert_eq!(root("/Volumes/Clé/Lumen").unwrap(), PathBuf::from("/Volumes/Clé/Lumen"));
    }
}
