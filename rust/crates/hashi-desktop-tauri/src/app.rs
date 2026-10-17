//! アプリ全体の状態とcommand。保存データ・認証要求の台帳・セッション一覧を持つ。
use crate::external::External;
use crate::import::Imports;
use crate::local::LocalPane;
use crate::{
    ai::AiState, editor::Editors, files::FileJobs, session::Session, settings::UpdateInfo,
};
use hashi_core::interaction::{Challenge, Interaction, Reply, Request};
use hashi_storage::{
    config::{Profile, Snapshot, Store},
    paths::Channel,
};
use serde::{Deserialize, Serialize};
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
    pub ai: AiState,
    pub files: FileJobs,
    pub editors: Editors,
    pub external: External,
    pub local: LocalPane,
    pub imports: Imports,
    pub update: Mutex<Option<UpdateInfo>>,
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
            ai: AiState::new(),
            files: FileJobs::new(),
            editors: Editors::default(),
            external: External::default(),
            local: LocalPane::default(),
            imports: Imports::default(),
            update: Mutex::new(None),
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
    pub fn writable(&self) -> Result<(), String> {
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

/// 画面（WebView）で起きた例外を、設定フォルダの`client-errors.log`へ残す（白い画面などの原因調査用）。
/// 画面が渡した文字列は長さを切り、ファイルは小さく保つ。秘密を書かないよう、画面側は例外の文面だけを渡す。
#[tauri::command]
pub fn client_log(message: String) -> Result<(), String> {
    use std::io::Write;
    const LIMIT: u64 = 64 * 1024;
    let root = hashi_storage::paths::config_root(Channel::Preview).map_err(|e| e.to_string())?;
    let path = root.join("client-errors.log");
    let rotate = std::fs::metadata(&path)
        .map(|m| m.len() > LIMIT)
        .unwrap_or(false);
    if rotate {
        let _ = std::fs::rename(&path, root.join("client-errors.old.log"));
    }
    let text: String = message.chars().take(4000).collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "[{now}] {text}").map_err(|e| e.to_string())
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
    let mut removed_id: Option<(String, bool)> = None;
    let updated = state.update_profiles(|profiles| {
        if index >= profiles.len() {
            return Err(std::io::Error::other("接続設定がありません"));
        }
        let removed = profiles.remove(index);
        removed_id = Some((
            removed.id(),
            profiles.iter().any(|p| p.id() == removed.id()),
        ));
        Ok(())
    })?;
    // 同じ接続先の設定がほかに残っていなければ、保存済みの秘密も消す。
    if let Some((id, still_used)) = removed_id {
        if !still_used {
            if let Err(error) = crate::import::forget_id(&id) {
                eprintln!("保存済みの秘密を削除できませんでした: {error}");
            }
        }
    }
    Ok(updated)
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

/// 外観の設定。値は固定の候補のidだけで、画面（appearance.ts）と同じ一覧を持つ。
const MODES: [&str; 3] = ["system", "light", "dark"];
const ACCENTS: [&str; 7] = ["teal", "blue", "green", "purple", "rose", "orange", "slate"];
const UI_FONTS: [&str; 5] = ["system", "yugothic", "meiryo", "biz", "noto"];
const TERM_FONTS: [&str; 5] = ["cascadia", "consolas", "bizud", "msgothic", "courier"];
const TERM_SCHEMES: [&str; 5] = ["auto", "campbell", "onedark", "solarized", "light"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub mode: String,
    pub accent: String,
    pub ui_font: String,
    pub term_font: String,
    pub term_size: u32,
    pub term_scheme: String,
}

impl Appearance {
    /// 候補にない値・範囲外のサイズは受け付けない。画面が送った文字列をそのまま保存しない。
    pub fn validate(&self) -> Result<(), String> {
        let ok = MODES.contains(&self.mode.as_str())
            && ACCENTS.contains(&self.accent.as_str())
            && UI_FONTS.contains(&self.ui_font.as_str())
            && TERM_FONTS.contains(&self.term_font.as_str())
            && TERM_SCHEMES.contains(&self.term_scheme.as_str())
            && (10..=24).contains(&self.term_size);
        if ok {
            Ok(())
        } else {
            Err("外観の設定値が不正です".into())
        }
    }
}

#[tauri::command]
pub fn appearance_get(state: State<'_, AppState>) -> Result<Option<Appearance>, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    // 壊れた保存値は無いものとして扱い、画面は既定値で開く（上書きもしない）。
    Ok(snapshot
        .settings
        .get("appearance")
        .and_then(|value| serde_json::from_value::<Appearance>(value.clone()).ok())
        .filter(|appearance| appearance.validate().is_ok()))
}

#[tauri::command]
pub fn appearance_set(state: State<'_, AppState>, appearance: Appearance) -> Result<(), String> {
    state.writable()?;
    appearance.validate()?;
    let value = serde_json::to_value(&appearance).map_err(|error| error.to_string())?;
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot.settings.insert("appearance".into(), value);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}

/// 画面の細かな表示設定（隠しファイル・2ペイン・ファイルパネルの開閉など）。
/// 危険側へ寄せる値（上書き転送など）は含めない。壊れた保存値は無いものとして扱う。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiPrefs {
    pub remote_hidden: bool,
    pub local_hidden: bool,
    pub two_pane: bool,
    pub dock_open: bool,
    /// "files" / "ai"
    pub dock_tab: String,
}

impl UiPrefs {
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.dock_tab.as_str(), "files" | "ai") {
            Ok(())
        } else {
            Err("表示設定の値が不正です".into())
        }
    }
}

#[tauri::command]
pub fn ui_prefs_get(state: State<'_, AppState>) -> Result<Option<UiPrefs>, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    Ok(snapshot
        .settings
        .get("ui_prefs")
        .and_then(|value| serde_json::from_value::<UiPrefs>(value.clone()).ok())
        .filter(|prefs| prefs.validate().is_ok()))
}

#[tauri::command]
pub fn ui_prefs_set(state: State<'_, AppState>, prefs: UiPrefs) -> Result<(), String> {
    state.writable()?;
    prefs.validate()?;
    let value = serde_json::to_value(&prefs).map_err(|error| error.to_string())?;
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot.settings.insert("ui_prefs".into(), value);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}

/// 終了を要求する。`force`でなければ未保存のエディタがある間は画面に確認させる。
/// 二重に押されても終了処理は1回だけ走らせる。
pub fn request_exit(app: &AppHandle, force: bool) {
    let state = app.state::<AppState>();
    if !force && state.editors.dirty_count() > 0 {
        let _ = app.emit("close-requested", ());
        return;
    }
    if state.begin_exit() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            shutdown(&app).await;
            app.exit(0);
        });
    }
}

/// 未保存の確認を済ませた上での終了（画面の「保存せず終了」）。
#[tauri::command]
pub fn app_quit(app: AppHandle) {
    request_exit(&app, true);
}

/// 全セッションを閉じる。接続の終了待ちには上限を置く。
pub async fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    // AI処理・共有・承認待ちを先に止めてから、端末と接続を終える。
    state.ai.stop_all();
    state.external.close();
    if let Ok(sessions) = state.sessions.lock() {
        for id in sessions.keys() {
            state.files.cancel_session(id);
        }
    }
    let sessions: Vec<Arc<Session>> = state
        .sessions
        .lock()
        .map(|mut sessions| sessions.drain().map(|(_, session)| session).collect())
        .unwrap_or_default();
    // 接続ごとの終了待ち(確定ACK・権限復元を含む)は並行に行い、待ち時間を接続数に比例させない。
    let mut closing = tokio::task::JoinSet::new();
    for session in sessions {
        let registry = state.ai.registry.clone();
        closing.spawn(async move { session.close(&registry).await });
    }
    let mut warnings = vec![];
    while let Some(result) = closing.join_next().await {
        match result {
            Ok(list) => warnings.extend(list),
            Err(_) => {
                warnings.push("接続の終了処理が途中で止まりました(結果は確認できていません)".into())
            }
        }
    }
    if warnings.is_empty() {
        return;
    }
    for warning in &warnings {
        eprintln!("Hashi: {warning}");
    }
    // 配布版はコンソールが無いので、閉じる前に画面で知らせる(復旧記録は残してある)。
    let text = format!(
        "終了時に確認できなかった処理があります。\n\n{}\n\n権限の復旧記録は消していません。",
        warnings.join("\n")
    );
    let _ = tauri::async_runtime::spawn_blocking(move || {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Hashi の終了")
            .set_description(text)
            .set_buttons(rfd::MessageButtons::Ok)
            .show()
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Appearance {
        Appearance {
            mode: "system".into(),
            accent: "teal".into(),
            ui_font: "system".into(),
            term_font: "cascadia".into(),
            term_size: 14,
            term_scheme: "auto".into(),
        }
    }

    #[test]
    fn ui_prefs_accept_only_known_tabs() {
        let mut prefs = UiPrefs {
            remote_hidden: true,
            local_hidden: false,
            two_pane: true,
            dock_open: true,
            dock_tab: "files".into(),
        };
        assert!(prefs.validate().is_ok());
        prefs.dock_tab = "ai".into();
        assert!(prefs.validate().is_ok());
        prefs.dock_tab = "<script>".into();
        assert!(prefs.validate().is_err());
    }

    #[test]
    fn appearance_accepts_only_known_ids_and_sizes() {
        assert!(valid().validate().is_ok());
        for bad in [
            Appearance {
                mode: "x".into(),
                ..valid()
            },
            Appearance {
                accent: "#ff0000".into(),
                ..valid()
            },
            Appearance {
                ui_font: "evil; }".into(),
                ..valid()
            },
            Appearance {
                term_size: 9,
                ..valid()
            },
            Appearance {
                term_size: 25,
                ..valid()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }
}
