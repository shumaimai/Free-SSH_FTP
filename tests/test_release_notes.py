"""tools/release_notes.py のテスト。"""
from tools.release_notes import extract

_SAMPLE = """# 変更履歴

## [Unreleased]

## [1.1.1] - 2026-08-02
一行サマリ。

### 追加
- 機能 A

## [1.1.0] - 2026-08-02
古い版。
"""


def test_extract_version_block():
    out = extract(_SAMPLE, "1.1.1")
    assert out.startswith("## [1.1.1]")
    assert "機能 A" in out
    assert "1.1.0" not in out


def test_extract_strips_v_prefix():
    out = extract(_SAMPLE, "v1.1.1")
    assert "1.1.1" in out
