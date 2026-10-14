use super::*;
use hashi_core::forward::{Destination, LocalForward, RemoteForward};

pub(super) enum ActiveForward {
    Local(LocalForward),
    Remote(RemoteForward),
}
impl ActiveForward {
    async fn stop(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Local(forward) => {
                forward.stop().await;
                Ok(())
            }
            Self::Remote(forward) => forward.stop().await,
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Local(f) => format!("ローカル待受 {}", f.address),
            Self::Remote(f) => format!("リモート待受 {}:{}", f.bind, f.port),
        }
    }
}
#[derive(Clone)]
pub(super) struct ForwardEditor {
    pub open: bool,
    kind: String,
    bind: String,
    port: u16,
    host: String,
    target_port: u16,
}
impl Default for ForwardEditor {
    fn default() -> Self {
        Self {
            open: false,
            kind: "L".into(),
            bind: "127.0.0.1".into(),
            port: 8080,
            host: "127.0.0.1".into(),
            target_port: 22,
        }
    }
}
impl App {
    pub(super) fn forward_dialog(&mut self, ctx: &egui::Context) {
        if !self.forward_editor.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut stop = None;
        egui::Window::new("ポート転送")
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(tab) = self.tabs.get(self.active) else {
                    ui.label("SSH接続を選択してください");
                    return;
                };
                ui.label(&tab.label);
                egui::ComboBox::from_id_salt("forward-kind")
                    .selected_text(&self.forward_editor.kind)
                    .show_ui(ui, |ui| {
                        for (kind, label) in [
                            ("L", "ローカル -L"),
                            ("R", "リモート -R"),
                            ("D", "SOCKS5 -D"),
                        ] {
                            ui.selectable_value(
                                &mut self.forward_editor.kind,
                                kind.to_owned(),
                                label,
                            );
                        }
                    });
                ui.label("待受アドレス（公開範囲を確認してください）");
                ui.text_edit_singleline(&mut self.forward_editor.bind);
                ui.label("待受ポート（0なら自動割当）");
                ui.add(egui::DragValue::new(&mut self.forward_editor.port).range(0..=65535));
                if self.forward_editor.kind != "D" {
                    ui.label("転送先ホスト");
                    ui.text_edit_singleline(&mut self.forward_editor.host);
                    ui.label("転送先ポート");
                    ui.add(
                        egui::DragValue::new(&mut self.forward_editor.target_port).range(1..=65535),
                    );
                } else {
                    ui.label("SOCKS5は認証なしのCONNECTです。標準のloopback待受を推奨します。");
                }
                start = ui
                    .add_enabled(
                        tab.session.is_some() && !tab.cancel.is_cancelled(),
                        egui::Button::new("開始"),
                    )
                    .clicked();
                ui.separator();
                for (index, forward) in tab.forwards.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(forward.label());
                        if ui.button("停止").clicked() {
                            stop = Some(index);
                        }
                    });
                }
            });
        self.forward_editor.open = open;
        if let Some(index) = stop {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let mut forward = tab.forwards.remove(index);
                let stamp = tab.stamp.clone();
                let tx = self.tx.clone();
                self.runtime.spawn(async move {
                    let message = match forward.stop().await {
                        Ok(()) => "ポート転送を停止しました".into(),
                        Err(error) => format!("停止は未確認です: {error}"),
                    };
                    let _ = tx.send(Message::Done { stamp, message }).await;
                });
            }
        }
        if start {
            let Some(tab) = self.tabs.get(self.active) else {
                return;
            };
            let Some(session) = tab.session.clone() else {
                return;
            };
            let stamp = tab.stamp.clone();
            let tx = self.tx.clone();
            let editor = self.forward_editor.clone();
            self.runtime.spawn(async move {
                let bind = if editor.bind.contains(':') {
                    format!("[{}]:{}", editor.bind, editor.port)
                } else {
                    format!("{}:{}", editor.bind, editor.port)
                };
                let result = match editor.kind.as_str() {
                    "R" => RemoteForward::start(
                        session,
                        editor.bind,
                        editor.port,
                        editor.host,
                        editor.target_port,
                    )
                    .await
                    .map(ActiveForward::Remote),
                    "D" => LocalForward::start(session, &bind, Destination::Socks5)
                        .await
                        .map(ActiveForward::Local),
                    _ => LocalForward::start(
                        session,
                        &bind,
                        Destination::Fixed {
                            host: editor.host,
                            port: editor.target_port,
                        },
                    )
                    .await
                    .map(ActiveForward::Local),
                };
                match result {
                    Ok(handle) => {
                        let _ = tx.send(Message::ForwardReady { stamp, handle }).await;
                    }
                    Err(error) => {
                        let _ = tx
                            .send(Message::Done {
                                stamp,
                                message: error.to_string(),
                            })
                            .await;
                    }
                }
            });
        }
    }
}
