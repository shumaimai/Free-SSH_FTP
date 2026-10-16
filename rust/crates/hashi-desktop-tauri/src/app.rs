//! アプリ全体の状態とcommand。保存データ・認証要求の台帳・セッション一覧を持つ。
use crate::session::Session;
use hashi_core::interaction::{Challenge, Interaction, Reply, Request};
use hashi_storage::{
    config::{Profile, Snapshot, Store},
    paths::Channel,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

pub struct AppState {
    pub store: Arc<Store>,
    pub snapshot: Mutex<Snapshot>,
    pub startup_error: Option<String>,
    pub interaction: Interaction,
    requests: Mutex<Option<mpsc::Receiver<Request>>>,
    /// 画面へ出した認証要求。応答の照合に使う対象はこちらが持ち、画面からは受け取らない。
    challenges: Mutex<HashMap<String, Request>>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    exiting: AtomicBool,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let store = Arc::new(Store::new(Channel::Preview)?);
        let (snapshot, startup_error) = match store.load() {
            Ok(snapshot) => (snapshot, None),
            Err(error) => (
                Snapshot::default(),
                Some(format!(
                    "保存データを復旧してから再起動してください: {error}"
                )),
            ),
        };
        let (interaction, requests) = Interaction::new();
        Ok(Self {
            store,
            snapshot: Mutex::new(snapshot),
            startup_error,
            interaction,
            requests: Mutex::new(Some(requests)),
            challenges: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            exiting: AtomicBool::new(false),
        })
    }
    /// 終了処理を開始できたら真。2回目以降は偽。
    pub fn begin_exit(&self) -> bool {
        !self.exiting.swap(true, Ordering::SeqCst)
    }
    pub fn is_exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }
    fn writable(&self) -> Result<(), String> {
        match &self.startup_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    fn update_profiles(
        &self,
        change: impl FnOnce(&mut Vec<Profile>) -> std::io::Result<()>,
    ) -> Result<Vec<Profile>, String> {
        self.writable()?;
        let snapshot = self
            .store
            .transact(|snapshot| change(&mut snapshot.profiles))
            .map_err(|error| error.to_string())?;
        let profiles = snapshot.profiles.clone();
        *self.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
        Ok(profiles)
    }
}

/// 認証要求（ホスト鍵の確認・秘密入力）を画面へ流す。受け取った要求は応答の照合用に控える。
pub fn start_challenge_pump(app: AppHandle) {
    let state = app.state::<AppState>();
    let Some(mut requests) = state.requests.lock().ok().and_then(|mut r| r.take()) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        while let Some(request) = requests.recv().await {
            let state = app.state::<AppState>();
            if let Ok(mut challenges) = state.challenges.lock() {
                // 期限切れ・取消済みの要求を台帳から外す。
                challenges.retain(|id, item| state.interaction.is_pending(id, &item.scope));
                challenges.insert(request.challenge_id.clone(), request.clone());
            }
            let _ = app.emit("challenge", &request);
        }
    });
}

#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    Ok(serde_json::json!({
        "profiles": snapshot.profiles,
        "error": state.startup_error,
    }))
}

#[tauri::command]
pub fn profile_save(
    state: State<'_, AppState>,
    index: Option<usize>,
    profile: Profile,
) -> Result<Vec<Profile>, String> {
    state.update_profiles(|profiles| {
        profile.validate()?;
        if profile.host.trim().is_empty() || profile.username.trim().is_empty() {
            return Err(std::io::Error::other("ホストと利用者名を入力してください"));
        }
        match index {
            Some(index) if index < profiles.len() => profiles[index] = profile,
            Some(_) => return Err(std::io::Error::other("接続設定がありません")),
            None => profiles.push(profile),
        }
        Ok(())
    })
}

#[tauri::command]
pub fn profile_delete(state: State<'_, AppState>, index: usize) -> Result<Vec<Profile>, String> {
    state.update_profiles(|profiles| {
        if index >= profiles.len() {
            return Err(std::io::Error::other("接続設定がありません"));
        }
        profiles.remove(index);
        Ok(())
    })
}

/// 秘密鍵の選択。OSのファイル選択を使い、選ばれたパスだけを返す（中身は読まない）。
#[tauri::command]
pub async fn pick_key_file() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .pick_file()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "ファイル選択に失敗しました".to_owned())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChallengeAnswer {
    Trust { confirmation: String },
    Secret { value: String },
    Cancel,
}

#[tauri::command]
pub fn challenge_reply(
    state: State<'_, AppState>,
    challenge_id: String,
    answer: ChallengeAnswer,
) -> Result<(), String> {
    let request = state
        .challenges
        .lock()
        .map_err(|_| "認証台帳が不正です")?
        .remove(&challenge_id)
        .ok_or("認証要求が終了しています")?;
    let reply = match (&request.challenge, answer) {
        (_, ChallengeAnswer::Cancel) => Reply::Cancel,
        (Challenge::HostKey { .. }, ChallengeAnswer::Trust { confirmation }) => Reply::HostTrust {
            accept: true,
            confirmation,
        },
        (Challenge::Secret { .. }, ChallengeAnswer::Secret { value }) => Reply::Secret(value),
        _ => return Err("認証応答の種類が違います".into()),
    };
    let trusted = matches!(reply, Reply::HostTrust { accept: true, .. });
    let mut saved = None;
    state
        .interaction
        .reply_with(&request.challenge_id, &request.scope, reply, || {
            // 信頼は、接続側が応答を受け取れると確認できた後にだけ保存する。
            if trusted {
                if let Challenge::HostKey { key, .. } = &request.challenge {
                    let host = format!("{}:{}", request.scope.host, request.scope.port);
                    saved = Some(state.store.transact(|snapshot| {
                        snapshot.known_hosts.insert(host, key.clone());
                        Ok(())
                    })?);
                }
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    if let Some(snapshot) = saved {
        *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    }
    Ok(())
}

/// 全セッションを閉じる。接続の終了待ちには上限を置く。
pub async fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    let sessions: Vec<Arc<Session>> = state
        .sessions
        .lock()
        .map(|mut sessions| sessions.drain().map(|(_, session)| session).collect())
        .unwrap_or_default();
    for session in sessions {
        session.close().await;
    }
}
