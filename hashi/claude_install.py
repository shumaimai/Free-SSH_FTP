"""公式インストーラーを変更せず、利用者のWindowsへ導入する。"""
import os
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

INSTALL_URL = "https://claude.ai/install.ps1"


class OfficialRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlsplit(newurl)
        if parsed.scheme != "https" or parsed.hostname not in {"claude.ai", "code.claude.com", "downloads.claude.ai"}:
            raise RuntimeError("公式インストーラーの接続先を確認できません")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def install_cli(cancel):
    if sys.platform != "win32":
        raise RuntimeError("公式CLIの自動導入はWindowsで利用できます")
    from .claude_cli import find_cli, probe_cli
    opener = urllib.request.build_opener(OfficialRedirect())
    if cancel.is_set():
        raise InterruptedError("導入を停止しました")
    with opener.open(INSTALL_URL, timeout=20) as response:
        script = response.read(1048577)
    if not script or len(script) > 1048576:
        raise RuntimeError("公式インストーラーを取得できませんでした")
    powershell = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
    with tempfile.TemporaryDirectory(prefix="hashi-claude-install-") as directory:
        script_path, log_path = Path(directory) / "install.ps1", Path(directory) / "install.log"
        script_path.write_bytes(script)
        with log_path.open("wb") as output:
            process = subprocess.Popen(
                [str(powershell), "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                 "-File", str(script_path), "stable"],
                stdout=output, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            deadline = time.monotonic() + 300
            try:
                while process.poll() is None:
                    if cancel.wait(.1):
                        raise InterruptedError("導入を停止しました")
                    if time.monotonic() >= deadline:
                        raise RuntimeError("公式CLIの導入が時間内に終わりませんでした。接続を確認して再試行してください。")
                if process.returncode:
                    raise RuntimeError("公式インストーラーが失敗しました。ネットワークと書込み権限を確認してください。")
            finally:
                if process.poll() is None:
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                   capture_output=True, timeout=5, check=False,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
                    process.wait(timeout=5)
    if cancel.is_set():
        raise InterruptedError("導入を停止しました")
    path = find_cli()
    if not path:
        raise RuntimeError("導入後のclaude.exeが見つかりません。公式の導入案内を確認してください。")
    return probe_cli(path, cancel)
