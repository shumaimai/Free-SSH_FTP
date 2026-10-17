//! スニペット（よく使うコマンド）。保存形式はPython版`snippets.json`と同じ配列。
use super::*;
use hashi_storage::snippets::{self, Snippet};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct SnippetWindow {
    pub(super) open: bool,
    /// 編集中の項目。`None`の位置は新規追加。
    editing: Option<(Option<usize>, Snippet)>,
    /// 変数入力中の項目と値。
    variables: Option<(Snippet, Vec<(String, String)>)>,
}

impl App {
    fn stored_snippets(&self) -> Result<Vec<Snippet>, String> {
        snippets::parse(&self.snapshot.snippets).map_err(|error| error.to_string())
    }
    /// 保存済みの一覧を読み直して変更する。形式不明の記録は上書きしない。
    fn change_snippets(&mut self, change: impl FnOnce(&mut Vec<Snippet>)) {
        if self.exiting || self.startup_error {
            return;
        }
        let result = self.store.transact(|snapshot| {
            let mut list = snippets::parse(&snapshot.snippets)?;
            change(&mut list);
            snapshot.snippets = snippets::to_value(&list);
            Ok(())
        });
        match result {
            Ok(snapshot) => self.snapshot = snapshot,
            Err(error) => self.notice = format!("スニペットを保存できません: {error}"),
        }
    }
    fn send_snippet(&mut self, snippet: &Snippet, values: &HashMap<String, String>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            self.notice = "端末の接続中だけスニペットを送信できます".into();
            return;
        };
        if tab.closing || (tab.input.is_none() && tab.local.is_none()) {
            self.notice = "端末の接続中だけスニペットを送信できます".into();
            return;
        }
        tab.send(snippets::terminal_bytes(snippet, values));
    }
    pub(super) fn snippet_dialog(&mut self, ctx: &egui::Context) {
        if !self.snippets.open {
            return;
        }
        let mut open = true;
        let list = self.stored_snippets();
        let mut action: Option<SnippetAction> = None;
        egui::Window::new("スニペット")
            .default_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let list = match &list {
                    Ok(list) => list,
                    Err(error) => {
                        ui.label(format!("{error}。保存データを変更しません"));
                        return;
                    }
                };
                ui.label("{{変数名}} は送信時に入力します。送信先は選択中の端末です。");
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for (index, snippet) in list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.button("送信").clicked() {
                                    action = Some(SnippetAction::Send(snippet.clone()));
                                }
                                if ui.small_button("編集").clicked() {
                                    action = Some(SnippetAction::Edit(index, snippet.clone()));
                                }
                                if ui.small_button("削除").clicked() {
                                    action = Some(SnippetAction::Remove(index));
                                }
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑").small())
                                    .clicked()
                                {
                                    action = Some(SnippetAction::Swap(index - 1));
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < list.len(),
                                        egui::Button::new("↓").small(),
                                    )
                                    .clicked()
                                {
                                    action = Some(SnippetAction::Swap(index));
                                }
                                ui.label(&snippet.name).on_hover_text(&snippet.body);
                            });
                        }
                    });
                if ui.button("追加").clicked() {
                    self.snippets.editing = Some((None, Snippet::default()));
                }
                if let Some((_, snippet)) = &mut self.snippets.editing {
                    ui.separator();
                    ui.label("名前");
                    ui.text_edit_singleline(&mut snippet.name);
                    ui.label("本文");
                    ui.add(egui::TextEdit::multiline(&mut snippet.body).desired_rows(4));
                    ui.checkbox(&mut snippet.send_enter, "送信後に Enter を押す");
                    ui.horizontal(|ui| {
                        let valid = !snippet.name.trim().is_empty() && !snippet.body.is_empty();
                        if ui.add_enabled(valid, egui::Button::new("保存")).clicked() {
                            action = Some(SnippetAction::Save);
                        }
                        if ui.button("キャンセル").clicked() {
                            action = Some(SnippetAction::CancelEdit);
                        }
                    });
                }
                if let Some((snippet, values)) = &mut self.snippets.variables {
                    ui.separator();
                    ui.label(format!("「{}」の変数を入力してください", snippet.name));
                    for (name, value) in values.iter_mut() {
                        ui.horizontal(|ui| {
                            ui.label(name.as_str());
                            ui.text_edit_singleline(value);
                        });
                    }
                    ui.label(egui::RichText::new(&snippet.body).weak());
                    ui.horizontal(|ui| {
                        if ui.button("送信").clicked() {
                            action = Some(SnippetAction::SendWithValues);
                        }
                        if ui.button("キャンセル").clicked() {
                            action = Some(SnippetAction::CancelVariables);
                        }
                    });
                }
            });
        if !open {
            self.snippets = SnippetWindow::default();
            return;
        }
        match action {
            None => {}
            Some(SnippetAction::Send(snippet)) => {
                let variables = snippets::find_variables(&snippet.body);
                if variables.is_empty() {
                    self.send_snippet(&snippet, &HashMap::new());
                } else {
                    let values = variables.into_iter().map(|name| (name, String::new()));
                    self.snippets.variables = Some((snippet, values.collect()));
                }
            }
            Some(SnippetAction::SendWithValues) => {
                if let Some((snippet, values)) = self.snippets.variables.take() {
                    self.send_snippet(&snippet, &values.into_iter().collect());
                }
            }
            Some(SnippetAction::CancelVariables) => self.snippets.variables = None,
            Some(SnippetAction::Edit(index, snippet)) => {
                self.snippets.editing = Some((Some(index), snippet));
            }
            Some(SnippetAction::CancelEdit) => self.snippets.editing = None,
            Some(SnippetAction::Save) => {
                if let Some((index, snippet)) = self.snippets.editing.take() {
                    self.change_snippets(|list| match index {
                        Some(index) if index < list.len() => list[index] = snippet,
                        _ => list.push(snippet),
                    });
                }
            }
            Some(SnippetAction::Remove(index)) => {
                self.snippets.editing = None;
                self.change_snippets(|list| {
                    if index < list.len() {
                        list.remove(index);
                    }
                });
            }
            Some(SnippetAction::Swap(index)) => {
                self.snippets.editing = None;
                self.change_snippets(|list| {
                    if index + 1 < list.len() {
                        list.swap(index, index + 1);
                    }
                });
            }
        }
    }
}

enum SnippetAction {
    Send(Snippet),
    SendWithValues,
    CancelVariables,
    Edit(usize, Snippet),
    CancelEdit,
    Save,
    Remove(usize),
    Swap(usize),
}
