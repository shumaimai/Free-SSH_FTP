"""MCP stdioヘルパー。JSON-RPC以外はstdoutへ書かない。"""
from __future__ import annotations

import json
import socket
import sys
import threading
import uuid
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

MAX_MESSAGE = 65536


def request(endpoint, client, message):
    data = {**endpoint, "client": client, "message": message}
    raw = json.dumps(data).encode() + b"\n"
    if len(raw) > MAX_MESSAGE:
        raise ValueError("MCP要求が大きすぎます")
    with socket.create_connection(("127.0.0.1", int(endpoint["port"])), timeout=5) as connection:
        connection.settimeout(180)
        connection.sendall(raw)
        with connection.makefile("rb") as reader:
            line = reader.readline(262145)
            if not line or len(line) > 262144:
                raise RuntimeError("起動中のHashiへ接続できません")
            return json.loads(line)["response"]


def serve(endpoint, stdin, stdout):
    client, lock = uuid.uuid4().hex, threading.Lock()
    slots = threading.BoundedSemaphore(4)
    def write(value):
        if value is not None:
            with lock:
                stdout.write(json.dumps(value, ensure_ascii=False) + "\n")
                stdout.flush()
    def send(message, release=False):
        try:
            write(request(endpoint, client, message))
        except Exception:
            if message.get("id") is not None:
                write({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32000,
                        "message": "Hashiへ接続できません。起動状態とMCPの許可を確認してください"}})
        finally:
            if release:
                slots.release()
    with ThreadPoolExecutor(max_workers=4) as executor:
        while True:
            line = stdin.readline(MAX_MESSAGE + 1)
            if not line:
                send({"jsonrpc": "2.0", "method": "notifications/hashi-disconnect"})
                break
            if len(line.encode("utf-8")) > MAX_MESSAGE:
                write({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Message too large"}})
                break
            try:
                message = json.loads(line)
                if not isinstance(message, dict):
                    raise ValueError()
            except ValueError:
                write({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Parse error"}})
                continue
            # initialize/initializedは順番を保持し、操作要求だけを並列化する。
            if message.get("method") != "tools/call":
                send(message)
            elif slots.acquire(blocking=False):
                executor.submit(send, message, True)
            else:
                write({"jsonrpc": "2.0", "id": message.get("id"), "error": {"code": -32000, "message": "Too many requests"}})


def main():
    if len(sys.argv) != 2:
        return 2
    try:
        endpoint = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
        if hasattr(sys.stdin, "reconfigure"):
            sys.stdin.reconfigure(encoding="utf-8")
            sys.stdout.reconfigure(encoding="utf-8")
        serve(endpoint, sys.stdin, sys.stdout)
        return 0
    except Exception:
        sys.stderr.write("Hashi MCPを開始できません。Hashiから接続設定を生成してください。\n")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
