"""sshd 堅牢化(Issue #12)のテスト。

実 sshd を触らずに、コマンド発行のシーケンスと安全ガードを検証する。
FakeSession は run_sudo / run_sudo_stdin の呼び出しを記録し、応答を差し込める。
"""
import pytest

from hashi import sshd_admin
from hashi.sshd_admin import SshdAdminError, apply_changes, build_dropin


class _FakeSftpFile:
    def __init__(self, store, path):
        self.store, self.path, self.buf = store, path, b""

    def write(self, data):
        self.buf += data

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.store[self.path] = self.buf


class _FakeSftp:
    def __init__(self, store):
        self.store = store

    def open(self, path, mode="r"):
        return _FakeSftpFile(self.store, path)

    def close(self):
        pass


class FakeSession:
    def __init__(self):
        self._hashi_sudo_pw = "pw"
        self.calls = []          # (command, kind)
        self.responses = {}      # command 部分一致 -> (rc, out, err)
        self.tmp_files = {}       # SFTP で書かれた一時ファイル
        self.reloaded = 0
        self.removed = False

    def _match(self, command):
        for key, resp in self.responses.items():
            if key in command:
                return resp
        return (0, "", "")

    def exec_command(self, command, timeout=15.0):
        self.calls.append((command, "exec"))
        if 'printf "%s" "$HOME"' in command:
            return (0, "/home/tester", "")
        return self._match(command)

    def open_sftp(self):
        return _FakeSftp(self.tmp_files)

    def run_sudo(self, command, password, timeout=20.0):
        self.calls.append((command, "sudo"))
        if command.startswith("systemctl reload") or "pkill -HUP" in command:
            self.reloaded += 1
        if command.startswith(f"rm -f {sshd_admin.DROPIN_PATH}"):
            self.removed = True
        return self._match(command)

    @property
    def dropin_content(self):
        # install コマンドで配置した一時ファイルの中身
        for path, buf in self.tmp_files.items():
            if path.endswith(".hashi-sshd-dropin.tmp"):
                return buf.decode("utf-8")
        return None

    def commands(self):
        return [c for c, _ in self.calls]


def _with_include(sess):
    sess.responses[f"cat {sshd_admin.MAIN_CONFIG}"] = (
        0, "Include /etc/ssh/sshd_config.d/*.conf\n", "")


def test_build_dropin_variants():
    assert build_dropin(True, 2222) == (
        "# Managed by Hashi (Issue #12). 消せば元の設定に戻ります。\n"
        "Port 2222\n"
        "PasswordAuthentication no\n"
        "KbdInteractiveAuthentication no\n")
    assert "PasswordAuthentication yes" in build_dropin(False, None)
    assert "Port" not in build_dropin(True, None)


def test_read_effective_parses_sshd_T():
    sess = FakeSession()
    sess.responses["sshd -T"] = (
        0, "port 22\nport 2222\npasswordauthentication yes\nx11forwarding no\n", "")
    eff = sshd_admin.read_effective(sess)
    assert eff["port"] == [22, 2222]
    assert eff["passwordauthentication"] == "yes"


def test_disable_password_blocked_without_key_login():
    sess = FakeSession()
    _with_include(sess)
    with pytest.raises(SshdAdminError, match="締め出し|鍵でログイン"):
        apply_changes(sess, disable_password=True,
                      verify_key_login=lambda: False)
    # 何も書き込んでいない
    assert sess.dropin_content is None
    assert all(not c.startswith("install ") for c in sess.commands())


def test_disable_password_proceeds_when_key_login_ok():
    sess = FakeSession()
    _with_include(sess)
    res = apply_changes(sess, disable_password=True, new_port=2222,
                        verify_key_login=lambda: True,
                        verify_reachable=lambda port: True)
    assert res["reloaded"] is True
    assert "PasswordAuthentication no" in sess.dropin_content
    assert "Port 2222" in sess.dropin_content
    # 順序: バックアップ → 書き込み → sshd -t → reload
    cmds = sess.commands()
    i_bak = next(i for i, c in enumerate(cmds) if c.startswith("cp -a"))
    i_write = next(i for i, c in enumerate(cmds) if c.startswith("install "))
    i_val = next(i for i, c in enumerate(cmds) if c.startswith("sshd -t"))
    i_reload = next(i for i, c in enumerate(cmds) if "reload" in c)
    assert i_bak < i_write < i_val < i_reload


def test_syntax_error_aborts_and_removes_dropin():
    sess = FakeSession()
    _with_include(sess)
    sess.responses["sshd -t"] = (1, "", "bad config line 3")
    with pytest.raises(SshdAdminError, match="構文エラー"):
        apply_changes(sess, new_port=2222)
    assert sess.removed is True        # ドロップインを消して原状復帰
    assert sess.reloaded == 0          # reload はしていない


def test_unreachable_rolls_back():
    sess = FakeSession()
    _with_include(sess)
    with pytest.raises(SshdAdminError, match="疎通確認"):
        apply_changes(sess, new_port=2222,
                      verify_reachable=lambda port: False)
    assert sess.removed is True
    assert sess.reloaded == 2          # 適用の reload + ロールバックの reload


def test_missing_include_is_rejected():
    sess = FakeSession()
    sess.responses[f"cat {sshd_admin.MAIN_CONFIG}"] = (
        0, "# no include here\nPort 22\n", "")
    with pytest.raises(SshdAdminError, match="ドロップイン"):
        apply_changes(sess, new_port=2222)


def test_invalid_port_rejected():
    sess = FakeSession()
    _with_include(sess)
    with pytest.raises(SshdAdminError, match="ポート番号"):
        apply_changes(sess, new_port=70000)


def test_no_changes_rejected():
    sess = FakeSession()
    with pytest.raises(SshdAdminError, match="変更内容"):
        apply_changes(sess)
