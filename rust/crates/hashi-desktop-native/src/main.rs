#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod ai_view;
mod editor_view;
mod external_view;
mod file_view;
mod forward_view;
mod keygen_view;
mod permission_view;
mod settings_view;
mod shutdown_view;
mod snippet_view;
mod sync_view;
use file_view::{Browser, FileEvent};
mod home_view;
mod style;
mod terminal_view;
mod theme;
use editor_view::{Editor, EditorEvent};
use eframe::egui;
use forward_view::{ActiveForward, ForwardEditor};
use hashi_core::work::Work;
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
    Quiesced {
        stamp: Stamp,
    },
    Closed {
        stamp: Stamp,
        errors: Vec<String>,
    },
    PermissionsReady {
        stamp: Stamp,
        result: Result<Arc<hashi_core::permissions::Manager>, String>,
    },
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
        failed: bool,
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
    pending_local_size: Option<(u16, u16)>,
    permissions: Option<Arc<hashi_core::permissions::Manager>>,
    permissions_busy: bool,
    work: Work,
    transport: Work,
    work_cancel: CancellationToken,
    closing: bool,
    remove_on_close: bool,
    /// SSH端末の受信ログ。CMDは対象外（Python版と同じ）。
    log: Option<hashi_core::session_log::SessionLog>,
    /// AI/MCP向け端末台帳への登録。
    binding: external_view::Binding,
}
impl Tab {
    fn new(label: String) -> Self {
        let cancel = CancellationToken::new();
        let work_cancel = cancel.child_token();
        Self {
            stamp: Stamp::default(),
            label,
            terminal: Terminal::new(80, 24),
            status: "接続中".into(),
            cancel,
            work_cancel,
            work: Work::default(),
            transport: Work::default(),
            closing: false,
            remove_on_close: false,
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
            pending_local_size: None,
            permissions: None,
            permissions_busy: false,
            log: None,
            binding: Default::default(),
        }
    }
    /// 確定した受信行をログへ渡す。失敗したらこのタブのログを止めて理由を返す。
    fn drain_log(&mut self) -> Option<String> {
        let log = self.log.as_mut()?;
        let (mut lines, dropped) = self.terminal.take_log_lines();
        if dropped > 0 {
            lines.push(format!("[Hashi: {dropped}行を記録できませんでした]"));
        }
        let error = log.write_lines(lines).err()?;
        self.log = None;
        self.terminal.set_line_log(false);
        Some(format!("セッションログ: {error}"))
    }
    fn begin_navigation(&mut self) -> (Stamp, CancellationToken) {
        self.navigation_cancel.cancel();
        self.navigation_cancel = self.work_cancel.child_token();
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
        if self.closing || self.work.is_closed() {
            return;
        }
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
        if self.closing || self.work.is_closed() {
            return;
        }
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
    runtime: shutdown_view::RuntimeOwner,
    work: Work,
    closing_work: Work,
    exiting: bool,
    exit_started: Option<std::time::Instant>,
    confirm_started: Option<std::time::Instant>,
    close_errors: Vec<String>,
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
    permission_open: bool,
    legacy_stopped: bool,
    snippets: snippet_view::SnippetWindow,
    settings: settings_view::SettingsWindow,
    update_check: Option<tokio::sync::oneshot::Receiver<hashi_core::update::Update>>,
    available_update: Option<hashi_core::update::Update>,
    keygen: keygen_view::KeygenWindow,
    external: external_view::External,
    ai: ai_view::AiWindow,
    show_home: bool,
    home: home_view::HomeState,
    dock: Dock,
    dock_open: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dock {
    Files,
    Ai,
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
        theme::apply(&cc.egui_ctx);
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
            runtime: shutdown_view::RuntimeOwner::new()?,
            work: Work::default(),
            closing_work: Work::default(),
            exiting: false,
            exit_started: None,
            confirm_started: None,
            close_errors: vec![],
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
            permission_open: false,
            legacy_stopped: false,
            snippets: Default::default(),
            settings: Default::default(),
            update_check: None,
            available_update: None,
            keygen: Default::default(),
            external: Default::default(),
            ai: Default::default(),
            show_home: true,
            home: Default::default(),
            dock: Dock::Ai,
            dock_open: false,
            allow_close: false,
            browser: Browser::default(),
            file_events,
            file_tx,
        };
        app.browser.sync.enabled = app
            .snapshot
            .settings
            .get("sync_browse")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        app.ai_restore();
        if let Some(path) = edit {
            app.load_local_editor(path);
        } else {
            app.start_update_check();
        }
        Ok(app)
    }
    fn connect(&mut self) {
        if self.exiting {
            return;
        }
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
        tab.transport.spawn(self.runtime.handle(),async move {
            let result=async {
                let session=Arc::new(SshSession::connect(&profile,stamp.clone(),known,interaction,cancel.clone()).await?);
                let setup=async {
                    let nav=Arc::new(tokio::sync::Mutex::new(session.sftp().await?));
                    let xfer=Arc::new(tokio::sync::Mutex::new(session.sftp().await?));
                    let channel=session.shell(80,24).await?;
                    anyhow::Ok((nav,xfer,channel))
                };
                let initialized=tokio::select! {biased;_=cancel.cancelled()=>Err(anyhow::anyhow!("接続の開始を中止しました")),result=tokio::time::timeout(Duration::from_secs(30),setup)=>result.map_err(|_|anyhow::anyhow!("接続の初期化がタイムアウトしました")).and_then(|result|result)};
                let (nav,xfer,channel)=match initialized {
                    Ok(value)=>value,
                    Err(error)=>{let _=tokio::time::timeout(Duration::from_secs(5),session.close()).await;return Err(error);}
                };
                let(mut reader,writer)=channel.split();
                let(input,mut input_rx)=mpsc::channel::<Vec<u8>>(32);
                let(resize,mut resize_rx)=mpsc::channel::<(u32,u32)>(8);
                tx.send(Message::Connected {stamp:stamp.clone(),session:session.clone(),nav,xfer,input,resize}).await?;
                loop {
                    tokio::select! {biased;
                        _=cancel.cancelled()=>{let _=tokio::time::timeout(Duration::from_secs(1),writer.close()).await;break;},
                        bytes=input_rx.recv()=>match bytes {Some(bytes)=>tokio::select! {_=cancel.cancelled()=>break,result=writer.data_bytes(bytes)=>result?},None=>break},
                        size=resize_rx.recv()=>if let Some((columns,rows))=size {tokio::select! {_=cancel.cancelled()=>break,result=writer.window_change(columns,rows,0,0)=>result?};},
                        message=reader.wait()=>match message {
                            Some(russh::ChannelMsg::Data {data})|Some(russh::ChannelMsg::ExtendedData {data,..})=>{tokio::select! {_=cancel.cancelled()=>break,result=tx.send(Message::Data {stamp:stamp.clone(),bytes:data.to_vec()})=>{result?;}}},
                            Some(russh::ChannelMsg::Close)|None=>break,
                            _=>{},
                        }
                    }
                }
                anyhow::Ok(())
            }.await;
            let message=match result {Ok(())=>"端末の接続が終了しました".into(),Err(error)=>error.to_string()};
            let _=tx.send(Message::Error {stamp,message}).await;
        }).expect("新しい接続の受付");
        self.active = self.tabs.len();
        self.show_home = false;
        self.dock = Dock::Files;
        self.dock_open = true;
        self.tabs.push(tab);
    }
    fn local(&mut self) {
        if self.exiting {
            return;
        }
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
                self.show_home = false;
                self.tabs.push(tab);
                self.bind_terminal(self.active, hashi_core::broker::Kind::Local);
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
                Message::Quiesced { stamp }
                | Message::Closed { stamp, .. }
                | Message::PermissionsReady { stamp, .. }
                | Message::ForwardReady { stamp, .. }
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
                Message::Quiesced { .. } => self.finish_close(index),
                Message::Closed { errors, .. } => self.closed_tab(index, errors),
                Message::PermissionsReady { result, .. } => {
                    self.tabs[index].permissions_busy = false;
                    match result {
                        Ok(manager) => {
                            self.tabs[index].permissions = Some(manager);
                            self.notice = "この接続の権限補助を有効にしました".into();
                        }
                        Err(error) => self.notice = error,
                    }
                }
                Message::ForwardReady { handle, .. } => {
                    // Quiescedより前に届く開始済みhandleも、終了処理でstopを待つ。
                    self.tabs[index].forwards.push(handle);
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
                    if !self.tabs[index].closing {
                        self.bind_terminal(index, hashi_core::broker::Kind::Ssh);
                    }
                    if self.session_log_enabled() && !self.tabs[index].closing {
                        let (custom, default) = self.session_log_directory();
                        let tab = &mut self.tabs[index];
                        match hashi_core::session_log::SessionLog::open(
                            custom.as_deref(),
                            &default,
                            &tab.label,
                        ) {
                            Ok(log) => {
                                tab.terminal.set_line_log(true);
                                tab.status = format!("接続済み・ログ: {}", log.path().display());
                                tab.log = Some(log);
                            }
                            Err(error) => {
                                self.notice = format!("セッションログを開始できません: {error:#}")
                            }
                        }
                    }
                    let initial = self.tabs[index].path.clone();
                    if !self.tabs[index].closing {
                        self.browse(index, initial);
                    }
                }
                Message::Data { bytes, .. } => {
                    let tab = &mut self.tabs[index];
                    tab.binding.capture(&bytes);
                    tab.terminal.feed(&bytes);
                    for reply in tab.terminal.replies() {
                        tab.send(reply);
                    }
                    if let Some(error) = tab.drain_log() {
                        self.notice = error;
                    }
                }
                Message::Error { message, .. } => {
                    self.tabs[index].status = message;
                    if !self.tabs[index].closing {
                        self.begin_close(index, false);
                    }
                }
                Message::DirectoryError {
                    navigation,
                    message,
                    ..
                } => {
                    if let Some(sync) = self.browser.sync.remote_failed(&navigation, &message) {
                        self.notice = sync;
                    } else if self.tabs[index].navigation == navigation {
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
                    let stamp = tab.stamp.clone();
                    if tab.apply_listing(navigation.clone(), path.clone(), entries) {
                        let follow = self.browser.sync.on_remote(&stamp, &navigation, &path);
                        if index == self.active {
                            self.apply_follow(follow);
                        }
                    }
                }
                Message::Done {
                    message, failed, ..
                } => {
                    if self.exiting && failed {
                        self.close_errors.push(message.clone());
                    }
                    self.notice = message;
                }
            }
        }
        for _ in 0..256 {
            let Ok(event) = self.local_events.try_recv() else {
                break;
            };
            match event {
                TerminalEvent::Data { stamp, bytes } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.stamp == stamp) {
                        tab.binding.capture(&bytes);
                        tab.terminal.feed(&bytes);
                        for reply in tab.terminal.replies() {
                            tab.send(reply);
                        }
                    }
                }
                TerminalEvent::Closed { stamp } => {
                    if let Some(index) = self.tabs.iter().position(|tab| tab.stamp == stamp) {
                        self.tabs[index].status = "終了".into();
                        self.unbind_terminal(index);
                    }
                }
            }
        }
        self.flush_bindings();
        while let Ok(progress) = self.progress.try_recv() {
            self.file_progress(&progress);
            self.notice = format!(
                "処理中のファイル {} / {} バイト",
                progress.done, progress.total
            );
        }
    }
    fn browse(&mut self, index: usize, path: String) -> Option<Stamp> {
        let tab = self.tabs.get_mut(index)?;
        if self.exiting || tab.closing || tab.work.is_closed() {
            return None;
        }
        let work = tab.work.clone();
        let nav = tab.nav.clone()?;
        let session = tab.session.clone()?;
        let (navigation, cancel) = tab.begin_navigation();
        let navigation_stamp = navigation.clone();
        let permissions = tab.permissions.clone();
        let stamp = tab.stamp.clone();
        let tx = self.tx.clone();
        let _ = work.spawn(self.runtime.handle(), async move {
            let nav = nav.lock().await;
            let result = async {
                let path = nav.canonicalize(path).await?;
                let entries = hashi_core::permissions::with_access(
                    permissions.as_ref(),
                    &path,
                    0o500,
                    || transfer::list_bounded(&session, &path, &cancel),
                )
                .await?;
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
        Some(navigation_stamp)
    }
    /// 保存できたかを返す。新規は保存後に編集中の接続先として扱い、続けて保存しても増やさない。
    fn save_profile(&mut self) -> bool {
        if self.exiting {
            return false;
        }
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
                if index.is_none() {
                    self.edit_index = self.snapshot.profiles.len().checked_sub(1);
                }
                self.notice = "保存しました".into();
                true
            }
            Err(error) => {
                self.notice = error.to_string();
                false
            }
        }
    }
    fn auth_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = self.pending.first().cloned() else {
            return;
        };
        let mut answer = None;
        egui::Window::new("接続先の確認")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(theme::title(format!(
                    "{}@{}:{}{}",
                    request.scope.username,
                    request.scope.host,
                    request.scope.port,
                    request
                        .scope
                        .hop
                        .map(|n| format!("（踏み台{}）", n + 1))
                        .unwrap_or_default()
                )));
                match &request.challenge {
                    Challenge::HostKey { key, old } => {
                        if let Some(old) = old {
                            ui.label(
                                egui::RichText::new("このサーバーの鍵が、前に信頼した鍵と違います")
                                    .color(theme::DANGER)
                                    .strong(),
                            );
                            ui.label(theme::muted(
                                "サーバーの入れ替えか、通信の乗っ取りの可能性があります。心当たりがなければ中止してください。",
                            ));
                            ui.label(theme::muted("前に信頼した指紋"));
                            ui.monospace(&old.fingerprint);
                            ui.label(theme::muted("今回の指紋"));
                        } else {
                            ui.label("初めて接続するサーバーです。");
                            ui.label(theme::muted(
                                "管理者から聞いた指紋と同じか確認してから信頼してください。",
                            ));
                        }
                        ui.label(theme::muted(&key.key_type));
                        ui.monospace(&key.fingerprint);
                        if old.is_some() {
                            ui.label(theme::muted("信頼するには trust と入力"));
                            ui.text_edit_singleline(&mut self.confirmation);
                        }
                        ui.horizontal(|ui| {
                            let allowed = old.is_none() || self.confirmation == "trust";
                            if ui
                                .add_enabled(allowed, theme::primary("信頼して接続"))
                                .clicked()
                            {
                                answer = Some(Reply::HostTrust {
                                    accept: true,
                                    confirmation: self.confirmation.clone(),
                                });
                            }
                            if ui.button("中止").clicked() {
                                answer = Some(Reply::Cancel);
                            }
                        });
                    }
                    Challenge::Secret { prompt } => {
                        ui.label(prompt);
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut self.secret)
                                .password(true)
                                .desired_width(f32::INFINITY),
                        );
                        field.request_focus();
                        let submit = field.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.add(theme::primary("送信")).clicked() || submit {
                                answer = Some(Reply::Secret(std::mem::take(&mut self.secret)));
                            }
                            if ui.button("中止").clicked() {
                                answer = Some(Reply::Cancel);
                            }
                        });
                    }
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
        self.poll_update();
        self.sync_track_active();
        self.poll_external();
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.reset_file_drop();
        if self.exiting {
            ui.disable();
        }
        self.top_bar(ui);
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::muted(&self.notice));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shared = self.external.broker.list_sessions().len();
                        if shared > 0 {
                            theme::chip(ui, &format!("AIへ公開中 {shared}端末"), theme::WARN);
                        }
                    });
                });
            });
        let in_session = !self.show_home && self.tabs.get(self.active).is_some();
        if self.dock_open {
            self.dock_panel(ui, in_session);
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ui, |ui| {
                if !in_session {
                    self.home_page(ui);
                    return;
                }
                let Some(tab) = self.tabs.get(self.active) else {
                    return;
                };
                let (label, status) = (tab.label.clone(), tab.status.clone());
                ui.horizontal(|ui| {
                    theme::dot(ui, self.tab_color(self.active));
                    ui.label(egui::RichText::new(label).strong());
                    ui.label(theme::muted(status));
                });
                ui.add_space(4.0);
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.closing {
                        ui.label("処理の終了を待っています");
                    } else {
                        terminal_view::show(ui, tab);
                    }
                }
            });
        if !self.exiting {
            self.ai_tick(&ctx);
            self.auth_dialog(&ctx);
            self.profile_editor(&ctx);
            self.forward_dialog(&ctx);
            self.permission_dialog(&ctx);
            self.snippet_dialog(&ctx);
            self.settings_dialog(&ctx);
            self.keygen_dialog(&ctx);
            self.external_dialog(&ctx);
            self.editor_windows(&ctx);
            self.file_windows(&ctx);
            self.dropped_files(&ctx);
        }
        self.shutdown_ui(&ctx);
    }
}
impl App {
    /// 接続の状態を示す色。緑は使える、黄は接続中か終了待ち。
    fn tab_color(&self, index: usize) -> egui::Color32 {
        match self.tabs.get(index) {
            Some(tab) if tab.closing => theme::WARN,
            Some(tab) if tab.session.is_some() || tab.local.is_some() => theme::OK,
            Some(_) => theme::WARN,
            None => theme::MUTED,
        }
    }
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let mut closed = None;
        let mut select = None;
        let in_session = !self.show_home && self.tabs.get(self.active).is_some();
        let has_files = in_session
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.nav.is_some() && !tab.closing && !tab.work.is_closed());
        egui::Panel::top("bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Hashi")
                            .size(19.0)
                            .strong()
                            .color(theme::ACCENT),
                    );
                    ui.add_space(8.0);
                    let tabs_width = (ui.available_width() - 360.0).max(160.0);
                    egui::ScrollArea::horizontal()
                        .max_width(tabs_width)
                        .id_salt("tabs")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui.selectable_label(!in_session, "ホーム").clicked() {
                                    select = Some(None);
                                }
                                for (index, tab) in self.tabs.iter().enumerate() {
                                    ui.add_space(2.0);
                                    theme::dot(ui, self.tab_color(index));
                                    if ui
                                        .selectable_label(
                                            in_session && self.active == index,
                                            &tab.label,
                                        )
                                        .clicked()
                                    {
                                        select = Some(Some(index));
                                    }
                                    if ui.small_button("×").clicked() {
                                        closed = Some(index);
                                    }
                                }
                            });
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("設定").clicked() {
                            self.settings.open = true;
                        }
                        ui.menu_button("ツール", |ui| {
                            for (label, action) in [
                                ("ファイル転送（2ペイン）", 0),
                                ("ポート転送", 1),
                                ("スニペット", 2),
                                ("権限補助", 3),
                                ("SSH鍵を作る", 4),
                                ("外部連携（MCP）", 5),
                                ("ファイルを編集", 6),
                            ] {
                                if ui.button(label).clicked() {
                                    match action {
                                        0 => {
                                            self.browser.open = true;
                                            self.refresh_local(
                                                std::env::current_dir().unwrap_or_default(),
                                            );
                                        }
                                        1 => self.forward_editor.open = true,
                                        2 => self.snippets.open = true,
                                        3 => self.permission_open = true,
                                        4 => self.keygen.open = true,
                                        5 => self.external.open = true,
                                        _ => self.open_local_editor(),
                                    }
                                    ui.close();
                                }
                            }
                        });
                        let mut ai = self.dock_open && self.dock == Dock::Ai;
                        if ui.toggle_value(&mut ai, "AI").changed() {
                            self.dock_open = ai;
                            self.dock = Dock::Ai;
                        }
                        let mut files = self.dock_open && self.dock == Dock::Files;
                        if ui
                            .add_enabled(has_files, egui::Button::selectable(files, "ファイル"))
                            .clicked()
                        {
                            files = !files;
                            self.dock_open = files;
                            self.dock = Dock::Files;
                        }
                        self.update_banner(ui);
                    });
                });
            });
        match select {
            Some(None) => self.show_home = true,
            Some(Some(index)) => {
                self.show_home = false;
                self.active = index;
            }
            None => {}
        }
        if let Some(index) = closed {
            self.begin_close(index, true);
        }
    }
    fn dock_panel(&mut self, ui: &mut egui::Ui, in_session: bool) {
        let has_files = in_session
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.nav.is_some() && !tab.closing && !tab.work.is_closed());
        let mut close = false;
        egui::Panel::right("dock")
            .default_size(400.0)
            .size_range(320.0..=760.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::segmented(
                        ui,
                        &mut self.dock,
                        &[(Dock::Files, "ファイル"), (Dock::Ai, "AI")],
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.small_button("閉じる").clicked();
                    });
                });
                ui.add_space(6.0);
                match self.dock {
                    Dock::Files if has_files => self.remote_file_panel(ui),
                    Dock::Files => {
                        ui.label(theme::muted(
                            "接続中のサーバーを開くと、ここにファイルが並びます。",
                        ));
                    }
                    Dock::Ai => self.ai_dock(ui),
                }
            });
        if close {
            self.dock_open = false;
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.cancel_files();
        for tab in &self.tabs {
            tab.work_cancel.cancel();
            tab.cancel.cancel();
            if let Some(local) = &tab.local {
                if let Ok(mut terminal) = local.try_lock() {
                    let _ = terminal.stop();
                }
            }
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
    #[test]
    fn terminal_resize_does_not_wait_for_local_close_lock_and_retries() {
        let mut tab = Tab::new("試験".into());
        let (tx, _rx) = mpsc::channel(1);
        let program = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let local = Arc::new(Mutex::new(
            LocalTerminal::spawn(program, &[], None, tab.stamp.clone(), tx).unwrap(),
        ));
        tab.local = Some(local.clone());
        let context = egui::Context::default();
        let show = |tab: &mut Tab| {
            let mut output =
                context.run_ui(egui::RawInput::default(), |ui| terminal_view::show(ui, tab));
            // この試験はrendererを持たず、texture差分は意図して破棄する。
            output.textures_delta.clear();
        };
        // closeが保持する同じmutexを取得済みでも、GUI frameは戻る。
        let lock = local.lock().unwrap();
        show(&mut tab);
        assert!(tab.pending_local_size.is_some());
        drop(lock);
        show(&mut tab);
        assert!(
            tab.pending_local_size.is_none(),
            "競合で保留したサイズを次frameで送る"
        );
        tab.closing = true;
        let lock = local.lock().unwrap();
        show(&mut tab);
        drop(lock);
        local.lock().unwrap().close().unwrap();
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
