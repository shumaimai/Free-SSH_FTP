"""起動モードの判定とエディタ単体起動。

Windows で「プログラムから選択」して Hashi.exe にファイルを関連付けたとき、
引数にファイルパスだけが渡される。このモジュールがそれを検出し、
メインウィンドウ(SSH/SFTP)を出さず内蔵エディタだけを起動する。

    Hashi.exe                    … 通常起動(接続 UI)
    Hashi.exe memo.txt           … エディタのみ
    Hashi.exe a.txt b.py         … 複数ファイルをそれぞれ開く
    Hashi.exe --editor           … エディタのみ(無題)
    Hashi.exe --editor memo.txt  … 明示的にエディタモード
"""
from __future__ import annotations

import argparse

from PySide6.QtWidgets import QApplication, QMessageBox

from hashi import __version__


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="Hashi",
        description="Hashi — SSH / SFTP クライアント / 内蔵テキストエディタ",
    )
    p.add_argument(
        "files",
        nargs="*",
        metavar="FILE",
        help="内蔵エディタで開くファイル(指定時はエディタのみ起動)",
    )
    p.add_argument(
        "--editor", "-e",
        action="store_true",
        help="内蔵エディタのみ起動(ファイル未指定時は無題の新規)",
    )
    p.add_argument(
        "--version", "-v",
        action="version",
        version=f"Hashi {__version__}",
    )
    return p


def parse_cli(argv: list[str] | None = None) -> argparse.Namespace:
    """コマンドライン引数を解析する(QApplication より前に呼ぶこと)。"""
    if argv is None:
        return build_parser().parse_args()
    # 先頭のプログラム名は argparse の位置引数に入れない
    return build_parser().parse_args(argv[1:])


def is_editor_mode(ns: argparse.Namespace) -> bool:
    """エディタ単体起動か(ファイル引数または --editor)。"""
    return bool(ns.editor or ns.files)


def normalize_paths(paths: list[str]) -> list[str]:
    """関連付け起動で付く余分な引用符などを除去する。"""
    out: list[str] = []
    for raw in paths:
        if not raw:
            continue
        p = raw.strip().strip('"').strip("'")
        if p:
            out.append(p)
    return out


def run_editor_only(
    app: QApplication,
    paths: list[str],
    *,
    new_if_empty: bool,
) -> int:
    """メインウィンドウなしで内蔵エディタだけを起動する。"""
    from hashi.config import Settings
    from hashi.editor import LocalEditorHub

    settings = Settings()
    hub = LocalEditorHub(settings)
    opened = 0
    failed: list[str] = []

    for path in paths:
        if hub.open_path(path, force=True) is not None:
            opened += 1
        else:
            failed.append(path)

    if opened == 0 and new_if_empty:
        hub.new_file()
        opened = 1

    if opened == 0:
        msg = "ファイルを開けませんでした。"
        if failed:
            shown = "\n".join(failed[:8])
            if len(failed) > 8:
                shown += f"\n… ほか {len(failed) - 8} 件"
            msg += f"\n\n{shown}"
        QMessageBox.warning(None, "Hashi エディタ", msg)
        return 1

    if failed:
        shown = "\n".join(failed[:8])
        if len(failed) > 8:
            shown += f"\n… ほか {len(failed) - 8} 件"
        QMessageBox.warning(
            None, "Hashi エディタ",
            f"一部のファイルを開けませんでした:\n\n{shown}")

    hub.all_closed.connect(app.quit)
    return app.exec()


def dispatch(argv: list[str] | None = None) -> tuple[str, argparse.Namespace]:
    """起動モードと解析結果を返す。mode は ``"app"`` または ``"editor"``。"""
    ns = parse_cli(argv)
    if is_editor_mode(ns):
        return "editor", ns
    return "app", ns
