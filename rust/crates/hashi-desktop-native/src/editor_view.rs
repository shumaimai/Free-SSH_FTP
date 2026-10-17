use super::*;
use hashi_core::editor::{self, Body, Document};
use std::path::PathBuf;

#[derive(Clone)]
pub(super) enum EditLocation {
    Local(PathBuf),
    Remote {
        session: Arc<SshSession>,
        work: Work,
        cancel: CancellationToken,
        xfer: Sftp,
        path: String,
    },
}
impl EditLocation {
    fn label(&self) -> String {
        match self {
            Self::Local(path) => path.to_string_lossy().into_owned(),
            Self::Remote { path, .. } => path.clone(),
        }
    }
}
pub(super) struct Editor {
    id: String,
    location: EditLocation,
    document: Document,
    saving: bool,
    open: bool,
    discard: bool,
    query: String,
    replacement: String,
    found: Option<std::ops::Range<usize>>,
    selected_byte: usize,
    hex: String,
    status: String,
}
pub(super) enum EditorEvent {
    Loaded {
        id: String,
        location: EditLocation,
        document: Document,
    },
    Failed {
        id: String,
        error: String,
    },
    Saved {
        id: String,
        payload: Vec<u8>,
        result: Result<(), String>,
        location: Option<EditLocation>,
    },
}
impl Editor {
    fn new(id: String, location: EditLocation, document: Document) -> Self {
        Self {
            id,
            location,
            document,
            saving: false,
            open: true,
            discard: false,
            query: String::new(),
            replacement: String::new(),
            found: None,
            selected_byte: 0,
            hex: String::new(),
            status: String::new(),
        }
    }
}
impl App {
    pub(super) fn editor_busy(&self) -> bool {
        self.editor_jobs > 0 || self.editors.iter().any(|editor| editor.saving)
    }
    pub(super) fn has_unsaved_edits(&self) -> bool {
        self.editors.iter().any(|editor| editor.document.dirty())
    }
    pub(super) fn process_editors(&mut self) {
        while let Ok(event) = self.editor_events.try_recv() {
            match event {
                EditorEvent::Loaded {
                    id,
                    location,
                    document,
                } => {
                    self.editor_jobs = self.editor_jobs.saturating_sub(1);
                    self.editors.push(Editor::new(id, location, document));
                }
                EditorEvent::Failed { id, error } => {
                    self.editor_jobs = self.editor_jobs.saturating_sub(1);
                    let _ = id;
                    self.notice = format!("読込失敗: {error}");
                    if self.exiting {
                        self.close_errors.push(self.notice.clone());
                    }
                }
                EditorEvent::Saved {
                    id,
                    payload,
                    result,
                    location,
                } => {
                    if self.exiting {
                        if let Err(error) = &result {
                            self.close_errors
                                .push(format!("編集ファイルの保存結果を確認してください: {error}"));
                        }
                    }
                    if let Some(editor) = self.editors.iter_mut().find(|e| e.id == id) {
                        editor.saving = false;
                        editor.status = match result {
                            Ok(()) => {
                                if let Some(location) = location {
                                    editor.location = location;
                                }
                                match editor.document.mark_saved(&payload) {
                                    Ok(()) => "保存しました".into(),
                                    Err(error) => error.to_string(),
                                }
                            }
                            Err(error) => format!("保存失敗（編集内容を保持しています）: {error}"),
                        };
                    }
                }
            }
        }
    }
    pub(super) fn open_local_editor(&mut self) {
        if self.exiting {
            return;
        }
        if self.editors.len() + self.editor_jobs >= 8 {
            self.notice = "エディタは8個までです".into();
            return;
        }
        let Some(path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.load_local_editor(path);
    }
    pub(super) fn load_local_editor(&mut self, path: PathBuf) {
        if self.exiting {
            return;
        }
        let tx = self.editor_tx.clone();
        let id = Stamp::default().terminal_id;
        self.editor_jobs += 1;
        self.work
            .spawn_blocking(self.runtime.handle(), move || {
                let event = match editor::read_local(&path) {
                    Ok(document) => EditorEvent::Loaded {
                        id,
                        location: EditLocation::Local(path),
                        document,
                    },
                    Err(error) => EditorEvent::Failed {
                        id,
                        error: error.to_string(),
                    },
                };
                let _ = tx.blocking_send(event);
            })
            .expect("読込の受付確認済み");
    }
    pub(super) fn open_remote_editor(&mut self) {
        if self.exiting {
            return;
        }
        if self.editors.len() + self.editor_jobs >= 8 {
            self.notice = "エディタは8個までです".into();
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.closing || tab.work.is_closed() {
            return;
        }
        let work = tab.work.clone();
        let location_work = work.clone();
        let cancel = tab.work_cancel.clone();
        let (Some(session), Some(xfer), Some(index)) =
            (tab.session.clone(), tab.xfer.clone(), tab.selected)
        else {
            return;
        };
        let Some(entry) = tab.entries.get(index) else {
            return;
        };
        if entry.directory {
            return;
        }
        let path = entry.path.clone();
        let tx = self.editor_tx.clone();
        let id = Stamp::default().terminal_id;
        self.editor_jobs += 1;
        work.spawn(self.runtime.handle(), async move {
            let result = async {
                let sftp = xfer.lock().await;
                let path = sftp.canonicalize(&path).await?;
                let doc = editor::read_remote(&sftp, &path, &cancel).await?;
                anyhow::Ok((path, doc))
            }
            .await;
            let event = match result {
                Ok((path, document)) => EditorEvent::Loaded {
                    id,
                    location: EditLocation::Remote {
                        work: location_work,
                        cancel,
                        session,
                        xfer,
                        path,
                    },
                    document,
                },
                Err(error) => EditorEvent::Failed {
                    id,
                    error: error.to_string(),
                },
            };
            let _ = tx.send(event).await;
        })
        .expect("接続の受付確認済み");
    }
    fn save_editor(&mut self, index: usize) {
        if self.exiting {
            return;
        }
        let editor = &mut self.editors[index];
        if editor.saving {
            return;
        }
        let payload = match editor.document.payload() {
            Ok(payload) => payload,
            Err(error) => {
                editor.status = error.to_string();
                return;
            }
        };
        let original = editor.document.original().to_vec();
        let location = editor.location.clone();
        let work = match &location {
            EditLocation::Local(_) => self.work.clone(),
            EditLocation::Remote { work, .. } => work.clone(),
        };
        if work.is_closed() {
            editor.status = "接続は終了しています。名前を付けてローカルへ保存できます".into();
            return;
        }
        let id = editor.id.clone();
        let tx = self.editor_tx.clone();
        editor.saving = true;
        work.spawn(self.runtime.handle(), async move {
            let result = match location {
                EditLocation::Local(path) => {
                    let bytes = payload.clone();
                    tokio::task::spawn_blocking(move || {
                        editor::save_local(&path, &original, &bytes)
                    })
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result)
                }
                EditLocation::Remote {
                    session,
                    xfer,
                    path,
                    cancel,
                    ..
                } => {
                    let sftp = xfer.lock().await;
                    editor::save_remote(&session, &sftp, &path, &original, &payload, &cancel).await
                }
            };
            let _ = tx
                .send(EditorEvent::Saved {
                    id,
                    payload,
                    result: result.map_err(|error| error.to_string()),
                    location: None,
                })
                .await;
        })
        .expect("保存の受付確認済み");
    }
    fn save_editor_as(&mut self, index: usize) {
        if self.exiting {
            return;
        }
        if self.editors[index].saving {
            return;
        }
        let Some(path) = rfd::FileDialog::new().save_file() else {
            return;
        };
        let editor = &mut self.editors[index];
        let payload = match editor.document.payload() {
            Ok(bytes) => bytes,
            Err(error) => {
                editor.status = error.to_string();
                return;
            }
        };
        let id = editor.id.clone();
        let tx = self.editor_tx.clone();
        editor.saving = true;
        self.work
            .spawn(self.runtime.handle(), async move {
                let bytes = payload.clone();
                let target = path.clone();
                let result =
                    tokio::task::spawn_blocking(move || editor::save_as_local(&target, &bytes))
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|result| result);
                let _ = tx
                    .send(EditorEvent::Saved {
                        id,
                        payload,
                        result: result.map_err(|error| error.to_string()),
                        location: Some(EditLocation::Local(path)),
                    })
                    .await;
            })
            .expect("別名保存の受付確認済み");
    }
    pub(super) fn editor_windows(&mut self, ctx: &egui::Context) {
        let mut save = None;
        let mut save_as = None;
        for (index, editor) in self.editors.iter_mut().enumerate() {
            let id = egui::Id::new(("editor", &editor.id));
            let save_shortcut = ctx.input(|input|input.events.iter().any(|event|matches!(event,egui::Event::Key {key:egui::Key::S,pressed:true,modifiers,..} if modifiers.ctrl && !modifiers.shift && !modifiers.alt)));
            let title = format!(
                "{}{}",
                if editor.document.dirty() { "* " } else { "" },
                editor.location.label()
            );
            egui::Window::new(title)
                .id(id)
                .open(&mut editor.open)
                .default_size([850.0, 600.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!editor.saving, egui::Button::new("保存 (Ctrl+S)"))
                            .clicked()
                        {
                            save = Some(index);
                        }
                        if ui
                            .add_enabled(!editor.saving, egui::Button::new("名前を付けて保存"))
                            .clicked()
                        {
                            save_as = Some(index);
                        }
                        ui.label(format!(
                            "{:?} / {:?}",
                            editor.document.encoding, editor.document.newline
                        ));
                        if editor.saving {
                            ui.label("保存中");
                        }
                    });
                    let mut selection = None;
                    let language =
                        hashi_core::editor_language::language_for(&editor.location.label());
                    if matches!(editor.document.body, Body::Text(_)) {
                        ui.horizontal(|ui| {
                            ui.label("検索");
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.query).desired_width(170.0),
                            );
                            let next = ui.button("次へ").clicked();
                            let previous = ui.button("前へ").clicked();
                            if next || previous {
                                let start = editor
                                    .found
                                    .as_ref()
                                    .map(|range| if next { range.end } else { range.start })
                                    .unwrap_or(0);
                                editor.found = editor.document.find(&editor.query, start, next);
                                selection = editor.found.clone();
                            }
                            ui.label("置換");
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.replacement)
                                    .desired_width(170.0),
                            );
                            if ui
                                .add_enabled(!editor.saving, egui::Button::new("全置換"))
                                .clicked()
                            {
                                editor.status = match editor
                                    .document
                                    .replace_all(&editor.query, &editor.replacement)
                                {
                                    Ok(count) => format!("{count}件を置換しました"),
                                    Err(error) => error.to_string(),
                                };
                            }
                        });
                    }
                    ui.add_enabled_ui(!editor.saving, |ui| match &mut editor.document.body {
                        Body::Text(text) => {
                            let edit_id = id.with("text");
                            if let Some(range) = selection {
                                if let Some(mut state) =
                                    egui::text_edit::TextEditState::load(ctx, edit_id)
                                {
                                    let start = text[..range.start].chars().count();
                                    let end = text[..range.end].chars().count();
                                    state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(start),
                                            egui::text::CCursor::new(end),
                                        ),
                                    ));
                                    state.store(ctx, edit_id);
                                    ctx.memory_mut(|m| m.request_focus(edit_id));
                                }
                            }
                            egui::ScrollArea::both()
                                .id_salt(id.with("scroll"))
                                .show(ui, |ui| {
                                    let mut layouter =
                                        |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                                            let text = text.as_str();
                                            let mut job = egui::text::LayoutJob::default();
                                            job.wrap.max_width = width;
                                            for span in hashi_core::editor_language::highlight(
                                                text, &language,
                                            ) {
                                                use hashi_core::editor_language::Kind;
                                                let color = match span.kind {
                                                    Kind::Plain => ui.visuals().text_color(),
                                                    Kind::Keyword => style::EDITOR_KEYWORD,
                                                    Kind::String => style::EDITOR_STRING,
                                                    Kind::Comment => style::EDITOR_COMMENT,
                                                    Kind::Number => style::EDITOR_NUMBER,
                                                };
                                                job.append(
                                                    &text[span.range],
                                                    0.0,
                                                    egui::TextFormat {
                                                        font_id: egui::FontId::monospace(
                                                            style::EDITOR_FONT_SIZE,
                                                        ),
                                                        color,
                                                        ..Default::default()
                                                    },
                                                );
                                            }
                                            ui.fonts_mut(|fonts| fonts.layout_job(job))
                                        };
                                    let output = egui::TextEdit::multiline(text)
                                        .code_editor()
                                        .layouter(&mut layouter)
                                        .id(edit_id)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(24)
                                        .char_limit(editor::MAX_BYTES)
                                        .show(ui);
                                    if output.response.has_focus() && save_shortcut {
                                        save = Some(index);
                                    }
                                });
                        }
                        Body::Hex(bytes) => {
                            ui.label("16進編集: 上書きのみ。長さは変更できません。");
                            ui.horizontal(|ui| {
                                ui.label(format!("offset {:08X}", editor.selected_byte));
                                ui.add(
                                    egui::TextEdit::singleline(&mut editor.hex)
                                        .desired_width(40.0)
                                        .char_limit(2),
                                );
                                if ui.button("値を適用").clicked() {
                                    match u8::from_str_radix(&editor.hex, 16) {
                                        Ok(value) if editor.selected_byte < bytes.len() => {
                                            bytes[editor.selected_byte] = value
                                        }
                                        _ => editor.status = "2桁の16進数を指定してください".into(),
                                    };
                                }
                            });
                            let rows = bytes.len().div_ceil(16);
                            egui::ScrollArea::vertical()
                                .id_salt(id.with("hex"))
                                .show_rows(ui, 20.0, rows, |ui, range| {
                                    for row in range {
                                        ui.horizontal(|ui| {
                                            ui.monospace(format!("{:08X}", row * 16));
                                            for (offset, value) in bytes
                                                .iter()
                                                .enumerate()
                                                .take(((row + 1) * 16).min(bytes.len()))
                                                .skip(row * 16)
                                            {
                                                if ui
                                                    .selectable_label(
                                                        editor.selected_byte == offset,
                                                        format!("{value:02X}"),
                                                    )
                                                    .clicked()
                                                {
                                                    editor.selected_byte = offset;
                                                    editor.hex = format!("{value:02X}");
                                                }
                                            }
                                            let ascii: String = bytes
                                                [row * 16..((row + 1) * 16).min(bytes.len())]
                                                .iter()
                                                .map(|b| {
                                                    if b.is_ascii_graphic() {
                                                        char::from(*b)
                                                    } else {
                                                        '.'
                                                    }
                                                })
                                                .collect();
                                            ui.monospace(ascii);
                                        });
                                    }
                                });
                        }
                    });
                    ui.label(&editor.status);
                });
            if !editor.open && (editor.document.dirty() || editor.saving) {
                editor.open = true;
                editor.discard = true;
            }
            if editor.discard {
                egui::Window::new("未保存の変更")
                    .id(id.with("discard"))
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.label("未保存の変更があります。");
                        if ui
                            .add_enabled(!editor.saving, egui::Button::new("破棄して閉じる"))
                            .clicked()
                        {
                            editor.open = false;
                            editor.discard = false;
                        }
                        if ui.button("編集に戻る").clicked() {
                            editor.discard = false;
                        }
                    });
            }
        }
        if let Some(index) = save {
            self.save_editor(index);
        }
        if let Some(index) = save_as {
            self.save_editor_as(index);
        }
        self.editors.retain(|editor| editor.open);
    }
}
