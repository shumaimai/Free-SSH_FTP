use super::*;
use hashi_core::permissions::{Manager, SftpBackend};

impl App {
    pub(super) fn permission_dialog(&mut self, ctx: &egui::Context) {
        if !self.permission_open {
            return;
        }
        let mut open = true;
        egui::Window::new("一時的な権限変更")
            .default_width(520.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("一覧と転送で権限不足の場合、所有ファイルのmodeを一時変更します。");
                ui.label("作業後に元へ戻します。復元失敗時は記録を残し、再接続後に復旧できます。");
                ui.label("sudo、ACL、旧版の未復元記録の取り込みはまだ対応していません。");
                ui.checkbox(
                    &mut self.legacy_stopped,
                    "マーカーで確認できない旧版も含め、すべての旧Python版を終了した",
                );
                let Some(tab) = self.tabs.get(self.active) else {
                    ui.label("SSH接続を選択してください");
                    return;
                };
                let enabled = tab.permissions.is_some();
                if let Some(note) = tab
                    .permissions
                    .as_ref()
                    .and_then(|manager| manager.migration_note())
                {
                    ui.label(note);
                }
                ui.label(
                    "旧版の未復元記録があれば権限補助を開始しません。旧版で復旧してください。",
                );
                let busy = tab.permissions_busy;
                let connected = tab.session.is_some()
                    && !tab.closing
                    && !tab.work.is_closed()
                    && !tab.cancel.is_cancelled();
                ui.label(format!(
                    "接続: {} / {}",
                    tab.label,
                    if enabled { "有効" } else { "無効" }
                ));
                if ui
                    .add_enabled(
                        connected && !enabled && !busy && self.legacy_stopped,
                        egui::Button::new("この接続で有効にする"),
                    )
                    .clicked()
                {
                    let tab = &mut self.tabs[self.active];
                    tab.permissions_busy = true;
                    let session = tab.session.clone().unwrap();
                    let stamp = tab.stamp.clone();
                    let tx = self.tx.clone();
                    tab.work
                        .spawn(self.runtime.handle(), async move {
                            let result = async {
                                let key = session
                                    .accepted
                                    .lock()
                                    .map_err(|_| anyhow::anyhow!("確認済み鍵を読めません"))?
                                    .get(&format!("{}:{}", session.scope.host, session.scope.port))
                                    .cloned()
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("接続先の確認済み鍵がありません")
                                    })?;
                                // usernameと検証済み鍵まで束縛し、異なる相手の復元記録を適用しない。
                                let connection = serde_json::to_string(&(
                                    session.scope.host.as_str(),
                                    session.scope.port,
                                    session.scope.username.as_str(),
                                    key.fingerprint,
                                ))?;
                                let backend = Arc::new(SftpBackend::new(session.sftp().await?));
                                Manager::scoped(backend, connection, true)
                            }
                            .await
                            .map_err(|e| e.to_string());
                            let _ = tx.send(Message::PermissionsReady { stamp, result }).await;
                        })
                        .expect("権限補助の受付確認済み");
                }
                if ui
                    .add_enabled(
                        connected && enabled && !busy,
                        egui::Button::new("この接続の未復元記録を復元"),
                    )
                    .clicked()
                {
                    let tab = &mut self.tabs[self.active];
                    let manager = tab.permissions.clone().unwrap();
                    let stamp = tab.stamp.clone();
                    let tx = self.tx.clone();
                    tab.work
                        .spawn(self.runtime.handle(), async move {
                            let (message, failed) = match manager.recover().await {
                                Ok((restored, pending)) => (
                                    format!(
                                    "権限復旧: {restored}件復元 / {pending}件未完了（記録保持）"
                                ),
                                    pending > 0,
                                ),
                                Err(error) => (
                                    format!("権限復旧できませんでした。記録を保持: {error}"),
                                    true,
                                ),
                            };
                            let _ = tx
                                .send(Message::Done {
                                    stamp,
                                    message,
                                    failed,
                                })
                                .await;
                        })
                        .expect("復旧の受付確認済み");
                }
            });
        self.permission_open = open;
    }
}
