"""起動中Hashiへの認証付きloopback IPC。外部HTTP公開はしない。"""
from __future__ import annotations

import csv
import json
import os
import secrets
import shutil
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import uuid
from pathlib import Path

from .ai_core import TerminalTools, redact, tool_definitions
from .jsonio import save_json_atomic

MAX_MESSAGE = 65536
VERSIONS = {"2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"}


def private_directory():
    path = Path(tempfile.mkdtemp(prefix="hashi-mcp-"))
    try:
        if sys.platform == "win32":
            flags = subprocess.CREATE_NO_WINDOW
            result = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"], check=True,
                                    capture_output=True, text=True, timeout=5, creationflags=flags)
            sid = next(csv.reader([result.stdout.strip()]))[-1]
            if not sid.startswith("S-1-") or not all(c.isdigit() or c in "S-" for c in sid):
                raise ValueError("ユーザーSIDを確認できません")
            subprocess.run(["icacls", str(path), "/inheritance:r", "/grant:r",
                            f"*{sid}:(OI)(CI)F", "*S-1-5-18:(OI)(CI)F"], check=True,
                           capture_output=True, timeout=5, creationflags=flags)
        else:
            os.chmod(path, 0o700)
        return path
    except Exception:
        shutil.rmtree(path)
        raise


class McpBridge:
    def __init__(self, broker):
        self.broker = broker
        self.instance, self.token = uuid.uuid4().hex, secrets.token_urlsafe(32)
        self.directory = private_directory()
        self._stopped = False
        self._clients, self._lock = {}, threading.RLock()
        self._slots = threading.BoundedSemaphore(8)
        bridge = self
        class Server(socketserver.ThreadingTCPServer):
            daemon_threads = True
            allow_reuse_address = False

            def process_request(self, request, address):
                if bridge._slots.acquire(blocking=False):
                    super().process_request(request, address)
                else:
                    self.shutdown_request(request)

            def process_request_thread(self, request, address):
                try:
                    super().process_request_thread(request, address)
                finally:
                    bridge._slots.release()

            def handle_error(self, request, address):
                return  # 要求本文・tokenを診断ログに出さない
        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                self.connection.settimeout(5)
                line = self.rfile.readline(MAX_MESSAGE + 1)
                if len(line) > MAX_MESSAGE:
                    return
                try:
                    envelope = json.loads(line)
                    if (bridge._stopped or envelope.get("instance") != bridge.instance or
                            not secrets.compare_digest(str(envelope.get("token", "")), bridge.token)):
                        return
                    client = envelope.get("client")
                    if not isinstance(client, str) or len(client) != 32 or not all(c in "0123456789abcdef" for c in client):
                        return
                    message = envelope.get("message")
                    done = threading.Event()
                    if isinstance(message, dict) and message.get("method") == "tools/call":
                        self.connection.settimeout(.2)
                        def monitor():
                            while not done.is_set():
                                try:
                                    if not self.connection.recv(1, socket.MSG_PEEK):
                                        if not done.is_set():
                                            bridge.disconnect(client)
                                        return
                                except socket.timeout:
                                    continue
                                except OSError:
                                    if not done.is_set():
                                        bridge.disconnect(client)
                                    return
                                done.wait(.1)
                        threading.Thread(target=monitor, daemon=True).start()
                    try:
                        reply = bridge.dispatch(client, message)
                    finally:
                        done.set()
                    raw = json.dumps({"response": reply}, ensure_ascii=False).encode() + b"\n"
                    self.wfile.write(raw)
                except (ValueError, OSError, TypeError):
                    return
        try:
            self.server = Server(("127.0.0.1", 0), Handler)
            self.endpoint = self.directory / "endpoint.json"
            save_json_atomic(self.endpoint, {"instance": self.instance, "token": self.token,
                                            "port": self.server.server_address[1]})
            if sys.platform != "win32":
                os.chmod(self.endpoint, 0o600)
            self.thread = threading.Thread(target=lambda: self.server.serve_forever(.1), daemon=True)
            self.thread.start()
        except Exception:
            if hasattr(self, "server"):
                self.server.server_close()
            shutil.rmtree(self.directory)
            raise

    def config(self):
        if getattr(sys, "frozen", False):
            executable = Path(sys.executable).with_name("HashiMCP.exe")
            if not executable.is_file():
                raise RuntimeError("HashiMCP.exeをHashi.exeと同じフォルダに配置してください")
            command, args = str(executable), [str(self.endpoint)]
        else:
            helper = Path(__file__).resolve().parent.parent / "tools" / "hashi_mcp.py"
            command = sys.executable
            args = [str(helper), str(self.endpoint)] if helper.is_file() else ["-m", "hashi.mcp_stdio", str(self.endpoint)]
        return {"mcpServers": {"hashi": {"type": "stdio", "command": command, "args": args}}}

    def disconnect(self, client):
        with self._lock:
            state = self._clients.pop(client, None)
            if state is not None:
                state["cancel"].set()
            self.broker.cancel_actor("mcp:" + self.instance + ":" + client)

    def dispatch(self, client, message):
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            return {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid request"}}
        request_id = message.get("id")
        method, params = message.get("method"), message.get("params", {})
        response = {"jsonrpc": "2.0", "id": request_id}
        if not isinstance(params, dict) or (request_id is not None and type(request_id) not in (str, int)):
            return {**response, "error": {"code": -32600, "message": "Invalid request"}}
        actor = "mcp:" + self.instance + ":" + client
        try:
            with self._lock:
                if self._stopped:
                    raise PermissionError("Hashi MCPは停止しています")
                if method == "notifications/hashi-disconnect":
                    self.disconnect(client)
                    return None
                if request_id is None and not (isinstance(method, str) and method.startswith("notifications/")):
                    return None  # notificationからツールを実行しない
                if method == "initialize":
                    if len(self._clients) >= 64 and client not in self._clients:
                        raise RuntimeError("接続数の上限です")
                    version = params.get("protocolVersion")
                    version = version if version in VERSIONS else "2025-11-25"
                    if client in self._clients:
                        raise ValueError("このクライアントは初期化済みです")
                    self._clients[client] = {"version": version, "ready": False, "cancel": threading.Event()}
                    return {**response, "result": {"protocolVersion": version,
                            "capabilities": {"tools": {}}, "serverInfo": {"name": "Hashi", "version": "1"}}}
                state = self._clients.get(client)
                if state is None:
                    raise PermissionError("初期化してください")
                if method == "notifications/initialized":
                    state["ready"] = True
                    return None
                if method == "notifications/cancelled":
                    self.broker.cancel_operation(actor, str(params.get("requestId")))
                    return None
                if not state["ready"]:
                    raise PermissionError("初期化が完了していません")
                cancel = state["cancel"]
            if method == "ping":
                result = {}
            elif method == "tools/list":
                result = {"tools": [{"name": d["name"], "description": d["description"],
                                     "inputSchema": d["parameters"]} for d in tool_definitions()]}
            elif method == "tools/call":
                try:
                    result = TerminalTools(self.broker, actor, cancel).call(params.get("name"), params.get("arguments", {}), str(request_id))
                    result = {"content": [{"type": "text", "text": redact(json.dumps(result, ensure_ascii=False))}], "isError": False}
                except (ValueError, PermissionError, RuntimeError, OSError) as exc:
                    result = {"content": [{"type": "text", "text": redact(str(exc))}], "isError": True}
            elif isinstance(method, str) and method.startswith("notifications/"):
                return None
            else:
                return {**response, "error": {"code": -32601, "message": "Method not found"}}
            return {**response, "result": result} if request_id is not None else None
        except (ValueError, PermissionError, RuntimeError) as exc:
            return {**response, "error": {"code": -32000, "message": redact(str(exc))}} if request_id is not None else None

    def close(self):
        if self._stopped:
            return
        self._stopped = True
        self.broker.stop()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(1)
        shutil.rmtree(self.directory)
