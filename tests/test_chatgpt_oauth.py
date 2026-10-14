import json
import threading
import time
import urllib.parse
import urllib.request

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa

from hashi.chatgpt_oauth import ISSUER, SCOPES, ChatGptAccounts, ChatGptProvider, OAuthAttempt


class Store:
    def __init__(self):
        self.data, self._lock = {}, threading.RLock()

    def get(self, key):
        return self.data.get(key)

    def set(self, key, value):
        self.data[key] = value


class Http:
    def __init__(self):
        self.key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        self.jwk = json.loads(jwt.algorithms.RSAAlgorithm.to_jwk(self.key.public_key()))
        self.jwk.update(kid="test", use="sig")
        self.nonce = ""
        self.refreshes = 0

    def token(self, *, nonce=None, audience="issued", subject="subject", exp=None):
        return jwt.encode({"iss": ISSUER, "aud": audience, "sub": subject,
                           "iat": int(time.time()), "exp": exp or int(time.time()) + 3600,
                           "nonce": nonce if nonce is not None else self.nonce,
                           "email": "test@example.com"}, self.key, algorithm="RS256", headers={"kid": "test"})

    def json(self, url, payload=None, headers=None, form=False):
        if url.endswith("openid-configuration"):
            return {"issuer": ISSUER, "jwks_uri": ISSUER + "/jwks", "revocation_endpoint": ISSUER + "/revoke"}
        if url.endswith("/jwks"):
            return {"keys": [self.jwk]}
        if url.endswith("/token"):
            assert payload["client_id"] == "issued"
            assert payload["resource"] == "https://api.openai.com/v1"
            if payload["grant_type"] == "refresh_token":
                self.refreshes += 1
                return {"access_token": "new-access", "refresh_token": "rotated", "expires_in": 3600, "token_type": "Bearer"}
            return {"id_token": self.token(), "access_token": "access", "refresh_token": "refresh",
                    "expires_in": 3600, "token_type": "Bearer", "scope": SCOPES}
        if url.endswith("/models"):
            return {"models": [{"slug": "visible", "display_name": "Visible", "visibility": "list"},
                               {"slug": "hidden", "visibility": "hide"}]}
        raise AssertionError(url)


@pytest.fixture
def setup(tmp_config, monkeypatch):
    import hashi.chatgpt_oauth as module
    monkeypatch.setattr(module, "config_dir", lambda: tmp_config)
    store, http = Store(), Http()
    return store, http, ChatGptAccounts(store, http)


def test_state_pkce_issued_id_and_reauthorization():
    attempt = OAuthAttempt("host")
    url = urllib.parse.urlsplit(attempt.authorization_url("http://127.0.0.1:1234/auth/callback"))
    query = urllib.parse.parse_qs(url.query)
    assert query["client_id"] == ["dynamic_agent_client"]
    assert query["agent_name_hint"] == ["Hashi"]
    assert query["code_challenge_method"] == ["S256"]
    with pytest.raises(ValueError):
        attempt.callback({"state": ["wrong"], "code": ["x"], "client_id": ["issued"]})
    with pytest.raises(ValueError):
        attempt.callback({"state": [attempt.state], "code": ["x"]})
    returning = OAuthAttempt("host", {"client_id": "issued", "id_token": "hint"})
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(returning.authorization_url("http://127.0.0.1:9/auth/callback")).query)
    assert "agent_name_hint" not in query
    with pytest.raises(ValueError):
        returning.callback({"state": [returning.state], "code": ["x"], "client_id": ["other"]})


def test_signature_nonce_audience_and_expiry(setup):
    _, http, accounts = setup
    assert accounts._identity(http.token(nonce="right"), "issued", "right")["sub"] == "subject"
    with pytest.raises(ValueError):
        accounts._identity(http.token(nonce="wrong"), "issued", "right")
    with pytest.raises(ValueError):
        accounts._identity(http.token(audience="wrong"), "issued")
    with pytest.raises(ValueError):
        accounts._identity(http.token(exp=int(time.time()) - 20), "issued")


def test_loopback_sign_in_and_serialized_refresh(setup):
    store, http, accounts = setup
    browsers = []
    def open_browser(url):
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)
        http.nonce = query["nonce"][0]
        callback = query["redirect_uri"][0] + "?" + urllib.parse.urlencode({"state": query["state"][0], "code": "code", "client_id": "issued"})
        def call():
            with urllib.request.urlopen(callback, timeout=3) as response:
                assert response.status == 200
        browser = threading.Thread(target=call, daemon=True)
        browsers.append(browser)
        browser.start()
        return True
    assert accounts.sign_in(threading.Event(), open_browser=open_browser) == "issued"
    for browser in browsers:
        browser.join(3)
    assert accounts.list()[0]["connected"]
    saved = json.loads(store.get("oauth:accounts"))
    saved["issued"]["expires_at"] = 0
    store.set("oauth:accounts", json.dumps(saved))
    values = []
    threads = [threading.Thread(target=lambda: values.append(accounts.access_token("issued"))) for _ in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(3)
    assert values == ["new-access"] * 4
    assert http.refreshes == 1
    assert json.loads(store.get("oauth:accounts"))["issued"]["refresh_token"] == "rotated"
    assert accounts.models("issued")[0]["slug"] == "visible"


def test_plan_scopes_and_payload(setup):
    _, _, accounts = setup
    with pytest.raises(PermissionError):
        accounts._record({"scope": "openid"}, "issued", {"sub": "test"})
    provider = ChatGptProvider(accounts, "issued", "model")
    payload = provider._responses_payload([{"role": "user", "content": "x"}], "instruction", [])
    assert payload["store"] is False and payload["stream"] is True
    assert payload["tools"][0]["type"] == "namespace"
    assert "previous_response_id" not in payload and "max_output_tokens" not in payload


def test_failed_remote_revocation_clears_local_tokens_but_keeps_registration(setup):
    store, _, accounts = setup
    store.set("oauth:accounts", json.dumps({"issued": {"client_id": "issued", "subject": "s", "refresh_token": "r", "access_token": "a", "id_token": "i"}}))
    assert accounts.sign_out("issued") is False
    record = json.loads(store.get("oauth:accounts"))["issued"]
    assert record == {"client_id": "issued", "subject": "s"}
