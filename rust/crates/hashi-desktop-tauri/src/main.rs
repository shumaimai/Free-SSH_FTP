#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Hashi Rust版のWeb画面（Tauri 2 + WebView2）。核はhashi-core/hashi-storage/hashi-terminalを使い、
//! 画面（frontend/）とはcommandとeventだけで話す。画面側へ渡すのは表示に要るものだけで、
//! 秘密・認証要求の照合・信頼情報の保存は常にこちら（Rust側）で行う。
mod ai;
mod app;
mod files;
mod session;

use tauri::{Manager, WindowEvent};

fn main() {
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
            app::pick_key_file,
            app::appearance_get,
            app::appearance_set,
            app::challenge_reply,
            ai::ai_services,
            ai::ai_protocols,
            ai::ai_open_key_page,
            ai::ai_status,
            ai::ai_has_key,
            ai::ai_forget,
            ai::ai_connect,
            ai::ai_switch_model,
            ai::ai_clear,
            ai::ai_stop,
            ai::ai_send,
            ai::ai_confirm,
            ai::ai_cancel_preview,
            ai::approval_reply,
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
            session::session_open_local,
            session::session_connect,
            session::session_input,
            session::session_resize,
            session::session_close,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                // ローカルのパスは画面へ渡さず、件数だけ知らせる。
                let app = window.app_handle();
                let count = app
                    .state::<app::AppState>()
                    .files
                    .set_dropped(paths.clone());
                let _ = tauri::Emitter::emit(app, "file-drop", count);
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 接続・端末を順に終わらせてから閉じる。閉じる前に二重に押されても1回だけ走らせる。
                api.prevent_close();
                let app = window.app_handle().clone();
                let state = app.state::<app::AppState>();
                if state.begin_exit() {
                    tauri::async_runtime::spawn(async move {
                        app::shutdown(&app).await;
                        app.exit(0);
                    });
                }
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
