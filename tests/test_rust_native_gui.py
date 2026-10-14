"""Rust GUIの実入力から保存までを、独立したXサーバーと合成ファイルで確認する。"""
import os
import select
import shutil
import subprocess
import time
from pathlib import Path

import pytest


@pytest.mark.skipif(os.name == 'nt', reason='Windows IME/物理GUI検証とは別のLinux描画試験')
def test_native_editor_ctrl_s_preserves_bom_and_crlf(tmp_path):
    binary = os.environ.get('HASHI_RUST_GUI_BINARY')
    if not binary:
        pytest.skip('ネイティブ候補のビルド付きCIで実行')
    xvfb, xdotool = shutil.which('Xvfb'), shutil.which('xdotool')
    assert xvfb and xdotool
    fixture = tmp_path / 'editor.py'
    fixture.write_bytes(b'\xef\xbb\xbfprint("hello")\r\n')
    runtime_env = os.environ.copy()
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
        finally:
            if app is not None:
                app.terminate()
                app.wait(timeout=10)
            server.terminate()
            server.wait(timeout=10)
