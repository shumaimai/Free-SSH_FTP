#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod terminal_view;
use eframe::egui;
use hashi_core::{
    interaction::{Challenge, Interaction, Reply, Request, Stamp},
    local::{LocalTerminal, TerminalEvent},
    ssh::SshSession,
    transfer::{self, Entry, Progress},
};
use hashi_storage::{
    config::{Profile, Snapshot, Store},
    paths::Channel,
};
use hashi_terminal::Terminal;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{runtime::Runtime, sync::mpsc};
use tokio_util::sync::CancellationToken;
type Sftp = Arc<tokio::sync::Mutex<russh_sftp::client::SftpSession>>;
enum Message {
    Connected {
        stamp: Stamp,
        session: Arc<SshSession>,
        nav: Sftp,
        xfer: Sftp,
        input: mpsc::Sender<Vec<u8>>,
        resize: mpsc::Sender<(u32, u32)>,
    },
    Data {
        stamp: Stamp,
        bytes: Vec<u8>,
    },
    Error {
        stamp: Stamp,
        message: String,
    },
    Entries {
        stamp: Stamp,
        path: String,
        entries: Vec<Entry>,
    },
    Done {
        stamp: Stamp,
        message: String,
    },
}
struct Tab {
    stamp: Stamp,
    label: String,
    terminal: Terminal,
    status: String,
    cancel: CancellationToken,
    session: Option<Arc<SshSession>>,
    nav: Option<Sftp>,
    xfer: Option<Sftp>,
    input: Option<mpsc::Sender<Vec<u8>>>,
    resize: Option<mpsc::Sender<(u32, u32)>>,
    local: Option<Arc<Mutex<LocalTerminal>>>,
    path: String,
    entries: Vec<Entry>,
    selected: Option<usize>,
    preedit: String,
    selection_start: Option<(i32, usize)>,
    pending_input: VecDeque<Vec<u8>>,
    pending_bytes: usize,
}
impl Tab {
    fn new(label: String) -> Self {
        Self {
            stamp: Stamp::default(),
            label,
            terminal: Terminal::new(80, 24),
            status: "接続中".into(),
            cancel: CancellationToken::new(),
            session: None,
            nav: None,
            xfer: None,
            input: None,
            resize: None,
            local: None,
            path: ".".into(),
            entries: vec![],
            selected: None,
            preedit: String::new(),
            selection_start: None,
            pending_input: VecDeque::new(),
            pending_bytes: 0,
        }
    }
    fn send(&mut self, bytes: Vec<u8>) {
        if self.pending_bytes + bytes.len() > 1024 * 1024 {
            self.status = "端末入力の待ち行列が満杯です。送信を中止しました".into();
            return;
        }
        self.pending_bytes += bytes.len();
        for chunk in bytes.chunks(32768) {
            self.pending_input.push_back(chunk.to_vec());
        }
        self.flush_input();
    }
    fn flush_input(&mut self) {
        while let Some(bytes) = self.pending_input.pop_front() {
            let size = bytes.len();
            if let Some(input) = &self.input {
                match input.try_send(bytes) {
                    Ok(()) => self.pending_bytes -= size,
                    Err(mpsc::error::TrySendError::Full(bytes)) => {
                        self.pending_input.push_front(bytes);
                        break;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        self.status = "端末の送信先が終了しました".into();
                        self.pending_input.clear();
                        self.pending_bytes = 0;
                        break;
                    }
                }
            } else if let Some(local) = &self.local {
                let result = local
                    .lock()
                    .map_err(|_| "ローカル端末を操作できません".to_owned())
                    .and_then(|mut terminal| terminal.send(&bytes).map_err(|e| e.to_string()));
                self.pending_bytes -= size;
                if let Err(error) = result {
                    self.status = error;
                    self.pending_input.clear();
                    self.pending_bytes = 0;
                    break;
                }
            } else {
                self.pending_input.push_front(bytes);
                break;
            }
        }
    }
}
struct App {
    runtime: Runtime,
    store: Arc<Store>,
    snapshot: Snapshot,
    profile: Profile,
    edit_index: Option<usize>,
    tabs: Vec<Tab>,
    active: usize,
    interaction: Interaction,
    requests: mpsc::Receiver<Request>,
    pending: Vec<Request>,
    secret: String,
    confirmation: String,
    events: mpsc::Receiver<Message>,
    tx: mpsc::Sender<Message>,
    local_events: mpsc::Receiver<TerminalEvent>,
    local_tx: mpsc::Sender<TerminalEvent>,
    progress: mpsc::Receiver<Progress>,
    progress_tx: mpsc::Sender<Progress>,
    notice: String,
    startup_error: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "NotoSansJP".into(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/NotoSansJP.ttf"
            ))),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("NotoSansJP".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let store = Arc::new(Store::new(Channel::Preview)?);
        let (snapshot, notice, startup_error) = match store.load() {
            Ok(snapshot) => (snapshot, String::new(), false),
            Err(error) => (
                Snapshot::default(),
                format!("保存データを復旧してから再起動してください: {error}"),
                true,
            ),
        };
        let (interaction, requests) = Interaction::new();
        let (tx, events) = mpsc::channel(128);
        let (local_tx, local_events) = mpsc::channel(128);
        let (progress_tx, progress) = mpsc::channel(128);
        Ok(Self {
            runtime: Runtime::new()?,
            store,
            snapshot,
            profile: Profile::default(),
            edit_index: None,
            tabs: vec![],
            active: 0,
            interaction,
            requests,
            pending: vec![],
            secret: String::new(),
            confirmation: String::new(),
            events,
            tx,
            local_events,
            local_tx,
            progress,
            progress_tx,
            notice,
            startup_error,
        })
    }
    fn connect(&mut self) {
        let tab = Tab::new(if self.profile.name.is_empty() {
            self.profile.host.clone()
        } else {
            self.profile.name.clone()
        });
        let profile = self.profile.clone();
        let stamp = tab.stamp.clone();
        let cancel = tab.cancel.clone();
        let known = self.snapshot.known_hosts.clone();
        let interaction = self.interaction.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move{let result=async{
            let session=Arc::new(SshSession::connect(&profile,stamp.clone(),known,interaction,cancel.clone()).await?);
            let nav=Arc::new(tokio::sync::Mutex::new(session.sftp().await?));let xfer=Arc::new(tokio::sync::Mutex::new(session.sftp().await?));
            let channel=session.shell(80,24).await?;let(mut reader,writer)=channel.split();let(input,mut input_rx)=mpsc::channel::<Vec<u8>>(32);let(resize,mut resize_rx)=mpsc::channel::<(u32,u32)>(8);
            tx.send(Message::Connected{stamp:stamp.clone(),session:session.clone(),nav,xfer,input,resize}).await?;
            loop{tokio::select!{_=cancel.cancelled()=>{let _=writer.close().await;let _=session.close().await;break;},bytes=input_rx.recv()=>match bytes{Some(bytes)=>writer.data_bytes(bytes).await?,None=>break},size=resize_rx.recv()=>if let Some((columns,rows))=size{writer.window_change(columns,rows,0,0).await?;},message=reader.wait()=>match message{Some(russh::ChannelMsg::Data{data})|Some(russh::ChannelMsg::ExtendedData{data,..})=>{tokio::select!{_=cancel.cancelled()=>break,result=tx.send(Message::Data{stamp:stamp.clone(),bytes:data.to_vec()})=>{result?;}}},Some(russh::ChannelMsg::Close)|None=>break,_=>{}}}}
            anyhow::Ok(())}.await;let message=match result{Ok(())=>"切断されました".into(),Err(error)=>error.to_string()};let _=tx.send(Message::Error{stamp,message}).await;});
        self.active = self.tabs.len();
        self.tabs.push(tab);
    }
    fn local(&mut self) {
        let mut tab = Tab::new("CMD".into());
        match LocalTerminal::spawn(
            if cfg!(windows) { "cmd.exe" } else { "/bin/sh" },
            &[],
            None,
            tab.stamp.clone(),
            self.local_tx.clone(),
        ) {
            Ok(local) => {
                tab.input = Some(local.input_sender());
                tab.local = Some(Arc::new(Mutex::new(local)));
                tab.status = "ローカル端末".into();
                self.active = self.tabs.len();
                self.tabs.push(tab);
            }
            Err(error) => self.notice = error.to_string(),
        }
    }
    fn process(&mut self) {
        for tab in &mut self.tabs {
            tab.flush_input();
        }
        while let Ok(request) = self.requests.try_recv() {
            if self
                .interaction
                .is_pending(&request.challenge_id, &request.scope)
            {
                self.pending.push(request);
            }
        }
        let had_pending = self
            .pending
            .first()
            .map(|request| request.challenge_id.clone());
        self.pending.retain(|request| {
            self.interaction
                .is_pending(&request.challenge_id, &request.scope)
                && self
                    .tabs
                    .iter()
                    .any(|tab| tab.stamp == request.scope.stamp && !tab.cancel.is_cancelled())
        });
        if had_pending
            != self
                .pending
                .first()
                .map(|request| request.challenge_id.clone())
        {
            self.secret.clear();
            self.confirmation.clear();
        }
        for _ in 0..256 {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            let stamp = match &event {
                Message::Connected { stamp, .. }
                | Message::Data { stamp, .. }
                | Message::Error { stamp, .. }
                | Message::Entries { stamp, .. }
                | Message::Done { stamp, .. } => stamp,
            };
            let Some(index) = self.tabs.iter().position(|tab| &tab.stamp == stamp) else {
                continue;
            };
            match event {
                Message::Connected {
                    session,
                    nav,
                    xfer,
                    input,
                    resize,
                    ..
                } => {
                    let tab = &mut self.tabs[index];
                    tab.session = Some(session);
                    tab.nav = Some(nav);
                    tab.xfer = Some(xfer);
                    tab.input = Some(input);
                    tab.resize = Some(resize);
                    tab.status = "接続済み".into();
                    self.browse(index, ".".into());
                }
                Message::Data { bytes, .. } => {
                    let tab = &mut self.tabs[index];
                    tab.terminal.feed(&bytes);
                    for reply in tab.terminal.replies() {
                        tab.send(reply);
                    }
                }
                Message::Error { message, .. } => self.tabs[index].status = message,
                Message::Entries { path, entries, .. } => {
                    let tab = &mut self.tabs[index];
                    tab.path = path;
                    tab.entries = entries;
                    tab.selected = None;
                }
                Message::Done { message, .. } => self.notice = message,
            }
        }
        for _ in 0..256 {
            let Ok(event) = self.local_events.try_recv() else {
                break;
            };
            match event {
                TerminalEvent::Data { stamp, bytes } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.stamp == stamp) {
                        tab.terminal.feed(&bytes);
                        for reply in tab.terminal.replies() {
                            tab.send(reply);
                        }
                    }
                }
                TerminalEvent::Closed { stamp } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.stamp == stamp) {
                        tab.status = "終了".into();
                    }
                }
            }
        }
        while let Ok(progress) = self.progress.try_recv() {
            self.notice = format!("転送 {} / {} バイト", progress.done, progress.total);
        }
    }
    fn browse(&self, index: usize, path: String) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let Some(nav) = tab.nav.clone() else {
            return;
        };
        let stamp = tab.stamp.clone();
        let cancel = tab.cancel.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let nav = nav.lock().await;
            let result = async {
                let path = nav.canonicalize(path).await?;
                let entries = transfer::list(&nav, &path, &cancel).await?;
                anyhow::Ok((path, entries))
            }
            .await;
            let event = match result {
                Ok((path, entries)) => Message::Entries {
                    stamp,
                    path,
                    entries,
                },
                Err(error) => Message::Error {
                    stamp,
                    message: error.to_string(),
                },
            };
            let _ = tx.send(event).await;
        });
    }
    fn save_profile(&mut self) {
        let profile = self.profile.clone();
        let index = self.edit_index;
        match self.store.transact(|snapshot| {
            profile.validate()?;
            if let Some(index) = index {
                if index < snapshot.profiles.len() {
                    snapshot.profiles[index] = profile.clone();
                } else {
                    return Err(std::io::Error::other("接続設定がありません"));
                }
            } else {
                snapshot.profiles.push(profile.clone());
            }
            Ok(())
        }) {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.notice = "保存しました".into();
            }
            Err(error) => self.notice = error.to_string(),
        }
    }
    fn transfer(&mut self, upload: bool) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(xfer) = tab.xfer.clone() else {
            return;
        };
        let stamp = tab.stamp.clone();
        let cancel = tab.cancel.child_token();
        let path = tab.path.clone();
        let selected = tab.selected.and_then(|i| tab.entries.get(i)).cloned();
        let tx = self.tx.clone();
        let progress = self.progress_tx.clone();
        let local = if upload {
            rfd::FileDialog::new().pick_file()
        } else {
            rfd::FileDialog::new()
                .set_file_name(
                    selected
                        .as_ref()
                        .map(|e| e.name.as_str())
                        .unwrap_or("download.bin"),
                )
                .save_file()
        };
        let Some(local) = local else {
            return;
        };
        let remote = if upload {
            format!(
                "{}/{}",
                path.trim_end_matches('/'),
                local
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default()
            )
        } else {
            let Some(selected) = selected else {
                return;
            };
            if selected.directory {
                self.notice = "再帰転送は実装中です".into();
                return;
            }
            selected.path
        };
        self.runtime.spawn(async move {
            let xfer = xfer.lock().await;
            let job = stamp.generation.clone();
            let result = if upload {
                transfer::upload(&xfer, &local, &remote, false, &job, &progress, &cancel).await
            } else {
                transfer::download(
                    &xfer,
                    &remote,
                    &local,
                    transfer::DownloadPolicy::default(),
                    &job,
                    &progress,
                    &cancel,
                )
                .await
            };
            let message = match result {
                Ok(()) => "転送完了".into(),
                Err(error) => error.to_string(),
            };
            let _ = tx.send(Message::Done { stamp, message }).await;
        });
    }
    fn auth_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = self.pending.first().cloned() else {
            return;
        };
        let mut answer = None;
        egui::Window::new("接続先の確認・認証")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{}@{}:{}{}",
                    request.scope.username,
                    request.scope.host,
                    request.scope.port,
                    request
                        .scope
                        .hop
                        .map(|n| format!("（踏み台{}）", n + 1))
                        .unwrap_or_default()
                ));
                match &request.challenge {
                    Challenge::HostKey { key, old } => {
                        ui.label(&key.key_type);
                        ui.monospace(&key.fingerprint);
                        if let Some(old) = old {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "鍵が違います。確認後に trust と入力してください。",
                            );
                            ui.monospace(&old.fingerprint);
                            ui.text_edit_singleline(&mut self.confirmation);
                        }
                        if ui.button("信頼して接続").clicked()
                            && (old.is_none() || self.confirmation == "trust")
                        {
                            answer = Some(Reply::HostTrust {
                                accept: true,
                                confirmation: self.confirmation.clone(),
                            });
                        }
                    }
                    Challenge::Secret { prompt } => {
                        ui.label(prompt);
                        ui.add(egui::TextEdit::singleline(&mut self.secret).password(true));
                        if ui.button("送信").clicked() {
                            answer = Some(Reply::Secret(std::mem::take(&mut self.secret)));
                        }
                    }
                }
                if ui.button("中止").clicked() {
                    answer = Some(Reply::Cancel);
                }
            });
        if let Some(answer) = answer {
            let mut saved = None;
            let trust = matches!(&answer, Reply::HostTrust { accept: true, .. });
            let result =
                self.interaction
                    .reply_with(&request.challenge_id, &request.scope, answer, || {
                        if trust {
                            if let Challenge::HostKey { key, .. } = &request.challenge {
                                let host = format!("{}:{}", request.scope.host, request.scope.port);
                                saved = Some(self.store.transact(|s| {
                                    s.known_hosts.insert(host, key.clone());
                                    Ok(())
                                })?);
                            }
                        }
                        Ok(())
                    });
            if let Err(e) = result {
                self.notice = e.to_string();
            }
            if let Some(snapshot) = saved {
                self.snapshot = snapshot;
            }
            self.pending.remove(0);
            self.secret.clear();
            self.confirmation.clear();
        }
    }
}
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.process();
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Hashi Rust プレビュー");
                if ui.button("ローカルCMD").clicked() {
                    self.local();
                }
            });
        });
        egui::Panel::left("profiles")
            .min_size(260.0)
            .show(ui, |ui| {
                ui.heading("接続先");
                for (index, p) in self.snapshot.profiles.clone().iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.edit_index == Some(index),
                            if p.name.is_empty() { &p.host } else { &p.name },
                        )
                        .clicked()
                    {
                        self.profile = p.clone();
                        self.edit_index = Some(index);
                    }
                }
                if ui.button("新規接続").clicked() {
                    self.profile = Profile::default();
                    self.edit_index = None;
                }
                ui.separator();
                ui.label("名前");
                ui.text_edit_singleline(&mut self.profile.name);
                ui.label("ホスト");
                ui.text_edit_singleline(&mut self.profile.host);
                ui.label("ポート");
                ui.add(egui::DragValue::new(&mut self.profile.port).range(1..=65535));
                ui.label("利用者名");
                ui.text_edit_singleline(&mut self.profile.username);
                egui::ComboBox::from_id_salt("auth")
                    .selected_text(&self.profile.auth_method)
                    .show_ui(ui, |ui| {
                        for method in ["key", "password", "agent"] {
                            ui.selectable_value(
                                &mut self.profile.auth_method,
                                method.into(),
                                method,
                            );
                        }
                    });
                if self.profile.auth_method == "key" {
                    ui.label("秘密鍵");
                    ui.text_edit_singleline(&mut self.profile.key_path);
                    if ui.button("秘密鍵を選択").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.profile.key_path = path.to_string_lossy().into_owned();
                        }
                    }
                }
                ui.label("踏み台（ProxyJump）");
                ui.text_edit_singleline(&mut self.profile.proxy_jump);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.startup_error, egui::Button::new("保存"))
                        .clicked()
                    {
                        self.save_profile();
                    }
                    if ui
                        .add_enabled(!self.startup_error, egui::Button::new("接続"))
                        .clicked()
                    {
                        self.connect();
                    }
                });
            });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.notice);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let mut closed = None;
            ui.horizontal_wrapped(|ui| {
                for (index, tab) in self.tabs.iter().enumerate() {
                    if ui
                        .selectable_label(self.active == index, &tab.label)
                        .clicked()
                    {
                        self.active = index;
                    }
                    if ui.small_button("×").clicked() {
                        closed = Some(index);
                    }
                }
            });
            if let Some(index) = closed {
                let tab = self.tabs.remove(index);
                tab.cancel.cancel();
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
            }
            let Some(tab) = self.tabs.get(self.active) else {
                ui.label("接続先を選択するか、ローカルCMDを開いてください。");
                return;
            };
            let status = tab.status.clone();
            let entries = tab.entries.clone();
            let path = tab.path.clone();
            let has_sftp = tab.nav.is_some();
            ui.label(status);
            if has_sftp {
                egui::Panel::right("files")
                    .default_size(330.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("親へ").clicked() {
                                self.browse(self.active, format!("{path}/.."));
                            }
                            if ui.button("更新").clicked() {
                                self.browse(self.active, path.clone());
                            }
                            if ui.button("送信").clicked() {
                                self.transfer(true);
                            }
                            if ui.button("受信").clicked() {
                                self.transfer(false);
                            }
                        });
                        ui.label(path);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (index, entry) in entries.iter().enumerate() {
                                let response = ui.selectable_label(
                                    self.tabs[self.active].selected == Some(index),
                                    format!(
                                        "{} {}",
                                        if entry.directory { "[dir]" } else { "[file]" },
                                        entry.name
                                    ),
                                );
                                if response.clicked() {
                                    self.tabs[self.active].selected = Some(index);
                                }
                                if response.double_clicked() && entry.directory {
                                    self.browse(self.active, entry.path.clone());
                                }
                            }
                        });
                    });
            }
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    terminal_view::show(ui, tab);
                }
            });
        });
        self.auth_dialog(&ctx);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        for tab in &self.tabs {
            tab.cancel.cancel();
        }
    }
}
fn main() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Hashi Rust Preview",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)?))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bounded_terminal_input_preserves_order_when_channel_is_full() {
        let mut tab = Tab::new("試験".into());
        let (tx, mut rx) = mpsc::channel(1);
        tab.input = Some(tx);
        tab.send(b"first".to_vec());
        tab.send(b"second".to_vec());
        assert_eq!(tab.pending_bytes, 6);
        assert_eq!(rx.recv().await.unwrap(), b"first");
        tab.flush_input();
        assert_eq!(rx.recv().await.unwrap(), b"second");
        assert_eq!(tab.pending_bytes, 0);
        tab.send(vec![0; 1024 * 1024 + 1]);
        assert_eq!(tab.pending_bytes, 0);
        assert!(tab.status.contains("満杯"));
    }
}
