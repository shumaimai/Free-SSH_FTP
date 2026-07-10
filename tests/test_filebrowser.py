"""filebrowser.py の外部アプリ変更監視テスト。"""
import os


def test_external_file_monitor_emits_only_for_new_content(qapp, tmp_path):
    from hashi.filebrowser import ExternalFileMonitor

    local = tmp_path / "sample.bin"
    local.write_bytes(b"before")
    monitor = ExternalFileMonitor()
    changes = []
    monitor.changed.connect(lambda remote, path: changes.append((remote, path)))
    monitor.watch("/srv/sample.bin", str(local))

    monitor._emit_if_changed(os.path.abspath(local))
    assert changes == []

    local.write_bytes(b"after")
    scheduled = []
    monitor._schedule_check = lambda path: scheduled.append(path)
    monitor._poll_files()
    assert scheduled == [os.path.abspath(local)]

    monitor._emit_if_changed(os.path.abspath(local))
    assert changes == [("/srv/sample.bin", os.path.abspath(local))]

    monitor._emit_if_changed(os.path.abspath(local))
    assert len(changes) == 1
    monitor.stop()


def test_external_save_uses_permission_override(qapp, tmp_path):
    from hashi.filebrowser import SftpWorker

    local = tmp_path / "sample.bin"
    local.write_bytes(b"changed")
    remote = "/srv/sample.bin"

    class FakeSftp:
        def __init__(self):
            self.calls = 0

        def put(self, source, target, callback=None):
            self.calls += 1
            if self.calls == 1:
                raise PermissionError("Permission denied")
            assert source == str(local)
            assert target == remote

    class FakePermManager:
        def __init__(self):
            self.paths = []

        def with_write_access(self, path, op):
            self.paths.append(path)
            try:
                return op()
            except PermissionError:
                return op()

    worker = SftpWorker(object(), "test")
    worker.sftp = FakeSftp()
    worker.pm = FakePermManager()
    worker.perm_override = True
    results = []
    worker.external_save_result.connect(
        lambda *args: results.append(args))

    worker._job_external_save({
        "remote": remote,
        "local": str(local),
    })

    assert worker.pm.paths == [remote]
    assert worker.sftp.calls == 2
    assert results == [(remote, str(local), True, "")]
