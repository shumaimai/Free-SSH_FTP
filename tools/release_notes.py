"""CHANGELOG.md から指定バージョンのリリースノートだけを抜き出す。

Release ワークフローは生成物を GitHub Release の本文に載せる。
CHANGELOG 全文を貼ると過去バージョンまで全部出てしまうため。
"""
from __future__ import annotations

import re
import sys

_HEADING = re.compile(r"^## \[(?P<ver>[^\]]+)\]", re.MULTILINE)


def extract(changelog: str, version: str) -> str:
    """``## [version]`` から次の ``## [`` 直前までを返す。"""
    version = version.lstrip("v")
    matches = list(_HEADING.finditer(changelog))
    for i, m in enumerate(matches):
        if m.group("ver").split()[0] == version:
            start = m.start()
            end = matches[i + 1].start() if i + 1 < len(matches) else len(changelog)
            block = changelog[start:end].strip()
            if block:
                return block
    raise SystemExit(f"CHANGELOG に [{version}] セクションが見つかりません")


def main(argv: list[str] | None = None) -> None:
    args = argv if argv is not None else sys.argv[1:]
    if len(args) != 2:
        raise SystemExit(f"使い方: {sys.argv[0]} CHANGELOG.md VERSION")
    path, version = args
    with open(path, encoding="utf-8") as f:
        text = f.read()
    print(extract(text, version))


if __name__ == "__main__":
    main()
