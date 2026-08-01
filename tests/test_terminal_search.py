"""ターミナルのスクロールバック検索(Issue #79)のテスト。"""
import pytest


@pytest.fixture()
def term(qapp):
    from hashi.terminal import TerminalWidget
    t = TerminalWidget()
    t.show()
    yield t
    t.close()


def _feed_lines(t, n: int, fmt: str = "line-{:04d}"):
    for i in range(n):
        t._on_data((fmt.format(i) + "\r\n").encode())


def test_all_lines_covers_history_and_screen(term):
    """履歴上部 + 可視バッファ + 履歴下部が文書順で拾える。"""
    _feed_lines(term, 60)          # 24 行画面 → 前半は履歴へ流れる
    lines = term._all_lines()
    text = "\n".join(lines)
    assert "line-0000" in text     # 履歴の先頭
    assert "line-0059" in text     # 最新
    # 文書順(先頭が古い)
    idx_old = next(i for i, ln in enumerate(lines) if "line-0000" in ln)
    idx_new = next(i for i, ln in enumerate(lines) if "line-0059" in ln)
    assert idx_old < idx_new


def test_find_scrolls_back_to_hit_and_wraps(term):
    """上方向の検索で履歴のヒットまでスクロールし、端で回り込む(#79)。"""
    _feed_lines(term, 120)
    term._search_query = "line-0000"
    assert term.find_in_scrollback(backward=True) is True
    # ヒット行が可視範囲に入っている
    t = len(term.screen.history.top)
    assert t <= term._search_pos < t + term._rows
    visible = [term._row_text(term.screen.buffer[y])
               for y in range(term.screen.lines)]
    assert any("line-0000" in ln for ln in visible)

    # もう上に無い → 回り込んで同じ(唯一の)ヒットに留まる
    pos = term._search_pos
    assert term.find_in_scrollback(backward=True) is True
    assert term._search_pos == pos


def test_find_forward_moves_down(term):
    """下方向で次のヒットへ進む。"""
    _feed_lines(term, 120)
    term._search_query = "line-00"      # 0000..0099 が該当
    assert term.find_in_scrollback(backward=True)   # 最も近い上のヒット(0099)
    assert term.find_in_scrollback(backward=True)   # さらに上(0098)
    upper = term._search_pos
    assert term.find_in_scrollback(backward=False)  # 下へ戻る(0099)
    assert term._search_pos == upper + 1
    # 最後尾ヒットから下へ → 先頭ヒットへ回り込む
    term._search_pos = 99
    assert term.find_in_scrollback(backward=False)
    assert term._search_pos == 0


def test_find_without_hits_returns_false(term):
    _feed_lines(term, 30)
    term._search_query = "存在しない語"
    assert term.find_in_scrollback(backward=True) is False
    assert term._search_pos is None


def test_case_sensitivity_toggle(term):
    _feed_lines(term, 5, fmt="Error-{:04d}")
    term._search_query = "error"
    term._search_case = False
    assert term.find_in_scrollback(backward=True) is True
    term._search_case = True
    term._search_pos = None
    assert term.find_in_scrollback(backward=True) is False


def test_search_row_cells_maps_columns(term):
    """可視行のヒットが正しい桁に割り当たる(描画ハイライト用)。"""
    term._on_data(b"abc NEEDLE xyz\r\n")
    term._search_query = "needle"
    cells = term._search_row_cells(0)
    assert cells == set(range(4, 10))   # "NEEDLE" は 4..9 桁目
    # 全角を挟んでも桁ずれしない(全角は 2 桁を占める)
    term._on_data("あerr\r\n".encode())
    term._search_query = "err"
    cells2 = term._search_row_cells(1)
    assert cells2 == {2, 3, 4}          # 「あ」が 0-1 桁、err は 2..4


def test_toggle_and_close_search_bar(term):
    """Ctrl+Shift+F でバーが開き、閉じるとハイライトが消える。"""
    term.toggle_search()
    assert term._search_bar.isVisibleTo(term)
    term._search_edit.setText("abc")
    assert term._search_query == "abc"
    term.close_search()
    assert not term._search_bar.isVisibleTo(term)
    assert term._search_query == ""
