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
    let font = egui::FontId::monospace(14.0);
    let width = 8.5;
    let height = 19.0;
    let available = ui.available_size();
    let columns = ((available.x / width).floor().max(1.0) as usize).min(1024);
    let rows = ((available.y / height).floor().max(1.0) as usize).min(512);
    if tab.terminal.size.columns != columns || tab.terminal.size.rows != rows {
        tab.terminal.resize(columns, rows);
        if let Some(resize) = &tab.resize {
            let _ = resize.try_send((columns as u32, rows as u32));
        }
        if let Some(local) = &tab.local {
            if let Ok(terminal) = local.lock() {
                let _ = terminal.resize(columns as u16, rows as u16);
            }
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
            if cell.inverse { fg } else { bg },
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
    if response.has_focus() {
        painter.rect_stroke(
            cursor,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
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
    if response.clicked() {
        response.request_focus();
        tab.terminal.clear_selection();
    }
    if response.drag_started() {
        tab.selection_start = response.interact_pointer_pos().map(cell_at);
    }
    if response.dragged() {
        if let (Some(start), Some(end)) = (tab.selection_start, response.interact_pointer_pos()) {
            tab.terminal.select(start, cell_at(end));
        }
    }
    if response.drag_stopped() {
        if let Some(text) = tab.terminal.selected_text() {
            ui.ctx().copy_text(text);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            tab.terminal.scroll((scroll / height) as i32);
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
                    if modifiers.ctrl && modifiers.shift && key == egui::Key::C {
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
