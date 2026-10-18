"""Rust側の日本語メッセージの翻訳台帳(画面の i18n/rust/*.json)を検査する。

    python tools/rust_messages.py check            台帳の抜け・余り・書式の違いがあれば失敗する
    python tools/rust_messages.py list [--missing]  Rustの日本語リテラルを JSON で出す(--missing は台帳に無いものだけ)

Rust側のエラー文などは日本語の文字列として画面へ来る。画面は `tr()`(frontend/src/i18n.ts)で台帳を引いて英語にする。
キーは Rust の文字列リテラルをアンエスケープした文(`{}` `{name}` `{:?}` は部分一致の穴)。値は英訳で、穴は出てくる順に
`{1}` `{2}`…と書く。画面に出ない文字列(AIへの指示文・ログなど)は値を null にして「訳さない」と明示する。
テスト用のコード(`#[cfg(test)]`・tests/)と、配布しない候補(hashi-desktop-native)・診断CLI(hashi-ssh-check)は対象外。
"""
import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ["hashi-core", "hashi-ai", "hashi-storage", "hashi-terminal", "hashi-mcp", "hashi-desktop-tauri"]
EXCLUDE_FILES = {"hashi-ssh-check.rs"}
CATALOG_DIR = ROOT / "rust/crates/hashi-desktop-tauri/frontend/src/i18n/rust"
JA = re.compile("[぀-ヿ㐀-鿿＀-￯]")
HOLE = re.compile(r"\{[^{}]*\}")


def unescape(raw: str) -> str:
    out = []
    i = 0
    while i < len(raw):
        c = raw[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = raw[i + 1]
        if n == "\n" or n == "\r":  # 行継続: 改行と次の行頭の空白を捨てる
            i += 2
            while i < len(raw) and raw[i] in " \t\r\n":
                i += 1
        elif n == "u":
            end = raw.index("}", i)
            out.append(chr(int(raw[i + 3 : end], 16)))
            i = end + 1
        elif n == "x":
            out.append(chr(int(raw[i + 2 : i + 4], 16)))
            i += 4
        else:
            out.append({"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}[n])
            i += 2
    return "".join(out)


def literals(src: str):
    """(行, 文字列) を返す。コメントと `#[cfg(test)]` の塊は飛ばす。"""
    i, n, line = 0, len(src), 1
    out = []
    depth_skip = None  # テスト塊を飛ばしている間の波括弧の深さ
    depth = 0
    pending_test = False
    raw_start = re.compile(r'b?r(#*)"')
    char_lit = re.compile(r"'(\\.[^']*|[^\\'])'")
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            d = 1
            i += 2
            while i < n and d:
                if src.startswith("/*", i):
                    d += 1
                    i += 2
                elif src.startswith("*/", i):
                    d -= 1
                    i += 2
                else:
                    line += src[i] == "\n"
                    i += 1
            continue
        if src.startswith("#[cfg(test)]", i):
            pending_test = True
            i += len("#[cfg(test)]")
            continue
        m = raw_start.match(src, i)
        if m and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            h = m.group(1)
            end = src.find('"' + h, m.end())
            text = src[m.end() : end]
            if depth_skip is None and JA.search(text):
                out.append((line, text))
            line += src.count("\n", i, end)
            i = end + len(h) + 1
            continue
        if c == '"' or (c == "b" and src.startswith('b"', i)):
            if c == "b":
                i += 1
            j = i + 1
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            text = src[i + 1 : j]
            if depth_skip is None and JA.search(text):
                out.append((line, unescape(text)))
            line += src.count("\n", i, j)
            i = j + 1
            continue
        if c == "'":
            m = char_lit.match(src, i)
            if m:
                i = m.end()
                continue
        if c == "{":
            if pending_test and depth_skip is None:
                depth_skip = depth
                pending_test = False
            depth += 1
        elif c == "}":
            depth -= 1
            if depth_skip is not None and depth == depth_skip:
                depth_skip = None
        elif c == ";" and pending_test:
            pending_test = False
        i += 1
    return out


def source_literals():
    found = {}
    for crate in CRATES:
        base = ROOT / "rust/crates" / crate / "src"
        for path in sorted(base.rglob("*.rs")):
            if path.name in EXCLUDE_FILES or "tests" in path.relative_to(base).parts:
                continue
            for line, text in literals(path.read_text(encoding="utf-8")):
                found.setdefault(text, []).append(f"{path.relative_to(ROOT)}:{line}")
    return found


def catalog():
    entries, errors = {}, []
    for path in sorted(CATALOG_DIR.glob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        for key, value in data.items():
            if key in entries:
                errors.append(f"{path.name}: キーが重複しています: {key[:40]!r}")
            entries[key] = value
    return entries, errors


def holes(text: str) -> int:
    return len(HOLE.findall(text.replace("{{", "").replace("}}", "")))


def check() -> int:
    found = source_literals()
    entries, errors = catalog()
    for key, where in found.items():
        if key not in entries:
            errors.append(f"台帳にありません: {key[:50]!r} ({where[0]})")
    for key, value in entries.items():
        if key not in found:
            errors.append(f"Rustに無い(余り)キー: {key[:50]!r}")
        if value is None:
            continue
        if not isinstance(value, str):
            errors.append(f"値が文字列でも null でもありません: {key[:50]!r}")
            continue
        if JA.search(value):
            errors.append(f"訳に日本語が残っています: {key[:50]!r}")
        k = holes(key)
        used = sorted(int(x) for x in re.findall(r"\{(\d+)\}", value))
        if sorted(set(used)) != list(range(1, k + 1)):
            errors.append(f"穴の数が合いません(原文 {k} 個。訳は {{1}}..{{{k}}} を使う): {key[:50]!r}")
    if errors:
        print(f"rust_messages: {len(errors)} 件", file=sys.stderr)
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"rust_messages: ok ({len(found)} 文言)")
    return 0


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    lst = sub.add_parser("list")
    lst.add_argument("--missing", action="store_true")
    lst.add_argument("--crate")
    args = parser.parse_args()
    if args.cmd == "check":
        return check()
    found = source_literals()
    entries, _ = catalog()
    rows = [
        {"text": text, "where": where}
        for text, where in found.items()
        if (not args.missing or text not in entries) and (not args.crate or any(f"crates/{args.crate}/" in w for w in where))
    ]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
