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
    update_check: bool,
    session_log: bool,
    session_log_dir: String,
}
impl Draft {
    fn from(settings: &serde_json::Map<String, Value>) -> Self {
        Self {
            update_check: settings
                .get("update_check")
                .and_then(Value::as_bool)
                .unwrap_or(true),
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
                ui.heading("更新");
                ui.checkbox(
                    &mut draft.update_check,
                    "起動時にRustプレビュー版の新しいリリースを確認する",
                );
                ui.label("旧Python版のリリースとは別に、rust-preview-v のタグだけを確認します。");
                ui.separator();
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
                    .insert("update_check".into(), Value::Bool(draft.update_check));
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

impl App {
    /// 起動時の更新確認。失敗はPython版と同じく画面へ出さない。
    pub(super) fn start_update_check(&mut self) {
        let enabled = self
            .snapshot
            .settings
            .get("update_check")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !enabled || self.startup_error {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 読取専用の確認なので終了待ちの対象にしない（終了をネットワーク待ちで遅らせない）。
        self.runtime.handle().spawn(async move {
            let result = tokio::task::spawn_blocking(|| {
                hashi_core::update::check(
                    hashi_core::update::UpdateChannel::Preview,
                    env!("CARGO_PKG_VERSION"),
                    None,
                )
            })
            .await;
            if let Ok(Ok(Some(update))) = result {
                let _ = tx.send(update);
            }
        });
        self.update_check = Some(rx);
    }
    pub(super) fn poll_update(&mut self) {
        let Some(receiver) = &mut self.update_check else {
            return;
        };
        match receiver.try_recv() {
            Ok(update) => {
                self.available_update = Some(update);
                self.update_check = None;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => self.update_check = None,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
    }
    pub(super) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(update) = &self.available_update else {
            return;
        };
        ui.separator();
        ui.label(format!("新しいプレビュー版 {} があります", update.tag));
        // selectで公式リポジトリのタグページと一致したURLだけを保持している。
        if ui.button("リリースページを開く").clicked() {
            if let Err(error) = webbrowser::open(&update.url) {
                self.notice = format!("ブラウザを開けません: {error}");
            }
        }
    }
}
