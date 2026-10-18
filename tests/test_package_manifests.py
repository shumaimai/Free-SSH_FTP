import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHA = "ab" * 32


def run(*args):
    return subprocess.run(
        [sys.executable, "-I", str(ROOT / "tools/package_manifests.py"), *args],
        capture_output=True, text=True, encoding="utf-8",
    )


def test_manifests_are_rendered(tmp_path):
    result = run("2.3.4", SHA, str(tmp_path))
    assert result.returncode == 0, result.stderr
    installer = (tmp_path / "winget/shumaimai.Hashi.installer.yaml").read_text(encoding="utf-8")
    assert "InstallerSha256: " + SHA.upper() in installer
    assert "v2.3.4/Hashi-v2.3.4-windows-x64.zip" in installer
    assert r"RelativeFilePath: Hashi-v2.3.4\Hashi.exe" in installer
    scoop = json.loads((tmp_path / "scoop/hashi.json").read_text(encoding="utf-8"))
    assert scoop["version"] == "2.3.4" and scoop["hash"] == SHA
    assert scoop["extract_dir"] == "Hashi-v2.3.4"
    for path in tmp_path.rglob("*"):
        if path.is_file():
            assert "@VERSION@" not in path.read_text(encoding="utf-8")
            assert "@SHA256@" not in path.read_text(encoding="utf-8")


def test_bad_arguments_are_rejected(tmp_path):
    assert run("v2.3.4", SHA, str(tmp_path)).returncode != 0
    assert run("2.3.4", "xyz", str(tmp_path)).returncode != 0
