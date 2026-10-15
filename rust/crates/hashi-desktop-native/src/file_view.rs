use super::*;
use crate::theme;
use hashi_core::transfer_tree::{self, Existing, Report, TaskContext};
use std::path::PathBuf;

#[derive(Clone)]
enum Operation {
    Upload { local: PathBuf, remote: String },
    Download { remote: String, local: PathBuf },
    Delete { remote: String },
    Search { root: String, query: String },
}
#[derive(PartialEq)]
enum Phase {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}
struct Job {
    id: String,
    stamp: Stamp,
    label: String,
    operation: Operation,
    policy: Existing,
    mode: transfer::Mode,
    cancel: CancellationToken,
    phase: Phase,
    done: u64,
    total: u64,
    message: String,
    navigation: Stamp,
}
pub(super) struct Browser {
    pub open: bool,
    local_path: PathBuf,
    local_entries: Vec<Entry>,
    local_selected: Option<usize>,
    local_revision: Stamp,
    hidden: bool,
    policy: Existing,
    mode: transfer::Mode,
    search: String,
    delete_confirmation: String,
    delete_target: Option<(Stamp, String)>,
    jobs: VecDeque<Job>,
    drop_area: Option<(egui::Rect, egui::LayerId)>,
    pub(super) sync: super::sync_view::SyncState,
}
impl Browser {
    pub(super) fn local_path(&self) -> &std::path::Path {
        &self.local_path
    }
}
impl Default for Browser {
    fn default() -> Self {
        Self {
            open: false,
            local_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            local_entries: vec![],
            local_selected: None,
            local_revision: Stamp::default(),
            hidden: false,
            policy: Existing::Refuse,
            mode: transfer::Mode::Atomic,
            search: String::new(),
            delete_confirmation: String::new(),
            delete_target: None,
            jobs: VecDeque::new(),
            drop_area: None,
            sync: Default::default(),
        }
    }
}
pub(super) enum FileEvent {
    Local {
        revision: Stamp,
        path: PathBuf,
        result: Result<Vec<Entry>, String>,
    },
    Finished {
        id: String,
        result: Result<Outcome, String>,
    },
}
pub(super) enum Outcome {
    Transfer(Report),
    Deleted,
    Search {
        navigation: Stamp,
        entries: Vec<Entry>,
    },
}

impl App {
    fn transfer_mode_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::muted("書き込み方式"));
        theme::segmented(
            ui,
            &mut self.browser.mode,
            &[
                (transfer::Mode::Atomic, "原本を守る"),
                (transfer::Mode::InPlace, "直接書込み（Python互換）"),
            ],
        );
        if self.browser.mode == transfer::Mode::InPlace {
            ui.colored_label(
                theme::WARN,
                "直接書込みは取消・失敗でも転送先が変更され、部分データが残る場合があります。",
            );
        } else {
            ui.label(theme::muted(
                "原本を守る方式には、親フォルダへの書込み権限とサーバーの対応が必要です。",
            ));
        }
        if self.browser.policy == Existing::Resume {
            ui.label(theme::muted(
                "続きからは内容を照合して、途中のファイルへ直接追記します。",
            ));
        }
    }
    pub(super) fn refresh_local(&mut self, path: PathBuf) -> Option<Stamp> {
        if self.exiting {
            return None;
        }
        self.browser.local_revision = Stamp::default();
        let revision = self.browser.local_revision.clone();
        let tx = self.file_tx.clone();
        self.work
            .spawn(self.runtime.handle(), async move {
                let result = transfer_tree::list_local(&path)
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx
                    .send(FileEvent::Local {
                        revision,
                        path,
                        result,
                    })
                    .await;
            })
            .expect("一覧の受付確認済み");
        Some(self.browser.local_revision.clone())
    }
    pub(super) fn process_files(&mut self) {
        let mut refresh = vec![];
        while let Ok(event) = self.file_events.try_recv() {
            match event {
                FileEvent::Local {
                    revision,
                    path,
                    result,
                } if revision == self.browser.local_revision => match result {
                    Ok(entries) => {
                        let follow = self.browser.sync.on_local(&revision, &path);
                        self.browser.local_path = path;
                        self.browser.local_entries = entries;
                        self.browser.local_selected = None;
                        self.apply_follow(follow);
                    }
                    Err(error) => {
                        self.notice = self.browser.sync.local_failed(&revision).unwrap_or(error)
                    }
                },
                FileEvent::Local { .. } => {}
                FileEvent::Finished { id, result } => {
                    if let Some(job) = self.browser.jobs.iter_mut().find(|job| job.id == id) {
                        match result {
                            Ok(Outcome::Transfer(report)) => {
                                refresh.push(job.stamp.clone());
                                job.phase = Phase::Done;
                                job.message = format!(
                                    "{}ファイル / {}フォルダ / {}スキップ",
                                    report.files, report.directories, report.skipped
                                );
                            }
                            Ok(Outcome::Deleted) => {
                                refresh.push(job.stamp.clone());
                                job.phase = Phase::Done;
                                job.message = "削除しました".into();
                            }
                            Ok(Outcome::Search {
                                navigation,
                                entries,
                            }) => {
                                job.phase = Phase::Done;
                                job.message = format!("{}件", entries.len());
                                if let Some(tab) =
                                    self.tabs.iter_mut().find(|t| t.stamp == job.stamp)
                                {
                                    let path = tab.path.clone();
                                    tab.apply_listing(navigation, path, entries);
                                }
                            }
                            Err(error) => {
                                if self.exiting {
                                    self.close_errors.push(format!("{}: {error}", job.label));
                                }
                                job.phase = if job.cancel.is_cancelled() {
                                    Phase::Cancelled
                                } else {
                                    Phase::Failed
                                };
                                job.message = error;
                            }
                        }
                    }
                }
            }
        }
        for stamp in refresh {
            if let Some(index) = self.tabs.iter().position(|tab| tab.stamp == stamp) {
                let path = self.tabs[index].path.clone();
                self.browse(index, path);
            }
        }
        for index in 0..self.browser.jobs.len() {
            if self.browser.jobs[index].phase != Phase::Queued {
                continue;
            }
            if self.browser.jobs[index].cancel.is_cancelled() {
                self.browser.jobs[index].phase = Phase::Cancelled;
                continue;
            }
            let stamp = self.browser.jobs[index].stamp.clone();
            if self
                .browser
                .jobs
                .iter()
                .any(|job| job.stamp == stamp && job.phase == Phase::Running)
            {
                continue;
            }
            if self
                .browser
                .jobs
                .iter()
                .filter(|job| job.phase == Phase::Running)
                .count()
                >= 4
            {
                break;
            }
            let Some(tab) = self.tabs.iter().find(|tab| tab.stamp == stamp) else {
                self.browser.jobs[index].phase = Phase::Cancelled;
                continue;
            };
            if tab.closing || tab.work.is_closed() {
                self.browser.jobs[index].phase = Phase::Cancelled;
                continue;
            }
            let work = tab.work.clone();
            let (Some(session), Some(xfer), Some(nav)) =
                (tab.session.clone(), tab.xfer.clone(), tab.nav.clone())
            else {
                continue;
            };
            let job = &mut self.browser.jobs[index];
            job.phase = Phase::Running;
            let operation = job.operation.clone();
            let policy = job.policy;
            let mode = job.mode;
            let id = job.id.clone();
            let cancel = job.cancel.clone();
            let navigation = job.navigation.clone();
            let permissions = tab.permissions.clone();
            let progress = self.progress_tx.clone();
            let tx = self.file_tx.clone();
            work.spawn(self.runtime.handle(), async move {
                let context = TaskContext {
                    job: &id,
                    progress: &progress,
                    cancel: &cancel,
                    permission: permissions.as_ref(),
                    mode,
                };
                let future = async {
                    match operation {
                        Operation::Upload { local, remote } => {
                            let sftp = xfer.lock().await;
                            transfer_tree::upload_tree(
                                &session, &sftp, &local, &remote, policy, &context,
                            )
                            .await
                            .map(Outcome::Transfer)
                        }
                        Operation::Download { remote, local } => {
                            let sftp = xfer.lock().await;
                            transfer_tree::download_tree(
                                &session, &sftp, &remote, &local, policy, &context,
                            )
                            .await
                            .map(Outcome::Transfer)
                        }
                        Operation::Delete { remote } => {
                            let sftp = nav.lock().await;
                            transfer::delete_tree(&sftp, &remote, "delete", &cancel)
                                .await
                                .map(|_| Outcome::Deleted)
                        }
                        Operation::Search { root, query } => {
                            let sftp = nav.lock().await;
                            transfer_tree::search(&session, &sftp, &root, &query, &cancel)
                                .await
                                .map(|entries| Outcome::Search {
                                    navigation,
                                    entries,
                                })
                        }
                    }
                };
                // tokenを各操作へ渡す。確定ACK/権限復元中のFutureを取消で捨てない。
                let result = future.await.map_err(|e| e.to_string());
                let _ = tx.send(FileEvent::Finished { id, result }).await;
            })
            .expect("転送の受付確認済み");
        }
    }
    fn queue_file(&mut self, operation: Operation, label: String) {
        if self.exiting {
            return;
        }
        if self.browser.jobs.len() >= 128 {
            self.browser
                .jobs
                .retain(|job| matches!(job.phase, Phase::Queued | Phase::Running));
        }
        if self.browser.jobs.len() >= 128 {
            self.notice = "転送待ち行列が満杯です".into();
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.closing || tab.work.is_closed() || tab.session.is_none() || tab.cancel.is_cancelled()
        {
            return;
        }
        let cancel = if matches!(operation, Operation::Search { .. }) {
            tab.navigation_cancel.child_token()
        } else {
            tab.work_cancel.child_token()
        };
        let job = Job {
            id: Stamp::default().terminal_id,
            stamp: tab.stamp.clone(),
            label,
            operation,
            policy: self.browser.policy,
            mode: self.browser.mode,
            cancel,
            phase: Phase::Queued,
            done: 0,
            total: 0,
            message: String::new(),
            navigation: tab.navigation.clone(),
        };
        self.browser.jobs.push_back(job);
    }
    pub(super) fn transfer(&mut self, upload: bool) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let path = tab.path.clone();
        let selected = tab.selected.and_then(|i| tab.entries.get(i)).cloned();
        if upload {
            if let Some(local) = rfd::FileDialog::new().pick_file() {
                let name = local
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let remote = format!("{}/{}", path.trim_end_matches('/'), name);
                self.queue_file(Operation::Upload { local, remote }, format!("送信 {name}"));
            }
        } else if let Some(selected) = selected {
            let local = if selected.directory {
                rfd::FileDialog::new()
                    .pick_folder()
                    .map(|path| path.join(&selected.name))
            } else {
                rfd::FileDialog::new()
                    .set_file_name(&selected.name)
                    .save_file()
            };
            if let Some(local) = local {
                self.queue_file(
                    Operation::Download {
                        remote: selected.path,
                        local,
                    },
                    format!("受信 {}", selected.name),
                );
            }
        }
    }
    pub(super) fn transfer_directory(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let path = tab.path.clone();
        if let Some(local) = rfd::FileDialog::new().pick_folder() {
            let name = local
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let remote = format!("{}/{}", path.trim_end_matches('/'), name);
            self.queue_file(Operation::Upload { local, remote }, format!("送信 {name}"));
        }
    }
    pub(super) fn file_progress(&mut self, progress: &Progress) {
        if let Some(job) = self
            .browser
            .jobs
            .iter_mut()
            .find(|job| job.id == progress.job_id)
        {
            job.done = progress.done;
            job.total = progress.total;
        }
    }
    fn remote_rows(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            ui.label("SSH接続を選択してください");
            return;
        };
        let hidden = self.browser.hidden;
        let indices: Vec<usize> = tab
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| hidden || !entry.name.starts_with('.'))
            .map(|(i, _)| i)
            .collect();
        let mut enter = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 28.0, indices.len(), |ui, rows| {
                for row in rows {
                    let index = indices[row];
                    let entry = &tab.entries[index];
                    let selected = tab.selected == Some(index);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 26.0),
                        egui::Sense::click(),
                    );
                    if selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(6),
                            if selected {
                                theme::ACCENT.gamma_multiply(0.22)
                            } else {
                                theme::CARD
                            },
                        );
                    }
                    let name_color = if entry.directory {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    };
                    let mut name = entry.name.clone();
                    if entry.directory {
                        name.push('/');
                    } else if entry.symlink {
                        name.push_str("  (リンク)");
                    }
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    ui.painter().text(
                        rect.left_center() + egui::vec2(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        font.clone(),
                        name_color,
                    );
                    if !entry.directory {
                        ui.painter().text(
                            rect.right_center() - egui::vec2(10.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            human_size(entry.size),
                            egui::TextStyle::Small.resolve(ui.style()),
                            theme::MUTED,
                        );
                    }
                    if response.clicked() {
                        tab.selected = Some(index);
                    }
                    if response.double_clicked() && entry.directory {
                        enter = Some(entry.path.clone());
                    }
                }
            });
        if let Some(path) = enter {
            self.browse(self.active, path);
        }
    }
    pub(super) fn remote_file_panel(&mut self, ui: &mut egui::Ui) {
        self.browser.drop_area = Some((ui.max_rect(), ui.layer_id()));
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let path = tab.path.clone();
        ui.horizontal(|ui| {
            if ui.button("親へ").clicked() {
                self.browse(self.active, format!("{path}/.."));
            }
            if ui.button("更新").clicked() {
                self.browse(self.active, path.clone());
            }
            ui.add(
                egui::Label::new(egui::RichText::new(&path).strong())
                    .truncate()
                    .selectable(false),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.add(theme::primary("受信")).clicked() {
                self.transfer(false);
            }
            if ui.button("送信").clicked() {
                self.transfer(true);
            }
            if ui.button("フォルダ送信").clicked() {
                self.transfer_directory();
            }
            if ui.button("編集").clicked() {
                self.open_remote_editor();
            }
            if ui.add(theme::danger("削除")).clicked() {
                if let Some(tab) = self.tabs.get(self.active) {
                    if let Some(entry) = tab.selected.and_then(|i| tab.entries.get(i)) {
                        self.browser.delete_target = Some((tab.stamp.clone(), entry.path.clone()));
                        self.browser.delete_confirmation.clear();
                    }
                }
            }
        });
        egui::CollapsingHeader::new("転送の設定").show(ui, |ui| {
            ui.label(theme::muted("すでに同名のファイルがあるとき"));
            theme::segmented(
                ui,
                &mut self.browser.policy,
                &[
                    (Existing::Refuse, "中止"),
                    (Existing::Overwrite, "上書き"),
                    (Existing::Resume, "続きから"),
                    (Existing::Skip, "飛ばす"),
                ],
            );
            self.transfer_mode_ui(ui);
            if self.browser.policy == Existing::Overwrite
                && self.browser.mode == transfer::Mode::Atomic
            {
                ui.colored_label(
                    theme::WARN,
                    "確定前の取消・失敗では元ファイルを保持します。確定開始後の通信失敗は結果不明になる場合があります。",
                );
            }
        });
        let profile = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.session.as_ref())
            .map(|session| {
                format!(
                    "{}@{}:{}",
                    session.scope.username, session.scope.host, session.scope.port
                )
            });
        if let Some(profile) = profile {
            let paths = self
                .snapshot
                .settings
                .get("bookmarks")
                .and_then(|value| value.get(&profile))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            ui.menu_button("ブックマーク", |ui| {
                if ui.button("現在のフォルダを登録").clicked() {
                    let path = path.clone();
                    let key = profile.clone();
                    match self.store.transact(|snapshot| {
                        let bookmarks = snapshot
                            .settings
                            .entry("bookmarks")
                            .or_insert_with(|| serde_json::json!({}))
                            .as_object_mut()
                            .ok_or_else(|| std::io::Error::other("ブックマークの形式が不正です"))?;
                        let entries = bookmarks
                            .entry(key)
                            .or_insert_with(|| serde_json::json!([]))
                            .as_array_mut()
                            .ok_or_else(|| std::io::Error::other("ブックマークの形式が不正です"))?;
                        if !entries.iter().any(|value| value.as_str() == Some(&path)) {
                            if entries.len() >= 128 {
                                return Err(std::io::Error::other("ブックマークは128件までです"));
                            }
                            entries.push(serde_json::json!(path));
                        }
                        Ok(())
                    }) {
                        Ok(snapshot) => self.snapshot = snapshot,
                        Err(error) => self.notice = error.to_string(),
                    };
                    ui.close();
                }
                for value in &paths {
                    if let Some(path) = value.as_str() {
                        if ui.button(path).clicked() {
                            self.browse(self.active, path.to_owned());
                            ui.close();
                        }
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.browser.search)
                    .hint_text("このフォルダ以下を検索")
                    .desired_width(ui.available_width() - 70.0),
            );
            if ui.button("検索").clicked() {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.begin_navigation();
                }
                let query = self.browser.search.clone();
                self.queue_file(
                    Operation::Search {
                        root: path.clone(),
                        query,
                    },
                    "検索".into(),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.browser.hidden, "隠しファイルを表示");
            ui.label(theme::muted("ファイルをここへドロップして送信"));
        });
        self.remote_rows(ui);
    }
    pub(super) fn reset_file_drop(&mut self) {
        self.browser.drop_area = None;
    }
    pub(super) fn dropped_files(&mut self, ctx: &egui::Context) {
        let Some((area, layer)) = self.browser.drop_area else {
            return;
        };
        let Some(position) = ctx.input(|input| input.pointer.latest_pos()) else {
            return;
        };
        if !area.contains(position) || ctx.layer_id_at(position) != Some(layer) {
            return;
        }
        let paths = ctx
            .input_mut(|input| std::mem::take(&mut input.raw.dropped_files))
            .iter()
            .map(|file| file.path().to_path_buf())
            .collect::<Vec<_>>();
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let remote_root = tab.path.clone();
        for local in paths {
            if !local.is_absolute() {
                self.notice = "ドロップ元は絶対パスを指定してください".into();
                continue;
            }
            let Some(name) = local
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                self.notice = "ファイル名がUTF-8ではありません".into();
                continue;
            };
            let remote = format!("{}/{}", remote_root.trim_end_matches('/'), name);
            self.queue_file(Operation::Upload { local, remote }, format!("送信 {name}"));
        }
    }
    pub(super) fn file_windows(&mut self, ctx: &egui::Context) {
        let mut open = self.browser.open;
        if open {
            egui::Window::new("ファイル・転送")
                .open(&mut open)
                .default_size([1000.0, 650.0])
                .show(ctx, |ui| {
                    self.transfer_mode_ui(ui);
                    let mut sync = self.browser.sync.enabled;
                    if ui
                        .checkbox(
                            &mut sync,
                            "同期ブラウズ（片側のフォルダ移動を相手側へ相対的に写す）",
                        )
                        .changed()
                    {
                        self.set_sync_browse(sync);
                    }
                    ui.horizontal(|ui| {
                        ui.label("既存ファイル");
                        for (policy, label) in [
                            (Existing::Refuse, "上書きしない"),
                            (Existing::Overwrite, "上書き"),
                            (Existing::Resume, "再開"),
                            (Existing::Skip, "スキップ"),
                        ] {
                            ui.radio_value(&mut self.browser.policy, policy, label);
                        }
                    });
                    ui.columns(2, |columns| {
                        columns[0].heading("ローカル");
                        columns[0].label(self.browser.local_path.to_string_lossy());
                        if columns[0].button("フォルダを選択").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.refresh_local(path);
                            }
                        }
                        if columns[0].button("更新").clicked() {
                            self.refresh_local(self.browser.local_path.clone());
                        }
                        let mut enter = None;
                        let mut send = None;
                        let indices: Vec<_> = self
                            .browser
                            .local_entries
                            .iter()
                            .enumerate()
                            .filter(|(_, entry)| {
                                self.browser.hidden || !entry.name.starts_with('.')
                            })
                            .map(|(i, _)| i)
                            .collect();
                        egui::ScrollArea::vertical()
                            .id_salt("local-files")
                            .show_rows(&mut columns[0], 22.0, indices.len(), |ui, rows| {
                                for row in rows {
                                    let index = indices[row];
                                    let entry = &self.browser.local_entries[index];
                                    let response = ui.selectable_label(
                                        self.browser.local_selected == Some(index),
                                        format!(
                                            "{} {}",
                                            if entry.directory { "[dir]" } else { "[file]" },
                                            entry.name
                                        ),
                                    );
                                    if response.clicked() {
                                        self.browser.local_selected = Some(index);
                                    }
                                    if response.double_clicked() && entry.directory {
                                        enter = Some(PathBuf::from(&entry.path));
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button("リモートへ送信").clicked() {
                                            send = Some((
                                                PathBuf::from(&entry.path),
                                                entry.name.clone(),
                                            ));
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        if let Some(path) = enter {
                            self.refresh_local(path);
                        }
                        if let Some((local, name)) = send {
                            if let Some(tab) = self.tabs.get(self.active) {
                                let remote = format!("{}/{}", tab.path.trim_end_matches('/'), name);
                                self.queue_file(
                                    Operation::Upload { local, remote },
                                    format!("送信 {name}"),
                                );
                            }
                        }
                        columns[1].heading("リモート");
                        self.remote_file_panel(&mut columns[1]);
                    });
                    ui.separator();
                    ui.label("転送待ち行列");
                    for job in &mut self.browser.jobs {
                        ui.horizontal(|ui| {
                            let phase = match job.phase {
                                Phase::Queued => "待機",
                                Phase::Running => "実行中",
                                Phase::Done => "完了",
                                Phase::Failed => "失敗",
                                Phase::Cancelled => "中止要求済み",
                            };
                            ui.label(format!(
                                "{phase}: {} {} / {} {}",
                                job.label, job.done, job.total, job.message
                            ));
                            if matches!(
                                job.operation,
                                Operation::Upload { .. } | Operation::Download { .. }
                            ) {
                                ui.label(if job.policy == Existing::Resume {
                                    "方式: 直接追記（再開）"
                                } else if job.mode == transfer::Mode::Atomic {
                                    "方式: 原本保持"
                                } else {
                                    "方式: 直接書込み"
                                });
                            }
                            if matches!(job.phase, Phase::Queued | Phase::Running)
                                && ui.button("中止").clicked()
                            {
                                job.cancel.cancel();
                                job.message =
                                    "中止を要求しました。確定応答と権限復元を待っています".into();
                            }
                        });
                    }
                });
        }
        self.browser.open = open;
        if let Some((stamp, path)) = self.browser.delete_target.clone() {
            let mut delete = false;
            let mut close = false;
            egui::Window::new("削除の確認")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("既存ファイル:");
                        for (policy, label) in [
                            (Existing::Refuse, "中止"),
                            (Existing::Overwrite, "上書き"),
                            (Existing::Resume, "再開"),
                            (Existing::Skip, "スキップ"),
                        ] {
                            ui.radio_value(&mut self.browser.policy, policy, label);
                        }
                    });
                    let profile = self
                        .tabs
                        .get(self.active)
                        .and_then(|tab| tab.session.as_ref())
                        .map(|session| {
                            format!(
                                "{}@{}:{}",
                                session.scope.username, session.scope.host, session.scope.port
                            )
                        });
                    if let Some(profile) = profile {
                        let paths = self
                            .snapshot
                            .settings
                            .get("bookmarks")
                            .and_then(|value| value.get(&profile))
                            .and_then(serde_json::Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        ui.menu_button("ブックマーク", |ui| {
                            if ui.button("現在のフォルダを登録").clicked() {
                                let path = path.clone();
                                let key = profile.clone();
                                match self.store.transact(|snapshot| {
                                    let bookmarks = snapshot
                                        .settings
                                        .entry("bookmarks")
                                        .or_insert_with(|| serde_json::json!({}))
                                        .as_object_mut()
                                        .ok_or_else(|| {
                                            std::io::Error::other("ブックマークの形式が不正です")
                                        })?;
                                    let entries = bookmarks
                                        .entry(key)
                                        .or_insert_with(|| serde_json::json!([]))
                                        .as_array_mut()
                                        .ok_or_else(|| {
                                            std::io::Error::other("ブックマークの形式が不正です")
                                        })?;
                                    if !entries.iter().any(|value| value.as_str() == Some(&path)) {
                                        if entries.len() >= 128 {
                                            return Err(std::io::Error::other(
                                                "ブックマークは128件までです",
                                            ));
                                        }
                                        entries.push(serde_json::json!(path));
                                    }
                                    Ok(())
                                }) {
                                    Ok(snapshot) => self.snapshot = snapshot,
                                    Err(error) => self.notice = error.to_string(),
                                };
                                ui.close();
                            }
                            for value in &paths {
                                if let Some(path) = value.as_str() {
                                    if ui.button(path).clicked() {
                                        self.browse(self.active, path.to_owned());
                                        ui.close();
                                    }
                                }
                            }
                        });
                    }
                    ui.label("ファイル・フォルダをここへドロップして送信");
                    ui.label(&path);
                    ui.label("再帰削除を実行するには delete と入力してください");
                    ui.text_edit_singleline(&mut self.browser.delete_confirmation);
                    delete = ui
                        .add_enabled(
                            self.browser.delete_confirmation == "delete",
                            egui::Button::new("削除"),
                        )
                        .clicked();
                    close = ui.button("戻る").clicked();
                });
            if delete {
                if self
                    .tabs
                    .get(self.active)
                    .is_some_and(|tab| tab.stamp == stamp)
                {
                    self.queue_file(Operation::Delete { remote: path }, "削除".into());
                }
                close = true;
            }
            if close {
                self.browser.delete_target = None;
                self.browser.delete_confirmation.clear();
            }
        }
    }
    pub(super) fn cancel_files(&self) {
        for job in &self.browser.jobs {
            job.cancel.cancel();
        }
    }
}

/// 一覧に出すファイルサイズ。1024単位で、小数は1桁。
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod size_tests {
    use super::human_size;
    #[test]
    fn sizes_are_shown_in_1024_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(u64::MAX), "16777216.0 TB");
    }
}
