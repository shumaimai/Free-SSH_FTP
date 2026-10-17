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
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 6);
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
