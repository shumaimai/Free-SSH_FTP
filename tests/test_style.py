"""UI 共通スタイル(Issue #87)のテスト。"""
import re

from hashi import style


def test_palette_matches_fusion_theme():
    """style.py のパレットは main.py の Fusion ダークと一致させる決まり。"""
    import inspect

    import main
    src = inspect.getsource(main)
    for const in (style.BG, style.BG_BASE, style.BG_RAISED, style.FG,
                  style.FG_DISABLED, style.ACCENT):
        assert const in src, f"{const} が main.py のテーマに見当たらない"


def test_colors_are_hex():
    for name in ("BG", "BG_BASE", "BG_RAISED", "FG", "FG_MUTED",
                 "FG_DISABLED", "ACCENT", "BORDER", "WARN", "ERROR", "OK"):
        assert re.fullmatch(r"#[0-9a-f]{6}", getattr(style, name), re.I), name


def test_warning_and_muted_labels(qapp):
    w = style.warning_label("危険な操作です")
    assert w.text().startswith("⚠")
    assert style.WARN in w.styleSheet()
    assert w.wordWrap()

    m = style.muted_label("補足です")
    assert style.FG_MUTED in m.styleSheet()
    assert style.FG_MUTED in style.muted_span("x")


def test_dialog_sizes_are_three_tiers():
    assert style.DIALOG_S < style.DIALOG_M < style.DIALOG_L
    assert style.SPACING == 8
