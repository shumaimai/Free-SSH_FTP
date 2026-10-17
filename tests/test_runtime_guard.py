"""旧版の複数起動・強制終了と、Rust移行ガードのOSロック互換。"""
import errno
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from hashi.runtime_guard import MigrationBusyError, MigrationGuard, PythonRuntime, process_start

ROOT = Path(__file__).resolve().parents[1]
WORKER = """
import sys
from pathlib import Path
from hashi.runtime_guard import PythonRuntime
with PythonRuntime(Path(sys.argv[1]), data_root=Path(sys.argv[2])):
    print('ready', flush=True)
    sys.stdin.readline()
"""


def python_worker(directory, data_root):
    process = subprocess.Popen([sys.executable, "-u", "-c", WORKER, str(directory),
                                str(data_root)], cwd=ROOT, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert process.stdout.readline().strip() == "ready"
    return process


def finish(process):
    if process.poll() is None:
        process.communicate("\n", timeout=15)
    assert process.returncode == 0


def test_marker_is_per_instance_and_contains_no_credentials(tmp_path):
    directory = tmp_path / "runtime"
    root = tmp_path / "data"
    with PythonRuntime(directory, data_root=root) as first:
        with PythonRuntime(directory, data_root=root) as second:
            assert first.marker != second.marker
            data = json.loads(first.marker.read_text())
            assert data["implementation"] == "python"
            assert data["process_start"] == process_start()
            assert data["pid"] == os.getpid()
            assert data["data_root"] == str(root.resolve())
            assert set(data) == {"schema_version", "implementation", "application_version",
                                 "pid", "process_start", "instance_id", "executable", "data_root"}
            with pytest.raises(OSError):
                with MigrationGuard(directory, exclusive=True):
                    pytest.fail("旧版の実行中に排他取得できてしまった")
    assert not list(directory.glob("python-*.json"))
    assert (directory / "migration.guard").is_file()


def test_multiple_processes_and_crash_leave_marker_but_release_lock(tmp_path):
    directory = tmp_path / "runtime"
    first = python_worker(directory, tmp_path / "data")
    second = python_worker(directory, tmp_path / "data")
    try:
        first.kill()
        first.wait(timeout=15)
        assert len(list(directory.glob("python-*.json"))) == 2
        with pytest.raises(OSError):
            with MigrationGuard(directory, exclusive=True):
                pytest.fail("もう1つの旧版を見落とした")
        finish(second)
        assert len(list(directory.glob("python-*.json"))) == 1
        with MigrationGuard(directory, exclusive=True):
            pass
    finally:
        for process in [first, second]:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=15)


def test_migration_blocks_new_python_start(tmp_path):
    directory = tmp_path / "runtime"
    with MigrationGuard(directory, exclusive=True):
        process = subprocess.run([sys.executable, "-c", WORKER, str(directory),
                                  str(tmp_path / "data")], cwd=ROOT, input="\n",
                                 capture_output=True, text=True, timeout=15)
        assert process.returncode != 0
        assert process.stdout == ""
        assert not list(directory.glob("python-*.json"))


def test_marker_failure_releases_guard(tmp_path, monkeypatch):
    import hashi.runtime_guard as module
    def fail(*_args, **_kwargs):
        raise PermissionError("書込拒否")
    monkeypatch.setattr(module, "save_json_atomic", fail)
    with pytest.raises(PermissionError):
        with PythonRuntime(tmp_path, data_root=tmp_path):
            pass
    with MigrationGuard(tmp_path, exclusive=True):
        pass


@pytest.mark.skipif(os.name == "nt", reason="Unixのシンボリックリンク検証")
def test_runtime_symlink_is_rejected(tmp_path):
    target = tmp_path / "target"
    target.mkdir()
    link = tmp_path / "link"
    link.symlink_to(target, target_is_directory=True)
    with pytest.raises(OSError):
        with PythonRuntime(link, data_root=tmp_path):
            pass


@pytest.mark.parametrize("mode", ["app", "editor"])
def test_main_holds_guard_for_both_entry_modes(qapp, tmp_path, monkeypatch, mode):
    import hashi.runtime_guard as module
    import main
    monkeypatch.setattr(module, "runtime_dir", lambda: tmp_path / "runtime")
    monkeypatch.setattr(main, "apply_dark_theme", lambda _app: None)
    monkeypatch.setattr(main, "QApplication", lambda _argv: qapp)
    def run(_app, actual_mode, _ns):
        assert actual_mode == mode
        assert len(list((tmp_path / "runtime").glob("python-*.json"))) == 1
        with pytest.raises(OSError):
            with MigrationGuard(tmp_path / "runtime", exclusive=True):
                pass
        return 7
    monkeypatch.setattr(main, "_run_application", run)
    assert main.main(["main.py"] + (["--editor"] if mode == "editor" else [])) == 7
    assert not list((tmp_path / "runtime").glob("python-*.json"))


def test_main_does_not_touch_data_when_guard_unavailable(qapp, tmp_path, monkeypatch):
    import hashi.runtime_guard as module
    import main
    monkeypatch.setattr(module, "runtime_dir", lambda: tmp_path)
    monkeypatch.setattr(main, "QApplication", lambda _argv: qapp)
    monkeypatch.setattr(main, "apply_dark_theme", lambda _app: None)
    monkeypatch.setattr(main.QMessageBox, "critical", lambda *_: None)
    monkeypatch.setattr(main, "_run_application", lambda *_: pytest.fail("移行中に起動した"))
    with MigrationGuard(tmp_path, exclusive=True):
        assert main.main(["main.py"]) == 1


@pytest.mark.parametrize('failure', ['lock_unsupported', 'path_denied', 'start_unknown',
                                  'marker_denied', 'start_command', 'marker_unicode'])
def test_main_warns_and_runs_when_marker_support_is_unavailable(qapp, tmp_path, monkeypatch, failure):
    import hashi.runtime_guard as module
    import main
    monkeypatch.setattr(module, 'runtime_dir', lambda: tmp_path / 'runtime')
    monkeypatch.setattr(main, 'QApplication', lambda _argv: qapp)
    monkeypatch.setattr(main, 'apply_dark_theme', lambda _app: None)
    warnings = []
    monkeypatch.setattr(main.QMessageBox, 'warning', lambda *_args: warnings.append(True))
    monkeypatch.setattr(main.QMessageBox, 'critical', lambda *_: pytest.fail('競合以外で起動を止めた'))
    def fail(*_args, **_kwargs):
        if failure == 'start_unknown':
            raise RuntimeError('開始時刻未対応')
        if failure == 'start_command':
            raise subprocess.CalledProcessError(1, 'ps')
        if failure == 'marker_unicode':
            raise UnicodeEncodeError('utf-8', '\udcff', 0, 1, 'fixture')
        raise OSError(errno.ENOTSUP if failure == 'lock_unsupported' else errno.EACCES, 'fixture')
    target = {'lock_unsupported': '_lock', 'path_denied': 'runtime_dir',
              'start_unknown': 'process_start', 'marker_denied': 'save_json_atomic',
              'start_command': 'process_start', 'marker_unicode': 'save_json_atomic'}[failure]
    monkeypatch.setattr(module, target, fail)
    ran = []
    def run(*_args):
        ran.append(True)
        if failure in ('start_unknown', 'marker_denied', 'start_command', 'marker_unicode'):
            # マーカーだけ失敗しても共有ロックでRust側を防ぐ。
            with pytest.raises(MigrationBusyError):
                with MigrationGuard(tmp_path / 'runtime', exclusive=True):
                    pass
        return 7
    monkeypatch.setattr(main, '_run_application', run)
    assert main.main(['main.py']) == 7
    assert ran == [True] and warnings == [True]


def test_best_effort_marker_failure_keeps_lock_until_normal_close(tmp_path, monkeypatch):
    import hashi.runtime_guard as module
    def fail(*_args):
        raise RuntimeError('開始時刻を照会できない')
    monkeypatch.setattr(module, 'process_start', fail)
    runtime = PythonRuntime(tmp_path, data_root=tmp_path).start_best_effort()
    try:
        assert runtime.warnings
        with pytest.raises(MigrationBusyError):
            with MigrationGuard(tmp_path, exclusive=True):
                pass
    finally:
        runtime.close()
    with MigrationGuard(tmp_path, exclusive=True):
        pass


@pytest.mark.skipif(os.name == 'nt', reason='Unixのシンボリックリンク検証')
def test_best_effort_does_not_write_into_rejected_runtime_directory(tmp_path):
    target = tmp_path / 'target'
    target.mkdir()
    link = tmp_path / 'link'
    link.symlink_to(target, target_is_directory=True)
    runtime = PythonRuntime(link, data_root=tmp_path).start_best_effort()
    try:
        assert runtime.warnings
        assert not list(target.iterdir())
        assert runtime.marker is None
    finally:
        runtime.close()


@pytest.mark.parametrize('kind', ['pending', 'corrupt', 'clear'])
def test_rust_checks_stale_marker_data_root_read_only(tmp_path, rust_binary, kind):
    from hashi.permjournal import PermJournal
    directory = tmp_path / 'runtime'
    legacy = tmp_path / 'legacy'
    legacy.mkdir()
    process = python_worker(directory, legacy)
    process.kill()
    process.wait(timeout=15)
    path = legacy / 'perm_journal.json'
    if kind == 'pending':
        PermJournal(path).record('fixture@host:22', '/fixture', 0o640, process.pid)
    else:
        path.write_bytes(b'{' if kind == 'corrupt' else b'{}')
    original = path.read_bytes()
    result = subprocess.run([str(rust_binary), str(directory), '--legacy-root', str(tmp_path / 'default')],
                            capture_output=True, text=True, timeout=15)
    assert (result.returncode == 0) == (kind == 'clear'), result.stderr
    assert path.read_bytes() == original
    assert len(list(directory.glob('python-*.json'))) == 1
    if kind == 'clear':
        assert '確認先: 2件' in result.stderr


@pytest.fixture
def rust_binary():
    value = os.environ.get("HASHI_RUST_GUARD_BINARY")
    if not value:
        pytest.skip("クロス言語試験はRustビルド付きCIで実行")
    binary = Path(value).resolve()
    assert binary.is_file()
    return binary


def test_rust_and_python_guards_are_bidirectionally_compatible(tmp_path, rust_binary):
    directory = tmp_path / "runtime"
    python = python_worker(directory, tmp_path / "data")
    try:
        result = subprocess.run([str(rust_binary), str(directory)], capture_output=True,
                                text=True, timeout=15)
        assert result.returncode != 0
    finally:
        finish(python)
    rust = subprocess.Popen([str(rust_binary), str(directory), "--hold"],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True)
    try:
        assert rust.stdout.readline().strip() == "guard-acquired"
        with pytest.raises(OSError):
            with PythonRuntime(directory, data_root=tmp_path):
                pytest.fail("Rustの移行中に旧版が起動した")
    finally:
        finish(rust)


def test_rust_recognizes_crashed_python_marker(tmp_path, rust_binary):
    directory = tmp_path / "runtime"
    python = python_worker(directory, tmp_path / "data")
    python.kill()
    python.wait(timeout=15)
    result = subprocess.run([str(rust_binary), str(directory)], capture_output=True,
                            text=True, timeout=15)
    assert result.returncode == 0, result.stderr
    assert "1件" in result.stderr


def test_rust_rejects_live_identity_even_without_held_lock(tmp_path, rust_binary):
    directory = tmp_path / "runtime"
    with PythonRuntime(directory, data_root=tmp_path) as runtime:
        data = runtime.marker.read_bytes()
        filename = runtime.marker.name
    (directory / filename).write_bytes(data)
    result = subprocess.run([str(rust_binary), str(directory)], capture_output=True,
                            text=True, timeout=15)
    assert result.returncode != 0


def test_rust_distinguishes_pid_reuse_and_rejects_future_schema(tmp_path, rust_binary):
    directory = tmp_path / "runtime"
    with PythonRuntime(directory, data_root=tmp_path) as runtime:
        data = json.loads(runtime.marker.read_text())
        filename = runtime.marker.name
    # 現在生きているPIDでも、記録と作成時刻が違うなら以前のプロセス。
    data["process_start"] = data["process_start"].rsplit(":", 1)[0] + ":0"
    (directory / filename).write_text(json.dumps(data))
    result = subprocess.run([str(rust_binary), str(directory)], capture_output=True,
                            text=True, timeout=15)
    assert result.returncode == 0, result.stderr
    data["schema_version"] = 999
    (directory / filename).write_text(json.dumps(data))
    result = subprocess.run([str(rust_binary), str(directory)], capture_output=True,
                            text=True, timeout=15)
    assert result.returncode != 0


def test_killed_rust_releases_guard_for_python(tmp_path, rust_binary):
    directory = tmp_path / "runtime"
    rust = subprocess.Popen([str(rust_binary), str(directory), "--hold"],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True)
    try:
        assert rust.stdout.readline().strip() == "guard-acquired"
        rust.kill()
        rust.wait(timeout=15)
        with PythonRuntime(directory, data_root=tmp_path):
            pass
    finally:
        if rust.poll() is None:
            rust.kill()
            rust.wait(timeout=15)
