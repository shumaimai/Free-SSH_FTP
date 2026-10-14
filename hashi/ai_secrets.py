"""AI認証はSSH資格情報および接続情報の同期から隔離する。"""
import threading
from contextlib import contextmanager

from PySide6.QtCore import QLockFile

from .credentials import _FernetFile


class AiSecretStore:
    def __init__(self, credentials):
        self._keyring = credentials._keyring
        self._file = None if self._keyring else _FernetFile("ai")
        self._lock = threading.RLock()

    def get(self, key):
        with self._transaction():
            return (self._keyring.get_password("Hashi.AI", key) if self._keyring
                    else self._file.get(key))

    def set(self, key, value):
        with self._transaction():
            if self._keyring:
                self._keyring.set_password("Hashi.AI", key, value)
            else:
                self._file.set(key, value)

    def delete(self, key):
        with self._transaction():
            if self._keyring:
                if self._keyring.get_password("Hashi.AI", key) is not None:
                    self._keyring.delete_password("Hashi.AI", key)
            else:
                self._file.delete(key)

    @contextmanager
    def _transaction(self):
        with self._lock:
            lock = None
            if self._file is not None:
                lock = QLockFile(str(self._file.dir / ".ai-secrets.lock"))
                lock.setStaleLockTime(60000)
                if not lock.tryLock(3000):
                    raise RuntimeError("別のHashiが認証情報を保存しています")
            try:
                yield
            finally:
                if lock is not None:
                    lock.unlock()
