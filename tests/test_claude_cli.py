import json
import subprocess
import sys
import threading
import time
from types import SimpleNamespace

import pytest

from hashi.claude_cli import (
    REQUIRED_FLAGS,
    CliLaunchDialog,
    OfficialCliPage,
    launch_arguments,
    probe_cli,
)
from hashi.command_broker import CommandBroker
from hashi.config import Settings
from hashi.local_terminal import ConPtyBackend
from hashi.mcp_bridge import McpBridge
from hashi.session_registry import SessionRegistry


def wait_gui(qapp, condition):
    deadline = time.monotonic() + 5
    while not condition() and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert condition()


def test_probe_requires_version_and_actual_options(tmp_path, monkeypatch):
    path = tmp_path / "claude.exe"
    path.touch()
    calls = []
    outputs = ["2.1.268 (Claude Code)", " ".join(REQUIRED_FLAGS)]
    def run(args, **kwargs):
        calls.append(args)
        assert "env" not in kwargs and not kwargs.get("shell")
        return SimpleNamespace(stdout=outputs.pop(0))
    monkeypatch.setattr(subprocess, "run", run)
    assert probe_cli(path)["version"] == "2.1.268"
    assert calls == [[str(path), "--version"], [str(path), "--help"]]
    for outputs in (["1.9.0 (Claude Code)"], ["2.1.268 (Other CLI)"], ["2.1.268 (Claude Code)", "--tools"]):
        with pytest.raises(ValueError):
            probe_cli(path)
    with pytest.raises(ValueError):
        probe_cli(tmp_path / "claude.cmd")


def test_profiles_keep_auth_and_permissions_native():
    args = launch_arguments("C:/Program Files/claude.exe", "C:/private/mcp.json")
    assert args[args.index("--tools") + 1] == ""
    assert json.loads(args[args.index("--settings") + 1]) == {"disableAllHooks": True}
    assert not any(a in args for a in ("--bare", "--dangerously-skip-permissions", "--allowedTools", "--permission-mode", "--setting-sources"))
    normal = launch_arguments("claude.exe", "config.json", builtins=True, disable_hooks=False, resume=True)
    assert normal[normal.index("--tools") + 1] == "default"
    assert "--settings" not in normal and "--continue" in normal


def test_mcp_result_visible_in_hashi_without_command_or_output(qapp):
    from hashi.ai_panel import AiPanel
    registry = SessionRegistry()
    backend = SimpleNamespace(send=lambda data: len(data))
    entry = registry.register(backend, label="target", kind="local", shell="cmd")
    broker = CommandBroker(registry)
    panel = AiPanel(broker)
    broker.configure("auto", [(entry.id, entry.generation)])
    broker.perform("mcp:instance:client", "send_input", entry.id, entry.generation,
                   text="password=never-record\n", request_id="visible")
    panel._refresh_audit()
    text = panel.audit_view.toPlainText()
    assert entry.id in text and "sent" in text and "unknown" in text
    assert "never-record" not in text and "password" not in text
    assert "never-record" not in json.dumps(broker.audit_snapshot())
    panel.stop()
    panel.deleteLater()
    qapp.processEvents()


def test_probe_dialog_defers_close_while_worker_runs(qapp, monkeypatch, tmp_path):
    import hashi.claude_cli as cli
    entered, finish = threading.Event(), threading.Event()
    def probe(path, cancel):
        entered.set()
        finish.wait(5)
        return {"path": str(tmp_path / "claude.exe"), "version": "2.1.268"}
    monkeypatch.setattr(cli, "probe_cli", probe)
    dialog = CliLaunchDialog()
    dialog.show()
    dialog._probe()
    assert entered.wait(5)
    dialog.reject()
    assert dialog._reject_pending and dialog.isVisible()
    finish.set()
    wait_gui(qapp, lambda: dialog.worker is None)
    assert not dialog.isVisible() and dialog.verified is None
    dialog.deleteLater()
    qapp.processEvents()


def test_cli_page_auth_output_never_enters_registry_or_hashi_log(qapp, tmp_config, monkeypatch):
    class Backend:
        is_terminal_backend = True
        def __init__(self):
            self.closed = threading.Event()
            self.sent = []

        def recv(self, size):
            self.closed.wait(5)
            return b""

        def send(self, data):
            self.sent.append(data)

        def resize_pty(self, **kwargs):
            return None

        def close(self):
            self.closed.set()
    backend, starts = Backend(), []
    def spawn(**kwargs):
        starts.append(kwargs)
        return backend
    monkeypatch.setattr(ConPtyBackend, "spawn", spawn)
    registry = SessionRegistry()
    bridge = McpBridge(CommandBroker(registry))
    try:
        launch = {"path": "claude.exe", "cwd": str(tmp_config), "builtins": False,
                  "disable_hooks": True, "resume": False}
        page = OfficialCliPage(Settings(), registry, bridge, launch)
        wait_gui(qapp, lambda: page.terminal._channel is not None)
        assert page.binding is None
        page.terminal.output_received.emit(b"OAUTH_CODE=private-code")
        page.terminal._on_data(b"\r\n[sudo] password for fake:\r")
        assert registry.list() == [] and page.terminal._session_log is None
        assert not backend.sent
        assert starts[0]["cwd"] == str(tmp_config)
        assert starts[0]["argv"][0] == "claude.exe"
        config = page.config_path
        assert json.loads(config.read_text())["mcpServers"]["hashi"]["args"][-1] == str(bridge.endpoint)
        assert bridge.token not in config.read_text()
        page._interrupt()
        assert backend.sent == [b"\x03"]
        page.shutdown()
        assert backend.closed.is_set() and not config.exists()
        page.deleteLater()
        qapp.processEvents()
    finally:
        bridge.close()


@pytest.mark.skipif(sys.platform != "win32", reason="Windows ConPTY引数の実検証")
def test_native_conpty_preserves_empty_argument_and_unicode(tmp_path):
    config_path = tmp_path / "空白 フォルダ" / "mcp.json"
    args = launch_arguments(sys.executable, config_path)
    script = "import sys,json;open('args.json','w',encoding='utf-8').write(json.dumps(sys.argv[1:]));print('HASHI_ARGS_OK');input()"
    backend = ConPtyBackend.spawn(argv=[sys.executable, "-c", script, *args[1:]], cwd=tmp_path)
    data, received = bytearray(), threading.Event()
    def read():
        while chunk := backend.recv(4096):
            data.extend(chunk)
            if b"HASHI_ARGS_OK" in data:
                received.set()
    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    try:
        backend.resize_pty(width=120, height=30)
        assert received.wait(15)
        actual = json.loads((tmp_path / "args.json").read_text(encoding="utf-8"))
        assert actual == args[1:]
    finally:
        backend.close()
        reader.join(2)
    assert not reader.is_alive()
