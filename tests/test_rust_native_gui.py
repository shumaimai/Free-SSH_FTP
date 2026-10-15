"""Rust GUIの実入力から保存までを、独立したXサーバーと合成ファイルで確認する。"""
import ctypes
import os
import select
import shlex
import shutil
import subprocess
import time
from pathlib import Path

import pytest


def request_window_close(display_name, window):
    """WMなしのXvfbでも、破壊やSIGTERMでなく通常の終了要求を送る。"""
    class Data(ctypes.Union):
        _fields_ = [('l', ctypes.c_long * 5)]

    class Message(ctypes.Structure):
        _fields_ = [('type', ctypes.c_int), ('serial', ctypes.c_ulong),
                    ('send_event', ctypes.c_int), ('display', ctypes.c_void_p),
                    ('window', ctypes.c_ulong), ('message_type', ctypes.c_ulong),
                    ('format', ctypes.c_int), ('data', Data)]

    class Event(ctypes.Union):
        _fields_ = [('message', Message), ('padding', ctypes.c_long * 24)]

    x11 = ctypes.CDLL('libX11.so.6')
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int,
                              ctypes.c_long, ctypes.POINTER(Event)]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = x11.XOpenDisplay(display_name.encode())
    assert display
    try:
        event = Event()
        event.message.type = 33  # ClientMessage
        event.message.display = display
        event.message.window = int(window)
        event.message.message_type = x11.XInternAtom(display, b'WM_PROTOCOLS', False)
        event.message.format = 32
        event.message.data.l[0] = x11.XInternAtom(display, b'WM_DELETE_WINDOW', False)
        assert x11.XSendEvent(display, int(window), False, 0, ctypes.byref(event))
        x11.XFlush(display)
    finally:
        x11.XCloseDisplay(display)


@pytest.mark.skipif(os.name == 'nt', reason='Windows IME/物理GUI検証とは別のLinux描画試験')
@pytest.mark.parametrize('exit_mode', ['saved', 'local_pty', 'dirty'])
def test_native_editor_ctrl_s_and_orderly_shutdown(tmp_path, exit_mode):
    binary = os.environ.get('HASHI_RUST_GUI_BINARY')
    if not binary:
        pytest.skip('ネイティブ候補のビルド付きCIで実行')
    xvfb, xdotool = shutil.which('Xvfb'), shutil.which('xdotool')
    assert xvfb and xdotool
    fixture = tmp_path / 'editor.py'
    fixture.write_bytes(b'\xef\xbb\xbfprint("hello")\r\n')
    runtime_env = os.environ.copy()
    runtime_env.pop('WAYLAND_DISPLAY', None)
    runtime_env.pop('WAYLAND_SOCKET', None)
    runtime_env.update(XDG_CONFIG_HOME=str(tmp_path / 'config'), LIBGL_ALWAYS_SOFTWARE='1')
    app = None
    with (tmp_path / 'xserver.log').open('w') as x_log, (tmp_path / 'app.log').open('w') as app_log:
        server = subprocess.Popen([xvfb, '-displayfd', '1', '-screen', '0', '1280x800x24', '-nolisten', 'tcp'],
                                  stdout=subprocess.PIPE, stderr=x_log, text=True)
        try:
            ready, _, _ = select.select([server.stdout], [], [], 10)
            assert ready, '独立Xサーバーが起動しませんでした'
            display = server.stdout.readline().strip()
            assert display.isdigit()
            runtime_env['DISPLAY'] = ':' + display
            app = subprocess.Popen([str(Path(binary).resolve()), '--edit', str(fixture)], env=runtime_env,
                                   stdout=app_log, stderr=app_log)
            window = None
            for _ in range(300):
                assert app.poll() is None, (tmp_path / 'app.log').read_text()
                found = subprocess.run([xdotool, 'search', '--onlyvisible', '--pid', str(app.pid)],
                                       env=runtime_env, capture_output=True, text=True, timeout=5)
                if found.returncode == 0:
                    window = found.stdout.splitlines()[0]
                    break
                time.sleep(.1)
            assert window, ((tmp_path / 'app.log').read_text(), (tmp_path / 'xserver.log').read_text())

            def input_event(*args):
                subprocess.run([xdotool, *args], env=runtime_env, check=True, timeout=5)

            input_event('windowfocus', '--sync', window)
            time.sleep(.3)
            input_event('mousemove', '180', '112', 'click', '1')
            time.sleep(.2)
            input_event('key', 'ctrl+a')
            time.sleep(.1)
            input_event('type', '--clearmodifiers', "print('HASHI_EDITOR_GUI')")
            input_event('key', 'Return')
            time.sleep(.1)
            input_event('key', 'ctrl+s')
            expected = b"\xef\xbb\xbfprint('HASHI_EDITOR_GUI')\r\n"
            for _ in range(100):
                if fixture.read_bytes() == expected:
                    break
                time.sleep(.05)
            assert fixture.read_bytes() == expected
            terminal_pid = None
            if exit_mode == 'local_pty':
                input_event('mousemove', '230', '12', 'click', '1')
                time.sleep(.3)
                input_event('mousemove', '600', '650', 'click', '1')
                pid_file = tmp_path / 'terminal.pid'
                input_event('type', '--clearmodifiers', f'echo $$ > {shlex.quote(str(pid_file))}; sleep 30')
                input_event('key', 'Return')
                for _ in range(100):
                    if pid_file.exists():
                        terminal_pid = int(pid_file.read_text())
                        break
                    time.sleep(.05)
                assert terminal_pid is not None, 'ローカルPTYへコマンドが届きません'
            if exit_mode == 'dirty':
                input_event('mousemove', '180', '112', 'click', '1')
                input_event('type', '--clearmodifiers', '# UNSAVED')
                time.sleep(.2)
            request_window_close(runtime_env['DISPLAY'], window)
            if exit_mode == 'dirty':
                time.sleep(.5)
                assert app.poll() is None, '未保存の編集を確認せずに終了しました'
                assert fixture.read_bytes() == expected, '未保存の編集を勝手に保存しました'
                return
            assert app.wait(timeout=10) == 0, (tmp_path / 'app.log').read_text()
            if terminal_pid:
                with pytest.raises(ProcessLookupError):
                    os.kill(terminal_pid, 0)
        finally:
            if app is not None and app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
            server.terminate()
            server.wait(timeout=10)
