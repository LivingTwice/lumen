use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::{ai::Engine, dict::Dicts};

pub struct AppState {
    pub data_dir: PathBuf,
    pub db: Mutex<Connection>,
    pub dicts: Dicts,
    pub ai: Engine,
    /// téléchargements de modèles et import LingQ en cours (annulables)
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// voix naturelle : une prononciation à la fois ; une nouvelle demande
    /// rend caduques les préparations en attente
    pub voice_lock: tokio::sync::Mutex<()>,
    pub voice_epoch: AtomicU64,
    /// sauvegarde de la progression (iCloud Drive ou dossier choisi)
    pub backup: crate::backup::Tracker,
    /// Découvrir : ce que les sources proposent (cache à part, `discover.db`)
    pub discover: crate::discover::Store,
    /// niveau estimé par langue, avec l'empreinte des mots connus qui l'a donné
    pub levels: Mutex<HashMap<String, (String, crate::level::Estimate)>>,
}
