use super::*;
use std::{ops::Deref, time::Instant};

pub(super) struct RuntimeOwner(Option<Runtime>);
impl RuntimeOwner {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(Some(Runtime::new()?)))
    }
}
impl Deref for RuntimeOwner {
    type Target = Runtime;
    fn deref(&self) -> &Runtime {
        self.0.as_ref().expect("実行環境は画面の終了まで保持する")
    }
}
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        // 通常は全仕事の完了後。OSによる強制closeや利用者の明示終了でも永久停止しない。
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}
impl App {
    pub(super) fn begin_close(&mut self, index: usize, remove: bool) {
        self.unbind_terminal(index);
        let tab = &mut self.tabs[index];
        tab.remove_on_close |= remove;
        if tab.closing {
            return;
        }
        tab.closing = true;
        tab.status = "処理の終了を待っています".into();
        tab.pending_input.clear();
        tab.pending_bytes = 0;
        tab.work_cancel.cancel();
        tab.work.close();
        let connecting = tab.session.is_none();
        if connecting {
            tab.cancel.cancel();
            tab.transport.close();
        }
        let (work, transport, stamp, tx) = (
            tab.work.clone(),
            tab.transport.clone(),
            tab.stamp.clone(),
            self.tx.clone(),
        );
        self.closing_work
            .spawn(self.runtime.handle(), async move {
                work.wait().await;
                if connecting {
                    transport.wait().await;
                }
                // 同じmessage queueのReadyを先に適用してから、最後の資源を取得する。
                let _ = tx.send(Message::Quiesced { stamp }).await;
            })
            .expect("終了処理の受付");
    }
    pub(super) fn finish_close(&mut self, index: usize) {
        let tab = &mut self.tabs[index];
        let (session, permissions, local) = (
            tab.session.clone(),
            tab.permissions.clone(),
            tab.local.clone(),
        );
        let (cancel, transport, stamp, tx) = (
            tab.cancel.clone(),
            tab.transport.clone(),
            tab.stamp.clone(),
            self.tx.clone(),
        );
        let forwards = std::mem::take(&mut tab.forwards);
        let log = tab.log.take().map(|mut log| {
            let (mut lines, dropped) = tab.terminal.take_log_lines();
            if dropped > 0 {
                lines.push(format!("[Hashi: {dropped}行を記録できませんでした]"));
            }
            lines.extend(tab.terminal.pending_log_line());
            let written = log.write_lines(lines);
            tab.terminal.set_line_log(false);
            (log, written)
        });
        self.closing_work
            .spawn(self.runtime.handle(), async move {
                let mut errors = vec![];
                if let Some((mut log, written)) = log {
                    let closed = tokio::task::spawn_blocking(move || {
                        log.close(Duration::from_secs(5))
                    })
                    .await
                    .unwrap_or_else(|_| Err("ログの終了処理が中断されました".into()));
                    if let Err(error) = written.and(closed) {
                        errors.push(format!("セッションログ: {error}"));
                    }
                }
                for mut forward in forwards {
                    if let Err(error) = forward.stop().await {
                        errors.push(format!("ポート転送の停止は未確認: {error}"));
                    }
                }
                if let Some(manager) = permissions {
                    match manager.shutdown().await {
                        Ok(0) => {}
                        Ok(pending) => errors.push(format!(
                            "以前の記録を含む{pending}件の権限復元が未完了です。復旧記録を保持しています"
                        )),
                        Err(error) => errors.push(format!(
                            "権限復元の確認が未完了です。復旧記録を保持: {error}"
                        )),
                    }
                }
                if let Some(session) = session {
                    session.background.close();
                    session.background.wait().await;
                    errors.extend(session.background.take_errors());
                    match tokio::time::timeout(Duration::from_secs(5), session.close()).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            errors.push(format!("SSH接続の終了応答は未確認: {error}"))
                        }
                        Err(_) => errors.push("SSH接続の終了応答がタイムアウトしました".into()),
                    }
                }
                cancel.cancel();
                transport.close();
                transport.wait().await;
                if let Some(local) = local {
                    match tokio::task::spawn_blocking(move || {
                        local
                            .lock()
                            .map_err(|_| anyhow::anyhow!("ローカル端末の状態を読めません"))
                            .and_then(|mut terminal| terminal.close())
                    })
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            errors.push(format!("ローカル端末を回収できません: {error}"))
                        }
                        Err(error) => {
                            errors.push(format!("ローカル端末の終了処理が失敗しました: {error}"))
                        }
                    }
                }
                let _ = tx.send(Message::Closed { stamp, errors }).await;
            })
            .expect("接続終了の受付");
    }
    pub(super) fn closed_tab(&mut self, index: usize, errors: Vec<String>) {
        if !errors.is_empty() {
            self.notice = errors.join(" / ");
            if self.exiting {
                self.close_errors.extend(errors);
            }
        }
        if self.tabs[index].remove_on_close {
            self.tabs.remove(index);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        } else {
            let tab = &mut self.tabs[index];
            tab.session = None;
            tab.nav = None;
            tab.xfer = None;
            tab.local = None;
            tab.input = None;
            tab.resize = None;
            tab.permissions = None;
            tab.status = "接続は終了しました".into();
            // スクロールバックは残し、次の×では完了済みタブを削除する。
            tab.closing = false;
        }
    }
    fn begin_exit(&mut self) {
        self.confirm_exit = false;
        self.exiting = true;
        self.exit_started = self.confirm_started.take().or_else(|| Some(Instant::now()));
        self.stop_external();
        self.cancel_files();
        self.work.close();
        for index in (0..self.tabs.len()).rev() {
            self.begin_close(index, true);
        }
    }
    pub(super) fn shutdown_ui(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.exiting {
                if !self.confirm_exit {
                    self.confirm_started = Some(Instant::now());
                }
                self.confirm_exit = true;
            }
        }
        if self.confirm_exit && !self.editor_busy() && !self.has_unsaved_edits() {
            self.begin_exit();
        }
        if self.confirm_exit {
            egui::Window::new("終了の確認")
                .collapsible(false)
                .show(ctx, |ui| {
                    if self.editor_busy() {
                        ui.label("ファイル処理の終了を待っています。");
                        if self.confirm_started.is_some_and(|started| started.elapsed() >= Duration::from_secs(30)) {
                            ui.label("応答待ちが続いています。未保存の編集や保存結果が未確認のまま残る場合があります。");
                            if ui.button("未保存の編集を破棄して終了処理へ進む").clicked() {
                                self.begin_exit();
                            }
                        }
                    } else {
                        ui.label("未保存の編集があります。");
                        if ui.button("編集を破棄して終了処理へ進む").clicked() {
                            self.begin_exit();
                        }
                    }
                    if ui.button("戻る").clicked() {
                        self.confirm_exit = false;
                        self.confirm_started = None;
                    }
                });
        }
        if !self.exiting {
            return;
        }
        if self.tabs.is_empty() {
            self.closing_work.close();
        }
        let finished =
            self.tabs.is_empty() && self.work.is_finished() && self.closing_work.is_finished();
        if finished && self.close_errors.is_empty() {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        egui::Window::new("終了処理").collapsible(false).show(ctx,|ui| {
            if finished {
                ui.label("終了時の処理結果を確認してください。");
                for error in &self.close_errors {ui.label(error);}
                if ui.button("結果を確認して終了").clicked() {self.allow_close=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            } else {
                ui.label("転送の確定応答・権限復元・端末の終了を待っています。");
                if self.exit_started.is_some_and(|started|started.elapsed()>=Duration::from_secs(30)) {
                    ui.label("接続先の応答待ちが続いています。ここで終了すると確定結果は未確認となり、復旧記録が残る場合があります。");
                    if ui.button("未確認の処理と記録を残して終了").clicked() {self.allow_close=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
                }
            }
        });
    }
}
