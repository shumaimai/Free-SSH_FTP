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

fn send(endpoint: &Value, client: &str, message: &Value, stdout: &Mutex<std::io::Stdout>) {
    match hashi_mcp::request(endpoint, client, message) {
        Ok(response) => write(stdout, response),
        Err(_) => {
            if let Some(id) = message.get("id").filter(|id| !id.is_null()) {
                write(
                    stdout,
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000,
                    "message": "Hashiへ接続できません。起動状態とMCPの許可を確認してください"}}),
                );
            }
        }
    }
}

fn main() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 1 {
        eprintln!("使い方: HashiMCP-RustPreview <endpoint.json>");
        return std::process::ExitCode::from(2);
    }
    let Some(endpoint) = std::fs::read(&arguments[0])
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|endpoint| endpoint.is_object())
    else {
        eprintln!("Hashi MCPを開始できません。Hashiから接続設定を生成してください。");
        return std::process::ExitCode::from(1);
    };
    let endpoint = Arc::new(endpoint);
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
