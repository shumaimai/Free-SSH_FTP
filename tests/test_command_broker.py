import socket
import threading
from types import SimpleNamespace

import paramiko
import pytest

from hashi.command_broker import CommandBroker
from hashi.session_registry import SessionRegistry


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
        entry = registry.register(Backend(), label="ssh", kind="ssh", shell="posix",
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
