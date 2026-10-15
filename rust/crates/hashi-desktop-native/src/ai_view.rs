//! AI相談（APIキー方式）。サービスを選び、キーを貼って「接続する」を押すだけで使えるようにする。
//! APIキーはAI専用の暗号化ストアへ保存し、SSH資格情報や設定JSONへ混ぜない。
//! 端末の共有対象と実行モードは会話欄の上で選び、操作は必ずBrokerを通す。
use super::*;
use crate::theme;
use hashi_ai::{api::ApiProvider, conversation::Conversation};
use hashi_core::broker::Mode;
use hashi_storage::secrets::{EncryptedSecrets, Namespace};
use serde_json::json;

enum Done {
    /// 保存済みのキー（空なら未保存）。
    Key(String),
    /// 接続確認まで済み、保存も終わった。
    Connected(Arc<ApiProvider>, Vec<String>),
    /// モデルを自動で決められなかった。入力欄を出す。
    NeedModel(String, Vec<String>),
    Forgotten,
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    User,
    Ai,
    Note,
    Error,
}
struct ChatMsg {
    role: Role,
    text: String,
}

/// 利用者が選ぶ接続先。種別とAPI形式は内部で決め、画面には出さない。
struct Service {
    id: &'static str,
    label: &'static str,
    kind: &'static str,
    base: &'static str,
    needs_key: bool,
    key_page: &'static str,
    blurb: &'static str,
}
const SERVICES: [Service; 4] = [
    Service {
        id: "openai",
        label: "OpenAI",
        kind: "openai",
        base: "https://api.openai.com/v1",
        needs_key: true,
        key_page: "https://platform.openai.com/api-keys",
        blurb: "ChatGPTのAPI。利用料はOpenAIの料金に従います。",
    },
    Service {
        id: "anthropic",
        label: "Claude",
        kind: "anthropic",
        base: "https://api.anthropic.com/v1",
        needs_key: true,
        key_page: "https://console.anthropic.com/settings/keys",
        blurb: "AnthropicのAPI。利用料はAnthropicの料金に従います。",
    },
    Service {
        id: "local",
        label: "自分のPC",
        kind: "compatible",
        base: "http://127.0.0.1:1234/v1",
        needs_key: false,
        key_page: "",
        blurb: "LM StudioやOllamaなど、このPCで動くAI。キーは要りません（Ollamaは http://127.0.0.1:11434/v1）。",
    },
    Service {
        id: "custom",
        label: "その他",
        kind: "compatible",
        base: "",
        needs_key: true,
        key_page: "",
        blurb: "OpenAI互換のAPIを出している他社サービス。接続先URLを入力してください。",
    },
];
const PROTOCOLS: [(&str, &str); 4] = [
    ("auto", "自動"),
    ("chat", "Chat Completions"),
    ("responses", "Responses"),
    ("messages", "Anthropic Messages"),
];

/// 公式でない接続先の種別。Command CodeのURLだけは専用の扱いにする。
fn kind_for(service: &Service, base: &str) -> &'static str {
    if service.kind == "compatible" {
        if let Ok(base) = hashi_ai::http::normalize_api_base(base) {
            if base == hashi_ai::api::default_base("commandcode") {
                return "commandcode";
            }
        }
    }
    service.kind
}

/// 一覧から最初に使うモデル。各社の主力に当たりやすい名前を優先し、なければ先頭。
fn default_model(kind: &str, models: &[String]) -> Option<String> {
    let preferred: &[&str] = match kind {
        "openai" => &["gpt-5", "gpt-4.1", "gpt-4o"],
        "anthropic" => &["claude-sonnet", "claude-opus", "claude"],
        _ => &[],
    };
    for prefix in preferred {
        if let Some(model) = models
            .iter()
            .filter(|m| m.starts_with(prefix) && !m.contains("mini") && !m.contains("nano"))
            .min_by_key(|m| m.len())
        {
            return Some(model.clone());
        }
    }
    models.first().cloned()
}

pub(super) struct AiWindow {
    service: usize,
    base: String,
    key: String,
    model: String,
    models: Vec<String>,
    protocol: usize,
    use_tools: bool,
    persist: bool,
    /// 設定画面を出す。未接続のときは常に出る。
    setup: bool,
    need_model: bool,
    status: String,
    failed: bool,
    task: Option<tokio::sync::oneshot::Receiver<Result<Done, String>>>,
    restoring: bool,
    provider: Option<Arc<ApiProvider>>,
    conversation: Option<Arc<tokio::sync::Mutex<Conversation>>>,
    chat: Vec<ChatMsg>,
    question: String,
    preview: Option<(String, String)>,
    running: Option<(
        CancellationToken,
        tokio::sync::oneshot::Receiver<Result<String, String>>,
    )>,
    streamed: bool,
    deltas: Option<mpsc::Receiver<String>>,
}
impl Default for AiWindow {
    fn default() -> Self {
        Self {
            service: 0,
            base: String::new(),
            key: String::new(),
            model: String::new(),
            models: vec![],
            protocol: 0,
            use_tools: true,
            persist: true,
            setup: true,
            need_model: false,
            status: String::new(),
            failed: false,
            task: None,
            restoring: false,
            provider: None,
            conversation: None,
            chat: vec![],
            question: String::new(),
            preview: None,
            running: None,
            streamed: false,
            deltas: None,
        }
    }
}

fn key_id(kind: &str, base: &str) -> Result<String, String> {
    use sha2::Digest;
    let base = hashi_ai::http::normalize_api_base(base).map_err(|e| e.to_string())?;
    let digest = sha2::Sha256::digest(base.as_bytes());
    Ok(format!(
        "api:{kind}:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}
fn secrets() -> Result<EncryptedSecrets, String> {
    EncryptedSecrets::new(Channel::Preview, Namespace::Ai).map_err(|e| e.to_string())
}

impl AiWindow {
    fn service(&self) -> &'static Service {
        &SERVICES[self.service]
    }
    /// 接続先のURL。公式サービスは固定、それ以外は入力値。
    fn effective_base(&self) -> String {
        match self.service().id {
            "local" | "custom" if !self.base.trim().is_empty() => self.base.trim().to_owned(),
            _ => self.service().base.to_owned(),
        }
    }
    fn say(&mut self, text: impl Into<String>, failed: bool) {
        self.status = text.into();
        self.failed = failed;
    }
}

impl App {
    /// 前回の接続設定を読み戻し、保存キーがあれば会話できる状態まで進める。
    pub(super) fn ai_restore(&mut self) {
        let saved = self
            .snapshot
            .settings
            .get("ai_connection")
            .cloned()
            .unwrap_or(json!(null));
        let Some(index) = SERVICES
            .iter()
            .position(|s| Some(s.id) == saved["service"].as_str())
        else {
            return;
        };
        let ai = &mut self.ai;
        ai.service = index;
        ai.base = saved["base"]
            .as_str()
            .unwrap_or(SERVICES[index].base)
            .to_owned();
        ai.model = saved["model"].as_str().unwrap_or_default().to_owned();
        ai.protocol = PROTOCOLS
            .iter()
            .position(|(id, _)| Some(*id) == saved["protocol"].as_str())
            .unwrap_or(0);
        ai.use_tools = saved["use_tools"].as_bool().unwrap_or(true);
        ai.restoring = !ai.model.is_empty();
        self.ai_load_key();
    }
    /// 選んだ接続先の保存キーを別タスクで読む。
    fn ai_load_key(&mut self) {
        self.ai.key.clear();
        let service = self.ai.service();
        match key_id(
            kind_for(service, &self.ai.effective_base()),
            &self.ai.effective_base(),
        ) {
            Ok(id) if service.needs_key => self.ai_start("", move || {
                let value = secrets()?.get(&id).map_err(|_| {
                    "保存したキーを読み込めませんでした。もう一度入力してください。".to_owned()
                })?;
                Ok(Done::Key(value.unwrap_or_default()))
            }),
            _ => {
                if self.ai.restoring {
                    self.ai_finish_restore();
                }
            }
        }
    }
    fn ai_start(
        &mut self,
        message: &str,
        job: impl FnOnce() -> Result<Done, String> + Send + 'static,
    ) {
        if self.ai.task.is_some() || self.exiting {
            return;
        }
        if !message.is_empty() {
            self.ai.say(message, false);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.handle().spawn(async move {
            let result = tokio::task::spawn_blocking(job)
                .await
                .unwrap_or_else(|_| Err("処理に失敗しました".into()));
            let _ = tx.send(result);
        });
        self.ai.task = Some(rx);
    }
    fn ai_build(&self, model: &str) -> Result<ApiProvider, String> {
        let ai = &self.ai;
        let service = ai.service();
        let base = ai.effective_base();
        let base = hashi_ai::http::normalize_api_base(&base).map_err(|e| e.to_string())?;
        let kind = kind_for(service, &base);
        if service.needs_key && ai.key.trim().is_empty() {
            return Err("APIキーを入力してください。".into());
        }
        let protocol = if matches!(kind, "compatible" | "commandcode") {
            PROTOCOLS[ai.protocol].0
        } else {
            "auto"
        };
        ApiProvider::new(
            kind,
            if model.is_empty() {
                "model-list"
            } else {
                model
            },
            ai.key.trim(),
            Some(&base),
            ai.use_tools,
            protocol,
        )
        .map_err(|e| e.to_string())
    }
    /// 「接続する」。モデル一覧の取得、応答の確認、設定とキーの保存を1回で行う。
    fn ai_connect(&mut self) {
        let typed = self.ai.model.trim().to_owned();
        let probe = match self.ai_build(&typed) {
            Ok(provider) => provider,
            Err(error) => return self.ai.say(error, true),
        };
        let key_id = match key_id(&probe.kind, &probe.base) {
            Ok(id) => id,
            Err(error) => return self.ai.say(error, true),
        };
        let persist = self.ai.persist && self.ai.service().needs_key;
        self.ai.need_model = false;
        self.ai_start("接続しています…", move || {
            let cancel = CancellationToken::new();
            let listed = probe.models(&cancel).map(|models| {
                models
                    .iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            });
            let models = listed.as_ref().cloned().unwrap_or_default();
            let model = if !typed.is_empty() {
                typed
            } else {
                match (&listed, default_model(&probe.kind, &models)) {
                    (Ok(_), Some(model)) => model,
                    (Ok(_), None) => {
                        return Ok(Done::NeedModel(
                            "使えるモデルが見つかりません。モデル名を入力してください。".into(),
                            models,
                        ))
                    }
                    (Err(error), _) => {
                        // キーや接続先の誤りは、モデル名の入力を求めず原因をそのまま返す。
                        let message = error.to_string();
                        return if message.contains("HTTP 401") || message.contains("HTTP 403") {
                            Err(message)
                        } else {
                            Ok(Done::NeedModel(
                                format!("モデル一覧を取得できませんでした（{message}）。モデル名を入力して、もう一度「接続する」を押してください。"),
                                models,
                            ))
                        };
                    }
                }
            };
            let provider = ApiProvider::new(
                &probe.kind,
                &model,
                probe.key(),
                Some(&probe.base),
                probe.use_tools,
                &probe.protocol,
            )
            .map_err(|e| e.to_string())?;
            // 接続確認ではtoolsを送らない。
            let mut check = provider.clone();
            check.use_tools = false;
            check
                .respond(
                    &[json!({"role": "user", "content": "OKとだけ答えてください。"})],
                    "接続確認です。端末操作をせず短く回答してください。",
                    &[],
                    &cancel,
                    &mut |_| {},
                )
                .map_err(|e| e.to_string())?;
            secrets()?
                .set(
                    &key_id,
                    persist
                        .then(|| provider.key().to_owned())
                        .as_deref()
                        .filter(|v| !v.is_empty()),
                )
                .map_err(|_| "APIキーを保存できませんでした。もう一度お試しください。".to_owned())?;
            Ok(Done::Connected(Arc::new(provider), models))
        });
    }
    fn ai_save_connection(&mut self, provider: &ApiProvider) -> bool {
        let saved = json!({
            "service": self.ai.service().id,
            "base": self.ai.base,
            "model": provider.model,
            "protocol": provider.protocol,
            "use_tools": provider.use_tools,
        });
        match self.store.transact(|snapshot| {
            snapshot.settings.insert("ai_connection".into(), saved);
            Ok(())
        }) {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                true
            }
            Err(_) => false,
        }
    }
    fn ai_finish_restore(&mut self) {
        self.ai.restoring = false;
        let model = self.ai.model.clone();
        if let Ok(provider) = self.ai_build(&model) {
            self.ai_use(Arc::new(provider));
            self.ai.setup = false;
        }
    }
    fn ai_use(&mut self, provider: Arc<ApiProvider>) {
        self.ai_stop();
        self.ai.model = provider.model.clone();
        self.ai.provider = Some(provider);
        self.ai_new_conversation();
    }
    fn ai_new_conversation(&mut self) {
        if let Some(provider) = &self.ai.provider {
            self.ai.conversation = Some(Arc::new(tokio::sync::Mutex::new(Conversation {
                provider: provider.clone(),
                tools: hashi_ai::tools::TerminalTools {
                    broker: self.external.broker.clone(),
                    actor: format!("ai:{}", uuid::Uuid::new_v4().simple()),
                    cancel: CancellationToken::new(),
                },
                messages: vec![],
            })));
        }
        self.ai.chat.clear();
    }
    pub(super) fn ai_stop(&mut self) {
        if let Some((cancel, _)) = &self.ai.running {
            cancel.cancel();
        }
        if let Some(conversation) = &self.ai.conversation {
            if let Ok(conversation) = conversation.try_lock() {
                conversation.tools.cancel.cancel();
            }
        }
    }
    fn ai_poll(&mut self) {
        if let Some(receiver) = &mut self.ai.task {
            match receiver.try_recv() {
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(_) => self.ai.task = None,
                Ok(result) => {
                    self.ai.task = None;
                    match result {
                        Err(error) => {
                            if self.ai.restoring {
                                self.ai.restoring = false;
                            }
                            self.ai.say(error, true);
                        }
                        Ok(Done::Key(key)) => {
                            self.ai.persist = true;
                            self.ai.key = key;
                            if self.ai.restoring {
                                self.ai_finish_restore();
                            }
                        }
                        Ok(Done::NeedModel(message, models)) => {
                            self.ai.need_model = true;
                            self.ai.models = models;
                            self.ai.say(message, true);
                        }
                        Ok(Done::Connected(provider, models)) => {
                            if self.ai_save_connection(&provider) {
                                self.ai.models = models;
                                self.ai_use(provider);
                                self.ai.setup = false;
                                self.ai.say("", false);
                            } else {
                                self.ai.say("接続設定を保存できませんでした。設定フォルダの書込み権限を確認してください。", true);
                            }
                        }
                        Ok(Done::Forgotten) => {
                            self.ai.key.clear();
                            self.ai.say("保存したキーを削除しました。", false);
                        }
                    }
                }
            }
        }
        if let Some(deltas) = &mut self.ai.deltas {
            while let Ok(text) = deltas.try_recv() {
                self.ai.streamed = true;
                if let Some(last) = self.ai.chat.last_mut().filter(|m| m.role == Role::Ai) {
                    last.text.push_str(&text);
                }
            }
        }
        if let Some((_, receiver)) = &mut self.ai.running {
            match receiver.try_recv() {
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                result => {
                    if let Some(deltas) = &mut self.ai.deltas {
                        while let Ok(text) = deltas.try_recv() {
                            self.ai.streamed = true;
                            if let Some(last) =
                                self.ai.chat.last_mut().filter(|m| m.role == Role::Ai)
                            {
                                last.text.push_str(&text);
                            }
                        }
                    }
                    let streamed = self.ai.streamed;
                    let empty_reply = self
                        .ai
                        .chat
                        .last()
                        .is_some_and(|m| m.role == Role::Ai && m.text.is_empty());
                    match result {
                        Ok(Ok(text)) if !streamed => {
                            if let Some(last) = self.ai.chat.last_mut() {
                                last.text = text;
                            }
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            if empty_reply {
                                self.ai.chat.pop();
                            }
                            self.ai.chat.push(ChatMsg {
                                role: Role::Error,
                                text: error,
                            });
                        }
                        Err(_) => {
                            if empty_reply {
                                self.ai.chat.pop();
                            }
                            self.ai.chat.push(ChatMsg {
                                role: Role::Error,
                                text: "AI処理が中断されました".into(),
                            });
                        }
                    }
                    self.ai.running = None;
                    self.ai.deltas = None;
                }
            }
        }
    }
    /// 共有対象があれば選択どおりに許可を設定し、送信内容を確認画面へ出す。
    fn ai_send(&mut self) {
        let question = self.ai.question.trim().to_owned();
        if question.is_empty() || self.ai.running.is_some() || self.ai.preview.is_some() {
            return;
        }
        if self.ai.conversation.is_none() {
            self.ai.setup = true;
            return;
        }
        let targets: Vec<_> = self
            .tabs
            .iter()
            .filter(|tab| self.external.selected.contains(&tab.stamp) && !tab.closing)
            .filter_map(|tab| tab.binding.id.clone())
            .collect();
        if let Err(error) = self.external.broker.configure(
            self.external.mode,
            &targets,
            std::time::Duration::from_secs(15 * 60),
        ) {
            self.ai.chat.push(ChatMsg {
                role: Role::Error,
                text: error.to_string(),
            });
            return;
        }
        if targets.is_empty() {
            self.ai_begin(question, String::new());
        } else {
            let context = hashi_ai::observe::context_snapshot(&self.external.broker);
            self.ai.preview = Some((question, context));
        }
    }
    fn ai_begin(&mut self, question: String, context: String) {
        let Some(conversation) = self.ai.conversation.clone() else {
            return;
        };
        self.ai.question.clear();
        self.ai.chat.push(ChatMsg {
            role: Role::User,
            text: question.clone(),
        });
        self.ai.chat.push(ChatMsg {
            role: Role::Ai,
            text: String::new(),
        });
        self.ai.streamed = false;
        let cancel = CancellationToken::new();
        let (delta_tx, deltas) = mpsc::channel(256);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let token = cancel.clone();
        self.runtime.handle().spawn(async move {
            let mut conversation = conversation.lock().await;
            // 停止で取り消した前回のtokenを使い回さない。
            conversation.tools.cancel = token.child_token();
            let result = conversation
                .ask(&question, &context, &token, delta_tx)
                .await;
            let _ = tx.send(result.map_err(|error| error.to_string()));
        });
        self.ai.running = Some((cancel, rx));
        self.ai.deltas = Some(deltas);
    }
    /// 一覧にあるモデルへ切り替える。通信はせず、会話は新しくする。
    fn ai_switch_model(&mut self, model: String) {
        let Ok(provider) = self.ai_build(&model) else {
            return;
        };
        if self.ai_save_connection(&provider) {
            self.ai_use(Arc::new(provider));
            self.ai.chat.push(ChatMsg {
                role: Role::Note,
                text: format!("モデルを {model} に切り替えました。会話は新しくなっています。"),
            });
        }
    }

    pub(super) fn ai_tick(&mut self, ctx: &egui::Context) {
        self.ai_poll();
        self.ai_preview(ctx);
    }

    /// 右側のAI欄。
    pub(super) fn ai_dock(&mut self, ui: &mut egui::Ui) {
        if self.ai.provider.is_none() || self.ai.setup {
            self.ai_setup(ui);
        } else {
            self.ai_chat(ui);
        }
    }

    fn ai_setup(&mut self, ui: &mut egui::Ui) {
        let busy = self.ai.task.is_some();
        let connected = self.ai.provider.is_some();
        let mut connect = false;
        let mut back = false;
        let mut forget = false;
        let mut service_changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(theme::title("AIをつなぐ"));
                ui.label(theme::muted(
                    "サービスを選び、APIキーを貼り付けるだけです。キーはこのPCの専用の暗号化ファイルにだけ保存します。",
                ));
                ui.add_space(6.0);
                ui.add_enabled_ui(!busy, |ui| {
                    ui.label(egui::RichText::new("1  サービス").strong());
                    let before = self.ai.service;
                    let items: Vec<(usize, &str)> = SERVICES
                        .iter()
                        .enumerate()
                        .map(|(index, service)| (index, service.label))
                        .collect();
                    theme::segmented(ui, &mut self.ai.service, &items);
                    service_changed = before != self.ai.service;
                    ui.label(theme::muted(self.ai.service().blurb));
                    let service = self.ai.service();
                    if matches!(service.id, "local" | "custom") {
                        ui.add_space(4.0);
                        ui.label(theme::muted("接続先URL"));
                        let edited = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.ai.base)
                                    .hint_text(if service.id == "local" {
                                        service.base
                                    } else {
                                        "https://example.com/v1"
                                    })
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                        // 入力済みのキーを、別の接続先へ送らない。
                        if edited {
                            self.ai.key.clear();
                            self.ai.need_model = false;
                        }
                    }
                    if service.needs_key {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("2  APIキー").strong());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ai.key)
                                .password(true)
                                .hint_text("ここに貼り付け")
                                .desired_width(f32::INFINITY),
                        );
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.ai.persist, "このPCに保存する");
                            if !service.key_page.is_empty()
                                && ui.link("キーの取得ページを開く").clicked()
                            {
                                let _ = webbrowser::open(service.key_page);
                            }
                        });
                    }
                    if self.ai.need_model {
                        ui.add_space(4.0);
                        ui.label(theme::muted("モデル名"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.ai.model)
                                    .hint_text("提供元のモデル名")
                                    .desired_width(240.0),
                            );
                            if !self.ai.models.is_empty() {
                                egui::ComboBox::from_id_salt("ai-setup-models")
                                    .selected_text("一覧から選ぶ")
                                    .show_ui(ui, |ui| {
                                        for model in &self.ai.models {
                                            ui.selectable_value(
                                                &mut self.ai.model,
                                                model.clone(),
                                                model,
                                            );
                                        }
                                    });
                            }
                        });
                    }
                    if matches!(service.id, "local" | "custom") {
                        egui::CollapsingHeader::new("詳細設定").show(ui, |ui| {
                            ui.label(theme::muted("通信の形式"));
                            egui::ComboBox::from_id_salt("ai-protocol")
                                .selected_text(PROTOCOLS[self.ai.protocol].1)
                                .show_ui(ui, |ui| {
                                    for (index, (_, label)) in PROTOCOLS.iter().enumerate() {
                                        ui.selectable_value(&mut self.ai.protocol, index, *label);
                                    }
                                });
                            ui.checkbox(
                                &mut self.ai.use_tools,
                                "このAIは関数呼び出し（ツール）に対応している",
                            );
                            ui.label(theme::muted(
                                "対応していないAIでは外してください。端末の閲覧だけになります。",
                            ));
                        });
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    connect = ui
                        .add_enabled(!busy, theme::primary("接続する"))
                        .clicked();
                    if busy {
                        ui.spinner();
                    }
                    if connected && ui.add_enabled(!busy, egui::Button::new("会話へ戻る")).clicked() {
                        back = true;
                    }
                });
                if !self.ai.status.is_empty() {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.ai.status).color(
                            if self.ai.failed { theme::DANGER } else { theme::MUTED },
                        ))
                        .wrap(),
                    );
                }
                ui.add_space(8.0);
                ui.label(theme::muted(
                    "接続時に短い確認の質問を1回送ります。利用料は提供元の料金に従います。",
                ));
                if self.ai.service().needs_key
                    && ui
                        .add_enabled(!busy, theme::danger("このサービスの保存キーを削除"))
                        .clicked()
                {
                    forget = true;
                }
            });
        if service_changed {
            self.ai.need_model = false;
            self.ai.model.clear();
            self.ai.base = self.ai.service().base.to_owned();
            self.ai.say("", false);
            self.ai_load_key();
        }
        if connect {
            self.ai_connect();
        }
        if back {
            self.ai.setup = false;
        }
        if forget {
            let service = self.ai.service();
            let base = self.ai.effective_base();
            match key_id(kind_for(service, &base), &base) {
                Ok(id) => {
                    self.ai_start("保存したキーを削除しています…", move || {
                        secrets()?
                            .set(&id, None)
                            .map_err(|_| "保存したキーを削除できませんでした。".to_owned())?;
                        Ok(Done::Forgotten)
                    })
                }
                Err(error) => self.ai.say(error, true),
            }
        }
    }

    fn ai_chat(&mut self, ui: &mut egui::Ui) {
        let running = self.ai.running.is_some();
        let terminals: Vec<(Stamp, String)> = self
            .tabs
            .iter()
            .filter(|tab| tab.binding.id.is_some() && !tab.closing)
            .map(|tab| (tab.stamp.clone(), tab.label.clone()))
            .collect();
        let mut send = false;
        let mut stop = false;
        let mut clear = false;
        let mut setup = false;
        let mut switch = None;
        let provider = self.ai.provider.clone();
        ui.horizontal(|ui| {
            if let Some(provider) = &provider {
                theme::dot(ui, theme::OK);
                let current = provider.model.clone();
                egui::ComboBox::from_id_salt("ai-model")
                    .selected_text(current.clone())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        let mut models = self.ai.models.clone();
                        if !models.contains(&current) {
                            models.insert(0, current.clone());
                        }
                        for model in models {
                            if ui.selectable_label(model == current, &model).clicked()
                                && model != current
                            {
                                switch = Some(model);
                            }
                        }
                    });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                setup = ui.button("接続設定").clicked();
                clear = ui
                    .add_enabled(!running, egui::Button::new("会話を消去"))
                    .clicked();
            });
        });
        self.share_strip(ui, &terminals);
        ui.add_space(2.0);
        egui::Panel::bottom("ai-input")
            .frame(egui::Frame::new())
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                let editor = ui.add(
                    egui::TextEdit::multiline(&mut self.ai.question)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("質問や、端末への依頼を入力（Ctrl+Enterで送信）"),
                );
                let shortcut = editor.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                ui.horizontal(|ui| {
                    if running {
                        stop = ui.add(theme::danger("停止")).clicked();
                        ui.spinner();
                    } else {
                        send = ui
                            .add_enabled(
                                !self.ai.question.trim().is_empty(),
                                theme::primary("送信"),
                            )
                            .clicked()
                            || (shortcut && !self.ai.question.trim().is_empty());
                    }
                });
            });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.ai.chat.is_empty() {
                    ui.add_space(12.0);
                    ui.label(theme::muted(
                        "何でも質問できます。上で端末を選ぶと、その画面を見せて相談できます。",
                    ));
                    for example in [
                        "この画面のエラーの原因を教えて",
                        "いまの出力を短く要約して",
                        "次に打つコマンドを提案して",
                    ] {
                        if ui.button(example).clicked() {
                            self.ai.question = example.into();
                        }
                    }
                }
                for message in &self.ai.chat {
                    bubble(ui, message);
                }
            });
        if setup {
            self.ai.setup = true;
        }
        if clear {
            self.ai_new_conversation();
        }
        if stop {
            self.ai_stop();
            self.external.broker.stop();
        }
        if send {
            self.ai_send();
        }
        if let Some(model) = switch {
            self.ai_switch_model(model);
        }
    }

    /// 会話の上の共有欄。端末を選ぶと、その画面だけをAIへ見せる。
    fn share_strip(&mut self, ui: &mut egui::Ui, terminals: &[(Stamp, String)]) {
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::muted("AIに見せる端末"));
                if terminals.is_empty() {
                    ui.label(theme::muted("（なし）端末を開くと選べます"));
                }
                for (stamp, label) in terminals {
                    let mut checked = self.external.selected.contains(stamp);
                    if ui.toggle_value(&mut checked, label).changed() {
                        self.external.selected.retain(|s| s != stamp);
                        if checked {
                            self.external.selected.push(stamp.clone());
                        }
                    }
                }
            });
            let shared = terminals
                .iter()
                .any(|(stamp, _)| self.external.selected.contains(stamp));
            ui.add_enabled_ui(shared, |ui| {
                theme::segmented(
                    ui,
                    &mut self.external.mode,
                    &[
                        (Mode::Consult, "読むだけ"),
                        (Mode::Confirm, "操作は確認"),
                        (Mode::Auto, "自動で操作"),
                    ],
                );
            });
            let (text, color) = if !shared {
                (
                    "端末は見せていません。質問だけがAIに届きます。",
                    theme::MUTED,
                )
            } else {
                match self.external.mode {
                    Mode::Consult => (
                        "画面を読むだけで、AIは操作しません。送る前に内容を確認できます。",
                        theme::MUTED,
                    ),
                    Mode::Confirm => ("AIが操作したいとき、そのつど許可を求めます。", theme::MUTED),
                    Mode::Auto => (
                        "選んだ端末で、確認なしにAIが実行します（15分間）。",
                        theme::WARN,
                    ),
                }
            };
            ui.label(egui::RichText::new(text).size(12.5).color(color));
        });
    }

    fn ai_preview(&mut self, ctx: &egui::Context) {
        let Some((question, context)) = &mut self.ai.preview else {
            return;
        };
        let mut decision = None;
        egui::Window::new("AIへ送る内容の確認")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([620.0, 480.0])
            .show(ctx, |ui| {
                ui.label(theme::muted(
                    "端末の画面がそのままAIに送られます。秘密情報が残っていないか確認し、必要なら直してください。",
                ));
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(context)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.add(theme::primary("この内容を送る")).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("戻る").clicked() {
                        decision = Some(false);
                    }
                });
            });
        let question = question.clone();
        match decision {
            Some(true) => {
                let (_, context) = self.ai.preview.take().unwrap();
                self.ai_begin(question, context);
            }
            Some(false) => {
                self.ai.preview = None;
                self.external.broker.stop();
            }
            None => {}
        }
    }
}

/// 会話の1件。``` で囲まれた部分はコードとして等幅で出す。
fn bubble(ui: &mut egui::Ui, message: &ChatMsg) {
    let width = ui.available_width();
    match message.role {
        Role::User => {
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                egui::Frame::new()
                    .fill(theme::ACCENT.gamma_multiply(0.2))
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(width * 0.85);
                        ui.add(egui::Label::new(&message.text).selectable(true).wrap());
                    });
            });
        }
        Role::Ai => {
            theme::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                if message.text.is_empty() {
                    ui.label(theme::muted("考えています…"));
                    return;
                }
                for (index, part) in message.text.split("```").enumerate() {
                    if index % 2 == 1 {
                        // 先頭行は言語名なので除く。
                        let code = part.split_once('\n').map_or(part, |(_, rest)| rest);
                        egui::Frame::new()
                            .fill(theme::BG)
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(code.trim_end()).monospace(),
                                    )
                                    .selectable(true)
                                    .wrap(),
                                );
                            });
                    } else if !part.trim().is_empty() {
                        ui.add(egui::Label::new(part.trim()).selectable(true).wrap());
                    }
                }
            });
        }
        Role::Note => {
            ui.label(theme::muted(&message.text));
        }
        Role::Error => {
            ui.add(
                egui::Label::new(egui::RichText::new(&message.text).color(theme::DANGER)).wrap(),
            );
        }
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_model_prefers_main_models_over_small_variants() {
        let models = names(&["gpt-4o-mini", "gpt-5-mini", "gpt-5", "gpt-5-2025", "o3"]);
        assert_eq!(default_model("openai", &models).as_deref(), Some("gpt-5"));
        let claude = names(&["claude-haiku-4", "claude-sonnet-4-5", "claude-opus-4"]);
        assert_eq!(
            default_model("anthropic", &claude).as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            default_model("compatible", &names(&["local-model", "b"])).as_deref(),
            Some("local-model")
        );
        assert_eq!(default_model("openai", &[]), None);
    }

    #[test]
    fn only_command_code_url_changes_the_kind_of_a_custom_service() {
        let custom = &SERVICES[3];
        assert_eq!(kind_for(custom, "https://example.com/v1"), "compatible");
        assert_eq!(
            kind_for(custom, "https://api.commandcode.ai/v1"),
            "commandcode"
        );
        assert_eq!(
            kind_for(&SERVICES[0], "https://api.commandcode.ai/v1"),
            "openai"
        );
        assert_eq!(kind_for(custom, "not a url"), "compatible");
    }

    #[test]
    fn official_services_ignore_a_typed_base_url() {
        let mut ai = AiWindow {
            base: "https://evil.example.com/v1".into(),
            ..AiWindow::default()
        };
        assert_eq!(ai.effective_base(), "https://api.openai.com/v1");
        ai.service = 3;
        assert_eq!(ai.effective_base(), "https://evil.example.com/v1");
    }
}
