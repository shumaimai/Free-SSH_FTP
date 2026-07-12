"""P2P 共有(Issue #43)のテスト。

暗号ヘルパーの性質(SAS 決定性・中間者で不一致)と、実ループバックでの
往復転送を検証する。バンドルの中身は portability のテストで担保済みなので、
ここでは「同じバイト列が安全に届くこと」に集中する。
"""
import socket
import threading

import pytest

from hashi import p2p
from hashi.config import KnownHosts, Profile, ProfileStore
from hashi.portability import dumps_bundle, loads_bundle, merge_bundle


def _keypair():
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    from cryptography.hazmat.primitives.serialization import (
        Encoding,
        PublicFormat,
    )
    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    return priv, pub


def test_sas_is_order_independent_and_deterministic():
    _a, pa = _keypair()
    _b, pb = _keypair()
    shared = b"x" * 32
    s1 = p2p.compute_sas(pa, pb, shared)
    s2 = p2p.compute_sas(pb, pa, shared)   # 順序を入れ替えても同じ
    assert s1 == s2
    assert len(s1) == 6 and s1.isdigit()


def test_sas_differs_for_mitm():
    """中間者がいると両区間で共有鍵が変わり SAS が一致しない。"""
    a_priv, a_pub = _keypair()
    b_priv, b_pub = _keypair()
    m_priv, m_pub = _keypair()
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PublicKey

    # A ↔ M 区間 と M ↔ B 区間
    a_m = a_priv.exchange(X25519PublicKey.from_public_bytes(m_pub))
    m_b = m_priv.exchange(X25519PublicKey.from_public_bytes(b_pub))
    sas_a_side = p2p.compute_sas(a_pub, m_pub, a_m)   # A が見る SAS
    sas_b_side = p2p.compute_sas(m_pub, b_pub, m_b)   # B が見る SAS
    assert sas_a_side != sas_b_side


def _run_pair(payload):
    """ループバックで送受信を並走させ、(受信データ, 送信SAS, 受信SAS) を返す。"""
    srv = p2p.listen("127.0.0.1", 0, timeout=5)
    port = srv.getsockname()[1]
    result = {}

    def receiver():
        conn, _ = srv.accept()
        srv.close()
        sess = p2p.Session(conn)
        result["recv_sas"] = sess.handshake()
        result["data"] = sess.receive_payload()
        sess.close()

    t = threading.Thread(target=receiver)
    t.start()
    sock = p2p.connect("127.0.0.1", port, timeout=5)
    sess = p2p.Session(sock)
    result["send_sas"] = sess.handshake()
    sess.send_payload(payload)
    sess.close()
    t.join(timeout=5)
    return result


def test_loopback_roundtrip_matches_and_sas_agree():
    payload = b"hello-bundle-\xf0\x9f\x94\x91" * 100
    r = _run_pair(payload)
    assert r["data"] == payload
    assert r["send_sas"] == r["recv_sas"]   # 中間者なしなので一致


def test_loopback_transfers_real_bundle(tmp_path):
    kh = KnownHosts(path=tmp_path / "kh.json")
    p = Profile(name="srv", host="203.0.113.5", username="deploy",
                proxy_jump="ops@bastion")
    kh.remember(p.host, p.port, "ssh-ed25519", "SHA256:abc")
    payload = dumps_bundle([p], kh)

    r = _run_pair(payload)
    bundle = loads_bundle(r["data"])
    store = ProfileStore(path=tmp_path / "profiles.json")
    dst_kh = KnownHosts(path=tmp_path / "dst_kh.json")
    counts = merge_bundle(bundle, store, dst_kh)
    assert counts["added"] == 1
    assert store.profiles[0].proxy_jump == "ops@bastion"
    assert counts["hosts_added"] == 1


def test_receive_rejects_non_hashi_peer():
    srv = p2p.listen("127.0.0.1", 0, timeout=5)
    port = srv.getsockname()[1]
    err = {}

    def receiver():
        conn, _ = srv.accept()
        srv.close()
        try:
            p2p.Session(conn).handshake()
        except p2p.P2PError as e:
            err["msg"] = str(e)
        conn.close()

    t = threading.Thread(target=receiver)
    t.start()
    sock = socket.create_connection(("127.0.0.1", port), timeout=5)
    # magic を送らずにゴミを送る
    import struct
    junk = b"not-hashi-protocol!!"
    sock.sendall(struct.pack(">I", len(junk)) + junk)
    t.join(timeout=5)
    sock.close()
    assert "Hashi" in err.get("msg", "")


def test_send_before_handshake_errors():
    a, b = socket.socketpair()
    sess = p2p.Session(a)
    with pytest.raises(p2p.P2PError, match="handshake"):
        sess.send_payload(b"x")
    a.close()
    b.close()
