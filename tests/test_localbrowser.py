"""ローカル側ファイルブラウザ / デュアルペイン (Issue #82) のテスト。

ネットワーク不要。転送そのものはリモート側 (SftpBrowser) が持つので、ここでは
「どのパスを送る / どこへ受け取る」の受け渡しと、ローカル操作の安全性を固める。
"""
import os

import pytest
from PySide6.QtCore import QMimeData, QObject, QUrl, Signal
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

    class _SyncBrowse:
        """SyncBrowse の代役。set_enabled の呼ばれ方だけ見る。"""

        def __init__(self):
            self.enabled = None

        def set_enabled(self, on):
            self.enabled = on

    class T:
        def __init__(self):
            self.settings = _Settings({"dual_pane": False, "sync_browse": False})
            self.terminal = object()
            self.browser = object()
            self._host = host                       # GC 防止
            self._use_browser = True
            self._term_pane = QWidget(host)
            self._local_pane = QWidget(host)
            self._browser_pane = QWidget(host)
            self._files_splitter = QWidget(host)
            self._local_pane.setParent(self._files_splitter)
            self._browser_pane.setParent(self._files_splitter)
            self.bt_term = QToolButton(host)
            self.bt_files = QToolButton(host)
            self.bt_local = QToolButton(host)
            self.bt_sync = QToolButton(host)
            for b in (self.bt_term, self.bt_files, self.bt_local, self.bt_sync):
                b.setCheckable(True)
            self.bt_term.setChecked(True)
            self.bt_files.setChecked(True)
            self.sync_browse = _SyncBrowse()
            self.flashes = []

        def sender(self):
            return None

        def _refresh_button_icon(self, _b):
            pass

        def _flash(self, text, warn=False):
            self.flashes.append((text, warn))

        _apply_visibility = SessionTab._apply_visibility
        _on_dual_pane_toggled = SessionTab._on_dual_pane_toggled
        _on_sync_browse_toggled = SessionTab._on_sync_browse_toggled

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


def test_sync_toggle_follows_dual_pane(tab):
    """2 ペインを畳んだら同期も畳む(ローカルが無い間の同期は無意味)。"""
    tab.bt_local.setChecked(True)
    tab._on_dual_pane_toggled(True)
    tab.bt_sync.setChecked(True)
    tab._on_sync_browse_toggled(True)
    assert tab.settings.get("sync_browse") is True
    assert tab.sync_browse.enabled is True

    tab.bt_local.setChecked(False)
    tab._on_dual_pane_toggled(False)
    assert not tab.bt_sync.isChecked()
    assert not tab.bt_sync.isEnabled()
    assert tab.sync_browse.enabled is False
    # 設定は消さないので 2 ペインへ戻せば復帰する
    assert tab.settings.get("sync_browse") is True

    tab.bt_local.setChecked(True)
    tab._on_dual_pane_toggled(True)
    assert tab.bt_sync.isChecked()
    assert tab.bt_sync.isEnabled()


# ---- 同期ブラウズの相対移動 (Issue #82 第 2 段) --------------------------------
def test_path_components_splits_root_and_parts():
    import posixpath

    assert localbrowser.path_components("/srv/app/logs", posixpath) == (
        "/", ["srv", "app", "logs"])
    assert localbrowser.path_components("/", posixpath) == ("/", [])


def test_mirror_move_down_and_up():
    """1 段下る / 上がる移動がそのまま相手側へ写る。"""
    import posixpath

    # ローカルが logs へ入る → リモートも logs へ
    assert localbrowser.mirror_move(
        "/home/me/proj", "/home/me/proj/logs", "/srv/app",
        posixpath, posixpath) == "/srv/app/logs"
    # ローカルが 1 つ上がる → リモートも 1 つ上がる
    assert localbrowser.mirror_move(
        "/home/me/proj", "/home/me", "/srv/app/logs",
        posixpath, posixpath) == "/srv/app"


def test_mirror_move_handles_sideways_jump():
    """兄弟フォルダへの移動は「1 つ上がって別名へ降りる」として写る。"""
    import posixpath

    assert localbrowser.mirror_move(
        "/home/me/a", "/home/me/b", "/srv/app/a",
        posixpath, posixpath) == "/srv/app/b"


def test_mirror_move_multi_level():
    """複数段の移動もまとめて写る。"""
    import posixpath

    assert localbrowser.mirror_move(
        "/home/me/proj", "/home/me/proj/x/y", "/srv/app",
        posixpath, posixpath) == "/srv/app/x/y"


def test_mirror_move_gives_up_past_root():
    """相手がルートを突き抜ける移動は追随しない(None を返す)。"""
    import posixpath

    assert localbrowser.mirror_move(
        "/a/b/c", "/", "/srv", posixpath, posixpath) is None


def test_mirror_move_gives_up_across_drives():
    """ドライブ(ルート)が変わる移動は相対移動として解釈しない。"""
    import ntpath

    assert localbrowser.mirror_move(
        r"C:\Users\me", r"D:\data", r"C:\other", ntpath, ntpath) is None


# ---- SyncBrowse の調停 ------------------------------------------------------
class _FakeRemote(QObject):
    """SftpBrowser のうち SyncBrowse が触る部分だけのフェイク。"""

    path_changed = Signal(str)
    sync_failed = Signal(str)

    def __init__(self, cwd="/srv/app"):
        super().__init__()
        self.cwd = cwd
        self.sync_calls = []

    def cd_sync(self, path):
        self.sync_calls.append(path)


@pytest.fixture()
def synced(qapp, tmp_path):
    """ローカル実体つきの SyncBrowse。tmp/base と tmp/base/logs を用意する。"""
    from hashi.localbrowser import SyncBrowse

    base = tmp_path / "base"
    (base / "logs").mkdir(parents=True)
    local = LocalBrowser(start_dir=str(base))
    remote = _FakeRemote()
    sync = SyncBrowse(local, remote)
    sync.set_enabled(True)
    yield sync, local, remote, base
    local.deleteLater()


def test_local_move_drives_remote(synced):
    sync, local, remote, base = synced
    local.cd(str(base / "logs"))
    assert remote.sync_calls == ["/srv/app/logs"]


def test_disabled_sync_does_not_move_remote(synced):
    sync, local, remote, base = synced
    sync.set_enabled(False)
    local.cd(str(base / "logs"))
    assert remote.sync_calls == []


def test_remote_move_drives_local(synced):
    sync, local, remote, base = synced
    remote.cwd = "/srv/app/logs"
    remote.path_changed.emit("/srv/app/logs")
    assert local.cwd == os.path.abspath(str(base / "logs"))


def test_no_feedback_loop(synced):
    """写した移動が跳ね返って無限に往復しないこと。"""
    sync, local, remote, base = synced
    local.cd(str(base / "logs"))
    assert remote.sync_calls == ["/srv/app/logs"]
    # リモートが実際に移動を完了した通知が返ってくる
    remote.cwd = "/srv/app/logs"
    remote.path_changed.emit("/srv/app/logs")
    # ここで再びローカルを動かそうとしない(ローカルは既にそこに居る)
    assert remote.sync_calls == ["/srv/app/logs"]
    assert local.cwd == os.path.abspath(str(base / "logs"))


def test_remote_failure_clears_the_skip_flag(synced):
    """追随に失敗したら待ち状態を残さない(次の移動を取りこぼさない)。"""
    sync, local, remote, base = synced
    problems = []
    sync.failed.connect(problems.append)

    local.cd(str(base / "logs"))            # リモートへ追随を依頼
    remote.sync_failed.emit("No such file")  # が、行き先が無かった
    assert problems and "移動できません" in problems[0]

    # 次にリモートが自力で動いたとき、ちゃんとローカルが追随する
    remote.cwd = "/srv/app/logs"
    remote.path_changed.emit("/srv/app/logs/deeper")
    # base/logs から見て deeper は無いので追随できない旨が出る(黙って無視しない)
    assert len(problems) == 2


def test_local_target_missing_reports_instead_of_moving(synced):
    """ローカルに対応フォルダが無ければ、黙らずに知らせて動かない。"""
    sync, local, remote, base = synced
    problems = []
    sync.failed.connect(problems.append)
    before = local.cwd

    remote.cwd = "/srv/app/nope"
    remote.path_changed.emit("/srv/app/nope")

    assert local.cwd == before
    assert problems and "ローカル側" in problems[0]
