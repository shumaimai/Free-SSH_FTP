"""内蔵コードエディタ。

リモートファイルを一時 DL → このエディタで編集 → Ctrl+S でサーバーへ書き戻し。
ローカルファイルもメモ帳のように開き、Ctrl+S でそのまま保存できる。
権限が足りなければ(権限無視スイッチが ON なら)自動で権限を付けて保存する。

機能: 行番号、現在行ハイライト、拡張子ベースの簡易シンタックスハイライト、
     検索/置換 (Ctrl+F / Ctrl+H)、行へ移動 (Ctrl+G)、折返しトグル、
     エンコード/改行/言語のステータス表示、未保存の警告。
     対応拡張子は `editlang.py` が単一ソース。
"""
from __future__ import annotations

import os
import re

from PySide6.QtCore import QRect, QSize, Qt, Signal
from PySide6.QtGui import (
    QColor,
    QFont,
    QKeySequence,
    QPainter,
    QPalette,
    QShortcut,
    QSyntaxHighlighter,
    QTextCharFormat,
    QTextCursor,
    QTextDocument,
    QTextFormat,
)
from PySide6.QtWidgets import (
    QFileDialog,
    QInputDialog,
    QLineEdit,
    QMainWindow,
    QMessageBox,
    QPlainTextEdit,
    QStatusBar,
    QTextEdit,
    QToolBar,
    QWidget,
)

from . import style
from .editlang import language_for, should_edit_internally
from .hexedit import SNIFF_BYTES, HexEdit, looks_binary
from .windowfit import fit_to_screen

# 内蔵エディタで開くファイルの上限(リモート/ローカル共通)
EDITOR_MAX_BYTES = 8 * 1024 * 1024


def _decode_text(raw: bytes) -> tuple[str, str, str]:
    """バイト列をテキストへ復号し、(本文, エンコード, 改行スタイル) を返す。

    Issue #122 で、保存時にファイルを書き換えてしまう 2 点を直すために導入した。

    1. **改行スタイルを保持する**。以前はテキストモードで開いていたため
       universal newlines が効いて `\\r\\n` が `\\n` になり、保存すると
       改行が LF へ書き換わっていた(Windows のゲームサーバーの CRLF 設定が
       まるごと差分になる)。元のスタイルを覚えて保存時に復元する。
    2. **BOM を勝手に増やさない**。`utf-8-sig` は BOM が無くてもデコードに
       成功するが、同じ名前でエンコードすると BOM を**付けてしまう**。
       BOM 付きだったファイルだけ `utf-8-sig` を使う。
    """
    if raw.startswith(b"\xef\xbb\xbf"):
        enc = "utf-8-sig"
    elif raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        enc = "utf-16"
    else:
        enc = "utf-8"
    try:
        text = raw.decode(enc)
    except (UnicodeDecodeError, UnicodeError):
        # 1 バイト系として扱う(latin-1 は必ず成功し、バイト値を保持する)
        enc, text = "latin-1", raw.decode("latin-1")
    # 改行スタイル: CRLF が 1 つでもあれば CRLF 優先(混在は CRLF に寄せる)
    if "\r\n" in text:
        newline = "\r\n"
    elif "\r" in text:
        newline = "\r"
    else:
        newline = "\n"
    # 編集中は LF に正規化して扱う(Qt 側も LF)
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    return normalized, enc, newline


# ---- シンタックスハイライト規則 -------------------------------------------------
# 色 (One Half Dark 系)
C_KEYWORD = "#c678dd"
C_STRING = "#98c379"
C_COMMENT = "#7f848e"
C_NUMBER = "#d19a66"
C_FUNC = "#61afef"
C_DECORATOR = "#e5c07b"

_PY_KW = (
    "def class return if elif else for while break continue import from as pass "
    "with try except finally raise lambda yield global nonlocal assert del in is "
    "not and or None True False async await self match case"
).split()
_C_KW = (
    "int char float double void long short unsigned signed struct union enum "
    "return if else for while do break continue switch case default goto sizeof "
    "typedef const static extern volatile register auto include define ifdef "
    "ifndef endif pragma class public private protected virtual namespace using "
    "template new delete this true false nullptr bool"
).split()
_JS_KW = (
    "function return if else for while break continue var let const new class "
    "extends import export from default async await try catch finally throw "
    "typeof instanceof this null undefined true false switch case do yield of in "
    "delete void super static get set"
).split()
_SH_KW = (
    "if then else elif fi for while do done case esac function return in select "
    "until break continue local export readonly declare source echo cd exit set "
    "unset trap eval exec test"
).split()
_RB_KW = (
    "def class module end if elsif else unless while until for do begin rescue "
    "ensure retry return yield break next super self nil true false and or not "
    "in when then require load include extend attr_reader attr_writer attr_accessor"
).split()
_PHP_KW = (
    "function class public private protected static return if else elseif endif "
    "while endfor foreach as switch case break continue echo print require "
    "include namespace use trait interface extends implements new true false "
    "null array string int float bool void"
).split()
_LUA_KW = (
    "function end if then else elseif while do repeat until for in local return "
    "break true false nil and or not"
).split()
_SQL_KW = (
    "select from where join inner left right outer on group by order having limit "
    "offset insert into values update set delete create drop alter table index view "
    "primary key foreign references not null default and or as distinct union all "
    "case when then else end exists between like in is"
).split()
_CSS_KW = (
    "color background margin padding border width height display position top left "
    "right bottom font-size font-weight text-align important inherit none block "
    "inline flex grid absolute relative fixed static"
).split()
_MK_KW = (
    "html head body div span p a img table tr td th ul ol li h1 h2 h3 h4 h5 h6 "
    "script style link meta title doctype xmlns"
).split()


def _lang_for(path: str) -> str:
    """後方互換。新規コードは editlang.language_for を使う。"""
    return language_for(path)


class Highlighter(QSyntaxHighlighter):
    """拡張子に応じた軽量ハイライタ(依存ライブラリなし)。"""

    def __init__(self, document, lang: str):
        super().__init__(document)
        self.rules: list[tuple[re.Pattern, QTextCharFormat]] = []
        self.lang = lang
        self._build()

    @staticmethod
    def _fmt(color: str, bold=False, italic=False) -> QTextCharFormat:
        f = QTextCharFormat()
        f.setForeground(QColor(color))
        if bold:
            f.setFontWeight(QFont.Bold)
        if italic:
            f.setFontItalic(True)
        return f

    def _build(self):
        lang = self.lang
        kw_map = {
            "python": _PY_KW, "c": _C_KW, "js": _JS_KW, "shell": _SH_KW,
            "ruby": _RB_KW, "php": _PHP_KW, "lua": _LUA_KW, "sql": _SQL_KW,
            "css": _CSS_KW, "markup": _MK_KW,
        }
        kw = kw_map.get(lang)
        if kw:
            kw_fmt = self._fmt(C_KEYWORD, bold=True)
            self.rules.append(
                (re.compile(r"\b(" + "|".join(kw) + r")\b"), kw_fmt))
        # 関数呼び出し / 定義名
        if lang in ("python", "c", "js", "ruby", "php", "lua", "sql"):
            self.rules.append(
                (re.compile(r"\b([A-Za-z_]\w*)\s*(?=\()"), self._fmt(C_FUNC)))
        # 数値
        self.rules.append(
            (re.compile(r"\b\d+\.?\d*([eE][+-]?\d+)?\b"), self._fmt(C_NUMBER)))
        # 文字列 (シングル/ダブル/バッククォート)
        str_fmt = self._fmt(C_STRING)
        self.rules.append((re.compile(r'"[^"\\]*(\\.[^"\\]*)*"'), str_fmt))
        self.rules.append((re.compile(r"'[^'\\]*(\\.[^'\\]*)*'"), str_fmt))
        if lang in ("js", "shell", "ruby", "php", "lua"):
            self.rules.append((re.compile(r"`[^`\\]*(\\.[^`\\]*)*`"), str_fmt))
        # デコレータ / プリプロセッサ
        if lang == "python":
            self.rules.append(
                (re.compile(r"^\s*@\w+"), self._fmt(C_DECORATOR)))
        if lang == "c":
            self.rules.append(
                (re.compile(r"^\s*#\w+"), self._fmt(C_DECORATOR)))
        if lang == "markup":
            self.rules.append(
                (re.compile(r"</?[\w:.-]+"), self._fmt(C_FUNC)))
            self.rules.append(
                (re.compile(r"<!--.*?-->"), self._fmt(C_COMMENT, italic=True)))
        if lang == "css":
            self.rules.append(
                (re.compile(r"#[\w-]+"), self._fmt(C_DECORATOR)))
            self.rules.append(
                (re.compile(r"\.[\w-]+"), self._fmt(C_FUNC)))
        # コメント (行コメントのみ簡易対応; 末尾で上書き)
        cmt_fmt = self._fmt(C_COMMENT, italic=True)
        if lang in ("python", "shell", "conf", "ruby", "php", "lua", "sql"):
            self._line_comment = (re.compile(r"#.*$"), cmt_fmt)
        elif lang in ("c", "js", "css", "json"):
            self._line_comment = (re.compile(r"//.*$"), cmt_fmt)
        elif lang == "markup":
            self._line_comment = None
        else:
            self._line_comment = None
        self._cmt_fmt = cmt_fmt
        # C/JS/CSS ブロックコメント用
        self._block = lang in ("c", "js", "css", "json")

    def highlightBlock(self, text: str):
        for pattern, fmt in self.rules:
            for m in pattern.finditer(text):
                self.setFormat(m.start(), m.end() - m.start(), fmt)
        # 行コメントは最後に上書き(文字列内 # を避けるため簡易に末尾優先)
        if self._line_comment is not None:
            pat, fmt = self._line_comment
            for m in pat.finditer(text):
                # 直前がクォート内でないかの厳密判定は省略(実用上十分)
                self.setFormat(m.start(), len(text) - m.start(), fmt)
        # C/JS ブロックコメント /* */
        if self._block:
            self._apply_block_comment(text)

    def _apply_block_comment(self, text: str):
        start_expr, end_expr = "/*", "*/"
        self.setCurrentBlockState(0)
        start = 0
        if self.previousBlockState() != 1:
            start = text.find(start_expr)
        while start >= 0:
            end = text.find(end_expr, start)
            if end == -1:
                self.setCurrentBlockState(1)
                length = len(text) - start
            else:
                length = end - start + len(end_expr)
            self.setFormat(start, length, self._cmt_fmt)
            start = text.find(start_expr, start + length)


# ---- 行番号エリア ---------------------------------------------------------------
class _LineNumberArea(QWidget):
    def __init__(self, editor):
        super().__init__(editor)
        self.editor = editor

    def sizeHint(self):
        return QSize(self.editor.line_number_width(), 0)

    def paintEvent(self, ev):
        self.editor.paint_line_numbers(ev)


class CodeEdit(QPlainTextEdit):
    """行番号 + 現在行ハイライト付きのエディタ本体。"""

    def __init__(self, parent=None, font_size=12, tab_width=4):
        super().__init__(parent)
        f = QFont()
        f.setFamilies(["Consolas", "Cascadia Mono", "MS Gothic", "Monospace"])
        f.setStyleHint(QFont.Monospace)
        f.setPointSize(font_size)
        self.setFont(f)
        self.setTabStopDistance(
            self.fontMetrics().horizontalAdvance(" ") * tab_width)
        self._tab_width = tab_width
        self.setLineWrapMode(QPlainTextEdit.NoWrap)

        self._lna = _LineNumberArea(self)
        self.blockCountChanged.connect(self._update_lna_width)
        self.updateRequest.connect(self._update_lna)
        self.cursorPositionChanged.connect(self._highlight_current_line)
        self._update_lna_width()
        self._highlight_current_line()

        pal = self.palette()
        pal.setColor(QPalette.Base, QColor(style.BG_BASE))
        pal.setColor(QPalette.Text, QColor(style.FG))
        self.setPalette(pal)

    def line_number_width(self) -> int:
        digits = max(3, len(str(max(1, self.blockCount()))))
        return 12 + self.fontMetrics().horizontalAdvance("9") * digits

    def _update_lna_width(self):
        self.setViewportMargins(self.line_number_width(), 0, 0, 0)

    def _update_lna(self, rect, dy):
        if dy:
            self._lna.scroll(0, dy)
        else:
            self._lna.update(0, rect.y(), self._lna.width(), rect.height())
        if rect.contains(self.viewport().rect()):
            self._update_lna_width()

    def resizeEvent(self, ev):
        super().resizeEvent(ev)
        cr = self.contentsRect()
        self._lna.setGeometry(QRect(cr.left(), cr.top(),
                                    self.line_number_width(), cr.height()))

    def _highlight_current_line(self):
        sel = QTextEdit.ExtraSelection()
        sel.format.setBackground(QColor("#242a35"))
        sel.format.setProperty(QTextFormat.FullWidthSelection, True)
        sel.cursor = self.textCursor()
        sel.cursor.clearSelection()
        self.setExtraSelections([sel])

    def paint_line_numbers(self, ev):
        painter = QPainter(self._lna)
        painter.fillRect(ev.rect(), QColor("#181c23"))
        block = self.firstVisibleBlock()
        num = block.blockNumber()
        top = self.blockBoundingGeometry(block).translated(
            self.contentOffset()).top()
        bottom = top + self.blockBoundingRect(block).height()
        cur_line = self.textCursor().blockNumber()
        while block.isValid() and top <= ev.rect().bottom():
            if block.isVisible() and bottom >= ev.rect().top():
                painter.setPen(QColor("#61afef") if num == cur_line
                               else QColor("#4b5263"))
                painter.drawText(
                    0, int(top), self._lna.width() - 6,
                    self.fontMetrics().height(),
                    Qt.AlignRight, str(num + 1))
            block = block.next()
            top = bottom
            bottom = top + self.blockBoundingRect(block).height()
            num += 1


class EditorWindow(QMainWindow):
    """1 ファイル分の編集ウィンドウ。

    - リモート: `remote_path` + `save_callback` で SFTP へ書き戻す。
    - ローカル: `local_path` に直接保存(メモ帳のように使える)。
    """

    closed = Signal(object)  # self

    def __init__(self, local_path: str, settings, *,
                 remote_path: str | None = None,
                 save_callback=None,
                 parent=None,
                 new_file: bool = False):
        super().__init__(parent)
        self.local_path = local_path or ""
        self.remote_path = remote_path
        self._save_cb = save_callback
        self._saving = False
        self._untitled = new_file or not self.local_path
        lang_path = remote_path or local_path or "無題.txt"

        if new_file or not self.local_path:
            raw = b""
            self.is_hex = False
        else:
            with open(local_path, "rb") as f:
                raw = f.read()
            self.is_hex = looks_binary(raw[:SNIFF_BYTES])

        if self.is_hex:
            self.editor = None
            self.hex = HexEdit(raw, font_size=settings.get("editor_font_size"))
            self.setCentralWidget(self.hex)
            self._encoding = None
            self._newline = None
        else:
            self.hex = None
            self.editor = CodeEdit(
                font_size=settings.get("editor_font_size"),
                tab_width=settings.get("editor_tab_width"),
            )
            self.setCentralWidget(self.editor)
            if new_file or not self.local_path:
                text, self._encoding, self._newline = "", "utf-8", "\n"
            else:
                text, self._encoding, self._newline = _decode_text(raw)
            self.editor.setPlainText(text)
            self.editor.document().setModified(False)
            self._lang = language_for(lang_path)
            self._hl = Highlighter(self.editor.document(), self._lang)
        fit_to_screen(self, 900, 640)

        self._build_toolbar()
        self.setStatusBar(QStatusBar())
        if self.is_hex:
            self.hex.modified_changed.connect(lambda _m: self._update_title())
            self.hex.cursor_moved.connect(self._update_hex_status)
        else:
            self.editor.document().modificationChanged.connect(self._update_title)
            self.editor.cursorPositionChanged.connect(self._update_cursor_status)
        self._update_title()

        QShortcut(QKeySequence.Save, self, self.save)
        if not self.is_hex:
            QShortcut(QKeySequence.SaveAs, self, self.save_as)
            QShortcut(QKeySequence.Find, self, self._focus_find)
            QShortcut(QKeySequence.FindNext, self, lambda: self._find(True))
            QShortcut(QKeySequence.Replace, self, self._focus_replace)
            QShortcut(QKeySequence("Ctrl+G"), self, self._goto_line)
            QShortcut(QKeySequence(Qt.Key_Escape), self, self._hide_find)

    @property
    def display_path(self) -> str:
        if self.remote_path:
            return self.remote_path
        if self.local_path:
            return self.local_path
        return "無題"

    def _set_language_from_path(self, path: str) -> None:
        self._lang = language_for(path)
        if self.editor is not None and not self.is_hex:
            self._hl = Highlighter(self.editor.document(), self._lang)

    # ---- ツールバー / 検索 --------------------------------------------------
    def _build_toolbar(self):
        tb = QToolBar()
        tb.setMovable(False)
        self.addToolBar(tb)
        tb.addAction("保存 (Ctrl+S)", self.save)
        if not self.remote_path or self._untitled:
            tb.addAction("名前を付けて保存", self.save_as)
        tb.addSeparator()
        if self.is_hex:
            # 16 進モードは検索を持たない(第 1 段)。誤解を招かないよう、
            # 上書き専用であることを明示する。
            self.find_edit = None
            lbl = style.muted_label(
                "16 進モード: バイナリのため上書き編集のみ"
                "(挿入・削除はできません。Tab で 16 進 / ASCII 欄を切替)")
            lbl.setWordWrap(False)
            tb.addWidget(lbl)
            return
        self.find_edit = QLineEdit()
        self.find_edit.setPlaceholderText("検索 (Ctrl+F)")
        self.find_edit.setMaximumWidth(200)
        self.find_edit.returnPressed.connect(lambda: self._find(True))
        tb.addWidget(self.find_edit)
        self.replace_edit = QLineEdit()
        self.replace_edit.setPlaceholderText("置換 (Ctrl+H)")
        self.replace_edit.setMaximumWidth(200)
        tb.addWidget(self.replace_edit)
        tb.addAction("次へ", lambda: self._find(True))
        tb.addAction("前へ", lambda: self._find(False))
        tb.addAction("置換", self._replace_one)
        tb.addAction("すべて置換", self._replace_all)
        tb.addSeparator()
        self._act_wrap = tb.addAction("折返し", self._toggle_wrap)
        self._act_wrap.setCheckable(True)
        tb.addAction("行へ移動", self._goto_line)

    def _focus_find(self):
        self.find_edit.setFocus()
        self.find_edit.selectAll()

    def _focus_replace(self):
        self.replace_edit.setFocus()
        self.replace_edit.selectAll()

    def _hide_find(self):
        self.editor.setFocus()

    def _toggle_wrap(self):
        if self.editor.lineWrapMode() == QPlainTextEdit.NoWrap:
            self.editor.setLineWrapMode(QPlainTextEdit.WidgetWidth)
        else:
            self.editor.setLineWrapMode(QPlainTextEdit.NoWrap)

    def _goto_line(self):
        cur = self.editor.textCursor().blockNumber() + 1
        total = self.editor.blockCount()
        line, ok = QInputDialog.getInt(
            self, "行へ移動", f"行番号 (1-{total}):", cur, 1, total)
        if not ok:
            return
        block = self.editor.document().findBlockByNumber(line - 1)
        cur = self.editor.textCursor()
        cur.setPosition(block.position())
        self.editor.setTextCursor(cur)
        self.editor.setFocus()

    def _replace_one(self):
        needle = self.find_edit.text()
        if not needle:
            return
        repl = self.replace_edit.text()
        cur = self.editor.textCursor()
        if cur.hasSelection() and cur.selectedText().replace("\u2029", "\n") == needle:
            cur.insertText(repl)
        self._find(True)

    def _replace_all(self):
        needle = self.find_edit.text()
        if not needle:
            return
        repl = self.replace_edit.text()
        text = self.editor.toPlainText()
        count = text.count(needle)
        if not count:
            self.statusBar().showMessage("置換対象が見つかりません", 2000)
            return
        self.editor.selectAll()
        self.editor.insertPlainText(text.replace(needle, repl))
        self.statusBar().showMessage(f"{count} 件を置換しました", 3000)

    def _find(self, forward: bool):
        text = self.find_edit.text()
        if not text:
            return
        # FindFlag は QPlainTextEdit ではなく QTextDocument 側にある
        flags = (QTextDocument.FindFlags() if forward
                 else QTextDocument.FindFlag.FindBackward)
        if not self.editor.find(text, flags):
            # 端まで来たら先頭/末尾へ回り込み
            cur = self.editor.textCursor()
            cur.movePosition(QTextCursor.Start if forward else QTextCursor.End)
            self.editor.setTextCursor(cur)
            self.editor.find(text, flags)

    # ---- 保存 ---------------------------------------------------------------
    def _write_local_payload(self) -> bool:
        """ローカルパスへバイナリ書き込み。成功なら True。"""
        if not self.local_path:
            return False
        try:
            if self.is_hex:
                payload = self.hex.data()
            else:
                text = self.editor.toPlainText()
                if self._newline != "\n":
                    text = text.replace("\n", self._newline)
                payload = text.encode(self._encoding)
            with open(self.local_path, "wb") as f:
                f.write(payload)
            return True
        except Exception as e:  # noqa: BLE001
            QMessageBox.warning(self, "保存", f"ファイルの書き込みに失敗:\n{e}")
            return False

    def _mark_saved(self) -> None:
        if self.is_hex:
            self.hex.mark_saved()
        else:
            self.editor.document().setModified(False)
        self._untitled = False
        self._update_title()

    def save_as(self) -> None:
        suggested = getattr(self, "_suggested_dir", None)
        start = self.local_path or suggested or os.path.expanduser("~")
        if os.path.isdir(start):
            start = os.path.join(start, "無題.txt")
        path, _ = QFileDialog.getSaveFileName(
            self, "名前を付けて保存", start, "すべてのファイル (*.*)")
        if not path:
            return
        old_key = self.local_path
        self.local_path = path
        if not self._write_local_payload():
            self.local_path = old_key
            return
        if not self.is_hex:
            self._set_language_from_path(path)
        self.statusBar().showMessage(f"保存しました: {path}", 4000)
        self._mark_saved()

    def save(self):
        if self._saving:
            return
        if self._untitled or not self.local_path:
            self.save_as()
            return
        if not self._write_local_payload():
            return
        if self.remote_path and self._save_cb:
            self._saving = True
            self.statusBar().showMessage("サーバーへ保存中…")
            self._save_cb(self.remote_path, self.local_path, self._on_saved)
            return
        self.statusBar().showMessage(f"保存しました: {self.local_path}", 4000)
        self._mark_saved()

    def _on_saved(self, ok: bool, message: str):
        self._saving = False
        if ok:
            self._mark_saved()
            self.statusBar().showMessage(f"保存しました: {self.remote_path}", 4000)
        else:
            self.statusBar().showMessage("保存に失敗しました", 4000)
            QMessageBox.warning(self, "保存エラー", message)
        self._update_title()

    # ---- タイトル / ステータス ------------------------------------------------
    def _is_dirty(self) -> bool:
        if self.is_hex:
            return self.hex.is_modified()
        return self.editor.document().isModified()

    def _update_title(self):
        dirty = "*" if self._is_dirty() else ""
        mode = " [HEX]" if self.is_hex else ""
        label = (os.path.basename(self.display_path)
                 if self.display_path != "無題" else "無題")
        where = self.remote_path or self.local_path or "ローカル"
        self.setWindowTitle(
            f"{dirty}{label}{mode} — {where} [Hashi エディタ]")

    def _update_cursor_status(self):
        c = self.editor.textCursor()
        eol = {"\r\n": "CRLF", "\r": "CR", "\n": "LF"}.get(
            self._newline or "\n", "LF")
        enc = self._encoding or "?"
        self.statusBar().showMessage(
            f"行 {c.blockNumber() + 1}, 列 {c.columnNumber() + 1}  |  "
            f"{enc}  |  {eol}  |  {self._lang}", 0)

    def _update_hex_status(self, offset: int):
        data = self.hex.data()
        byte = data[offset] if offset < len(data) else 0
        self.statusBar().showMessage(
            f"オフセット 0x{offset:08X} ({offset})   "
            f"値 0x{byte:02X} ({byte})   全 {len(data)} バイト", 0)

    def closeEvent(self, ev):
        if self._is_dirty():
            name = os.path.basename(self.display_path)
            if self.display_path == "無題":
                name = "無題"
            r = QMessageBox.question(
                self, "未保存の変更",
                f"{name} は未保存です。保存しますか?",
                QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel,
            )
            if r == QMessageBox.Save:
                self.save()
                ev.ignore()
                return
            if r == QMessageBox.Cancel:
                ev.ignore()
                return
        self.closed.emit(self)
        ev.accept()


class LocalEditorHub:
    """ローカルファイル用の内蔵エディタ窓をまとめて開く(メモ帳のように使う)。"""

    def __init__(self, settings):
        self.settings = settings
        self._open: dict[str, EditorWindow] = {}   # local_path -> window
        self._untitled: set[int] = set()          # id(win) for 無題

    def _track(self, win: EditorWindow) -> None:
        key = win.local_path or f"__untitled_{id(win)}"
        self._open[key] = win
        if win._untitled:
            self._untitled.add(id(win))
        win.closed.connect(self._on_closed)

    def _on_closed(self, win: EditorWindow) -> None:
        key = win.local_path or f"__untitled_{id(win)}"
        self._open.pop(key, None)
        self._untitled.discard(id(win))

    def open_path(self, path: str, parent=None) -> EditorWindow | None:
        """ローカルファイルを内蔵エディタで開く。既に開いていれば前面へ。"""
        path = os.path.abspath(path)
        if path in self._open:
            w = self._open[path]
            w.raise_()
            w.activateWindow()
            return w
        if not os.path.isfile(path):
            return None
        if not self.settings.get("open_text_in_editor", True):
            return None
        if not should_edit_internally(os.path.basename(path)):
            return None
        try:
            size = os.path.getsize(path)
        except OSError:
            return None
        if size > EDITOR_MAX_BYTES:
            QMessageBox.warning(
                parent, "エディタ",
                f"ファイルが大きすぎます ({size // (1024 * 1024)} MB)。\n"
                f"上限は {EDITOR_MAX_BYTES // (1024 * 1024)} MB です。")
            return None
        try:
            win = EditorWindow(path, self.settings, parent=parent)
        except Exception as e:  # noqa: BLE001
            QMessageBox.warning(parent, "エディタ", f"開けませんでした:\n{e}")
            return None
        self._track(win)
        win.show()
        return win

    def new_file(self, parent=None, start_dir: str = "") -> EditorWindow:
        """新規の無題テキスト(メモ帳の「新規作成」相当)。"""
        win = EditorWindow(
            "", self.settings, parent=parent, new_file=True)
        if start_dir and os.path.isdir(start_dir):
            win._suggested_dir = start_dir  # save_as の初期フォルダ用
        self._track(win)
        win.show()
        return win

    def open_file_dialog(self, parent=None) -> EditorWindow | None:
        path, _ = QFileDialog.getOpenFileName(
            parent, "テキストを開く", os.path.expanduser("~"),
            "すべてのファイル (*.*)")
        if not path:
            return None
        return self.open_path(path, parent=parent)
