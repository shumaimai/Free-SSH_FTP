//! 設定（更新通知・セッションログ）と、新しいプレビュー版の確認。
//! 保存キーはPython版と同じ（`update_check` / `session_log` / `session_log_dir`）。他のキーには触れない。
use crate::app::AppState;
use hashi_core::update::{self, Update, UpdateChannel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub update_check: bool,
    pub session_log: bool,
    pub session_log_dir: String,
}

impl Settings {
    fn from_map(map: &serde_json::Map<String, Value>) -> Self {
        Self {
            update_check: map
                .get("update_check")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            session_log: map
                .get("session_log")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            session_log_dir: map
                .get("session_log_dir")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

pub fn read(state: &AppState) -> Settings {
    state
        .snapshot
        .lock()
        .map(|snapshot| Settings::from_map(&snapshot.settings))
        .unwrap_or(Settings {
            update_check: true,
            session_log: false,
            session_log_dir: String::new(),
        })
}

/// セッションログの保存先。空欄なら設定フォルダ/logs。旧Python版のlogsへは書かない。
pub fn log_directory(settings: &Settings) -> (Option<PathBuf>, PathBuf) {
    let custom = Some(settings.session_log_dir.trim())
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let default = hashi_storage::paths::config_root(hashi_storage::paths::Channel::Preview)
        .map(|root| root.join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("Hashi-Preview-logs"));
    (custom, default)
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Settings {
    read(&state)
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    state.writable()?;
    if settings.session_log_dir.len() > 1024 || settings.session_log_dir.contains('\0') {
        return Err("ログの保存先が不正です".into());
    }
    let dir = settings.session_log_dir.trim().to_owned();
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot
                .settings
                .insert("update_check".into(), Value::Bool(settings.update_check));
            snapshot
                .settings
                .insert("session_log".into(), Value::Bool(settings.session_log));
            snapshot
                .settings
                .insert("session_log_dir".into(), Value::String(dir));
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}

/// ログの保存先フォルダを選ぶ（OSのダイアログ）。
#[tauri::command]
pub async fn settings_pick_log_dir() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .pick_folder()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "フォルダの選択に失敗しました".to_owned())
}

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    tag: String,
    url: String,
}

/// 起動時の更新確認。読み取りだけで、失敗は画面へ出さない。終了を通信待ちで遅らせない。
pub fn start_update_check(app: AppHandle) {
    let state = app.state::<AppState>();
    if state.startup_error.is_some() || !read(&state).update_check {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let found = tokio::task::spawn_blocking(|| {
            update::check(UpdateChannel::Stable, env!("CARGO_PKG_VERSION"), None)
        })
        .await;
        if let Ok(Ok(Some(update))) = found {
            let Update { tag, url, .. } = update;
            let state = app.state::<AppState>();
            if let Ok(mut slot) = state.update.lock() {
                *slot = Some(UpdateInfo {
                    tag: tag.clone(),
                    url: url.clone(),
                });
            }
            let _ = app.emit("update-available", UpdateInfo { tag, url });
        }
    });
}

#[tauri::command]
pub fn update_status(state: State<'_, AppState>) -> Option<UpdateInfo> {
    state.update.lock().ok().and_then(|slot| slot.clone())
}

/// 新しい版のリリースページを開く。URLは画面から受け取らず、確認で得た公式のものだけを開く。
#[tauri::command]
pub fn update_open_page(state: State<'_, AppState>) -> Result<(), String> {
    let info = state
        .update
        .lock()
        .map_err(|_| "状態が不正です")?
        .clone()
        .ok_or("新しい版はありません")?;
    crate::browser::open(&info.url)
}
