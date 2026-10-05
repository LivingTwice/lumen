//! Commandes appelées par l'interface (invoke).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::{Channel, Response};
use tauri::{Emitter, State};

use crate::ai::{self, Priority};
use crate::backup;
use crate::db::{self, LessonPatch, NewLesson, PlaylistPatch, TermQuery, TermUpdate};
use crate::dict::DictResult;
use crate::discover;
use crate::link;
use crate::lingq;
use crate::lyrics;
use crate::level;
use crate::podcast;
use crate::search;
use crate::media::{self, ImportEvent};
use crate::models::{self, DownloadEvent};
use crate::online;
use crate::i18n::{self, t};
use crate::state::AppState;
use crate::text;
use crate::user;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    hex::encode(h.finalize())
}

// ---------- application et réglages ----------

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
    platform: String,
    ytdlp: bool,
    transcriber: bool,
    dict_langs: Vec<String>,
}

#[tauri::command]
pub async fn app_info(state: State<'_, AppState>) -> R<AppInfo> {
    let langs = state.dicts.langs();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.data_dir.display().to_string(),
        platform: std::env::consts::OS.to_string(),
        ytdlp: crate::tools::status(&state.data_dir).0,
        transcriber: media::sidecar_path().is_ok(),
        dict_langs: langs,
    })
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> R<HashMap<String, String>> {
    db::settings_all(&state.db.lock()).map_err(err)
}

#[tauri::command]
pub async fn settings_set(app: tauri::AppHandle, state: State<'_, AppState>, key: String, value: String) -> R<()> {
    db::setting_set(&state.db.lock(), &key, &value).map_err(err)?;
    if key == "ui_lang" {
        i18n::set(&value);
    }
    // Windows : carte graphique ou processeur ; le modèle est rechargé à la demande suivante
    if key == "ai_gpu" {
        ai::set_gpu(value != "0");
        state.ai.unload();
    }
    // nouvel objectif du jour : une journée déjà au-dessus est acquise
    if key == "daily_goal" {
        db::goal_refresh(&state.db.lock()).map_err(err)?;
    }
    // les dictionnaires des langues étudiées, dans la langue de l'interface, arrivent en arrière-plan
    if key == "ui_lang" || key == "langs" {
        let langs = db::setting(&state.db.lock(), "langs").unwrap_or_default();
        for lang in langs.split(',').filter(|l| state.dicts.missing(l)) {
            fetch_dict(&app, lang);
        }
    }
    Ok(())
}

/// Télécharge en arrière-plan le dictionnaire d'une langue, dans la langue de l'interface.
fn fetch_dict(app: &tauri::AppHandle, lang: &str) {
    use tauri::Manager;
    let (app, lang) = (app.clone(), lang.to_string());
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        // prêt ou en échec (hors ligne) : l'interface relit l'état dans les deux cas
        let _ = st.dicts.fetch(&lang).await;
        let _ = app.emit("dict", &lang);
    });
}

// ---------- leçons ----------

#[tauri::command]
pub async fn lessons_list(state: State<'_, AppState>, lang: String) -> R<Vec<db::LessonSummary>> {
    db::lessons_list(&state.db.lock(), &lang).map_err(err)
}

#[derive(Serialize)]
pub struct OpenedLesson {
    lesson: db::Lesson,
    tokens: Vec<text::Token>,
    terms: HashMap<String, db::Term>,
}

#[tauri::command]
pub async fn lesson_open(app: tauri::AppHandle, state: State<'_, AppState>, id: i64) -> R<OpenedLesson> {
    let conn = state.db.lock();
    let lesson = db::lesson_get(&conn, id).map_err(err)?;
    let tokens = text::tokenize(&lesson.text, &lesson.lang);
    let keys: Vec<String> = tokens.iter().filter(|t| t.w).map(|t| t.k.clone()).collect();
    let terms = db::terms_for_keys(&conn, &lesson.lang, &keys).map_err(err)?;
    let lang = lesson.lang.clone();
    drop(conn);
    // prépare le dictionnaire en arrière-plan (ou le télécharge)
    if state.dicts.available(&lang) {
        state.dicts.warm(&lang);
    } else if state.dicts.missing(&lang) {
        fetch_dict(&app, &lang);
    }
    Ok(OpenedLesson { lesson, tokens, terms })
}

#[tauri::command]
pub async fn lesson_create(state: State<'_, AppState>, lesson: NewLesson) -> R<i64> {
    if lesson.text.trim().is_empty() {
        return Err(t("Le texte est vide.", "The text is empty.").into());
    }
    db::lesson_create(&state.db.lock(), &lesson).map_err(err)
}

#[tauri::command]
pub async fn lesson_update(state: State<'_, AppState>, id: i64, patch: LessonPatch) -> R<()> {
    db::lesson_update(&state.db.lock(), id, &patch).map_err(err)
}

#[tauri::command]
pub async fn lesson_delete(state: State<'_, AppState>, id: i64) -> R<()> {
    let files = db::lesson_delete(&state.db.lock(), id).map_err(err)?;
    for m in files {
        let p = PathBuf::from(m);
        if p.starts_with(media::media_dir(&state.data_dir)) {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(())
}

/// Enregistre l'image de couverture choisie (déjà réduite par l'interface),
/// ou la retire si `data` est vide. Renvoie le chemin de la nouvelle image.
#[tauri::command]
pub async fn lesson_set_cover(state: State<'_, AppState>, id: i64, data: Option<Vec<u8>>, ext: Option<String>) -> R<Option<String>> {
    let media = media::media_dir(&state.data_dir);
    let new_path = match data {
        Some(bytes) if !bytes.is_empty() => {
            if bytes.len() > 30_000_000 {
                return Err(t("Cette image est trop lourde (30 Mo au plus).", "This image is too large (30 MB at most).").into());
            }
            let ext = ext.unwrap_or_default().to_lowercase();
            let ext = if ["jpg", "jpeg", "png", "webp", "gif", "heic", "avif"].contains(&ext.as_str()) { ext } else { "jpg".into() };
            std::fs::create_dir_all(&media).map_err(err)?;
            let p = media.join(format!("{}.cover.{ext}", media::new_stem()));
            std::fs::write(&p, bytes).map_err(|e| tr!("Image impossible à enregistrer : {e}", "Couldn't save the image: {e}"))?;
            Some(p.display().to_string())
        }
        _ => None,
    };
    let old = db::lesson_set_cover(&state.db.lock(), id, new_path.as_deref()).map_err(err)?;
    if let Some(old) = old.map(PathBuf::from) {
        if old.starts_with(&media) {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(new_path)
}

// ---------- playlists ----------

#[tauri::command]
pub async fn playlists_list(state: State<'_, AppState>, lang: String) -> R<Vec<db::Playlist>> {
    db::playlists_list(&state.db.lock(), &lang).map_err(err)
}

#[tauri::command]
pub async fn playlist_create(state: State<'_, AppState>, lang: String, name: String, lessons: Vec<i64>) -> R<i64> {
    db::playlist_create(&mut state.db.lock(), &lang, &name, &lessons).map_err(err)
}

#[tauri::command]
pub async fn playlist_update(state: State<'_, AppState>, id: i64, patch: PlaylistPatch) -> R<()> {
    db::playlist_update(&mut state.db.lock(), id, &patch).map_err(err)
}

#[tauri::command]
pub async fn playlist_delete(state: State<'_, AppState>, id: i64) -> R<()> {
    db::playlist_delete(&state.db.lock(), id).map_err(err)
}

// ---------- vocabulaire ----------

#[tauri::command]
pub async fn term_set(state: State<'_, AppState>, update: TermUpdate) -> R<()> {
    db::term_set(&state.db.lock(), &update).map_err(err)
}

#[tauri::command]
pub async fn terms_mark_known(state: State<'_, AppState>, lang: String, keys: Vec<String>, words_read: i64) -> R<i64> {
    db::terms_mark_known(&mut state.db.lock(), &lang, &keys, words_read).map_err(err)
}

#[derive(Serialize)]
pub struct TermPage {
    items: Vec<db::Term>,
    total: i64,
}

#[tauri::command]
pub async fn terms_list(state: State<'_, AppState>, query: TermQuery) -> R<TermPage> {
    let (items, total) = db::terms_list(&state.db.lock(), &query).map_err(err)?;
    Ok(TermPage { items, total })
}

#[tauri::command]
pub async fn stats(state: State<'_, AppState>, lang: String) -> R<db::Stats> {
    db::stats(&state.db.lock(), &lang).map_err(err)
}

#[tauri::command]
pub async fn activity_add(state: State<'_, AppState>, lang: String, words_read: i64, listen_secs: i64, learn_secs: Option<i64>) -> R<Option<db::GoalReached>> {
    db::activity_add(&state.db.lock(), &lang, words_read, listen_secs, learn_secs.unwrap_or(0)).map_err(err)
}

#[tauri::command]
pub async fn export_vocab(state: State<'_, AppState>, lang: String, path: String) -> R<()> {
    let csv = db::export_csv(&state.db.lock(), &lang).map_err(err)?;
    std::fs::write(path, csv).map_err(err)
}

// ---------- dictionnaire ----------

/// État du dictionnaire d'une langue (prêt, en téléchargement) ; le télécharge s'il manque.
#[tauri::command]
pub async fn dict_status(app: tauri::AppHandle, state: State<'_, AppState>, lang: String) -> R<crate::dict::DictStatus> {
    if state.dicts.missing(&lang) {
        fetch_dict(&app, &lang);
    }
    Ok(state.dicts.status(&lang))
}

/// `after` : la suite de la phrase, pour le japonais et le vietnamien (mots de plusieurs jetons).
#[tauri::command]
pub async fn dict_lookup(app: tauri::AppHandle, state: State<'_, AppState>, lang: String, word: String, after: Option<String>) -> R<DictResult> {
    if state.dicts.missing(&lang) {
        fetch_dict(&app, &lang);
        return Ok(DictResult { pending: true, ..Default::default() });
    }
    if !state.dicts.available(&lang) {
        return Ok(DictResult::default());
    }
    state.dicts.lookup_ctx(i18n::native(), &lang, &word, after.as_deref().unwrap_or("")).map_err(err)
}

// ---------- IA (sur ce Mac ou en ligne) ----------

fn active_model(state: &AppState, kind: &str) -> R<&'static models::ModelInfo> {
    let key = if kind == "llm" { "llm_model" } else { "asr_model" };
    let default = if kind == "llm" { "qwen3.5-2b" } else { "whisper-turbo" };
    let id = db::setting(&state.db.lock(), key).unwrap_or_else(|| default.to_string());
    let m = models::find(&id).or_else(|| models::find(default)).ok_or(t("modèle inconnu", "unknown model"))?;
    if !models::installed(&state.data_dir, m) {
        // à défaut, n'importe quel modèle installé du même type
        if let Some(other) = models::CATALOG.iter().find(|x| x.kind == kind && models::installed(&state.data_dir, x)) {
            return Ok(other);
        }
        return Err(if kind == "llm" {
            t(
                "NO_MODEL:Aucun modèle de traduction n'est installé. Ouvrez Réglages › IA.",
                "NO_MODEL:No translation model is installed. Open Settings › AI.",
            )
            .into()
        } else {
            t(
                "NO_MODEL:Aucun modèle de transcription n'est installé. Ouvrez Réglages › IA.",
                "NO_MODEL:No transcription model is installed. Open Settings › AI.",
            )
            .into()
        });
    }
    Ok(m)
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Piece { text: String },
}

/// Qui répond : Qwen3.5 sur ce Mac, ou l'IA en ligne choisie par l'apprenant.
enum Brain {
    Local(&'static models::ModelInfo),
    Online(online::Config),
}

impl Brain {
    /// Identifiant dans le cache des traductions (celles de ce Mac gardent leur clé).
    fn id(&self) -> String {
        match self {
            Brain::Local(m) => m.id.to_string(),
            Brain::Online(c) => c.id(),
        }
    }
}

/// L'IA en ligne si l'apprenant l'a choisie pour ce rôle, sinon le modèle de ce Mac.
fn brain(state: &AppState, role: online::Role) -> R<Brain> {
    let cfg = online::config(&state.db.lock(), role);
    match cfg {
        Some(Ok(c)) => Ok(Brain::Online(c)),
        Some(Err(e)) => Err(e),
        None => Ok(Brain::Local(active_model(state, "llm")?)),
    }
}

/// Réponse simple (traduction, réécriture), envoyée au fil de l'eau.
async fn generate(st: &AppState, brain: &Brain, messages: &[(&str, String)], max_tokens: usize, priority: Priority, on_event: &Channel<AiEvent>) -> anyhow::Result<String> {
    let send = |piece: &str| {
        let _ = on_event.send(AiEvent::Piece { text: piece.to_string() });
        true
    };
    match brain {
        Brain::Online(cfg) => online::generate(cfg, messages, max_tokens, online::halter(&st.ai, &priority), send).await,
        Brain::Local(m) => {
            let path = models::path_of(&st.data_dir, m);
            tokio::task::block_in_place(|| st.ai.generate(&path, messages, max_tokens, priority, send))
        }
    }
}

/// Traduction : l'IA en ligne, et si elle est injoignable (pas de connexion),
/// le modèle de ce Mac s'il est installé. Renvoie la réponse et qui l'a donnée.
async fn translate(st: &AppState, brain: Brain, messages: &[(&str, String)], max_tokens: usize, epoch: u64, on_event: &Channel<AiEvent>) -> R<(String, Brain)> {
    match generate(st, &brain, messages, max_tokens, Priority::Interactive(epoch), on_event).await {
        Ok(out) => Ok((out, brain)),
        Err(e) if online::is_unreachable(&e) => match active_model(st, "llm") {
            Ok(m) => {
                let local = Brain::Local(m);
                let out = generate(st, &local, messages, max_tokens, Priority::Interactive(epoch), on_event).await.map_err(err)?;
                Ok((out, local))
            }
            Err(_) => Err(e.to_string()),
        },
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Serialize)]
pub struct WordAnswer {
    translation: String,
    note: String,
    cached: bool,
}

#[tauri::command]
pub async fn ai_word(
    state: State<'_, AppState>,
    lang: String,
    word: String,
    sentence: String,
    on_event: Channel<AiEvent>,
) -> R<WordAnswer> {
    let brain = brain(&state, online::Role::Words)?;
    // traductions en français (clé d'origine) ou en anglais (clé à part)
    let native = i18n::native();
    let version = if native == "en" { "w3en" } else { "w3" };
    let normalized = text::normalize_for(&word, &lang);
    let key_of = |b: &Brain| hash(&[version, &b.id(), &lang, &normalized, sentence.trim()]);
    if let Some(v) = db::cache_get(&state.db.lock(), &key_of(&brain)) {
        let (t, n) = v.split_once('\u{1f}').unwrap_or((&v, ""));
        return Ok(WordAnswer { translation: t.to_string(), note: n.to_string(), cached: true });
    }
    let epoch = state.ai.next_epoch();
    // indice du dictionnaire : forme de base et premiers sens
    let mut hint = String::new();
    if !word.contains(' ') && state.dicts.available(&lang) {
        if let Ok(d) = state.dicts.lookup(&lang, &word) {
            let glosses: Vec<String> = d.entries.iter().flat_map(|e| e.glosses.iter().take(2).cloned()).take(3).collect();
            if let Some(l) = &d.lemma {
                hint = tr!("forme de {l}", "form of {l}");
                if !glosses.is_empty() {
                    hint.push_str(t(" : ", ": "));
                }
            }
            hint.push_str(&glosses.join(" ; ").to_lowercase());
        }
    }
    let messages = ai::word_messages(native, &lang, &word, &sentence, &hint);
    let (raw, by) = translate(state.inner(), brain, &messages, 72, epoch, &on_event).await?;
    let (translation, note) = ai::parse_word_answer(&raw);
    if !translation.is_empty() {
        db::cache_put(&state.db.lock(), &key_of(&by), &format!("{translation}\u{1f}{note}"));
    }
    Ok(WordAnswer { translation, note, cached: false })
}

#[tauri::command]
pub async fn ai_sentence(
    state: State<'_, AppState>,
    lang: String,
    sentence: String,
    on_event: Channel<AiEvent>,
) -> R<String> {
    let brain = brain(&state, online::Role::Words)?;
    let native = i18n::native();
    let version = if native == "en" { "s1en" } else { "s1" };
    let key_of = |b: &Brain| hash(&[version, &b.id(), &lang, sentence.trim()]);
    if let Some(v) = db::cache_get(&state.db.lock(), &key_of(&brain)) {
        return Ok(v);
    }
    let epoch = state.ai.next_epoch();
    let messages = ai::sentence_messages(native, &lang, &sentence);
    let (out, by) = translate(state.inner(), brain, &messages, 260, epoch, &on_event).await?;
    let out = out.trim().trim_matches(['«', '»', '"']).trim().to_string();
    if !out.is_empty() {
        db::cache_put(&state.db.lock(), &key_of(&by), &out);
    }
    Ok(out)
}

#[tauri::command]
pub async fn ai_simplify(
    state: State<'_, AppState>,
    lang: String,
    text: String,
    level: String,
    on_event: Channel<AiEvent>,
) -> R<String> {
    let brain = brain(&state, online::Role::Chat)?;
    // découpe en blocs de paragraphes d'environ 1 200 caractères (4 000 en ligne :
    // les grands modèles gardent le fil d'un plus long passage)
    let size = if matches!(brain, Brain::Online(_)) { 4000 } else { 1200 };
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if !cur.is_empty() && cur.len() + para.len() > size {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    let st = state.inner();
    let mut result = String::new();
    for (i, chunk) in chunks.iter().take(12).enumerate() {
        if i > 0 {
            result.push_str("\n\n");
            let _ = on_event.send(AiEvent::Piece { text: "\n\n".into() });
        }
        let messages = ai::simplify_messages(i18n::native(), &lang, &level, chunk);
        let out = generate(st, &brain, &messages, size * 3 / 4, Priority::Background, &on_event).await.map_err(err)?;
        result.push_str(out.trim());
    }
    Ok(result)
}

#[tauri::command]
pub async fn ai_warmup(state: State<'_, AppState>) -> R<bool> {
    // tout part en ligne : inutile d'occuper la mémoire avec le modèle de ce Mac
    let online_all = {
        let c = state.db.lock();
        matches!(online::config(&c, online::Role::Words), Some(Ok(_))) && matches!(online::config(&c, online::Role::Chat), Some(Ok(_)))
    };
    if online_all {
        return Ok(false);
    }
    let m = match active_model(&state, "llm") {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };
    let path = models::path_of(&state.data_dir, m);
    if state.ai.is_loaded(&path) {
        return Ok(true);
    }
    let st = state.inner();
    tokio::task::block_in_place(|| st.ai.load(&path)).map_err(err)?;
    Ok(true)
}

/// Cartes graphiques que l'IA locale peut employer, et où calcule le modèle
/// chargé (Réglages › IA, sous Windows). La première lecture interroge Vulkan.
#[tauri::command]
pub async fn gpu_info(state: State<'_, AppState>) -> R<ai::GpuInfo> {
    let st = state.inner();
    tokio::task::block_in_place(|| ai::gpu_info(&st.ai)).map_err(err)
}

// ---------- chat ----------

/// Place de la leçon jointe et des échanges précédents dans la mémoire du modèle (en octets).
const CHAT_LESSON_BYTES: usize = 24_000;
const CHAT_HISTORY_BYTES: usize = 16_000;
/// En ligne : les grands modèles lisent bien plus long.
const ONLINE_LESSON_BYTES: usize = 64_000;
const ONLINE_HISTORY_BYTES: usize = 32_000;
/// Longueur maximale d'une réponse, réflexion non comprise (en jetons).
const CHAT_ANSWER_TOKENS: usize = 1600;

#[derive(Serialize)]
pub struct ChatThread {
    chat: db::ChatSummary,
    messages: Vec<db::ChatMessage>,
}

#[derive(serde::Deserialize)]
pub struct ChatOptions {
    /// réflexion du modèle avant de répondre
    think: bool,
    /// "low", "medium" ou "high" : longueur permise à la réflexion
    effort: String,
    /// passage lu dans la leçon jointe (position UTF-16), pour les longues leçons
    focus: Option<usize>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    Thought { text: String },
    Answer { text: String },
}

#[derive(Serialize)]
pub struct ChatReply {
    chat: db::ChatSummary,
    /// question et réponse enregistrées (aucune si l'arrêt est venu avant le premier mot)
    user: Option<db::ChatMessage>,
    assistant: Option<db::ChatMessage>,
    stopped: bool,
}

#[tauri::command]
pub async fn chats_list(state: State<'_, AppState>, lang: String) -> R<Vec<db::ChatSummary>> {
    db::chats_list(&state.db.lock(), &lang).map_err(err)
}

#[tauri::command]
pub async fn chat_open(state: State<'_, AppState>, id: i64) -> R<ChatThread> {
    let c = state.db.lock();
    Ok(ChatThread { chat: db::chat_get(&c, id).map_err(err)?, messages: db::chat_messages(&c, id).map_err(err)? })
}

#[tauri::command]
pub async fn chat_create(state: State<'_, AppState>, lang: String, lesson: Option<i64>) -> R<db::ChatSummary> {
    let c = state.db.lock();
    let id = db::chat_create(&c, &lang, lesson).map_err(err)?;
    db::chat_get(&c, id).map_err(err)
}

#[tauri::command]
pub async fn chat_update(state: State<'_, AppState>, id: i64, patch: db::ChatPatch) -> R<db::ChatSummary> {
    let c = state.db.lock();
    db::chat_update(&c, id, &patch).map_err(err)?;
    db::chat_get(&c, id).map_err(err)
}

#[tauri::command]
pub async fn chat_delete(state: State<'_, AppState>, id: i64) -> R<()> {
    db::chat_delete(&state.db.lock(), id).map_err(err)
}

/// Pose une question au chat. La réflexion (si elle est demandée) puis la
/// réponse arrivent au fil de l'eau ; la question et la réponse sont
/// enregistrées à la fin. Annulable avec `model_cancel("chat:<id>")` : ce qui
/// est déjà écrit est gardé.
#[tauri::command]
pub async fn chat_send(
    state: State<'_, AppState>,
    id: i64,
    text: String,
    options: ChatOptions,
    on_event: Channel<ChatEvent>,
) -> R<ChatReply> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(t("Écrivez d'abord votre question.", "Write your question first.").into());
    }
    let brain = brain(&state, online::Role::Chat)?;
    let online = matches!(brain, Brain::Online(_));
    // tout est lu d'un coup : aucun verrou n'est tenu pendant la génération
    let (chat, history, lesson, learner) = {
        let c = state.db.lock();
        let chat = db::chat_get(&c, id).map_err(err)?;
        let history: Vec<(String, String)> = db::chat_messages(&c, id)
            .map_err(err)?
            .into_iter()
            .filter(|m| !m.content.trim().is_empty())
            .map(|m| (m.role, m.content))
            .collect();
        let lesson = match chat.lesson_id {
            Some(l) => db::lesson_brief(&c, l).map_err(err)?,
            None => None,
        };
        let known = db::known_words(&c, &chat.lang).unwrap_or(0);
        // le profil de l'apprenant : son nom, ce qui le motive, ses centres d'intérêt ;
        // il ne quitte jamais ce Mac (en ligne, seul le nombre de mots connus part)
        let learner = if online { ai::Learner::knows(known) } else { user::learner(&c, i18n::native(), known) };
        (chat, history, lesson, learner)
    };
    let (lesson_bytes, history_bytes) = if online { (ONLINE_LESSON_BYTES, ONLINE_HISTORY_BYTES) } else { (CHAT_LESSON_BYTES, CHAT_HISTORY_BYTES) };
    let excerpt = lesson.map(|(title, text)| {
        let (text, partial) = ai::lesson_excerpt(&text, options.focus, lesson_bytes);
        (title, text, partial)
    });
    let context = excerpt.as_ref().map(|(title, text, partial)| ai::LessonContext { title, text, partial: *partial });
    let native = i18n::native();
    let hints: Vec<String> = ai::quoted_words(&text).iter().filter_map(|w| ai::dict_hint(&state.dicts, native, &chat.lang, w)).collect();
    let messages = ai::chat_messages(native, &chat.lang, &learner, context.as_ref(), ai::recent_history(&history, history_bytes), &text, &hints);

    let key = format!("chat:{id}");
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock();
        if dl.contains_key(&key) {
            return Err(t("Lumen répond déjà dans cette conversation.", "Lumen is already answering in this conversation.").into());
        }
        dl.insert(key.clone(), cancel.clone());
    }
    let st = state.inner();
    // durée de la réflexion : du premier mot pensé au premier mot de la réponse
    let mut thinking_since: Option<std::time::Instant> = None;
    let mut thought_secs = 0.0;
    let mut on_piece = |piece: ai::Piece| {
        match piece {
            ai::Piece::Thought(t) => {
                thinking_since.get_or_insert_with(std::time::Instant::now);
                let _ = on_event.send(ChatEvent::Thought { text: t.to_string() });
            }
            ai::Piece::Answer(t) => {
                if let Some(since) = thinking_since.take() {
                    thought_secs = since.elapsed().as_secs_f64();
                }
                let _ = on_event.send(ChatEvent::Answer { text: t.to_string() });
            }
        }
        true
    };
    let priority = Priority::Stoppable(cancel);
    let res = match &brain {
        Brain::Online(cfg) => {
            let ask = online::Ask { messages: &messages, max_tokens: CHAT_ANSWER_TOKENS, think: options.think.then_some(options.effort.as_str()), exact: false };
            online::run(cfg, ask, online::halter(&st.ai, &priority), &mut on_piece).await
        }
        Brain::Local(m) => {
            let path = models::path_of(&st.data_dir, m);
            let g = ai::Gen {
                max_tokens: CHAT_ANSWER_TOKENS,
                think: options.think.then(|| ai::think_budget(&options.effort)),
                sampling: ai::Sampling::Natural,
                priority,
            };
            tokio::task::block_in_place(|| st.ai.run(&path, &messages, g, &mut on_piece))
        }
    };
    state.downloads.lock().remove(&key);
    let out = res.map_err(err)?;
    if let Some(since) = thinking_since {
        thought_secs = since.elapsed().as_secs_f64();
    }
    let answer = out.answer.trim();
    if answer.is_empty() {
        if out.stopped {
            // arrêt avant le premier mot : rien n'est enregistré, la question revient dans le champ
            return Ok(ChatReply { chat, user: None, assistant: None, stopped: true });
        }
        return Err(t("L'IA n'a pas su répondre. Reformulez votre question, ou réessayez.", "The AI couldn't answer. Rephrase your question, or try again.").into());
    }
    let (user, assistant) = db::chat_append(&mut state.db.lock(), id, &text, answer, &out.thought, thought_secs, &ai::chat_title(&text)).map_err(err)?;
    let chat = db::chat_get(&state.db.lock(), id).map_err(err)?;
    Ok(ChatReply { chat, user: Some(user), assistant: Some(assistant), stopped: out.stopped })
}

// ---------- modèles ----------

#[derive(Serialize)]
pub struct ModelRow {
    #[serde(flatten)]
    info: models::ModelInfo,
    installed: bool,
    active: bool,
    downloading: bool,
    partial: u64,
}

#[tauri::command]
pub async fn models_list(state: State<'_, AppState>) -> R<Vec<ModelRow>> {
    let settings = db::settings_all(&state.db.lock()).map_err(err)?;
    let llm = settings.get("llm_model").cloned().unwrap_or_else(|| "qwen3.5-2b".into());
    let asr = settings.get("asr_model").cloned().unwrap_or_else(|| "whisper-turbo".into());
    let dl = state.downloads.lock();
    Ok(models::CATALOG
        .iter()
        .map(|m| {
            let part = models::part_of(&state.data_dir, m);
            let mut info = m.clone();
            info.detail = t(m.detail, m.detail_en);
            ModelRow {
                info,
                installed: models::installed(&state.data_dir, m),
                active: m.id == llm || m.id == asr,
                downloading: dl.contains_key(m.id),
                partial: std::fs::metadata(part).map(|x| x.len()).unwrap_or(0),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn model_download(state: State<'_, AppState>, id: String, on_event: Channel<DownloadEvent>) -> R<()> {
    let m = models::find(&id).ok_or(t("modèle inconnu", "unknown model"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock();
        if dl.contains_key(m.id) {
            return Err(t("Ce modèle est déjà en cours de téléchargement.", "This model is already downloading.").into());
        }
        dl.insert(m.id.to_string(), cancel.clone());
    }
    let res = models::download(&state.data_dir, m, cancel, |e| {
        let _ = on_event.send(e);
    })
    .await;
    state.downloads.lock().remove(m.id);
    res.map_err(err)
}

#[tauri::command]
pub async fn model_cancel(state: State<'_, AppState>, id: String) -> R<()> {
    if let Some(flag) = state.downloads.lock().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn model_delete(state: State<'_, AppState>, id: String) -> R<()> {
    let m = models::find(&id).ok_or(t("modèle inconnu", "unknown model"))?;
    let p = models::path_of(&state.data_dir, m);
    if state.ai.is_loaded(&p) {
        state.ai.unload();
    }
    let _ = std::fs::remove_file(models::part_of(&state.data_dir, m));
    if let Some(c) = models::companion_path(&state.data_dir, m) {
        let _ = std::fs::remove_file(c.with_extension("part"));
        let _ = std::fs::remove_file(c);
    }
    if p.is_dir() {
        std::fs::remove_dir_all(&p).map_err(err)?;
    } else if p.exists() {
        std::fs::remove_file(&p).map_err(err)?;
    }
    if m.kind == "tts" {
        crate::voice::remove_engine(&state.data_dir);
    }
    Ok(())
}

// ---------- voix naturelle ----------

/// Voix choisie pour une langue (Réglages › Voix), sinon le réglage général.
fn voice_for(state: &AppState, lang: &str) -> R<String> {
    let s = db::settings_all(&state.db.lock()).map_err(err)?;
    Ok(s.get(&format!("tts_voice_{lang}")).or_else(|| s.get("tts_voice")).cloned().unwrap_or_else(|| "0".into()))
}

fn voice_model(state: &AppState) -> R<&'static models::ModelInfo> {
    models::CATALOG
        .iter()
        .find(|m| m.kind == "tts" && models::installed(&state.data_dir, m))
        .ok_or_else(|| t("NO_VOICE: aucune voix naturelle n'est installée", "NO_VOICE: no natural voice is installed").to_string())
}

#[derive(Serialize)]
pub struct VoicedLesson {
    media_path: String,
    timings: String,
    timing_v: i64,
    duration: f64,
}

/// Crée l'audio d'une leçon de texte avec la voix naturelle. Si Whisper est
/// installé, il réécoute l'audio pour caler la lanterne au mot près.
/// Annulable avec `model_cancel("voice:<id>")`.
#[tauri::command]
pub async fn lesson_voice(state: State<'_, AppState>, id: i64, on_event: Channel<ImportEvent>) -> R<VoicedLesson> {
    let m = voice_model(&state)?;
    let (lang, text, _) = db::lesson_media(&state.db.lock(), id).map_err(err)?;
    let voice = voice_for(&state, &lang)?;
    let key = format!("voice:{id}");
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock();
        if dl.contains_key(&key) {
            return Err(t("L'audio de cette leçon est déjà en cours de création.", "The audio for this lesson is already being created.").into());
        }
        dl.insert(key.clone(), cancel.clone());
    }
    let asr = active_model(&state, "asr").ok();
    // avec Whisper : 85 % pour la voix, 15 % pour le recalage
    let share = if asr.is_some() { 0.85 } else { 1.0 };
    let _ = on_event.send(ImportEvent::Stage { stage: "voice".into() });
    let res = crate::voice::lesson_audio(&state.data_dir, m, &lang, &text, &voice, cancel, |p| {
        let _ = on_event.send(ImportEvent::Progress { value: p * share });
    })
    .await;
    state.downloads.lock().remove(&key);
    let audio = res.map_err(err)?;
    let mut timings = serde_json::to_string(&audio.timings).map_err(err)?;
    let mut version = 1;
    if let Some(asr) = asr {
        let _ = on_event.send(ImportEvent::Stage { stage: "align".into() });
        let words = media::transcribe(&models::path_of(&state.data_dir, asr), &audio.path, &lang, None, |e| {
            if let ImportEvent::Progress { value } = e {
                let _ = on_event.send(ImportEvent::Progress { value: 85.0 + value * 0.15 });
            }
        })
        .await;
        if let Ok(words) = words {
            let (aligned, found) = media::align_timings(&text, &lang, &words);
            // la voix lit exactement le texte : un bon recalage retrouve presque tout
            if found >= 0.6 {
                timings = aligned;
                version = db::TIMING_PRECISE;
            }
        }
    }
    let media_path = audio.path.to_string_lossy().into_owned();
    let old = db::lesson_set_voice(&state.db.lock(), id, &media_path, &timings, version, audio.duration).map_err(err)?;
    if let Some(old) = old {
        let _ = std::fs::remove_file(old);
    }
    Ok(VoicedLesson { media_path, timings, timing_v: version, duration: audio.duration })
}

/// Prononce un mot ou une expression avec la voix naturelle et renvoie le
/// fichier WAV. `prefetch` : préparation en arrière-plan (au toucher d'un mot),
/// abandonnée si une demande plus récente arrive entre-temps.
#[tauri::command]
pub async fn tts_say(state: State<'_, AppState>, lang: String, text: String, prefetch: bool) -> R<String> {
    let m = voice_model(&state)?;
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(600).collect();
    if text.is_empty() {
        return Err(t("Rien à prononcer.", "Nothing to pronounce.").into());
    }
    let voice = voice_for(&state, &lang)?;
    let cached = crate::voice::cached_path(&state.data_dir, m, &lang, &text, &voice);
    if cached.exists() {
        return Ok(cached.to_string_lossy().into_owned());
    }
    let my = state.voice_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    let _turn = state.voice_lock.lock().await;
    if prefetch && state.voice_epoch.load(Ordering::SeqCst) != my {
        return Err("interrompu".into());
    }
    let path = crate::voice::say(&state.data_dir, m, &lang, &text, &voice).await.map_err(err)?;
    Ok(path.to_string_lossy().into_owned())
}

// ---------- import ----------

#[tauri::command]
pub async fn read_file(path: String) -> R<Response> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| tr!("Lecture impossible : {e}", "Couldn't read the file: {e}"))?;
    Ok(Response::new(bytes))
}

/// Qwen3-ASR (modèle et partie audio), s'il est installé et connaît la langue.
fn text_model(state: &AppState, lang: &str) -> Option<(PathBuf, PathBuf)> {
    crate::asr::language_name(lang)?;
    let m = models::CATALOG.iter().find(|m| m.kind == "asrtext" && models::installed(&state.data_dir, m))?;
    Some((models::path_of(&state.data_dir, m), models::companion_path(&state.data_dir, m)?))
}

/// Transcrit un son pour en faire une leçon (Whisper, et Qwen3-ASR s'il est
/// installé et connaît la langue) : voir `media::lesson_transcript`.
async fn transcribe_media(
    state: &AppState,
    media: &std::path::Path,
    lang: &str,
    reference: Option<&str>,
    on_event: &Channel<ImportEvent>,
) -> R<(String, String)> {
    let m = active_model(state, "asr")?;
    let whisper = models::path_of(&state.data_dir, m);
    media::lesson_transcript(&state.ai, &whisper, text_model(state, lang), media, lang, reference, |e| {
        let _ = on_event.send(e);
    })
    .await
    .map_err(err)
}

async fn transcribe_into_lesson(
    state: &AppState,
    lang: &str,
    stored: PathBuf,
    title: String,
    kind: &str,
    source: String,
    video_path: Option<String>,
    on_event: &Channel<ImportEvent>,
) -> R<i64> {
    let (text, timings) = transcribe_media(state, &stored, lang, None, on_event).await?;
    if text.trim().is_empty() {
        let _ = std::fs::remove_file(&stored);
        return Err(t("Aucune parole n'a été reconnue dans ce fichier.", "No speech was recognized in this file.").into());
    }
    let lesson = NewLesson {
        lang: lang.to_string(),
        title,
        collection: String::new(),
        kind: kind.to_string(),
        source,
        text,
        media_path: Some(stored.display().to_string()),
        timings: Some(timings.clone()),
        video_path,
    };
    let conn = state.db.lock();
    let id = db::lesson_create(&conn, &lesson).map_err(err)?;
    db::lesson_set_timings(&conn, id, &timings, db::TIMING_PRECISE).map_err(err)?;
    Ok(id)
}

#[tauri::command]
pub async fn import_media(
    state: State<'_, AppState>,
    lang: String,
    path: String,
    title: Option<String>,
    on_event: Channel<ImportEvent>,
) -> R<i64> {
    let src = PathBuf::from(&path);
    let _ = on_event.send(ImportEvent::Stage { stage: "copy".into() });
    let stored = media::store_media(&state.data_dir, &src).map_err(err)?;
    let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
        src.file_stem().and_then(|s| s.to_str()).unwrap_or(t("Enregistrement", "Recording")).replace(['_', '-'], " ")
    });
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let is_video = ["mp4", "mov", "m4v", "mkv", "webm"].contains(&ext.as_str());
    let kind = if is_video { "video" } else { "audio" };
    let video = if is_video { Some(stored.display().to_string()) } else { None };
    let res = transcribe_into_lesson(&state, &lang, stored.clone(), title, kind, path, video, &on_event).await;
    if res.is_err() {
        let _ = std::fs::remove_file(&stored);
    }
    res
}

// ---------- import d'un lien : article, vidéo, podcast, Spotify ----------

/// Regarde ce qu'il y a derrière un lien : voir `link::probe`.
#[tauri::command]
pub async fn link_probe(state: State<'_, AppState>, url: String, on_event: Channel<ImportEvent>) -> R<link::LinkInfo> {
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    let mut send = |e: ImportEvent| {
        let _ = on_event.send(e);
    };
    link::probe(&state.data_dir, &url, browser.as_deref(), &mut send).await.map_err(err)
}

/// Importe un son ou une vidéo trouvé par `link_probe` : téléchargement (le
/// fichier tel quel, ou par yt-dlp avec l'image en parallèle), transcription,
/// couverture. `text` : le texte de la page, qui devient celui de la leçon si
/// le son le suit (sinon, transcription).
#[tauri::command]
pub async fn import_link(
    state: State<'_, AppState>,
    lang: String,
    item: link::MediaItem,
    text: Option<String>,
    on_event: Channel<ImportEvent>,
) -> R<i64> {
    // vérifie le modèle de transcription avant de télécharger quoi que ce soit
    active_model(&state, "asr")?;
    let data_dir = state.data_dir.clone();
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    let send = |stage: &str| {
        let _ = on_event.send(ImportEvent::Stage { stage: stage.into() });
    };
    let mut prog = |p: f64| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    };
    let stem = media::new_stem();
    let mut title = item.title.trim().to_string();
    let mut video: Option<String> = None;
    let mut video_task = None;
    let audio = if item.direct {
        send(if item.video { "file" } else { "download" });
        let p = link::download(&data_dir, &item.url, &stem, item.video, &mut prog).await.map_err(err)?;
        // une vidéo téléchargée telle quelle porte aussi le son
        if item.video {
            video = Some(p.display().to_string());
        }
        p
    } else {
        if crate::tools::find_ytdlp(&data_dir).is_none() {
            send("tools");
        }
        let ytdlp = crate::tools::ensure_youtube_tools(&data_dir, &mut prog).await.map_err(err)?;
        send("download");
        let (a, found) = media::yt_audio(&data_dir, &ytdlp, &item.url, &stem, browser.as_deref(), &mut prog).await.map_err(err)?;
        if title.is_empty() {
            title = found;
        }
        // l'image se télécharge pendant la transcription
        if item.video {
            let (dd, yt, u, st, br) = (data_dir.clone(), ytdlp.clone(), item.url.clone(), stem.clone(), browser.clone());
            video_task = Some(tokio::spawn(async move {
                let mut quiet = |_p: f64| {};
                media::yt_video(&dd, &yt, &u, &st, br.as_deref(), &mut quiet).await
            }));
        }
        a
    };
    // couverture (podcast, Spotify, sites vidéo) ; YouTube a déjà ses miniatures
    let cover_task = (!item.image.is_empty() && !link::is_youtube(&item.page) && !link::is_youtube(&item.url)).then(|| {
        let (dd, u) = (data_dir.clone(), item.image.clone());
        tokio::spawn(async move { link::fetch_cover(&dd, &u).await })
    });

    let reference = text.as_deref().map(str::trim).filter(|x| !x.is_empty());
    let transcript = transcribe_media(&state, &audio, &lang, reference, &on_event).await;
    if let Some(task) = &video_task {
        if transcript.is_err() {
            task.abort();
        }
    }
    if let Some(task) = video_task {
        if transcript.is_ok() {
            send("video");
        }
        if let Ok(Ok(p)) = task.await {
            video = Some(p.display().to_string());
        }
    }
    let cover = match cover_task {
        Some(task) => task.await.ok().flatten(),
        None => None,
    };
    let cleanup = |video: &Option<String>| {
        let _ = std::fs::remove_file(&audio);
        if let Some(v) = video {
            let _ = std::fs::remove_file(v);
        }
        if let Some(c) = &cover {
            let _ = std::fs::remove_file(c);
        }
    };
    let (text, timings) = match transcript {
        Ok(x) => x,
        Err(e) => {
            cleanup(&video);
            return Err(e);
        }
    };
    if text.trim().is_empty() {
        cleanup(&video);
        return Err(t("Aucune parole n'a été reconnue à cette adresse.", "No speech was recognized at this address.").into());
    }
    let lesson = NewLesson {
        lang,
        title: if title.is_empty() { t("Sans titre", "Untitled").into() } else { title },
        collection: item.collection.clone(),
        kind: if video.is_some() { "video" } else { "audio" }.into(),
        source: if item.page.is_empty() { item.url.clone() } else { item.page.clone() },
        text,
        media_path: Some(audio.display().to_string()),
        timings: Some(timings.clone()),
        video_path: video,
    };
    let conn = state.db.lock();
    let id = db::lesson_create(&conn, &lesson).map_err(err)?;
    db::lesson_set_timings(&conn, id, &timings, db::TIMING_PRECISE).map_err(err)?;
    if let Some(c) = cover {
        db::lesson_set_cover(&conn, id, Some(&c.display().to_string())).map_err(err)?;
    }
    Ok(id)
}

/// Recale la lanterne d'une leçon audio ou vidéo : l'audio est réécouté avec
/// le minutage précis et les mots entendus sont alignés sur le texte, qui ne
/// change pas. Renvoie les nouveaux horodatages.
#[tauri::command]
pub async fn lesson_resync(state: State<'_, AppState>, id: i64, on_event: Channel<ImportEvent>) -> R<String> {
    let (lang, text, media) = db::lesson_media(&state.db.lock(), id).map_err(err)?;
    let media = media.ok_or(t("Cette leçon n'a pas d'audio à recaler.", "This lesson has no audio to realign."))?;
    let m = active_model(&state, "asr")?;
    let model_path = models::path_of(&state.data_dir, m);
    let words = media::transcribe(&model_path, std::path::Path::new(&media), &lang, None, |e| {
        let _ = on_event.send(e);
    })
    .await
    .map_err(err)?;
    let (timings, found) = media::align_timings(&text, &lang, &words);
    if found < 0.3 {
        return Err(t(
            "L'audio ne correspond pas assez au texte de la leçon pour recaler la lanterne.",
            "The audio doesn't match the lesson text closely enough to realign the lantern.",
        )
        .into());
    }
    db::lesson_set_timings(&state.db.lock(), id, &timings, db::TIMING_PRECISE).map_err(err)?;
    Ok(timings)
}

/// Télécharge l'image d'une vidéo déjà transcrite (leçons importées sans vidéo).
#[tauri::command]
pub async fn lesson_fetch_video(state: State<'_, AppState>, id: i64, on_event: Channel<ImportEvent>) -> R<String> {
    let (source, current) = db::lesson_source(&state.db.lock(), id).map_err(err)?;
    if let Some(v) = current {
        if std::path::Path::new(&v).exists() && !v.ends_with(".m4a") {
            return Ok(v);
        }
    }
    if !source.starts_with("http") {
        return Err(t("Cette leçon n'a pas de vidéo en ligne d'origine.", "This lesson has no original online video.").into());
    }
    let data_dir = state.data_dir.clone();
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    let ytdlp = crate::tools::ensure_youtube_tools(&data_dir, |p| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    })
    .await
    .map_err(err)?;
    let _ = on_event.send(ImportEvent::Stage { stage: "video".into() });
    let mut prog = |p: f64| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    };
    let path = media::yt_video(&data_dir, &ytdlp, &source, &media::new_stem(), browser.as_deref(), &mut prog).await.map_err(err)?;
    let p = path.display().to_string();
    db::lesson_set_video(&state.db.lock(), id, &p).map_err(err)?;
    Ok(p)
}

// ---------- IA en ligne ----------

/// Vérifie une clé d'IA en ligne : modèles proposés, modèle retenu, temps de réponse.
/// `url` : adresse d'un serveur compatible (fournisseur « custom »).
#[tauri::command]
pub async fn online_check(provider: String, key: String, url: Option<String>, model: Option<String>) -> R<online::Check> {
    online::check(&provider, &key, url.as_deref().unwrap_or(""), model.as_deref().unwrap_or("")).await.map_err(err)
}

// ---------- podcasts sur mesure (Gemini) ----------

/// Vérifie une clé Gemini et dit quels modèles elle ouvre (pour écrire, pour dire).
#[tauri::command]
pub async fn gemini_check(key: String) -> R<podcast::Models> {
    let c = podcast::client().map_err(err)?;
    podcast::models(&c, &key).await.map_err(err)
}

/// Crée un podcast sur mesure et en fait une leçon : Gemini l'écrit puis le dit
/// (voir `podcast`). Le texte de la leçon est celui du script ; Whisper, s'il
/// est installé, cale la lanterne mot à mot (et si la voix s'en écartait trop,
/// le son serait transcrit comme un import ordinaire).
#[tauri::command]
pub async fn podcast_create(state: State<'_, AppState>, lang: String, request: podcast::Request, on_event: Channel<ImportEvent>) -> R<i64> {
    let send = |stage: &str| {
        let _ = on_event.send(ImportEvent::Stage { stage: stage.into() });
    };
    let (key, words) = {
        let c = state.db.lock();
        let key = db::setting(&c, "gemini_key").map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        let words = if request.use_words { db::learning_terms(&c, &lang, 25).map_err(err)? } else { Vec::new() };
        (key, words)
    };
    let key = key.ok_or(t("Ajoutez d'abord votre clé Gemini (Réglages › Podcasts).", "First add your Gemini key (Settings › Podcasts)."))?;
    if request.topic.trim().is_empty() {
        return Err(t("Choisissez un sujet pour le podcast.", "Choose a topic for the podcast.").into());
    }
    send("script");
    let c = podcast::client().map_err(err)?;
    let models = podcast::models(&c, &key).await.map_err(err)?;
    let script = podcast::write(&c, &key, &models.text, &lang, &request, &words).await.map_err(err)?;
    send("studio");
    let rec = podcast::record(&c, &key, &models.tts, &state.data_dir, &lang, request.level, &script, |p| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    })
    .await
    .map_err(err)?;
    let mut text = rec.text.clone();
    let mut timings = serde_json::to_string(&rec.timings).map_err(err)?;
    let mut version = db::TIMING_LINES;
    if let Ok(m) = active_model(&state, "asr") {
        let whisper = models::path_of(&state.data_dir, m);
        let heard = media::lesson_transcript(&state.ai, &whisper, text_model(&state, &lang), &rec.path, &lang, Some(&rec.text), |e| {
            let _ = on_event.send(e);
        })
        .await;
        match heard {
            Ok((x, y)) if !x.trim().is_empty() => {
                text = x;
                timings = y;
                version = db::TIMING_PRECISE;
            }
            // la leçon se crée quand même, avec la lanterne réplique par réplique
            Ok(_) => {}
            Err(e) => eprintln!("podcast : recalage impossible : {e}"),
        }
    }
    send("lesson");
    let lesson = NewLesson {
        lang,
        title: script.title.clone(),
        collection: t("Mes podcasts", "My podcasts").into(),
        kind: "audio".into(),
        source: String::new(),
        text,
        media_path: Some(rec.path.display().to_string()),
        timings: Some(timings.clone()),
        video_path: None,
    };
    let conn = state.db.lock();
    let id = db::lesson_create(&conn, &lesson).map_err(err)?;
    db::lesson_set_timings(&conn, id, &timings, version).map_err(err)?;
    // la bibliothèque connaît la durée avant la première écoute
    db::lesson_update(&conn, id, &LessonPatch { duration: Some(rec.duration), ..Default::default() }).map_err(err)?;
    Ok(id)
}

// ---------- LingQ ----------

/// Clé de l'import LingQ dans la table des travaux annulables.
const LINGQ_JOB: &str = "lingq-import";

#[tauri::command]
pub async fn lingq_scan(key: String) -> R<Vec<lingq::LangSummary>> {
    lingq::scan(&key).await.map_err(err)
}

#[tauri::command]
pub async fn lingq_import(
    state: State<'_, AppState>,
    key: String,
    plan: lingq::ImportPlan,
    on_event: Channel<lingq::LingqEvent>,
) -> R<lingq::Report> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = state.downloads.lock();
        if jobs.contains_key(LINGQ_JOB) {
            return Err(t("Un import LingQ est déjà en cours.", "A LingQ import is already running.").into());
        }
        jobs.insert(LINGQ_JOB.to_string(), cancel.clone());
    }
    let res = lingq::import(&state.db, &state.data_dir, &key, &plan, &cancel, |e| {
        let _ = on_event.send(e);
    })
    .await;
    state.downloads.lock().remove(LINGQ_JOB);
    res.map_err(err)
}

#[tauri::command]
pub async fn lingq_cancel(state: State<'_, AppState>) -> R<()> {
    if let Some(flag) = state.downloads.lock().get(LINGQ_JOB) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

// ---------- sauvegarde (iCloud Drive ou dossier choisi) ----------

#[tauri::command]
pub async fn backup_status(state: State<'_, AppState>) -> R<backup::Status> {
    let st = state.inner();
    Ok(tokio::task::block_in_place(|| backup::status(st)))
}

/// Sauvegarde maintenant ; renvoie l'état à jour (aussi diffusé par l'événement « backup »).
#[tauri::command]
pub async fn backup_run(app: tauri::AppHandle, state: State<'_, AppState>) -> R<backup::Status> {
    let st = state.inner();
    let res = tokio::task::block_in_place(|| backup::run_now(st));
    let status = tokio::task::block_in_place(|| backup::status(st));
    let _ = app.emit("backup", &status);
    res.map(|_| status)
}

#[tauri::command]
pub async fn backup_list(state: State<'_, AppState>) -> R<Vec<backup::Info>> {
    let st = state.inner();
    tokio::task::block_in_place(|| backup::list_for(st))
}

/// Nuages installés sur ce Mac (iCloud Drive, Dropbox, Google Drive, OneDrive…),
/// lus à leur nom seulement : macOS ne demande rien tant qu'on n'en choisit pas un.
#[tauri::command]
pub async fn backup_places() -> R<Vec<backup::Place>> {
    Ok(tokio::task::block_in_place(backup::places))
}

/// Dossier à retenir comme emplacement de sauvegarde pour un nuage choisi
/// (« Mon Drive » pour Google Drive).
#[tauri::command]
pub async fn backup_place_dir(path: String) -> R<String> {
    tokio::task::block_in_place(|| backup::place_dir(&path))
}

/// Remplace la progression de ce Mac par une sauvegarde (`day` : version d'un jour précédent).
#[tauri::command]
pub async fn backup_restore(state: State<'_, AppState>, key: String, day: Option<String>, on_event: Channel<ImportEvent>) -> R<backup::Restored> {
    let st = state.inner();
    tokio::task::block_in_place(|| {
        backup::restore_for(st, &key, day.as_deref(), |e| {
            let _ = on_event.send(e);
        })
    })
}

// ---------- Découvrir : leçons venues d'ailleurs ----------

/// Ce que les sources d'une langue proposent (dernière lecture), avec les leçons déjà créées.
#[tauri::command]
pub async fn discover_list(state: State<'_, AppState>, lang: String) -> R<discover::Feed> {
    let lessons = db::lesson_sources(&state.db.lock(), &lang).map_err(err)?;
    state.discover.list(&lang, i18n::native(), &lessons).map_err(err)
}

/// Relit maintenant les sources d'une langue (sans attendre la lecture du jour).
#[tauri::command]
pub async fn discover_refresh(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    lang: String,
    force: Option<bool>,
    on_event: Channel<ImportEvent>,
) -> R<discover::Report> {
    let mut send = |e: ImportEvent| {
        let _ = on_event.send(e);
    };
    let report = discover::refresh(&state.data_dir, &state.discover, &lang, i18n::native(), force.unwrap_or(true), &mut send).await.map_err(err)?;
    let _ = app.emit("discover", &lang);
    Ok(report)
}

/// Relie un élément de Découvrir à la leçon créée à partir de lui.
#[tauri::command]
pub async fn discover_mark(state: State<'_, AppState>, id: String, lesson: i64) -> R<()> {
    state.discover.mark(&id, lesson).map_err(err)
}

/// Écarte un élément de Découvrir (il ne revient pas).
#[tauri::command]
pub async fn discover_hide(state: State<'_, AppState>, id: String) -> R<()> {
    state.discover.hide(&id).map_err(err)
}

// ---------- Chercher : vidéos, podcasts, chansons, articles en ligne ----------

/// Cherche sur une plateforme, dans la langue étudiée (voir `search::search`).
#[tauri::command]
pub async fn search_online(
    state: State<'_, AppState>,
    lang: String,
    platform: String,
    query: String,
    page: usize,
    filter: String,
    on_event: Channel<ImportEvent>,
) -> R<search::SearchPage> {
    let mut send = |e: ImportEvent| {
        let _ = on_event.send(e);
    };
    search::search(&state.data_dir, &lang, &platform, &query, page, &filter, &mut send).await.map_err(err)
}

/// Adresses de lecture d'une vidéo (ou du seul son) pour l'aperçu.
#[tauri::command]
pub async fn media_stream(state: State<'_, AppState>, url: String, audio: bool) -> R<search::Stream> {
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    search::stream(&state.data_dir, &url, audio, browser.as_deref()).await.map_err(err)
}

/// La vidéo YouTube d'une chanson (version de l'album de préférence).
#[tauri::command]
pub async fn song_find(state: State<'_, AppState>, artist: String, title: String) -> R<String> {
    search::song_url(&state.data_dir, &artist, &title).await.map_err(err)
}

/// Les paroles d'une chanson (minutées si possible), ou rien.
#[tauri::command]
pub async fn lyrics_find(artist: String, title: String, album: String, duration: f64) -> R<Option<lyrics::Lyrics>> {
    let c = lyrics::client().map_err(err)?;
    Ok(lyrics::find(&c, &artist, &title, &album, duration).await.filter(|l| !l.is_empty()))
}

/// Une chanson à importer : ce que la recherche ou Découvrir en sait.
#[derive(serde::Deserialize, Debug)]
pub struct SongItem {
    /// vidéo YouTube de la chanson (vide : cherchée sur YouTube Music)
    #[serde(default)]
    pub url: String,
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: f64,
    /// pochette (sauf clip YouTube, qui a ses miniatures)
    #[serde(default)]
    pub image: String,
    /// page d'origine (source de la leçon)
    #[serde(default)]
    pub page: String,
    /// paroles déjà trouvées (sinon cherchées)
    #[serde(default)]
    pub lyrics: Option<lyrics::Lyrics>,
    /// garder l'image du clip, sinon le son seul
    #[serde(default)]
    pub video: bool,
}

/// Une chanson devient une leçon : les paroles pour texte, une ligne par
/// paragraphe ; la lanterne suit les lignes des paroles minutées, recalée mot à
/// mot sur la voix si Whisper est installé. Pas de transcription : le chant se
/// transcrit mal, les paroles publiées sont justes.
#[tauri::command]
pub async fn import_song(state: State<'_, AppState>, lang: String, song: SongItem, on_event: Channel<ImportEvent>) -> R<i64> {
    let data_dir = state.data_dir.clone();
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    let send = |stage: &str| {
        let _ = on_event.send(ImportEvent::Stage { stage: stage.into() });
    };
    let mut prog = |p: f64| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    };
    send("lyrics");
    let found = match song.lyrics.clone().filter(|l| !l.is_empty()) {
        Some(l) => Some(l),
        None => {
            let c = lyrics::client().map_err(err)?;
            lyrics::find(&c, &song.artist, &song.title, &song.album, song.duration).await.filter(|l| !l.is_empty())
        }
    };
    let words = found.as_ref().ok_or(t(
        "Lumen n'a pas trouvé les paroles de cette chanson. Vous pouvez l'importer comme une vidéo : elle sera transcrite.",
        "Lumen couldn't find the lyrics of this song. You can import it as a video: it will be transcribed.",
    ))?;
    let (text, lrc) = lyrics::layout(words, &lang, song.duration);
    if crate::text::word_keys(&text, &lang).len() < 8 {
        return Err(t("Les paroles trouvées sont trop courtes pour une leçon.", "The lyrics found are too short for a lesson.").into());
    }
    if crate::tools::find_ytdlp(&data_dir).is_none() {
        send("tools");
    }
    let ytdlp = crate::tools::ensure_youtube_tools(&data_dir, &mut prog).await.map_err(err)?;
    let url = if song.url.trim().is_empty() {
        send("probe");
        search::song_url(&data_dir, &song.artist, &song.title).await.map_err(err)?
    } else {
        song.url.clone()
    };
    send("download");
    let stem = media::new_stem();
    let (audio, _) = media::yt_audio(&data_dir, &ytdlp, &url, &stem, browser.as_deref(), &mut prog).await.map_err(err)?;
    let video_task = song.video.then(|| {
        let (dd, yt, u, st, br) = (data_dir.clone(), ytdlp.clone(), url.clone(), stem.clone(), browser.clone());
        tokio::spawn(async move {
            let mut quiet = |_p: f64| {};
            media::yt_video(&dd, &yt, &u, &st, br.as_deref(), &mut quiet).await
        })
    });
    let cover_task = (!song.image.is_empty() && !song.video).then(|| {
        let (dd, u) = (data_dir.clone(), song.image.clone());
        tokio::spawn(async move { link::fetch_cover(&dd, &u).await })
    });

    // la lanterne : mot à mot sur la voix si Whisper l'entend assez, sinon ligne à ligne
    let mut timings = lrc.clone();
    let mut version = db::TIMING_LINES;
    if let Ok(m) = active_model(&state, "asr") {
        let whisper = models::path_of(&state.data_dir, m);
        let heard = media::transcribe(&whisper, &audio, &lang, None, |e| {
            let _ = on_event.send(e);
        })
        .await;
        if let Ok(heard) = heard {
            let (aligned, found) = media::align_timings(&text, &lang, &heard);
            if found >= 0.35 || (lrc.is_none() && found >= 0.15) {
                timings = Some(aligned);
                version = db::TIMING_PRECISE;
            }
        }
    }
    let mut video = None;
    if let Some(task) = video_task {
        send("video");
        if let Ok(Ok(p)) = task.await {
            video = Some(p.display().to_string());
        }
    }
    let cover = match cover_task {
        Some(task) => task.await.ok().flatten(),
        None => None,
    };
    let timings = timings.unwrap_or_else(|| "[]".into());
    let lesson = NewLesson {
        lang,
        title: song.title.trim().to_string(),
        collection: song.artist.trim().to_string(),
        kind: if video.is_some() { "video" } else { "audio" }.into(),
        source: [&song.page, &url].iter().find(|s| s.starts_with("http")).map(|s| s.to_string()).unwrap_or_default(),
        text,
        media_path: Some(audio.display().to_string()),
        timings: Some(timings.clone()),
        video_path: video,
    };
    let conn = state.db.lock();
    let id = db::lesson_create(&conn, &lesson).map_err(err)?;
    db::lesson_set_timings(&conn, id, &timings, if timings == "[]" { 0 } else { version }).map_err(err)?;
    if let Some(c) = cover {
        db::lesson_set_cover(&conn, id, Some(&c.display().to_string())).map_err(err)?;
    }
    Ok(id)
}

// ---------- niveau estimé, mots connus d'un texte ----------

/// Niveau estimé dans une langue, d'après les mots connus regroupés par lemme
/// (voir `level`). Gardé tant que les mots connus et les dictionnaires ne changent pas.
#[tauri::command]
pub async fn level_estimate(state: State<'_, AppState>, lang: String) -> R<level::Estimate> {
    let (known, stamp) = {
        let c = state.db.lock();
        (db::known_terms(&c, &lang).map_err(err)?, db::known_stamp(&c, &lang).map_err(err)?)
    };
    let lemmatizer = state.dicts.lemmatizer(&lang);
    let sig = format!("{stamp}:{}:{}", lemmatizer.is_some(), i18n::native());
    if let Some((s, e)) = state.levels.lock().get(&lang) {
        if *s == sig {
            return Ok(e.clone());
        }
    }
    let l = lang.clone();
    let e = tokio::task::spawn_blocking(move || level::estimate(&l, &known, lemmatizer.as_ref())).await.map_err(err)?;
    state.levels.lock().insert(lang, (sig, e.clone()));
    Ok(e)
}

/// Part des mots d'un texte que l'apprenant connaît déjà (article, paroles) :
/// de quoi savoir s'il est à sa portée avant d'en faire une leçon.
#[tauri::command]
pub async fn text_stats(state: State<'_, AppState>, lang: String, text: String) -> R<db::TextStats> {
    db::text_stats(&state.db.lock(), &lang, &text).map_err(err)
}
