//! AI相談（APIキー方式）。接続設定・会話・送信前の観測データ確認を行う。
//! APIキーはAI専用の暗号化ストアへ保存し、SSH資格情報や設定JSONへ混ぜない。
//! 端末の共有対象と実行モードは「外部操作」画面の選択を使い、操作は必ずBrokerを通す。
use super::*;
use hashi_ai::{api::ApiProvider, conversation::Conversation};
use hashi_storage::secrets::{EncryptedSecrets, Namespace};
use serde_json::{json, Value};

enum Done {
    Key(String),
    Models(Vec<String>),
    Checked,
    Saved(Arc<ApiProvider>),
    Forgotten,
}

pub(super) struct AiWindow {
    pub(super) open: bool,
    kind: usize,
    base: String,
    key: String,
    model: String,
    models: Vec<String>,
    protocol: usize,
    use_tools: bool,
    persist: bool,
    status: String,
    task: Option<tokio::sync::oneshot::Receiver<Result<Done, String>>>,
    provider: Option<Arc<ApiProvider>>,
    conversation: Option<Arc<tokio::sync::Mutex<Conversation>>>,
    transcript: String,
    question: String,
    preview: Option<(String, String)>,
    running: Option<(
        CancellationToken,
        tokio::sync::oneshot::Receiver<Result<String, String>>,
    )>,
    streamed: bool,
    deltas: Option<mpsc::Receiver<String>>,
    loaded: bool,
}
impl Default for AiWindow {
    fn default() -> Self {
        Self {
            open: false,
            kind: 0,
            base: String::new(),
            key: String::new(),
            model: String::new(),
            models: vec![],
            protocol: 0,
            use_tools: true,
            persist: false,
            status: "接続先を選び、キーを入力してモデルを選んでください。".into(),
            task: None,
            provider: None,
            conversation: None,
            transcript: String::new(),
            question: String::new(),
            preview: None,
            running: None,
            streamed: false,
            deltas: None,
            loaded: false,
        }
    }
}
const PROTOCOLS: [(&str, &str); 4] = [
    ("auto", "自動選択"),
    ("chat", "Chat Completions"),
    ("responses", "Responses"),
    ("messages", "Anthropic Messages"),
];
fn kind_id(index: usize) -> &'static str {
    hashi_ai::api::KINDS[index].0
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

impl App {
    fn ai_profile(&self, kind: &str) -> Value {
        self.snapshot.settings["ai_api_profiles"][kind].clone()
    }
    /// 接続先の保存値を読み込み、保存キーを別タスクで読む。
    fn ai_load(&mut self) {
        let kind = kind_id(self.ai.kind);
        let profile = self.ai_profile(kind);
        let ai = &mut self.ai;
        ai.base = profile["base"]
            .as_str()
            .unwrap_or(hashi_ai::api::default_base(kind))
            .to_owned();
        if kind != "compatible" {
            ai.base = hashi_ai::api::default_base(kind).to_owned();
        }
        ai.model = profile["model"].as_str().unwrap_or_default().to_owned();
        ai.models.clear();
        ai.protocol = PROTOCOLS
            .iter()
            .position(|(id, _)| Some(*id) == profile["protocol"].as_str())
            .unwrap_or(0);
        ai.use_tools = profile["use_tools"].as_bool().unwrap_or(true);
        ai.key.clear();
        ai.persist = false;
        match key_id(kind, &ai.base) {
            Ok(id) => self.ai_start(
                "このPCの保存キーを読み込んでいます…",
                move || {
                    let value = secrets()?.get(&id).map_err(|_| {
                        "保存キーを読み込めませんでした。キーを再入力してください。".to_owned()
                    })?;
                    Ok(Done::Key(value.unwrap_or_default()))
                },
            ),
            Err(error) => self.ai.status = error,
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
        self.ai.status = message.into();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.handle().spawn(async move {
            let result = tokio::task::spawn_blocking(job)
                .await
                .unwrap_or_else(|_| Err("処理に失敗しました".into()));
            let _ = tx.send(result);
        });
        self.ai.task = Some(rx);
    }
    fn ai_provider(&self, require_model: bool) -> Result<ApiProvider, String> {
        let ai = &self.ai;
        let kind = kind_id(ai.kind);
        let base = hashi_ai::http::normalize_api_base(&ai.base).map_err(|e| e.to_string())?;
        if ai.key.trim().is_empty() && !(kind == "compatible" && base.starts_with("http://")) {
            return Err("APIキーを入力してください。".into());
        }
        let model = ai.model.trim();
        if require_model && model.is_empty() {
            return Err("モデル一覧から選ぶか、モデル名を入力してください。".into());
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
        self.ai.transcript.clear();
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
                        Err(error) => self.ai.status = error,
                        Ok(Done::Key(key)) => {
                            if self.ai.key.is_empty() {
                                self.ai.persist = !key.is_empty();
                                self.ai.key = key;
                            }
                            self.ai.status =
                                "モデル一覧を取得するか、利用可能なモデル名を入力してください。"
                                    .into();
                        }
                        Ok(Done::Models(models)) => {
                            self.ai.status = if models.is_empty() {
                                "一覧にモデルがありません。提供元のモデル名を手動で入力してください。".into()
                            } else {
                                format!(
                                    "{}個のモデルを取得しました。モデルを選んでください。",
                                    models.len()
                                )
                            };
                            self.ai.models = models;
                        }
                        Ok(Done::Checked) => {
                            self.ai.status =
                                "接続できました。「この接続を使う」で会話へ戻れます。".into()
                        }
                        Ok(Done::Forgotten) => {
                            self.ai.key.clear();
                            self.ai.persist = false;
                            self.ai.status = "この接続先の保存キーを削除しました。".into();
                        }
                        Ok(Done::Saved(provider)) => {
                            let kind = provider.kind.clone();
                            let profile = json!({"base": provider.base, "model": provider.model,
                                "protocol": provider.protocol, "use_tools": provider.use_tools});
                            match self.store.transact(|snapshot| {
                                let mut profiles = snapshot.settings["ai_api_profiles"].clone();
                                if !profiles.is_object() {
                                    profiles = json!({});
                                }
                                profiles[&kind] = profile;
                                snapshot.settings.insert("ai_api_profiles".into(), profiles);
                                snapshot.settings.insert("ai_api_kind".into(), json!(kind));
                                Ok(())
                            }) {
                                Ok(snapshot) => {
                                    self.snapshot = snapshot;
                                    self.ai_stop();
                                    self.ai.provider = Some(provider);
                                    self.ai_new_conversation();
                                    self.ai.status = "この接続で会話できます。".into();
                                }
                                Err(_) => {
                                    self.ai.status = "接続設定を保存できませんでした。設定フォルダの書込み権限を確認してください。".into()
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(deltas) = &mut self.ai.deltas {
            while let Ok(text) = deltas.try_recv() {
                self.ai.streamed = true;
                self.ai.transcript.push_str(&text);
            }
        }
        if let Some((_, receiver)) = &mut self.ai.running {
            match receiver.try_recv() {
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                result => {
                    if let Some(deltas) = &mut self.ai.deltas {
                        while let Ok(text) = deltas.try_recv() {
                            self.ai.transcript.push_str(&text);
                            self.ai.streamed = true;
                        }
                    }
                    match result {
                        Ok(Ok(text)) if !self.ai.streamed => self.ai.transcript.push_str(&text),
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => self.ai.transcript.push_str(&format!("\n{error}")),
                        Err(_) => self.ai.transcript.push_str("\nAI処理が中断されました"),
                    }
                    self.ai.transcript.push('\n');
                    self.ai.running = None;
                    self.ai.deltas = None;
                }
            }
        }
    }
    /// 共有対象があれば「外部操作」の選択で許可を設定し、送信内容を確認画面へ出す。
    fn ai_send(&mut self) {
        let question = self.ai.question.trim().to_owned();
        if question.is_empty() || self.ai.running.is_some() || self.ai.preview.is_some() {
            return;
        }
        if self.ai.conversation.is_none() {
            self.ai
                .transcript
                .push_str("AI接続設定でプロバイダーを選んでください\n");
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
            self.ai.transcript.push_str(&format!("{error}\n"));
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
        self.ai
            .transcript
            .push_str(&format!("あなた: {question}\nAI: "));
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

    pub(super) fn ai_dialog(&mut self, ctx: &egui::Context) {
        self.ai_poll();
        self.ai_preview(ctx);
        if !self.ai.open {
            return;
        }
        if !self.ai.loaded {
            self.ai.loaded = true;
            let kind = self.snapshot.settings["ai_api_kind"]
                .as_str()
                .unwrap_or("openai")
                .to_owned();
            self.ai.kind = hashi_ai::api::KINDS
                .iter()
                .position(|(k, _)| *k == kind)
                .unwrap_or(0);
            self.ai_load();
        }
        let mut open = true;
        let mut action = None;
        let busy = self.ai.task.is_some();
        let running = self.ai.running.is_some();
        let connected = self
            .ai
            .provider
            .as_ref()
            .map(|p| format!("{} · {}", p.kind, p.model));
        let ai = &mut self.ai;
        egui::Window::new("AI相談")
            .default_size([620.0, 640.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(connected.as_deref().unwrap_or("AIは未接続です"));
                ui.label("共有する端末と実行モードは「外部操作」で選びます。未選択なら端末を見せません。");
                egui::CollapsingHeader::new("接続設定（APIキー方式）")
                    .default_open(connected.is_none())
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!busy && !running, |ui| {
                            let before = ai.kind;
                            egui::ComboBox::from_label("接続先")
                                .selected_text(hashi_ai::api::KINDS[ai.kind].1)
                                .show_ui(ui, |ui| {
                                    for (index, (_, label)) in hashi_ai::api::KINDS.iter().enumerate() {
                                        ui.selectable_value(&mut ai.kind, index, *label);
                                    }
                                });
                            if before != ai.kind {
                                action = Some("load");
                            }
                            let compatible = kind_id(ai.kind) == "compatible";
                            ui.label("APIのURL");
                            if ui
                                .add_enabled(compatible, egui::TextEdit::singleline(&mut ai.base))
                                .changed()
                            {
                                ai.key.clear();
                                ai.persist = false;
                            }
                            ui.label("APIキー");
                            ui.add(egui::TextEdit::singleline(&mut ai.key).password(true)
                                .hint_text("この接続先のAPIキーを貼り付け"));
                            ui.label("モデル");
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut ai.model);
                                egui::ComboBox::from_id_salt("ai-models")
                                    .selected_text("一覧")
                                    .show_ui(ui, |ui| {
                                        for model in &ai.models {
                                            ui.selectable_value(&mut ai.model, model.clone(), model);
                                        }
                                    });
                            });
                            ui.add_enabled_ui(matches!(kind_id(ai.kind), "compatible" | "commandcode"), |ui| {
                                egui::ComboBox::from_label("API形式")
                                    .selected_text(PROTOCOLS[ai.protocol].1)
                                    .show_ui(ui, |ui| {
                                        for (index, (_, label)) in PROTOCOLS.iter().enumerate() {
                                            ui.selectable_value(&mut ai.protocol, index, *label);
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                if ui.button("モデル一覧を取得").clicked() {
                                    action = Some("models");
                                }
                                if ui.button("接続を確認").clicked() {
                                    action = Some("check");
                                }
                            });
                            ui.checkbox(&mut ai.use_tools, "AIから端末ツールを使う");
                            ui.checkbox(&mut ai.persist, "APIキーをこのPCに保存する");
                            ui.horizontal(|ui| {
                                if ui.button("この接続を使う").clicked() {
                                    action = Some("save");
                                }
                                if ui.button("この接続先の保存キーを削除").clicked() {
                                    action = Some("forget");
                                }
                            });
                        });
                        ui.label(&ai.status);
                        ui.label("接続確認は短い回答を取得します。API利用料は提供元の料金に従います。");
                    });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(&ai.transcript).selectable(true).wrap());
                    });
                ui.add(egui::TextEdit::multiline(&mut ai.question)
                    .desired_rows(3)
                    .hint_text("質問や端末への操作依頼を入力"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!running && connected.is_some(), egui::Button::new("送信する")).clicked() {
                        action = Some("send");
                    }
                    if ui.add_enabled(running, egui::Button::new("停止")).clicked() {
                        action = Some("stop");
                    }
                    if ui.add_enabled(!running, egui::Button::new("会話を消去")).clicked() {
                        action = Some("clear");
                    }
                });
            });
        if !open {
            self.ai.open = false;
        }
        match action {
            Some("load") => self.ai_load(),
            Some("send") => self.ai_send(),
            Some("stop") => {
                self.ai_stop();
                self.external.broker.stop();
            }
            Some("clear") => self.ai_new_conversation(),
            Some("models") => match self.ai_provider(false) {
                Ok(provider) => {
                    self.ai_start("モデル一覧を取得しています…", move || {
                        let models = provider
                            .models(&CancellationToken::new())
                            .map_err(|e| e.to_string())?;
                        Ok(Done::Models(
                            models
                                .iter()
                                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                                .collect(),
                        ))
                    })
                }
                Err(error) => self.ai.status = error,
            },
            Some("check") => {
                match self.ai_provider(true) {
                    Ok(mut provider) => self.ai_start(
                        "モデルからの応答を確認しています…",
                        move || {
                            // 接続確認ではtoolsを送らない。
                            provider.use_tools = false;
                            provider
                        .respond(&[json!({"role": "user", "content": "OKとだけ答えてください。"})],
                            "接続確認です。端末操作をせず短く回答してください。", &[],
                            &CancellationToken::new(), &mut |_| {})
                        .map_err(|e| e.to_string())?;
                            Ok(Done::Checked)
                        },
                    ),
                    Err(error) => self.ai.status = error,
                }
            }
            Some("forget") => match key_id(kind_id(self.ai.kind), &self.ai.base) {
                Ok(id) => self.ai_start("保存キーを削除しています…", move || {
                    secrets()?
                        .set(&id, None)
                        .map_err(|_| "保存キーを削除できませんでした。".to_owned())?;
                    Ok(Done::Forgotten)
                }),
                Err(error) => self.ai.status = error,
            },
            Some("save") => match (
                self.ai_provider(true),
                key_id(kind_id(self.ai.kind), &self.ai.base),
            ) {
                (Ok(provider), Ok(id)) => {
                    let persist = self.ai.persist;
                    self.ai_start("接続設定を保存しています…", move || {
                        let store = secrets()?;
                        let value = persist.then(|| provider.key().to_owned());
                        store
                            .set(&id, value.as_deref().filter(|v| !v.is_empty()))
                            .map_err(|_| {
                                "APIキーの保存・削除に失敗しました。再試行してください。".to_owned()
                            })?;
                        Ok(Done::Saved(Arc::new(provider)))
                    })
                }
                (Err(error), _) | (_, Err(error)) => self.ai.status = error,
            },
            _ => {}
        }
    }
    fn ai_preview(&mut self, ctx: &egui::Context) {
        let Some((question, context)) = &mut self.ai.preview else {
            return;
        };
        let mut decision = None;
        egui::Window::new("AIへ送信する内容")
            .collapsible(false)
            .default_size([620.0, 480.0])
            .show(ctx, |ui| {
                ui.label("内容は編集できます。秘密情報が残っていないか確認してください。");
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(context)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
                ui.horizontal(|ui| {
                    if ui.button("この内容を送信する").clicked() {
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
