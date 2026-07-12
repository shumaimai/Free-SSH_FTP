"""P2P で接続情報を送り合う(Issue #43)。

サーバーを介さず、同じ LAN 内(または相手の IP を知っている同士)で Hashi 同士が
#42 のバンドルを直接転送する。

**安全性の考え方(SAS 認証つき ECDH)**:

- 双方が使い捨ての X25519 公開鍵を交換し、ECDH で共有鍵を作る。
- 共有鍵と両公開鍵から **6 桁の確認コード(SAS)** を導出し、両者が口頭など別経路で
  照合する。中間者がいると各区間で別の鍵になり **SAS が一致しない**ので気づける
  (照合を省くと中間者を防げない = TOFU と同じ「人間が確認する」思想)。
- バンドルは共有鍵由来の Fernet で暗号化して送る。
- 秘密情報(パスワード等)は #42 と同じくバンドル側でパスフレーズ暗号化されている
  ときだけ含まれる。P2P の転送暗号とは別レイヤ。

`Session` は 1 本の接続済みソケットを受け取り、ハンドシェイク → SAS 提示 →
(呼び出し側が SAS 照合)→ 送信 or 受信、という手順で使う。ソケットの受け付け/
接続自体は呼び出し側(GUI)が行う。
"""
from __future__ import annotations

import base64
import hashlib
import logging
import socket
import struct

logger = logging.getLogger(__name__)

MAGIC = b"HASHI-P2P\x00"
VERSION = 1
DEFAULT_PORT = 53517
_MAX_PAYLOAD = 8 * 1024 * 1024   # 8MB 上限(接続情報にしては十分。DoS 抑止)
_SAS_DIGITS = 6


class P2PError(Exception):
    """P2P 転送の失敗(メッセージはそのまま表示できる日本語)。"""


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise P2PError("接続が相手から切断されました。")
        buf += chunk
    return buf


def _send_frame(sock: socket.socket, data: bytes) -> None:
    sock.sendall(struct.pack(">I", len(data)) + data)


def _recv_frame(sock: socket.socket, limit: int = _MAX_PAYLOAD) -> bytes:
    (n,) = struct.unpack(">I", _recv_exact(sock, 4))
    if n > limit:
        raise P2PError("受信データが大きすぎます(中止しました)。")
    return _recv_exact(sock, n)


def compute_sas(pub_a: bytes, pub_b: bytes, shared: bytes) -> str:
    """両公開鍵(順序非依存)と共有鍵から 6 桁の確認コードを導出する。"""
    lo, hi = sorted((pub_a, pub_b))
    digest = hashlib.sha256(MAGIC + lo + hi + shared).digest()
    num = int.from_bytes(digest[:8], "big") % (10 ** _SAS_DIGITS)
    return str(num).zfill(_SAS_DIGITS)


def _derive_fernet_key(shared: bytes, pub_a: bytes, pub_b: bytes) -> bytes:
    lo, hi = sorted((pub_a, pub_b))
    key = hashlib.sha256(b"hashi-p2p-key" + lo + hi + shared).digest()
    return base64.urlsafe_b64encode(key)


class Session:
    """接続済みソケット 1 本の上で行う P2P セッション。"""

    def __init__(self, sock: socket.socket):
        self.sock = sock
        self.sas: str | None = None
        self._fernet = None
        self._peer_pub = None
        self._my_pub = None

    def handshake(self) -> str:
        """鍵交換を行い、確認コード(SAS)を返す。転送前に必ず呼ぶ。"""
        from cryptography.hazmat.primitives.asymmetric.x25519 import (
            X25519PrivateKey,
            X25519PublicKey,
        )
        from cryptography.hazmat.primitives.serialization import (
            Encoding,
            PublicFormat,
        )

        priv = X25519PrivateKey.generate()
        my_pub = priv.public_key().public_bytes(
            Encoding.Raw, PublicFormat.Raw)
        self._my_pub = my_pub

        # ハロー(magic + version + 公開鍵)を送受信
        _send_frame(self.sock, MAGIC + bytes([VERSION]) + my_pub)
        hello = _recv_frame(self.sock, limit=1024)
        if not hello.startswith(MAGIC):
            raise P2PError("相手が Hashi の P2P ではありません。")
        rest = hello[len(MAGIC):]
        if not rest or rest[0] != VERSION:
            raise P2PError("相手の P2P バージョンが違います。")
        peer_pub = rest[1:]
        if len(peer_pub) != 32:
            raise P2PError("相手の公開鍵が不正です。")
        self._peer_pub = peer_pub

        shared = priv.exchange(X25519PublicKey.from_public_bytes(peer_pub))
        self.sas = compute_sas(my_pub, peer_pub, shared)
        from cryptography.fernet import Fernet
        self._fernet = Fernet(_derive_fernet_key(shared, my_pub, peer_pub))
        return self.sas

    def send_payload(self, payload: bytes) -> None:
        """ハンドシェイク後、暗号化してバンドルを送る。相手の ack を待つ。"""
        if self._fernet is None:
            raise P2PError("先に handshake() を呼んでください。")
        _send_frame(self.sock, self._fernet.encrypt(payload))
        ack = _recv_frame(self.sock, limit=64)
        if ack != b"OK":
            raise P2PError("相手が受信を確認しませんでした。")

    def receive_payload(self) -> bytes:
        """ハンドシェイク後、暗号化されたバンドルを受け取り復号する。"""
        if self._fernet is None:
            raise P2PError("先に handshake() を呼んでください。")
        from cryptography.fernet import InvalidToken
        token = _recv_frame(self.sock)
        try:
            payload = self._fernet.decrypt(token)
        except InvalidToken as e:
            raise P2PError(
                "受信データを復号できませんでした"
                "(確認コードが一致しない相手かもしれません)。") from e
        _send_frame(self.sock, b"OK")
        return payload

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            logger.debug("P2P ソケットのクローズに失敗 (無視)", exc_info=True)


def listen(host: str = "0.0.0.0", port: int = DEFAULT_PORT,
           timeout: float | None = None) -> socket.socket:
    """受信待ちのリッスンソケットを作る(呼び出し側が accept する)。

    常時リッスンはしない設計。受信する瞬間だけ開き、終わったら閉じる。
    """
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(1)
    if timeout is not None:
        srv.settimeout(timeout)
    return srv


def connect(host: str, port: int = DEFAULT_PORT,
            timeout: float = 15.0) -> socket.socket:
    """送信側: 相手のリッスンへ接続したソケットを返す。"""
    try:
        return socket.create_connection((host, port), timeout=timeout)
    except OSError as e:
        raise P2PError(f"{host}:{port} に接続できません ({e})") from e
