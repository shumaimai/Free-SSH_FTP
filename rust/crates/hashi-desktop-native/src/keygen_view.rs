//! SSH鍵の生成と、接続中サーバーへの公開鍵登録（Python版KeygenDialog相当）。
use super::*;
use hashi_core::keygen::{self, KeyKind};

pub(super) struct KeygenWindow {
    pub(super) open: bool,
    kind: usize,
    bits: usize,
    passphrase: String,
    confirm: String,
    comment: String,
    path: String,
    overwrite: bool,
    register: bool,
    running: Option<tokio::sync::oneshot::Receiver<Result<Generated, String>>>,
    result: Option<Generated>,
}
pub(super) struct Generated {
    path: std::path::PathBuf,
    public_line: String,
    registered: Option<bool>,
    warning: Option<&'static str>,
}
impl Default for KeygenWindow {
    fn default() -> Self {
        Self {
            open: false,
            kind: 0,
            bits: 0,
            passphrase: String::new(),
            confirm: String::new(),
            comment: String::new(),
            path: default_path(0),
            overwrite: false,
            register: false,
            running: None,
            result: None,
        }
    }
}
const KINDS: [&str; 3] = ["Ed25519", "ECDSA", "RSA"];
fn bit_choices(kind: usize) -> Vec<usize> {
    match kind {
        1 => keygen::ECDSA_BITS
            .iter()
            .map(|bits| *bits as usize)
            .collect(),
        2 => keygen::RSA_BITS.to_vec(),
        _ => vec![],
    }
}
fn default_path(kind: usize) -> String {
    let name = ["id_ed25519", "id_ecdsa", "id_rsa"][kind];
    std::env::home_dir()
        .map(|home| home.join(".ssh").join(name).to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl App {
    pub(super) fn keygen_dialog(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &mut self.keygen.running {
            match receiver.try_recv() {
                Ok(Ok(generated)) => {
                    self.keygen.running = None;
                    self.notice = "SSH 鍵を生成しました".into();
                    self.keygen.result = Some(generated);
                }
                Ok(Err(error)) => {
                    self.keygen.running = None;
                    self.notice = format!("SSH 鍵の生成: {error}");
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.keygen.running = None;
                    self.notice = "SSH 鍵の生成を完了できませんでした".into();
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if !self.keygen.open {
            return;
        }
        let can_register = self.tabs.get(self.active).is_some_and(|tab| {
            tab.session.is_some() && tab.nav.is_some() && !tab.closing && !tab.work.is_closed()
        });
        let mut open = true;
        let mut start = false;
        let mut use_key = None;
        let window = &mut self.keygen;
        let exists = std::path::Path::new(window.path.trim()).exists();
        egui::Window::new("SSH 鍵を生成")
            .default_width(520.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let busy = window.running.is_some();
                ui.add_enabled_ui(!busy, |ui| {
                    let before = window.kind;
                    egui::ComboBox::from_label("鍵種別")
                        .selected_text(KINDS[window.kind])
                        .show_ui(ui, |ui| {
                            for (index, name) in KINDS.iter().enumerate() {
                                ui.selectable_value(&mut window.kind, index, *name);
                            }
                        });
                    if before != window.kind {
                        window.bits = 0;
                        if window.path == default_path(before) {
                            window.path = default_path(window.kind);
                        }
                    }
                    let choices = bit_choices(window.kind);
                    if choices.is_empty() {
                        ui.label("ビット数: （固定）");
                    } else {
                        egui::ComboBox::from_label("ビット数")
                            .selected_text(choices[window.bits].to_string())
                            .show_ui(ui, |ui| {
                                for (index, bits) in choices.iter().enumerate() {
                                    ui.selectable_value(&mut window.bits, index, bits.to_string());
                                }
                            });
                    }
                    ui.label("パスフレーズ（任意）");
                    ui.add(egui::TextEdit::singleline(&mut window.passphrase).password(true));
                    ui.label("パスフレーズの確認");
                    ui.add(egui::TextEdit::singleline(&mut window.confirm).password(true));
                    ui.label("コメント（任意）");
                    ui.text_edit_singleline(&mut window.comment);
                    ui.label("秘密鍵の保存先");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut window.path);
                        if ui.button("参照").clicked() {
                            if let Some(path) = rfd::FileDialog::new().save_file() {
                                window.path = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                    if exists {
                        ui.checkbox(&mut window.overwrite, "既存の秘密鍵ファイルを上書きする");
                    }
                    ui.add_enabled(
                        can_register,
                        egui::Checkbox::new(&mut window.register, "現在の接続先に公開鍵を登録する"),
                    )
                    .on_disabled_hover_text("接続中のセッションがありません");
                    ui.label("全種別をOpenSSH形式で保存します。Unixでは権限を600にします。");
                });
                let mismatch = window.passphrase != window.confirm;
                if mismatch {
                    ui.colored_label(egui::Color32::YELLOW, "パスフレーズが一致しません");
                }
                let ready = !busy
                    && !mismatch
                    && !window.path.trim().is_empty()
                    && (!exists || window.overwrite);
                if busy {
                    ui.label("生成しています…");
                }
                start = ui.add_enabled(ready, egui::Button::new("生成")).clicked();
                if let Some(result) = &window.result {
                    ui.separator();
                    ui.label(format!("秘密鍵を保存しました: {}", result.path.display()));
                    match result.registered {
                        Some(true) => {
                            ui.label("公開鍵を接続先の authorized_keys に登録しました。");
                        }
                        Some(false) => {
                            ui.label("接続先には同じ公開鍵が登録済みでした。");
                        }
                        None => {}
                    }
                    if let Some(warning) = result.warning {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                    ui.label("公開鍵");
                    let mut line = result.public_line.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut line)
                            .desired_rows(2)
                            .interactive(true),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("公開鍵をコピー").clicked() {
                            ui.ctx().copy_text(result.public_line.clone());
                        }
                        if ui.button("この秘密鍵を接続設定に使う").clicked() {
                            use_key = Some(result.path.to_string_lossy().into_owned());
                        }
                    });
                }
            });
        if let Some(path) = use_key {
            self.profile.auth_method = "key".into();
            self.profile.key_path = path;
            self.notice = "接続設定の秘密鍵を変更しました。保存すると記録されます".into();
        }
        if !open {
            if self.keygen.running.is_none() {
                self.keygen = KeygenWindow::default();
            } else {
                self.keygen.open = false;
            }
            return;
        }
        if start && !self.exiting {
            self.start_keygen(can_register);
        }
    }
    fn start_keygen(&mut self, can_register: bool) {
        let window = &mut self.keygen;
        let kind = match window.kind {
            1 => KeyKind::Ecdsa(keygen::ECDSA_BITS[window.bits]),
            2 => KeyKind::Rsa(keygen::RSA_BITS[window.bits]),
            _ => KeyKind::Ed25519,
        };
        let path = std::path::PathBuf::from(window.path.trim());
        let passphrase = std::mem::take(&mut window.passphrase);
        window.confirm.clear();
        let comment = window.comment.clone();
        let overwrite = window.overwrite;
        let register = (window.register && can_register)
            .then(|| self.tabs.get(self.active))
            .flatten()
            .and_then(|tab| Some((tab.session.clone()?, tab.nav.clone()?, tab.work.clone())));
        window.result = None;
        let tab_work = register.as_ref().map(|(_, _, work)| work.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        window.running = Some(rx);
        let home = std::env::home_dir().unwrap_or_default();
        let job = async move {
            let result = async {
                let path_for_write = path.clone();
                let public_line = tokio::task::spawn_blocking(move || {
                    let key = keygen::generate(kind, &comment)?;
                    let content = keygen::private_openssh(&key, &passphrase)?;
                    keygen::write_private_key(&path_for_write, &content, overwrite)?;
                    keygen::public_line(&key)
                })
                .await
                .map_err(|_| anyhow::anyhow!("生成処理が中断されました"))??;
                let registered = match &register {
                    Some((session, nav, _)) => {
                        let nav = nav.lock().await;
                        Some(
                            keygen::register_public_key(session, &nav, &public_line)
                                .await
                                .map_err(|error| {
                                    anyhow::anyhow!(
                                        "秘密鍵は保存しましたが、公開鍵を登録できません: {error:#}"
                                    )
                                })?,
                        )
                    }
                    None => None,
                };
                anyhow::Ok(Generated {
                    warning: keygen::location_warning(&path, cfg!(windows), &home),
                    path,
                    public_line,
                    registered,
                })
            }
            .await;
            let _ = tx.send(result.map_err(|error| format!("{error:#}")));
        };
        // 登録はタブの接続を使うので、タブの終了処理がこの完了を待つ。
        let work = tab_work.unwrap_or_else(|| self.work.clone());
        let accepted = work.spawn(self.runtime.handle(), job).is_ok();
        if !accepted {
            self.keygen.running = None;
            self.notice = "終了処理中のため鍵を生成できません".into();
        }
    }
}
