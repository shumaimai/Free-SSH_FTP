"""Rust移行中の旧版停止確認。ファイルの有無ではなくOSロックで調停する。"""
from __future__ import annotations

import ctypes
import errno
import logging
import os
import stat
import subprocess
import sys
import uuid
from pathlib import Path

from . import __version__
from .jsonio import save_json_atomic

logger = logging.getLogger(__name__)
SCHEMA_VERSION = 1
GUARD_NAME = "migration.guard"


class MigrationBusyError(OSError):
    """実際のOSロック競合だけを表す。保存先/対応OSの異常と区別する。"""


def _lock(file, *, exclusive: bool) -> None:
    try:
        if os.name == "nt":
            _windows_lock(file, exclusive=exclusive)
        else:
            import fcntl
            fcntl.flock(file, (fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH)
                        | fcntl.LOCK_NB)
    except OSError as error:
        busy = (getattr(error, "winerror", None) == 33 if os.name == "nt" else
                error.errno in (errno.EAGAIN, errno.EWOULDBLOCK))
        if busy:
            raise MigrationBusyError("移行処理が実行中です") from error
        raise


def runtime_dir() -> Path:
    if os.name == "nt":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
        return base / "Hashi-Migration" / "runtime"
    # XDG_RUNTIME_DIRがない環境も同じ利用者の各版で同じ場所を使う。
    base = Path(os.environ.get("XDG_RUNTIME_DIR") or
                os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return base / "Hashi-Migration" / "runtime"


def process_start() -> str:
    if os.name == "nt":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentProcess.restype = wintypes.HANDLE
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [
            ctypes.POINTER(wintypes.FILETIME)] * 4
        kernel.GetProcessTimes.restype = wintypes.BOOL
        times = [wintypes.FILETIME() for _ in range(4)]
        if not kernel.GetProcessTimes(kernel.GetCurrentProcess(),
                                     *(ctypes.byref(t) for t in times)):
            raise ctypes.WinError(ctypes.get_last_error())
        created = times[0].dwLowDateTime | (times[0].dwHighDateTime << 32)
        return f"windows:{created}"
    if sys.platform.startswith("linux"):
        boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        fields = Path(f"/proc/{os.getpid()}/stat").read_text().rsplit(")", 1)[1].split()
        return f"linux:{boot}:{fields[19]}"
    if sys.platform == "darwin":
        created = subprocess.check_output(
            ["/bin/ps", "-p", str(os.getpid()), "-o", "lstart="], text=True).strip()
        if created:
            return f"darwin:{created}"
    # 未検証OSでPIDだけに格下げしない。
    raise RuntimeError("実行中マーカーのプロセス開始時刻を取得できないOSです")


def _windows_lock(file, *, exclusive: bool, unlock: bool = False) -> None:
    import msvcrt
    from ctypes import wintypes

    class Overlapped(ctypes.Structure):
        _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                    ("Offset", wintypes.DWORD), ("OffsetHigh", wintypes.DWORD),
                    ("hEvent", wintypes.HANDLE)]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    overlapped = Overlapped()
    handle = msvcrt.get_osfhandle(file.fileno())
    if unlock:
        func = kernel.UnlockFileEx
        func.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                         wintypes.DWORD, ctypes.POINTER(Overlapped)]
        args = (handle, 0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(overlapped))
    else:
        func = kernel.LockFileEx
        func.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                         wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(Overlapped)]
        # fs2のWindowsバックエンドと同じオフセット/長さ。待機はしない。
        flags = 1 | (2 if exclusive else 0)
        args = (handle, flags, 0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(overlapped))
    func.restype = wintypes.BOOL
    if not func(*args):
        raise ctypes.WinError(ctypes.get_last_error())


class MigrationGuard:
    """旧版は共有、移行側は排他。ロックファイル自体は削除しない。"""

    def __init__(self, directory: Path | None = None, *, exclusive: bool = False):
        self._directory = directory
        self.exclusive = exclusive
        self._file = None
        self.can_write_marker = False

    @property
    def directory(self) -> Path:
        if self._directory is None:
            self._directory = runtime_dir()
        return self._directory

    def __enter__(self):
        self.can_write_marker = False
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        info = self.directory.lstat()
        if (not stat.S_ISDIR(info.st_mode) or
                getattr(info, "st_file_attributes", 0) & 0x400):
            raise OSError("移行マーカーの保存先が通常のディレクトリではありません")
        if os.name != "nt":
            if info.st_uid != os.getuid():
                raise PermissionError("移行マーカーの保存先の所有者が異なります")
            self.directory.chmod(0o700)
        path = self.directory / GUARD_NAME
        if path.is_symlink() or (path.exists() and
                getattr(path.lstat(), "st_file_attributes", 0) & 0x400):
            raise OSError("移行ガードにリンクは使用できません")
        fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
        file = os.fdopen(fd, "r+b")
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode):
                raise OSError("移行ガードは通常ファイルである必要があります")
            if os.name != "nt" and info.st_uid != os.getuid():
                raise PermissionError("移行ガードの所有者が異なります")
            self.can_write_marker = True
            _lock(file, exclusive=self.exclusive)
        except BaseException:
            file.close()
            raise
        self._file = file
        return self

    def close(self):
        file, self._file = self._file, None
        if file is not None:
            # closeはOSロックを解放する。ファイルをunlinkすると別inodeをロックしうる。
            file.close()

    def __exit__(self, *_):
        self.close()


class PythonRuntime:
    """全保存データ処理を共有ロックの内側へ置き、起動ごとに識別記録を残す。"""

    def __init__(self, directory: Path | None = None, *, data_root: Path | None = None):
        self.guard = MigrationGuard(directory)
        self.data_root = data_root
        self.marker: Path | None = None
        self.warnings: list[str] = []

    def __enter__(self):
        return self._start(strict=True)

    def start_best_effort(self):
        """安定版は競合時だけ停止し、それ以外は警告して起動する。"""
        return self._start(strict=False)

    def _start(self, *, strict: bool):
        try:
            self.guard.__enter__()
        except MigrationBusyError:
            raise
        except (OSError, RuntimeError, UnicodeError, subprocess.SubprocessError) as error:
            if strict:
                raise
            self.warnings.append(f"移行ガードを取得できません: {error}")
            if not self.guard.can_write_marker:
                return self
        try:
            from .config import config_dir
            root = self.data_root if self.data_root is not None else config_dir()
            instance = uuid.uuid4().hex
            self.marker = self.guard.directory / f"python-{instance}.json"
            save_json_atomic(self.marker, {
                "schema_version": SCHEMA_VERSION,
                "implementation": "python",
                "application_version": __version__,
                "pid": os.getpid(),
                "process_start": process_start(),
                "instance_id": instance,
                "executable": str(Path(sys.executable).resolve()),
                "data_root": str(root.resolve()),
            }, ensure_ascii=False, fsync=True)
            return self
        except (OSError, RuntimeError, UnicodeError, subprocess.SubprocessError) as error:
            if strict:
                self.close()
                raise
            # マーカーだけ失敗しても、取得できた共有ロックは終了まで保つ。
            self.warnings.append(f"実行中マーカーを作成できません: {error}")
            return self
        except BaseException:
            self.close()
            raise

    def close(self):
        marker, self.marker = self.marker, None
        try:
            if marker is not None:
                marker.unlink(missing_ok=True)
        except OSError:
            logger.warning("実行中マーカーを削除できません", exc_info=True)
        finally:
            self.guard.close()

    def __exit__(self, *_):
        self.close()
