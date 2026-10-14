"""端末の描画から接続方式を分離する。

recv/send/resize_pty は既存チャネルと同じ形にし、キー変換や描画には手を加えない。
"""
from __future__ import annotations

import threading
from typing import Protocol


class TerminalBackend(Protocol):
    is_terminal_backend: bool

    def recv(self, size: int) -> bytes: ...
    def send(self, data: bytes) -> int: ...
    def resize_pty(self, *, width: int, height: int) -> None: ...
    def interrupt(self) -> None: ...
    def close(self) -> None: ...


class SshTerminalBackend:
    """対話シェル専用のアダプター。SFTP用チャネルは受け取らない。"""

    is_terminal_backend = True

    def __init__(self, channel):
        self.channel = channel
        self._closed = False
        self._write_lock = threading.RLock()

    def recv(self, size: int) -> bytes:
        return self.channel.recv(size)

    def send(self, data: bytes) -> int:
        with self._write_lock:
            if self._closed:
                raise EOFError("端末は閉じています")
            offset = 0
            while offset < len(data):
                count = self.channel.send(data[offset:])
                if count is None:  # 旧テスト用チャネルとの互換
                    count = len(data) - offset
                if count <= 0:
                    raise EOFError("端末への送信が中断されました")
                offset += count
            return offset

    def resize_pty(self, *, width: int, height: int) -> None:
        with self._write_lock:
            if not self._closed:
                self.channel.resize_pty(width=width, height=height)

    def interrupt(self) -> None:
        self.send(b"\x03")

    def close(self) -> None:
        with self._write_lock:
            if not self._closed:
                self._closed = True
                self.channel.close()
