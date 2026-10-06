//! Sauvegarde de la progression dans iCloud Drive, ou dans un autre nuage
//! installé sur le Mac (Dropbox, Google Drive, OneDrive… : `places`), ou dans
//! tout dossier choisi par l'utilisateur (clé USB, disque réseau).
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
use crate::user;

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
/// « elle reste sur ce Mac »), l'emplacement de la sauvegarde, propre à ce disque,
/// et le choix de la carte graphique d'un PC.
const LOCAL_ONLY: [&str; 4] = ["lingq_key", "gemini_key", "backup_dir", "ai_gpu"];
/// Réglages de ce Mac conservés quand on restaure une sauvegarde (la langue de
/// l'interface aussi : celle qu'on vient de choisir à l'accueil reste).
const KEEP_ON_RESTORE: [&str; 9] = ["lingq_key", "gemini_key", "backup_dir", "backup_on", "backup_audio", "backup_video", "backup_snooze", "ui_lang", "ai_gpu"];
/// IA en ligne (`online_*` : clés, fournisseur, modèle, adresse) : ni sauvegardée
/// ni remplacée par une restauration, comme les réglages ci-dessus.
const LOCAL_PREFIX: &str = "online_";
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
    /// nom et avatar de l'apprenant (absents des sauvegardes d'avant le profil)
    #[serde(flatten)]
    pub user: user::Card,
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
    /// le nuage (ou le disque, le dossier) qui reçoit la sauvegarde
    pub place: Place,
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
    /// nom et avatar de l'apprenant
    #[serde(flatten)]
    pub user: user::Card,
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

// ---------- nuages installés : Dropbox, Google Drive, OneDrive… ----------

/// Un nuage de ce Mac où la sauvegarde peut aller.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Place {
    /// "icloud", "dropbox", "gdrive", "onedrive", "box", "proton", "pcloud",
    /// "nextcloud", "synology", "mega", "cloud" (autre service), "drive" (disque
    /// externe), "folder" (dossier ordinaire)
    pub kind: String,
    /// nom du service (« Google Drive »), du disque, ou du dossier
    pub name: String,
    /// compte (adresse, équipe) : il peut y en avoir plusieurs pour un même service
    pub account: Option<String>,
    /// dossier qui recevra « Lumen » (à passer par `place_dir`) ; vide : iCloud Drive
    pub path: String,
}

/// Services qui se rangent dans ~/Library/CloudStorage (« GoogleDrive-lea@gmail.com »),
/// avec leur ordre de présentation.
const PROVIDERS: [(&str, &str, &str); 9] = [
    ("Dropbox", "dropbox", "Dropbox"),
    ("GoogleDrive", "gdrive", "Google Drive"),
    ("OneDrive", "onedrive", "OneDrive"),
    ("Box", "box", "Box"),
    ("ProtonDrive", "proton", "Proton Drive"),
    ("pCloud", "pcloud", "pCloud"),
    ("Nextcloud", "nextcloud", "Nextcloud"),
    ("SynologyDrive", "synology", "Synology Drive"),
    ("MEGA", "mega", "MEGA"),
];

/// Dossiers des anciennes versions de ces services, directement dans le dossier personnel.
const HOME_FOLDERS: [(&str, &str); 5] = [("Dropbox", "dropbox"), ("Google Drive", "gdrive"), ("OneDrive", "onedrive"), ("pCloud Drive", "pcloud"), ("Nextcloud", "nextcloud")];

/// Windows : dossiers que ces services créent dans le dossier personnel (en plus des précédents).
const WINDOWS_FOLDERS: [(&str, &str); 3] = [("Box", "box"), ("SynologyDrive", "synology"), ("MEGA", "mega")];

/// « Mon Drive » selon la langue du Mac : Google Drive n'accepte rien à la racine d'un compte.
const MY_DRIVE: [&str; 14] = [
    "My Drive", "Mon Drive", "Meine Ablage", "Mi unidad", "Il mio Drive", "Meu Drive", "Mijn Drive", "Min enhet", "Mit drev", "Mój dysk", "Мой диск",
    "マイドライブ", "내 드라이브", "我的云端硬盘",
];

fn provider(kind: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    PROVIDERS.iter().find(|p| p.1 == kind)
}

/// Service et compte d'un dossier de ~/Library/CloudStorage : « OneDrive-Personal »,
/// « ProtonDrive-lea@proton.me-folder », « Box-Box ».
fn cloud_storage_place(entry: &str, path: PathBuf) -> Place {
    let (prefix, rest) = entry.split_once('-').unwrap_or((entry, ""));
    let rest = rest.strip_suffix("-folder").unwrap_or(rest);
    let (kind, name) = match PROVIDERS.iter().find(|p| p.0.eq_ignore_ascii_case(prefix)) {
        Some(p) => (p.1.to_string(), p.2.to_string()),
        None => ("cloud".to_string(), prefix.to_string()),
    };
    let account = match rest {
        "" => None,
        r if r == prefix || r == name => None,
        "Personal" => Some(crate::i18n::t("Personnel", "Personal").to_string()),
        r => Some(r.to_string()),
    };
    Place { kind, name, account, path: path.display().to_string() }
}

/// Dossiers de Dropbox d'après son propre fichier de réglages (dossier déplacé,
/// compte personnel et compte d'équipe).
fn dropbox_paths(home: &Path) -> Vec<(String, bool)> {
    // Windows : dans AppData (itinérant ou local) plutôt que dans le dossier personnel
    let mut files = vec![home.join(".dropbox").join("info.json")];
    if cfg!(windows) {
        files.extend(["APPDATA", "LOCALAPPDATA"].iter().filter_map(|k| std::env::var_os(k)).map(|d| PathBuf::from(d).join("Dropbox").join("info.json")));
    }
    let Some(raw) = files.iter().find_map(|f| fs::read_to_string(f).ok()) else { return Vec::new() };
    let Ok(info) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    ["personal", "business"]
        .iter()
        .filter_map(|k| Some((info.get(*k)?.get("path")?.as_str()?.to_string(), *k == "business")))
        .collect()
}

/// Nuages installés sur ce Mac, iCloud Drive en tête. Rien n'est lu à
/// l'intérieur des nuages (macOS demanderait l'accès) : seulement leurs noms.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if icloud_drive().is_some() {
        out.push(Place { kind: "icloud".into(), name: "iCloud Drive".into(), account: None, path: String::new() });
    }
    let Some(home) = home() else { return out };
    let mut found: Vec<Place> = Vec::new();
    if let Ok(dir) = fs::read_dir(home.join("Library/CloudStorage")) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()) {
                continue;
            }
            found.push(cloud_storage_place(&name, e.path()));
        }
    }
    for (path, business) in dropbox_paths(&home) {
        if Path::new(&path).is_dir() && !found.iter().any(|p| p.path == path) {
            let account = business.then(|| crate::i18n::t("Équipe", "Team").to_string());
            found.push(Place { kind: "dropbox".into(), name: "Dropbox".into(), account, path });
        }
    }
    // anciennes versions (dossier dans le dossier personnel), si le service n'est pas déjà là
    for (folder, kind) in HOME_FOLDERS {
        let p = home.join(folder);
        if !found.iter().any(|f| f.kind == kind) && p.is_dir() {
            let name = provider(kind).map_or(folder, |p| p.2).to_string();
            found.push(Place { kind: kind.into(), name, account: None, path: p.display().to_string() });
        }
    }
    if cfg!(windows) {
        windows_places(&home, &mut found);
    }
    // Google Drive sur les macOS d'avant le dossier CloudStorage
    let volume = Path::new("/Volumes/GoogleDrive");
    if !found.iter().any(|f| f.kind == "gdrive") && volume.is_dir() {
        found.push(Place { kind: "gdrive".into(), name: "Google Drive".into(), account: None, path: volume.display().to_string() });
    }
    let rank = |k: &str| PROVIDERS.iter().position(|p| p.1 == k).unwrap_or(PROVIDERS.len());
    found.sort_by(|a, b| rank(&a.kind).cmp(&rank(&b.kind)).then_with(|| a.name.cmp(&b.name)).then_with(|| a.account.cmp(&b.account)));
    out.extend(found);
    out
}

/// Windows : OneDrive (personnel et professionnel, d'après les variables que
/// pose son app), Google Drive (lecteur G: ou dossier « Mon Drive » en miroir),
/// Box, Synology Drive, MEGA.
fn windows_places(home: &Path, found: &mut Vec<Place>) {
    // un emplacement qui existe, et une seule fois
    fn add(found: &mut Vec<Place>, p: Place) {
        if Path::new(&p.path).is_dir() && !found.iter().any(|f| f.path == p.path) {
            found.push(p);
        }
    }
    for (var, business) in [("OneDriveConsumer", false), ("OneDrive", false), ("OneDriveCommercial", true)] {
        if let Some(dir) = std::env::var_os(var) {
            let path = PathBuf::from(dir);
            let account = business.then(|| onedrive_account(&path)).flatten();
            add(found, Place { kind: "onedrive".into(), name: "OneDrive".into(), account, path: path.display().to_string() });
        }
    }
    if let Ok(dir) = fs::read_dir(home) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if name.starts_with("OneDrive - ") {
                add(found, Place { kind: "onedrive".into(), name: "OneDrive".into(), account: onedrive_account(&p), path: p.display().to_string() });
            } else if MY_DRIVE.contains(&name.as_str()) {
                // Google Drive en miroir : le dossier « Mon Drive » lui-même
                add(found, Place { kind: "gdrive".into(), name: "Google Drive".into(), account: None, path: p.display().to_string() });
            }
        }
    }
    for letter in drive_letters() {
        let root = PathBuf::from(format!("{letter}:\\"));
        if gdrive_root(&root) {
            add(found, Place { kind: "gdrive".into(), name: "Google Drive".into(), account: None, path: root.display().to_string() });
        }
    }
    for (folder, kind) in WINDOWS_FOLDERS {
        if !found.iter().any(|f| f.kind == kind) {
            let name = provider(kind).map_or(folder, |p| p.2).to_string();
            add(found, Place { kind: kind.into(), name, account: None, path: home.join(folder).display().to_string() });
        }
    }
}

/// Lecteurs locaux de ce PC (Google Drive en est un), sans le lecteur de Windows.
#[cfg(windows)]
fn drive_letters() -> Vec<char> {
    let system = std::env::var("SystemDrive").ok().and_then(|d| d.chars().next()).unwrap_or('C');
    crate::win::local_drives().into_iter().filter(|l| !l.eq_ignore_ascii_case(&system)).collect()
}

#[cfg(not(windows))]
fn drive_letters() -> Vec<char> {
    Vec::new()
}

/// « OneDrive - Contoso » : le compte professionnel (Contoso).
fn onedrive_account(path: &Path) -> Option<String> {
    let n = path.file_name()?.to_string_lossy().to_string();
    n.strip_prefix("OneDrive - ").map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
}

/// Lecteur de Google Drive sous Windows (G: par défaut) : sa racine contient « Mon Drive ».
fn gdrive_root(p: &Path) -> bool {
    cfg!(windows) && p.parent().is_none() && MY_DRIVE.iter().any(|m| p.join(m).is_dir())
}

/// Le nuage, le disque ou le dossier d'un emplacement de sauvegarde (réglage
/// `backup_dir`), d'après son seul chemin.
pub fn place_of(custom: &str) -> Place {
    let custom = custom.trim();
    if custom.is_empty() {
        return Place { kind: "icloud".into(), name: "iCloud Drive".into(), account: None, path: String::new() };
    }
    let p = Path::new(custom);
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    // …/Library/CloudStorage/<service>-<compte>/…
    if let Some(i) = parts.windows(2).position(|w| w[0] == "Library" && w[1] == "CloudStorage") {
        if let Some(entry) = parts.get(i + 2) {
            let root: PathBuf = parts[..=i + 2].iter().collect();
            return cloud_storage_place(entry, root);
        }
    }
    if cfg!(windows) {
        if let Some(place) = windows_place_of(p, &parts) {
            return place;
        }
    }
    if let Some(home) = home() {
        if let Ok(rel) = p.strip_prefix(&home) {
            let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
            if let Some((folder, kind)) = HOME_FOLDERS.iter().find(|(f, _)| *f == first) {
                let name = provider(kind).map_or(*folder, |p| p.2).to_string();
                return Place { kind: (*kind).into(), name, account: None, path: home.join(folder).display().to_string() };
            }
        }
        for (path, business) in dropbox_paths(&home) {
            if p.starts_with(&path) {
                let account = business.then(|| crate::i18n::t("Équipe", "Team").to_string());
                return Place { kind: "dropbox".into(), name: "Dropbox".into(), account, path };
            }
        }
    }
    if parts.len() >= 3 && parts[1] == "Volumes" {
        let root: PathBuf = parts[..3].iter().collect();
        let (kind, name) = if parts[2] == "GoogleDrive" { ("gdrive", "Google Drive".to_string()) } else { ("drive", parts[2].clone()) };
        return Place { kind: kind.into(), name, account: None, path: root.display().to_string() };
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Place { kind: "folder".into(), name, account: None, path: custom.to_string() }
}

/// Windows : OneDrive (personnel ou « OneDrive - Contoso »), Google Drive (lecteur
/// ou « Mon Drive » en miroir), Box…, ou un autre lecteur (clé USB, disque).
fn windows_place_of(p: &Path, parts: &[String]) -> Option<Place> {
    let home = home();
    let rel = home.as_ref().and_then(|h| p.strip_prefix(h).ok());
    let first = rel.and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
    let under = |folder: &str| home.as_ref().map(|h| h.join(folder).display().to_string()).unwrap_or_default();
    if first == "OneDrive" || first.starts_with("OneDrive - ") {
        let path = under(&first);
        return Some(Place { kind: "onedrive".into(), name: "OneDrive".into(), account: onedrive_account(Path::new(&path)), path });
    }
    if MY_DRIVE.contains(&first.as_str()) {
        return Some(Place { kind: "gdrive".into(), name: "Google Drive".into(), account: None, path: under(&first) });
    }
    if let Some((folder, kind)) = WINDOWS_FOLDERS.iter().find(|(f, _)| *f == first) {
        let name = provider(kind).map_or(*folder, |p| p.2).to_string();
        return Some(Place { kind: (*kind).into(), name, account: None, path: under(folder) });
    }
    // « G:\Mon Drive\… » : composants « G: », « \ », « Mon Drive »
    let drive = parts.first().filter(|d| d.len() == 2 && d.ends_with(':'))?;
    let root = format!("{drive}\\");
    if parts.get(2).is_some_and(|d| MY_DRIVE.contains(&d.as_str())) {
        return Some(Place { kind: "gdrive".into(), name: "Google Drive".into(), account: None, path: root });
    }
    // un autre lecteur que celui de Windows : clé USB, disque externe
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    if !drive.eq_ignore_ascii_case(&system) {
        return Some(Place { kind: "drive".into(), name: crate::tr!("Disque {drive}", "Drive {drive}"), account: None, path: root });
    }
    None
}

/// Dossier à retenir (`backup_dir`) pour un nuage choisi. Google Drive
/// n'accepte rien à la racine d'un compte : on prend « Mon Drive ». C'est ici
/// que macOS peut demander l'accès au nuage, au moment où l'utilisateur le choisit.
pub fn place_dir(path: &str) -> Result<String, String> {
    let p = Path::new(path.trim());
    let gdrive = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("GoogleDrive-")) || p == Path::new("/Volumes/GoogleDrive") || gdrive_root(p);
    if !gdrive {
        return if p.is_dir() {
            Ok(p.display().to_string())
        } else {
            Err(crate::i18n::t("Ce dossier est introuvable. Le service est-il toujours installé et connecté ?", "This folder can't be found. Is the service still installed and signed in?").into())
        };
    }
    let not_ready = || -> String {
        crate::i18n::t(
            "Google Drive n'est pas encore prêt. Ouvrez Google Drive, connectez-vous, puis réessayez.",
            "Google Drive isn't ready yet. Open Google Drive, sign in, then try again.",
        )
        .into()
    };
    let mut dirs: Vec<String> = fs::read_dir(p)
        .map_err(|e| match e.kind() {
            io::ErrorKind::PermissionDenied if cfg!(windows) => crate::i18n::t(
                "Lumen n'a pas accès à Google Drive. Ouvrez Google Drive, vérifiez que vous êtes connecté, puis réessayez.",
                "Lumen has no access to Google Drive. Open Google Drive, check that you're signed in, then try again.",
            )
            .into(),
            io::ErrorKind::PermissionDenied => crate::i18n::t(
                "Lumen n'a pas accès à Google Drive. Autorisez-le dans Réglages Système › Confidentialité et sécurité › Fichiers et dossiers, puis réessayez.",
                "Lumen has no access to Google Drive. Allow it in System Settings › Privacy & Security › Files and Folders, then try again.",
            )
            .into(),
            _ => not_ready(),
        })?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    dirs.sort();
    let mine = MY_DRIVE.iter().find(|m| dirs.iter().any(|d| d == *m)).map(|m| m.to_string());
    // autre langue : le premier dossier qui n'est ni partagé ni un autre ordinateur
    let shared = |d: &str| {
        let d = d.to_lowercase();
        ["shared", "partag", "geteilt", "compartid", "condivis", "other computers", "autres ordinateurs", "computer"].iter().any(|w| d.contains(w))
    };
    let pick = mine.or_else(|| dirs.iter().find(|d| !shared(d)).cloned()).ok_or_else(not_ready)?;
    Ok(p.join(pick).display().to_string())
}

fn short_hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))[..12].to_string()
}

#[cfg(windows)]
fn hardware_uuid() -> Option<String> {
    crate::win::machine_guid()
}

#[cfg(not(windows))]
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
        // Windows : le nom de l'appareil (Paramètres › Système › Informations système)
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| if cfg!(windows) { crate::i18n::t("Ce PC", "This PC") } else { crate::i18n::t("Ce Mac", "This Mac") }.into())
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
    format!("{} ({key})", if !clean.is_empty() { &clean } else if cfg!(windows) { "PC" } else { "Mac" })
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

/// Nom du fichier d'un chemin écrit par un autre ordinateur : « /Users/…/a.m4a »
/// (Mac) comme « C:\\Users\\…\\a.m4a » (PC), quel que soit le système qui le lit.
fn stored_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or_default().to_string()
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
    let mut st = c.prepare("SELECT media_path, video_path, cover_path, source FROM lessons")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (audio, video, cover, source) = row?;
        if let Some(v) = video.filter(|v| Some(v) != audio.as_ref()) {
            // une vidéo importée d'un fichier (séparée de son son à l'allègement) ne se
            // retélécharge pas : elle part avec le son, comme avant d'être séparée
            let b = source.as_bytes();
            let from_file = source.starts_with('/') || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
            out.push((PathBuf::from(v), if from_file { Kind::Audio } else { Kind::Video }));
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
    let (profile, card) = {
        let src = Connection::open(db::path(data_dir))?;
        src.busy_timeout(Duration::from_secs(15))?;
        if !has_progress(&src)? {
            return Ok(None);
        }
        let p = ensure_profile(&src, device)?;
        src.execute("VACUUM INTO ?1", [snap.to_string_lossy()])?;
        (p, user::card(&src))
    };
    // 2. sans le cache des traductions ni les réglages propres à ce Mac
    let (counts, refs) = {
        let c = Connection::open(&snap)?;
        c.execute("DELETE FROM tcache", [])?;
        for k in LOCAL_ONLY {
            c.execute("DELETE FROM settings WHERE key=?1", [k])?;
        }
        c.execute("DELETE FROM settings WHERE substr(key, 1, ?2) = ?1", params![LOCAL_PREFIX, LOCAL_PREFIX.len() as i64])?;
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
        user: card,
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
            user: m.user,
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
            let name = stored_name(p);
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
        c.execute("DELETE FROM settings WHERE substr(key, 1, ?2) = ?1", params![LOCAL_PREFIX, LOCAL_PREFIX.len() as i64])?;
        for (k, v) in db::settings_all(&l)? {
            if k.starts_with(LOCAL_PREFIX) {
                db::setting_set(&c, &k, &v)?;
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
    if cfg!(windows) {
        return crate::i18n::t(
            "Choisissez où sauvegarder : OneDrive, un autre nuage ou un dossier (Réglages › Sauvegarde).",
            "Choose where to back up: OneDrive, another cloud or a folder (Settings › Backup).",
        )
        .into();
    }
    crate::i18n::t(
        "iCloud Drive n'est pas activé sur ce Mac. Activez-le dans Réglages Système › votre nom › iCloud, ou choisissez un autre dossier.",
        "iCloud Drive isn't turned on on this Mac. Turn it on in System Settings › your name › iCloud, or choose another folder.",
    )
    .into()
}

/// Message compréhensible pour les erreurs de fichier les plus courantes,
/// selon l'emplacement : iCloud Drive, un autre nuage, un disque ou un dossier.
fn io_message(io: &io::Error, place: &Place) -> Option<String> {
    if matches!(io.raw_os_error(), Some(28) | Some(112)) {
        return Some(if cfg!(windows) { crate::i18n::t("Il n'y a plus assez d'espace disque sur ce PC.", "There isn't enough disk space left on this PC.") } else { crate::i18n::t("Il n'y a plus assez d'espace disque sur ce Mac.", "There isn't enough disk space left on this Mac.") }.into());
    }
    let cloud = !matches!(place.kind.as_str(), "drive" | "folder");
    let name = &place.name;
    match io.kind() {
        io::ErrorKind::PermissionDenied if cloud && cfg!(windows) => Some(crate::tr!(
            "Lumen n'a pas accès à {name}. Ouvrez {name}, vérifiez que vous êtes connecté, puis réessayez.",
            "Lumen has no access to {name}. Open {name}, check that you're signed in, then try again."
        )),
        io::ErrorKind::PermissionDenied if cloud => Some(crate::tr!(
            "Lumen n'a pas accès à {name}. Autorisez-le dans Réglages Système › Confidentialité et sécurité › Fichiers et dossiers, puis réessayez.",
            "Lumen has no access to {name}. Allow it in System Settings › Privacy & Security › Files and Folders, then try again."
        )),
        io::ErrorKind::PermissionDenied => Some(crate::i18n::t("Lumen ne peut pas écrire dans ce dossier. Choisissez-en un autre.", "Lumen can't write to this folder. Choose another one.").into()),
        // iCloud Drive absent : `no_place` le dit déjà
        io::ErrorKind::NotFound if place.kind == "icloud" => None,
        io::ErrorKind::NotFound if cloud && cfg!(windows) => Some(crate::tr!(
            "Le dossier {name} est introuvable. {name} est-il toujours installé et connecté sur ce PC ?",
            "The {name} folder can't be found. Is {name} still installed and signed in on this PC?"
        )),
        io::ErrorKind::NotFound if cloud => Some(crate::tr!(
            "Le dossier {name} est introuvable. {name} est-il toujours installé et connecté sur ce Mac ?",
            "The {name} folder can't be found. Is {name} still installed and signed in on this Mac?"
        )),
        io::ErrorKind::NotFound => Some(crate::i18n::t("Le dossier de sauvegarde est introuvable. Le disque est-il branché ?", "The backup folder can't be found. Is the drive connected?").into()),
        _ => None,
    }
}

/// Message d'erreur lisible ; `what` : (« Sauvegarde impossible », « Backup failed »)…
/// Les messages écrits pour l'utilisateur (`bail!`) passent tels quels.
fn friendly(e: &anyhow::Error, place: &Place, what: (&'static str, &'static str)) -> String {
    let what = crate::i18n::t(what.0, what.1);
    if let Some(io) = e.chain().find_map(|c| c.downcast_ref::<io::Error>()) {
        return io_message(io, place).unwrap_or_else(|| crate::tr!("{what} : {io}", "{what}: {io}"));
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
    let place = place_of(&p.dir);
    let res = match root(&p.dir) {
        None => Err(no_place()),
        Some(root) => run(&state.data_dir, &Options { root, audio: p.audio, video: p.video }).map_err(|e| friendly(&e, &place, ("Sauvegarde impossible", "Backup failed"))),
    };
    let mut error = state.backup.error.lock();
    match res {
        Ok(out) => {
            state.backup.seen.store(changes, Ordering::SeqCst);
            *error = out.and_then(|(_, w)| w).map(|w| {
                let why = io_message(&w, &place).unwrap_or_else(|| w.to_string());
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
        place: place_of(&p.dir),
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
    list(&state.data_dir, &root, p.profile.as_deref()).map_err(|e| friendly(&e, &place_of(&p.dir), ("Lecture des sauvegardes impossible", "Couldn't read the backups")))
}

pub fn restore_for(state: &AppState, key: &str, day: Option<&str>, on: impl Fn(ImportEvent)) -> Result<Restored, String> {
    let _running = state.backup.wait();
    let p = prefs(&state.db.lock());
    let root = root(&p.dir).ok_or_else(no_place)?;
    let out = restore(&state.db, &state.data_dir, &root, key, day, on).map_err(|e| friendly(&e, &place_of(&p.dir), ("Restauration impossible", "Restore failed")))?;
    // la progression restaurée rejoint la sauvegarde de ce Mac à la prochaine occasion
    state.backup.seen.store(u64::MAX, Ordering::SeqCst);
    state.backup.last_try.store(0, Ordering::SeqCst);
    *state.backup.error.lock() = None;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_names_from_any_computer() {
        assert_eq!(stored_name("/Users/lea/Library/Application Support/app.lumen.reader/media/20260101.audio.m4a"), "20260101.audio.m4a");
        assert_eq!(stored_name("C:\\Users\\Léa\\AppData\\Roaming\\app.lumen.reader\\media\\20260101.cover.jpg"), "20260101.cover.jpg");
        assert_eq!(stored_name("a.m4a"), "a.m4a");
    }

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
    fn local_video_travels_with_its_sound() {
        let data = temp("localvideo");
        let c = db::open(&db::path(&data)).unwrap();
        let mk = |source: &str, n: u32| {
            db::lesson_create(&c, &db::NewLesson {
                lang: "it".into(), title: format!("v{n}"), collection: String::new(), kind: "video".into(),
                source: source.into(), text: "Il faro è alto.".into(),
                media_path: Some(format!("/m/{n}.light.m4a")), timings: None, video_path: Some(format!("/m/{n}.light.mp4")),
            })
            .unwrap();
        };
        // vidéo du Mac ou du PC séparée de son son, vidéo de YouTube
        mk("/Users/lea/Films/cours.mov", 1);
        mk("C:\\Users\\lea\\cours.mp4", 2);
        mk("https://www.youtube.com/watch?v=x", 3);
        let refs = media_refs(&c).unwrap();
        let kind = |name: &str| refs.iter().find(|(p, _)| p.ends_with(name)).map(|r| r.1);
        assert_eq!(kind("1.light.mp4"), Some(Kind::Audio));
        assert_eq!(kind("2.light.mp4"), Some(Kind::Audio));
        assert_eq!(kind("3.light.mp4"), Some(Kind::Video));
        drop(c);
        let _ = fs::remove_dir_all(data);
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
        db::setting_set(&ca, "online_on", "1").unwrap();
        db::setting_set(&ca, "online_key_deepseek", "sk-secret").unwrap();
        db::setting_set(&ca, "langs", "it").unwrap();
        // le profil de l'apprenant : il voyage avec la progression
        db::setting_set(&ca, "user_name", "Léa").unwrap();
        db::setting_set(&ca, "user_avatar", "photo:34:7").unwrap();
        db::setting_set(&ca, "user_photo", "data:image/jpeg;base64,AAAA").unwrap();
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
        // ni l'IA en ligne de ce Mac (sa clé, son choix)
        assert_eq!((s.get("online_on"), s.get("online_key_deepseek")), (None, None));

        // second Mac : sa propre clé LingQ, sauvegarde coupée
        let b = temp("b");
        fs::write(b.join("device-id"), "bbbbbbbbbbbb").unwrap();
        let cb = db::open(&db::path(&b)).unwrap();
        db::setting_set(&cb, "lingq_key", "autre").unwrap();
        db::setting_set(&cb, "online_key_mistral", "m-key").unwrap();
        db::setting_set(&cb, "backup_on", "0").unwrap();
        let live = Mutex::new(cb);
        let found = list(&b, &root, None).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].mine && !found[0].this_device && !found[0].newer);
        assert_eq!(found[0].counts.known, 2);
        // on reconnaît sa sauvegarde à son nom et à son avatar
        assert_eq!((found[0].user.name.as_str(), found[0].user.avatar.as_str(), found[0].user.photo.as_str()), ("Léa", "photo:34:7", "data:image/jpeg;base64,AAAA"));

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
        assert_eq!((db::setting(&cb, "online_key_mistral").as_deref(), db::setting(&cb, "online_on")), (Some("m-key"), None));
        assert_eq!(db::setting(&cb, "backup_on").as_deref(), Some("0"));
        assert_eq!(db::setting(&cb, "profile_id"), Some(m.profile.clone()));
        assert_eq!(db::setting(&cb, "user_name").as_deref(), Some("Léa"));
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
    // liens symboliques du Mac : essai réservé au Mac
    #[cfg(unix)]
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

    // les nuages tels que Windows les range (sur un PC seulement)
    #[cfg(windows)]
    #[test]
    fn windows_cloud_places() {
        let home = home().unwrap();
        let p = place_of(&home.join("OneDrive - Contoso").join("Lumen").display().to_string());
        assert_eq!((p.kind.as_str(), p.account.as_deref()), ("onedrive", Some("Contoso")));
        assert_eq!(place_of(&home.join("OneDrive").join("Documents").display().to_string()).kind, "onedrive");
        assert_eq!(place_of(&home.join("Mon Drive").join("Lumen").display().to_string()).kind, "gdrive");
        let p = place_of("G:\\Mon Drive\\Lumen");
        assert_eq!((p.kind.as_str(), p.path.as_str()), ("gdrive", "G:\\"));
        assert_eq!(place_of("Z:\\Sauvegardes").kind, "drive");
        assert_eq!(place_of(&home.join("Sauvegardes").display().to_string()).kind, "folder");
        assert_eq!(onedrive_account(Path::new("C:\\Users\\x\\OneDrive - Contoso")).as_deref(), Some("Contoso"));
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

    // les nuages tels que macOS les range (chemins du Mac)
    #[cfg(not(windows))]
    #[test]
    fn cloud_places() {
        // dossiers de ~/Library/CloudStorage : service et compte
        let cs = Path::new("/Users/x/Library/CloudStorage");
        let p = cloud_storage_place("GoogleDrive-lea@gmail.com", cs.join("GoogleDrive-lea@gmail.com"));
        assert_eq!((p.kind.as_str(), p.name.as_str(), p.account.as_deref()), ("gdrive", "Google Drive", Some("lea@gmail.com")));
        let p = cloud_storage_place("Dropbox", cs.join("Dropbox"));
        assert_eq!((p.kind.as_str(), p.account.as_deref()), ("dropbox", None));
        assert_eq!(cloud_storage_place("Box-Box", cs.join("Box-Box")).account, None);
        assert_eq!(cloud_storage_place("ProtonDrive-lea@proton.me-folder", cs.join("x")).account.as_deref(), Some("lea@proton.me"));
        assert_eq!(cloud_storage_place("OneDrive-Contoso", cs.join("x")).account.as_deref(), Some("Contoso"));
        let p = cloud_storage_place("Lumière-moi", cs.join("x"));
        assert_eq!((p.kind.as_str(), p.name.as_str()), ("cloud", "Lumière"));

        // l'emplacement retenu, d'après son seul chemin
        assert_eq!(place_of("").kind, "icloud");
        let p = place_of("/Users/x/Library/CloudStorage/GoogleDrive-lea@gmail.com/Mon Drive");
        assert_eq!((p.kind.as_str(), p.path.as_str()), ("gdrive", "/Users/x/Library/CloudStorage/GoogleDrive-lea@gmail.com"));
        assert_eq!(place_of("/Users/x/Library/CloudStorage/OneDrive-Personal/Documents").kind, "onedrive");
        let p = place_of("/Volumes/Clé USB/Sauvegardes");
        assert_eq!((p.kind.as_str(), p.name.as_str(), p.path.as_str()), ("drive", "Clé USB", "/Volumes/Clé USB"));
        let p = place_of("/Users/x/Sauvegardes");
        assert_eq!((p.kind.as_str(), p.name.as_str()), ("folder", "Sauvegardes"));
        if let Some(home) = home() {
            assert_eq!(place_of(&home.join("Dropbox/Perso").display().to_string()).kind, "dropbox");
        }

        // Google Drive : « Mon Drive » (dans la langue du Mac), jamais la racine du compte
        let tmp = temp("gdrive").join("GoogleDrive-lea@gmail.com");
        fs::create_dir_all(tmp.join("Drive partagés")).unwrap();
        fs::create_dir_all(tmp.join(".shortcut-targets-by-id")).unwrap();
        assert!(place_dir(&tmp.display().to_string()).is_err());
        fs::create_dir_all(tmp.join("Mein Laufwerk")).unwrap();
        assert_eq!(place_dir(&tmp.display().to_string()).unwrap(), tmp.join("Mein Laufwerk").display().to_string());
        fs::create_dir_all(tmp.join("Mon Drive")).unwrap();
        assert_eq!(place_dir(&tmp.display().to_string()).unwrap(), tmp.join("Mon Drive").display().to_string());
        // autres nuages : le dossier tel quel, s'il existe
        let dropbox = tmp.parent().unwrap().join("Dropbox");
        assert!(place_dir(&dropbox.display().to_string()).is_err());
        fs::create_dir_all(&dropbox).unwrap();
        assert_eq!(place_dir(&dropbox.display().to_string()).unwrap(), dropbox.display().to_string());
    }
}
