import os

import pytest

from hashi.filebrowser import _safe_local_child


@pytest.mark.parametrize("name", ["../secret", "..\\secret", "/etc/passwd", "C:\\secret", ".."])
def test_safe_local_child_rejects_path_traversal(tmp_path, name):
    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(tmp_path), name)


def test_safe_local_child_rejects_symlink_escape(tmp_path):
    outside = tmp_path.parent / "outside"
    outside.mkdir()
    link = tmp_path / "escape"
    try:
        os.symlink(outside, link, target_is_directory=True)
    except (OSError, NotImplementedError):
        pytest.skip("シンボリックリンクを作成できない環境")

    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(tmp_path), "escape")
    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(link), "secret.txt")


def test_safe_local_child_keeps_regular_name_inside_root(tmp_path):
    path = _safe_local_child(str(tmp_path), str(tmp_path), "report.txt")
    assert path == str(tmp_path / "report.txt")


# ---- リッチテキスト解釈の防止(#113) ---------------------------------------
# QLabel の既定 textFormat は AutoText で、`<b>` や `<img src=…>` を含む文字列を
# HTML として解釈する。リモートが決めたファイル名やサーバーのメッセージを
# そのまま渡すと表示の偽装や意図しないリソース読み込みにつながるため、
# 信頼できない文字列を出すラベルは PlainText 固定であることを固定する。

EVIL_TEXT = '<img src="/etc/passwd">&<b>bold</b>'


def test_plain_label_forces_plaintext(qapp):
    from PySide6.QtCore import Qt

    from hashi import style

    lbl = style.plain_label(EVIL_TEXT)
    assert lbl.textFormat() == Qt.PlainText
    assert lbl.text() == EVIL_TEXT      # そのまま保持され、解釈されない
    lbl.deleteLater()


def test_untrusted_labels_use_plain_label_helper():
    """リモート由来の文字列を出す箇所が plain_label を通っていること。

    実ウィジェットを組むとワーカースレッドが要るので、ソース上で
    「生の QLabel に渡していない」ことを軽量に確認する(退行の早期検知)。
    """
    import pathlib
    fb = pathlib.Path("hashi/filebrowser.py").read_text(encoding="utf-8")
    assert "self.lb_progress = style.plain_label()" in fb
    assert "self.lb_status = style.plain_label(" in fb
    mw = pathlib.Path("hashi/mainwindow.py").read_text(encoding="utf-8")
    assert "self._xfer_label = style.plain_label()" in mw
    assert "self._toast = style.plain_label()" in mw


def test_hostkey_dialog_escapes_host_and_keeps_fingerprint_plain(qapp):
    """RichText の見出しはエスケープ、指紋は PlainText で生のまま(#113)。"""
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QLabel

    from hashi.dialogs import HostKeyDialog

    dlg = HostKeyDialog(None, {
        "status": "new", "host": "<b>evil</b>.example.com", "port": 22,
        "key_type": "ssh-ed25519", "fingerprint": f"SHA256:{EVIL_TEXT}",
    })
    labels = dlg.findChildren(QLabel)
    # 見出し(RichText)ではホスト名の < > がエスケープされている
    rich = [w.text() for w in labels if w.textFormat() == Qt.RichText]
    assert rich and any("&lt;b&gt;evil&lt;/b&gt;" in t for t in rich)
    assert not any("<b>evil</b>" in t for t in rich)
    # 指紋は PlainText なので加工されず保持される
    plain = [w.text() for w in labels if w.textFormat() == Qt.PlainText]
    assert any(EVIL_TEXT in t for t in plain)
    dlg.deleteLater()
