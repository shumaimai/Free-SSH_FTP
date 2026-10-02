"""ssh_core.py のユニットテスト。"""
from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from hashi.config import Profile
from hashi.ssh_core import SshSession


@pytest.mark.parametrize("sudo", [False, True])
def test_command_without_exit_status_has_total_deadline_and_closes(sudo):
    import time

    session = SshSession(Profile())
    channel = MagicMock()
    channel.recv_ready.return_value = False
    channel.recv_stderr_ready.return_value = False
    channel.exit_status_ready.return_value = False
    session.transport = MagicMock()
    session.transport.open_session.return_value = channel
    started = time.monotonic()
    with pytest.raises(TimeoutError, match="終了は不明"):
        if sudo:
            session.run_sudo("sleep 60", "test-password", timeout=.05)
        else:
            session.exec_command("sleep 60", timeout=.05)
    assert time.monotonic() - started < 1
    channel.close.assert_called()
    channel.recv_exit_status.assert_not_called()
    # Windowsのmonotonicは同じtickを返し、減算結果に丸め誤差が残ることがある。
    assert 0 < session.transport.open_session.call_args.kwargs["timeout"] <= .05 + 1e-9
    if sudo:
        channel.sendall.assert_called_once_with(b"test-password\n")


def test_receive_failure_does_not_enter_blocking_exit_status_wait():
    session = SshSession(Profile())
    channel = MagicMock()
    session.transport = MagicMock()
    session.transport.open_session.return_value = channel
    channel.recv_ready.return_value = True
    channel.recv.side_effect = OSError("receive failed")
    with pytest.raises(OSError, match="receive failed"):
        session.exec_command("test")
    channel.recv_exit_status.assert_not_called()
    channel.close.assert_called()


def test_sudo_keeps_both_output_streams_and_exit_status():
    session = SshSession(Profile())
    channel = MagicMock()
    session.transport = MagicMock()
    session.transport.open_session.return_value = channel
    channel.recv_ready.side_effect = [True, False]
    channel.recv_stderr_ready.side_effect = [True, False]
    channel.recv.return_value = "日本語".encode()
    channel.recv_stderr.return_value = b"error"
    channel.exit_status_ready.return_value = True
    channel.recv_exit_status.return_value = 7
    assert session.run_sudo("test", None) == (7, "日本語", "error")
    channel.sendall.assert_not_called()
    channel.close.assert_called()


def test_security_summary_reports_negotiated_cipher():
    """接続確立後の transport からネゴシエート済み暗号 / MAC をまとめる(#113)。"""
    session = SshSession(Profile())
    # transport 未接続なら空文字
    assert session.security_summary() == ""

    t = MagicMock()
    t.is_active.return_value = True
    t.remote_cipher = "aes256-ctr"
    t.local_cipher = "aes256-ctr"
    t.remote_mac = "hmac-sha2-256"
    t.local_mac = "hmac-sha2-256"
    session.transport = t
    assert session.security_summary() == "aes256-ctr / hmac-sha2-256"

    # GCM 系は MAC を内包するので省く
    t.remote_cipher = "aes256-gcm@openssh.com"
    t.local_cipher = "aes256-gcm@openssh.com"
    assert session.security_summary() == "aes256-gcm@openssh.com"

    # 非アクティブなら空
    t.is_active.return_value = False
    assert session.security_summary() == ""


def test_open_shell_attaches_agent_request_handler_when_enabled():
    """agent_forwarding=True のとき、シェルチャネルに AgentRequestHandler を仕掛け保持する。"""
    session = SshSession(Profile(agent_forwarding=True))
    fake_ch = MagicMock()
    fake_transport = MagicMock()
    fake_transport.open_session.return_value = fake_ch
    session.transport = fake_transport

    with patch("paramiko.agent.AgentRequestHandler") as MockHandler:
        handler = MockHandler.return_value
        ch = session.open_shell()

    assert ch is fake_ch
    MockHandler.assert_called_once_with(fake_ch)
    assert session._agent_handlers == [handler]
    assert ch._hashi_agent_handler is handler


def test_open_shell_does_not_attach_agent_handler_when_disabled():
    """agent_forwarding=False のとき、AgentRequestHandler は作られない。"""
    session = SshSession(Profile(agent_forwarding=False))
    fake_ch = MagicMock()
    fake_transport = MagicMock()
    fake_transport.open_session.return_value = fake_ch
    session.transport = fake_transport

    with patch("paramiko.agent.AgentRequestHandler") as MockHandler:
        session.open_shell()

    MockHandler.assert_not_called()
    assert session._agent_handlers == []


def test_close_closes_agent_handlers_and_transport():
    """SshSession.close は保持している AgentRequestHandler を先に閉じる。"""
    session = SshSession(Profile())
    fake_transport = MagicMock()
    session.transport = fake_transport

    handler = MagicMock()
    session._agent_handlers.append(handler)

    session.close()

    handler.close.assert_called_once()
    fake_transport.close.assert_called_once()
    assert session._agent_handlers == []
    assert session.transport is None


def test_close_is_safe_with_partially_broken_handler():
    """AgentRequestHandler の close で例外が出ても transport.close は続行する。"""
    session = SshSession(Profile())
    fake_transport = MagicMock()
    session.transport = fake_transport

    handler = MagicMock()
    handler.close.side_effect = RuntimeError("boom")
    session._agent_handlers.append(handler)

    session.close()

    handler.close.assert_called_once()
    fake_transport.close.assert_called_once()
    assert session._agent_handlers == []
