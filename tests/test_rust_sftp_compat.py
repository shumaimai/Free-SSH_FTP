"""fsyncなし・chown不可の管理下SFTPへ実接続して互換モードを確認する。"""
import base64
import errno
import hashlib
import json
import os
import socket
import subprocess
import threading
import time
from pathlib import Path

import paramiko
import pytest


@pytest.mark.skipif(os.name == 'nt', reason='Unixのmode/inode/書込み不可directory試験')
def test_direct_transfer_without_fsync_or_directory_write(tmp_path):
    binary = os.environ.get('HASHI_RUST_SSH_BINARY')
    if not binary:
        pytest.skip('ビルド付きのSFTP互換CIで実行')
    host, identity = paramiko.RSAKey.generate(2048), paramiko.RSAKey.generate(2048)
    identity_path = tmp_path / 'identity.pem'
    identity.write_private_key_file(str(identity_path))
    parent = tmp_path / 'read-only'
    parent.mkdir()
    target = parent / 'existing'
    target.write_bytes(b'original')
    (tmp_path / 'slow-source').write_bytes(b'slow-source')
    for mode in ('atomic', 'direct'):
        (parent / f'race-{mode}').write_bytes(b'race-original')
    parent.chmod(0o500)
    mutations = []
    observed = set()

    class Server(paramiko.ServerInterface):
        def check_auth_publickey(self, username, key):
            return paramiko.AUTH_SUCCESSFUL if username == 'fixture' and key == identity else paramiko.AUTH_FAILED

        def get_allowed_auths(self, _username):
            return 'publickey'

        def check_channel_request(self, kind, _channel):
            return paramiko.OPEN_SUCCEEDED if kind == 'session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def attributes(stat):
        result = paramiko.SFTPAttributes.from_stat(stat)
        # 他所有者のmetadataを報告し、所有者変更を受け付けないサーバー契約。
        result.st_uid = os.getuid() + 1
        return result

    class Handle(paramiko.SFTPHandle):
        def stat(self):
            return attributes(os.fstat((getattr(self, 'readfile', None) or self.writefile).fileno()))

        def chattr(self, attr):
            if attr.st_uid is not None or attr.st_gid is not None or attr.st_mode is not None:
                mutations.append('attributes')
                return paramiko.SFTP_PERMISSION_DENIED
            if attr.st_size is not None:
                os.ftruncate(self.writefile.fileno(), attr.st_size)
            return paramiko.SFTP_OK

    class Files(paramiko.SFTPServerInterface):
        def resolve(self, path):
            resolved = Path(path).resolve()
            if not resolved.is_relative_to(tmp_path):
                raise OSError(errno.EACCES, 'fixture root')
            return resolved

        def stat(self, path):
            try:
                if Path(path).name.startswith('race-') and path not in observed:
                    observed.add(path)
                    return paramiko.SFTP_NO_SUCH_FILE
                return attributes(self.resolve(path).stat())
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        lstat = stat

        def open(self, path, flags, attr):
            try:
                if Path(path).name == 'slow-source':
                    time.sleep(.1)
                descriptor = os.open(self.resolve(path), flags, (attr.st_mode or 0o666) & 0o777)
                handle = Handle(flags)
                if flags & os.O_WRONLY:
                    handle.writefile = os.fdopen(descriptor, 'wb', buffering=0)
                else:
                    handle.readfile = os.fdopen(descriptor, 'rb', buffering=0)
                return handle
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        def chattr(self, _path, _attr):
            mutations.append('path-attributes')
            return paramiko.SFTP_PERMISSION_DENIED

    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    listener.listen(1)
    listener.settimeout(10)
    port = listener.getsockname()[1]
    transports = []
    failures = []
    stopped = threading.Event()

    def serve():
        try:
            client, _ = listener.accept()
            transport = paramiko.Transport(client)
            transports.append(transport)
            transport.add_server_key(host)
            transport.set_subsystem_handler('sftp', paramiko.SFTPServer, Files)
            transport.start_server(server=Server())
            while not stopped.wait(.05) and transport.is_active():
                pass
        except Exception as error:
            failures.append(repr(error))

    worker = threading.Thread(target=serve, daemon=True)
    worker.start()
    try:
        request = {'profile': {'host': '127.0.0.1', 'port': port, 'username': 'fixture',
                               'auth_method': 'key', 'key_path': str(identity_path)},
                   'expected_key': {'key_type': host.get_name(), 'fingerprint': 'SHA256:' + base64.b64encode(hashlib.sha256(host.asbytes()).digest()).decode().rstrip('=')},
                   'directory': str(tmp_path), 'compat_only': True}
        result = subprocess.run([str(Path(binary).resolve())], input=json.dumps(request),
                                capture_output=True, text=True, timeout=40)
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == 'ssh-sftp-compat-verified'
        assert not mutations, '直接書込みで所有者・親modeを変更しようとした'
        assert not failures
        assert target.exists()
    finally:
        stopped.set()
        for transport in transports:
            transport.close()
        listener.close()
        worker.join(timeout=5)
        parent.chmod(0o700)
