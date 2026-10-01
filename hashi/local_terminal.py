"""Windows ConPTY端末。Windows以外では依存を読み込まない。"""
from __future__ import annotations

import codecs
import logging
import os
import subprocess
import sys
import threading
from pathlib import Path

from PySide6.QtCore import QThread, Signal
from PySide6.QtWidgets import QVBoxLayout, QWidget

from . import style
from .terminal import TerminalWidget
from .terminal_binding import TerminalBinding

logger = logging.getLogger(__name__)
_workers = set()


class ConPtyBackend:
    is_terminal_backend = True

    def __init__(self, process):
        self.process = process
        self._closed = False
        self._lock = threading.RLock()
        self._decoder = codecs.getincrementaldecoder("utf-8")("strict")

    @classmethod
    def spawn(cls, *, argv=None, cwd=None):
        if sys.platform != "win32":
            raise RuntimeError("ローカルCMDはWindows 10 1809以降で利用できます")
        from winpty import Backend, PtyProcess
        command = argv or [os.path.join(os.environ.get("SystemRoot", r"C:\Windows"),
                                       "System32", "cmd.exe"), "/D", "/Q"]
        path = str(Path(cwd or Path.home()).resolve())
        if not Path(path).is_dir():
            raise ValueError("開始フォルダが見つかりません")
        return cls(PtyProcess.spawn(command, cwd=path, dimensions=(24, 80),
                                   backend=Backend.ConPTY))

    def recv(self, size):
        if self._closed:
            return b""
        try:
            return self.process.read(size).encode("utf-8")
        except EOFError:
            return b""

    def send(self, data):
        with self._lock:
            if self._closed:
                raise EOFError("端末は閉じています")
            text = self._decoder.decode(data)
            if text:
                self.process.write(text)
            return len(data)

    def resize_pty(self, *, width, height):
        with self._lock:
            if not self._closed:
                self.process.setwinsize(max(1, height), max(1, width))

    def interrupt(self):
        self.send(b"\x03")

    def close(self):
        with self._lock:
            if self._closed:
                return
            self._closed = True
        # この端末が生成したPIDだけを対象にする。名前による一括終了はしない。
        try:
            if self.process.isalive() and sys.platform == "win32":
                subprocess.run(["taskkill", "/PID", str(self.process.pid), "/T", "/F"],
                               timeout=3, check=False, capture_output=True,
                               creationflags=subprocess.CREATE_NO_WINDOW)
            self.process.close(force=True)
        except (OSError, RuntimeError, subprocess.TimeoutExpired):
            logger.warning("ローカル端末の終了に失敗", exc_info=True)
        finally:
            # 自然終了でclosed=Trueになった場合もpywinptyの読取ハンドルを解放する。
            for name in ("fileobj", "_server"):
                handle = getattr(self.process, name, None)
                if handle is not None:
                    try:
                        handle.close()
                    except OSError:
                        logger.debug("ConPTY読取ハンドルは終了済み", exc_info=True)


class _StartWorker(QThread):
    ready = Signal(object)
    failed = Signal(str)

    def __init__(self, argv, cwd):
        super().__init__()
        self.argv, self.cwd = argv, cwd
        self.cancelled = threading.Event()

    def run(self):
        try:
            backend = ConPtyBackend.spawn(argv=self.argv, cwd=self.cwd)
            if self.cancelled.is_set():
                backend.close()
            else:
                self.ready.emit(backend)
        except Exception as exc:
            logger.warning("ローカル端末を開始できません", exc_info=True)
            self.failed.emit(str(exc))


class LocalTerminalPane(QWidget):
    """単独タブとWペインが共有するローカル端末。"""
    ready = Signal()
    failed = Signal(str)

    def __init__(self, settings, registry, parent=None, *, argv=None, cwd=None,
                 label="ローカルCMD", kind="local", shareable=True):
        super().__init__(parent)
        self._closed = False
        self.binding = None
        self.registry = registry
        self.label, self.kind, self.shareable = label, kind, shareable
        self.cwd = str(Path(cwd or Path.home()).resolve())
        self.terminal = TerminalWidget(
            font_size=settings.get("terminal_font_size"),
            right_click_paste=settings.get("right_click_paste"),
            theme=settings.get("terminal_theme") or "",
            font_family=settings.get("terminal_font_family") or "")
        self.status = style.plain_label(f"{label}を起動しています…")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.status)
        layout.addWidget(self.terminal, 1)
        self.terminal.session_closed.connect(lambda: self.status.setText(f"{label}は終了しました"))
        self.worker = _StartWorker(argv, self.cwd)
        _workers.add(self.worker)
        self.worker.finished.connect(lambda w=self.worker: _workers.discard(w))
        self.worker.ready.connect(self._attach)
        self.worker.failed.connect(self._failed)
        self.worker.start()

    def _attach(self, backend):
        if self._closed:
            backend.close()
            return
        self.terminal.attach(backend)
        self.binding = TerminalBinding(self.registry, self.terminal, label=self.label,
                                       kind=self.kind, shell="cmd", cwd=None,
                                       shareable=self.shareable)
        self.status.setText(f"{self.label} · 開始フォルダ: {self.cwd}")
        self.terminal.setFocus()
        self.ready.emit()

    def _failed(self, message):
        self.status.setText(f"起動できません: {message}")
        self.failed.emit(message)

    def shutdown(self):
        self._closed = True
        self.worker.cancelled.set()
        if self.binding is not None:
            self.binding.close()
            self.binding = None
        self.terminal.detach()
        # 起動中のworkerは保持集合に残し、Qtの実行中破棄を防ぐ。
        self.worker.wait(250)


class LocalTerminalPage(LocalTerminalPane):
    def has_active_transfers(self):
        return False
