//! 設定画面。Python版と同じsettingsのキー名で保存する。未移植の項目は値を保持して触らない。
use super::*;
use serde_json::Value;

#[derive(Default)]
pub(super) struct SettingsWindow {
    pub(super) open: bool,
    draft: Option<Draft>,
}
#[derive(Clone, PartialEq)]
struct Draft {
    session_log: bool,
    session_log_dir: String,
}
impl Draft {
    fn from(settings: &serde_json::Map<String, Value>) -> Self {
        Self {
            session_log: settings
                .get("session_log")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            session_log_dir: settings
                .get("session_log_dir")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

impl App {
    /// 空欄なら各版の設定ルート/logs。旧Python版のlogsへは書かない。
    pub(super) fn session_log_directory(&self) -> (Option<std::path::PathBuf>, std::path::PathBuf) {
        let custom = self
            .snapshot
            .settings
            .get("session_log_dir")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(std::path::PathBuf::from);
        let default = hashi_storage::paths::config_root(Channel::Preview)
            .map(|root| root.join("logs"))
            .unwrap_or_else(|_| std::env::temp_dir().join("Hashi-RustPreview-logs"));
        (custom, default)
    }
    pub(super) fn session_log_enabled(&self) -> bool {
        self.snapshot
            .settings
            .get("session_log")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            self.settings.draft = None;
            return;
        }
        let draft = self
            .settings
            .draft
            .get_or_insert_with(|| Draft::from(&self.snapshot.settings));
        let mut open = true;
        let mut save = false;
        egui::Window::new("設定")
            .default_width(480.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("セッションログ");
                ui.checkbox(
                    &mut draft.session_log,
                    "SSH端末の受信出力を自動保存（PuTTY logging 相当）",
                );
                ui.add_enabled_ui(draft.session_log, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.session_log_dir)
                                .hint_text("空欄でRust版の設定フォルダ/logs"),
                        );
                        if ui.button("参照").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                draft.session_log_dir = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                });
                ui.label("改行で確定した行だけを記録します。vim等の全画面表示は記録しません。");
                ui.label("新しい接続から適用します。");
                ui.separator();
                save = ui
                    .add_enabled(!self.startup_error, egui::Button::new("保存"))
                    .clicked();
            });
        if save {
            let draft = draft.clone();
            match self.store.transact(|snapshot| {
                snapshot
                    .settings
                    .insert("session_log".into(), Value::Bool(draft.session_log));
                snapshot.settings.insert(
                    "session_log_dir".into(),
                    Value::String(draft.session_log_dir.trim().to_owned()),
                );
                Ok(())
            }) {
                Ok(snapshot) => {
                    self.snapshot = snapshot;
                    self.notice = "設定を保存しました".into();
                    self.settings.open = false;
                }
                Err(error) => self.notice = format!("設定を保存できません: {error}"),
            }
        }
        if !open {
            self.settings.open = false;
        }
    }
}
