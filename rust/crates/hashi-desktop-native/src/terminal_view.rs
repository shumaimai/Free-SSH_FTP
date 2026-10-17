use super::{egui, Tab};
use hashi_terminal::CellColor;
fn color(value: &CellColor, foreground: bool) -> egui::Color32 {
    match *value {
        CellColor::Rgb { r, g, b } => egui::Color32::from_rgb(r, g, b),
        CellColor::Named { index: 256 } if foreground => egui::Color32::from_rgb(218, 222, 230),
        CellColor::Named { index: 257 } => egui::Color32::from_rgb(20, 23, 29),
        CellColor::Indexed { index } => indexed(index as usize),
        CellColor::Named { index } => indexed(index as usize),
    }
}
fn indexed(index: usize) -> egui::Color32 {
    let colors = [
        0x171a21, 0xef6b73, 0x9bdc97, 0xf1cf8a, 0x80aaff, 0xc6a0ef, 0x83d5e5, 0xd9dce4, 0x6c7382,
        0xff8992, 0xb8efb0, 0xffe1a3, 0xa4c2ff, 0xdbbafa, 0xa5e9f3, 0xffffff,
    ];
    if index < 16 {
        let n = colors[index];
        egui::Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
    } else if index < 232 {
        let n = index - 16;
        let component = |n: usize| if n == 0 { 0 } else { 55 + n as u8 * 40 };
        egui::Color32::from_rgb(component(n / 36), component((n / 6) % 6), component(n % 6))
    } else {
        egui::Color32::from_gray((8 + index.saturating_sub(232).min(23) * 10) as u8)
    }
}
pub(super) fn show(ui: &mut egui::Ui, tab: &mut Tab) {
    if tab.closing || tab.work.is_closed() {
        ui.label("端末の終了処理中です");
        return;
    }
    if tab.search_open {
        ui.horizontal(|ui| {
            ui.label("端末検索");
            ui.text_edit_singleline(&mut tab.terminal_search);
            if ui.button("次へ").clicked() {
                tab.search_position = tab
                    .terminal
                    .find_next(&tab.terminal_search, tab.search_position);
            }
            if ui.button("閉じる").clicked() {
                tab.search_open = false;
            }
        });
    }
    let font = egui::FontId::monospace(14.0);
    let width = 8.5;
    let height = 19.0;
    let available = ui.available_size();
    let columns = ((available.x / width).floor().max(1.0) as usize).min(1024);
    let rows = ((available.y / height).floor().max(1.0) as usize).min(512);
    if tab.terminal.size.columns != columns || tab.terminal.size.rows != rows {
        tab.terminal.resize(columns, rows);
        if let Some(resize) = &tab.resize {
            let size = (columns as u32, rows as u32);
            if resize.try_send(size).is_err() {
                tab.pending_size = Some(size);
            }
        }
        if tab.local.is_some() {
            tab.pending_local_size = Some((columns as u16, rows as u16));
        }
    }
    if let (Some(local), Some((columns, rows))) = (&tab.local, tab.pending_local_size) {
        if let Ok(terminal) = local.try_lock() {
            let _ = terminal.resize(columns, rows);
            tab.pending_local_size = None;
        } else {
            ui.ctx().request_repaint();
        }
    }
    let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 23, 29));
    for cell in tab.terminal.cells() {
        let pos = rect.min + egui::vec2(cell.column as f32 * width, cell.row as f32 * height);
        let size = egui::vec2(if cell.wide { width * 2.0 } else { width }, height);
        let fg = color(&cell.foreground, true);
        let bg = color(&cell.background, false);
        painter.rect_filled(
            egui::Rect::from_min_size(pos, size),
            0.0,
            if cell.selected {
                super::style::TERMINAL_SELECTION
            } else if cell.inverse {
                fg
            } else {
                bg
            },
        );
        painter.text(
            pos,
            egui::Align2::LEFT_TOP,
            &cell.text,
            font.clone(),
            if cell.inverse { bg } else { fg },
        );
    }
    let (row, column) = tab.terminal.cursor();
    let cursor = egui::Rect::from_min_size(
        rect.min + egui::vec2(column as f32 * width, row as f32 * height),
        egui::vec2(width, height),
    );
    if response.has_focus() && tab.terminal.cursor_visible() {
        painter.rect_stroke(
            cursor,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        ui.ctx().output_mut(|output| {
            output.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Terminal,
                rect,
                cursor_rect: cursor,
                should_interrupt_composition: false,
            })
        });
    }
    let cell_at = |pos: egui::Pos2| {
        (
            (((pos.y - rect.top()) / height).floor().max(0.0) as i32).min(rows as i32 - 1),
            (((pos.x - rect.left()) / width).floor().max(0.0) as usize).min(columns - 1),
        )
    };
    let report_mouse = tab.terminal.modes().intersects(alacritty_mouse_modes())
        && !ui.input(|input| input.modifiers.shift);
    if response.clicked_by(egui::PointerButton::Secondary) && !report_mouse {
        response.request_focus();
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
    if response.clicked() {
        if ui.input(|input| input.modifiers.ctrl) {
            if let Some(position) = response.interact_pointer_pos() {
                let (row, column) = cell_at(position);
                if let Some(url) = tab.terminal.http_url_at(row as usize, column) {
                    if let Err(error) = webbrowser::open(&url) {
                        tab.status = error.to_string();
                    }
                }
            }
        }
        response.request_focus();
        tab.terminal.clear_selection();
    }
    if response.drag_started() && !report_mouse {
        tab.selection_start = response.interact_pointer_pos().map(cell_at);
    }
    if response.dragged() && !report_mouse {
        if let (Some(start), Some(end)) = (tab.selection_start, response.interact_pointer_pos()) {
            tab.terminal.select(start, cell_at(end));
        }
    }
    if response.drag_stopped() && !report_mouse {
        if let Some(text) = tab.terminal.selected_text() {
            ui.ctx().copy_text(text);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if report_mouse {
                if let Some(position) = ui.input(|input| input.pointer.latest_pos()) {
                    let (row, column) = cell_at(position);
                    let modifiers = ui.input(|input| input.modifiers);
                    let bytes = tab.terminal.mouse(hashi_terminal::MouseInput {
                        button: if scroll > 0.0 { 64 } else { 65 },
                        column,
                        row: row as usize,
                        release: false,
                        motion: false,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        ctrl: modifiers.ctrl,
                    });
                    tab.send(bytes);
                }
            } else {
                tab.terminal.scroll((scroll / height) as i32);
            }
        }
    }
    if report_mouse {
        for event in ui.input(|input| input.events.clone()) {
            if let egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } = event
            {
                if rect.contains(pos) {
                    let (row, column) = cell_at(pos);
                    let button = match button {
                        egui::PointerButton::Primary => 0,
                        egui::PointerButton::Middle => 1,
                        egui::PointerButton::Secondary => 2,
                        _ => 3,
                    };
                    tab.send(tab.terminal.mouse(hashi_terminal::MouseInput {
                        button,
                        column,
                        row: row as usize,
                        release: !pressed,
                        motion: false,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        ctrl: modifiers.ctrl,
                    }));
                }
            } else if let egui::Event::PointerMoved(pos) = event {
                if rect.contains(pos) {
                    let (row, column) = cell_at(pos);
                    let modifiers = ui.input(|input| input.modifiers);
                    let button = ui.input(|input| {
                        if input.pointer.primary_down() {
                            0
                        } else if input.pointer.middle_down() {
                            1
                        } else if input.pointer.secondary_down() {
                            2
                        } else {
                            3
                        }
                    });
                    tab.send(tab.terminal.mouse(hashi_terminal::MouseInput {
                        button,
                        column,
                        row: row as usize,
                        release: false,
                        motion: true,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        ctrl: modifiers.ctrl,
                    }));
                }
            }
        }
    }
    if response.has_focus() {
        for event in ui.input(|input| input.events.clone()) {
            match event {
                egui::Event::Text(text) => {
                    if tab.preedit.is_empty() {
                        let mut bytes = text.into_bytes();
                        if ui.input(|input| input.modifiers.alt && !input.modifiers.ctrl) {
                            bytes.insert(0, 0x1b);
                        }
                        tab.send(bytes);
                    }
                }
                egui::Event::Paste(text) => {
                    tab.send(tab.terminal.paste(&text));
                    tab.terminal.bottom();
                }
                egui::Event::Copy => {
                    if let Some(text) = tab.terminal.selected_text() {
                        ui.ctx().copy_text(text);
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.ctrl && modifiers.shift && key == egui::Key::F {
                        tab.search_open = true;
                    } else if modifiers.ctrl && modifiers.shift && key == egui::Key::C {
                        if let Some(text) = tab.terminal.selected_text() {
                            ui.ctx().copy_text(text);
                        }
                    } else if tab.preedit.is_empty() {
                        let name = match key {
                            egui::Key::Space => " ".to_owned(),
                            egui::Key::OpenBracket => "[".to_owned(),
                            _ => format!("{key:?}"),
                        };
                        let special = matches!(
                            key,
                            egui::Key::Enter
                                | egui::Key::Tab
                                | egui::Key::Backspace
                                | egui::Key::Escape
                                | egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::Home
                                | egui::Key::End
                                | egui::Key::Insert
                                | egui::Key::Delete
                                | egui::Key::PageUp
                                | egui::Key::PageDown
                                | egui::Key::F1
                                | egui::Key::F2
                                | egui::Key::F3
                                | egui::Key::F4
                                | egui::Key::F5
                                | egui::Key::F6
                                | egui::Key::F7
                                | egui::Key::F8
                                | egui::Key::F9
                                | egui::Key::F10
                                | egui::Key::F11
                                | egui::Key::F12
                        );
                        if special || modifiers.ctrl {
                            tab.send(tab.terminal.key(
                                &name,
                                modifiers.ctrl,
                                modifiers.alt,
                                modifiers.shift,
                            ));
                        }
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => tab.preedit = text,
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    tab.preedit.clear();
                    tab.send(text.into_bytes());
                }
                _ => {}
            }
        }
    }
    if !tab.preedit.is_empty() {
        painter.text(
            cursor.left_bottom(),
            egui::Align2::LEFT_TOP,
            &tab.preedit,
            font,
            egui::Color32::YELLOW,
        );
    }
}

fn alacritty_mouse_modes() -> hashi_terminal::TermMode {
    hashi_terminal::TermMode::MOUSE_MODE
}
