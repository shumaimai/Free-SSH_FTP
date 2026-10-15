#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod editor_view;
mod file_view;
mod forward_view;
use file_view::{Browser, FileEvent};
mod style;
mod terminal_view;
use editor_view::{Editor, EditorEvent};
use eframe::egui;
use forward_view::{ActiveForward, ForwardEditor};
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
    ForwardReady {
        stamp: Stamp,
        handle: ActiveForward,
    },
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
    DirectoryError {
        stamp: Stamp,
        navigation: Stamp,
        message: String,
    },
    Entries {
        stamp: Stamp,
        navigation: Stamp,
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
    forwards: Vec<ActiveForward>,
    navigation: Stamp,
    navigation_cancel: CancellationToken,
    terminal_search: String,
    search_open: bool,
    search_position: Option<(i32, usize)>,
    pending_size: Option<(u32, u32)>,
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
            forwards: vec![],
            navigation: Stamp::default(),
            navigation_cancel: CancellationToken::new(),
            terminal_search: String::new(),
            search_open: false,
            search_position: None,
            pending_size: None,
        }
    }
    fn begin_navigation(&mut self) -> (Stamp, CancellationToken) {
        self.navigation_cancel.cancel();
        self.navigation_cancel = self.cancel.child_token();
        self.navigation = Stamp::default();
        (self.navigation.clone(), self.navigation_cancel.clone())
    }
    fn apply_listing(&mut self, navigation: Stamp, path: String, entries: Vec<Entry>) -> bool {
        if self.navigation != navigation {
            return false;
        }
        self.path = path;
        self.entries = entries;
        self.selected = None;
        true
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
        if let (Some(size), Some(resize)) = (self.pending_size, self.resize.as_ref()) {
            if resize.try_send(size).is_ok() {
                self.pending_size = None;
            }
        }
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
    forward_editor: ForwardEditor,
    editors: Vec<Editor>,
    editor_jobs: usize,
    editor_events: mpsc::Receiver<EditorEvent>,
    editor_tx: mpsc::Sender<EditorEvent>,
    confirm_exit: bool,
    allow_close: bool,
    browser: Browser,
    file_events: mpsc::Receiver<FileEvent>,
    file_tx: mpsc::Sender<FileEvent>,
}
impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        edit: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
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
        let (editor_tx, editor_events) = mpsc::channel(8);
        let (file_tx, file_events) = mpsc::channel(16);
        let (tx, events) = mpsc::channel(128);
        let (local_tx, local_events) = mpsc::channel(128);
        let (progress_tx, progress) = mpsc::channel(128);
        let mut app = Self {
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
            forward_editor: ForwardEditor::default(),
            editors: vec![],
            editor_jobs: 0,
            editor_events,
            editor_tx,
            confirm_exit: false,
            allow_close: false,
            browser: Browser::default(),
            file_events,
            file_tx,
        };
        if let Some(path) = edit {
            app.load_local_editor(path);
        }
        Ok(app)
    }
    fn connect(&mut self) {
        let mut tab = Tab::new(if self.profile.name.is_empty() {
            self.profile.host.clone()
        } else {
            self.profile.name.clone()
        });
        let profile = self.profile.clone();
        if !profile.initial_path.is_empty() {
            tab.path = profile.initial_path.clone();
        }
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
                Message::ForwardReady { stamp, .. }
                | Message::Connected { stamp, .. }
                | Message::Data { stamp, .. }
                | Message::Error { stamp, .. }
                | Message::DirectoryError { stamp, .. }
                | Message::Entries { stamp, .. }
                | Message::Done { stamp, .. } => stamp,
            };
            let Some(index) = self.tabs.iter().position(|tab| &tab.stamp == stamp) else {
                continue;
            };
            match event {
                Message::ForwardReady { handle, .. } => {
                    if !self.tabs[index].cancel.is_cancelled() {
                        self.tabs[index].forwards.push(handle);
                    }
                }
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
                    let initial = self.tabs[index].path.clone();
                    self.browse(index, initial);
                }
                Message::Data { bytes, .. } => {
                    let tab = &mut self.tabs[index];
                    tab.terminal.feed(&bytes);
                    for reply in tab.terminal.replies() {
                        tab.send(reply);
                    }
                }
                Message::Error { message, .. } => self.tabs[index].status = message,
                Message::DirectoryError {
                    navigation,
                    message,
                    ..
                } => {
                    if self.tabs[index].navigation == navigation {
                        self.notice = message;
                    }
                }
                Message::Entries {
                    path,
                    entries,
                    navigation,
                    ..
                } => {
                    let tab = &mut self.tabs[index];
                    tab.apply_listing(navigation, path, entries);
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
            self.file_progress(&progress);
            self.notice = format!(
                "処理中のファイル {} / {} バイト",
                progress.done, progress.total
            );
        }
    }
    fn browse(&mut self, index: usize, path: String) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let Some(nav) = tab.nav.clone() else {
            return;
        };
        let Some(session) = tab.session.clone() else {
            return;
        };
        let (navigation, cancel) = tab.begin_navigation();
        let stamp = tab.stamp.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let nav = nav.lock().await;
            let result = async {
                let path = nav.canonicalize(path).await?;
                let entries = transfer::list_bounded(&session, &path, &cancel).await?;
                anyhow::Ok((path, entries))
            }
            .await;
            let event = match result {
                Ok((path, entries)) => Message::Entries {
                    stamp,
                    navigation,
                    path,
                    entries,
                },
                Err(error) => Message::DirectoryError {
                    stamp,
                    navigation,
                    message: error.to_string(),
                },
            };
            tokio::select! { biased; _=cancel.cancelled()=>{},_=tx.send(event)=>{} }
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
        self.process_editors();
        self.process_files();
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.reset_file_drop();
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Hashi Rust プレビュー");
                if ui.button("ローカルCMD").clicked() {
                    self.local();
                }
                if ui.button("ローカル編集").clicked() {
                    self.open_local_editor();
                }
                if ui.button("ファイル・転送").clicked() {
                    self.browser.open = true;
                    self.refresh_local(std::env::current_dir().unwrap_or_default());
                }
                if ui.button("ポート転送").clicked() {
                    self.forward_editor.open = true;
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
                ui.checkbox(
                    &mut self.profile.agent_forwarding,
                    "エージェント転送（接続先に署名を許可）",
                );
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

            let has_sftp = tab.nav.is_some();
            ui.label(status);
            if has_sftp {
                egui::Panel::right("files")
                    .default_size(330.0)
                    .show(ui, |ui| self.remote_file_panel(ui));
            }
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    terminal_view::show(ui, tab);
                }
            });
        });
        self.auth_dialog(&ctx);
        self.forward_dialog(&ctx);
        self.editor_windows(&ctx);
        self.file_windows(&ctx);
        self.dropped_files(&ctx);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.cancel_files();
        for tab in &self.tabs {
            tab.cancel.cancel();
        }
    }
}
fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let edit = match arguments.next() {
        None => None,
        Some(flag) if flag == "--edit" => {
            Some(std::path::PathBuf::from(arguments.next().ok_or_else(
                || anyhow::anyhow!("--editにはファイルを指定してください"),
            )?))
        }
        _ => anyhow::bail!("使い方: Hashi-RustPreview [--edit FILE]"),
    };
    if arguments.next().is_some() {
        anyhow::bail!("引数が多すぎます");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Hashi Rust Preview",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, edit)?))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_navigation_cannot_replace_the_new_directory() {
        let mut tab = Tab::new("試験".into());
        let (old, old_cancel) = tab.begin_navigation();
        let (current, current_cancel) = tab.begin_navigation();
        assert!(old_cancel.is_cancelled());
        assert!(!current_cancel.is_cancelled());
        assert!(!tab.apply_listing(old, "/old".into(), vec![]));
        assert_eq!(tab.path, ".");
        assert!(tab.apply_listing(current, "/new".into(), vec![]));
        assert_eq!(tab.path, "/new");
    }
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
