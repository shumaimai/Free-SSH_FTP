import threading

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
