//! Rust端末の解析・画面モデル。描画はGUIのアダプターで行う。
pub use alacritty_terminal::term::TermMode;
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{cell::Flags, Config, Osc52},
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
    pub selected: bool,
}

pub struct MouseInput {
    pub button: u8,
    pub column: usize,
    pub row: usize,
    pub release: bool,
    pub motion: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// 受信出力のプレーンテキストログ。LFで確定した行だけを集め、GUI側が取り出して保存する。
#[derive(Default)]
struct LineLog {
    lines: VecDeque<String>,
    bytes: usize,
    dropped: usize,
}
const LOG_LINES: usize = 4096;
const LOG_BYTES: usize = 4 * 1024 * 1024;
const LOG_WRAPPED_ROWS: usize = 64;

pub struct Terminal {
    term: Term<Sink>,
    parser: Processor,
    sink: Sink,
    pub size: Size,
    log: Option<LineLog>,
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
            log: None,
        }
    }
    /// 受信出力のログ収集を切り替える。無効化すると未取出しの行も捨てる。
    pub fn set_line_log(&mut self, enabled: bool) {
        match (enabled, self.log.is_some()) {
            (true, false) => self.log = Some(LineLog::default()),
            (false, true) => self.log = None,
            _ => {}
        }
    }
    /// LFで確定した行と、上限超過で捨てた行数を返す。代替画面（vim等）の内容は含めない。
    pub fn take_log_lines(&mut self) -> (Vec<String>, usize) {
        match &mut self.log {
            Some(log) => {
                log.bytes = 0;
                (
                    log.lines.drain(..).collect(),
                    std::mem::take(&mut log.dropped),
                )
            }
            None => (vec![], 0),
        }
    }
    /// 終了時に、まだLFで確定していないカーソル行（最後のプロンプト等）を返す。
    pub fn pending_log_line(&self) -> Option<String> {
        if self.log.is_none() || self.term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }
        let text = self.cursor_logical_line();
        (!text.is_empty()).then_some(text)
    }
    fn row_text(&self, line: Line) -> String {
        let row = &self.term.grid()[line];
        let mut text = String::new();
        for column in 0..self.size.columns {
            let cell = &row[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter());
            }
        }
        text
    }
    /// カーソル行と、折返しでつながる直前の行をまとめた1行。
    fn cursor_logical_line(&self) -> String {
        let grid = self.term.grid();
        let top = -(grid.history_size() as i32);
        let last = Column(self.size.columns.saturating_sub(1));
        let mut start = grid.cursor.point.line;
        for _ in 0..LOG_WRAPPED_ROWS {
            let previous = Line(start.0 - 1);
            if previous.0 < top || !grid[previous][last].flags.contains(Flags::WRAPLINE) {
                break;
            }
            start = previous;
        }
        let mut text = String::new();
        let mut line = start;
        while line <= grid.cursor.point.line {
            text.push_str(&self.row_text(line));
            line = Line(line.0 + 1);
        }
        text.trim_end().to_owned()
    }
    fn log_cursor_line(&mut self) {
        if self.log.is_none() || self.term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let text = self.cursor_logical_line();
        let Some(log) = &mut self.log else { return };
        if log.lines.len() >= LOG_LINES || log.bytes + text.len() > LOG_BYTES {
            log.dropped += 1;
            return;
        }
        log.bytes += text.len();
        log.lines.push_back(text);
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if *byte == b'\n' {
                self.log_cursor_line();
            }
            self.parser
                .advance(&mut self.term, std::slice::from_ref(byte));
            // 結合文字は表示セルごとに32個まで。CSIで移動する前に上限を適用する。
            let grid = self.term.grid_mut();
            let point = grid.cursor.point;
            let mut column = point.column;
            if !grid.cursor.input_needs_wrap {
                column.0 = column.0.saturating_sub(1);
            }
            if grid[point.line][column]
                .flags
                .contains(Flags::WIDE_CHAR_SPACER)
            {
                column.0 = column.0.saturating_sub(1);
            }
            let cell = &mut grid[point.line][column];
            if cell.zerowidth().is_some_and(|extra| extra.len() > 32) {
                let extra: Vec<char> = cell.zerowidth().unwrap().iter().take(32).copied().collect();
                let c = cell.c;
                let flags = cell.flags;
                cell.clear_wide();
                cell.c = c;
                cell.flags = flags;
                for c in extra {
                    cell.push_zerowidth(c);
                }
            }
        }
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
                    selected: content
                        .selection
                        .is_some_and(|selection| selection.contains(indexed.point)),
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
    pub fn http_url_at(&self, row: usize, column: usize) -> Option<String> {
        let line = self.visible_text().get(row)?.clone();
        let mut offset = 0;
        for token in line.split_whitespace() {
            let start = line[offset..].find(token)? + offset;
            let end = start + token.len();
            offset = end;
            let first = unicode_width::UnicodeWidthStr::width(&line[..start]);
            let last = unicode_width::UnicodeWidthStr::width(&line[..end]);
            if column < first || column >= last {
                continue;
            }
            let token = token
                .trim_matches(['(', ')', '[', ']', '<', '>', '"', '\''])
                .trim_end_matches([',', ';', '.']);
            let url = url::Url::parse(token).ok()?;
            if !["http", "https"].contains(&url.scheme())
                || !url.username().is_empty()
                || url.password().is_some()
                || url.host_str().is_none()
            {
                return None;
            }
            return Some(url.to_string());
        }
        None
    }
    pub fn cursor_visible(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR)
    }
    pub fn mouse(&self, input: MouseInput) -> Vec<u8> {
        let MouseInput {
            button,
            column,
            row,
            release,
            motion,
            shift,
            alt,
            ctrl,
        } = input;
        if ![0, 1, 2, 3, 64, 65].contains(&button) {
            return vec![];
        }
        if motion && button == 3 && !self.term.mode().contains(TermMode::MOUSE_MOTION) {
            return vec![];
        }
        if shift || !self.term.mode().intersects(TermMode::MOUSE_MODE) {
            return vec![];
        }
        if motion
            && !self
                .term
                .mode()
                .intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
        {
            return vec![];
        }
        let code = button
            + if motion { 32 } else { 0 }
            + if alt { 8 } else { 0 }
            + if ctrl { 16 } else { 0 };
        let column = column.min(self.size.columns - 1) + 1;
        let row = row.min(self.size.rows - 1) + 1;
        if self.term.mode().contains(TermMode::SGR_MOUSE) {
            format!(
                "\x1b[<{code};{column};{row}{}",
                if release { 'm' } else { 'M' }
            )
            .into_bytes()
        } else if column < 224 && row < 224 {
            vec![
                27,
                b'[',
                b'M',
                32 + if release { 3 } else { code },
                32 + column as u8,
                32 + row as u8,
            ]
        } else {
            vec![]
        }
    }
    pub fn find_next(&mut self, query: &str, after: Option<(i32, usize)>) -> Option<(i32, usize)> {
        if query.is_empty() || query.len() > 1024 {
            return None;
        }
        let first = -(self.term.grid().history_size() as i32);
        let mut found = None;
        let mut wrap = None;
        for line in first..self.size.rows as i32 {
            let mut text = String::new();
            let mut columns = vec![];
            for column in 0..self.size.columns {
                let cell = &self.term.grid()[Line(line)][Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                columns.push((text.len(), column));
                text.push(cell.c);
                for c in cell.zerowidth().into_iter().flatten() {
                    text.push(*c);
                }
            }
            for (index, _) in text.match_indices(query) {
                let column = columns
                    .iter()
                    .rev()
                    .find(|(byte, _)| *byte <= index)
                    .map(|(_, column)| *column)
                    .unwrap_or(0);
                let position = (line, column);
                wrap.get_or_insert(position);
                if after.is_none_or(|after| position > after) {
                    found = Some(position);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let found = found.or(wrap)?;
        let offset = (-found.0).max(0) as usize;
        self.term.scroll_display(Scroll::Top);
        self.term.scroll_display(Scroll::Delta(
            -(self.term.grid().history_size().saturating_sub(offset) as i32),
        ));
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(Line(found.0), Column(found.1)),
            Side::Left,
        );
        selection.update(
            Point::new(
                Line(found.0),
                Column(
                    (found.1 + unicode_width::UnicodeWidthStr::width(query).saturating_sub(1))
                        .min(self.size.columns - 1),
                ),
            ),
            Side::Right,
        );
        self.term.selection = Some(selection);
        Some(found)
    }
    pub fn key(&self, key: &str, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
        let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
        if modifier > 1 {
            let suffix = match key {
                "ArrowUp" => Some('A'),
                "ArrowDown" => Some('B'),
                "ArrowRight" => Some('C'),
                "ArrowLeft" => Some('D'),
                "Home" => Some('H'),
                "End" => Some('F'),
                "F1" => Some('P'),
                "F2" => Some('Q'),
                "F3" => Some('R'),
                "F4" => Some('S'),
                _ => None,
            };
            if let Some(suffix) = suffix {
                return format!("\x1b[1;{modifier}{suffix}").into_bytes();
            }
            let number = match key {
                "Insert" => Some(2),
                "Delete" => Some(3),
                "PageUp" => Some(5),
                "PageDown" => Some(6),
                "F5" => Some(15),
                "F6" => Some(17),
                "F7" => Some(18),
                "F8" => Some(19),
                "F9" => Some(20),
                "F10" => Some(21),
                "F11" => Some(23),
                "F12" => Some(24),
                _ => None,
            };
            if let Some(number) = number {
                return format!("\x1b[{number};{modifier}~").into_bytes();
            }
        }
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
    fn combining_storage_is_bounded_and_wide_cells_stay_wide() {
        let mut terminal = Terminal::new(80, 24);
        terminal.feed("日".as_bytes());
        for _ in 0..10000 {
            terminal.feed("\u{0301}".as_bytes());
        }
        let cell = &terminal.term.grid()[Line(0)][Column(0)];
        assert_eq!(cell.zerowidth().unwrap().len(), 32);
        assert_eq!(cell.c, '日');
        assert!(cell.flags.contains(Flags::WIDE_CHAR));
        terminal.feed(b"\x1b[?25l");
        assert!(!terminal.cursor_visible());
    }
    #[test]
    fn search_finds_unicode_history_and_selects_full_cell_width() {
        let mut terminal = Terminal::new(20, 2);
        terminal.feed("日本\r\nline2\r\nline3\r\n".as_bytes());
        let found = terminal.find_next("日本", None).unwrap();
        assert!(found.0 < 0);
        assert_eq!(terminal.selected_text().as_deref(), Some("日本"));
        assert!(terminal.cells().iter().any(|cell| cell.selected));
    }
    #[test]
    fn modified_keys_and_sgr_mouse_have_correct_sequences() {
        let mut terminal = Terminal::new(80, 24);
        assert_eq!(terminal.key("ArrowLeft", true, false, false), b"\x1b[1;5D");
        terminal.feed(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            terminal.mouse(MouseInput {
                button: 0,
                column: 2,
                row: 3,
                release: false,
                motion: false,
                shift: false,
                alt: false,
                ctrl: false
            }),
            b"\x1b[<0;3;4M"
        );
        assert_eq!(
            terminal.mouse(MouseInput {
                button: 0,
                column: 2,
                row: 3,
                release: true,
                motion: false,
                shift: false,
                alt: false,
                ctrl: false
            }),
            b"\x1b[<0;3;4m"
        );
        assert!(terminal
            .mouse(MouseInput {
                button: 0,
                column: 2,
                row: 3,
                release: false,
                motion: false,
                shift: true,
                alt: false,
                ctrl: false
            })
            .is_empty());
    }
    #[test]
    fn only_visible_http_urls_can_be_activated() {
        let mut terminal = Terminal::new(80, 4);
        terminal.feed(b"https://example.com/page\r\nfile:///secret\r\n\x1b]8;;https://hidden.example\x07label\x1b]8;;\x07");
        assert_eq!(
            terminal.http_url_at(0, 3).as_deref(),
            Some("https://example.com/page")
        );
        assert!(terminal.http_url_at(1, 3).is_none());
        assert!(terminal.http_url_at(2, 3).is_none());
    }
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
    fn line_log_records_committed_lines_but_not_alternate_screen() {
        let mut terminal = Terminal::new(10, 4);
        terminal.feed(b"before-log\r\n");
        terminal.set_line_log(true);
        terminal.feed("progress 1\rprogress 2\r\n日本語の長い行が折り返す\r\n".as_bytes());
        terminal.feed(b"\x1b[?1049hvim screen\r\nmore\r\n\x1b[?1049l");
        terminal.feed(b"$ ");
        let (lines, dropped) = terminal.take_log_lines();
        assert_eq!(lines, ["progress 2", "日本語の長い行が折り返す"]);
        assert_eq!(dropped, 0);
        assert_eq!(terminal.pending_log_line().as_deref(), Some("$"));
        terminal.set_line_log(false);
        terminal.feed(b"off\r\n");
        assert!(terminal.take_log_lines().0.is_empty());
    }
    #[test]
    fn line_log_is_bounded_and_counts_dropped_lines() {
        let mut terminal = Terminal::new(20, 4);
        terminal.set_line_log(true);
        for _ in 0..LOG_LINES + 3 {
            terminal.feed(b"x\r\n");
        }
        let (lines, dropped) = terminal.take_log_lines();
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(dropped, 3);
        terminal.feed(b"y\r\n");
        assert_eq!(terminal.take_log_lines(), (vec!["y".to_owned()], 0));
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
