"""ブラウザ風タブ(Issue #115)のテスト。

AppWindow が「サーバー一覧」タブを持ち、接続すると新しいタブ(SessionPage)が
開くことを確認する。実接続はさせない(start_connect を差し替え)。
"""
import pytest

from hashi.config import Profile


@pytest.fixture()
def app_win(qapp, tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path))
    monkeypatch.setenv("APPDATA", str(tmp_path))
    from hashi.config import config_dir
    from hashi.mainwindow import AppWindow, SessionPage
    (config_dir() / "settings.json").write_text(
        '{"update_check": false}', encoding="utf-8"
    )
    # 接続処理は起こさない
    monkeypatch.setattr(SessionPage, "start_connect", lambda self: None)
    before = list(SessionPage._pages)
    w = AppWindow()
    yield w
    for page in list(SessionPage._pages):
        if page not in before:
            page.session_tab = None
            page._alive_timer.stop()
            if page in SessionPage._pages:
                SessionPage._pages.remove(page)
    w.close()


def test_open_session_adds_tab_sharing_services(app_win):
    from hashi.mainwindow import SessionPage

    n_before = app_win.tabs.count()
    profile = Profile(host="h", username="u")
    page = app_win.open_session(profile)

    assert isinstance(page, SessionPage)
    assert app_win.tabs.count() == n_before + 1
    assert app_win.tabs.currentWidget() is page
    assert page.profile is profile
    # ストア類は同一実体を共有
    assert page.store is app_win.store
    assert page.known_hosts is app_win.known_hosts
    assert page.credentials is app_win.credentials
    assert page.settings is app_win.settings
    # 接続完了まではセッションメニュー無効
    assert not app_win.m_sess.isEnabled()


def test_launcher_tab_is_first_and_not_closable(app_win):
    from PySide6.QtWidgets import QTabBar

    from hashi.mainwindow import LauncherPage
    assert isinstance(app_win.tabs.widget(0), LauncherPage)
    # ランチャータブには閉じるボタンが無い
    assert app_win.tabs.tabBar().tabButton(0, QTabBar.ButtonPosition.RightSide) is None


def test_doubleclick_opens_session(app_win, monkeypatch):
    app_win.launcher.store.profiles.append(Profile(host="h", username="u"))
    app_win.launcher._reload_list()
    opened = []
    monkeypatch.setattr(app_win, "open_session",
                        lambda p, mode="both": opened.append((p, mode)))
    app_win.launcher._connect_item(app_win.launcher.list.item(0))
    assert len(opened) == 1


def test_close_tab_removes_page(app_win):
    from hashi.mainwindow import SessionPage
    page = app_win.open_session(Profile(host="h", username="u"))
    idx = app_win.tabs.indexOf(page)
    assert page in SessionPage._pages
    app_win._on_tab_close(idx)
    assert page not in SessionPage._pages


def test_import_refreshes_launcher_list(app_win):
    """読み込み等が反映される refresh_launcher。"""
    app_win.store.profiles.append(Profile(host="new", username="x"))
    app_win.refresh_launcher()
    lst = app_win.launcher.list
    labels = [lst.item(i).text() for i in range(lst.count())]
    assert any("new" in name or "x@new" in name for name in labels)


def test_launcher_detail_pane_follows_selection(app_win):
    """2 カラムカードの右側が選択に追従し、未選択ならボタンを無効化(#113)。"""
    lp = app_win.launcher
    lp.store.profiles.clear()
    lp.store.profiles.append(
        Profile(name="本番 web", host="203.0.113.9", port=2222,
                username="deploy", tags=["prod"]))
    lp._reload_list()

    # 未選択では接続系ボタンが無効
    lp.list.setCurrentItem(None)
    lp._update_detail()
    assert not lp.bt_connect_both.isEnabled()
    assert not lp.bt_edit.isEnabled()

    lp.list.setCurrentRow(0)
    assert lp.bt_connect_both.isEnabled()
    assert lp._detail_name.text() == "本番 web"
    assert "deploy@203.0.113.9:2222" in lp._detail_addr.text()
    assert "prod" in lp._detail_meta.text()


def test_launcher_connect_buttons_pass_mode(app_win, monkeypatch):
    """SSH のみ / ファイルのみ ボタンが正しい mode で接続する(#112/#113)。"""
    lp = app_win.launcher
    lp.store.profiles.clear()
    lp.store.profiles.append(Profile(host="h", username="u"))
    lp._reload_list()
    lp.list.setCurrentRow(0)

    opened = []
    monkeypatch.setattr(app_win, "open_session",
                        lambda p, mode="both": opened.append(mode))
    lp.bt_connect_both.click()
    lp.bt_connect_ssh.click()
    lp.bt_connect_sftp.click()
    assert opened == ["both", "ssh", "sftp"]


def test_launcher_empty_state(app_win):
    """接続先ゼロ / 検索一致なしで空状態プレースホルダを出す(#113)。"""
    lp = app_win.launcher
    lp.store.profiles.clear()
    lp._reload_list()
    assert lp._empty.isVisibleTo(lp) and not lp.list.isVisibleTo(lp)
    assert "まだ接続先がありません" in lp._empty.text()

    lp.store.profiles.append(Profile(name="srv", host="h", username="u"))
    lp._reload_list()
    assert lp.list.isVisibleTo(lp) and not lp._empty.isVisibleTo(lp)

    lp.ed_search.setText("該当しないはず")
    assert lp._empty.isVisibleTo(lp)
    assert "一致する接続先がありません" in lp._empty.text()


class _FakeSFTP:
    def listdir_attr(self, path="."): return []
    def normalize(self, path): return path or "/home/u"
    def stat(self, path): raise IOError("no such file")
    def close(self): pass


class _ModeSession:
    """SessionTab のモード別構築テスト用の最小フェイク session。"""

    class _Prof:
        username = "u"
        host = "h"
        port = 22
        initial_path = ""
        id = "u@h:22"

        def label(self):
            return "u@h"

        def id_str(self):
            return "u@h:22"

    def __init__(self):
        self.profile = self._Prof()
        self.transport = None
        self.shell_opened = 0

    def open_shell(self, cols=80, rows=24):
        self.shell_opened += 1
        class _Ch:
            def get_transport(self): return None
            def settimeout(self, *a): pass
            def recv(self, n): return b""
            def recv_ready(self): return False
            def send(self, d): pass
            def resize_pty(self, **k): pass
            def close(self): pass
            active = True
        return _Ch()

    def open_sftp(self):
        return _FakeSFTP()

    def run_sudo(self, cmd, pw): return (1, "", "")
    def is_alive(self): return True
    def close(self): pass


def _make_tab(qapp, mode):
    import pathlib
    import tempfile
    from types import SimpleNamespace

    from hashi.config import Settings
    from hashi.mainwindow import SessionTab
    st = Settings(pathlib.Path(tempfile.mkdtemp()) / "s.json")
    ctx = SimpleNamespace(
        get_sudo_password=lambda allow_prompt=True: None,
        get_login_password=lambda: None)
    return SessionTab(_ModeSession(), st, ctx, mode=mode)


def _cleanup(qapp, tab):
    """ワーカースレッドを確実に止めてから破棄する(他テストへの漏れ防止)。"""
    tab.session_log = None
    tab.shutdown()
    for _ in range(20):
        qapp.processEvents()
    tab.deleteLater()
    qapp.processEvents()


def test_toolbar_buttons_emit_requests_not_logic(qapp):
    """ツールバーは依頼(シグナル)を投げるだけ。処理は Page 側が持つ(#113)。"""
    tab = _make_tab(qapp, "both")
    got = []
    tab.request_snippets.connect(lambda: got.append("snippets"))
    tab.request_tunnel.connect(lambda: got.append("tunnel"))
    tab.request_session_log.connect(lambda: got.append("log"))
    tab.bt_snippets.click()
    tab.bt_tunnel.click()
    tab.bt_log.click()
    assert got == ["snippets", "tunnel", "log"]
    _cleanup(qapp, tab)


def test_toolbar_icons_track_state(qapp):
    """トグルのアイコン色が状態に追従する(チェック時はアクセント色)(#113)。"""
    from hashi import style

    tab = _make_tab(qapp, "both")
    # チェック中はアクセント色のアイコン、外すと通常色
    tab.bt_term.setChecked(True)
    on_icon = style.icon("terminal", style.ACCENT).cacheKey()
    assert tab.bt_term.icon().cacheKey() == on_icon
    tab.bt_term.setChecked(False)
    off_icon = style.icon("terminal", style.FG).cacheKey()
    assert tab.bt_term.icon().cacheKey() == off_icon
    _cleanup(qapp, tab)


def test_sftp_only_disables_terminal_side_buttons(qapp):
    """ファイルのみモードでは端末側の操作ボタンを無効化(#112/#113)。"""
    tab = _make_tab(qapp, "sftp")
    assert not tab.bt_sendpw.isEnabled()
    assert not tab.bt_snippets.isEnabled()
    assert not tab.bt_log.isEnabled()
    _cleanup(qapp, tab)


def test_session_tab_ssh_only_has_no_browser(qapp):
    tab = _make_tab(qapp, "ssh")
    assert tab.terminal is not None
    assert tab.browser is None
    assert tab.session.shell_opened == 1
    assert not tab.bt_files.isEnabled()
    assert tab.bt_term.isEnabled()
    _cleanup(qapp, tab)


def test_session_tab_sftp_only_has_no_terminal(qapp):
    tab = _make_tab(qapp, "sftp")
    assert tab.terminal is None
    assert tab.browser is not None
    assert tab.session.shell_opened == 0   # シェルを開かない
    assert not tab.bt_term.isEnabled()
    assert not tab.bt_sendpw.isEnabled()
    assert tab.toggle_session_log() is False   # ターミナルなし → 何もしない
    tab._on_password_prompt("manual")          # None ガードで落ちない
    _cleanup(qapp, tab)


def test_session_tab_both_has_terminal_and_browser(qapp):
    tab = _make_tab(qapp, "both")
    assert tab.terminal is not None and tab.browser is not None
    assert tab.bt_term.isEnabled() and tab.bt_files.isEnabled()
    _cleanup(qapp, tab)


def test_browser_hidden_toggle_syncs_with_menu(qapp):
    """ツールバーの隠しファイルトグルとメニューの状態が同期する(#80)。"""
    tab = _make_tab(qapp, "sftp")
    b = tab.browser
    assert not b.btn_hidden.isChecked()
    b.btn_hidden.setChecked(True)
    assert b._act_hidden.isChecked() and b._show_hidden
    b._act_hidden.setChecked(False)
    assert not b.btn_hidden.isChecked() and not b._show_hidden
    _cleanup(qapp, tab)


# ---- デュアルペイン (Issue #82) --------------------------------------------
def test_session_tab_builds_local_pane_only_with_browser(qapp):
    """ローカルペインはファイル側があるモードにだけ作られる(#82/#112)。"""
    tab = _make_tab(qapp, "ssh")
    assert tab.local is None
    assert tab._files_splitter is None
    assert not tab.bt_local.isEnabled()
    _cleanup(qapp, tab)

    tab = _make_tab(qapp, "sftp")
    assert tab.local is not None
    assert tab._files_splitter is not None
    assert tab.bt_local.isEnabled()
    _cleanup(qapp, tab)


def test_dual_pane_defaults_off_and_toggles(qapp):
    """既定はリモートのみ。トグルでローカル側が出る(既存の体験を壊さない)。"""
    tab = _make_tab(qapp, "both")
    assert not tab.bt_local.isChecked()
    assert not tab._local_pane.isVisibleTo(tab._files_splitter)

    tab.bt_local.setChecked(True)
    assert tab._local_pane.isVisibleTo(tab._files_splitter)
    assert tab.settings.get("dual_pane") is True   # 次回の既定として覚える

    # ファイル側を丸ごと隠すとローカルも消える
    tab.bt_files.setChecked(False)
    assert not tab._local_pane.isVisibleTo(tab._files_splitter)
    _cleanup(qapp, tab)


def test_local_upload_request_reaches_remote_precheck(qapp, tmp_path):
    """ローカルの「→ アップロード」が実際に nav の上書き事前確認へ届く(#82)。

    SessionTab が張った本番の接続をそのまま通す(テスト用に張り直さない)。
    """
    tab = _make_tab(qapp, "sftp")
    src = tmp_path / "a.txt"
    src.write_text("x", encoding="utf-8")
    tab.browser.cwd = "/srv"
    jobs = []
    tab.browser.nav.enqueue = jobs.append   # ワーカーへ流さず横取りする

    tab.local.upload_requested.emit([str(src)])

    assert [j["kind"] for j in jobs] == ["precheck_upload"]
    assert jobs[0]["files"] == [(str(src), "/srv/a.txt")]
    _cleanup(qapp, tab)


def test_local_download_request_reaches_remote_browser(qapp, tmp_path):
    """ローカルへのドロップ/「← ダウンロード」が本番の download_to へ届く。"""
    tab = _make_tab(qapp, "sftp")
    tab.browser.lb_status.setText("")

    tab.local.download_requested.emit(str(tmp_path))

    # 選択が無いので転送は始まらないが、受け口には確かに届いている
    assert "選択してください" in tab.browser.lb_status.text()
    _cleanup(qapp, tab)


def test_sync_browse_is_wired_and_gated_on_dual_pane(qapp):
    """同期ブラウズ(#82 第 2 段)の配線と、2 ペインへの従属を確認する。"""
    tab = _make_tab(qapp, "sftp")
    # 調停役が実際に両ペインへつながっている
    assert tab.sync_browse is not None
    assert tab.sync_browse._local is tab.local
    assert tab.sync_browse._remote is tab.browser
    # 既定は 2 ペイン OFF なので、同期は押せない
    assert not tab.bt_local.isChecked()
    assert not tab.bt_sync.isEnabled()
    assert tab.sync_browse.enabled is False

    tab.bt_local.setChecked(True)
    assert tab.bt_sync.isEnabled()
    tab.bt_sync.setChecked(True)
    assert tab.sync_browse.enabled is True
    assert tab.settings.get("sync_browse") is True

    # 2 ペインを畳むと同期も畳まれる
    tab.bt_local.setChecked(False)
    assert not tab.bt_sync.isChecked()
    assert tab.sync_browse.enabled is False
    _cleanup(qapp, tab)


def test_local_move_enqueues_sync_list_on_remote(qapp, tmp_path):
    """同期 ON でローカルを移動すると、リモートへ list_sync が積まれる。

    通常の list ではなく list_sync を使うのは、追随先が無いのは普通に起こる
    ことで、そのたびにエラーダイアログを出さないため。
    """
    tab = _make_tab(qapp, "sftp")
    (tmp_path / "base" / "logs").mkdir(parents=True)
    tab.local.cd(str(tmp_path / "base"))
    tab.browser.cwd = "/srv/app"
    tab.sync_browse.set_enabled(True)
    jobs = []
    tab.browser.nav.enqueue = jobs.append   # ワーカーへ流さず横取りする

    tab.local.cd(str(tmp_path / "base" / "logs"))

    assert jobs == [{"kind": "list_sync", "path": "/srv/app/logs"}]
    _cleanup(qapp, tab)


def test_sync_list_job_reports_failure_without_error_dialog(qapp):
    """追随先が無いとき、error(ダイアログ)ではなく sync_failed で知らせる。

    左右でフォルダ構成が違うのは当たり前なので、追随の失敗でダイアログを出すと
    同期ブラウズが使い物にならない。通常の list は従来どおり例外を上げる。
    """
    import pytest

    from hashi.filebrowser import SftpWorker

    class _Boom:
        def normalize(self, path):
            raise IOError("No such file")

    worker = SftpWorker(None, "test")     # スレッドは起こさない
    worker.sftp = _Boom()
    errors, failures, listed = [], [], []
    worker.error.connect(errors.append)
    worker.sync_failed.connect(failures.append)
    worker.listed.connect(lambda *a: listed.append(a))

    worker._job_list_sync({"path": "/no/such/dir"})
    assert failures and not errors and not listed

    with pytest.raises(IOError):          # 通常の list は握り潰さない
        worker._job_list({"path": "/no/such/dir"})


def test_download_to_without_selection_does_nothing(qapp, tmp_path):
    """選択が無ければ何も投入しない(ドロップ先だけ決まった状態での事故防止)。"""
    tab = _make_tab(qapp, "sftp")
    before = tab.browser.xfer.q.qsize()
    tab.browser.download_to(str(tmp_path))
    assert tab.browser.xfer.q.qsize() == before
    _cleanup(qapp, tab)
