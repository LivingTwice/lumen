#[macro_use]
mod i18n;
mod ai;
// iPhone et iPad : pont vers l'interface d'iOS (dossiers de l'app Fichiers, iCloud, haptique)
#[cfg(target_os = "ios")]
mod ios;
mod asr;
mod backup;
mod chatgpt;
mod commands;
mod compress;
mod db;
mod dict;
mod discover;
mod langid;
mod level;
mod link;
mod lingq;
mod lyrics;
mod media;
mod models;
mod online;
mod parakeet;
mod podcast;
mod proc;
mod search;
mod sherpa;
mod speakers;
// iPhone et iPad : la voix et Parakeet par la bibliothèque sherpa-onnx, dans l'app
#[cfg(target_os = "ios")]
mod sherpa_ios;
mod state;
mod sync;
mod text;
mod tools;
mod user;
mod voice;
#[cfg(target_os = "linux")]
mod mediaserv;
#[cfg(windows)]
mod vulkan;
// Whisper dans l'app : iPhone et iPad (essais sur Mac : LUMEN_WHISPER_INAPP=1)
#[cfg(whisper_inapp)]
mod whisper;
mod ytnative;
#[cfg(windows)]
mod win;

use std::collections::HashMap;

use parking_lot::Mutex;
use tauri::Manager;

// iPhone et iPad : point d'entrée appelé par le projet Xcode (gen/apple)
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux : le rendu DMABUF de WebKitGTK (par son processus graphique) est utilisé tel quel :
    // c'est lui qui rend la fenêtre fluide. Il avait été écarté parce que l'accueil se figeait à
    // l'étape des langues ; le coupable était le flou de fond des cartes de cette étape
    // (onboarding.css), pas le rendu. Si un pilote graphique s'entend mal avec lui,
    // WEBKIT_DISABLE_DMABUF_RENDERER=1 revient au rendu par le processeur (lent).
    // Linux : le lecteur média que WebKitGTK prend d'office (l'ancien playbin) annonce
    // la vidéo chargée avant qu'elle le soit, s'arrête pour mettre en tampon et ne
    // reprend pas toujours ; un saut peut figer l'image pour de bon. Le lecteur récent
    // (playbin3) n'a aucun de ces défauts : la vidéo joue d'emblée à 30 images par
    // seconde, les sauts sont propres. Posée seulement si l'apprenant n'a pas choisi
    // lui-même : WEBKIT_GST_USE_PLAYBIN3=0 revient à l'ancien lecteur.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_GST_USE_PLAYBIN3").is_none() {
        std::env::set_var("WEBKIT_GST_USE_PLAYBIN3", "1");
    }
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init());
    // mises à jour et relance : Mac, Windows et Linux (sur iPhone et iPad, l'App Store s'en charge)
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build()).plugin(tauri_plugin_process::init());
    builder
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            // connexion à ChatGPT (jetons hors de la base, propres à cet appareil)
            chatgpt::init(&data_dir);
            let conn = db::open(&db::path(&data_dir))?;
            // langue de l'interface, des messages et des traductions
            i18n::set(&db::setting(&conn, "ui_lang").unwrap_or_default());
            // Windows : la carte graphique pour l'IA, sauf si l'apprenant a choisi le processeur
            ai::set_gpu(db::setting(&conn, "ai_gpu").as_deref() != Some("0"));
            // allègement des médias : fichiers d'un travail interrompu, originaux mis de côté
            compress::sweep(&media::media_dir(&data_dir), |p| db::media_used(&conn, p, None));
            // iPhone et iPad : le dossier de l'app change à chaque mise à jour ; les médias suivent
            let _ = db::rebase_media(&conn, &media::media_dir(&data_dir));
            // Dictionnaires livrés : le dossier de ressources du paquet. Sous Linux sans
            // paquet (nix, `cargo build`), il n'existe pas à l'endroit attendu : la variable
            // LUMEN_RESOURCES le donne, sinon Lumen démarre sans les dictionnaires livrés
            // (les autres se téléchargent comme d'habitude).
            let resource_dir = std::env::var_os("LUMEN_RESOURCES")
                .map(std::path::PathBuf::from)
                .or_else(|| app.path().resource_dir().ok())
                .unwrap_or_else(std::path::PathBuf::new)
                .join("dicts");
            let dicts = dict::Dicts::new(resource_dir, dict::dict_dir(&data_dir));
            let discover = discover::Store::open(&data_dir);
            // Linux : l'audio et la vidéo des leçons passent par le petit serveur
            // local (l'élément média d'WebKitGTK refuse le protocole des ressources)
            #[cfg(target_os = "linux")]
            let media_base = mediaserv::start(data_dir.join("media")).unwrap_or_default();
            #[cfg(not(target_os = "linux"))]
            let media_base = String::new();
            app.manage(state::AppState {
                data_dir,
                db: Mutex::new(conn),
                dicts,
                ai: ai::Engine::new(),
                downloads: Mutex::new(HashMap::new()),
                voice_lock: tokio::sync::Mutex::new(()),
                voice_epoch: std::sync::atomic::AtomicU64::new(0),
                backup: backup::Tracker::default(),
                discover,
                levels: Mutex::new(HashMap::new()),
                codecs: Mutex::new(Vec::new()),
                media_base,
            });
            // barre des menus (construite par l'interface, lib/menu.ts) : l'élément choisi lui
            // revient par son identifiant ; les actions JavaScript des éléments de sous-menus
            // sont oubliées par Tauri dès leur création (pas de barre des menus sur iPhone et iPad)
            #[cfg(desktop)]
            app.on_menu_event(|app, event| {
                let _ = tauri::Emitter::emit(app, "menu", event.id().as_ref());
            });
            // sauvegarde automatique, au plus toutes les 10 minutes
            tauri::async_runtime::spawn(backup::auto_loop(app.handle().clone()));
            // Découvrir : les sources des langues étudiées, relues une fois par jour
            tauri::async_runtime::spawn(discover::auto_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::app_relaunch,
            commands::settings_get,
            commands::settings_set,
            commands::lessons_list,
            commands::lesson_open,
            commands::lesson_create,
            commands::lesson_update,
            commands::lesson_delete,
            commands::lessons_delete,
            commands::lessons_set_completed,
            commands::lesson_set_cover,
            commands::lesson_cover_fetch,
            commands::lesson_links,
            commands::playlists_list,
            commands::playlist_create,
            commands::playlist_update,
            commands::playlist_delete,
            commands::book_create,
            commands::term_set,
            commands::terms_mark_known,
            commands::terms_list,
            commands::stats,
            commands::activity_add,
            commands::export_vocab,
            commands::dict_lookup,
            commands::dict_status,
            commands::ai_word,
            commands::ai_sentence,
            commands::ai_simplify,
            commands::ai_warmup,
            commands::gpu_info,
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
            commands::read_file,
            commands::import_media,
            commands::link_probe,
            commands::import_link,
            commands::lesson_fetch_video,
            commands::media_codecs,
            commands::media_usage,
            commands::media_lighten,
            commands::lesson_resync,
            commands::lingq_scan,
            commands::lingq_import,
            commands::lingq_cancel,
            commands::backup_status,
            commands::backup_run,
            commands::backup_list,
            commands::backup_restore,
            commands::backup_places,
            commands::backup_place_dir,
            commands::backup_pick_folder,
            commands::backup_sync,
            commands::backup_merge,
            commands::haptic,
            commands::device_info,
            commands::discover_list,
            commands::discover_refresh,
            commands::discover_mark,
            commands::discover_hide,
            commands::search_online,
            commands::media_stream,
            commands::song_find,
            commands::lyrics_find,
            commands::import_song,
            commands::gemini_check,
            commands::online_check,
            commands::chatgpt_status,
            commands::chatgpt_sign_in,
            commands::chatgpt_sign_out,
            commands::podcast_create,
            commands::level_estimate,
            commands::text_stats,
        ])
        .build(tauri::generate_context!())
        .expect("Lumen couldn't start")
        .run(|app, event| {
            // dernière sauvegarde en quittant, si la progression a changé
            if let tauri::RunEvent::Exit = event {
                if let Some(st) = app.try_state::<state::AppState>() {
                    backup::on_exit(&st);
                }
                // « Redémarrer » après une mise à jour : la nouvelle version, au premier plan
                proc::relaunch_on_exit();
            }
        });
}
