"""editor.py のテスト(Issue #7)。言語判定・ハイライト・検索・保存フロー。"""
import pytest
from PySide6.QtGui import QTextDocument


@pytest.mark.parametrize("path,expected", [
    ("/srv/app/main.py", "python"),
    ("/x/win.PYW", "python"),
    ("/x/kernel.c", "c"),
    ("/x/lib.rs", "c"),
    ("/x/Main.java", "c"),
    ("/x/app.tsx", "js"),
    ("/x/package.json", "json"),
    ("/home/tester/.bashrc", "shell"),
    ("/x/deploy.sh", "shell"),
    ("/etc/nginx/nginx.conf", "conf"),
    ("/x/pyproject.toml", "conf"),
    ("/x/README.md", "markup"),
    ("/x/style.css", "css"),
    ("/x/query.sql", "sql"),
    ("/x/noext", "plain"),
])
def test_lang_for(path, expected):
    from hashi.editor import _lang_for
    assert _lang_for(path) == expected


_SAMPLES = {
    "python": 'def f(x):\n    # comment\n    return "text"\n',
    "c": '/* block\n   comment */\nint main() { return 0; } // eol\n',
    "js": 'const f = (x) => { return `t`; } // c\n/* b */\n',
    "shell": 'if [ -f x ]; then\n  echo "hi" # c\nfi\n',
    "conf": '[section]\nkey = value  # c\n',
    "ruby": 'def f\n  # c\nend\n',
    "php": '<?php\n// c\necho "hi";\n',
    "lua": 'function f()\n  -- not hash\nend\n',
    "sql": 'SELECT * FROM t; -- c\n',
    "css": '.foo { color: red; /* c */ }\n',
    "markup": '<div class="x">text</div>\n',
    "json": '{"a": 1, "b": "c"}\n',
    "plain": 'ただのテキスト\n',
}


@pytest.mark.parametrize("lang", sorted(_SAMPLES))
def test_highlighter_smoke(qapp, lang):
    """各言語のハイライトが例外なく走る(複数行ブロックコメント含む)。"""
    from hashi.editor import Highlighter
    doc = QTextDocument()
    hl = Highlighter(doc, lang)
    doc.setPlainText(_SAMPLES[lang])
    hl.rehighlight()  # 明示的に全行を通す


def test_code_edit_line_number_width_grows(qapp):
    from hashi.editor import CodeEdit
    e = CodeEdit()
    e.setPlainText("x")
    w_small = e.line_number_width()
    e.setPlainText("\n" * 9999)
    assert e.line_number_width() > w_small


class _FakeSettings:
    _d = {"editor_font_size": 12, "editor_tab_width": 4}

    def get(self, key):
        return self._d[key]


@pytest.fixture()
def editor_window(qapp, tmp_path):
    from hashi.editor import EditorWindow
    p = tmp_path / "sample.py"
    p.write_text("alpha\nbeta\nalpha tail\n", encoding="utf-8")
    calls = []

    def save_cb(remote, local, done):
        calls.append((remote, local, done))

    w = EditorWindow("/srv/sample.py", str(p), save_cb, _FakeSettings())
    w._calls = calls
    yield w
    w.editor.document().setModified(False)  # closeEvent の確認ダイアログ回避
    w.close()


def test_find_forward_and_wrap(editor_window):
    """前方検索でヒットを辿り、末尾まで来たら先頭へ回り込む。"""
    w = editor_window
    w.find_edit.setText("alpha")
    w._find(True)
    first = w.editor.textCursor().selectionStart()
    w._find(True)
    second = w.editor.textCursor().selectionStart()
    assert second > first
    w._find(True)  # もうヒットが無い → 先頭へ回り込み
    assert w.editor.textCursor().selectionStart() == first


def test_find_backward(editor_window):
    w = editor_window
    w.find_edit.setText("alpha")
    w._find(False)  # 末尾へ回り込んで最後のヒット
    assert w.editor.textCursor().selectionStart() > 0


def test_save_writes_local_and_calls_callback(editor_window, tmp_path):
    """save() はローカル一時ファイルへ書いてからアップロード用コールバックを呼ぶ。"""
    w = editor_window
    w.editor.selectAll()
    w.editor.insertPlainText("changed body\n")  # setPlainText は modified を立てない
    assert w.editor.document().isModified()
    w.save()
    assert (tmp_path / "sample.py").read_text(encoding="utf-8") == "changed body\n"
    assert len(w._calls) == 1
    remote, local, done = w._calls[0]
    assert remote == "/srv/sample.py"
    # 保存中の再入は無視される
    w.save()
    assert len(w._calls) == 1
    # アップロード完了 → modified フラグが下りる
    done(True, "")
    assert not w.editor.document().isModified()


def test_save_failure_keeps_modified(editor_window):
    w = editor_window
    w.editor.selectAll()
    w.editor.insertPlainText("v2\n")
    w.save()
    _, _, done = w._calls[0]
    # QMessageBox を出さないよう差し替え
    from unittest.mock import patch
    with patch("hashi.editor.QMessageBox.warning"):
        done(False, "permission denied")
    assert w.editor.document().isModified()


def test_find_with_empty_query_is_noop(editor_window):
    """検索クエリが空でもクラッシュせず何もしない。"""
    w = editor_window
    w.find_edit.setText("")
    before = w.editor.textCursor().position()
    w._find(True)
    assert w.editor.textCursor().position() == before


def test_find_miss_leaves_no_selection(editor_window):
    w = editor_window
    w.find_edit.setText("does-not-exist-xyz")
    w._find(True)
    assert not w.editor.textCursor().hasSelection()


def test_update_title_reflects_modified(editor_window):
    w = editor_window
    w.editor.document().setModified(False)
    w._update_title()
    assert not w.windowTitle().startswith("*")
    w.editor.document().setModified(True)
    w._update_title()
    assert w.windowTitle().startswith("*")
    assert "sample.py" in w.windowTitle()


def test_cursor_status_is_one_based(editor_window):
    from PySide6.QtGui import QTextCursor
    w = editor_window
    cur = w.editor.textCursor()
    cur.movePosition(QTextCursor.Start)
    w.editor.setTextCursor(cur)
    w._update_cursor_status()
    msg = w.statusBar().currentMessage()
    assert "行 1" in msg and "列 1" in msg


def test_lang_for_expanded_extensions():
    """追加拡張子の言語判定。"""
    from hashi.editor import _lang_for

    assert _lang_for("Main.kt") == "c"
    assert _lang_for("app.swift") == "c"
    assert _lang_for("App.vue") == "js"
    assert _lang_for("deploy.ps1") == "shell"
    assert _lang_for("analysis.R") == "shell"
    assert _lang_for(".editorconfig") == "conf"
    assert _lang_for("app.properties") == "conf"
    assert _lang_for("Gemfile") == "shell"
    assert _lang_for("schema.graphql") == "js"
    assert _lang_for("readme.unknownext") == "plain"


def test_replace_all(editor_window):
    w = editor_window
    w.find_edit.setText("alpha")
    w.replace_edit.setText("omega")
    w._replace_all()
    assert "omega" in w.editor.toPlainText()
    assert "alpha" not in w.editor.toPlainText()


def test_goto_line(editor_window):
    w = editor_window
    from PySide6.QtGui import QTextCursor
    cur = w.editor.textCursor()
    cur.movePosition(QTextCursor.End)
    w.editor.setTextCursor(cur)
    block = w.editor.document().findBlockByNumber(1)
    cur.setPosition(block.position())
    w.editor.setTextCursor(cur)
    w._update_cursor_status()
    assert "行 2" in w.statusBar().currentMessage()


def test_status_shows_encoding_and_language(editor_window):
    w = editor_window
    w._update_cursor_status()
    msg = w.statusBar().currentMessage()
    assert "utf-8" in msg
    assert "python" in msg


# ---- バイナリ / 改行 / BOM を壊さない(Issue #122) --------------------------
def _open_editor(tmp_path, name: str, raw: bytes):
    """生バイトのファイルを内蔵エディタで開いてウィンドウと保存記録を返す。"""
    from hashi.editor import EditorWindow
    p = tmp_path / name
    p.write_bytes(raw)
    calls = []
    w = EditorWindow("/srv/" + name, str(p),
                     lambda remote, local, done: calls.append(
                         (remote, local, done)), _FakeSettings())
    w._calls = calls
    return w, p


def _save_and_finish(w):
    """save() 後にアップロード完了を通知する(_saving の再入ガードを解除)。"""
    w.save()
    assert w._calls, "保存コールバックが呼ばれていない"
    w._calls[-1][2](True, "")


def test_binary_opens_in_hex_mode_and_roundtrips(qapp, tmp_path):
    """バイナリ(.dat)は 16 進モードで開き、無編集の保存で 1 バイトも変わらない。

    回帰: 以前は latin-1 + テキストモードで読み書きしていたため、
    0x0D 0x0A が 0x0A に潰れてファイルが壊れていた。
    """
    raw = bytes([0x1F, 0x8B, 0x08, 0x00, 0x0D, 0x0A, 0x0D, 0x00, 0xFF, 0xFE]) * 8
    w, p = _open_editor(tmp_path, "level.dat", raw)
    assert w.is_hex is True
    assert w.hex is not None and w.editor is None

    _save_and_finish(w)            # 何も編集せず保存
    assert p.read_bytes() == raw   # 完全一致(改行変換もバイト欠落もない)

    # 1 バイトだけ書き換えると、その 1 バイトだけが変わる
    w.hex._move_to(4)
    w.hex._handle_input("0")
    w.hex._handle_input("9")
    _save_and_finish(w)
    saved = p.read_bytes()
    assert saved[4] == 0x09
    assert len(saved) == len(raw)
    assert saved[:4] == raw[:4] and saved[5:] == raw[5:]

    w.hex.mark_saved()
    w.close()


def test_text_editor_preserves_crlf(qapp, tmp_path):
    """CRLF の設定ファイルを保存しても LF に書き換えない(回帰)。"""
    raw = b"server-port=25565\r\nmax-players=20\r\n"
    w, p = _open_editor(tmp_path, "server.properties", raw)
    assert w.is_hex is False
    assert w._newline == "\r\n"
    # 編集中は LF に正規化されている(Qt 側の扱いに合わせる)
    assert "\r" not in w.editor.toPlainText()

    _save_and_finish(w)
    assert p.read_bytes() == raw          # CRLF のまま

    w.editor.selectAll()
    w.editor.insertPlainText("a=1\nb=2\n")
    _save_and_finish(w)
    assert p.read_bytes() == b"a=1\r\nb=2\r\n"
    w.editor.document().setModified(False)
    w.close()


def test_text_editor_does_not_add_bom(qapp, tmp_path):
    """BOM の無い UTF-8 に BOM を足さない / BOM 付きは維持する(回帰)。"""
    raw = "モット=こんにちは\n".encode()
    w, p = _open_editor(tmp_path, "motd.txt", raw)
    assert w._encoding == "utf-8"
    _save_and_finish(w)
    assert p.read_bytes() == raw
    assert not p.read_bytes().startswith(b"\xef\xbb\xbf")
    w.editor.document().setModified(False)
    w.close()

    raw_bom = b"\xef\xbb\xbf" + "設定=1\n".encode()
    w2, p2 = _open_editor(tmp_path, "bom.cfg", raw_bom)
    assert w2._encoding == "utf-8-sig"
    assert not w2.editor.toPlainText().startswith("﻿")
    _save_and_finish(w2)
    assert p2.read_bytes() == raw_bom     # BOM を保持
    w2.editor.document().setModified(False)
    w2.close()


def test_hex_mode_has_no_find_widgets(qapp, tmp_path):
    """16 進モードでは検索 UI を作らない(第 1 段)。落ちないことを確認。"""
    w, _ = _open_editor(tmp_path, "data.bin", bytes(range(64)))
    assert w.is_hex and w.find_edit is None
    w._update_title()
    assert "[HEX]" in w.windowTitle()
    w.close()
