//! 観測データの秘密除去と、構造・端末IDを保ったままのサイズ制限付きJSON化。
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

const TEXT_LIMIT: usize = 32000;
/// 短縮しないキー。端末ID・操作IDを切ると誤対象操作につながる。
const IDENTIFIERS: [&str; 3] = ["session_id", "operation_id", "tool_call_id"];

static SECRETS: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    let rule = |pattern: &str, replacement| (regex::Regex::new(pattern).unwrap(), replacement);
    vec![
        // 秘密鍵。ENDが無い(途中で切れた)ものは末尾まで、BEGINが切れたものは本文からENDまで伏せる。
        rule(
            r"(?s)-----BEGIN [^-]*PRIVATE KEY-----.*?(?:-----END [^-]*PRIVATE KEY-----|\z)",
            "[秘密鍵を省略]",
        ),
        rule(
            r"(?:[A-Za-z0-9+/=]{16,}\r?\n)+-----END [^-]*PRIVATE KEY-----",
            "[秘密鍵を省略]",
        ),
        rule(
            r"(?s)PuTTY-User-Key-File-\d+:.*?(?:Private-MAC:[^\n]*|\z)",
            "[秘密鍵を省略]",
        ),
        // よく見る形式のトークン。
        rule(
            r"(?i)(?:authorization:\s*(?:bearer|basic|token)\s+|\bsk-(?:proj-|ant-)?)[\w.\-/+=]{8,}",
            "[認証トークンを省略]",
        ),
        rule(
            r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_\-]{30,}|glpat-[\w-]{16,})",
            "[認証トークンを省略]",
        ),
        rule(
            r"\beyJ[\w-]{8,}\.eyJ[\w-]{8,}\.[\w-]{8,}",
            "[認証トークンを省略]",
        ),
        // URLに埋め込んだ利用者名:パスワード。
        rule(
            r"([a-zA-Z][a-zA-Z0-9+.-]*://[^/\s:@]+:)[^@\s/]+@",
            "${1}[省略]@",
        ),
        // `--password x` や `--token=x`。
        rule(
            r"(?i)(--?(?:password|passwd|pass|token|secret|api-?key)(?:=|\s+))\S+",
            "${1}[省略]",
        ),
        // 名前に秘密らしい語を含む `キー: 値` / `キー=値` / JSONの `"キー": "値"`。
        // 先の規則で伏せ済みの`[…省略]`は値として扱わない。
        rule(
            r#"(?i)([\w.-]*(?:password|passwd|passphrase|secret|token|api[_-]?key|apikey|access[_-]?key|private[_-]?key|credential)[\w.-]*["']?\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|[^\s,;\[][^\s,;]*)"#,
            "${1}[省略]",
        ),
    ]
});

/// 秘密らしいパターンを置換し、改行/タブ以外の制御文字を除いて32000文字までにする。
pub fn redact(text: &str) -> String {
    let mut text = text.to_owned();
    for (pattern, replacement) in SECRETS.iter() {
        text = pattern.replace_all(&text, *replacement).into_owned();
    }
    text.chars()
        .filter(|c| matches!(c, '\n' | '\t') || *c as u32 >= 32)
        .take(TEXT_LIMIT)
        .collect()
}

fn clean(value: &Value, truncated: &mut bool) -> Value {
    match value {
        Value::String(text) => {
            *truncated |= text.chars().count() > TEXT_LIMIT;
            Value::String(redact(text))
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), clean(child, truncated)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|child| clean(child, truncated)).collect())
        }
        other => other.clone(),
    }
}

fn clip(
    value: &Value,
    key: Option<&str>,
    text_limit: usize,
    item_limit: usize,
    truncated: &mut bool,
) -> Value {
    match value {
        Value::String(text) => {
            if key.is_some_and(|key| IDENTIFIERS.contains(&key))
                || text.chars().count() <= text_limit
            {
                value.clone()
            } else {
                *truncated = true;
                Value::String(
                    text.chars().take(text_limit).collect::<String>() + "[長い内容を省略]",
                )
            }
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, child)| {
                    (
                        name.clone(),
                        clip(child, Some(name), text_limit, item_limit, truncated),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => {
            *truncated |= items.len() > item_limit;
            Value::Array(
                items
                    .iter()
                    .take(item_limit)
                    .map(|child| clip(child, None, text_limit, item_limit, truncated))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// 文字列をマスク・短縮してからJSON化する。完成したJSONを切らず、UTF-8で`limit`バイト以下にする。
pub fn observation_json(value: &Value, limit: usize) -> String {
    let mut initial = false;
    let cleaned = clean(value, &mut initial);
    let (mut text_limit, mut item_limit) = (TEXT_LIMIT, 128);
    loop {
        let mut truncated = initial;
        let mut data = clip(&cleaned, None, text_limit, item_limit, &mut truncated);
        if truncated {
            data = match data {
                Value::Object(mut map) => {
                    map.insert("truncated".into(), Value::Bool(true));
                    Value::Object(map)
                }
                other => {
                    let mut map = Map::new();
                    map.insert("items".into(), other);
                    map.insert("truncated".into(), Value::Bool(true));
                    Value::Object(map)
                }
            };
        }
        let encoded = data.to_string();
        if encoded.len() <= limit {
            return encoded;
        }
        if text_limit > 128 {
            text_limit /= 2;
        } else if item_limit > 1 {
            item_limit /= 2;
        } else {
            return json!({"truncated": true, "error": "観測データが大きすぎるため省略しました"})
                .to_string();
        }
    }
}

/// 共有中の端末の画面と最近の出力をまとめた、AIへ送る前の観測データ。
pub fn context_snapshot(broker: &hashi_core::broker::Broker) -> String {
    let mut data = vec![];
    for session in broker.list_sessions() {
        let id = session["session_id"].as_str().unwrap_or_default();
        let generation = session["generation"].as_u64().unwrap_or_default();
        let Ok(source) = broker.read_output(id, generation, 8000) else {
            continue;
        };
        let mut item = session.clone();
        item["label"] = json!(redact(session["label"].as_str().unwrap_or_default()));
        item["screen"] = json!(redact(source["screen"].as_str().unwrap_or_default()));
        item["recent_output"] = json!(redact(source["output"].as_str().unwrap_or_default()));
        item["trust"] = json!("untrusted_observation");
        data.push(item);
    }
    observation_json(&json!({"terminal_observations": data}), 64000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_matches_python_patterns() {
        let text =
            "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\nb";
        assert_eq!(redact(text), "a\n[秘密鍵を省略]\nb");
        assert_eq!(
            redact("Authorization: Bearer abc.def-ghi/jkl"),
            "[認証トークンを省略]"
        );
        assert_eq!(
            redact("key sk-proj-abcdefgh1234 end"),
            "key [認証トークンを省略] end"
        );
        assert_eq!(
            redact("PASSWORD = hunter2, next"),
            "PASSWORD = [省略], next"
        );
        assert_eq!(redact("api-key:xyz;"), "api-key:[省略];");
        // よく漏れる形式。
        for (input, leaked) in [
            (r#"{"password": "hunter 2"}"#, "hunter"),
            ("GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwx", "ghp_"),
            ("export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI", "wJalr"),
            ("id AKIAIOSFODNN7EXAMPLE", "AKIAIOSF"),
            ("git clone https://me:pa55@host/x", "pa55"),
            ("mysql --password hunter2", "hunter2"),
            ("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.abcdefghijk", "eyJzdWIi"),
            (
                "PuTTY-User-Key-File-3: ssh-ed25519\nPrivate-Lines: 1\nAAAAsecret\nPrivate-MAC: ab",
                "AAAAsecret",
            ),
        ] {
            assert!(
                !redact(input).contains(leaked),
                "{input} -> {}",
                redact(input)
            );
        }
        // 先頭(BEGIN)が切れた秘密鍵、末尾(END)が切れた秘密鍵。
        let body = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFB\n".repeat(3);
        let tail = format!("{body}-----END OPENSSH PRIVATE KEY-----\n$");
        assert_eq!(redact(&tail), "[秘密鍵を省略]\n$");
        let head = format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{body}");
        assert_eq!(redact(&head), "[秘密鍵を省略]");
        assert_eq!(redact("x\x1b[31my\x07\tz"), "x[31my\tz");
        assert_eq!(redact(&"あ".repeat(40000)).chars().count(), 32000);
    }

    #[test]
    fn observation_keeps_identifiers_and_valid_json_under_limit() {
        let id = "i".repeat(50000);
        let value = json!({"session_id": id, "output": "o".repeat(50000), "list": vec![1; 300]});
        let encoded = observation_json(&value, 20000);
        assert!(encoded.len() <= 20000 || encoded.contains("大きすぎる"));
        let parsed: Value = serde_json::from_str(&encoded).unwrap();
        if parsed.get("session_id").is_some() {
            assert_eq!(
                parsed["session_id"].as_str().unwrap().len(),
                32000,
                "redactの上限だけ"
            );
        }
        let small = observation_json(
            &json!({"session_id": "abc", "output": "x".repeat(100), "items": vec![0; 200]}),
            64000,
        );
        let parsed: Value = serde_json::from_str(&small).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert_eq!(parsed["items"].as_array().unwrap().len(), 128);
        assert_eq!(parsed["session_id"], "abc");
        let list = observation_json(&json!(["a", "b"]), 1000);
        assert_eq!(
            serde_json::from_str::<Value>(&list).unwrap(),
            json!(["a", "b"])
        );
        let huge = observation_json(&json!({"session_id": "s".repeat(30000)}), 100);
        assert!(huge.contains("大きすぎる"));
    }
}
