from hashi.config import Profile


def test_fernet_file_roundtrip(tmp_config):
    from hashi.credentials import _FernetFile
    f = _FernetFile()
    f.set("k1", "secret-value")
    assert f.get("k1") == "secret-value"
    f.delete("k1")
    assert f.get("k1") is None


def test_credential_store_roundtrip(tmp_config, monkeypatch):
    # keyring を強制的に無効化してファイルバックエンドを使わせる
    import hashi.credentials as creds

    class _NoKeyring:
        def _init_backend(self_inner):
            self_inner._keyring = None
            self_inner._file = creds._FernetFile()
            self_inner.backend_name = "encrypted-file"

    monkeypatch.setattr(creds.CredentialStore, "_init_backend",
                        _NoKeyring._init_backend)
    store = creds.CredentialStore()
    assert store.available
    assert store.is_secure() is False

    p = Profile(host="h", port=22, username="u")
    store.set(p, "password", "pw1")
    store.set(p, "sudo", "sudopw")
    assert store.get(p, "password") == "pw1"
    assert store.get(p, "sudo") == "sudopw"
    store.delete(p, "password")
    assert store.get(p, "password") is None
    store.clear_profile(p)
    assert store.get(p, "sudo") is None


def test_probe_keyring_success():
    """書き込みプローブが成功すれば True。"""
    from hashi.credentials import CredentialStore

    class _FakeKeyring:
        def set_password(self, *a):
            self.written = a

        def delete_password(self, *a):
            pass

    assert CredentialStore._probe_keyring(_FakeKeyring()) is True


def test_probe_keyring_error_falls_back():
    """プローブが例外を投げれば使えないと判断する。"""
    from hashi.credentials import CredentialStore

    class _BoomKeyring:
        def set_password(self, *a):
            raise RuntimeError("no backend")

        def delete_password(self, *a):
            pass

    assert CredentialStore._probe_keyring(_BoomKeyring()) is False


def test_probe_keyring_hang_times_out():
    """バックエンドがブロックしても時間内に False を返す(起動を固めない)。"""
    import threading
    import time

    from hashi.credentials import CredentialStore

    release = threading.Event()

    class _HangKeyring:
        def set_password(self, *a):
            release.wait(30)   # 応答しない Secret Service を模す

        def delete_password(self, *a):
            pass

    t0 = time.monotonic()
    ok = CredentialStore._probe_keyring(_HangKeyring(), timeout=0.2)
    elapsed = time.monotonic() - t0
    release.set()   # 取り残したデーモンスレッドを解放
    assert ok is False
    assert elapsed < 5   # タイムアウトで速やかに戻る
