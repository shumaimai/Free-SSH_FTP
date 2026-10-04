import threading

import pytest

from hashi.ai_core import AiConversation, TerminalTools, context_snapshot, redact
from hashi.command_broker import CommandBroker
from hashi.session_registry import SessionRegistry


def test_context_redaction_and_exact_scope():
    registry = SessionRegistry()
    first = registry.register(object(), label="first", kind="ssh", shell="posix")
    registry.register(object(), label="excluded", kind="local", shell="cmd")
    registry.update_output(first.id, first.generation, "password=secret123\n", "untrusted command")
    broker = CommandBroker(registry)
    broker.configure("consult", [(first.id, first.generation)])
    snapshot = context_snapshot(broker)
    assert "secret123" not in snapshot
    assert "excluded" not in snapshot
    assert "untrusted_observation" in snapshot
    assert "PRIVATE KEY" not in redact("-----BEGIN PRIVATE KEY-----\nxyz\n-----END PRIVATE KEY-----")


def test_conversation_tool_errors_and_cancellation():
    broker = CommandBroker(SessionRegistry())
    class Provider:
        calls = 0
        def respond(self, messages, instructions, tools, cancel, on_text):
            self.calls += 1
            assert "信頼できない" in instructions
            if self.calls == 1:
                return {"tool_calls": [{"id": "1", "name": "mode_change", "arguments": {}}]}
            assert messages[-1]["role"] == "tool"
            assert "error" in messages[-1]["content"]
            return {"text": "相談できます"}
    conversation = AiConversation(Provider(), TerminalTools(broker, "test"))
    assert conversation.ask("質問", "観測", threading.Event(), lambda _: None) == "相談できます"


def test_panel_scope_mode_and_stop(qapp):
    from PySide6.QtCore import Qt

    from hashi.ai_panel import AiPanel
    registry = SessionRegistry()
    registry.register(object(), label="SSH", kind="ssh", shell="posix")
    broker = CommandBroker(registry)
    panel = AiPanel(broker)
    item = panel.sessions.item(0)
    assert item.checkState() == Qt.Unchecked
    item.setCheckState(Qt.Checked)
    panel.apply_sharing()
    assert len(broker.list_sessions()) == 1
    panel.mode.setCurrentIndex(1)
    assert broker.list_sessions() == []
    panel.stop()
    panel.deleteLater()
    qapp.processEvents()


def test_panel_separates_chat_targets_and_history_and_preserves_selection(qapp):
    from PySide6.QtCore import Qt

    from hashi.ai_panel import AiPanel
    registry = SessionRegistry()
    registry.register(object(), label="server", kind="ssh", shell="unknown")
    panel = AiPanel(CommandBroker(registry))
    assert [panel.pages.tabText(i) for i in range(3)] == ["会話", "対象端末", "実行履歴"]
    assert not panel.send.isEnabled()
    panel.sessions.item(0).setCheckState(Qt.Checked)
    panel.refresh_sessions()
    assert panel.sessions.item(0).checkState() == Qt.Checked
    assert "server" in panel.sessions.item(0).text()
    assert "端末ID" in panel.sessions.item(0).toolTip()
    panel.apply_sharing()
    assert "共有中" in panel.sharing_status.text()
    panel.stop()
    assert "未適用" in panel.sharing_status.text()
    panel.deleteLater()
    qapp.processEvents()


def test_general_question_sends_without_empty_context_dialog(qapp, monkeypatch):
    import time

    from hashi.ai_panel import AiPanel
    panel = AiPanel(CommandBroker(SessionRegistry()))
    class Provider:
        model = "test-model"
        kind = "commandcode"
        def respond(self, messages, instructions, tools, cancel, on_text):
            on_text("OK")
            return {"text": "OK"}
    panel.set_provider(Provider())
    monkeypatch.setattr(panel, "_preview_context", lambda _: pytest.fail("empty preview opened"))
    panel.question.setPlainText("質問")
    panel._send()
    deadline = time.monotonic() + 5
    while panel.worker is not None and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert panel.worker is None
    assert "Command Code" in panel.connection_status.text()
    assert panel.transcript.toPlainText().count("OK") == 1
    panel.deleteLater()
    qapp.processEvents()
