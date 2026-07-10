import os

import pytest

from hashi.filebrowser import _safe_local_child


@pytest.mark.parametrize("name", ["../secret", "..\\secret", "/etc/passwd", "C:\\secret", ".."])
def test_safe_local_child_rejects_path_traversal(tmp_path, name):
    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(tmp_path), name)


def test_safe_local_child_rejects_symlink_escape(tmp_path):
    outside = tmp_path.parent / "outside"
    outside.mkdir()
    link = tmp_path / "escape"
    try:
        os.symlink(outside, link, target_is_directory=True)
    except (OSError, NotImplementedError):
        pytest.skip("シンボリックリンクを作成できない環境")

    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(tmp_path), "escape")
    with pytest.raises(ValueError):
        _safe_local_child(str(tmp_path), str(link), "secret.txt")


def test_safe_local_child_keeps_regular_name_inside_root(tmp_path):
    path = _safe_local_child(str(tmp_path), str(tmp_path), "report.txt")
    assert path == str(tmp_path / "report.txt")
