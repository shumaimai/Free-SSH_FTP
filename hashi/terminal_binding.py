"""GUIスレッド上で端末の表示を管理台帳へ写す。"""
from __future__ import annotations

import codecs
import re

_ANSI = re.compile(r"\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]")


class TerminalBinding:
    def __init__(self, registry, terminal, *, label, kind, shell,
                 ssh_session=None, cwd=None, shareable=True):
        self.registry = registry
        self.terminal = terminal
        self.metadata = {"label": label, "kind": kind, "shell": shell,
                         "cwd": cwd, "shareable": shareable}
        self.entry = None
        self.replace(ssh_session=ssh_session)
        terminal.output_received.connect(self._capture)
        terminal.session_closed.connect(self._disconnected)

    def replace(self, *, ssh_session=None):
        sid = self.entry.id if self.entry else None
        self.entry = self.registry.register(
            self.terminal._channel, session_id=sid, ssh_session=ssh_session,
            **self.metadata)
        self._decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def _capture(self, data):
        text = _ANSI.sub("", self._decoder.decode(data))
        screen = "\n".join(self.terminal.screen.display)
        self.registry.update_output(self.entry.id, self.entry.generation, text, screen)

    def _disconnected(self):
        self.registry.disconnect(self.entry.id, self.entry.generation)

    def close(self):
        self._disconnected()
        self.registry.remove(self.entry.id)
        self.terminal.output_received.disconnect(self._capture)
        self.terminal.session_closed.disconnect(self._disconnected)
