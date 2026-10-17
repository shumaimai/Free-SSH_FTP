//! AI相談。接続設定・会話・端末の共有は、egui版と同じ規則（hashi-ai）を使う。
//! APIキーは画面へ返さない（保存済みかどうかだけ伝える）。端末の操作は必ずBrokerを通す。
use crate::app::AppState;
use hashi_ai::{
    api::ApiProvider,
    connect::{self, PROTOCOLS, SERVICES},
    conversation::Conversation,
};
use hashi_core::broker::{Action, ApprovalRequest, Broker, Mode, Registry, NO_LIMIT};
use hashi_storage::{
    paths::Channel,
    secrets::{EncryptedSecrets, Namespace},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct AiState {
    pub registry: Arc<Registry>,
    /// AIドック専用の許可。外部連携(MCP)の許可とは別に持ち、互いに上書き・停止しない。
    pub broker: Arc<Broker>,
    /// 外部連携(MCP)専用の許可。承認窓の経路だけAIと共有する。
    pub external_broker: Arc<Broker>,
    approval_rx: Mutex<Option<mpsc::Receiver<ApprovalRequest>>>,
    approvals: Mutex<HashMap<String, ApprovalRequest>>,
    pub(crate) provider: Mutex<Option<Arc<ApiProvider>>>,
    /// 接続時のサービスid・入力URL・モデル一覧。モデル切替で同じ接続先を再構築するのに使う。
    pub(crate) conn: Mutex<Option<Conn>>,
    pub(crate) conversation: Mutex<Option<Arc<tokio::sync::Mutex<Conversation>>>>,
    pub(crate) running: Mutex<Option<CancellationToken>>,
    /// 確認待ちの送信（質問・実行モード・対象）。確認されるまで実行モードは与えない。
    pub(crate) preview: Mutex<Option<Pending>>,
    pub(crate) connecting: Mutex<bool>,
}

pub(crate) struct Pending {
    question: String,
    images: Vec<serde_json::Value>,
    mode: Mode,
    ids: Vec<(String, u64)>,
}

#[derive(Clone)]
pub(crate) struct Conn {
    service: String,
    models: Vec<String>,
}

impl AiState {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::default());
        let (approval_tx, approval_rx) = mpsc::channel(16);
        Self {
            broker: Arc::new(Broker::new(registry.clone(), Some(approval_tx.clone()))),
            external_broker: Arc::new(Broker::new(registry.clone(), Some(approval_tx))),
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
        self.external_broker.stop();
        if let Ok(mut approvals) = self.approvals.lock() {
            for (_, request) in approvals.drain() {
                let _ = request.reply.send(false);
            }
        }
    }

    pub(crate) fn cancel_running(&self) {
        if let Some(token) = self.running.lock().ok().and_then(|mut r| r.take()) {
            token.cancel();
        }
        if let Some(conversation) = self.conversation.lock().ok().and_then(|c| c.clone()) {
            if let Ok(conversation) = conversation.try_lock() {
                conversation.tools.cancel.cancel();
            }
        }
    }

    pub(crate) fn new_conversation(&self) {
        self.cancel_running();
        if let Ok(mut preview) = self.preview.lock() {
            *preview = None;
        }
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

pub(crate) fn secrets() -> Result<EncryptedSecrets, String> {
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
                            Action::SendKeys => "send_keys",
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
    /// キーではなくアカウントでサインインして使う(ChatGPT)。
    sign_in: bool,
}

/// ChatGPTのサインイン。キーの代わりにアカウントで使う(`ai_models`が扱う)。
pub(crate) const CHATGPT: &str = "chatgpt";

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
            sign_in: false,
        })
        .chain(std::iter::once(ServiceInfo {
            id: CHATGPT,
            label: "ChatGPT（サインイン）",
            needs_key: false,
            has_key_page: false,
            blurb: "ChatGPTのアカウントでサインインして使います（APIキーは不要）。ChatGPTプランの利用枠は、ChatGPTの他のアプリと共有されます。",
            default_base: "",
            sign_in: true,
        }))
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
    if service == CHATGPT {
        return Ok(false);
    }
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

pub(crate) fn load_saved_key(service: &connect::Service, base: &str) -> Result<String, String> {
    let id = connect::key_id(connect::kind_for(service, base), base)?;
    Ok(secrets()?
        .get(&id)
        .map_err(|_| "保存したキーを読み込めませんでした。もう一度入力してください。".to_owned())?
        .unwrap_or_default())
}

pub(crate) fn save_connection(
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

pub(crate) fn adopt(state: &AppState, service: &str, provider: ApiProvider, models: Vec<String>) {
    let conn = Conn {
        service: service.to_owned(),
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

/// 起動時に前回の接続を読み戻す。通信はせず、保存キーが読めれば会話できる状態にする。
pub fn restore(app: &AppHandle) {
    crate::ai_models::restore_active(app);
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
    attachments: Option<Vec<crate::attach::Attachment>>,
) -> Result<SendResult, String> {
    let question = question.trim().to_owned();
    let attachments = attachments.unwrap_or_default();
    if question.is_empty() && attachments.is_empty() {
        return Err("質問を入力してください".into());
    }
    if question.len() > 64 * 1024 {
        return Err("質問が長すぎます".into());
    }
    let (attached, images) = crate::attach::prepare(attachments)?;
    let question = format!("{question}{attached}").trim().to_owned();
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
    let mode = parse_mode(&mode)?;
    if ids.is_empty() {
        state
            .ai
            .broker
            .configure(mode, &ids, NO_LIMIT)
            .map_err(|error| error.to_string())?;
        begin(&app, &state, question, images, String::new(), mode)?;
        return Ok(SendResult { preview: None });
    }
    // 送る前の確認のあいだは、読み取りだけを許す。選んだ実行モードは確認されてから与える。
    state
        .ai
        .broker
        .configure(Mode::Consult, &ids, NO_LIMIT)
        .map_err(|error| error.to_string())?;
    let context = hashi_ai::observe::context_snapshot(&state.ai.broker);
    *state.ai.preview.lock().map_err(|_| "状態が不正です")? = Some(Pending {
        question,
        images,
        mode,
        ids,
    });
    Ok(SendResult {
        preview: Some(context),
    })
}

/// 確認した内容でAIへ送る。画面で編集した内容をそのまま使う。
/// 失敗しても確認待ちは消える（画面の確認窓は閉じているため、残すと以後の送信がすべて断られる）。
#[tauri::command]
pub async fn ai_confirm(
    app: AppHandle,
    state: State<'_, AppState>,
    context: String,
) -> Result<(), String> {
    let pending = state
        .ai
        .preview
        .lock()
        .map_err(|_| "状態が不正です")?
        .take()
        .ok_or("確認待ちの送信がありません")?;
    if context.len() > 512 * 1024 {
        state.ai.broker.stop();
        return Err("送る内容が大きすぎます".into());
    }
    state
        .ai
        .broker
        .configure(pending.mode, &pending.ids, NO_LIMIT)
        .map_err(|error| error.to_string())?;
    begin(
        &app,
        &state,
        pending.question,
        pending.images,
        context,
        pending.mode,
    )
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
    images: Vec<serde_json::Value>,
    context: String,
    mode: Mode,
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
    let forwarder = tauri::async_runtime::spawn(async move {
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
                .ask_with(
                    &question,
                    &context,
                    images,
                    &token,
                    delta_tx,
                    mode == Mode::Consult,
                )
                .await
        };
        // 途中の文字列をすべて画面へ渡してから終わりを知らせる（順番が逆だと本文が二重になる）。
        let _ = forwarder.await;
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
