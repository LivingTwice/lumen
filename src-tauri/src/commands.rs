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
use crate::lingq;
use crate::media::{self, ImportEvent};
use crate::models::{self, DownloadEvent};
use crate::state::AppState;
use crate::text;

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
    let langs = ["en", "es", "it", "de", "pt", "ru"].iter().filter(|l| state.dicts.available(l)).map(|s| s.to_string()).collect();
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
pub async fn settings_set(state: State<'_, AppState>, key: String, value: String) -> R<()> {
    db::setting_set(&state.db.lock(), &key, &value).map_err(err)
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
pub async fn lesson_open(state: State<'_, AppState>, id: i64) -> R<OpenedLesson> {
    let conn = state.db.lock();
    let lesson = db::lesson_get(&conn, id).map_err(err)?;
    let tokens = text::tokenize(&lesson.text, &lesson.lang);
    let keys: Vec<String> = tokens.iter().filter(|t| t.w).map(|t| t.k.clone()).collect();
    let terms = db::terms_for_keys(&conn, &lesson.lang, &keys).map_err(err)?;
    let lang = lesson.lang.clone();
    drop(conn);
    // prépare le dictionnaire en arrière-plan
    let dicts_ready = state.dicts.available(&lang);
    if dicts_ready {
        state.dicts.warm(&lang);
    }
    Ok(OpenedLesson { lesson, tokens, terms })
}

#[tauri::command]
pub async fn lesson_create(state: State<'_, AppState>, lesson: NewLesson) -> R<i64> {
    if lesson.text.trim().is_empty() {
        return Err("Le texte est vide.".into());
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
                return Err("Cette image est trop lourde (30 Mo au plus).".into());
            }
            let ext = ext.unwrap_or_default().to_lowercase();
            let ext = if ["jpg", "jpeg", "png", "webp", "gif", "heic", "avif"].contains(&ext.as_str()) { ext } else { "jpg".into() };
            std::fs::create_dir_all(&media).map_err(err)?;
            let p = media.join(format!("{}.cover.{ext}", media::new_stem()));
            std::fs::write(&p, bytes).map_err(|e| format!("Image impossible à enregistrer : {e}"))?;
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
pub async fn activity_add(state: State<'_, AppState>, lang: String, words_read: i64, listen_secs: i64) -> R<()> {
    db::activity_add(&state.db.lock(), &lang, words_read, listen_secs).map_err(err)
}

#[tauri::command]
pub async fn export_vocab(state: State<'_, AppState>, lang: String, path: String) -> R<()> {
    let csv = db::export_csv(&state.db.lock(), &lang).map_err(err)?;
    std::fs::write(path, csv).map_err(err)
}

// ---------- dictionnaire ----------

#[tauri::command]
pub async fn dict_lookup(state: State<'_, AppState>, lang: String, word: String) -> R<DictResult> {
    if !state.dicts.available(&lang) {
        return Ok(DictResult::default());
    }
    state.dicts.lookup(&lang, &word).map_err(err)
}

// ---------- IA locale ----------

fn active_model(state: &AppState, kind: &str) -> R<&'static models::ModelInfo> {
    let key = if kind == "llm" { "llm_model" } else { "asr_model" };
    let default = if kind == "llm" { "qwen3.5-2b" } else { "whisper-turbo" };
    let id = db::setting(&state.db.lock(), key).unwrap_or_else(|| default.to_string());
    let m = models::find(&id).or_else(|| models::find(default)).ok_or("modèle inconnu")?;
    if !models::installed(&state.data_dir, m) {
        // à défaut, n'importe quel modèle installé du même type
        if let Some(other) = models::CATALOG.iter().find(|x| x.kind == kind && models::installed(&state.data_dir, x)) {
            return Ok(other);
        }
        return Err(if kind == "llm" {
            "NO_MODEL:Aucun modèle de traduction n'est installé. Ouvrez Réglages › IA locale.".into()
        } else {
            "NO_MODEL:Aucun modèle de transcription n'est installé. Ouvrez Réglages › IA locale.".into()
        });
    }
    Ok(m)
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Piece { text: String },
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
    let m = active_model(&state, "llm")?;
    let key = hash(&["w3", m.id, &lang, &text::normalize_for(&word, &lang), sentence.trim()]);
    if let Some(v) = db::cache_get(&state.db.lock(), &key) {
        let (t, n) = v.split_once('\u{1f}').unwrap_or((&v, ""));
        return Ok(WordAnswer { translation: t.to_string(), note: n.to_string(), cached: true });
    }
    let path = models::path_of(&state.data_dir, m);
    let epoch = state.ai.next_epoch();
    // indice du dictionnaire : forme de base et premiers sens
    let mut hint = String::new();
    if !word.contains(' ') && state.dicts.available(&lang) {
        if let Ok(d) = state.dicts.lookup(&lang, &word) {
            let glosses: Vec<String> = d.entries.iter().flat_map(|e| e.glosses.iter().take(2).cloned()).take(3).collect();
            if let Some(l) = &d.lemma {
                hint = format!("forme de {l}");
                if !glosses.is_empty() {
                    hint.push_str(" : ");
                }
            }
            hint.push_str(&glosses.join(" ; ").to_lowercase());
        }
    }
    let messages = ai::word_messages(&lang, &word, &sentence, &hint);
    let st = state.inner();
    let raw = tokio::task::block_in_place(|| {
        st.ai.generate(&path, &messages, 72, Priority::Interactive(epoch), |piece| {
            let _ = on_event.send(AiEvent::Piece { text: piece.to_string() });
            true
        })
    })
    .map_err(err)?;
    let (translation, note) = ai::parse_word_answer(&raw);
    if !translation.is_empty() {
        db::cache_put(&state.db.lock(), &key, &format!("{translation}\u{1f}{note}"));
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
    let m = active_model(&state, "llm")?;
    let key = hash(&["s1", m.id, &lang, sentence.trim()]);
    if let Some(v) = db::cache_get(&state.db.lock(), &key) {
        return Ok(v);
    }
    let path = models::path_of(&state.data_dir, m);
    let epoch = state.ai.next_epoch();
    let messages = ai::sentence_messages(&lang, &sentence);
    let st = state.inner();
    let out = tokio::task::block_in_place(|| {
        st.ai.generate(&path, &messages, 260, Priority::Interactive(epoch), |piece| {
            let _ = on_event.send(AiEvent::Piece { text: piece.to_string() });
            true
        })
    })
    .map_err(err)?;
    let out = out.trim().trim_matches(['«', '»', '"']).trim().to_string();
    if !out.is_empty() {
        db::cache_put(&state.db.lock(), &key, &out);
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
    let m = active_model(&state, "llm")?;
    let path = models::path_of(&state.data_dir, m);
    // découpe en blocs de paragraphes d'environ 1 200 caractères
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if !cur.is_empty() && cur.len() + para.len() > 1200 {
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
        let messages = ai::simplify_messages(&lang, &level, chunk);
        let out = tokio::task::block_in_place(|| {
            st.ai.generate(&path, &messages, 900, Priority::Background, |piece| {
                let _ = on_event.send(AiEvent::Piece { text: piece.to_string() });
                true
            })
        })
        .map_err(err)?;
        result.push_str(out.trim());
    }
    Ok(result)
}

#[tauri::command]
pub async fn ai_warmup(state: State<'_, AppState>) -> R<bool> {
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

// ---------- chat ----------

/// Place de la leçon jointe et des échanges précédents dans la mémoire du modèle (en octets).
const CHAT_LESSON_BYTES: usize = 24_000;
const CHAT_HISTORY_BYTES: usize = 16_000;
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
        return Err("Écrivez d'abord votre question.".into());
    }
    let m = active_model(&state, "llm")?;
    let path = models::path_of(&state.data_dir, m);
    // tout est lu d'un coup : aucun verrou n'est tenu pendant la génération
    let (chat, history, lesson, known) = {
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
        (chat, history, lesson, known)
    };
    let excerpt = lesson.map(|(title, text)| {
        let (text, partial) = ai::lesson_excerpt(&text, options.focus, CHAT_LESSON_BYTES);
        (title, text, partial)
    });
    let context = excerpt.as_ref().map(|(title, text, partial)| ai::LessonContext { title, text, partial: *partial });
    let hints: Vec<String> = ai::quoted_words(&text).iter().filter_map(|w| ai::dict_hint(&state.dicts, &chat.lang, w)).collect();
    let messages = ai::chat_messages(&chat.lang, known, context.as_ref(), ai::recent_history(&history, CHAT_HISTORY_BYTES), &text, &hints);

    let key = format!("chat:{id}");
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock();
        if dl.contains_key(&key) {
            return Err("Lumen répond déjà dans cette conversation.".into());
        }
        dl.insert(key.clone(), cancel.clone());
    }
    let g = ai::Gen {
        max_tokens: CHAT_ANSWER_TOKENS,
        think: options.think.then(|| ai::think_budget(&options.effort)),
        sampling: ai::Sampling::Natural,
        priority: Priority::Stoppable(cancel),
    };
    let st = state.inner();
    // durée de la réflexion : du premier mot pensé au premier mot de la réponse
    let mut thinking_since: Option<std::time::Instant> = None;
    let mut thought_secs = 0.0;
    let res = tokio::task::block_in_place(|| {
        st.ai.run(&path, &messages, g, |piece| {
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
        })
    });
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
        return Err("L'IA n'a pas su répondre. Reformulez votre question, ou réessayez.".into());
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
            ModelRow {
                info: m.clone(),
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
    let m = models::find(&id).ok_or("modèle inconnu")?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut dl = state.downloads.lock();
        if dl.contains_key(m.id) {
            return Err("Ce modèle est déjà en cours de téléchargement.".into());
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
    let m = models::find(&id).ok_or("modèle inconnu")?;
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
        .ok_or_else(|| "NO_VOICE: aucune voix naturelle n'est installée".to_string())
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
            return Err("L'audio de cette leçon est déjà en cours de création.".into());
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
        return Err("Rien à prononcer.".into());
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
pub async fn fetch_url(url: String) -> R<String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(err)?;
    let resp = client.get(&url).send().await.map_err(|e| format!("Page inaccessible : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("La page a répondu {}.", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(err)?;
    if bytes.len() > 15_000_000 {
        return Err("Page trop volumineuse.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[tauri::command]
pub async fn read_file(path: String) -> R<Response> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("Lecture impossible : {e}"))?;
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
async fn transcribe_media(state: &AppState, media: &std::path::Path, lang: &str, on_event: &Channel<ImportEvent>) -> R<(String, String)> {
    let m = active_model(state, "asr")?;
    let whisper = models::path_of(&state.data_dir, m);
    media::lesson_transcript(&state.ai, &whisper, text_model(state, lang), media, lang, |e| {
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
    let (text, timings) = transcribe_media(state, &stored, lang, on_event).await?;
    if text.trim().is_empty() {
        let _ = std::fs::remove_file(&stored);
        return Err("Aucune parole n'a été reconnue dans ce fichier.".into());
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
        src.file_stem().and_then(|s| s.to_str()).unwrap_or("Enregistrement").replace(['_', '-'], " ")
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

#[tauri::command]
pub async fn import_youtube(state: State<'_, AppState>, lang: String, url: String, on_event: Channel<ImportEvent>) -> R<i64> {
    let data_dir = state.data_dir.clone();
    // vérifie le modèle de transcription avant de télécharger quoi que ce soit
    active_model(&state, "asr")?;
    let browser = db::setting(&state.db.lock(), "youtube_browser").filter(|b| !b.is_empty());
    let send = |stage: &str| {
        let _ = on_event.send(ImportEvent::Stage { stage: stage.into() });
    };
    send("tools");
    let ytdlp = crate::tools::ensure_youtube_tools(&data_dir, |p| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    })
    .await
    .map_err(err)?;
    send("download");
    let stem = media::new_stem();
    let mut prog = |p: f64| {
        let _ = on_event.send(ImportEvent::Progress { value: p });
    };
    let (audio, title) = media::yt_audio(&data_dir, &ytdlp, &url, &stem, browser.as_deref(), &mut prog).await.map_err(err)?;

    // l'image se télécharge pendant la transcription
    let (dd, yt, u, st, br) = (data_dir.clone(), ytdlp.clone(), url.clone(), stem.clone(), browser.clone());
    let video_task = tokio::spawn(async move {
        let mut quiet = |_p: f64| {};
        media::yt_video(&dd, &yt, &u, &st, br.as_deref(), &mut quiet).await
    });

    let (text, timings) = match transcribe_media(&state, &audio, &lang, &on_event).await {
        Ok(t) => t,
        Err(e) => {
            video_task.abort();
            let _ = std::fs::remove_file(&audio);
            return Err(e);
        }
    };
    send("video");
    let video = match video_task.await {
        Ok(Ok(p)) => Some(p.display().to_string()),
        _ => None,
    };
    if text.trim().is_empty() {
        let _ = std::fs::remove_file(&audio);
        if let Some(v) = &video {
            let _ = std::fs::remove_file(v);
        }
        return Err("Aucune parole n'a été reconnue dans cette vidéo.".into());
    }
    let lesson = NewLesson {
        lang,
        title,
        collection: String::new(),
        kind: "video".into(),
        source: url,
        text,
        media_path: Some(audio.display().to_string()),
        timings: Some(timings.clone()),
        video_path: video,
    };
    let conn = state.db.lock();
    let id = db::lesson_create(&conn, &lesson).map_err(err)?;
    db::lesson_set_timings(&conn, id, &timings, db::TIMING_PRECISE).map_err(err)?;
    Ok(id)
}

/// Recale la lanterne d'une leçon audio ou vidéo : l'audio est réécouté avec
/// le minutage précis et les mots entendus sont alignés sur le texte, qui ne
/// change pas. Renvoie les nouveaux horodatages.
#[tauri::command]
pub async fn lesson_resync(state: State<'_, AppState>, id: i64, on_event: Channel<ImportEvent>) -> R<String> {
    let (lang, text, media) = db::lesson_media(&state.db.lock(), id).map_err(err)?;
    let media = media.ok_or("Cette leçon n'a pas d'audio à recaler.")?;
    let m = active_model(&state, "asr")?;
    let model_path = models::path_of(&state.data_dir, m);
    let words = media::transcribe(&model_path, std::path::Path::new(&media), &lang, None, |e| {
        let _ = on_event.send(e);
    })
    .await
    .map_err(err)?;
    let (timings, found) = media::align_timings(&text, &lang, &words);
    if found < 0.3 {
        return Err("L'audio ne correspond pas assez au texte de la leçon pour recaler la lanterne.".into());
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
        return Err("Cette leçon n'a pas de vidéo en ligne d'origine.".into());
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
            return Err("Un import LingQ est déjà en cours.".into());
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
