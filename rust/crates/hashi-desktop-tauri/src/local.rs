//! ローカル側のファイルペイン（2ペインの左）と同期ブラウズ。
//! 開いているフォルダはRust側が持ち、転送の依頼は「そのフォルダ内の名前」だけで受ける
//! （画面からローカルの絶対パスを受け取って読み書きしない）。左右の移動の写し方は
//! `hashi_core::sync_browse`（相対移動だけ。絶対パスは写さない）で、跳ね返りは要求のStampで判定する。
use crate::{
    app::AppState,
    files::{enqueue, file_name, files_of, parse_mode, parse_policy, remote_join, Operation},
};
use hashi_core::{
    interaction::Stamp,
    sync_browse::{Follow, SyncState},
    transfer::{self, Entry},
    transfer_tree,
};
use serde::Serialize;
#[cfg(windows)]
use std::{path::Path, time::Duration};
use std::{path::PathBuf, sync::Mutex};
use tauri::{AppHandle, State};

pub struct LocalPane {
    dir: Mutex<Option<PathBuf>>,
    /// 最新の一覧要求。古い要求の結果は捨てる。
    revision: Mutex<Stamp>,
    sync: Mutex<SyncState>,
}

impl Default for LocalPane {
    fn default() -> Self {
        Self {
            dir: Mutex::new(None),
            revision: Mutex::new(Stamp::default()),
            sync: Mutex::new(SyncState::default()),
        }
    }
}

/// 画面へ返す追随の指示。`remote`/`local`は追随先の絶対パス、`failed`は理由（通知に出す）。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FollowView {
    Remote { path: String },
    Local { path: String },
    Failed { message: String },
}

fn view(follow: Option<Follow>) -> Option<FollowView> {
    follow.map(|follow| match follow {
        Follow::Remote(path) => FollowView::Remote { path },
        Follow::Local(path) => FollowView::Local {
            path: path.to_string_lossy().into_owned(),
        },
        Follow::Failed(message) => FollowView::Failed { message },
    })
}

/// リモート側の「タブ」の識別（接続のidで足りる）。
pub fn remote_stamp(session_id: &str) -> Stamp {
    Stamp {
        terminal_id: session_id.to_owned(),
        generation: "preview".into(),
    }
}

impl LocalPane {
    pub fn directory(&self) -> Option<PathBuf> {
        self.dir.lock().ok().and_then(|dir| dir.clone())
    }
    pub fn expect_remote(&self, navigation: Stamp) {
        if let Ok(mut sync) = self.sync.lock() {
            sync.expect_remote(navigation);
        }
    }
    pub fn remote_failed(&self, navigation: &Stamp, message: &str) -> Option<String> {
        self.sync
            .lock()
            .ok()
            .and_then(|mut sync| sync.remote_failed(navigation, message))
    }
    pub fn on_remote(&self, tab: &Stamp, navigation: &Stamp, path: &str) -> Option<FollowView> {
        let follow = self
            .sync
            .lock()
            .ok()
            .and_then(|mut sync| sync.on_remote(tab, navigation, path));
        view(follow)
    }
}

#[derive(Serialize)]
pub struct LocalListing {
    path: String,
    /// 親フォルダ（ルートでは無し）。
    parent: Option<String>,
    entries: Vec<Entry>,
    follow: Option<FollowView>,
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

async fn list(
    state: &AppState,
    path: Option<String>,
    follow: bool,
) -> Result<LocalListing, String> {
    let pane = &state.local;
    let target = match path.filter(|p| !p.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => pane.directory().unwrap_or_else(home),
    };
    if !target.is_absolute() {
        return Err("ローカルのフォルダは絶対パスで指定してください".into());
    }
    // `..`や`.`を含むパスは開かない(同期ブラウズの追随などで、見た目と違うフォルダへ移らないように)。
    if target.components().any(|part| {
        matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err("「..」を含むパスは開けません。フォルダを直接指定してください".into());
    }
    let revision = Stamp::default();
    *pane.revision.lock().map_err(|_| "状態が不正です")? = revision.clone();
    if follow {
        pane.sync
            .lock()
            .map_err(|_| "状態が不正です")?
            .expect_local(revision.clone());
    }
    let result = transfer_tree::list_local(&target).await;
    if pane.revision.lock().map_err(|_| "状態が不正です")?.clone() != revision {
        return Err("別の一覧要求に置き換えられました".into());
    }
    let entries = match result {
        Ok(entries) => entries,
        Err(error) => {
            let message = error.to_string();
            return Err(pane
                .sync
                .lock()
                .ok()
                .and_then(|mut sync| sync.local_failed(&revision))
                .unwrap_or(message));
        }
    };
    let follow = pane
        .sync
        .lock()
        .map_err(|_| "状態が不正です")?
        .on_local(&revision, &target);
    *pane.dir.lock().map_err(|_| "状態が不正です")? = Some(target.clone());
    Ok(LocalListing {
        parent: target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned()),
        path: target.to_string_lossy().into_owned(),
        entries,
        follow: view(follow),
    })
}

/// ローカルの一覧。pathが空なら今開いているフォルダ（無ければホーム）。
#[tauri::command]
pub async fn local_list(
    state: State<'_, AppState>,
    path: Option<String>,
    follow: Option<bool>,
) -> Result<LocalListing, String> {
    list(&state, path, follow.unwrap_or(false)).await
}

/// OSのフォルダ選択で選んだフォルダを開く。取り消したらNone。
#[tauri::command]
pub async fn local_pick_dir(state: State<'_, AppState>) -> Result<Option<LocalListing>, String> {
    let start = state.local.directory();
    let picked = tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder()
    })
    .await
    .map_err(|_| "フォルダの選択に失敗しました".to_owned())?;
    match picked {
        Some(path) => list(&state, Some(path.to_string_lossy().into_owned()), false)
            .await
            .map(Some),
        None => Ok(None),
    }
}

/// 開けるドライブ（Windows）。応答しないネットワークドライブで固まらないよう、待ちに上限を置く。
#[tauri::command]
pub async fn local_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        let probe = tokio::task::spawn_blocking(|| {
            ('A'..='Z')
                .map(|letter| format!("{letter}:\\"))
                .filter(|root| Path::new(root).exists())
                .collect::<Vec<_>>()
        });
        tokio::time::timeout(Duration::from_millis(2500), probe)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        vec!["/".to_owned()]
    }
}

/// ローカルペインで選んだ項目（開いているフォルダ内の名前）を、リモートのフォルダへ送る。
#[tauri::command]
pub fn local_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    names: Vec<String>,
    remote_dir: String,
    policy: String,
    mode: String,
) -> Result<usize, String> {
    let (policy, mode) = (parse_policy(&policy)?, parse_mode(&mode)?);
    files_of(&state, &id)?;
    let dir = state
        .local
        .directory()
        .ok_or("ローカルのフォルダがまだ開かれていません")?;
    if names.is_empty() || names.len() > 256 {
        return Err("送る項目を選んでください（一度に256件まで）".into());
    }
    let mut queued = 0;
    for name in names {
        transfer::safe_name(&name).map_err(|e| e.to_string())?;
        let local = dir.join(&name);
        // 画面が選べるのは一覧にあった名前だけ。無くなっていたら送らない。
        if std::fs::symlink_metadata(&local).is_err() {
            return Err(format!("{name} が見つかりません。一覧を更新してください"));
        }
        let label = file_name(&local);
        enqueue(
            &app,
            &state,
            &id,
            format!("送信 {label}"),
            Operation::Upload {
                remote: remote_join(&remote_dir, &name),
                local,
            },
            policy,
            mode,
        )?;
        queued += 1;
    }
    Ok(queued)
}

#[tauri::command]
pub fn sync_get(state: State<'_, AppState>) -> bool {
    state
        .snapshot
        .lock()
        .ok()
        .and_then(|snapshot| {
            snapshot
                .settings
                .get("sync_browse")
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false)
}

/// 同期ブラウズの入/切と基準の取り直し。表示中の接続（またはNone）と、そのリモートの現在位置を基準にする。
#[tauri::command]
pub fn sync_set(
    state: State<'_, AppState>,
    enabled: bool,
    session_id: Option<String>,
    remote_path: Option<String>,
) -> Result<(), String> {
    state.writable()?;
    {
        let mut sync = state.local.sync.lock().map_err(|_| "状態が不正です")?;
        sync.enabled = enabled;
        let local = state.local.directory().unwrap_or_else(home);
        let tab = session_id.map(|id| (remote_stamp(&id), remote_path.unwrap_or_default()));
        sync.rebase(&local, tab.as_ref().map(|(s, p)| (s, p.as_str())));
    }
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot
                .settings
                .insert("sync_browse".into(), serde_json::Value::Bool(enabled));
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}
