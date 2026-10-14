"""公式Sign in with ChatGPTの公開クライアント認証とResponses接続。"""
from __future__ import annotations

import base64
import hashlib
import json
import secrets
import threading
import time
import urllib.parse
import uuid
import webbrowser
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, HTTPServer

import jwt
from PySide6.QtCore import QLockFile

from .ai_api import ApiProvider
from .ai_http import JsonHttp, validate_base_url
from .config import config_dir

ISSUER = "https://auth.openai.com"
AUTHORIZE = ISSUER + "/api/accounts/authorize"
TOKEN = ISSUER + "/api/accounts/oauth/token"
RESOURCE = "https://api.openai.com/v1"
SCOPES = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"


def official_url(url):
    validate_base_url(url)
    p = urllib.parse.urlsplit(url)
    if p.scheme != "https" or p.hostname != "auth.openai.com" or p.port not in (None, 443):
        raise ValueError("認証メタデータの接続先が公式ホストではありません")
    return url


class OAuthAttempt:
    def __init__(self, host_id, registration=None):
        self.registration = registration
        self.state, self.nonce, self.verifier = (secrets.token_urlsafe(32) for _ in range(3))
        self.host_id = host_id

    def authorization_url(self, redirect):
        challenge = base64.urlsafe_b64encode(hashlib.sha256(self.verifier.encode()).digest()).decode().rstrip("=")
        query = {"client_id": self.registration["client_id"] if self.registration else "dynamic_agent_client",
                 "ext_agent_host_id": self.host_id, "response_type": "code", "redirect_uri": redirect,
                 "scope": SCOPES, "resource": RESOURCE, "state": self.state, "nonce": self.nonce,
                 "code_challenge_method": "S256", "code_challenge": challenge}
        if self.registration:
            if self.registration.get("id_token"):
                query["id_token_hint"] = self.registration["id_token"]
            if self.registration.get("email"):
                query["login_hint"] = self.registration["email"]
        else:
            query["agent_name_hint"] = "Hashi"
        return AUTHORIZE + "?" + urllib.parse.urlencode(query)

    def callback(self, query):
        def value(name):
            values = query.get(name, [])
            if len(values) != 1:
                raise ValueError("認証コールバックが不正です")
            return values[0]
        if not secrets.compare_digest(value("state"), self.state):
            raise ValueError("認証状態が一致しません")
        if "error" in query:
            raise PermissionError("ChatGPTの認証がキャンセルされました")
        issued = value("client_id") if "client_id" in query else (
            self.registration["client_id"] if self.registration else None)
        if not issued or issued == "dynamic_agent_client" or len(issued) > 512:
            raise ValueError("発行されたクライアントIDがありません")
        if self.registration and issued != self.registration["client_id"]:
            raise ValueError("選択した登録とクライアントIDが一致しません")
        return value("code"), issued


class ChatGptAccounts:
    def __init__(self, store, http=None):
        self.store, self.http = store, http or JsonHttp()
        self._lock = threading.RLock()

    @contextmanager
    def _transaction(self):
        with self._lock, self.store._lock:
            lock = QLockFile(str(config_dir() / ".ai-oauth.lock"))
            lock.setStaleLockTime(60000)
            if not lock.tryLock(3000):
                raise RuntimeError("別のHashiが認証処理中です。少し待ってください")
            try:
                yield
            finally:
                lock.unlock()

    def _load(self):
        return json.loads(self.store.get("oauth:accounts") or "{}")

    def _save(self, data):
        self.store.set("oauth:accounts", json.dumps(data))

    def list(self):
        with self._transaction():
            return [{"id": key, "label": f"{r.get('email') or r['subject']} · {key[:10]}",
                     "connected": bool(r.get("refresh_token"))} for key, r in self._load().items()]

    def _metadata(self):
        data = self.http.json(ISSUER + "/.well-known/openid-configuration")
        if data.get("issuer") != ISSUER:
            raise ValueError("認証発行者が一致しません")
        return data

    def _identity(self, token, client_id, nonce=None):
        header = jwt.get_unverified_header(token)
        if header.get("alg") != "RS256":
            raise ValueError("IDトークンの署名方式が不正です")
        keys = self.http.json(official_url(self._metadata()["jwks_uri"]))["keys"]
        matching = [k for k in keys if k.get("kid") == header.get("kid") and k.get("use", "sig") == "sig"]
        if len(matching) != 1:
            raise ValueError("IDトークンの署名鍵が見つかりません")
        try:
            claims = jwt.decode(token, jwt.PyJWK.from_dict(matching[0]).key, algorithms=["RS256"],
                                audience=client_id, issuer=ISSUER,
                                options={"require": ["exp", "iat", "iss", "aud", "sub"]})
        except jwt.PyJWTError:
            raise ValueError("IDトークンを検証できません") from None
        if nonce is not None and claims.get("nonce") != nonce:
            raise ValueError("認証nonceが一致しません")
        if not isinstance(claims["sub"], str) or not claims["sub"]:
            raise ValueError("アカウントIDが不正です")
        return claims

    def _record(self, tokens, client_id, identity, previous=None):
        scopes = tokens.get("scope", " ".join((previous or {}).get("scopes", []))).split()
        if "chatgpt.tokens.use.direct" not in scopes:
            raise PermissionError("ChatGPTプランの使用許可がありません。ブラウザで再認証してください")
        if str(tokens.get("token_type", "")).lower() != "bearer" or not tokens.get("access_token"):
            raise ValueError("アクセストークンが不正です")
        record = {**(previous or {}), "client_id": client_id, "subject": identity["sub"],
                  "email": identity.get("email", ""), "scopes": scopes,
                  "access_token": tokens["access_token"], "expires_at": time.time() + int(tokens["expires_in"])}
        for name in ("id_token", "refresh_token"):
            if tokens.get(name):
                record[name] = tokens[name]
        if not record.get("refresh_token"):
            raise ValueError("更新トークンがありません")
        return record

    def sign_in(self, cancel, registration_id=None, open_browser=webbrowser.open):
        # 既存アカウントは検証完了まで変更しない。認証URL・トークンをログへ出さない。
        with self._transaction():
            accounts = self._load()
            registration = accounts.get(registration_id) if registration_id else None
            if registration_id and registration is None:
                raise ValueError("選択した登録がありません")
            host = self.store.get("oauth:host")
            if not host:
                host = "urn:uuid:" + str(uuid.uuid4())
                self.store.set("oauth:host", host)
        attempt, result = OAuthAttempt(host, registration), {}
        class Handler(BaseHTTPRequestHandler):
            def setup(self):
                super().setup()
                self.connection.settimeout(2)

            def do_GET(self):
                p = urllib.parse.urlsplit(self.path)
                if p.path != "/auth/callback" or len(self.path) > 16000:
                    self.send_error(404)
                    return
                if self.headers.get("Host") != f"127.0.0.1:{self.server.server_port}":
                    self.send_error(400)
                    return
                try:
                    code, client_id = attempt.callback(urllib.parse.parse_qs(p.query, keep_blank_values=True))
                    result.update(code=code, client_id=client_id)
                    status, message = 200, b"Hashi: Sign-in received. You can close this window."
                except (ValueError, PermissionError) as exc:
                    # 状態違いの雑音は試行を消費しない。正しいstateの拒否は終了する。
                    query = urllib.parse.parse_qs(p.query)
                    if query.get("state") == [attempt.state]:
                        result["error"] = exc
                    status, message = 400, b"Hashi: Sign-in was rejected."
                self.send_response(status)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(message)

            def log_message(self, *args):
                return  # URLには認証コードが含まれるため標準HTTPログを無効化
        with HTTPServer(("127.0.0.1", 0), Handler) as server:
            server.timeout = .2
            redirect = f"http://127.0.0.1:{server.server_port}/auth/callback"
            if not open_browser(attempt.authorization_url(redirect)):
                raise RuntimeError("システムブラウザを開けませんでした")
            deadline = time.monotonic() + 180
            while not result and not cancel.is_set() and time.monotonic() < deadline:
                server.handle_request()
        if cancel.is_set():
            raise InterruptedError("認証を停止しました")
        if not result:
            raise TimeoutError("認証がタイムアウトしました")
        if "error" in result:
            raise result["error"]
        tokens = self.http.json(TOKEN, {"grant_type": "authorization_code", "client_id": result["client_id"],
                   "code": result["code"], "code_verifier": attempt.verifier, "redirect_uri": redirect,
                   "resource": RESOURCE}, form=True)
        identity = self._identity(tokens["id_token"], result["client_id"], attempt.nonce)
        if registration and identity["sub"] != registration["subject"]:
            raise ValueError("選択したChatGPTアカウントと一致しません")
        record = self._record(tokens, result["client_id"], identity, registration)
        if cancel.is_set():
            raise InterruptedError("認証を停止しました")
        with self._transaction():
            accounts = self._load()
            accounts[result["client_id"]] = record
            self._save(accounts)
        return result["client_id"]

    def access_token(self, account_id):
        with self._transaction():
            accounts = self._load()
            record = accounts.get(account_id)
            if not record or not record.get("refresh_token"):
                raise PermissionError("ChatGPTへ再ログインしてください")
            if "chatgpt.tokens.use.direct" not in record.get("scopes", []):
                raise PermissionError("ChatGPTプラン使用の許可がありません")
            if record["expires_at"] <= time.time() + 60:
                tokens = self.http.json(TOKEN, {"grant_type": "refresh_token", "client_id": record["client_id"],
                           "refresh_token": record["refresh_token"], "resource": RESOURCE}, form=True)
                identity = {"sub": record["subject"], "email": record.get("email", "")}
                if tokens.get("id_token"):
                    identity = self._identity(tokens["id_token"], record["client_id"])
                    if identity["sub"] != record["subject"]:
                        raise ValueError("更新後のアカウントが一致しません")
                record = self._record(tokens, record["client_id"], identity, record)
                accounts[account_id] = record
                self._save(accounts)
            return record["access_token"]

    def models(self, account_id):
        data = self.http.json(RESOURCE + "/models", headers={"Authorization": "Bearer " + self.access_token(account_id)})
        return [m for m in data.get("models", []) if m.get("visibility") == "list"]

    def sign_out(self, account_id):
        with self._transaction():
            accounts = self._load()
            record = accounts.get(account_id)
            if not record:
                return True
            revoked = True
            try:
                if record.get("refresh_token"):
                    endpoint = official_url(self._metadata()["revocation_endpoint"])
                    body = urllib.parse.urlencode({"token": record["refresh_token"], "token_type_hint": "refresh_token",
                                                  "client_id": record["client_id"]}).encode()
                    with self.http.open(endpoint, body, {"Content-Type": "application/x-www-form-urlencoded"}):
                        pass
            except Exception:
                revoked = False
            accounts[account_id] = {key: record[key] for key in ("client_id", "subject", "email") if key in record}
            self._save(accounts)
            return revoked


class ChatGptProvider(ApiProvider):
    def __init__(self, accounts, account_id, model):
        super().__init__("openai", model, "", http=accounts.http)
        self.accounts, self.account_id = accounts, account_id

    def _responses_payload(self, messages, instructions, tools):
        data = super()._responses_payload(messages, instructions, tools)
        data["tools"] = [{"type": "namespace", "name": "hashi", "description": "Hashi端末操作",
                          "tools": data["tools"]}]
        return data

    def respond(self, messages, instructions, tools, cancel, on_text):
        self._key = self.accounts.access_token(self.account_id)
        try:
            return super().respond(messages, instructions, tools, cancel, on_text)
        finally:
            self._key = ""
