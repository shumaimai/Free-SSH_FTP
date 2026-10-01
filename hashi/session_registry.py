"""端末IDと接続世代による管理。Qtや認証情報のストアには依存しない。"""
from __future__ import annotations

import threading
import uuid
from dataclasses import dataclass, field


@dataclass
class TerminalSession:
    id: str
    generation: int
    label: str
    kind: str
    shell: str
    backend: object
    ssh_session: object = None
    cwd: str | None = None
    connected: bool = True
    shareable: bool = True
    output: str = ""
    screen: str = ""
    input_lock: threading.RLock = field(default_factory=threading.RLock, repr=False)

    def public(self) -> dict:
        return {"session_id": self.id, "generation": self.generation,
                "label": self.label, "kind": self.kind, "shell": self.shell,
                "cwd": self.cwd, "connected": self.connected}


class SessionRegistry:
    """再接続前の要求は、接続世代の不一致で拒否する。"""

    OUTPUT_LIMIT = 64 * 1024

    def __init__(self):
        self._lock = threading.RLock()
        self._sessions: dict[str, TerminalSession] = {}
        self._generations: dict[str, int] = {}

    def register(self, backend, *, label: str, kind: str, shell: str,
                 session_id: str | None = None, ssh_session=None,
                 cwd: str | None = None, shareable: bool = True) -> TerminalSession:
        if kind not in {"local", "ssh"}:
            raise ValueError("不明な端末種別です")
        with self._lock:
            sid = session_id or uuid.uuid4().hex
            generation = self._generations.get(sid, 0) + 1
            self._generations[sid] = generation
            session = TerminalSession(sid, generation, label, kind, shell, backend,
                                      ssh_session=ssh_session, cwd=cwd, shareable=shareable)
            self._sessions[sid] = session
            return session

    def resolve(self, session_id: str, generation: int) -> TerminalSession:
        with self._lock:
            session = self._sessions.get(session_id)
            if session is None or not session.connected:
                raise ValueError("端末は閉じているか、接続が切れています")
            if session.generation != generation:
                raise ValueError("再接続前の端末への要求は実行できません")
            return session

    def disconnect(self, session_id: str, generation: int) -> None:
        with self._lock:
            session = self._sessions.get(session_id)
            if session and session.generation == generation:
                session.connected = False

    def remove(self, session_id: str) -> None:
        with self._lock:
            self._sessions.pop(session_id, None)
            # 世代は残す。同じIDを再登録しても古い要求を受け付けない。

    def list(self, *, shareable_only: bool = False) -> list[dict]:
        with self._lock:
            return [s.public() for s in self._sessions.values()
                    if s.connected and (s.shareable or not shareable_only)]

    def update_output(self, session_id: str, generation: int,
                      text: str, screen: str) -> None:
        with self._lock:
            session = self._sessions.get(session_id)
            if session and session.generation == generation and session.connected:
                session.output = (session.output + text)[-self.OUTPUT_LIMIT:]
                session.screen = screen[-self.OUTPUT_LIMIT:]

    def read_output(self, session_id: str, generation: int, limit: int = 16000) -> dict:
        with self._lock:
            session = self.resolve(session_id, generation)
            limit = max(1, min(int(limit), self.OUTPUT_LIMIT))
            return {**session.public(), "output": session.output[-limit:],
                    "screen": session.screen[-limit:], "completion": "unknown"}
