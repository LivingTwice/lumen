use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
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
}
