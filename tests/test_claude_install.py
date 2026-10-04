import io
import threading
from pathlib import Path
from types import SimpleNamespace

import pytest

import hashi.claude_cli as cli
import hashi.claude_install as installer


@pytest.fixture
def official_installer(monkeypatch):
    script = b"# Official installer bytes preserved\r\n"
    monkeypatch.setattr(installer.sys, "platform", "win32")
    monkeypatch.setattr(installer.subprocess, "CREATE_NO_WINDOW", 0, raising=False)
    monkeypatch.setattr(installer.urllib.request, "build_opener", lambda _: SimpleNamespace(
        open=lambda url, timeout: io.BytesIO(script)))
    monkeypatch.setattr(cli, "find_cli", lambda: "C:/Users/test/.local/bin/claude.exe")
    monkeypatch.setattr(cli, "probe_cli", lambda path, cancel: {"path": path, "version": "2.1.268"})
    return script


def test_installs_exact_official_script_without_shell_or_elevation(official_installer, monkeypatch):
    calls = []
    def spawn(args, **kwargs):
        assert not kwargs.get("shell") and "env" not in kwargs
        script_path = Path(args[args.index("-File") + 1])
        assert script_path.read_bytes() == official_installer
        assert args[-1] == "stable" and "-NonInteractive" in args
        calls.append(script_path)
        return SimpleNamespace(poll=lambda: 0, returncode=0)
    monkeypatch.setattr(installer.subprocess, "Popen", spawn)
    result = installer.install_cli(threading.Event())
    assert result["path"].endswith("claude.exe")
    assert calls and not calls[0].exists()


def test_cancelling_installer_terminates_its_process_tree(official_installer, monkeypatch):
    cancel, state, killed = threading.Event(), {"alive": True}, []
    def spawn(args, **kwargs):
        cancel.set()
        return SimpleNamespace(pid=1234, poll=lambda: None if state["alive"] else 1,
                               wait=lambda timeout: state.update(alive=False))
    def kill(args, **kwargs):
        killed.append(args)
        state["alive"] = False
    monkeypatch.setattr(installer.subprocess, "Popen", spawn)
    monkeypatch.setattr(installer.subprocess, "run", kill)
    with pytest.raises(InterruptedError):
        installer.install_cli(cancel)
    assert killed == [["taskkill", "/PID", "1234", "/T", "/F"]]


def test_failed_installer_does_not_probe_or_leak_its_output(official_installer, monkeypatch):
    monkeypatch.setattr(installer.subprocess, "Popen", lambda *args, **kwargs: SimpleNamespace(poll=lambda: 1, returncode=1))
    monkeypatch.setattr(cli, "probe_cli", lambda *args: pytest.fail("failed installation must not probe"))
    with pytest.raises(RuntimeError, match="公式インストーラーが失敗"):
        installer.install_cli(threading.Event())


def test_cancellation_during_download_never_starts_installer(official_installer, monkeypatch):
    cancel = threading.Event()
    class Response(io.BytesIO):
        def read(self, size):
            cancel.set()
            return super().read(size)
    monkeypatch.setattr(installer.urllib.request, "build_opener", lambda _: SimpleNamespace(
        open=lambda *args, **kwargs: Response(official_installer)))
    monkeypatch.setattr(installer.subprocess, "Popen", lambda *args, **kwargs: pytest.fail("cancelled installer started"))
    with pytest.raises(InterruptedError):
        installer.install_cli(cancel)


@pytest.mark.parametrize("url", ["http://claude.ai/install.ps1", "https://untrusted.example/install.ps1"])
def test_installer_rejects_unofficial_redirects(url):
    with pytest.raises(RuntimeError, match="接続先"):
        installer.OfficialRedirect().redirect_request(None, None, 302, "", {}, url)


def test_missing_cli_is_installed_automatically_and_uses_native_defaults(qapp, monkeypatch, tmp_path):
    import time
    monkeypatch.setattr(cli, "find_cli", lambda: "")
    monkeypatch.setattr(installer, "install_cli", lambda cancel: {"path": str(tmp_path / "claude.exe"), "version": "2.1.268"})
    dialog = cli.CliLaunchDialog(auto_setup=True)
    dialog.show()
    deadline = time.monotonic() + 5
    while (dialog.verified is None or dialog.worker is not None) and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert dialog.worker is None and dialog.verified is not None
    assert dialog.mode.currentData() is True and not dialog.hooks.isChecked()
    dialog.reject()
    dialog.deleteLater()
    qapp.processEvents()
