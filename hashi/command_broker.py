"""AI/MCPからの入力を端末ID・世代・ユーザー許可で仲介する。"""
from __future__ import annotations

import hashlib
import os
import subprocess
import sys
import threading
import time
import uuid
from collections import deque
from dataclasses import dataclass, field

from .ssh_core import command_channel


def _pipe_available(pipe):
    """同じ読取ハンドルを他スレッドに渡さず、読み取れる分だけ取得する。"""
    import ctypes
    import msvcrt
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    peek = kernel.PeekNamedPipe
    peek.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD,
                     ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(wintypes.DWORD),
                     ctypes.POINTER(wintypes.DWORD)]
    peek.restype = wintypes.BOOL
    available = wintypes.DWORD()
    if not peek(msvcrt.get_osfhandle(pipe.fileno()), None, 0, None, ctypes.byref(available), None):
        error = ctypes.get_last_error()
        if error in (109, 232):  # ERROR_BROKEN_PIPE / ERROR_NO_DATA
            return None
        raise ctypes.WinError(error)
    return available.value


class _Cancellation(threading.Event):
    def __init__(self, parent=None):
        super().__init__()
        self.parent = parent

    def is_set(self):
        return super().is_set() or (self.parent is not None and self.parent.is_set())


@dataclass
class Operation:
    id: str
    actor: str
    action: str
    session_id: str
    generation: int
    text: str = ""
    cwd: str | None = None
    cancel: threading.Event = field(default_factory=threading.Event, repr=False)


class CommandBroker:
    """許可はメモリ内だけ。モデルやMCPツールからモード変更はできない。"""

    def __init__(self, registry, approval=None):
        self.registry = registry
        self.approval = approval
        self._lock = threading.RLock()
        self._mode = "consult"
        self._targets = set()
        self._expires = 0.0
        self._epoch = 0
        self._active = {}
        self._results = {}
        self.audit = deque(maxlen=500)

    def configure(self, mode, targets, *, lifetime=900):
        if mode not in {"consult", "confirm", "auto"}:
            raise ValueError("不明な実行モード")
        verified = set()
        for sid, generation in targets:
            entry = self.registry.resolve(sid, generation)
            if not entry.shareable:
                raise ValueError("この端末は外部操作を許可していません")
            verified.add((sid, generation))
        self.stop()
        with self._lock:
            self._mode = mode
            self._targets = verified
            self._expires = time.monotonic() + min(max(lifetime, 1), 3600)

    def stop(self):
        with self._lock:
            self._epoch += 1
            self._mode = "consult"
            self._targets.clear()
            for operation in self._active.values():
                operation.cancel.set()

    def cancel_operation(self, actor, operation_id):
        with self._lock:
            operation = self._active.get((actor, operation_id))
            if operation is None:
                return {"status": "not_running", "completion": "unknown"}
            operation.cancel.set()
            return {"status": "cancel_requested", "completion": "unknown"}

    def cancel_actor(self, actor):
        with self._lock:
            for operation in self._active.values():
                if operation.actor == actor:
                    operation.cancel.set()

    def audit_snapshot(self):
        with self._lock:
            return {"active": [{"id": o.id, "actor": o.actor, "action": o.action,
                                "session_id": o.session_id, "generation": o.generation,
                                "status": "cancel_requested" if o.cancel.is_set() else "pending"}
                               for o in self._active.values()],
                    "finished": [item.copy() for item in self.audit]}

    def _check(self, sid, generation):
        entry = self.registry.resolve(sid, generation)
        if not entry.shareable:
            raise PermissionError("この端末は外部操作の対象外です")
        if (sid, generation) not in self._targets or time.monotonic() >= self._expires:
            raise PermissionError("端末の共有許可がありません。対象端末を選び直してください")
        return entry

    def list_sessions(self):
        with self._lock:
            if time.monotonic() >= self._expires:
                return []
            return [s for s in self.registry.list(shareable_only=True)
                    if (s["session_id"], s["generation"]) in self._targets]

    def permission_snapshot(self):
        with self._lock:
            return {"mode": self._mode, "seconds": max(0, int(self._expires - time.monotonic()))}

    def read_output(self, sid, generation, limit=16000):
        with self._lock:
            self._check(sid, generation)
            return self.registry.read_output(sid, generation, limit)

    def perform(self, actor, action, sid, generation, *, text="", cwd=None,
                request_id=None, timeout=30, cancel=None):
        if action not in {"send_input", "run_command", "interrupt"}:
            raise ValueError("不明な端末操作")
        if not isinstance(text, str) or len(text) > 16000 or "\x00" in text:
            raise ValueError("入力が長すぎるか不正です")
        operation = Operation(request_id or uuid.uuid4().hex, actor, action, sid,
                              generation, text, cwd, _Cancellation(cancel))
        fingerprint = hashlib.sha256(repr((action, sid, generation, text, cwd)).encode()).hexdigest()
        key = (actor, operation.id)
        with self._lock:
            entry = self._check(sid, generation)
            if self._mode == "consult":
                raise PermissionError("相談のみモードでは実行できません")
            if key in self._results:
                previous, result = self._results[key]
                if previous != fingerprint:
                    raise ValueError("同じ要求IDの内容が変わっています")
                return result.copy()
            if key in self._active:
                raise ValueError("同じ要求は実行中です")
            self._active[key] = operation
            epoch, mode = self._epoch, self._mode
        result = None
        try:
            if mode == "confirm":
                if self.approval is None or not self.approval(operation):
                    raise PermissionError("ユーザーが実行を許可しませんでした")
            if not entry.input_lock.acquire(blocking=False):
                raise RuntimeError("この端末は別の操作を実行中です")
            try:
                with self._lock:
                    self._check(sid, generation)
                    if epoch != self._epoch or operation.cancel.is_set():
                        raise PermissionError("停止または許可変更により取り消されました")
                    if action == "send_input":
                        entry.backend.send(text.replace("\n", "\r").encode("utf-8"))
                        result = {"status": "sent", "completion": "unknown", "exit_code": None}
                    elif action == "interrupt":
                        entry.backend.interrupt()
                        result = {"status": "interrupt_sent", "completion": "unknown"}
                    else:
                        result = None
                if action == "run_command":
                    result = self._run(entry, operation, min(max(timeout, 1), 120))
            finally:
                entry.input_lock.release()
            with self._lock:
                self._results[key] = (fingerprint, result)
                if len(self._results) > 500:
                    self._results.pop(next(iter(self._results)))
            return result
        finally:
            with self._lock:
                self._active.pop(key, None)
                # コマンド本文・出力・認証情報を監査ログへ保存しない。
                self.audit.append({"id": operation.id, "actor": actor, "action": action,
                                   "session_id": sid, "generation": generation,
                                   "cancelled": operation.cancel.is_set(),
                                   "status": result["status"] if result is not None else "not_completed",
                                   "completion": result.get("completion", "unknown") if result is not None else "unknown",
                                   "exit_code": result.get("exit_code") if result is not None else None})

    def _run(self, entry, operation, timeout):
        deadline = time.monotonic() + timeout
        output, error = bytearray(), bytearray()
        if entry.kind == "ssh":
            if entry.ssh_session is None:
                raise ValueError("独立コマンド実行に対応しないSSH端末です")
            try:
                with command_channel(entry.ssh_session.transport, deadline=deadline,
                                     cancel=operation.cancel) as channel:
                    channel.exec_command(operation.text)
                    while True:
                        if operation.cancel.is_set() or time.monotonic() >= deadline:
                            return {"status": "cancelled" if operation.cancel.is_set() else "timeout",
                                    "completion": "unknown", "exit_code": None,
                                    "output": output.decode(errors="replace"),
                                    "error": error.decode(errors="replace")}
                        if channel.recv_ready():
                            output.extend(channel.recv(4096))
                            output[:] = output[-65536:]
                        if channel.recv_stderr_ready():
                            error.extend(channel.recv_stderr(4096))
                            error[:] = error[-65536:]
                        if channel.exit_status_ready() and not channel.recv_ready() and not channel.recv_stderr_ready():
                            code = channel.recv_exit_status()
                            return {"status": "completed" if code != -1 else "closed",
                                    "completion": "known" if code != -1 else "unknown",
                                    "exit_code": code if code != -1 else None,
                                    "output": output.decode(errors="replace"),
                                    "error": error.decode(errors="replace")}
                        operation.cancel.wait(.02)
            except (TimeoutError, InterruptedError):
                return {"status": "cancelled" if operation.cancel.is_set() else "timeout",
                        "completion": "unknown", "exit_code": None,
                        "output": output.decode(errors="replace"),
                        "error": error.decode(errors="replace")}
        if sys.platform != "win32":
            raise RuntimeError("ローカルコマンド実行はWindows専用です")
        if not operation.cwd or not os.path.isdir(operation.cwd):
            raise ValueError("独立実行には明示した開始フォルダが必要です。対話CMDのcwdは引き継ぎません")
        # 独立プロセスなので対話CMDの環境変更やTUI状態に影響しない。
        executable = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "cmd.exe")
        # 外側はASCIIだけで起動し、chcp後の遅延展開で内側へUnicodeを渡す。
        # 入力中の%/!を外側で再展開しない。内側の/Sは最外の引用符だけを除く。
        variable = "HASHI_COMMAND_" + uuid.uuid4().hex.upper()
        environment = os.environ.copy()
        environment[variable] = operation.text
        quoted_exe = subprocess.list2cmdline([executable])
        command_line = (quoted_exe + ' /D /V:ON /S /C "chcp 65001>nul & ' + quoted_exe +
                        ' /D /V:OFF /S /C "!' + variable + '!""')
        process = subprocess.Popen(
            command_line,
            cwd=operation.cwd, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW)
        chunks = {"output": bytearray(), "error": bytearray()}
        pipes = ((process.stdout, "output"), (process.stderr, "error"))
        status = "completed"
        complete_output = False
        try:
            while True:
                got_data, ended = False, 0
                for pipe, name in pipes:
                    count = _pipe_available(pipe)
                    if count is None:
                        ended += 1
                    elif count:
                        data = os.read(pipe.fileno(), min(count, 4096))
                        chunks[name].extend(data)
                        chunks[name][:] = chunks[name][-65536:]
                        got_data = got_data or bool(data)
                if process.poll() is not None and not got_data:
                    complete_output = ended == len(pipes)
                    break
                if operation.cancel.is_set() or time.monotonic() >= deadline:
                    status = "cancelled" if operation.cancel.is_set() else "timeout"
                    if process.poll() is None:
                        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                       capture_output=True, timeout=3, check=False,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
                        process.wait(timeout=3)
                    break
                if not got_data:
                    operation.cancel.wait(.02)
            return {"status": status, "completion": "known" if status == "completed" and complete_output else "unknown",
                    "exit_code": process.returncode if status == "completed" else None,
                    "output_complete": complete_output,
                    **{name: bytes(data).decode("utf-8", errors="replace") for name, data in chunks.items()}}
        finally:
            for pipe in (process.stdout, process.stderr):
                pipe.close()
