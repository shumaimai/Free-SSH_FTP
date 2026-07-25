"""ローカル側ファイルブラウザ / デュアルペイン (Issue #82) のテスト。

ネットワーク不要。転送そのものはリモート側 (SftpBrowser) が持つので、ここでは
「どのパスを送る / どこへ受け取る」の受け渡しと、ローカル操作の安全性を固める。
"""
import os

import pytest
from PySide6.QtCore import QMimeData, QUrl
from PySide6.QtWidgets import QToolButton, QWidget

from hashi import localbrowser
from hashi.filebrowser import REMOTE_DRAG_MIME
from hashi.localbrowser import LocalBrowser


@pytest.fixture()
def tree_dir(tmp_path):
    """ファイル / フォルダ / 隠しファイルが 1 つずつあるディレクトリ。"""
    (tmp_path / "notes.txt").write_text("hello", encoding="utf-8")
    (tmp_path / "sub").mkdir()
    (tmp_path / ".hidden").write_text("x", encoding="utf-8")
    return tmp_path


# ---- 純粋関数 -------------------------------------------------------------
def test_scan_dir_shapes_entries_like_remote(tree_dir):
    """リモート側 (_job_list) と同じキーで返す(描画コードを共有するため)。"""
    entries = {e["name"]: e for e in localbrowser.scan_dir(str(tree_dir))}
    assert set(entries) == {"notes.txt", "sub", ".hidden"}
    for e in entries.values():
        assert set(e) >= {"name", "path", "is_dir", "is_link", "size",
                          "mtime", "mode_str"}
    assert entries["sub"]["is_dir"] is True
    assert entries["notes.txt"]["is_dir"] is False
    assert entries["notes.txt"]["size"] == 5
    assert entries["notes.txt"]["mode_str"].startswith("-")


def test_scan_dir_survives_broken_symlink(tmp_path):
    """壊れたリンクでも例外にせず、行としては残す。"""
    os.symlink(str(tmp_path / "does-not-exist"), str(tmp_path / "broken"))
    entries = {e["name"]: e for e in localbrowser.scan_dir(str(tmp_path))}
    assert entries["broken"]["is_link"] is True
    assert entries["broken"]["is_dir"] is False


def test_is_hidden_dotfiles():
    assert localbrowser.is_hidden(".bashrc")
    assert not localbrowser.is_hidden("notes.txt")


def test_delete_local_path_does_not_follow_symlinked_dir(tmp_path):
    """🔴 リンクを辿ってリンク先の実体を消さないこと。

    os.path.isdir() はリンク先がディレクトリなら True になるため、islink を
    先に見ないと shutil.rmtree がリンク先を丸ごと消してしまう。
    """
    target = tmp_path / "real"
    target.mkdir()
    (target / "keep.txt").write_text("大事なファイル", encoding="utf-8")
    link = tmp_path / "link"
    os.symlink(str(target), str(link))

    localbrowser.delete_local_path(str(link))

    assert not os.path.lexists(link)          # リンクだけ消える
    assert (target / "keep.txt").exists()     # 実体は無傷


def test_delete_local_path_removes_file_and_tree(tmp_path):
    f = tmp_path / "a.txt"
    f.write_text("x", encoding="utf-8")
    d = tmp_path / "d"
    (d / "inner").mkdir(parents=True)
    (d / "inner" / "b.txt").write_text("y", encoding="utf-8")

    localbrowser.delete_local_path(str(f))
    localbrowser.delete_local_path(str(d))

    assert not f.exists()
    assert not d.exists()


# ---- ウィジェット ---------------------------------------------------------
def _names(browser) -> set[str]:
    tree = browser.tree
    return {tree.topLevelItem(i).text(0) for i in range(tree.topLevelItemCount())}


def _select(browser, name: str):
    tree = browser.tree
    for i in range(tree.topLevelItemCount()):
        item = tree.topLevelItem(i)
        if item.text(0) == name:
            item.setSelected(True)
            return item
    raise AssertionError(f"項目が見つかりません: {name}")


@pytest.fixture()
def browser(qapp, tree_dir):
    b = LocalBrowser(start_dir=str(tree_dir))
    yield b
    b.deleteLater()


def test_lists_start_dir_and_hides_dotfiles(browser, tree_dir):
    assert browser.cwd == os.path.abspath(str(tree_dir))
    assert _names(browser) == {"notes.txt", "sub"}   # 隠しは既定で非表示


def test_hidden_toggle_shows_dotfiles(browser):
    browser.btn_hidden.setChecked(True)
    assert ".hidden" in _names(browser)
    browser.btn_hidden.setChecked(False)
    assert ".hidden" not in _names(browser)


def test_navigation_down_and_up(browser, tree_dir):
    browser.cd(str(tree_dir / "sub"))
    assert browser.cwd == os.path.abspath(str(tree_dir / "sub"))
    browser.go_up()
    assert browser.cwd == os.path.abspath(str(tree_dir))


def test_cd_to_missing_dir_keeps_current(browser, tree_dir):
    before = browser.cwd
    browser.cd(str(tree_dir / "no-such-dir"))
    assert browser.cwd == before


def test_upload_request_carries_selected_paths(browser, tree_dir):
    got = []
    browser.upload_requested.connect(got.append)
    _select(browser, "notes.txt")
    browser._request_upload()
    assert got == [[os.path.join(str(tree_dir), "notes.txt")]]


def test_upload_request_without_selection_emits_nothing(browser):
    got = []
    browser.upload_requested.connect(got.append)
    browser._request_upload()
    assert got == []


def test_download_request_targets_current_dir(browser, tree_dir):
    got = []
    browser.download_requested.connect(got.append)
    browser._request_download()
    assert got == [os.path.abspath(str(tree_dir))]


def test_remote_drop_requests_download_into_current_dir(browser, tree_dir):
    """リモート側からのドロップ = このフォルダへのダウンロード要求。"""
    got = []
    browser.download_requested.connect(got.append)
    browser.tree.remote_dropped.emit()
    assert got == [os.path.abspath(str(tree_dir))]


def test_local_drag_carries_file_urls(browser, tree_dir):
    """ローカル → リモートのドラッグは URL で運ぶ(OS へのドラッグとも共通)。"""
    item = _select(browser, "notes.txt")
    md = browser.tree.mimeData([item])
    assert md.hasUrls()
    assert [u.toLocalFile() for u in md.urls()] == [
        os.path.join(str(tree_dir), "notes.txt")
    ]


# ---- リモート側のドラッグ / ドロップ受け口 ------------------------------------
def test_remote_tree_drag_carries_marker_not_data(qapp):
    """リモートのドラッグには印だけを載せる(実体はまだ手元に無い)。"""
    from hashi.filebrowser import _DropTree

    tree = _DropTree()
    md = tree.mimeData([])
    assert md.hasFormat(REMOTE_DRAG_MIME)
    assert not md.hasUrls()
    tree.deleteLater()


def test_local_tree_ignores_its_own_url_drag(qapp):
    """自分(ローカル)から始まった URL ドラッグはローカル側で受けない。"""
    from hashi.localbrowser import _LocalTree

    class _Ev:
        def __init__(self, md):
            self._md = md
            self.accepted = False
            self.ignored = False

        def mimeData(self):
            return self._md

        def acceptProposedAction(self):
            self.accepted = True

        def ignore(self):
            self.ignored = True

    tree = _LocalTree()
    urls = QMimeData()
    urls.setUrls([QUrl.fromLocalFile("/tmp/x")])
    ev = _Ev(urls)
    tree.dragEnterEvent(ev)
    assert ev.ignored and not ev.accepted

    marker = QMimeData()
    marker.setData(REMOTE_DRAG_MIME, b"1")
    ev2 = _Ev(marker)
    tree.dragEnterEvent(ev2)
    assert ev2.accepted
    tree.deleteLater()


# ---- SessionTab のペイン表示切替 ---------------------------------------------
class _Settings(dict):
    def get(self, key, default=None):
        return dict.get(self, key, default)

    def set(self, key, value):
        self[key] = value


@pytest.fixture()
def tab(qapp):
    """SessionTab の表示切替に必要な属性だけ持つフェイク。"""
    from hashi.mainwindow import SessionTab

    host = QWidget()

    class T:
        def __init__(self):
            self.settings = _Settings({"dual_pane": False})
            self.terminal = object()
            self.browser = object()
            self._host = host                       # GC 防止
            self._term_pane = QWidget(host)
            self._local_pane = QWidget(host)
            self._browser_pane = QWidget(host)
            self._files_splitter = QWidget(host)
            self._local_pane.setParent(self._files_splitter)
            self._browser_pane.setParent(self._files_splitter)
            self.bt_term = QToolButton(host)
            self.bt_files = QToolButton(host)
            self.bt_local = QToolButton(host)
            for b in (self.bt_term, self.bt_files, self.bt_local):
                b.setCheckable(True)
            self.bt_term.setChecked(True)
            self.bt_files.setChecked(True)

        def sender(self):
            return None

        _apply_visibility = SessionTab._apply_visibility
        _on_dual_pane_toggled = SessionTab._on_dual_pane_toggled

    return T()


def _shown(w: QWidget) -> bool:
    return w.isVisibleTo(w.parentWidget())


def test_local_pane_hidden_until_dual_pane_on(tab):
    tab._apply_visibility()
    assert not _shown(tab._local_pane)      # 既定はリモートのみ
    assert _shown(tab._browser_pane)

    tab.bt_local.setChecked(True)
    tab._apply_visibility()
    assert _shown(tab._local_pane)


def test_hiding_files_hides_local_pane_too(tab):
    tab.bt_local.setChecked(True)
    tab._apply_visibility()
    assert _shown(tab._local_pane)

    tab.bt_files.setChecked(False)
    tab._apply_visibility()
    assert not _shown(tab._files_splitter)
    assert not _shown(tab._local_pane)


def test_dual_pane_toggle_is_remembered(tab):
    tab._on_dual_pane_toggled(True)
    assert tab.settings.get("dual_pane") is True
    tab._on_dual_pane_toggled(False)
    assert tab.settings.get("dual_pane") is False
