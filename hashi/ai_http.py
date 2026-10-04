"""認証情報を転送・ログ出力しないJSON/SSE通信。自動POST再送なし。"""
from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request


def validate_base_url(url):
    url = url.strip()
    p = urllib.parse.urlsplit(url)
    if (not p.hostname or p.username or p.password or p.query or p.fragment
            or (p.scheme != "https" and not (p.scheme == "http" and p.hostname in
                                            {"127.0.0.1", "localhost", "::1"}))):
        raise ValueError("HTTPS、またはローカルHTTPのURLを指定してください")
    return url.rstrip("/")


def normalize_api_base(url):
    """APIの個別エンドポイントを貼り付けても二重に連結しない。"""
    base = validate_base_url(url)
    for suffix in ("/chat/completions", "/responses", "/messages", "/models"):
        if base.endswith(suffix):
            base = base[:-len(suffix)]
            break
    if base in {"https://api.commandcode.ai", "https://api.commandcode.ai/v1"}:
        base = "https://api.commandcode.ai/provider/v1"
    return base


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise RuntimeError("認証付き通信のリダイレクトは許可しません")


class JsonHttp:
    def __init__(self):
        self.opener = urllib.request.build_opener(NoRedirect())

    def open(self, url, data=None, headers=None):
        validate_base_url(url)
        try:
            return self.opener.open(urllib.request.Request(url, data=data,
                                    headers=headers or {}), timeout=15)
        except urllib.error.HTTPError as exc:
            code = exc.code
            exc.close()
            hint = {400: "モデル名とAPI形式を確認してください。ツール非対応なら端末ツールの利用を外してください",
                    401: "この接続先のAPIキーを確認してください",
                    403: "APIキーの権限と、このモデルを利用できる契約・残高を確認してください",
                    404: "APIのURLとモデル名を確認してください",
                    429: "利用上限です。時間を置いて確認してください"}.get(code, "接続設定を確認してください")
            raise RuntimeError(f"AI接続エラー HTTP {code}: {hint}") from None
        except (urllib.error.URLError, OSError):
            raise RuntimeError("AI接続に失敗しました。接続先とネットワークを確認してください") from None
        except (ValueError, TypeError):
            # urllibのヘッダー検証例外には、APIキーの値自体が含まれ得る。
            raise RuntimeError("APIキーと接続先の入力形式を確認してください。改行を含むキーは使えません") from None

    def json(self, url, payload=None, headers=None, *, form=False):
        data, headers = None, dict(headers or {})
        if payload is not None:
            data = (urllib.parse.urlencode(payload).encode() if form else json.dumps(payload).encode())
            headers["Content-Type"] = "application/x-www-form-urlencoded" if form else "application/json"
        with self.open(url, data, headers) as response:
            raw = response.read(2097153)
        if len(raw) > 2097152:
            raise ValueError("応答が大きすぎます")
        return json.loads(raw)

    def events(self, url, payload, headers, cancel):
        headers = {**headers, "Content-Type": "application/json", "Accept": "text/event-stream"}
        if cancel.is_set():
            raise InterruptedError("AI処理を停止しました")
        with self.open(url, json.dumps(payload).encode(), headers) as response:
            parts, total = [], 0
            while True:
                if cancel.is_set():
                    raise InterruptedError("AI処理を停止しました")
                raw = response.readline(65537)
                if not raw:
                    return
                total += len(raw)
                if len(raw) > 65536 or total > 4194304:
                    raise ValueError("AIストリームが大きすぎます")
                line = raw.decode("utf-8").rstrip("\r\n")
                if line.startswith("data:"):
                    parts.append(line[5:].lstrip())
                elif not line and parts:
                    data = "\n".join(parts)
                    parts.clear()
                    if data == "[DONE]":
                        yield {"type": "done"}
                        return
                    yield json.loads(data)
