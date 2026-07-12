"""sshd 設定の堅牢化(Issue #12): パスワードログインの無効化 / ポート変更。

sshd_config を直接いじって SSH を締め出す事故が一番怖いので、次の順で守る:

1. **鍵ログインの事前検証**: パスワードログインを切る前に「登録済みの鍵で本当に
   入れるか」を別接続で確認する。入れないなら切らせない。
2. **バックアップ**: 変更前に sshd の設定を root 権限でタイムスタンプ付き退避。
3. **構文検証**: 書き込んだ後 `sshd -t` で構文を検証し、NG なら適用(reload)しない。
4. **ドロップイン優先**: `Include /etc/ssh/sshd_config.d/*.conf` があればメイン設定を
   触らずドロップインファイルだけ置く(復旧が容易。消せば元に戻る)。
5. **到達確認**: reload 後、呼び出し側が新しい接続で疎通を確かめる(このモジュールは
   検証コールバックを受け取るだけ。実接続は GUI/呼び出し側)。

GUI 非依存。session は SshSession 互換(exec_command / run_sudo / open_sftp)。
"""
from __future__ import annotations

import logging
import posixpath
import re
import time

logger = logging.getLogger(__name__)

DROPIN_PATH = "/etc/ssh/sshd_config.d/00-hashi.conf"
MAIN_CONFIG = "/etc/ssh/sshd_config"
_INCLUDE_RE = re.compile(r"^\s*Include\s+/etc/ssh/sshd_config\.d/\*\.conf",
                         re.IGNORECASE | re.MULTILINE)


class SshdAdminError(Exception):
    """sshd 設定変更の失敗(メッセージはそのまま表示できる日本語)。"""


def read_effective(session) -> dict:
    """`sshd -T` で現在の実効設定を読む。returns {"port": [..], "passwordauthentication": ".."}。

    sshd -T は root 権限が要ることがあるので sudo で叩く。
    """
    rc, out, err = session.run_sudo("sshd -T", _sudo_pw(session))
    if rc != 0:
        raise SshdAdminError(
            "現在の sshd 設定を取得できませんでした"
            f"(sshd -T が失敗)。{err.strip() or out.strip()}")
    ports: list[int] = []
    passwordauth = None
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) != 2:
            continue
        key, val = parts[0].lower(), parts[1].strip()
        if key == "port":
            try:
                ports.append(int(val))
            except ValueError:
                pass
        elif key == "passwordauthentication":
            passwordauth = val.lower()
    return {"port": ports or [22], "passwordauthentication": passwordauth}


def _sudo_pw(session):
    # SshSession には sudo パスワードの保持機構がないので、呼び出し側が
    # session._hashi_sudo_pw に積んでおく取り決め(GUI 層が設定する)。
    return getattr(session, "_hashi_sudo_pw", None)


def supports_dropin(session) -> bool:
    """メイン設定に sshd_config.d の Include があるか(=ドロップインが効くか)。"""
    rc, out, _ = session.run_sudo(f"cat {MAIN_CONFIG}", _sudo_pw(session))
    if rc != 0:
        return False
    return bool(_INCLUDE_RE.search(out))


def build_dropin(disable_password: bool | None, new_port: int | None) -> str:
    """ドロップインファイルの内容を組み立てる。"""
    lines = ["# Managed by Hashi (Issue #12). 消せば元の設定に戻ります。"]
    if new_port is not None:
        lines.append(f"Port {new_port}")
    if disable_password is True:
        lines.append("PasswordAuthentication no")
        lines.append("KbdInteractiveAuthentication no")
    elif disable_password is False:
        lines.append("PasswordAuthentication yes")
    return "\n".join(lines) + "\n"


def backup_config(session, path: str = MAIN_CONFIG) -> str:
    """設定ファイルを root 権限でタイムスタンプ付き退避。バックアップ先パスを返す。"""
    ts = time.strftime("%Y%m%d-%H%M%S")
    dest = f"{path}.hashi-bak-{ts}"
    rc, _out, err = session.run_sudo(f"cp -a {path} {dest}", _sudo_pw(session))
    if rc != 0:
        raise SshdAdminError(f"設定のバックアップに失敗しました: {err.strip()}")
    return dest


def _validate(session) -> None:
    rc, out, err = session.run_sudo("sshd -t", _sudo_pw(session))
    if rc != 0:
        raise SshdAdminError(
            "変更後の sshd 設定が構文エラーです(適用を中止しました)。\n"
            f"{err.strip() or out.strip()}")


def _write_dropin(session, content: str) -> None:
    """SFTP でホームに一時ファイルを書き、root 権限で配置する。

    設定内容を sudo の標準入力に通さないので、NOPASSWD sudo でパスワード行が
    ファイルに混入する事故が起きない。一時ファイルは必ず消す。
    """
    rc, home, _ = session.exec_command('printf "%s" "$HOME"')
    home = home.strip()
    if rc != 0 or not home.startswith("/"):
        raise SshdAdminError("ホームディレクトリを取得できませんでした。")
    tmp = posixpath.join(home, ".hashi-sshd-dropin.tmp")
    sftp = session.open_sftp()
    try:
        with sftp.open(tmp, "wb") as f:
            f.write(content.encode("utf-8"))
    except OSError as e:
        raise SshdAdminError(f"一時ファイルの書き込みに失敗しました: {e}") from e
    finally:
        try:
            sftp.close()
        except Exception:
            logger.debug("SFTP クローズに失敗 (無視)", exc_info=True)
    try:
        rc, _out, err = session.run_sudo(
            f"install -o root -g root -m 644 {tmp} {DROPIN_PATH}",
            _sudo_pw(session))
        if rc != 0:
            raise SshdAdminError(f"設定ファイルの配置に失敗しました: {err.strip()}")
    finally:
        session.run_sudo(f"rm -f {tmp}", _sudo_pw(session))


def _reload(session) -> None:
    # reload は既存接続を切らない(restart と違い安全側)。systemd 環境では
    # systemctl、それ以外(コンテナ等)では sshd マスターへ HUP を送る。
    # || チェーン全体を sudo 下で動かすため sh -c でまとめる(sudo は先頭
    # コマンドにしか効かないので、素の連結だと HUP が非 root で失敗する)。
    # HUP は接続ごとの子 sshd ではなくマスター(最古 = -o)だけに送る。
    # -x sshd で全 sshd に送ると自分の接続まで切れてしまう。
    rc, _out, err = session.run_sudo(
        "sh -c 'systemctl reload ssh 2>/dev/null "
        "|| systemctl reload sshd 2>/dev/null "
        "|| pkill -HUP -o -x sshd'", _sudo_pw(session))
    if rc != 0:
        raise SshdAdminError(f"sshd の再読み込みに失敗しました: {err.strip()}")


def apply_changes(session, *, disable_password: bool | None = None,
                  new_port: int | None = None,
                  verify_key_login=None, verify_reachable=None) -> dict:
    """sshd 設定を安全に変更する。

    - disable_password=True の場合、`verify_key_login()` が True を返さないと中止
      (鍵で入れないのにパスワードを切ると締め出される)。
    - ドロップイン方式のみ対応(Include がない古い環境は明示エラー)。
    - `verify_reachable(port)` があれば reload 後に呼び、False なら
      バックアップから戻して(ドロップイン削除)エラーにする。
    returns {"backup": path, "dropin": path, "reloaded": True}。
    """
    if disable_password is None and new_port is None:
        raise SshdAdminError("変更内容がありません。")
    if new_port is not None and not (1 <= new_port <= 65535):
        raise SshdAdminError(f"ポート番号が不正です: {new_port}")

    if disable_password is True:
        if verify_key_login is None or not verify_key_login():
            raise SshdAdminError(
                "登録済みの鍵でログインできることを確認できませんでした。\n"
                "鍵ログインが確実にできる状態になるまで、パスワード認証は"
                "無効化しません(締め出し防止)。")

    if not supports_dropin(session):
        raise SshdAdminError(
            "この環境は sshd_config.d のドロップインに未対応です"
            f"({MAIN_CONFIG} に Include 行がありません)。"
            "安全に自動編集できないため中止しました。")

    backup = backup_config(session)
    content = build_dropin(disable_password, new_port)
    _write_dropin(session, content)
    try:
        _validate(session)
    except SshdAdminError:
        _remove_dropin(session)
        raise
    _reload(session)

    if verify_reachable is not None:
        port = new_port if new_port is not None else None
        if not verify_reachable(port):
            _remove_dropin(session)
            _reload(session)
            raise SshdAdminError(
                "変更後の疎通確認に失敗しました。設定を元に戻しました"
                "(ドロップインを削除)。ファイアウォールで新ポートが"
                "塞がれている可能性があります。")
    return {"backup": backup, "dropin": DROPIN_PATH, "reloaded": True}


def _remove_dropin(session) -> None:
    session.run_sudo(f"rm -f {DROPIN_PATH}", _sudo_pw(session))
