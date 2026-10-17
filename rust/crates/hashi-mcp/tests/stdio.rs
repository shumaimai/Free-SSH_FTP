//! 実際のstdioヘルパーを起動し、stdoutがJSON-RPCだけで、切断で要求が取り消されることを確認する。
use hashi_core::broker::{Broker, Kind, Mode, Registration, Registry};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

#[tokio::test(flavor = "multi_thread")]
async fn helper_relays_json_rpc_and_disconnect() {
    let registry = Arc::new(Registry::default());
    let (input, mut received) = tokio::sync::mpsc::channel(8);
    let (id, generation) = registry.register(
        None,
        Registration {
            label: "端末".into(),
            kind: Kind::Local,
            shell: "unknown".into(),
            cwd: None,
            shareable: true,
            input,
            ssh: None,
        },
    );
    let broker = Arc::new(Broker::new(registry, None));
    broker
        .configure(
            Mode::Auto,
            &[(id.clone(), generation)],
            Duration::from_secs(60),
        )
        .unwrap();
    let bridge = hashi_mcp::McpBridge::start(broker.clone()).await.unwrap();
    let endpoint = bridge.endpoint().to_owned();
    let lines = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_HashiMCP-RustPreview"))
            .arg(endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ];
        for message in &messages {
            writeln!(stdin, "{message}").unwrap();
        }
        writeln!(stdin, "not json").unwrap();
        let call = json!({"jsonrpc":"2.0","id":"c1","method":"tools/call","params":{"name":"send_input",
            "arguments":{"session_id": id, "generation": generation, "text": "echo 日本", "enter": true}}});
        writeln!(stdin, "{call}").unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut lines = vec![];
        for _ in 0..4 {
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            lines.push(serde_json::from_str::<Value>(&line).expect("stdoutはJSONだけ"));
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
        assert!(rest.trim().is_empty(), "余分なstdout: {rest}");
        lines
    })
    .await
    .unwrap();
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "Hashi");
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 7);
    assert_eq!(lines[2]["error"]["code"], -32700);
    assert_eq!(lines[3]["id"], "c1");
    assert_eq!(lines[3]["result"]["isError"], false);
    assert_eq!(received.recv().await.unwrap(), "echo 日本\r".as_bytes());
    let finished = broker.audit_snapshot()["finished"].clone();
    assert!(
        !finished.to_string().contains("日本"),
        "監査記録に本文を入れない"
    );
    bridge.close();
}

fn exchange(
    stdin: &mut std::process::ChildStdin,
    stdout: &mut BufReader<std::process::ChildStdout>,
    message: Value,
) -> Value {
    writeln!(stdin, "{message}").unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// 登録は固定(引数なし)。Hashiより先にヘルパーが起動しても落ちず、後から開始した/
/// 再起動したHashiへ、初期化をやり直さずにつながる。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn helper_without_arguments_follows_hashi_start_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("mcp");
    let registry = Arc::new(Registry::default());
    let (input, _received) = tokio::sync::mpsc::channel(8);
    let (id, generation) = registry.register(
        None,
        Registration {
            label: "端末".into(),
            kind: Kind::Local,
            shell: "unknown".into(),
            cwd: None,
            shareable: true,
            input,
            ssh: None,
        },
    );
    let broker = Arc::new(Broker::new(registry, None));
    broker
        .configure(Mode::Consult, &[(id, generation)], Duration::from_secs(60))
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_HashiMCP-RustPreview"))
        .env(hashi_mcp::DIRECTORY_ENV, &directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let list_sessions = |n: u64| {
        json!({"jsonrpc":"2.0","id":n,"method":"tools/call",
        "params":{"name":"list_sessions","arguments":{}}})
    };

    // 1. Hashiが未起動でも初期化と一覧は成功し、呼び出しは原因を返す。
    let init = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "Hashi");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let tools = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 7);
    let offline = exchange(&mut stdin, &mut stdout, list_sessions(3));
    assert_eq!(offline["result"]["isError"], true);

    // 2. 後からHashiがMCPを開始する。ヘルパーの再起動は要らない。
    let bridge = hashi_mcp::McpBridge::start_at(broker.clone(), &directory)
        .await
        .unwrap();
    let online = exchange(&mut stdin, &mut stdout, list_sessions(4));
    assert_eq!(online["result"]["isError"], false, "{online}");
    bridge.close();

    // 3. Hashiを再起動(instanceとtokenが変わる)しても、同じヘルパーでつながる。
    let again = hashi_mcp::McpBridge::start_at(broker.clone(), &directory)
        .await
        .unwrap();
    let restarted = exchange(&mut stdin, &mut stdout, list_sessions(5));
    assert_eq!(restarted["result"]["isError"], false, "{restarted}");
    again.close();
    drop(stdin);
    let _ = child.wait();
}
