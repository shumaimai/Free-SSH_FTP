//! 端末セッション（ローカルCMD・SSH）。出力は画面（xterm.js）へ生のバイト列で渡し、
//! 同じ出力をhashi-terminalのモデルへも流す（AI観測・ログ用。画面の描画には使わない）。
use crate::app::AppState;
use hashi_core::{
    broker::{Kind, OutputCapture, Registration, Registry},
    interaction::Stamp,
    local::{LocalTerminal, TerminalEvent},
    ssh::SshSession,
};
use hashi_terminal::Terminal;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const MAX_INPUT: usize = 1024 * 1024;

/// AI/MCPの端末台帳への登録（台帳のid・世代）。
type Binding = Arc<Mutex<Option<(String, u64)>>>;

/// 受信出力を台帳へ写す前の溜め。描画ごとに台帳を更新しないよう、一定間隔でまとめて渡す。
#[derive(Default)]
struct Pending {
    capture: OutputCapture,
    text: String,
    dirty: bool,
}

pub struct Session {
    pub id: String,
    pub label: String,
    cancel: CancellationToken,
    input: mpsc::Sender<Vec<u8>>,
    resize: Option<mpsc::Sender<(u32, u32)>>,
    local: Option<Arc<Mutex<LocalTerminal>>>,
    model: Arc<Mutex<Terminal>>,
    tracker: TaskTracker,
    binding: Binding,
    pending: Arc<Mutex<Pending>>,
}

#[derive(Serialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Clone)]
struct StateEvent<'a> {
    id: &'a str,
    state: &'a str,
    message: String,
}

fn emit_state(app: &AppHandle, id: &str, state: &str, message: impl Into<String>) {
    let _ = app.emit(
        "session-state",
        StateEvent {
            id,
            state,
            message: message.into(),
        },
    );
}

impl Session {
    fn new(label: String, cols: u16, rows: u16, input: mpsc::Sender<Vec<u8>>) -> (Self, Stamp) {
        let stamp = Stamp::default();
        let cancel = CancellationToken::new();
        (
            Self {
                id: stamp.terminal_id.clone(),
                label,
                cancel,
                input,
                resize: None,
                local: None,
                model: Arc::new(Mutex::new(Terminal::new(cols as usize, rows as usize))),
                tracker: TaskTracker::new(),
                binding: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(Pending::default())),
            },
            stamp,
        )
    }

    /// 取消 → 端末/接続の終了 → 受信タスクの回収。待ちには上限を置く。
    pub async fn close(&self, registry: &Registry) {
        self.cancel.cancel();
        unbind(registry, &self.binding);
        if let Some(local) = self.local.clone() {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    if let Ok(mut terminal) = local.lock() {
                        let _ = terminal.close();
                    }
                }),
            )
            .await;
        }
        self.tracker.close();
        let _ = tokio::time::timeout(Duration::from_secs(8), self.tracker.wait()).await;
    }
}

/// 台帳へ登録する。再接続で同じidを再登録すると世代が上がる。
fn bind(
    registry: &Registry,
    binding: &Binding,
    label: &str,
    kind: Kind,
    input: mpsc::Sender<Vec<u8>>,
    ssh: Option<Arc<SshSession>>,
) {
    let previous = binding
        .lock()
        .ok()
        .and_then(|b| b.as_ref().map(|(id, _)| id.clone()));
    let id = registry.register(
        previous,
        Registration {
            label: label.to_owned(),
            kind,
            shell: match kind {
                Kind::Ssh => "unknown".into(),
                Kind::Local if cfg!(windows) => "cmd".into(),
                Kind::Local => "sh".into(),
            },
            cwd: None,
            shareable: true,
            input,
            ssh,
        },
    );
    if let Ok(mut slot) = binding.lock() {
        *slot = Some(id);
    }
}

fn unbind(registry: &Registry, binding: &Binding) {
    if let Some((id, generation)) = binding.lock().ok().and_then(|mut b| b.take()) {
        registry.disconnect(&id, generation);
        registry.remove(&id);
    }
}

/// 受信出力をAI観測用に溜める。
fn capture(pending: &Mutex<Pending>, bytes: &[u8]) {
    if let Ok(mut pending) = pending.lock() {
        let text = pending.capture.push(bytes);
        pending.text.push_str(&text);
        pending.dirty = true;
    }
}

/// 溜めた出力と表示中の画面を、一定間隔で台帳へ写す。
fn spawn_flusher(session: &Session, registry: Arc<Registry>) {
    let (binding, pending, model, cancel) = (
        session.binding.clone(),
        session.pending.clone(),
        session.model.clone(),
        session.cancel.clone(),
    );
    session.tracker.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(150));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tick.tick() => {}
            }
            let text = {
                let Ok(mut pending) = pending.lock() else {
                    continue;
                };
                if !pending.dirty {
                    continue;
                }
                pending.dirty = false;
                std::mem::take(&mut pending.text)
            };
            let Some((id, generation)) = binding.lock().ok().and_then(|b| b.clone()) else {
                continue;
            };
            // 末尾の空行はAIへ送る観測データを増やすだけなので除く。
            let screen = model
                .lock()
                .map(|model| model.visible_text().join("\n").trim_end().to_owned())
                .unwrap_or_default();
            registry.update_output(&id, generation, &text, &screen);
        }
    });
}

/// 画面が選んだ端末（セッションid）を、台帳のid・世代へ変える。接続中・終了済みは除く。
pub fn bindings_for(state: &AppState, targets: &[String]) -> Vec<(String, u64)> {
    let Ok(sessions) = state.sessions.lock() else {
        return vec![];
    };
    targets
        .iter()
        .filter_map(|id| sessions.get(id))
        .filter_map(|session| session.binding.lock().ok().and_then(|b| b.clone()))
        .collect()
}

fn send_bytes(channel: &Channel<InvokeResponseBody>, bytes: Vec<u8>) -> bool {
    channel.send(InvokeResponseBody::Raw(bytes)).is_ok()
}

fn register(state: &AppState, session: Session) -> Result<Arc<Session>, String> {
    if state.is_exiting() {
        return Err("終了処理中です".into());
    }
    let session = Arc::new(session);
    state
        .sessions
        .lock()
        .map_err(|_| "セッション台帳が不正です")?
        .insert(session.id.clone(), session.clone());
    Ok(session)
}

fn clamp_size(cols: u32, rows: u32) -> Result<(u16, u16), String> {
    if cols == 0 || rows == 0 || cols > 1024 || rows > 512 {
        return Err("端末サイズが不正です".into());
    }
    Ok((cols as u16, rows as u16))
}

#[tauri::command]
pub async fn session_open_local(
    app: AppHandle,
    state: State<'_, AppState>,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
) -> Result<SessionInfo, String> {
    let (cols, rows) = clamp_size(cols, rows)?;
    let (tx, mut events) = mpsc::channel::<TerminalEvent>(128);
    let placeholder = mpsc::channel(1).0;
    let (mut session, stamp) = Session::new("CMD".into(), cols, rows, placeholder);
    let program = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    let local =
        LocalTerminal::spawn(program, &[], None, stamp, tx).map_err(|error| error.to_string())?;
    local
        .resize(cols, rows)
        .map_err(|error| error.to_string())?;
    session.input = local.input_sender();
    session.local = Some(Arc::new(Mutex::new(local)));
    let session = register(&state, session)?;
    let registry = state.ai.registry.clone();
    bind(
        &registry,
        &session.binding,
        &session.label,
        Kind::Local,
        session.input.clone(),
        None,
    );
    spawn_flusher(&session, registry.clone());
    let (id, model, cancel) = (
        session.id.clone(),
        session.model.clone(),
        session.cancel.clone(),
    );
    let (pending, binding) = (session.pending.clone(), session.binding.clone());
    let task_app = app.clone();
    session.tracker.spawn(async move {
        let app = task_app;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                event = events.recv() => match event {
                    Some(TerminalEvent::Data { bytes, .. }) => {
                        if let Ok(mut model) = model.lock() {
                            model.feed(&bytes);
                        }
                        capture(&pending, &bytes);
                        if !send_bytes(&on_data, bytes) {
                            break;
                        }
                    }
                    Some(TerminalEvent::Closed { .. }) | None => {
                        unbind(&registry, &binding);
                        emit_state(&app, &id, "closed", "ローカル端末が終了しました");
                        break;
                    }
                }
            }
        }
    });
    emit_state(&app, &session.id, "connected", "ローカル端末");
    Ok(SessionInfo {
        id: session.id.clone(),
        label: session.label.clone(),
    })
}

#[tauri::command]
pub async fn session_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
) -> Result<SessionInfo, String> {
    let (cols, rows) = clamp_size(cols, rows)?;
    let (profile, known) = {
        let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
        let profile = snapshot
            .profiles
            .get(index)
            .cloned()
            .ok_or("接続設定がありません")?;
        (profile, snapshot.known_hosts.clone())
    };
    let label = if profile.name.is_empty() {
        profile.host.clone()
    } else {
        profile.name.clone()
    };
    let (input, mut input_rx) = mpsc::channel::<Vec<u8>>(32);
    let (resize, mut resize_rx) = mpsc::channel::<(u32, u32)>(8);
    let (mut session, stamp) = Session::new(label, cols, rows, input);
    session.resize = Some(resize);
    let session = register(&state, session)?;
    let registry = state.ai.registry.clone();
    spawn_flusher(&session, registry.clone());
    let (id, model, cancel) = (
        session.id.clone(),
        session.model.clone(),
        session.cancel.clone(),
    );
    let (pending, binding, input_for_binding) = (
        session.pending.clone(),
        session.binding.clone(),
        session.input.clone(),
    );
    let label_for_binding = session.label.clone();
    let interaction = state.interaction.clone();
    emit_state(&app, &id, "connecting", "接続中");
    let task_app = app.clone();
    session.tracker.spawn(async move {
        let app = task_app;
        let result: anyhow::Result<()> = async {
            let ssh = Arc::new(
                SshSession::connect(&profile, stamp, known, interaction, cancel.clone()).await?,
            );
            let setup = ssh.shell(cols as u32, rows as u32);
            let channel = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(anyhow::anyhow!("接続の開始を中止しました")),
                result = tokio::time::timeout(Duration::from_secs(30), setup) => result
                    .map_err(|_| anyhow::anyhow!("接続の初期化がタイムアウトしました"))
                    .and_then(|result| result),
            };
            let channel = match channel {
                Ok(channel) => channel,
                Err(error) => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), ssh.close()).await;
                    return Err(error);
                }
            };
            // 接続できてから台帳へ登録する（独立実行は同じSSH接続の別チャネルを使う）。
            bind(
                &registry,
                &binding,
                &label_for_binding,
                Kind::Ssh,
                input_for_binding,
                Some(ssh.clone()),
            );
            emit_state(&app, &id, "connected", "接続済み");
            let (mut reader, writer) = channel.split();
            let outcome: anyhow::Result<()> = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            let _ = tokio::time::timeout(Duration::from_secs(1), writer.close()).await;
                            break;
                        }
                        bytes = input_rx.recv() => match bytes {
                            Some(bytes) => tokio::select! {
                                _ = cancel.cancelled() => break,
                                result = writer.data_bytes(bytes) => result?,
                            },
                            None => break,
                        },
                        size = resize_rx.recv() => if let Some((columns, rows)) = size {
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                result = writer.window_change(columns, rows, 0, 0) => result?,
                            }
                        },
                        message = reader.wait() => match message {
                            Some(russh::ChannelMsg::Data { data })
                            | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                                let bytes = data.to_vec();
                                if let Ok(mut model) = model.lock() {
                                    model.feed(&bytes);
                                }
                                capture(&pending, &bytes);
                                if !send_bytes(&on_data, bytes) {
                                    break;
                                }
                            }
                            Some(russh::ChannelMsg::Close) | None => break,
                            _ => {}
                        }
                    }
                }
                Ok(())
            }
            .await;
            unbind(&registry, &binding);
            let _ = tokio::time::timeout(Duration::from_secs(5), ssh.close()).await;
            outcome
        }
        .await;
        unbind(&registry, &binding);
        match result {
            Ok(()) => emit_state(&app, &id, "closed", "端末の接続が終了しました"),
            Err(error) => emit_state(&app, &id, "error", error.to_string()),
        }
    });
    Ok(SessionInfo {
        id: session.id.clone(),
        label: session.label.clone(),
    })
}

fn find(state: &AppState, id: &str) -> Result<Arc<Session>, String> {
    state
        .sessions
        .lock()
        .map_err(|_| "セッション台帳が不正です".to_owned())?
        .get(id)
        .cloned()
        .ok_or_else(|| "セッションが見つかりません".to_owned())
}

/// 端末への入力。画面（xterm.js）が組み立てた文字列をそのまま送る。満杯なら捨てずに失敗を返す。
#[tauri::command]
pub async fn session_input(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<(), String> {
    if data.len() > MAX_INPUT {
        return Err("端末入力が大きすぎます".into());
    }
    let session = find(&state, &id)?;
    tokio::time::timeout(
        Duration::from_secs(2),
        session.input.send(data.into_bytes()),
    )
    .await
    .map_err(|_| "端末入力の送信待ちがあふれました".to_owned())?
    .map_err(|_| "端末は終了しています".to_owned())
}

#[tauri::command]
pub fn session_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let (cols, rows) = clamp_size(cols, rows)?;
    let session = find(&state, &id)?;
    if let Ok(mut model) = session.model.lock() {
        model.resize(cols as usize, rows as usize);
    }
    if let Some(local) = &session.local {
        // 終了処理が同じロックを持っている間は待たず、次のresizeに任せる。
        if let Ok(local) = local.try_lock() {
            local
                .resize(cols, rows)
                .map_err(|error| error.to_string())?;
        }
    }
    if let Some(resize) = &session.resize {
        // 満杯なら最後のサイズだけが次の呼び出しで届けばよい。
        let _ = resize.try_send((cols as u32, rows as u32));
    }
    Ok(())
}

#[tauri::command]
pub async fn session_close(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let session = state
        .sessions
        .lock()
        .map_err(|_| "セッション台帳が不正です")?
        .remove(&id);
    if let Some(session) = session {
        session.close(&state.ai.registry).await;
        emit_state(&app, &id, "closed", "閉じました");
    }
    Ok(())
}
