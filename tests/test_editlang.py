"""editlang.py の拡張子レジストリと言語判定のテスト。"""
import pytest

from hashi.editlang import (
    AMBIGUOUS_EXTENSIONS,
    TEXT_EXTENSIONS,
    language_for,
    looks_text_filename,
    should_edit_internally,
)


@pytest.mark.parametrize("name,expected", [
    ("readme.md", True),
    ("app.tsx", True),
    ("Dockerfile", True),
    ("Makefile", True),
    (".gitignore", True),
    (".env", True),
    ("Cargo.toml", True),
    ("schema.graphql", True),
    ("main.tf", True),
    ("LICENSE", True),          # 拡張子なし
    ("config", True),           # 拡張子なし
    ("photo.png", False),
    ("archive.tar.gz", False),
    ("movie.mp4", False),
])
def test_looks_text_filename(name, expected):
    assert looks_text_filename(name) is expected


@pytest.mark.parametrize("name,expected", [
    ("server.properties", True),
    ("level.dat", True),
    ("world.sav", True),
    ("photo.png", False),
])
def test_should_edit_internally(name, expected):
    assert should_edit_internally(name) is expected


@pytest.mark.parametrize("path,lang", [
    ("main.py", "python"),
    ("app.rb", "ruby"),
    ("index.php", "php"),
    ("init.lua", "lua"),
    ("query.sql", "sql"),
    ("style.css", "css"),
    ("page.html", "markup"),
    ("README.md", "markup"),
    ("data.json", "json"),
    ("Dockerfile", "shell"),
    ("Gemfile", "shell"),
    (".editorconfig", "conf"),
    ("unknown.xyz", "plain"),
])
def test_language_for(path, lang):
    assert language_for(path) == lang


def test_text_extensions_is_large():
    """汎用エディタとして十分な数の拡張子をカバーしている。"""
    assert len(TEXT_EXTENSIONS) >= 150
    assert "graphql" in TEXT_EXTENSIONS
    assert "tf" in TEXT_EXTENSIONS
    assert "mdx" in TEXT_EXTENSIONS


def test_ambiguous_extensions_includes_game_formats():
    assert "dat" in AMBIGUOUS_EXTENSIONS
    assert "mca" in AMBIGUOUS_EXTENSIONS
