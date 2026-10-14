//! Rust端末の解析・画面モデル。描画はGUIのアダプターで行う。
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{cell::Flags, Config, Osc52, TermMode},
    vte::ansi::{Color, NamedColor, Processor},
    Term,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy)]
pub struct Size {
    pub columns: usize,
    pub rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone)]
struct Sink(Arc<Mutex<VecDeque<Vec<u8>>>>);
impl EventListener for Sink {
    fn send_event(&self, event: Event) {
        // リモートからのクリップボード読取/変更やtitleによる機能起動を許可しない。
        if let Event::PtyWrite(text) = event {
            if let Ok(mut queue) = self.0.lock() {
                if queue.len() < 64 {
                    queue.push_back(text.into_bytes());
                }
            }
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CellColor {
    Rgb { r: u8, g: u8, b: u8 },
    Indexed { index: u8 },
    Named { index: u16 },
}
impl From<Color> for CellColor {
    fn from(color: Color) -> Self {
        match color {
            Color::Spec(rgb) => Self::Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            },
            Color::Indexed(index) => Self::Indexed { index },
            Color::Named(name) => Self::Named { index: name as u16 },
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Cell {
    pub row: i32,
    pub column: usize,
    pub text: String,
    pub foreground: CellColor,
    pub background: CellColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
}

pub struct Terminal {
    term: Term<Sink>,
    parser: Processor,
    sink: Sink,
    pub size: Size,
}
impl Terminal {
    pub fn new(columns: usize, rows: usize) -> Self {
        let size = Size {
            columns: columns.clamp(1, 1024),
            rows: rows.clamp(1, 512),
        };
        let sink = Sink(Arc::new(Mutex::new(VecDeque::new())));
        let term = Term::new(
            Config {
                scrolling_history: 5000,
                osc52: Osc52::Disabled,
                kitty_keyboard: false,
                ..Default::default()
            },
            &size,
            sink.clone(),
        );
        Self {
            term,
            parser: Processor::default(),
            sink,
            size,
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }
    pub fn resize(&mut self, columns: usize, rows: usize) {
        self.size = Size {
            columns: columns.clamp(1, 1024),
            rows: rows.clamp(1, 512),
        };
        self.term.resize(self.size);
    }
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }
    pub fn bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }
    pub fn replies(&mut self) -> Vec<Vec<u8>> {
        self.sink
            .0
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
    pub fn modes(&self) -> TermMode {
        *self.term.mode()
    }
    pub fn cursor(&self) -> (i32, usize) {
        let content = self.term.renderable_content();
        (
            content.cursor.point.line.0 + content.display_offset as i32,
            content.cursor.point.column.0,
        )
    }
    pub fn cells(&self) -> Vec<Cell> {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        content
            .display_iter
            .filter(|indexed| {
                !indexed
                    .cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            })
            .map(|indexed| {
                let cell = indexed.cell;
                let mut text = cell.c.to_string();
                if let Some(extra) = cell.zerowidth() {
                    for c in extra {
                        text.push(*c);
                    }
                }
                Cell {
                    row: indexed.point.line.0 + offset,
                    column: indexed.point.column.0,
                    text,
                    foreground: cell.fg.into(),
                    background: cell.bg.into(),
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.contains(Flags::UNDERLINE),
                    inverse: cell.flags.contains(Flags::INVERSE),
                    wide: cell.flags.contains(Flags::WIDE_CHAR),
                }
            })
            .collect()
    }
    pub fn visible_text(&self) -> Vec<String> {
        let mut lines = vec![String::new(); self.size.rows];
        for cell in self.cells() {
            if cell.row >= 0 && (cell.row as usize) < lines.len() {
                lines[cell.row as usize].push_str(&cell.text);
            }
        }
        lines
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect()
    }
    pub fn select(&mut self, start: (i32, usize), end: (i32, usize)) {
        let offset = self.term.grid().display_offset() as i32;
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(Line(start.0 - offset), Column(start.1)),
            Side::Left,
        );
        selection.update(Point::new(Line(end.0 - offset), Column(end.1)), Side::Right);
        self.term.selection = Some(selection);
    }
    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }
    pub fn paste(&self, text: &str) -> Vec<u8> {
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
        } else {
            text.replace('\n', "\r").into_bytes()
        }
    }
    pub fn key(&self, key: &str, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
        let app = self.term.mode().contains(TermMode::APP_CURSOR);
        let text = match key {
            "Enter" => "\r".into(),
            "Tab" => {
                if shift {
                    "\x1b[Z".into()
                } else {
                    "\t".into()
                }
            }
            "Backspace" => "\x7f".into(),
            "Escape" => "\x1b".into(),
            "ArrowUp" => {
                if app {
                    "\x1bOA".into()
                } else {
                    "\x1b[A".into()
                }
            }
            "ArrowDown" => {
                if app {
                    "\x1bOB".into()
                } else {
                    "\x1b[B".into()
                }
            }
            "ArrowRight" => {
                if app {
                    "\x1bOC".into()
                } else {
                    "\x1b[C".into()
                }
            }
            "ArrowLeft" => {
                if app {
                    "\x1bOD".into()
                } else {
                    "\x1b[D".into()
                }
            }
            "Home" => "\x1b[H".into(),
            "End" => "\x1b[F".into(),
            "Insert" => "\x1b[2~".into(),
            "Delete" => "\x1b[3~".into(),
            "F5" => "\x1b[15~".into(),
            "F6" => "\x1b[17~".into(),
            "F7" => "\x1b[18~".into(),
            "F8" => "\x1b[19~".into(),
            "F9" => "\x1b[20~".into(),
            "F10" => "\x1b[21~".into(),
            "F11" => "\x1b[23~".into(),
            "F12" => "\x1b[24~".into(),
            "[" if ctrl => "\x1b".into(),
            " " if ctrl => "\0".into(),
            "PageUp" => "\x1b[5~".into(),
            "PageDown" => "\x1b[6~".into(),
            "F1" => "\x1bOP".into(),
            "F2" => "\x1bOQ".into(),
            "F3" => "\x1bOR".into(),
            "F4" => "\x1bOS".into(),
            text if ctrl && text.len() == 1 && text.as_bytes()[0].is_ascii_alphabetic() => {
                ((text.as_bytes()[0].to_ascii_uppercase() - b'A' + 1) as char).to_string()
            }
            text => text.to_owned(),
        };
        if alt {
            format!("\x1b{text}").into_bytes()
        } else {
            text.into_bytes()
        }
    }
    pub fn default_foreground_index() -> u16 {
        NamedColor::Foreground as u16
    }
    pub fn default_background_index() -> u16 {
        NamedColor::Background as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_split_wide_text_and_alternate_screen() {
        let mut terminal = Terminal::new(80, 24);
        let bytes = "日本語".as_bytes();
        for byte in bytes {
            terminal.feed(&[*byte]);
        }
        assert_eq!(terminal.visible_text()[0], "日本語");
        assert_eq!(terminal.cursor(), (0, 6));
        terminal.feed(b"\x1b[?1049hother\x1b[?1049l");
        assert_eq!(terminal.visible_text()[0], "日本語");
        terminal.resize(100, 30);
        assert_eq!(terminal.visible_text()[0], "日本語");
    }
    #[test]
    fn clipboard_is_never_loaded_or_written_by_remote_sequences() {
        let mut terminal = Terminal::new(80, 24);
        terminal.feed(b"\x1b]52;c;ZXZpbA==\x07\x1b]52;c;?\x07");
        assert!(terminal.replies().is_empty());
        terminal.feed(b"\x1b]8;;file:///secret\x07label\x1b]8;;\x07");
        assert_eq!(terminal.visible_text()[0], "label");
    }
    #[test]
    fn application_cursor_bracketed_paste_and_selection() {
        let mut terminal = Terminal::new(80, 24);
        assert_eq!(terminal.key("c", true, false, false), b"\x03");
        terminal.feed(b"\x1b[?1h\x1b[?2004hhello");
        assert_eq!(terminal.key("ArrowUp", false, false, false), b"\x1bOA");
        assert_eq!(terminal.paste("test"), b"\x1b[200~test\x1b[201~");
        terminal.select((0, 0), (0, 4));
        assert_eq!(terminal.selected_text().as_deref(), Some("hello"));
    }
}
