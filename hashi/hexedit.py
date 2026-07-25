"""16 進エディタ(Issue #122)。

ゲームサーバーのデータファイル(`.dat` 等)のように**テキストとして開けない
バイナリ**を、Visual Studio や HxD を入れずに Hashi 単体で覗いて直せるようにする。

設計方針:

- **上書き専用(サイズ不変)**。挿入・削除はできない。バイナリ形式は多くが
  オフセットやレングスを内部に持つため、長さが変わると壊れる。事故を防ぐため
  「1 バイトを別の値に置き換える」ことだけを許す。
- **バイト単位で忠実**。読み書きは常に `rb`/`wb`。改行変換もエンコード変換も
  一切しない(テキストモードで開くと 0x0D が失われてファイルが壊れる)。
- 表示は `オフセット | 16 進 16 バイト | ASCII` の 3 カラム。等幅フォントで
  自前描画せず QPlainTextEdit を使うと編集位置の管理が複雑になるため、
  **QAbstractScrollArea へ自前描画**する(ターミナルと同じ方針)。
"""
from __future__ import annotations

from PySide6.QtCore import QSize, Qt, Signal
from PySide6.QtGui import QColor, QFont, QFontMetricsF, QPainter
from PySide6.QtWidgets import QAbstractScrollArea

from . import style

BYTES_PER_ROW = 16

SNIFF_BYTES = 8192      # 判定に使う先頭バイト数


def looks_binary(sample: bytes) -> bool:
    """先頭バイト列を見て「バイナリか」を判定する(Issue #122)。

    拡張子だけでは `.dat` のようにテキストともバイナリとも取れるものを
    正しく分岐できないため、**中身で決める**。判定は保守的に:

    - BOM 付き UTF-8 / UTF-16 はテキスト
    - NUL バイトを含めばバイナリ(UTF-16 は BOM で先に救済済み)
    - UTF-8 としてデコードできず、latin-1 でも制御文字が多ければバイナリ
    """
    if not sample:
        return False                    # 空ファイルはテキスト扱い(編集できる)
    if sample.startswith((b"\xef\xbb\xbf", b"\xff\xfe", b"\xfe\xff")):
        return False
    if b"\x00" in sample:
        return True
    try:
        sample.decode("utf-8")
    except UnicodeDecodeError:
        # 途中で切れた多バイト文字は誤判定になるので末尾を削って再試行する
        for cut in range(1, 4):
            try:
                sample[:-cut].decode("utf-8")
                break
            except UnicodeDecodeError:
                continue
        else:
            return True
    # タブ・改行・復帰・改頁以外の制御文字が多ければバイナリ
    allowed = {0x09, 0x0A, 0x0D, 0x0C, 0x1B}
    ctrl = sum(1 for b in sample if b < 0x20 and b not in allowed)
    return ctrl / len(sample) > 0.05


def sniff_file(path: str) -> bool:
    """ファイルの先頭を読んでバイナリか判定する。読めなければテキスト扱い。"""
    try:
        with open(path, "rb") as f:
            return looks_binary(f.read(SNIFF_BYTES))
    except OSError:
        return False


class HexEdit(QAbstractScrollArea):
    """バイト列の 16 進表示 + 上書き編集ウィジェット。"""

    modified_changed = Signal(bool)
    cursor_moved = Signal(int)          # 現在のオフセット

    def __init__(self, data: bytes = b"", parent=None, font_size: int = 12):
        super().__init__(parent)
        self._data = bytearray(data)
        self._original = bytes(data)
        self._cursor = 0                # バイトオフセット
        self._nibble = 0                # 0=上位 4bit, 1=下位 4bit
        self._in_ascii = False          # ASCII 欄を編集中か
        self._modified = False

        self._font = QFont()
        self._font.setFamilies(["Consolas", "Menlo", "DejaVu Sans Mono", "Monospace"])
        self._font.setPointSize(font_size)
        self._font.setStyleHint(QFont.Monospace)
        self._metrics(self._font)

        self.setFocusPolicy(Qt.StrongFocus)
        self.viewport().setCursor(Qt.IBeamCursor)
        self._update_scroll()

    # ---- 計測 / レイアウト ---------------------------------------------------
    def _metrics(self, font: QFont) -> None:
        fm = QFontMetricsF(font)
        self._cw = fm.horizontalAdvance("0") or 8.0
        self._ch = fm.height() or 14.0
        self._ascent = fm.ascent()
        # 桁: オフセット 8 + 2 空白 | 16 進 (2+1)*16 | 2 空白 | ASCII 16
        self._x_hex = self._cw * 10
        self._x_ascii = self._x_hex + self._cw * (3 * BYTES_PER_ROW + 1)

    def set_font_size(self, size: int) -> None:
        self._font.setPointSize(max(6, int(size)))
        self._metrics(self._font)
        self._update_scroll()
        self.viewport().update()

    def _rows(self) -> int:
        if not self._data:
            return 1
        return (len(self._data) + BYTES_PER_ROW - 1) // BYTES_PER_ROW

    def _visible_rows(self) -> int:
        return max(1, int(self.viewport().height() / self._ch))

    def _update_scroll(self) -> None:
        bar = self.verticalScrollBar()
        bar.setPageStep(self._visible_rows())
        bar.setRange(0, max(0, self._rows() - self._visible_rows()))
        self.horizontalScrollBar().setRange(
            0, max(0, int(self._x_ascii + self._cw * (BYTES_PER_ROW + 2)
                          - self.viewport().width())))

    def resizeEvent(self, ev):
        super().resizeEvent(ev)
        self._update_scroll()

    def sizeHint(self) -> QSize:
        return QSize(int(self._x_ascii + self._cw * (BYTES_PER_ROW + 2)),
                     int(self._ch * 25))

    # ---- データ -------------------------------------------------------------
    def data(self) -> bytes:
        return bytes(self._data)

    def is_modified(self) -> bool:
        return self._modified

    def mark_saved(self) -> None:
        self._original = bytes(self._data)
        self._set_modified(False)
        self.viewport().update()

    def _set_modified(self, flag: bool) -> None:
        if flag != self._modified:
            self._modified = flag
            self.modified_changed.emit(flag)

    def cursor_offset(self) -> int:
        return self._cursor

    def changed_offsets(self) -> set[int]:
        """元データと異なるバイトのオフセット集合(色分け用)。"""
        return {i for i, b in enumerate(self._data)
                if i >= len(self._original) or self._original[i] != b}

    # ---- 描画 ---------------------------------------------------------------
    def paintEvent(self, ev):
        painter = QPainter(self.viewport())
        painter.setFont(self._font)
        painter.fillRect(self.viewport().rect(), QColor(style.BG))

        top = self.verticalScrollBar().value()
        x_off = -self.horizontalScrollBar().value()
        changed = self.changed_offsets()
        c_off = QColor(style.FG_MUTED)
        c_fg = QColor(style.FG)
        c_mod = QColor(style.WARN)
        c_sel = QColor(style.SEL)

        for row in range(self._visible_rows() + 1):
            line = top + row
            if line >= self._rows():
                break
            y = self._ch * row
            baseline = y + self._ascent
            base = line * BYTES_PER_ROW

            painter.setPen(c_off)
            painter.drawText(x_off + 0.0, baseline, f"{base:08X}")

            for col in range(BYTES_PER_ROW):
                idx = base + col
                if idx >= len(self._data):
                    break
                byte = self._data[idx]
                hx = self._x_hex + self._cw * 3 * col
                ax = self._x_ascii + self._cw * col
                # カーソル位置の下敷き
                if idx == self._cursor:
                    painter.fillRect(
                        int(x_off + (ax if self._in_ascii else hx)), int(y),
                        int(self._cw * (1 if self._in_ascii else 2)),
                        int(self._ch), c_sel)
                painter.setPen(c_mod if idx in changed else c_fg)
                painter.drawText(x_off + hx, baseline, f"{byte:02X}")
                ch = chr(byte) if 0x20 <= byte < 0x7F else "."
                painter.drawText(x_off + ax, baseline, ch)
        painter.end()

    # ---- カーソル移動 -------------------------------------------------------
    def _ensure_visible(self) -> None:
        row = self._cursor // BYTES_PER_ROW
        bar = self.verticalScrollBar()
        if row < bar.value():
            bar.setValue(row)
        elif row >= bar.value() + self._visible_rows():
            bar.setValue(row - self._visible_rows() + 1)

    def _move_to(self, offset: int) -> None:
        if not self._data:
            return
        self._cursor = max(0, min(len(self._data) - 1, offset))
        self._nibble = 0
        self._ensure_visible()
        self.cursor_moved.emit(self._cursor)
        self.viewport().update()

    def mousePressEvent(self, ev):
        if not self._data:
            return
        pos = ev.position()
        x = pos.x() + self.horizontalScrollBar().value()
        line = self.verticalScrollBar().value() + int(pos.y() / self._ch)
        if x >= self._x_ascii:
            self._in_ascii = True
            col = int((x - self._x_ascii) / self._cw)
        else:
            self._in_ascii = False
            col = int((x - self._x_hex) / (self._cw * 3))
        col = max(0, min(BYTES_PER_ROW - 1, col))
        self._move_to(line * BYTES_PER_ROW + col)

    def keyPressEvent(self, ev):
        key = ev.key()
        if key == Qt.Key_Right:
            self._move_to(self._cursor + 1)
        elif key == Qt.Key_Left:
            self._move_to(self._cursor - 1)
        elif key == Qt.Key_Down:
            self._move_to(self._cursor + BYTES_PER_ROW)
        elif key == Qt.Key_Up:
            self._move_to(self._cursor - BYTES_PER_ROW)
        elif key == Qt.Key_PageDown:
            self._move_to(self._cursor + BYTES_PER_ROW * self._visible_rows())
        elif key == Qt.Key_PageUp:
            self._move_to(self._cursor - BYTES_PER_ROW * self._visible_rows())
        elif key == Qt.Key_Home:
            self._move_to(0)
        elif key == Qt.Key_End:
            self._move_to(len(self._data) - 1)
        elif key == Qt.Key_Tab:
            self._in_ascii = not self._in_ascii
            self._nibble = 0
            self.viewport().update()
        else:
            self._handle_input(ev.text())

    def _handle_input(self, text: str) -> None:
        """1 文字の入力を上書きとして適用する(サイズは変えない)。"""
        if not text or not self._data:
            return
        ch = text[0]
        if self._in_ascii:
            code = ord(ch)
            if code > 0xFF:
                return          # 1 バイトで表せない文字は無視
            self._write_byte(self._cursor, code)
            self._move_to(self._cursor + 1)
            return
        if ch not in "0123456789abcdefABCDEF":
            return
        val = int(ch, 16)
        cur = self._data[self._cursor]
        if self._nibble == 0:
            self._write_byte(self._cursor, (val << 4) | (cur & 0x0F))
            self._nibble = 1
            self.viewport().update()
        else:
            self._write_byte(self._cursor, (cur & 0xF0) | val)
            self._move_to(self._cursor + 1)

    def _write_byte(self, offset: int, value: int) -> None:
        if 0 <= offset < len(self._data) and self._data[offset] != value:
            self._data[offset] = value
            self._set_modified(bytes(self._data) != self._original)
