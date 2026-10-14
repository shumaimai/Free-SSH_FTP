"""AI認証はSSH資格情報および接続情報の同期から隔離する。"""
import threading

from .credentials import _FernetFile


class AiSecretStore:
    def __init__(self, credentials):
        self._keyring = credentials._keyring
        self._file = None if self._keyring else _FernetFile("ai")
        self._lock = threading.RLock()

    def get(self, key):
        with self._lock:
            return (self._keyring.get_password("Hashi.AI", key) if self._keyring
                    else self._file.get(key))

    def set(self, key, value):
        with self._lock:
            if self._keyring:
                self._keyring.set_password("Hashi.AI", key, value)
            else:
                self._file.set(key, value)

    def delete(self, key):
        with self._lock:
            if self._keyring:
                if self._keyring.get_password("Hashi.AI", key) is not None:
                    self._keyring.delete_password("Hashi.AI", key)
            else:
                self._file.delete(key)
