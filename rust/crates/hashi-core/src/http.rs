//! 限定したHTTPSクライアント。証明書はOSの検証器を使い、本文サイズと時間を制限する。
//! 平文HTTPはloopback（試験用）だけに限り、HTTPSからの格下げリダイレクトを許可しない。
use anyhow::{bail, Context, Result};
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};

pub struct Limits {
    pub timeout: Duration,
    pub max_bytes: u64,
}

fn is_loopback_http(url: &str) -> bool {
    let Ok(parsed) = url.parse::<ureq::http::Uri>() else {
        return false;
    };
    parsed.scheme_str() == Some("http")
        && matches!(
            parsed.host(),
            Some("127.0.0.1") | Some("[::1]") | Some("::1")
        )
}

fn agent(url: &str, limits: &Limits) -> Result<ureq::Agent> {
    let loopback = is_loopback_http(url);
    if !url.starts_with("https://") && !loopback {
        bail!("HTTPSのURLだけを使用します");
    }
    let config = ureq::Agent::config_builder()
        .https_only(!loopback)
        .timeout_global(Some(limits.timeout))
        .max_redirects(3)
        // 認証付き要求はリダイレクト先へ送らない方針。ここではヘッダーに秘密を載せない。
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build();
    Ok(config.into())
}

/// GETしてJSONを返す。上限を超える本文、2xx以外、JSONでない応答はエラー。
pub fn get_json(url: &str, headers: &[(&str, &str)], limits: &Limits) -> Result<serde_json::Value> {
    let agent = agent(url, limits)?;
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .call()
        .with_context(|| format!("HTTP要求に失敗しました: {url}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limits.max_bytes)
        .read_to_vec()
        .context("応答本文を読めないか、上限を超えました")?;
    serde_json::from_slice(&bytes).context("応答がJSONではありません")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn plain_http_is_rejected_except_loopback() {
        let limits = Limits {
            timeout: Duration::from_secs(1),
            max_bytes: 1024,
        };
        assert!(get_json("http://example.com/", &[], &limits).is_err());
        assert!(get_json("ftp://example.com/", &[], &limits).is_err());
        assert!(is_loopback_http("http://127.0.0.1:9/x"));
        assert!(!is_loopback_http("http://127.0.0.1.example.com/x"));
    }

    fn serve_once(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        });
        format!("http://{address}/releases")
    }

    #[test]
    fn body_limit_and_json_are_enforced() {
        let limits = Limits {
            timeout: Duration::from_secs(5),
            max_bytes: 16,
        };
        let url = serve_once(br#"{"tag":"rust-v1.0.0"}"#);
        assert!(get_json(&url, &[], &limits).is_err(), "上限超過を拒否");
        let url = serve_once(br#"[1,2]"#);
        assert_eq!(
            get_json(&url, &[("Accept", "application/json")], &limits).unwrap(),
            serde_json::json!([1, 2])
        );
        let url = serve_once(b"not json");
        assert!(get_json(&url, &[], &limits).is_err());
    }
}
