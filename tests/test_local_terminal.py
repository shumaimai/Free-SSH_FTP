import sys
import threading

import pytest

from hashi.local_terminal import ConPtyBackend


def test_local_start_directory_survives_settings_reload(tmp_config):
    from hashi.config import Settings

    settings = Settings()
    assert settings.get("local_terminal_start_dir") == ""
    settings.set("local_terminal_start_dir", str(tmp_config / "日本語 フォルダ"))
    reloaded = Settings()
    assert reloaded.get("local_terminal_start_dir") == str(tmp_config / "日本語 フォルダ")
    assert reloaded.get("local_start_dir") == ""


class Process:
    def __init__(self):
        self.writes, self.sizes = [], []
        self.closes = 0

    def write(self, text):
        self.writes.append(text)

    def read(self, size):
        raise EOFError()

    def setwinsize(self, rows, cols):
        self.sizes.append((rows, cols))

    def isalive(self):
        return False

    def close(self, force=False):
        self.closes += 1


def test_conpty_utf8_resize_and_close():
    p = Process()
    backend = ConPtyBackend(p)
    encoded = "日本語".encode()
    backend.send(encoded[:2])
    backend.send(encoded[2:])
    backend.interrupt()
    backend.resize_pty(width=90, height=30)
    assert p.writes == ["日本語", "\x03"]
    assert p.sizes == [(30, 90)]
    assert backend.recv(100) == b""
    backend.close()
    backend.close()
    assert p.closes == 1
    with pytest.raises(EOFError):
        backend.send(b"x")


def test_close_cancels_and_joins_internal_reader_before_process_close():
    from types import SimpleNamespace

    cancelled = threading.Event()
    process = Process()
    process.pty = SimpleNamespace(cancel_io=cancelled.set)
    process._thread = threading.Thread(target=lambda: cancelled.wait(5))
    process._thread.start()
    process.close = lambda force=False: None if not process._thread.is_alive() else pytest.fail("readerは未回収")
    ConPtyBackend(process).close()
    assert cancelled.is_set() and not process._thread.is_alive()


@pytest.mark.skipif(sys.platform != "win32", reason="Windows ConPTY実機テスト")
def test_real_conpty_cmd(tmp_path):
    backend = ConPtyBackend.spawn(cwd=tmp_path)
    data = bytearray()
    done = threading.Event()

    def read():
        while True:
            chunk = backend.recv(4096)
            if not chunk:
                break
            data.extend(chunk)
            if b"HASHI_CONPTY_OK" in data:
                done.set()

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    try:
        backend.resize_pty(width=100, height=30)
        backend.send(b"echo HASHI_CONPTY_OK\r")
        assert done.wait(15), data.decode(errors="replace")
    finally:
        backend.close()
        reader.join(2)
    assert not reader.is_alive()
    assert not backend.process._thread.is_alive()


@pytest.mark.skipif(sys.platform != "win32", reason="Windows ConPTYの日本語出力とCtrl+C実検証")
def test_real_conpty_unicode_and_interrupt(tmp_path):
    # 入力のエコーに日本語/完了マーカーが現れないよう、スクリプトはASCIIで渡す。
    script = ("import time;print(chr(26085)+chr(26412)+chr(35486),flush=True);"
              "exec('try:\\n time.sleep(30)\\nexcept KeyboardInterrupt:\\n print(chr(20013)+chr(26029),flush=True)')")
    backend = ConPtyBackend.spawn(argv=[sys.executable, "-u", "-c", script], cwd=tmp_path)
    data = bytearray()
    ready, interrupted = threading.Event(), threading.Event()

    def read():
        while chunk := backend.recv(4096):
            data.extend(chunk)
            if "日本語".encode() in data:
                ready.set()
            if "中断".encode() in data:
                interrupted.set()

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    try:
        assert ready.wait(15), data.decode(errors="replace")
        backend.interrupt()
        assert interrupted.wait(15), data.decode(errors="replace")
    finally:
        backend.close()
        reader.join(2)
    assert not reader.is_alive()
    assert not backend.process._thread.is_alive()
