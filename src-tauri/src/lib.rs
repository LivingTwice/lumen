#[macro_use]
mod i18n;
mod ai;
mod asr;
mod backup;
mod commands;
mod db;
mod dict;
mod lingq;
mod media;
mod models;
mod state;
mod text;
mod tools;
mod voice;

use std::collections::HashMap;

use parking_lot::Mutex;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::open(&db::path(&data_dir))?;
            // langue de l'interface, des messages et des traductions
            i18n::set(&db::setting(&conn, "ui_lang").unwrap_or_default());
            let resource_dir = app.path().resource_dir()?.join("dicts");
            let dicts = dict::Dicts::new(resource_dir, dict::dict_dir(&data_dir));
            app.manage(state::AppState {
                data_dir,
                db: Mutex::new(conn),
                dicts,
                ai: ai::Engine::new(),
                downloads: Mutex::new(HashMap::new()),
                voice_lock: tokio::sync::Mutex::new(()),
                voice_epoch: std::sync::atomic::AtomicU64::new(0),
                backup: backup::Tracker::default(),
            });
            // sauvegarde automatique, au plus toutes les 10 minutes
            tauri::async_runtime::spawn(backup::auto_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::settings_get,
            commands::settings_set,
            commands::lessons_list,
            commands::lesson_open,
            commands::lesson_create,
            commands::lesson_update,
            commands::lesson_delete,
            commands::lesson_set_cover,
            commands::playlists_list,
            commands::playlist_create,
            commands::playlist_update,
            commands::playlist_delete,
            commands::term_set,
            commands::terms_mark_known,
            commands::terms_list,
            commands::stats,
            commands::activity_add,
            commands::export_vocab,
            commands::dict_lookup,
            commands::ai_word,
            commands::ai_sentence,
            commands::ai_simplify,
            commands::ai_warmup,
            commands::chats_list,
            commands::chat_open,
            commands::chat_create,
            commands::chat_update,
            commands::chat_delete,
            commands::chat_send,
            commands::models_list,
            commands::model_download,
            commands::model_cancel,
            commands::model_delete,
            commands::tts_say,
            commands::lesson_voice,
            commands::fetch_url,
            commands::read_file,
            commands::import_media,
            commands::import_youtube,
            commands::lesson_fetch_video,
            commands::lesson_resync,
            commands::lingq_scan,
            commands::lingq_import,
            commands::lingq_cancel,
            commands::backup_status,
            commands::backup_run,
            commands::backup_list,
            commands::backup_restore,
        ])
        .build(tauri::generate_context!())
        .expect("Lumen couldn't start")
        .run(|app, event| {
            // dernière sauvegarde en quittant, si la progression a changé
            if let tauri::RunEvent::Exit = event {
                if let Some(st) = app.try_state::<state::AppState>() {
                    backup::on_exit(&st);
                }
            }
        });
}
