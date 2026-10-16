#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Hashi Rust版のWeb画面（Tauri 2 + WebView2）。核はhashi-core/hashi-storage/hashi-terminalを使い、
//! 画面（frontend/）とはcommandとeventだけで話す。画面側へ渡すのは表示に要るものだけで、
//! 秘密・認証要求の照合・信頼情報の保存は常にこちら（Rust側）で行う。
mod app;
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::profiles_list,
            app::profile_save,
            app::profile_delete,
            app::pick_key_file,
            app::challenge_reply,
            session::session_open_local,
            session::session_connect,
            session::session_input,
            session::session_resize,
            session::session_close,
        ])
        .on_window_event(|window, event| {
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
