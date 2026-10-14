import json
import os
import queue
import subprocess
import sys
import threading
import uuid
from contextlib import contextmanager
from pathlib import Path

import pytest

from hashi.command_broker import CommandBroker
from hashi.mcp_bridge import McpBridge
from hashi.mcp_stdio import request
from hashi.session_registry import SessionRegistry


class Backend:
    def __init__(self):
        self.sent = []

    def send(self, data):
        self.sent.append(data)


@pytest.fixture
def bridge():
    registry, backend = SessionRegistry(), Backend()
    entry = registry.register(backend, label="対象", kind="local", shell="cmd")
    registry.register(Backend(), label="除外", kind="local", shell="cmd")
    broker = CommandBroker(registry)
    broker.configure("auto", [(entry.id, entry.generation)])
    value = McpBridge(broker)
    yield value, entry, backend
    value.close()
    assert not value.directory.exists()


def msg(method, request_id=None, **params):
    value = {"jsonrpc": "2.0", "method": method, "params": params}
    if request_id is not None:
        value["id"] = request_id
    return value


def initialize(call):
    reply = call(msg("initialize", 1, protocolVersion="2025-11-25", capabilities={},
                     clientInfo={"name": "test", "version": "1"}))
    assert reply["result"]["protocolVersion"] == "2025-11-25"
    call(msg("notifications/initialized"))


def test_large_mcp_result_is_valid_bounded_json_after_secret_masking(bridge, tmp_path, monkeypatch):
    value, entry, backend = bridge
    monkeypatch.setattr(value.broker, "perform", lambda *args, **kwargs: {
        "output": "password=private\n" + '日本語\\"' * 16000,
        "error": "E" * 65536, "completion": "known", "exit_code": 7, "status": "completed"})
    with helper(value, tmp_path) as (_, call, _, _):
        initialize(call)
        reply = call(msg("tools/call", 22, name="run_command", arguments={
            "session_id": entry.id, "generation": entry.generation, "text": "echo test", "cwd": str(tmp_path)}))
        content = reply["result"]["content"][0]["text"]
        result = json.loads(content)
        assert not reply["result"]["isError"] and len(content.encode()) <= 32000
        assert "private" not in content and result["truncated"]
        assert result["session_id"] == entry.id and result["exit_code"] == 7


@contextmanager
def helper(value, cwd, executable=None):
    config = value.config()["mcpServers"]["hashi"]
    args = [config["command"], *config["args"]] if executable is None else [str(executable), str(value.endpoint)]
    process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, encoding="utf-8", cwd=cwd)
    replies = queue.Queue()
    def read():
        for line in process.stdout:
            replies.put(json.loads(line))  # 診断ログがstdoutへ混じれば失敗する
    threading.Thread(target=read, daemon=True).start()
    def send(message):
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()
    def call(message):
        send(message)
        return replies.get(timeout=10) if "id" in message else None
    try:
        yield process, call, send, replies
    finally:
        if process.poll() is None:
            process.stdin.close()
            try:
                process.wait(10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(5)


def test_real_stdio_scope_input_stale_and_disconnect(bridge, tmp_path):
    value, entry, backend = bridge
    with helper(value, tmp_path) as (process, call, send, replies):
        initialize(call)
        tools = call(msg("tools/list", 2))["result"]["tools"]
        assert {t["name"] for t in tools} >= {"run_command", "cancel_command", "read_output"}
        result = call(msg("tools/call", 3, name="list_sessions", arguments={}))
        entries = json.loads(result["result"]["content"][0]["text"])
        assert [e["session_id"] for e in entries] == [entry.id]
        args = {"session_id": entry.id, "generation": entry.generation, "text": "echo 日本語\n"}
        result = call(msg("tools/call", 4, name="send_input", arguments=args))
        assert json.loads(result["result"]["content"][0]["text"])["completion"] == "unknown"
        assert backend.sent == ["echo 日本語\r".encode()]
        send(msg("tools/call", name="send_input", arguments=args))
        assert call(msg("ping", 5))["id"] == 5
        assert len(backend.sent) == 1  # idなしnotificationは操作しない
        value.broker.registry.register(Backend(), label="再接続", kind="local", shell="cmd", session_id=entry.id)
        assert call(msg("tools/call", 6, name="send_input", arguments=args))["result"]["isError"]
        process.stdin.close()
        assert process.wait(10) == 0
    assert not value._clients


def test_authenticated_instance_and_private_endpoint(bridge):
    value, _, _ = bridge
    endpoint = json.loads(value.endpoint.read_text(encoding="utf-8"))
    client = uuid.uuid4().hex
    for altered in ({**endpoint, "token": "bad"}, {**endpoint, "instance": "other"}):
        with pytest.raises(RuntimeError):
            request(altered, client, msg("initialize", 1))
    if sys.platform != "win32":
        assert os.stat(value.directory).st_mode & 0o777 == 0o700
        assert os.stat(value.endpoint).st_mode & 0o777 == 0o600
    assert value.token not in json.dumps(value.config())
    other = McpBridge(CommandBroker(SessionRegistry()))
    try:
        assert other.instance != value.instance and other.token != value.token
        assert other.server.server_address != value.server.server_address
        with pytest.raises(RuntimeError):
            request({**endpoint, "port": other.server.server_address[1]}, client, msg("ping", 2))
    finally:
        other.close()


def test_cancellation_when_all_worker_slots_busy_and_eof(bridge, tmp_path):
    value, entry, backend = bridge
    pending = queue.Queue()
    def approve(operation):
        pending.put(operation)
        operation.cancel.wait(10)
        return False
    value.broker.approval = approve
    value.broker.configure("confirm", [(entry.id, entry.generation)])
    with helper(value, tmp_path) as (process, call, send, replies):
        initialize(call)
        for request_id in range(10, 14):
            send(msg("tools/call", request_id, name="send_input", arguments={
                "session_id": entry.id, "generation": entry.generation, "text": "x"}))
        operations = [pending.get(timeout=10) for _ in range(4)]
        send(msg("notifications/cancelled", requestId=10))
        assert replies.get(timeout=10)["id"] == 10
        assert next(o for o in operations if o.id == "10").cancel.is_set()
        # 単独取消がクライアント全体の取消になることを防ぐ。
        assert not next(o for o in operations if o.id == "11").cancel.is_set()
        process.stdin.close()
        assert process.wait(10) == 0
        assert all(o.cancel.is_set() for o in operations)
        assert backend.sent == []


@pytest.mark.skipif(sys.platform != "win32" or not Path("dist/HashiMCP.exe").exists(), reason="Windowsビルド後に検証")
def test_packaged_stdio_helper(bridge, tmp_path):
    value, _, _ = bridge
    with helper(value, tmp_path, Path("dist/HashiMCP.exe").resolve()) as (_, call, _, _):
        initialize(call)
        assert call(msg("ping", 2))["result"] == {}


def test_helper_process_death_cancels_pending_approval(bridge, tmp_path):
    value, entry, backend = bridge
    pending = queue.Queue()
    def approve(operation):
        pending.put(operation)
        operation.cancel.wait(10)
        return False
    value.broker.approval = approve
    value.broker.configure("confirm", [(entry.id, entry.generation)])
    with helper(value, tmp_path) as (process, call, send, _):
        initialize(call)
        send(msg("tools/call", 4, name="send_input", arguments={
            "session_id": entry.id, "generation": entry.generation, "text": "x"}))
        operation = pending.get(timeout=10)
        process.kill()
        process.wait(5)
        assert operation.cancel.wait(5)
        assert backend.sent == []
