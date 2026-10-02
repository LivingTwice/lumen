mod ai;
mod commands;
mod db;
mod dict;
mod media;
mod models;
mod state;
mod text;
mod tools;

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
            let conn = db::open(&data_dir.join("lumen.db"))?;
            let resource_dir = app.path().resource_dir()?.join("dicts");
            let dicts = dict::Dicts::new(resource_dir, dict::dict_dir(&data_dir));
            app.manage(state::AppState {
                data_dir,
                db: Mutex::new(conn),
                dicts,
                ai: ai::Engine::new(),
                downloads: Mutex::new(HashMap::new()),
            });
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
            commands::models_list,
            commands::model_download,
            commands::model_cancel,
            commands::model_delete,
            commands::fetch_url,
            commands::read_file,
            commands::import_media,
            commands::import_youtube,
            commands::lesson_fetch_video,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Lumen");
}
