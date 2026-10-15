//! 外部操作（AI/MCP）の共有許可・承認とMCP連携。端末操作は必ずBrokerを通す。
//! 許可はメモリ内だけで15分。停止・対象変更で進行中の要求も取り消す。
use super::*;
use hashi_core::broker::{ApprovalRequest, Broker, Kind, Mode, Registration, Registry};
use std::time::Duration;

const SHARE_LIFETIME: Duration = Duration::from_secs(15 * 60);

pub(super) struct External {
    pub(super) open: bool,
    pub(super) registry: Arc<Registry>,
    pub(super) broker: Arc<Broker>,
    approvals: mpsc::Receiver<ApprovalRequest>,
    pending: VecDeque<ApprovalRequest>,
    mode: Mode,
    selected: Vec<Stamp>,
    mcp: Option<Arc<hashi_mcp::McpBridge>>,
    mcp_starting: Option<tokio::sync::oneshot::Receiver<Result<Arc<hashi_mcp::McpBridge>, String>>>,
    mcp_config: String,
}
impl Default for External {
    fn default() -> Self {
        let registry = Arc::new(Registry::default());
        let (approval, approvals) = mpsc::channel(16);
        Self {
            open: false,
            broker: Arc::new(Broker::new(registry.clone(), Some(approval))),
            registry,
            approvals,
            pending: VecDeque::new(),
            mode: Mode::Consult,
            selected: vec![],
            mcp: None,
            mcp_starting: None,
            mcp_config: String::new(),
        }
    }
}

/// 端末台帳への登録。受信出力は1frameにまとめて台帳へ写す。
#[derive(Default)]
pub(super) struct Binding {
    pub(super) id: Option<(String, u64)>,
    capture: hashi_core::broker::OutputCapture,
    pending: String,
    dirty: bool,
}
impl Binding {
    pub(super) fn capture(&mut self, bytes: &[u8]) {
        if self.id.is_some() {
            let text = self.capture.push(bytes);
            self.pending.push_str(&text);
            self.dirty = true;
        }
    }
}

impl App {
    pub(super) fn bind_terminal(&mut self, index: usize, kind: Kind) {
        let tab = &mut self.tabs[index];
        let Some(input) = tab.input.clone() else {
            return;
        };
        let previous = tab.binding.id.as_ref().map(|(id, _)| id.clone());
        let id = self.external.registry.register(
            previous,
            Registration {
                label: tab.label.clone(),
                kind,
                shell: match kind {
                    Kind::Ssh => "unknown".into(),
                    Kind::Local if cfg!(windows) => "cmd".into(),
                    Kind::Local => "sh".into(),
                },
                cwd: None,
                shareable: true,
                input,
                ssh: tab.session.clone(),
            },
        );
        tab.binding = Binding {
            id: Some(id),
            ..Default::default()
        };
    }
    pub(super) fn unbind_terminal(&mut self, index: usize) {
        if let Some((id, generation)) = self.tabs[index].binding.id.take() {
            self.external.registry.disconnect(&id, generation);
            self.external.registry.remove(&id);
        }
    }
    /// 受信出力と表示中の画面を台帳へ写す。frameごとに1回だけ画面を作る。
    pub(super) fn flush_bindings(&mut self) {
        for tab in &mut self.tabs {
            if !tab.binding.dirty {
                continue;
            }
            tab.binding.dirty = false;
            if let Some((id, generation)) = &tab.binding.id {
                let screen = tab.terminal.visible_text().join("\n");
                let text = std::mem::take(&mut tab.binding.pending);
                self.external
                    .registry
                    .update_output(id, *generation, &text, &screen);
            }
        }
    }
    pub(super) fn poll_external(&mut self) {
        while let Ok(request) = self.external.approvals.try_recv() {
            self.external.pending.push_back(request);
        }
        // 取り消された承認待ち（受信側が閉じた）を画面から外す。
        self.external
            .pending
            .retain(|request| !request.reply.is_closed());
        if let Some(receiver) = &mut self.external.mcp_starting {
            match receiver.try_recv() {
                Ok(Ok(bridge)) => {
                    self.external.mcp_config = match bridge.config() {
                        Ok(config) => serde_json::to_string_pretty(&config).unwrap_or_default(),
                        Err(error) => format!("{error:#}"),
                    };
                    self.external.mcp = Some(bridge);
                    self.external.mcp_starting = None;
                }
                Ok(Err(error)) => {
                    self.notice = format!("MCPを開始できません: {error}");
                    self.external.mcp_starting = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.external.mcp_starting = None
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            }
        }
    }
    pub(super) fn stop_external(&mut self) {
        self.external.broker.stop();
        if let Some(bridge) = self.external.mcp.take() {
            bridge.close();
        }
        for request in self.external.pending.drain(..) {
            let _ = request.reply.send(false);
        }
    }
    pub(super) fn external_dialog(&mut self, ctx: &egui::Context) {
        self.approval_dialog(ctx);
        if !self.external.open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut stop = false;
        let mut start_mcp = false;
        let mut stop_mcp = false;
        let terminals: Vec<(Stamp, String, bool)> = self
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.stamp.clone(),
                    tab.label.clone(),
                    tab.binding.id.is_some() && !tab.closing,
                )
            })
            .collect();
        let external = &mut self.external;
        egui::Window::new("外部操作（AI・MCP）")
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("AIやMCPクライアントに端末の閲覧・操作を許可します。");
                ui.label(
                    "端末の出力は信頼できない観測データです。秘密の除去は完全ではありません。",
                );
                ui.separator();
                for (mode, label) in [
                    (Mode::Consult, "読むだけ"),
                    (Mode::Confirm, "操作ごとに確認"),
                    (Mode::Auto, "選択した端末で自動実行"),
                ] {
                    ui.radio_value(&mut external.mode, mode, label);
                }
                ui.label("共有する端末");
                for (stamp, label, available) in &terminals {
                    let mut checked = external.selected.contains(stamp);
                    if ui
                        .add_enabled(
                            *available,
                            egui::Checkbox::new(&mut checked, label.as_str()),
                        )
                        .changed()
                    {
                        external.selected.retain(|s| s != stamp);
                        if checked {
                            external.selected.push(stamp.clone());
                        }
                    }
                }
                ui.label("共有は15分間です。端末を選び直すか停止すると、実行許可も取り消します。");
                ui.horizontal(|ui| {
                    apply = ui.button("共有を開始").clicked();
                    stop = ui.button("停止").clicked();
                });
                let state = external.broker.permission_snapshot();
                let shared = external.broker.list_sessions().len();
                ui.label(format!(
                    "{shared}端末を共有中 · {} · 残り{}分",
                    match state["mode"].as_str() {
                        Some("confirm") => "操作ごとに確認",
                        Some("auto") => "自動実行",
                        _ => "読むだけ",
                    },
                    state["seconds"].as_u64().unwrap_or(0) / 60
                ));
                let audit = external.broker.audit_snapshot();
                let active = audit["active"].as_array().cloned().unwrap_or_default();
                if !active.is_empty() {
                    ui.label(format!("実行中・承認待ち: {}件", active.len()));
                }
                ui.separator();
                ui.heading("MCP（ローカル）");
                ui.label(
                    "起動中のこのHashiへだけ接続できるstdioサーバーです。外部へ公開しません。",
                );
                match &external.mcp {
                    None => {
                        let starting = external.mcp_starting.is_some();
                        start_mcp = ui
                            .add_enabled(!starting, egui::Button::new("MCPを開始"))
                            .clicked();
                    }
                    Some(_) => {
                        ui.label("MCPクライアントの設定（このHashiを終了すると無効になります）");
                        let mut config = external.mcp_config.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut config)
                                .code_editor()
                                .desired_rows(6),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("設定をコピー").clicked() {
                                ui.ctx().copy_text(external.mcp_config.clone());
                            }
                            stop_mcp = ui.button("MCPを停止").clicked();
                        });
                    }
                }
            });
        if !open {
            self.external.open = false;
        }
        if stop {
            self.external.broker.stop();
            self.notice = "外部操作の共有を停止しました".into();
        }
        if apply {
            let targets: Vec<_> = self
                .tabs
                .iter()
                .filter(|tab| self.external.selected.contains(&tab.stamp) && !tab.closing)
                .filter_map(|tab| tab.binding.id.clone())
                .collect();
            match self
                .external
                .broker
                .configure(self.external.mode, &targets, SHARE_LIFETIME)
            {
                Ok(()) => self.notice = format!("{}端末の共有を開始しました", targets.len()),
                Err(error) => self.notice = format!("共有を開始できません: {error}"),
            }
        }
        if stop_mcp {
            if let Some(bridge) = self.external.mcp.take() {
                bridge.close();
            }
            self.external.mcp_config.clear();
        }
        if start_mcp && !self.exiting {
            let broker = self.external.broker.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.runtime.handle().spawn(async move {
                let result = hashi_mcp::McpBridge::start(broker).await;
                let _ = tx.send(result.map_err(|error| format!("{error:#}")));
            });
            self.external.mcp_starting = Some(rx);
        }
    }
    fn approval_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = self.external.pending.front() else {
            return;
        };
        let mut answer = None;
        egui::Window::new("外部からの端末操作の確認")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("依頼元: {}", request.actor));
                ui.label(format!(
                    "操作: {}",
                    match request.action {
                        hashi_core::broker::Action::SendInput => "対話端末へ入力",
                        hashi_core::broker::Action::RunCommand => "独立コマンドを実行",
                        hashi_core::broker::Action::Interrupt => "Ctrl+Cを送信",
                    }
                ));
                ui.label(format!(
                    "端末: {}（世代{}）",
                    request.label, request.generation
                ));
                if let Some(cwd) = &request.cwd {
                    ui.label(format!("開始フォルダ: {cwd}"));
                }
                if !request.text.is_empty() {
                    ui.label("内容（実行前に確認してください）");
                    let mut text = request.text.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_rows(4),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("許可して実行").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("拒否").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(answer) = answer {
            if let Some(request) = self.external.pending.pop_front() {
                let _ = request.reply.send(answer);
            }
        }
    }
}
