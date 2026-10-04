import threading
import time
from types import SimpleNamespace

import pytest

from hashi.ai_api import ApiProvider, responses_input
from hashi.ai_core import tool_definitions
from hashi.ai_http import JsonHttp, validate_base_url
from hashi.ai_secrets import AiSecretStore


class Http:
    def __init__(self, events):
        self.data, self.requests = events, []

    def events(self, url, payload, headers, cancel):
        self.requests.append((url, payload, headers))
        yield from self.data


def test_invalid_header_error_does_not_display_api_key():
    http = JsonHttp()
    def invalid(*args, **kwargs):
        raise ValueError("Invalid header value b'Bearer test-sensitive-key\\n'")
    http.opener.open = invalid
    with pytest.raises(RuntimeError) as caught:
        http.open("https://example.com/v1/models", headers={"Authorization": "Bearer test-sensitive-key"})
    assert "入力形式" in str(caught.value) and "test-sensitive-key" not in str(caught.value)


def test_responses_tool_and_reasoning_history():
    output = [{"type": "reasoning", "id": "r", "encrypted_content": "encrypted", "summary": []},
              {"type": "function_call", "call_id": "c", "name": "list_sessions", "arguments": "{}"}]
    http = Http([{"type": "response.output_text.delta", "delta": "日本語"},
                 {"type": "response.completed", "response": {"output": output}}])
    chunks = []
    reply = ApiProvider("openai", "test", "fake", http=http).respond([], "", tool_definitions(), threading.Event(), chunks.append)
    assert chunks == ["日本語"]
    assert reply["tool_calls"] == [{"id": "c", "name": "list_sessions", "arguments": {}}]
    assert http.requests[0][1]["store"] is False
    assert responses_input([{"role": "assistant", "raw_output": reply["raw_output"]}]) == output
    with pytest.raises(RuntimeError):
        ApiProvider("openai", "test", "fake", http=Http([])).respond([], "", [], threading.Event(), lambda _: None)


def test_anthropic_tool_stream():
    http = Http([{"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "1", "name": "list_sessions"}},
                 {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{}"}},
                 {"type": "message_delta", "delta": {"stop_reason": "tool_use"}}, {"type": "message_stop"}])
    reply = ApiProvider("anthropic", "test", "fake", http=http).respond([], "", [], threading.Event(), lambda _: None)
    assert reply["tool_calls"][0]["arguments"] == {}


def test_compatible_fragmented_call():
    http = Http([{"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "1", "function": {"name": "list_sessions", "arguments": "{"}}]}}]},
                 {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "}"}}]}, "finish_reason": "tool_calls"}]}])
    reply = ApiProvider("compatible", "test", "", base_url="http://127.0.0.1:1234/v1", http=http).respond([], "", [], threading.Event(), lambda _: None)
    assert reply["tool_calls"][0]["arguments"] == {}


def test_compatible_advice_mode_omits_tools_and_rejects_unsolicited_calls():
    http = Http([{"choices": [{"delta": {"content": "助言"}, "finish_reason": "stop"}]}])
    provider = ApiProvider("compatible", "test", "", base_url="http://127.0.0.1:1234/v1",
                           http=http, use_tools=False)
    reply = provider.respond([], "", tool_definitions(), threading.Event(), lambda _: None)
    assert reply["text"] == "助言" and reply["tool_calls"] == []
    assert "tools" not in http.requests[0][1]
    http.data = [{"choices": [{"delta": {"tool_calls": [{"index": 0}]}, "finish_reason": "tool_calls"}]}]
    with pytest.raises(RuntimeError, match="実行していません"):
        provider.respond([], "", tool_definitions(), threading.Event(), lambda _: None)


@pytest.mark.parametrize("url", [
    "https://api.commandcode.ai", "https://api.commandcode.ai/v1/",
    "https://api.commandcode.ai/provider/v1/chat/completions",
])
def test_commandcode_normalizes_url(url):
    provider = ApiProvider("commandcode", "test", "fake", base_url=url)
    assert provider.base == "https://api.commandcode.ai/provider/v1"


def test_commandcode_routes_claude_to_messages_with_bearer():
    http = Http([{"type": "content_block_delta", "index": 0,
                  "delta": {"type": "text_delta", "text": "OK"}},
                 {"type": "message_delta", "delta": {"stop_reason": "end_turn"}},
                 {"type": "message_stop"}])
    provider = ApiProvider("commandcode", "claude-sonnet-4-6", "fake", http=http)
    assert provider.respond([], "", [], threading.Event(), lambda _: None)["text"] == "OK"
    assert http.requests[0][0].endswith("/provider/v1/messages")
    assert http.requests[0][2]["Authorization"] == "Bearer fake"


def test_settings_save_failure_is_specific_and_does_not_close(qapp, tmp_config):
    from hashi.ai_settings import AiSettingsDialog
    from hashi.config import Settings

    class Secrets:
        def get(self, key):
            return None
        def set(self, key, value):
            raise OSError("test-sensitive-key")
    dialog = AiSettingsDialog(Settings(), Secrets())
    dialog.kind.setCurrentIndex(dialog.kind.findData("commandcode"))
    dialog.model.setText("test-model")
    dialog.key.setText("test-sensitive-key")
    dialog.persist.setChecked(True)
    dialog.show()
    dialog._accept()
    deadline = time.monotonic() + 5
    while dialog.worker is not None and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert dialog.isVisible() and dialog.provider is None
    assert "APIキーの保存" in dialog.status.text()
    assert "test-sensitive-key" not in dialog.status.text()
    dialog.close()


def test_commandcode_catalog_selects_supported_protocol(qapp, tmp_config):
    from hashi.ai_settings import AiSettingsDialog
    from hashi.config import Settings
    dialog = AiSettingsDialog(Settings(), AiSecretStore(SimpleNamespace(_keyring=None)))
    dialog.kind.setCurrentIndex(dialog.kind.findData("commandcode"))
    dialog.key.setText("fake")
    dialog._got_models([{"id": "future-model", "supported_endpoints": ["/v1/messages"]}])
    assert dialog._provider().protocol == "messages"
    dialog._accept()
    deadline = time.monotonic() + 5
    while dialog.worker is not None and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert dialog.provider is not None
    restored = AiSettingsDialog(Settings(), AiSecretStore(SimpleNamespace(_keyring=None)))
    restored.key.setText("fake")
    assert restored._provider().protocol == "messages"
    dialog.deleteLater()
    restored.deleteLater()
    qapp.processEvents()


@pytest.mark.parametrize("url", ["http://remote.example", "https://user:pw@example.com", "https://example.com?key=x", "file:///tmp/key"])
def test_unsafe_endpoint(url):
    with pytest.raises(ValueError):
        validate_base_url(url)


def test_ai_secrets_separate(tmp_config):
    store = AiSecretStore(SimpleNamespace(_keyring=None))
    store.set("api:test", "sensitive-token")
    assert store.get("api:test") == "sensitive-token"
    assert b"sensitive-token" not in (tmp_config / "ai-secrets.dat").read_bytes()
    assert not (tmp_config / "creds.dat").exists()
    store.delete("api:test")
    assert store.get("api:test") is None


def test_settings_key_reset(qapp, tmp_config):
    from hashi.ai_settings import AiSettingsDialog
    from hashi.config import Settings
    dialog = AiSettingsDialog(Settings(tmp_config / "settings.json"), AiSecretStore(SimpleNamespace(_keyring=None)))
    dialog.kind.setCurrentIndex(2)
    dialog.key.setText("fake")
    dialog.base.textEdited.emit("new")
    assert not dialog.key.text()
    dialog.deleteLater()
    qapp.processEvents()


def test_api_connection_settings_reload_without_secret(qapp, tmp_config):
    from hashi.ai_settings import AiSettingsDialog
    from hashi.config import Settings

    settings = Settings()
    secrets = AiSecretStore(SimpleNamespace(_keyring=None))
    dialog = AiSettingsDialog(settings, secrets)
    dialog.kind.setCurrentIndex(dialog.kind.findData("compatible"))
    dialog.base.setText("http://127.0.0.1:4321/v1")
    dialog.model.setText("test-model")
    dialog.key.setText("test-sensitive-key")
    dialog.persist.setChecked(True)
    dialog.use_tools.setChecked(False)
    dialog._accept()
    deadline = time.monotonic() + 5
    while dialog.worker is not None and time.monotonic() < deadline:
        qapp.processEvents()
        time.sleep(.005)
    assert dialog.provider is not None
    restored = AiSettingsDialog(Settings(), secrets)
    assert restored.kind.currentData() == "compatible"
    assert restored.base.text() == "http://127.0.0.1:4321/v1"
    assert restored.model.text() == "test-model"
    assert restored.key.text() == "test-sensitive-key"
    assert not restored.use_tools.isChecked()
    assert "test-sensitive-key" not in settings.path.read_text()
    dialog.deleteLater()
    restored.deleteLater()
    qapp.processEvents()


def test_ai_secrets_do_not_enter_ssh_export_or_sync_bundle(tmp_config):
    import json

    from hashi.config import KnownHosts, Profile, Settings
    from hashi.credentials import CredentialStore
    from hashi.portability import build_bundle_dict

    credentials = CredentialStore()
    store = AiSecretStore(credentials)
    store.set("api:test", "test-api-key")
    store.set("oauth:account", "test-access-refresh-id-tokens")
    profile = Profile(host="host", username="user")
    credentials.set(profile, "password", "ssh-password")
    bundle, count = build_bundle_dict([profile], KnownHosts(), credentials,
                                     passphrase="test-export-passphrase")
    # 同期も同じbuild_bundle_dictを使用。秘密の対象はSSH資格情報だけ。
    assert count == 1
    serialized = json.dumps(bundle) + json.dumps(Settings()._data)
    assert "test-api-key" not in serialized
    assert "test-access-refresh-id-tokens" not in serialized
    assert store.get("api:test") == "test-api-key"
