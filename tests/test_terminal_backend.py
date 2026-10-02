"""部分送信と再接続後の古い通知を回帰検証する。"""
from hashi.session_registry import SessionRegistry
from hashi.terminal_backend import SshTerminalBackend


def test_adapter_completes_partial_send_and_closes_once():
    class Channel:
        sent = b""
        closes = 0

        def send(self, data):
            self.sent += data[:2]
            return min(2, len(data))

        def close(self):
            self.closes += 1

    channel = Channel()
    backend = SshTerminalBackend(channel)
    assert backend.send("日本語".encode()) == 9
    assert channel.sent == "日本語".encode()
    backend.close()
    backend.close()
    assert channel.closes == 1


def test_old_connection_cannot_disconnect_or_append_to_new_one():
    import pytest

    registry = SessionRegistry()
    old = registry.register(object(), label="SSH", kind="ssh", shell="posix")
    new = registry.register(object(), label="SSH", kind="ssh", shell="posix",
                            session_id=old.id)
    registry.disconnect(old.id, old.generation)
    registry.update_output(old.id, old.generation, "old", "old")
    assert registry.resolve(new.id, new.generation).connected
    assert registry.read_output(new.id, new.generation)["output"] == ""
    with pytest.raises(ValueError, match="再接続前"):
        registry.resolve(old.id, old.generation)
    registry.remove(new.id)
    again = registry.register(object(), label="CMD", kind="local", shell="cmd",
                              session_id=new.id)
    assert again.generation > new.generation


def test_terminal_ignores_late_output_and_close_notification(qapp):
    from hashi.terminal import TerminalWidget

    terminal = TerminalWidget()
    terminal._connection_epoch = 2
    events = []
    terminal.session_closed.connect(lambda: events.append("closed"))
    terminal._receive_data(1, b"old")
    terminal._receive_closed(1)
    assert not terminal._closed
    assert not events
    terminal._receive_data(2, b"new")
    assert "new" in terminal.screen.display[0]
    terminal._receive_closed(2)
    terminal._receive_closed(2)
    assert events == ["closed"]
    terminal.detach()
