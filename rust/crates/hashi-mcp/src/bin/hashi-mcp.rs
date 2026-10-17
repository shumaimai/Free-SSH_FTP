//! MCP stdioヘルパー（console-mode）。stdoutへはJSON-RPCだけを書き、診断はstderrへ出す。
//! 使い方: HashiMCP-RustPreview <endpoint.json>（Hashiの「外部操作」画面が生成する設定から起動）。
use serde_json::{json, Value};
use std::{
    io::{BufRead, Write},
    sync::{Arc, Mutex},
};

fn write(stdout: &Mutex<std::io::Stdout>, value: Value) {
    if value.is_null() {
        return;
    }
    if let Ok(mut stdout) = stdout.lock() {
        let _ = writeln!(stdout, "{value}");
        let _ = stdout.flush();
    }
}

/// 起動中のHashiとの対応付け。接続情報は要求のたびに読み直すので、Hashiの再起動や
/// 後からの起動に追従できる。新しいHashi(instanceが違う)へは、初期化の手順を再生してから送る。
struct Link {
    path: std::path::PathBuf,
    handshake: Mutex<Handshake>,
}

#[derive(Default)]
struct Handshake {
    /// クライアントから受けたinitializeとnotifications/initialized。
    messages: Vec<Value>,
    /// 初期化を再生済みのHashiのinstance。
    synced: Option<String>,
}

impl Link {
    fn endpoint(&self) -> Option<Value> {
        let raw = std::fs::read(&self.path).ok()?;
        serde_json::from_slice::<Value>(&raw)
            .ok()
            .filter(|v| v.is_object() && v["instance"].is_string())
    }

    /// Hashiへ送る。つながらなければNone(呼び出し側がオフライン応答にする)。
    fn forward(&self, client: &str, message: &Value) -> Option<Value> {
        let endpoint = self.endpoint()?;
        let instance = endpoint["instance"].as_str()?.to_owned();
        let method = message["method"].as_str().unwrap_or_default();
        {
            let mut handshake = self.handshake.lock().ok()?;
            if method == "initialize" {
                handshake.messages.clear();
                handshake.synced = None;
            }
            if handshake.synced.as_deref() != Some(instance.as_str()) {
                for earlier in handshake.messages.clone() {
                    hashi_mcp::request(&endpoint, client, &earlier).ok()?;
                }
                handshake.synced = Some(instance);
            }
            if method == "initialize" || method == "notifications/initialized" {
                handshake.messages.push(message.clone());
            }
        }
        match hashi_mcp::request(&endpoint, client, message) {
            Ok(response) => Some(response),
            Err(_) => {
                if let Ok(mut handshake) = self.handshake.lock() {
                    handshake.synced = None;
                }
                None
            }
        }
    }

    /// 初期化の記録だけ更新する(Hashiに届かなかった場合)。
    fn remember(&self, message: &Value) {
        let method = message["method"].as_str().unwrap_or_default();
        if let Ok(mut handshake) = self.handshake.lock() {
            if method == "initialize" {
                handshake.messages.clear();
                handshake.synced = None;
            }
            if method == "initialize" || method == "notifications/initialized" {
                handshake.messages.push(message.clone());
            }
        }
    }
}

fn send(link: &Link, client: &str, message: &Value, stdout: &Mutex<std::io::Stdout>) {
    match link.forward(client, message) {
        Some(response) => write(stdout, response),
        None => {
            link.remember(message);
            write(stdout, hashi_mcp::offline_reply(message));
        }
    }
}

fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    // 引数なし: 固定フォルダの接続情報を使う(通常)。引数1つ: そのendpoint.jsonを使う。
    let path = match arguments.len() {
        0 => hashi_mcp::default_endpoint().ok(),
        1 => Some(std::path::PathBuf::from(&arguments[0])),
        _ => None,
    };
    let Some(path) = path else {
        eprintln!("使い方: HashiMCP-RustPreview [endpoint.json]");
        return std::process::ExitCode::from(2);
    };
    // Hashiが未起動でもここでは終了しない。呼び出し時に原因と対処を返す。
    let endpoint = Arc::new(Link {
        path,
        handshake: Mutex::new(Handshake::default()),
    });
    let client = uuid::Uuid::new_v4().simple().to_string();
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut workers = vec![];
    loop {
        let mut line = Vec::new();
        let read = std::io::Read::take(&mut reader, hashi_mcp::MAX_MESSAGE as u64 + 1)
            .read_until(b'\n', &mut line);
        if matches!(read, Ok(0) | Err(_)) {
            send(
                &endpoint,
                &client,
                &json!({"jsonrpc": "2.0", "method": "notifications/hashi-disconnect"}),
                &stdout,
            );
            break;
        }
        if line.len() > hashi_mcp::MAX_MESSAGE {
            write(
                &stdout,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Message too large"}}),
            );
            break;
        }
        let Some(message) = serde_json::from_slice::<Value>(&line)
            .ok()
            .filter(Value::is_object)
        else {
            write(
                &stdout,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
            );
            continue;
        };
        // initialize/initializedは順番を保持し、操作要求だけを並列化する（同時4件まで）。
        if message["method"] != "tools/call" {
            send(&endpoint, &client, &message, &stdout);
            continue;
        }
        if running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 4 {
            running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            write(
                &stdout,
                json!({"jsonrpc": "2.0", "id": message.get("id"), "error": {"code": -32000, "message": "Too many requests"}}),
            );
            continue;
        }
        let (endpoint, client, stdout, running) = (
            endpoint.clone(),
            client.clone(),
            stdout.clone(),
            running.clone(),
        );
        workers.push(std::thread::spawn(move || {
            send(&endpoint, &client, &message, &stdout);
            running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }));
        workers.retain(|worker| !worker.is_finished());
    }
    for worker in workers {
        let _ = worker.join();
    }
    std::process::ExitCode::SUCCESS
}
