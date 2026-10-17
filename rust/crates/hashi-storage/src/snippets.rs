//! Python版`snippets.py`互換のスニペット。保存形式は`[{name, body, send_enter}]`。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    pub body: String,
    pub send_enter: bool,
}
impl Default for Snippet {
    fn default() -> Self {
        Self {
            name: String::new(),
            body: String::new(),
            send_enter: true,
        }
    }
}

/// 保存値を読む。型が違う記録は空として扱わず、上書きを避けるためエラーにする。
pub fn parse(value: &Value) -> io::Result<Vec<Snippet>> {
    let Value::Array(items) = value else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "スニペットの保存形式が不正です",
        ));
    };
    items
        .iter()
        .map(|item| {
            if !item.is_object() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "スニペットの保存形式が不正です",
                ));
            }
            serde_json::from_value(item.clone()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "スニペットの項目が不正です")
            })
        })
        .collect()
}
pub fn to_value(snippets: &[Snippet]) -> Value {
    serde_json::to_value(snippets).expect("スニペットはJSONへ変換できる")
}

/// Pythonの`\{\{\s*([^{}\s]+)\s*\}\}`と同じ一致を、左から重ならずに返す。
fn matches(body: &str) -> Vec<(usize, usize, &str)> {
    let mut found = vec![];
    let mut start = 0;
    while let Some(offset) = body[start..].find("{{") {
        let at = start + offset;
        match match_at(body, at) {
            Some((end, name)) => {
                found.push((at, end, name));
                start = end;
            }
            None => start = at + 1,
        }
    }
    found
}
fn match_at(body: &str, at: usize) -> Option<(usize, &str)> {
    let rest = &body[at + 2..];
    let trimmed = rest.trim_start();
    let name_start = at + 2 + (rest.len() - trimmed.len());
    let name_len = trimmed
        .find(|c: char| c == '{' || c == '}' || c.is_whitespace())
        .unwrap_or(trimmed.len());
    if name_len == 0 {
        return None;
    }
    let name = &body[name_start..name_start + name_len];
    let after = &body[name_start + name_len..];
    let closing = after.trim_start();
    if !closing.starts_with("}}") {
        return None;
    }
    Some((body.len() - closing.len() + 2, name))
}

/// 本文中の`{{変数名}}`を出現順・重複なしで返す。
pub fn find_variables(body: &str) -> Vec<String> {
    let mut variables: Vec<String> = vec![];
    for (_, _, name) in matches(body) {
        if !variables.iter().any(|known| known == name) {
            variables.push(name.to_owned());
        }
    }
    variables
}

/// 値がある変数だけ置換し、値が無い変数は元の`{{...}}`のまま残す。
pub fn expand(body: &str, values: &HashMap<String, String>) -> String {
    let mut output = String::with_capacity(body.len());
    let mut last = 0;
    for (start, end, name) in matches(body) {
        output.push_str(&body[last..start]);
        match values.get(name) {
            Some(value) => output.push_str(value),
            None => output.push_str(&body[start..end]),
        }
        last = end;
    }
    output.push_str(&body[last..]);
    output
}

/// 端末へ送るbytes。Python版`send_text`と同じく改行をCRへ変換する。
pub fn terminal_bytes(snippet: &Snippet, values: &HashMap<String, String>) -> Vec<u8> {
    let mut body = expand(&snippet.body, values);
    if snippet.send_enter {
        body.push('\n');
    }
    body.replace('\n', "\r").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn variables_follow_python_regex() {
        assert_eq!(
            find_variables("ls {{ dir }} {{dir}} {{{x}} {{a b}} {{}} {{日本}}"),
            ["dir", "x", "日本"]
        );
        let values = HashMap::from([("dir".to_owned(), "/srv".to_owned())]);
        assert_eq!(
            expand("cd {{ dir }} && echo {{missing}}", &values),
            "cd /srv && echo {{missing}}"
        );
        assert_eq!(expand("{{{x}}", &HashMap::new()), "{{{x}}");
        assert_eq!(
            expand("{{{x}}", &HashMap::from([("x".into(), "1".into())])),
            "{1"
        );
    }

    #[test]
    fn stored_python_format_round_trips_and_rejects_wrong_types() {
        let stored = json!([
            {"name":"再起動","body":"sudo systemctl restart {{svc}}","send_enter":false,"future":1},
            {"name":"一覧","body":"ls -la"}
        ]);
        let snippets = parse(&stored).unwrap();
        assert!(!snippets[0].send_enter);
        assert!(snippets[1].send_enter);
        assert_eq!(
            to_value(&snippets)[1],
            json!({"name":"一覧","body":"ls -la","send_enter":true})
        );
        assert!(parse(&json!({"name":"x"})).is_err());
        assert!(parse(&json!([{"name":1}])).is_err());
        assert!(parse(&json!(["x"])).is_err());
    }

    #[test]
    fn enter_and_newlines_become_carriage_returns() {
        let snippet = Snippet {
            name: "複数行".into(),
            body: "echo a\necho {{v}}".into(),
            send_enter: true,
        };
        let values = HashMap::from([("v".to_owned(), "日本".to_owned())]);
        assert_eq!(
            terminal_bytes(&snippet, &values),
            "echo a\recho 日本\r".as_bytes()
        );
    }
}
