import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def test_rust_message_catalog_is_complete():
    result = subprocess.run(
        [sys.executable, "-I", str(ROOT / "tools/rust_messages.py"), "check"],
        capture_output=True, text=True, encoding="utf-8",
    )
    assert result.returncode == 0, result.stderr


def test_literal_scanner_skips_test_modules_and_comments():
    sys.path.insert(0, str(ROOT / "tools"))
    import rust_messages

    src = '''
// コメントの日本語
fn a() -> &'static str { "本番の文言" }
fn b() -> String { format!("{}を開けません: \\"{name}\\"", x) }
#[cfg(test)]
mod tests { fn t() { assert_eq!(a(), "テスト専用"); } }
fn c() { let _ = r#"生文字列の{}日本語"#; }
'''
    found = [text for _, text in rust_messages.literals(src)]
    assert found == ['本番の文言', '{}を開けません: "{name}"', '生文字列の{}日本語']
