"""v1保存形式のPython/Rust両方向試験。全データは合成した試験用。"""
import json
import os
import subprocess
import unicodedata
from dataclasses import asdict
from pathlib import Path

import pytest
from cryptography.fernet import Fernet

from hashi.cloudsync import CloudSyncError, decrypt_blob, encrypt_blob
from hashi.config import Profile
from hashi.portability import _decrypt_secrets, loads_bundle

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def rust_storage_binary():
    value = os.environ.get("HASHI_RUST_STORAGE_BINARY")
    if not value:
        pytest.skip("Rust保存形式のクロス言語試験はビルド付きCIで実行")
    binary = Path(value).resolve()
    assert binary.is_file()
    return binary


def call(binary, **request):
    result = subprocess.run([str(binary)], input=json.dumps(request, ensure_ascii=False),
                            capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def test_rust_ciphertext_is_readable_by_python(rust_storage_binary):
    phrase = "日本語の鍵🔑e\u0301"
    plaintext = "合成した試験データ\n\u0000"
    result = call(rust_storage_binary, operation="encrypt_cloud",
                  plaintext=plaintext, passphrase=phrase)
    envelope = json.dumps(result["envelope"]).encode()
    assert decrypt_blob(envelope, phrase) == plaintext.encode()
    with pytest.raises(CloudSyncError):
        decrypt_blob(envelope, unicodedata.normalize("NFC", phrase))
    assert plaintext not in envelope.decode()
    secrets = {"demo@server.invalid:22": {"password": "試験用パスワード"}}
    result = call(rust_storage_binary, operation="encrypt_secrets",
                  secrets=secrets, passphrase=phrase)
    assert _decrypt_secrets(result["envelope"], phrase) == secrets
    assert "試験用パスワード" not in json.dumps(result, ensure_ascii=False)
    key = Fernet.generate_key()
    result = call(rust_storage_binary, operation="encrypt_fernet", key=key.decode(),
                  plaintext=plaintext)
    assert Fernet(key).decrypt(result["token"].encode()) == plaintext.encode()


def test_python_cloud_ciphertext_is_readable_by_rust(rust_storage_binary):
    phrase, plaintext = "別の試験用鍵", "PythonからRustへ\n"
    envelope = json.loads(encrypt_blob(plaintext.encode(), phrase))
    result = call(rust_storage_binary, operation="decrypt_cloud",
                  envelope=envelope, passphrase=phrase)
    assert result["plaintext"] == plaintext


def test_export_profile_roundtrip_keeps_proxy_auth_and_settings(rust_storage_binary):
    profile = Profile(name="合成", host="server.invalid", port=2222, username="demo",
                      auth_method="agent", proxy_jump="demo@hop.invalid:2200",
                      tags=["テスト"], agent_forwarding=True, initial_path="/tmp")
    result = call(rust_storage_binary, operation="profile_roundtrip", profile=asdict(profile))
    assert Profile.from_dict(result) == profile
    bundle = {"format": "hashi-export", "version": 1, "profiles": [asdict(profile)],
              "known_hosts": {"server.invalid:2222": {"key_type": "ssh-rsa", "fingerprint": "SHA256:test"}}}
    result = call(rust_storage_binary, operation="bundle_roundtrip", bundle=bundle)
    restored = loads_bundle(json.dumps(result).encode())
    assert restored.profiles == [profile]
    assert restored.known_hosts == bundle["known_hosts"]
    assert not restored.has_encrypted_secrets


def test_unknown_future_envelope_is_rejected_without_returning_plaintext(rust_storage_binary):
    envelope = json.loads(encrypt_blob(b"test", "test"))
    envelope["version"] = 999
    result = subprocess.run([str(rust_storage_binary)],
                            input=json.dumps({"operation": "decrypt_cloud", "envelope": envelope,
                                              "passphrase": "test"}),
                            capture_output=True, text=True, timeout=30)
    assert result.returncode != 0
    assert not result.stdout
