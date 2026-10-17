#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Hashi Rust版のWeb画面（Tauri 2 + WebView2）。核はhashi-core/hashi-storage/hashi-terminalを使い、
//! 画面（frontend/）とはcommandとeventだけで話す。画面側へ渡すのは表示に要るものだけで、
//! 秘密・認証要求の照合・信頼情報の保存は常にこちら（Rust側）で行う。
mod ai;
mod ai_models;
mod app;
mod attach;
mod editor;
mod external;
mod files;
mod forwards;
mod import;
mod keygen;
mod local;
mod mcp_register;
mod session;
mod settings;
mod snippets;

use tauri::{Manager, WindowEvent};

/// Rust側の異常終了（panic）を設定フォルダの`rust-panic.log`へ残す。画面が固まったときの調査用。
fn record_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(root) = hashi_storage::paths::config_root(hashi_storage::paths::Channel::Preview)
        {
            use std::io::Write;
            let thread = std::thread::current();
            let text = format!(
                "[{}] thread={} {info}\n",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                thread.name().unwrap_or("?"),
            );
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("rust-panic.log"))
            {
                let _ = file.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

fn main() {
    record_panics();
    let state = match app::AppState::new() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("起動できません: {error:#}");
            std::process::exit(1);
        }
    };
    let built = tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            app::start_challenge_pump(app.handle().clone());
            ai::start_pumps(app.handle().clone());
            files::start_pumps(app.handle().clone());
            settings::start_update_check(app.handle().clone());
            let handle = app.handle().clone();
            // 保存キーの復号は重いので、画面の起動を待たせない。
            tauri::async_runtime::spawn_blocking(move || {
                ai::restore(&handle);
                // 画面は先に起動しているので、読み戻しの終了を知らせる。
                let _ = tauri::Emitter::emit(&handle, "ai-restored", ());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::profiles_list,
            app::profile_save,
            app::profile_delete,
            app::client_log,
            import::import_legacy_inspect,
            import::import_bundle_pick,
            import::import_bundle_unlock,
            import::import_cancel,
            import::import_apply,
            import::secrets_status,
            import::secrets_forget,
            app::pick_key_file,
            app::appearance_get,
            app::appearance_set,
            app::ui_prefs_get,
            app::ui_prefs_set,
            app::challenge_reply,
            ai::ai_services,
            ai::ai_protocols,
            ai::ai_open_key_page,
            ai::ai_status,
            ai::ai_has_key,
            ai::ai_forget,
            ai_models::ai_models_list,
            ai_models::ai_model_save,
            ai_models::ai_model_delete,
            ai_models::ai_model_use,
            ai_models::ai_model_check,
            ai_models::ai_model_candidates,
            ai::ai_clear,
            ai::ai_stop,
            ai::ai_send,
            ai::ai_confirm,
            ai::ai_cancel_preview,
            ai::approval_reply,
            editor::editor_open_remote,
            editor::editor_open_local,
            editor::editor_save,
            editor::editor_save_as,
            editor::editor_set_dirty,
            editor::editor_close,
            app::app_quit,
            external::external_status,
            external::external_share,
            external::external_stop,
            mcp_register::mcp_register_status,
            mcp_register::mcp_register_desktop,
            local::local_list,
            local::local_pick_dir,
            local::local_roots,
            local::local_upload,
            local::sync_get,
            local::sync_set,
            files::files_download_here,
            files::permissions_status,
            files::permissions_enable,
            files::permissions_recover,
            files::files_list,
            files::files_search,
            files::files_jobs,
            files::files_cancel,
            files::files_clear_jobs,
            files::files_upload_pick,
            files::files_upload_dropped,
            files::files_download,
            files::files_delete,
            files::files_bookmarks,
            files::files_bookmark_add,
            settings::settings_get,
            settings::settings_set,
            settings::settings_pick_log_dir,
            settings::update_status,
            settings::update_open_page,
            keygen::keygen_defaults,
            keygen::keygen_path_exists,
            keygen::keygen_pick_path,
            keygen::keygen_generate,
            forwards::forward_list,
            forwards::forward_start,
            forwards::forward_stop,
            snippets::snippets_list,
            snippets::snippets_save,
            snippets::snippet_variables,
            snippets::snippet_send,
            session::session_open_local,
            session::session_connect,
            session::session_input,
            session::session_resize,
            session::session_close,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, position }) = event {
                // ローカルのパスは画面へ渡さず、件数と落とした位置だけ知らせる。
                // 送信するかどうかは、画面がファイル一覧の上に落ちたかで決める。
                let app = window.app_handle();
                let count = app
                    .state::<app::AppState>()
                    .files
                    .set_dropped(paths.clone());
                let scale = window.scale_factor().unwrap_or(1.0);
                let _ = tauri::Emitter::emit(
                    app,
                    "file-drop",
                    serde_json::json!({"count": count, "x": position.x / scale, "y": position.y / scale}),
                );
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 未保存の編集があれば画面で確認する。なければ接続・端末を順に終わらせて閉じる。
                api.prevent_close();
                app::request_exit(window.app_handle(), false);
            }
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|_, _| {}),
        Err(error) => {
            eprintln!("画面を開始できません: {error}");
            std::process::exit(1);
        }
    }
}
