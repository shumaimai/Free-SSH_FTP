import socket
import subprocess
import sys
import threading
import time
from types import SimpleNamespace

import paramiko
import pytest

from hashi.command_broker import CommandBroker
from hashi.session_registry import SessionRegistry


@pytest.mark.parametrize("cancelled", [False, True])
def test_real_ssh_unacknowledged_exec_obeys_timeout_or_stop(cancelled):
    """サーバーがexec要求をACKしなくても、専用チャネルだけを閉じて戻る。"""
    import time

    entered, release = threading.Event(), threading.Event()
    client_socket, server_socket = socket.socketpair()
    server_transport = paramiko.Transport(server_socket)
    server_transport.add_server_key(paramiko.RSAKey.generate(2048))

    class Server(paramiko.ServerInterface):
        def check_auth_password(self, username, password):
            return paramiko.AUTH_SUCCESSFUL

        def check_channel_request(self, kind, chanid):
            return paramiko.OPEN_SUCCEEDED

        def check_channel_exec_request(self, channel, command):
            entered.set()
            release.wait(5)
            return True

    server = threading.Thread(target=lambda: server_transport.start_server(server=Server()), daemon=True)
    server.start()
    client = paramiko.Transport(client_socket)
    stopper = None
    try:
        client.connect(username="test", password="test")
        registry = SessionRegistry()
        entry = registry.register(Backend(), label="ssh", kind="ssh", shell="unknown",
                                  ssh_session=SimpleNamespace(transport=client))
        broker = CommandBroker(registry)
        broker.configure("auto", [(entry.id, entry.generation)])
        if cancelled:
            def stop():
                if entered.wait(2):
                    broker.stop()
            stopper = threading.Thread(target=stop)
            stopper.start()
        started = time.monotonic()
        result = broker.perform("test", "run_command", entry.id, entry.generation,
                                text="test", timeout=2 if cancelled else .1)
        assert entered.is_set()
        assert time.monotonic() - started < (1 if cancelled else 2)
        assert result["status"] == ("cancelled" if cancelled else "timeout")
        assert result["completion"] == "unknown" and result["exit_code"] is None
        assert client.is_active()
    finally:
        release.set()
        client.close()
        server_transport.close()
        server.join(2)
        if stopper is not None:
            stopper.join(2)


class Backend:
    def __init__(self):
        self.sent = []

    def send(self, data):
        self.sent.append(data)

    def interrupt(self):
        self.sent.append(b"\x03")


@pytest.fixture
def setup():
    registry = SessionRegistry()
    backend = Backend()
    entry = registry.register(backend, label="test", kind="local", shell="cmd")
    broker = CommandBroker(registry)
    return registry, backend, entry, broker


def test_consult_scope_duplicate_and_stop(setup):
    registry, backend, entry, broker = setup
    args = ("ai", "send_input", entry.id, entry.generation)
    with pytest.raises(PermissionError):
        broker.perform(*args, text="echo hello\n")
    broker.configure("consult", [(entry.id, entry.generation)])
    assert len(broker.list_sessions()) == 1
    with pytest.raises(PermissionError):
        broker.perform(*args)
    broker.configure("auto", [(entry.id, entry.generation)])
    for _ in range(2):
        result = broker.perform(*args, text="echo hello\n", request_id="same")
    assert backend.sent == [b"echo hello\r"]
    assert result["completion"] == "unknown"
    with pytest.raises(ValueError):
        broker.perform(*args, text="different", request_id="same")
    broker.stop()
    assert broker.list_sessions() == []
    with pytest.raises(PermissionError):
        broker.perform(*args)
    assert "text" not in broker.audit[-1]


def test_approval_revalidates_generation_and_stop(setup):
    registry, backend, entry, broker = setup
    def replace(operation):
        registry.register(Backend(), label="new", kind="local", shell="cmd", session_id=entry.id)
        return True
    broker.approval = replace
    broker.configure("confirm", [(entry.id, entry.generation)])
    with pytest.raises(ValueError):
        broker.perform("ai", "send_input", entry.id, entry.generation, text="x")
    assert not backend.sent


def test_dedicated_cli_not_shareable(setup):
    registry, backend, entry, broker = setup
    entry.shareable = False
    with pytest.raises(ValueError):
        broker.configure("auto", [(entry.id, entry.generation)])


def test_real_ssh_exec_stdout_stderr_and_exit_code():
    """実ParamikoトランスポートとTCPを通す。SFTP/対話PTYを使わない。"""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    transports = []
    errors = []
    class Server(paramiko.ServerInterface):
        def check_auth_password(self, username, password):
            return paramiko.AUTH_SUCCESSFUL

        def check_channel_request(self, kind, chanid):
            return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

        def check_channel_exec_request(self, channel, command):
            assert command == b"test-command"
            def output():
                channel.send("日本語\n".encode())
                channel.send_stderr(b"error\n")
                channel.send_exit_status(7)
                channel.shutdown_write()
            threading.Thread(target=output, daemon=True).start()
            return True
    def serve():
        try:
            conn, _ = listener.accept()
            transport = paramiko.Transport(conn)
            transports.append(transport)
            transport.add_server_key(paramiko.RSAKey.generate(2048))
            transport.start_server(server=Server())
            channel = transport.accept(10)
            if channel is not None:
                threading.Event().wait(.2)
        except Exception as exc:
            errors.append(exc)
    server = threading.Thread(target=serve, daemon=True)
    server.start()
    client = paramiko.Transport(listener.getsockname())
    try:
        client.connect(username="user", password="test")
        registry = SessionRegistry()
        entry = registry.register(Backend(), label="ssh", kind="ssh", shell="unknown",
                                  ssh_session=SimpleNamespace(transport=client))
        broker = CommandBroker(registry)
        broker.configure("auto", [(entry.id, entry.generation)])
        result = broker.perform("test", "run_command", entry.id, entry.generation,
                                text="test-command", timeout=5)
        assert result == {"status": "completed", "completion": "known", "exit_code": 7,
                          "output": "日本語\n", "error": "error\n"}
    finally:
        client.close()
        listener.close()
        for transport in transports:
            transport.close()
        server.join(2)
    assert not errors


@pytest.mark.skipif(sys.platform != "win32", reason="Windows独立CMDの実検証")
def test_real_local_cmd_cwd_unicode_stderr_exit_and_timeout(setup, tmp_path):
    registry, backend, entry, broker = setup
    broker.configure("auto", [(entry.id, entry.generation)])
    result = broker.perform("test", "run_command", entry.id, entry.generation,
                            text="echo 日本語 & echo !literal! & for %i in (OK) do @echo %i & echo stderr-test 1>&2 & cd & exit /b 7", cwd=str(tmp_path))
    assert result["completion"] == "known" and result["exit_code"] == 7
    assert "日本語" in result["output"] and str(tmp_path).lower() in result["output"].lower()
    assert "stderr-test" in result["error"] and result["output_complete"]
    assert "!literal!" in result["output"] and "OK" in result["output"]
    assert backend.sent == []  # 対話端末へ送らない
    start = time.monotonic()
    result = broker.perform("test", "run_command", entry.id, entry.generation,
                            text="ping -n 30 127.0.0.1 >nul", cwd=str(tmp_path), timeout=1)
    assert time.monotonic() - start < 8
    assert result["status"] == "timeout" and result["completion"] == "unknown"
    assert result["exit_code"] is None


@pytest.mark.skipif(sys.platform != "win32", reason="Windows子プロセスのパイプ実検証")
def test_local_detached_child_does_not_block_pipe_close(setup, tmp_path):
    registry, backend, entry, broker = setup
    parent = tmp_path / "空白 フォルダ" / "parent.py"
    parent.parent.mkdir()
    pid_file = tmp_path / "child.pid"
    parent.write_text(
        "import subprocess,sys,pathlib\n"
        "p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'],stdout=sys.stdout,stderr=sys.stderr)\n"
        f"pathlib.Path({str(pid_file)!r}).write_text(str(p.pid))\n"
        "print('parent exited')\n", encoding="utf-8")
    broker.configure("auto", [(entry.id, entry.generation)])
    start = time.monotonic()
    try:
        result = broker.perform("test", "run_command", entry.id, entry.generation,
                                text=subprocess.list2cmdline([sys.executable, str(parent)]),
                                cwd=str(tmp_path), timeout=2)
        assert time.monotonic() - start < 5
        assert "parent exited" in result["output"]
        assert result["exit_code"] == 0 and result["completion"] == "unknown"
        assert result["output_complete"] is False
    finally:
        if pid_file.exists():
            child_pid = int(pid_file.read_text())
            subprocess.run(["taskkill", "/PID", str(child_pid), "/T", "/F"],
                           capture_output=True, timeout=5, check=False,
                           creationflags=subprocess.CREATE_NO_WINDOW)
