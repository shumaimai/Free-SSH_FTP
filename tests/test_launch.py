"""launch.py / 関連付け起動のテスト。"""

def test_no_args_is_app_mode():
    from hashi.launch import dispatch, is_editor_mode, parse_cli
    ns = parse_cli(["main.py"])
    assert not is_editor_mode(ns)
    assert dispatch(["main.py"]) == ("app", ns)


def test_file_arg_is_editor_mode():
    from hashi.launch import dispatch, is_editor_mode, parse_cli
    ns = parse_cli(["main.py", r"C:\notes\memo.txt"])
    assert is_editor_mode(ns)
    assert ns.files == [r"C:\notes\memo.txt"]
    assert dispatch(["main.py", r"C:\notes\memo.txt"])[0] == "editor"


def test_editor_flag_without_file():
    from hashi.launch import is_editor_mode, parse_cli
    ns = parse_cli(["main.py", "--editor"])
    assert is_editor_mode(ns)
    assert ns.files == []


def test_normalize_paths_strips_quotes():
    from hashi.launch import normalize_paths
    assert normalize_paths([r'"C:\path with spaces\a.txt"']) == [
        r"C:\path with spaces\a.txt"
    ]


def test_open_path_force_ignores_extension_filter(qapp, tmp_path):
    """関連付け起動(force=True)では拡張子フィルタを通さない。"""
    from hashi.editor import LocalEditorHub
    p = tmp_path / "data.zzzunknown"
    p.write_text("plain\n", encoding="utf-8")
    hub = LocalEditorHub(_FakeSettings(open_text=False))
    win = hub.open_path(str(p), force=True)
    assert win is not None
    win.editor.document().setModified(False)
    win.close()


def test_open_path_respects_setting_without_force(qapp, tmp_path):
    from hashi.editor import LocalEditorHub
    p = tmp_path / "note.txt"
    p.write_text("x\n", encoding="utf-8")
    hub = LocalEditorHub(_FakeSettings(open_text=False))
    assert hub.open_path(str(p)) is None


def test_all_closed_signal(qapp, tmp_path):
    from hashi.editor import LocalEditorHub
    p = tmp_path / "a.txt"
    p.write_text("hi\n", encoding="utf-8")
    hub = LocalEditorHub(_FakeSettings())
    fired = []
    hub.all_closed.connect(lambda: fired.append(True))
    win = hub.open_path(str(p))
    win.editor.document().setModified(False)
    win.close()
    assert fired == [True]
    assert not hub.has_windows()


class _FakeSettings:
    def __init__(self, open_text=True):
        self._open_text = open_text

    def get(self, key, default=None):
        if key == "open_text_in_editor":
            return self._open_text
        return {"editor_font_size": 12, "editor_tab_width": 4}.get(key, default)
