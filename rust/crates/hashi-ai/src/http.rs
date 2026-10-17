//! 認証情報を転送・表示しないJSON/SSE通信（Python版`ai_http.py`相当）。
//! HTTPS（loopbackだけHTTP可）、リダイレクト禁止、応答サイズ上限、POSTの自動再送なし。
use anyhow::{bail, Result};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const JSON_LIMIT: u64 = 2 * 1024 * 1024;
const LINE_LIMIT: usize = 65536;
const STREAM_LIMIT: usize = 4 * 1024 * 1024;

/// HTTPSか、ローカルのHTTP。資格情報・query・fragment付きURLは拒否する。末尾の`/`を除く。
pub fn validate_base_url(url: &str) -> Result<String> {
    let url = url.trim();
    let error = || anyhow::anyhow!("HTTPS、またはローカルHTTPのURLを指定してください");
    let parsed = url::Url::parse(url).map_err(|_| error())?;
    let host = parsed.host_str().ok_or_else(error)?;
    let local = matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1");
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !(parsed.scheme() == "https" || (parsed.scheme() == "http" && local))
    {
        return Err(error());
    }
    Ok(url.trim_end_matches('/').to_owned())
}

/// APIの個別エンドポイントを貼り付けても二重に連結しない。Command Codeの公式URLへ正規化する。
pub fn normalize_api_base(url: &str) -> Result<String> {
    let mut base = validate_base_url(url)?;
    for suffix in ["/chat/completions", "/responses", "/messages", "/models"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_owned();
            break;
        }
    }
    if base == "https://api.commandcode.ai" || base == "https://api.commandcode.ai/v1" {
        base = "https://api.commandcode.ai/provider/v1".into();
    }
    Ok(base)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_send_request(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

fn hint(code: u16) -> &'static str {
    match code {
        400 => {
            "モデル名とAPI形式を確認してください。ツール非対応なら端末ツールの利用を外してください"
        }
        401 => "この接続先のAPIキーを確認してください",
        403 => "APIキーの権限と、このモデルを利用できる契約・残高を確認してください",
        404 => "APIのURLとモデル名を確認してください",
        429 => "利用上限です。時間を置いて確認してください",
        _ => "接続設定を確認してください",
    }
}

/// 要求を送り、2xxの本文を返す。エラー文にURL以外の入力（キー等）を含めない。
fn open(
    url: &str,
    body: Option<&[u8]>,
    headers: &[(String, String)],
    accept_stream: bool,
) -> Result<ureq::Body> {
    validate_base_url(url)?;
    for (name, value) in headers {
        if value.contains(['\r', '\n']) || name.contains(['\r', '\n']) {
            bail!("APIキーと接続先の入力形式を確認してください。改行を含むキーは使えません");
        }
    }
    let agent = agent();
    let response = match body {
        Some(body) => {
            let mut request = agent.post(url).header("Content-Type", "application/json");
            if accept_stream {
                request = request.header("Accept", "text/event-stream");
            }
            for (name, value) in headers {
                request = request.header(name.as_str(), value.as_str());
            }
            request.send(body)
        }
        None => {
            let mut request = agent.get(url);
            for (name, value) in headers {
                request = request.header(name.as_str(), value.as_str());
            }
            request.call()
        }
    }
    .map_err(|_| anyhow::anyhow!("AI接続に失敗しました。接続先とネットワークを確認してください"))?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        bail!("認証付き通信のリダイレクトは許可しません");
    }
    if !(200..300).contains(&status) {
        bail!("AI接続エラー HTTP {status}: {}", hint(status));
    }
    Ok(response.into_body())
}

pub fn get_json(url: &str, headers: &[(String, String)]) -> Result<Value> {
    let mut body = open(url, None, headers, false)?;
    read_json(body.with_config().limit(JSON_LIMIT).reader())
}
pub fn post_json(url: &str, payload: &Value, headers: &[(String, String)]) -> Result<Value> {
    let mut body = open(url, Some(&serde_json::to_vec(payload)?), headers, false)?;
    read_json(body.with_config().limit(JSON_LIMIT).reader())
}
fn read_json(mut reader: impl Read) -> Result<Value> {
    let mut bytes = vec![];
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("応答が大きすぎるか、途中で切れました"))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("応答がJSONではありません"))
}

/// SSEのdataを1件ずつ渡す。`[DONE]`で`{"type":"done"}`を渡して終える。取消は行の区切りで確認する。
pub fn events(
    url: &str,
    payload: &Value,
    headers: &[(String, String)],
    cancel: &CancellationToken,
    mut on_event: impl FnMut(Value) -> Result<()>,
) -> Result<()> {
    if cancel.is_cancelled() {
        bail!("AI処理を停止しました");
    }
    let body = open(url, Some(&serde_json::to_vec(payload)?), headers, true)?;
    parse_events(BufReader::new(body.into_reader()), cancel, &mut on_event)
}

pub fn parse_events(
    mut reader: impl BufRead,
    cancel: &CancellationToken,
    on_event: &mut impl FnMut(Value) -> Result<()>,
) -> Result<()> {
    let mut parts: Vec<String> = vec![];
    let mut total = 0;
    loop {
        if cancel.is_cancelled() {
            bail!("AI処理を停止しました");
        }
        let mut raw = vec![];
        let count = (&mut reader)
            .take(LINE_LIMIT as u64 + 1)
            .read_until(b'\n', &mut raw)
            .map_err(|_| anyhow::anyhow!("AIストリームが途中で切れました"))?;
        if count == 0 {
            return Ok(());
        }
        total += count;
        if count > LINE_LIMIT || total > STREAM_LIMIT {
            bail!("AIストリームが大きすぎます");
        }
        let line = String::from_utf8(raw)
            .map_err(|_| anyhow::anyhow!("AIストリームの文字コードが不正です"))?;
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(data) = line.strip_prefix("data:") {
            parts.push(data.trim_start().to_owned());
        } else if line.is_empty() && !parts.is_empty() {
            let data = parts.join("\n");
            parts.clear();
            if data == "[DONE]" {
                on_event(serde_json::json!({"type": "done"}))?;
                return Ok(());
            }
            on_event(
                serde_json::from_str(&data)
                    .map_err(|_| anyhow::anyhow!("AIストリームの形式が不正です"))?,
            )?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_python_rules() {
        assert_eq!(
            validate_base_url(" https://api.openai.com/v1/ ").unwrap(),
            "https://api.openai.com/v1"
        );
        assert!(validate_base_url("http://api.example.com/v1").is_err());
        assert!(validate_base_url("http://127.0.0.1:1234/v1").is_ok());
        assert!(validate_base_url("http://localhost/v1").is_ok());
        assert!(validate_base_url("https://user:pass@example.com").is_err());
        assert!(validate_base_url("https://example.com/v1?x=1").is_err());
        assert!(validate_base_url("https://example.com/v1#x").is_err());
        assert!(validate_base_url("ftp://example.com").is_err());
        assert_eq!(
            normalize_api_base("https://api.openai.com/v1/chat/completions").unwrap(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_api_base("https://api.commandcode.ai/v1").unwrap(),
            "https://api.commandcode.ai/provider/v1"
        );
    }

    #[test]
    fn sse_parser_joins_data_lines_and_stops_at_done() {
        let stream = b"event: x\ndata: {\"a\":\ndata: 1}\n\n: comment\ndata: {\"b\":2}\r\n\r\ndata: [DONE]\n\ndata: {\"c\":3}\n\n";
        let mut events = vec![];
        parse_events(&stream[..], &CancellationToken::new(), &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            events,
            vec![
                serde_json::json!({"a":1}),
                serde_json::json!({"b":2}),
                serde_json::json!({"type":"done"})
            ]
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(parse_events(&stream[..], &cancel, &mut |_| Ok(())).is_err());
        let long = format!("data: {}\n\n", "x".repeat(LINE_LIMIT + 10));
        assert!(parse_events(long.as_bytes(), &CancellationToken::new(), &mut |_| Ok(())).is_err());
    }

    #[test]
    fn redirects_and_errors_do_not_follow_or_leak() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for status in [
                "302 Found\r\nLocation: http://127.0.0.1:9/steal",
                "401 Unauthorized",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buffer);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        let url = format!("http://{address}/v1/models");
        let headers = vec![("Authorization".to_owned(), "Bearer sk-secret".to_owned())];
        let error = get_json(&url, &headers).unwrap_err().to_string();
        assert!(error.contains("リダイレクト"), "{error}");
        let error = get_json(&url, &headers).unwrap_err().to_string();
        assert!(
            error.contains("401") && !error.contains("sk-secret"),
            "{error}"
        );
        let bad = vec![("Authorization".to_owned(), "Bearer a\nb".to_owned())];
        assert!(get_json(&url, &bad)
            .unwrap_err()
            .to_string()
            .contains("改行"));
    }
}
