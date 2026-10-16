//! AI相談。接続設定・会話・端末の共有は、egui版と同じ規則（hashi-ai）を使う。
//! APIキーは画面へ返さない（保存済みかどうかだけ伝える）。端末の操作は必ずBrokerを通す。
use crate::app::AppState;
use hashi_ai::{
    api::ApiProvider,
    connect::{self, Outcome, PROTOCOLS, SERVICES},
    conversation::Conversation,
};
use hashi_core::broker::{Action, ApprovalRequest, Broker, Mode, Registry};
use hashi_storage::{
    paths::Channel,
    secrets::{EncryptedSecrets, Namespace},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const SHARE_LIFETIME: Duration = Duration::from_secs(15 * 60);

pub struct AiState {
    pub registry: Arc<Registry>,
    pub broker: Arc<Broker>,
    approval_rx: Mutex<Option<mpsc::Receiver<ApprovalRequest>>>,
    approvals: Mutex<HashMap<String, ApprovalRequest>>,
    provider: Mutex<Option<Arc<ApiProvider>>>,
    /// 接続時のサービスid・入力URL・モデル一覧。モデル切替で同じ接続先を再構築するのに使う。
    conn: Mutex<Option<Conn>>,
    conversation: Mutex<Option<Arc<tokio::sync::Mutex<Conversation>>>>,
    running: Mutex<Option<CancellationToken>>,
    preview: Mutex<Option<String>>,
    connecting: Mutex<bool>,
}

#[derive(Clone)]
struct Conn {
    service: String,
    base: String,
    models: Vec<String>,
}

impl AiState {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::default());
        let (approval_tx, approval_rx) = mpsc::channel(16);
        Self {
            broker: Arc::new(Broker::new(registry.clone(), Some(approval_tx))),
            registry,
            approval_rx: Mutex::new(Some(approval_rx)),
            approvals: Mutex::new(HashMap::new()),
            provider: Mutex::new(None),
            conn: Mutex::new(None),
            conversation: Mutex::new(None),
            running: Mutex::new(None),
            preview: Mutex::new(None),
            connecting: Mutex::new(false),
        }
    }

    /// 進行中のAI処理・共有・承認待ちをすべて止める。
    pub fn stop_all(&self) {
        self.cancel_running();
        self.broker.stop();
        if let Ok(mut approvals) = self.approvals.lock() {
            for (_, request) in approvals.drain() {
                let _ = request.reply.send(false);
            }
        }
    }

    fn cancel_running(&self) {
        if let Some(token) = self.running.lock().ok().and_then(|mut r| r.take()) {
            token.cancel();
        }
        if let Some(conversation) = self.conversation.lock().ok().and_then(|c| c.clone()) {
            if let Ok(conversation) = conversation.try_lock() {
                conversation.tools.cancel.cancel();
            }
        }
    }

    fn new_conversation(&self) {
        self.cancel_running();
        let provider = self.provider.lock().ok().and_then(|p| p.clone());
        let conversation = provider.map(|provider| {
            Arc::new(tokio::sync::Mutex::new(Conversation {
                provider,
                tools: hashi_ai::tools::TerminalTools {
                    broker: self.broker.clone(),
                    actor: format!("ai:{}", uuid::Uuid::new_v4().simple()),
                    cancel: CancellationToken::new(),
                },
                messages: vec![],
            }))
        });
        if let Ok(mut slot) = self.conversation.lock() {
            *slot = conversation;
        }
    }
}

fn secrets() -> Result<EncryptedSecrets, String> {
    EncryptedSecrets::new(Channel::Preview, Namespace::Ai).map_err(|e| e.to_string())
}

/// 承認待ちを画面へ流す。受信側が閉じた（取消済みの）要求は画面から外す。
pub fn start_pumps(app: AppHandle) {
    let state = app.state::<AppState>();
    let Some(mut requests) = state.ai.approval_rx.lock().ok().and_then(|mut r| r.take()) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                request = requests.recv() => {
                    let Some(request) = request else { break };
                    let id = uuid::Uuid::new_v4().to_string();
                    let payload = json!({
                        "id": id,
                        "actor": request.actor,
                        "action": match request.action {
                            Action::SendInput => "send_input",
                            Action::RunCommand => "run_command",
                            Action::Interrupt => "interrupt",
                        },
                        "label": request.label,
                        "generation": request.generation,
                        "cwd": request.cwd,
                        "text": request.text,
                    });
                    let state = app.state::<AppState>();
                    if let Ok(mut approvals) = state.ai.approvals.lock() {
                        approvals.insert(id, request);
                    }
                    let _ = app.emit("approval", payload);
                }
                _ = tick.tick() => {
                    let state = app.state::<AppState>();
                    let closed: Vec<String> = state.ai.approvals.lock().map(|approvals| {
                        approvals.iter().filter(|(_, r)| r.reply.is_closed()).map(|(id, _)| id.clone()).collect()
                    }).unwrap_or_default();
                    if !closed.is_empty() {
                        if let Ok(mut approvals) = state.ai.approvals.lock() {
                            for id in &closed {
                                approvals.remove(id);
                            }
                        }
                        for id in closed {
                            let _ = app.emit("approval-closed", id);
                        }
                    }
                }
            }
        }
    });
}

#[tauri::command]
pub fn approval_reply(state: State<'_, AppState>, id: String, allow: bool) -> Result<(), String> {
    let request = state
        .ai
        .approvals
        .lock()
        .map_err(|_| "承認台帳が不正です")?
        .remove(&id)
        .ok_or("承認要求は終了しています")?;
    request
        .reply
        .send(allow)
        .map_err(|_| "承認要求は終了しています".to_owned())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    id: &'static str,
    label: &'static str,
    needs_key: bool,
    has_key_page: bool,
    blurb: &'static str,
    default_base: &'static str,
}

#[tauri::command]
pub fn ai_services() -> Vec<ServiceInfo> {
    SERVICES
        .iter()
        .map(|s| ServiceInfo {
            id: s.id,
            label: s.label,
            needs_key: s.needs_key,
            has_key_page: !s.key_page.is_empty(),
            blurb: s.blurb,
            default_base: s.base,
        })
        .collect()
}

/// キーの取得ページを開く。URLは画面から受け取らず、サービスごとの固定のものだけを開く。
#[tauri::command]
pub fn ai_open_key_page(service: String) -> Result<(), String> {
    let service = connect::service(&service).ok_or("サービスが不正です")?;
    if service.key_page.is_empty() {
        return Err("このサービスにはキーの取得ページがありません".into());
    }
    webbrowser::open(service.key_page).map_err(|error| format!("ブラウザを開けません: {error}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    connected: bool,
    service: Option<String>,
    model: Option<String>,
    models: Vec<String>,
    /// 前回の接続設定（キーは含まない）。
    saved: Option<Value>,
}

fn status(state: &AppState) -> AiStatus {
    let provider = state.ai.provider.lock().ok().and_then(|p| p.clone());
    let conn = state.ai.conn.lock().ok().and_then(|c| c.clone());
    let saved = state
        .snapshot
        .lock()
        .ok()
        .and_then(|s| s.settings.get("ai_connection").cloned());
    AiStatus {
        connected: provider.is_some(),
        service: conn.as_ref().map(|c| c.service.clone()),
        model: provider.map(|p| p.model.clone()),
        models: conn.map(|c| c.models).unwrap_or_default(),
        saved,
    }
}

#[tauri::command]
pub fn ai_status(state: State<'_, AppState>) -> AiStatus {
    status(&state)
}

/// 保存済みのキーがあるか。キーそのものは返さない。
#[tauri::command]
pub async fn ai_has_key(service: String, base: String) -> Result<bool, String> {
    let service = connect::service(&service).ok_or("サービスが不正です")?;
    if !service.needs_key {
        return Ok(false);
    }
    let base = connect::effective_base(service, &base);
    let id = connect::key_id(connect::kind_for(service, &base), &base)?;
    tokio::task::spawn_blocking(move || {
        Ok(secrets()?
            .get(&id)
            .map_err(|_| "保存したキーを読み込めませんでした。".to_owned())?
            .is_some_and(|key| !key.is_empty()))
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())?
}

#[tauri::command]
pub async fn ai_forget(service: String, base: String) -> Result<(), String> {
    let service = connect::service(&service).ok_or("サービスが不正です")?;
    let base = connect::effective_base(service, &base);
    let id = connect::key_id(connect::kind_for(service, &base), &base)?;
    tokio::task::spawn_blocking(move || {
        secrets()?
            .set(&id, None)
            .map_err(|_| "保存したキーを削除できませんでした。".to_owned())
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectArgs {
    service: String,
    base: String,
    /// 空なら保存済みのキーを使う。
    key: String,
    model: String,
    protocol: String,
    use_tools: bool,
    persist: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResult {
    /// "connected" か "need_model"。
    kind: &'static str,
    message: String,
    model: Option<String>,
    models: Vec<String>,
}

fn load_saved_key(service: &connect::Service, base: &str) -> Result<String, String> {
    let id = connect::key_id(connect::kind_for(service, base), base)?;
    Ok(secrets()?
        .get(&id)
        .map_err(|_| "保存したキーを読み込めませんでした。もう一度入力してください。".to_owned())?
        .unwrap_or_default())
}

fn save_connection(
    state: &AppState,
    service: &str,
    base: &str,
    provider: &ApiProvider,
) -> Result<(), String> {
    let saved = json!({
        "service": service,
        "base": base,
        "model": provider.model,
        "protocol": provider.protocol,
        "protocol_auto": provider.auto_protocol,
        "use_tools": provider.use_tools,
    });
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot.settings.insert("ai_connection".into(), saved);
            Ok(())
        })
        .map_err(|_| {
            "接続設定を保存できませんでした。設定フォルダの書込み権限を確認してください。"
                .to_owned()
        })?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}

fn adopt(state: &AppState, service: &str, base: &str, provider: ApiProvider, models: Vec<String>) {
    let conn = Conn {
        service: service.to_owned(),
        base: base.to_owned(),
        models,
    };
    state.ai.cancel_running();
    if let Ok(mut slot) = state.ai.provider.lock() {
        *slot = Some(Arc::new(provider));
    }
    if let Ok(mut slot) = state.ai.conn.lock() {
        *slot = Some(conn);
    }
    state.ai.new_conversation();
}

/// 「接続する」。モデル一覧の取得・応答の確認・設定とキーの保存を1回で行う。
#[tauri::command]
pub async fn ai_connect(
    state: State<'_, AppState>,
    args: ConnectArgs,
) -> Result<ConnectResult, String> {
    state.writable()?;
    {
        let mut busy = state.ai.connecting.lock().map_err(|_| "状態が不正です")?;
        if *busy {
            return Err("接続の確認中です".into());
        }
        *busy = true;
    }
    struct Release<'a>(&'a AppState);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            if let Ok(mut busy) = self.0.ai.connecting.lock() {
                *busy = false;
            }
        }
    }
    let _release = Release(&state);
    let service = connect::service(&args.service).ok_or("サービスが不正です")?;
    let base = connect::effective_base(service, &args.base);
    let typed_model = args.model.trim().to_owned();
    let mut key = args.key.trim().to_owned();
    let (persist, protocol, use_tools) = (args.persist, args.protocol.clone(), args.use_tools);
    let base_for_job = args.base.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        if key.is_empty() && service.needs_key {
            key = load_saved_key(service, &base)?;
        }
        let probe = connect::build(
            service,
            &base_for_job,
            &key,
            &typed_model,
            &protocol,
            use_tools,
        )?;
        let id = connect::key_id(&probe.kind, &probe.base)?;
        connect::connect(
            probe,
            &typed_model,
            &id,
            persist && service.needs_key,
            &secrets()?,
        )
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())??;
    match outcome {
        Outcome::NeedModel(message, models) => Ok(ConnectResult {
            kind: "need_model",
            message,
            model: None,
            models,
        }),
        Outcome::Connected(provider, models) => {
            save_connection(&state, &args.service, &args.base, &provider)?;
            let model = provider.model.clone();
            adopt(&state, &args.service, &args.base, *provider, models.clone());
            Ok(ConnectResult {
                kind: "connected",
                message: String::new(),
                model: Some(model),
                models,
            })
        }
    }
}

/// 起動時に前回の接続を読み戻す。通信はせず、保存キーが読めれば会話できる状態にする。
pub fn restore(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(saved) = state
        .snapshot
        .lock()
        .ok()
        .and_then(|s| s.settings.get("ai_connection").cloned())
    else {
        return;
    };
    let (Some(service_id), Some(model)) = (saved["service"].as_str(), saved["model"].as_str())
    else {
        return;
    };
    let Some(service) = connect::service(service_id) else {
        return;
    };
    let typed = saved["base"].as_str().unwrap_or_default();
    let base = connect::effective_base(service, typed);
    let key = if service.needs_key {
        match load_saved_key(service, &base) {
            Ok(key) if !key.is_empty() => key,
            _ => return,
        }
    } else {
        String::new()
    };
    let protocol = saved["protocol"].as_str().unwrap_or("auto");
    let use_tools = saved["use_tools"].as_bool().unwrap_or(true);
    if let Ok(mut provider) = connect::build(service, typed, &key, model, protocol, use_tools) {
        provider.auto_protocol = saved["protocol_auto"].as_bool().unwrap_or(false);
        adopt(&state, service_id, typed, provider, vec![]);
    }
}

/// モデルを切り替える。通信はせず、会話は新しくする。
#[tauri::command]
pub async fn ai_switch_model(state: State<'_, AppState>, model: String) -> Result<(), String> {
    state.writable()?;
    let current = state
        .ai
        .provider
        .lock()
        .map_err(|_| "状態が不正です")?
        .clone()
        .ok_or("AIは未接続です")?;
    let conn = state
        .ai
        .conn
        .lock()
        .map_err(|_| "状態が不正です")?
        .clone()
        .ok_or("AIは未接続です")?;
    let model = model.trim().to_owned();
    if model.is_empty() {
        return Err("モデルを指定してください".into());
    }
    // 方式を利用者が選んでいないなら、切り替え先のモデルに合う方式を一覧から選び直す（取得できなければ名前で決める）。
    let protocol = if current.auto_protocol {
        let probe = current.clone();
        let listed = tokio::task::spawn_blocking(move || {
            probe
                .models(&tokio_util::sync::CancellationToken::new())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        connect::protocol_from_endpoints(&model, &listed)
            .unwrap_or("auto")
            .to_owned()
    } else {
        current.protocol.clone()
    };
    let mut provider = ApiProvider::new(
        &current.kind,
        &model,
        current.key(),
        Some(&current.base),
        current.use_tools,
        &protocol,
    )
    .map_err(|e| e.to_string())?;
    provider.auto_protocol = current.auto_protocol;
    save_connection(&state, &conn.service, &conn.base, &provider)?;
    adopt(&state, &conn.service, &conn.base, provider, conn.models);
    Ok(())
}

#[tauri::command]
pub fn ai_clear(state: State<'_, AppState>) {
    state.ai.new_conversation();
}

#[tauri::command]
pub fn ai_stop(state: State<'_, AppState>) {
    state.ai.cancel_running();
    state.ai.broker.stop();
}

fn parse_mode(mode: &str) -> Result<Mode, String> {
    match mode {
        "consult" => Ok(Mode::Consult),
        "confirm" => Ok(Mode::Confirm),
        "auto" => Ok(Mode::Auto),
        _ => Err("実行モードが不正です".into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// 共有する端末があるとき、AIへ送る前に利用者が確認・編集する内容。
    preview: Option<String>,
}

/// 質問を送る。共有対象があれば、観測データを確認画面へ返して止まる（確認なしでは送らない）。
#[tauri::command]
pub async fn ai_send(
    app: AppHandle,
    state: State<'_, AppState>,
    question: String,
    targets: Vec<String>,
    mode: String,
) -> Result<SendResult, String> {
    let question = question.trim().to_owned();
    if question.is_empty() {
        return Err("質問を入力してください".into());
    }
    if question.len() > 64 * 1024 {
        return Err("質問が長すぎます".into());
    }
    if state
        .ai
        .running
        .lock()
        .map_err(|_| "状態が不正です")?
        .is_some()
        || state
            .ai
            .preview
            .lock()
            .map_err(|_| "状態が不正です")?
            .is_some()
    {
        return Err("前の送信が終わっていません".into());
    }
    if state
        .ai
        .conversation
        .lock()
        .map_err(|_| "状態が不正です")?
        .is_none()
    {
        return Err("AIが未接続です".into());
    }
    let ids = crate::session::bindings_for(&state, &targets);
    state
        .ai
        .broker
        .configure(parse_mode(&mode)?, &ids, SHARE_LIFETIME)
        .map_err(|error| error.to_string())?;
    if ids.is_empty() {
        begin(&app, &state, question, String::new())?;
        return Ok(SendResult { preview: None });
    }
    let context = hashi_ai::observe::context_snapshot(&state.ai.broker);
    *state.ai.preview.lock().map_err(|_| "状態が不正です")? = Some(question);
    Ok(SendResult {
        preview: Some(context),
    })
}

/// 確認した内容でAIへ送る。画面で編集した内容をそのまま使う。
#[tauri::command]
pub fn ai_confirm(
    app: AppHandle,
    state: State<'_, AppState>,
    context: String,
) -> Result<(), String> {
    if context.len() > 512 * 1024 {
        return Err("送る内容が大きすぎます".into());
    }
    let question = state
        .ai
        .preview
        .lock()
        .map_err(|_| "状態が不正です")?
        .take()
        .ok_or("確認待ちの送信がありません")?;
    begin(&app, &state, question, context)
}

#[tauri::command]
pub fn ai_cancel_preview(state: State<'_, AppState>) {
    if let Ok(mut preview) = state.ai.preview.lock() {
        *preview = None;
    }
    state.ai.broker.stop();
}

fn begin(
    app: &AppHandle,
    state: &AppState,
    question: String,
    context: String,
) -> Result<(), String> {
    let conversation = state
        .ai
        .conversation
        .lock()
        .map_err(|_| "状態が不正です")?
        .clone()
        .ok_or("AIが未接続です")?;
    let token = CancellationToken::new();
    *state.ai.running.lock().map_err(|_| "状態が不正です")? = Some(token.clone());
    let (delta_tx, mut deltas) = mpsc::channel::<String>(256);
    let forward = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(text) = deltas.recv().await {
            let _ = forward.emit("ai-delta", text);
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = {
            let mut conversation = conversation.lock().await;
            // 停止で取り消した前回のtokenを使い回さない。
            conversation.tools.cancel = token.child_token();
            conversation
                .ask(&question, &context, &token, delta_tx)
                .await
        };
        let state = app.state::<AppState>();
        if let Ok(mut running) = state.ai.running.lock() {
            *running = None;
        }
        let payload = match result {
            Ok(text) => json!({"error": null, "text": text}),
            Err(error) => json!({"error": error.to_string(), "text": ""}),
        };
        let _ = app.emit("ai-done", payload);
    });
    Ok(())
}

/// 詳細設定で選べるAPIの通信形式（id, 表示名）。
#[tauri::command]
pub fn ai_protocols() -> Vec<(&'static str, &'static str)> {
    PROTOCOLS.to_vec()
}
