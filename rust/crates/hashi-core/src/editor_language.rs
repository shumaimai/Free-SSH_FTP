//! 旧版の拡張子/キーワード表を静的データで引き継ぐ。リッチテキストは解釈しない。
use serde_json::Value;
use std::sync::OnceLock;
fn registry() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!("edit-languages.json")).expect("静的な言語表")
    })
}
pub fn language_for(path: &str) -> String {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    let language = registry()["basenames"]
        .get(&base)
        .or_else(|| registry()["extensions"].get(base.rsplit('.').next().unwrap_or(&base)));
    language
        .and_then(Value::as_str)
        .unwrap_or("plain")
        .to_owned()
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
}
pub struct Span {
    pub range: std::ops::Range<usize>,
    pub kind: Kind,
}
pub fn highlight(text: &str, language: &str) -> Vec<Span> {
    static KEYWORDS: OnceLock<Value> = OnceLock::new();
    let words = KEYWORDS.get_or_init(|| {
        serde_json::from_str(include_str!("edit-keywords.json")).expect("静的なキーワード表")
    });
    let words = words.get(language).and_then(Value::as_array);
    let mut spans = vec![];
    let mut index = 0;
    while index < text.len() {
        if spans.len() >= 65536 {
            spans.push(Span {
                range: index..text.len(),
                kind: Kind::Plain,
            });
            break;
        }
        let start = index;
        let current = text[index..].chars().next().unwrap();
        let line_comment = (current == '#'
            && ["python", "shell", "conf", "ruby", "php"].contains(&language))
            || (text[index..].starts_with("//")
                && ["c", "js", "php", "css", "json"].contains(&language))
            || (text[index..].starts_with("--") && ["lua", "sql"].contains(&language));
        let kind = if line_comment {
            index += text[index..].find('\n').unwrap_or(text.len() - index);
            Kind::Comment
        } else if text[index..].starts_with("/*")
            && ["c", "js", "php", "css", "json"].contains(&language)
        {
            index += text[index + 2..]
                .find("*/")
                .map(|n| n + 4)
                .unwrap_or(text.len() - index);
            Kind::Comment
        } else if matches!(current, '\'' | '"' | '`') {
            index += current.len_utf8();
            let mut escaped = false;
            while index < text.len() {
                let c = text[index..].chars().next().unwrap();
                index += c.len_utf8();
                if c == '\n' || (!escaped && c == current) {
                    break;
                }
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                }
            }
            Kind::String
        } else if current.is_alphanumeric() || current == '_' {
            index += current.len_utf8();
            while index < text.len() {
                let c = text[index..].chars().next().unwrap();
                if !c.is_alphanumeric() && c != '_' {
                    break;
                }
                index += c.len_utf8();
            }
            let token = &text[start..index];
            if words.is_some_and(|words| {
                words.iter().any(|word| {
                    word.as_str().is_some_and(|word| {
                        if language == "sql" {
                            word.eq_ignore_ascii_case(token)
                        } else {
                            word == token
                        }
                    })
                })
            }) {
                Kind::Keyword
            } else if token.chars().all(|c| c.is_ascii_digit()) {
                Kind::Number
            } else {
                Kind::Plain
            }
        } else {
            index += current.len_utf8();
            Kind::Plain
        };
        spans.push(Span {
            range: start..index,
            kind,
        });
    }
    spans
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_registry_and_unicode_spans_keep_text_intact() {
        for (path, language) in [
            ("main.py", "python"),
            ("日本.rs", "c"),
            (".bashrc", "shell"),
            ("App.vue", "js"),
            ("settings.toml", "conf"),
            ("unknown.bin", "plain"),
        ] {
            assert_eq!(language_for(path), language);
        }
        let text = "def 日本():\n  return '# literal' # comment\n";
        let spans = highlight(text, "python");
        let rebuilt: String = spans.iter().map(|span| &text[span.range.clone()]).collect();
        assert_eq!(rebuilt, text);
        assert!(spans
            .iter()
            .any(|span| span.kind == Kind::String && &text[span.range.clone()] == "'# literal'"));
        assert!(spans
            .iter()
            .any(|span| span.kind == Kind::Comment && &text[span.range.clone()] == "# comment"));
    }
}
