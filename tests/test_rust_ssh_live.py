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


@pytest.mark.parametrize("mode", ["direct", "alias", "two_hops", "agent_forward", "aes128-cbc", "aes192-cbc", "aes256-cbc", "3des-cbc", "umask027"])
@pytest.mark.skipif(os.name == "nt", reason="この管理下OpenSSH fixtureはLinux用")
def test_rust_auth_exec_sftp_and_resume_with_openssh(tmp_path, mode):
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
    if mode.endswith('-cbc'):
        with config.open('a') as output:
            output.write(f'Ciphers {mode}\nMACs hmac-sha2-256\n')
    process = subprocess.Popen([daemon, "-D", "-e", "-f", str(config)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                               umask=0o027 if mode == 'umask027' else 0o022)
    agent = None
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
        if mode in ("alias", "two_hops"):
            client_config = tmp_path / 'ssh-config'
            client_config.write_text(f"Host target hop\nHostName 127.0.0.1\nUser {getpass.getuser()}\nPort {port}\nIdentityFile {tmp_path / 'identity'}\n")
            profile.update(host='target', port=22, username='', key_path='')
            if mode == "two_hops":
                profile['proxy_jump'] = 'hop,hop'
            request['ssh_config'] = str(client_config)
        runtime_env = os.environ.copy()
        if mode.endswith('-cbc'):
            import paramiko
            with paramiko.Transport(('127.0.0.1', port)) as baseline:
                baseline.get_security_options().ciphers = (mode,)
                baseline.connect(username=getpass.getuser(), pkey=paramiko.Ed25519Key.from_private_key_file(str(tmp_path / 'identity')))
                assert baseline.remote_cipher == mode
                assert baseline.get_remote_server_key().asbytes() == base64.b64decode(pub[1])
        if mode == "agent_forward":
            socket_path = tmp_path / 'agent.sock'
            agent = subprocess.Popen(['ssh-agent', '-D', '-a', str(socket_path)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(100):
                if socket_path.exists():
                    break
                time.sleep(.02)
            runtime_env['SSH_AUTH_SOCK'] = str(socket_path)
            subprocess.run(['ssh-add', str(tmp_path / 'identity')], env=runtime_env, check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
            profile['agent_forwarding'] = True
            request['expected_agent_key'] = (tmp_path / 'identity.pub').read_text().split()[1]
        result = subprocess.run([str(Path(binary).resolve())], input=json.dumps(request), env=runtime_env,
                                capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == "ssh-sftp-verified"
    finally:
        if agent is not None:
            agent.terminate()
            agent.wait(timeout=10)
        process.terminate()
        process.wait(timeout=10)
