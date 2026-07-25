"""16 進エディタとテキスト/バイナリ判定(Issue #122)のテスト。

ゲームサーバーのデータファイル(.dat 等)を壊さずに開いて直せることを固定する。
とくに **保存でファイルが書き換わらない**(改行変換 / BOM 付与 / バイト欠落が
起きない)ことを重点的に確認する。
"""
import pytest

from hashi.hexedit import BYTES_PER_ROW, HexEdit, looks_binary, sniff_file


# ---- テキスト / バイナリ判定 -------------------------------------------------
@pytest.mark.parametrize("sample", [
    b"",                                   # 空はテキスト扱い(編集できる)
    b"server-port=25565\nmax-players=20\n",
    b"\xef\xbb\xbfBOM \xe4\xbb\x98\xe3\x81\x8d",   # UTF-8 BOM
    b"\xff\xfe\x53\x00\x56\x00",                    # UTF-16 LE BOM
    "日本語の設定ファイル\n".encode(),
    b"line1\r\nline2\r\n",                 # CRLF はテキスト
])
def test_looks_binary_false_for_text(sample):
    assert looks_binary(sample) is False


@pytest.mark.parametrize("sample", [
    b"\x00\x01\x02\x03",                   # NUL を含む
    b"\x1f\x8b\x08\x00\x00\x00\x00\x00",   # gzip ヘッダ(level.dat 等)
    bytes(range(0, 32)) * 4,               # 制御文字だらけ
    b"\xff\xd8\xff\xe0\x00\x10JFIF",       # JPEG
])
def test_looks_binary_true_for_binary(sample):
    assert looks_binary(sample) is True


def test_looks_binary_tolerates_truncated_multibyte():
    """先頭 N バイトで切ったとき、途中で切れた UTF-8 を誤判定しない。"""
    text = ("あ" * 100).encode()
    assert looks_binary(text[:99]) is False      # 3 バイト文字の途中で切断


def test_sniff_file(tmp_path):
    txt = tmp_path / "server.properties"
    txt.write_bytes(b"motd=Hello\n")
    assert sniff_file(str(txt)) is False
    bin_ = tmp_path / "level.dat"
    bin_.write_bytes(b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03")
    assert sniff_file(str(bin_)) is True
    assert sniff_file(str(tmp_path / "存在しない")) is False


# ---- HexEdit ----------------------------------------------------------------
def test_hex_overwrite_keeps_size_and_tracks_changes(qapp):
    data = bytes(range(32))
    w = HexEdit(data)
    assert w.data() == data
    assert not w.is_modified()

    # 16 進欄で 1 バイトを上書き(上位→下位の 2 打鍵で 1 バイト確定)
    w._move_to(0)
    w._handle_input("F")
    w._handle_input("F")
    assert w.data()[0] == 0xFF
    assert w.is_modified()
    assert len(w.data()) == len(data)          # サイズ不変(挿入・削除しない)
    assert w.changed_offsets() == {0}

    # 保存済みにすると差分表示がリセットされる
    w.mark_saved()
    assert not w.is_modified()
    assert w.changed_offsets() == set()
    w.deleteLater()


def test_hex_ascii_column_edit(qapp):
    w = HexEdit(b"AAAA")
    w._move_to(1)
    w._in_ascii = True
    w._handle_input("Z")
    assert w.data() == b"AZAA"
    # 1 バイトで表せない文字は無視する(サイズを変えないため)
    before = w.data()
    w._handle_input("あ")
    assert w.data() == before
    w.deleteLater()


def test_hex_cursor_navigation_clamps(qapp):
    w = HexEdit(bytes(BYTES_PER_ROW * 3))
    w._move_to(-5)
    assert w.cursor_offset() == 0
    w._move_to(10**6)
    assert w.cursor_offset() == len(w.data()) - 1
    w.deleteLater()


def test_hex_empty_data_is_safe(qapp):
    """空ファイルでも移動・入力で落ちない。"""
    w = HexEdit(b"")
    w._move_to(0)
    w._handle_input("A")
    assert w.data() == b""
    w.deleteLater()
