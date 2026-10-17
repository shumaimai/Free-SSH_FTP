//! ホーム画面。保存した接続先をカードで並べ、1回押して接続する。
//! 編集は専用の窓で行い、一覧と入力欄を同じ場所に並べない。
use super::*;
use crate::theme;

#[derive(Default)]
pub(super) struct HomeState {
    pub(super) query: String,
    pub(super) editing: bool,
    delete_armed: bool,
}

enum CardAction {
    Connect,
    Edit,
}

fn auth_label(method: &str) -> &'static str {
    match method {
        "password" => "パスワード",
        "agent" => "SSH agent",
        _ => "秘密鍵",
    }
}

/// 検索語に合う接続先の添字。名前・ホスト・利用者名を大文字小文字無視で照合する。
pub(super) fn matching(profiles: &[Profile], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    profiles
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            query.is_empty()
                || [&p.name, &p.host, &p.username]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

fn card(ui: &mut egui::Ui, profile: &Profile, width: f32) -> Option<CardAction> {
    let size = egui::vec2(width, 100.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(theme::RADIUS),
        if hovered {
            theme::CARD_HOVER
        } else {
            theme::CARD
        },
        egui::Stroke::new(
            1.0,
            if hovered {
                theme::ACCENT.gamma_multiply(0.6)
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let title = if profile.name.is_empty() {
        &profile.host
    } else {
        &profile.name
    };
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(14.0)), |ui| {
        // 選択できる文字がカードのクリックを奪わないようにする。
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal_top(|ui| {
            theme::avatar(ui, title, 44.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.add(egui::Label::new(egui::RichText::new(title).strong().size(16.0)).truncate());
                ui.add(
                    egui::Label::new(theme::muted(format!(
                        "{}@{}:{}",
                        profile.username, profile.host, profile.port
                    )))
                    .truncate(),
                );
                ui.horizontal(|ui| {
                    theme::chip(ui, auth_label(&profile.auth_method), theme::ACCENT);
                    if !profile.proxy_jump.is_empty() {
                        theme::chip(ui, "踏み台", theme::WARN);
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.small_button("編集").clicked() {
                    action = Some(CardAction::Edit);
                }
            });
        });
    });
    if action.is_none() && response.clicked() {
        action = Some(CardAction::Connect);
    }
    action
}

impl App {
    pub(super) fn home_page(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        let mut new = false;
        let mut local = false;
        let mut local_edit = false;
        let mut keygen = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("接続").size(26.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        new = ui
                            .add_enabled(!self.startup_error, theme::primary("新しい接続"))
                            .clicked();
                        ui.add(
                            egui::TextEdit::singleline(&mut self.home.query)
                                .hint_text("名前・ホストで検索")
                                .desired_width(220.0),
                        );
                    });
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(theme::muted("すぐ使う"));
                    local = ui.button("ローカル端末").clicked();
                    local_edit = ui.button("ファイルを編集").clicked();
                    keygen = ui.button("SSH鍵を作る").clicked();
                });
                ui.add_space(10.0);
                let profiles = self.snapshot.profiles.clone();
                let shown = matching(&profiles, &self.home.query);
                if profiles.is_empty() {
                    theme::card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(theme::title("まだ接続先がありません"));
                        ui.label(theme::muted(
                            "ホスト名と利用者名を入れるだけで登録できます。パスワードや秘密鍵は接続時に入力します。",
                        ));
                    });
                } else if shown.is_empty() {
                    ui.label(theme::muted("該当する接続先がありません"));
                }
                let width = ui.available_width();
                let columns = ((width + theme::GAP) / (300.0 + theme::GAP)).floor().max(1.0);
                let card_width = (width - theme::GAP * (columns - 1.0)) / columns;
                for row in shown.chunks(columns as usize) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme::GAP;
                        for &index in row {
                            match card(ui, &profiles[index], card_width) {
                                Some(CardAction::Connect) => open = Some((index, true)),
                                Some(CardAction::Edit) => open = Some((index, false)),
                                None => {}
                            }
                        }
                    });
                }
            });
        if new {
            self.profile = Profile::default();
            self.edit_index = None;
            self.home.editing = true;
            self.home.delete_armed = false;
        }
        if local {
            self.local();
        }
        if local_edit {
            self.open_local_editor();
        }
        if keygen {
            self.keygen.open = true;
        }
        if let Some((index, connect)) = open {
            if let Some(profile) = self.snapshot.profiles.get(index) {
                self.profile = profile.clone();
                self.edit_index = Some(index);
                if connect {
                    self.connect();
                } else {
                    self.home.editing = true;
                    self.home.delete_armed = false;
                }
            }
        }
    }

    pub(super) fn profile_editor(&mut self, ctx: &egui::Context) {
        if !self.home.editing {
            return;
        }
        let mut open = true;
        let (mut save, mut connect, mut delete) = (false, false, false);
        let existing = self.edit_index.is_some();
        let armed = self.home.delete_armed;
        let profile = &mut self.profile;
        egui::Window::new(if existing {
            "接続を編集"
        } else {
            "新しい接続"
        })
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(theme::muted("名前"));
            ui.add(
                egui::TextEdit::singleline(&mut profile.name)
                    .hint_text("例: 本番サーバー（空ならホスト名）")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(theme::muted("ホスト"));
                    ui.add(
                        egui::TextEdit::singleline(&mut profile.host)
                            .hint_text("example.com")
                            .desired_width(260.0),
                    );
                });
                ui.vertical(|ui| {
                    ui.label(theme::muted("ポート"));
                    ui.add(egui::DragValue::new(&mut profile.port).range(1..=65535));
                });
            });
            ui.label(theme::muted("利用者名"));
            ui.add(egui::TextEdit::singleline(&mut profile.username).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            ui.label(theme::muted("ログイン方法"));
            theme::segmented(
                ui,
                &mut profile.auth_method,
                &[
                    ("key".to_owned(), "秘密鍵"),
                    ("password".to_owned(), "パスワード"),
                    ("agent".to_owned(), "SSH agent"),
                ],
            );
            match profile.auth_method.as_str() {
                "key" => {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut profile.key_path)
                                .hint_text("空なら ~/.ssh の標準の鍵を探します")
                                .desired_width(300.0),
                        );
                        if ui.button("選ぶ").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                profile.key_path = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                }
                "password" => {
                    ui.label(theme::muted("パスワードは接続するときに入力します。"));
                }
                _ => {
                    ui.label(theme::muted("起動中のSSH agentを使います。"));
                }
            }
            egui::CollapsingHeader::new("詳細設定").show(ui, |ui| {
                ui.label(theme::muted("踏み台（ProxyJump）"));
                ui.add(
                    egui::TextEdit::singleline(&mut profile.proxy_jump)
                        .hint_text("例: user@bastion.example.com")
                        .desired_width(f32::INFINITY),
                );
                ui.label(theme::muted("ファイル画面の開始フォルダ"));
                ui.add(
                    egui::TextEdit::singleline(&mut profile.initial_path)
                        .hint_text("空ならホームフォルダ")
                        .desired_width(f32::INFINITY),
                );
                ui.checkbox(
                    &mut profile.agent_forwarding,
                    "SSH agentを接続先へ転送する（接続先に署名を許可します）",
                );
            });
            ui.add_space(6.0);
            let ready = !profile.host.trim().is_empty() && !profile.username.trim().is_empty();
            ui.horizontal(|ui| {
                connect = ui
                    .add_enabled(ready && !self.startup_error, theme::primary("保存して接続"))
                    .clicked();
                save = ui
                    .add_enabled(ready && !self.startup_error, egui::Button::new("保存"))
                    .clicked();
                if existing {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        delete = ui
                            .add(theme::danger(if armed {
                                "もう一度押すと削除"
                            } else {
                                "削除"
                            }))
                            .clicked();
                    });
                }
            });
            if !ready {
                ui.label(theme::muted("ホストと利用者名を入力してください。"));
            }
        });
        if !open {
            self.home.editing = false;
        }
        if delete {
            if self.home.delete_armed {
                self.delete_profile();
            } else {
                self.home.delete_armed = true;
            }
        }
        if save || connect {
            if self.save_profile() && connect {
                self.home.editing = false;
                self.connect();
            } else if save {
                self.home.editing = self.notice != "保存しました";
            }
        }
    }

    fn delete_profile(&mut self) {
        let Some(index) = self.edit_index else {
            return;
        };
        match self.store.transact(|snapshot| {
            if index < snapshot.profiles.len() {
                snapshot.profiles.remove(index);
            }
            Ok(())
        }) {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.edit_index = None;
                self.profile = Profile::default();
                self.home.editing = false;
                self.notice = "接続先を削除しました".into();
            }
            Err(error) => self.notice = format!("削除できません: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, host: &str, user: &str) -> Profile {
        Profile {
            name: name.into(),
            host: host.into(),
            username: user.into(),
            ..Profile::default()
        }
    }

    #[test]
    fn search_matches_name_host_and_user_ignoring_case() {
        let profiles = vec![
            profile("本番", "prod.example.com", "deploy"),
            profile("", "test.local", "Alice"),
        ];
        assert_eq!(matching(&profiles, ""), vec![0, 1]);
        assert_eq!(matching(&profiles, "PROD"), vec![0]);
        assert_eq!(matching(&profiles, " alice "), vec![1]);
        assert_eq!(matching(&profiles, "本"), vec![0]);
        assert!(matching(&profiles, "nope").is_empty());
    }
}
