"""管理下のloopback OpenSSHへ合成鍵で接続し、Rustの転送内容を検証する。"""
import base64
import getpass
import hashlib
import json
import os
import shutil
import socket
import subprocess
import time
from pathlib import Path

import pytest


@pytest.mark.skipif(os.name == "nt", reason="この管理下OpenSSH fixtureはLinux用")
def test_rust_auth_exec_sftp_and_resume_with_openssh(tmp_path):
    binary = os.environ.get("HASHI_RUST_SSH_BINARY")
    if not binary:
        pytest.skip("ビルド付きのSSH結合CIで実行")
    daemon = shutil.which("sshd") or "/usr/sbin/sshd"
    assert Path(daemon).is_file()
    keygen = shutil.which("ssh-keygen")
    assert keygen
    for name in ["host", "identity"]:
        subprocess.run([keygen, "-q", "-t", "ed25519", "-N", "", "-f", str(tmp_path / name)],
                       check=True, timeout=15)
    pub = (tmp_path / "host.pub").read_text().split()
    fingerprint = "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(pub[1])).digest()).decode().rstrip("=")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    config = tmp_path / "sshd.conf"
    config.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {tmp_path / 'host'}
AuthorizedKeysFile {tmp_path / 'identity.pub'}
PidFile {tmp_path / 'sshd.pid'}
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
StrictModes no
Subsystem sftp internal-sftp
LogLevel ERROR
''')
    process = subprocess.Popen([daemon, "-D", "-e", "-f", str(config)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    try:
        for _ in range(100):
            if process.poll() is not None:
                pytest.fail(process.stderr.read())
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=.1):
                    break
            except OSError:
                time.sleep(.05)
        else:
            pytest.fail("管理下のsshdが起動しませんでした")
        profile = {"host": "127.0.0.1", "port": port, "username": getpass.getuser(),
                   "auth_method": "key", "key_path": str(tmp_path / "identity")}
        request = {"profile": profile, "expected_key": {"key_type": pub[0], "fingerprint": fingerprint},
                   "directory": str(tmp_path)}
        result = subprocess.run([str(Path(binary).resolve())], input=json.dumps(request),
                                capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == "ssh-sftp-verified"
    finally:
        process.terminate()
        process.wait(timeout=10)
