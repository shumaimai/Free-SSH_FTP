//! 起動中のHashiへ接続する認証付きloopback IPCとMCP JSON-RPCの処理（Python版`mcp_bridge.py`相当）。
//! 接続情報は本人だけが使える一時フォルダへ置き、インスタンスID/tokenを毎回照合する。
//! notificationからツールを実行しない。クライアントの切断で承認待ち・実行中の要求を取り消す。
use anyhow::{bail, Context, Result};
use hashi_ai::{observe, tools};
use hashi_core::broker::Broker;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

pub const MAX_MESSAGE: usize = 65536;
pub const MAX_RESPONSE: usize = 262144;
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const HELPER_NAME: &str = if cfg!(windows) {
    "HashiMCP-RustPreview.exe"
} else {
    "HashiMCP-RustPreview"
};

struct Client {
    ready: bool,
    cancel: CancellationToken,
}

pub struct McpBridge {
    broker: Arc<Broker>,
    instance: String,
    token: String,
    directory: PathBuf,
    endpoint: PathBuf,
    port: u16,
    clients: Mutex<HashMap<String, Client>>,
    stopped: CancellationToken,
}

fn random_hex() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 本人だけが使える一時フォルダ。Windowsは継承を外し、本人とSYSTEMだけへ権限を与える。
fn private_directory() -> Result<PathBuf> {
    let directory = tempfile::Builder::new()
        .prefix("hashi-mcp-")
        .tempdir()?
        .keep();
    let result = (|| -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            let mode = std::fs::metadata(&directory)?.permissions().mode();
            if mode & 0o077 != 0 {
                bail!("一時フォルダの権限を制限できません");
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const NO_WINDOW: u32 = 0x0800_0000;
            let output = std::process::Command::new("whoami")
                .args(["/user", "/fo", "csv", "/nh"])
                .creation_flags(NO_WINDOW)
                .output()?;
            let text = String::from_utf8_lossy(&output.stdout);
            let sid = text
                .trim()
                .rsplit(',')
                .next()
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned();
            if !sid.starts_with("S-1-")
                || !sid
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == 'S' || c == '-')
            {
                bail!("ユーザーSIDを確認できません");
            }
            let status = std::process::Command::new("icacls")
                .arg(&directory)
                .args(["/inheritance:r", "/grant:r"])
                .arg(format!("*{sid}:(OI)(CI)F"))
                .arg("*S-1-5-18:(OI)(CI)F")
                .creation_flags(NO_WINDOW)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
            if !status.success() {
                bail!("一時フォルダの権限を制限できません");
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&directory);
        return Err(error);
    }
    Ok(directory)
}

fn same(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

impl McpBridge {
    /// tokioのruntime上で待受を開始する。接続数は同時8まで。
    pub async fn start(broker: Arc<Broker>) -> Result<Arc<Self>> {
        let directory = private_directory()?;
        let started = async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let port = listener.local_addr()?.port();
            let bridge = Arc::new(Self {
                broker,
                instance: random_hex(),
                token: random_hex() + &random_hex(),
                endpoint: directory.join("endpoint.json"),
                directory: directory.clone(),
                port,
                clients: Mutex::new(HashMap::new()),
                stopped: CancellationToken::new(),
            });
            let content = json!({"instance": bridge.instance, "token": bridge.token, "port": port});
            let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
            std::io::Write::write_all(&mut temp, content.to_string().as_bytes())?;
            temp.as_file().sync_all()?;
            temp.persist(&bridge.endpoint)
                .map_err(|error| error.error)?;
            let server = bridge.clone();
            tokio::spawn(async move { server.serve(listener).await });
            anyhow::Ok(bridge)
        }
        .await;
        if started.is_err() {
            let _ = std::fs::remove_dir_all(&directory);
        }
        started
    }
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    /// MCPクライアントへ登録するstdio設定。ヘルパーはHashiと同じフォルダに置く。
    pub fn config(&self) -> Result<Value> {
        let executable = std::env::current_exe()?.with_file_name(HELPER_NAME);
        if !executable.is_file() {
            bail!("{HELPER_NAME}をHashiと同じフォルダに配置してください");
        }
        Ok(
            json!({"mcpServers": {"hashi-rust-preview": {"type": "stdio",
            "command": executable, "args": [self.endpoint]}}}),
        )
    }
    async fn serve(self: Arc<Self>, listener: TcpListener) {
        let slots = Arc::new(Semaphore::new(8));
        loop {
            let accepted = tokio::select! {
                _ = self.stopped.cancelled() => return,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else { continue };
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                continue; // 上限超過は接続を閉じる
            };
            let bridge = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = bridge.handle(stream).await;
            });
        }
    }
    async fn handle(self: Arc<Self>, stream: tokio::net::TcpStream) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            (&mut reader)
                .take(MAX_MESSAGE as u64 + 1)
                .read_until(b'\n', &mut line),
        )
        .await??;
        if line.len() > MAX_MESSAGE {
            return Ok(());
        }
        let envelope: Value = serde_json::from_slice(&line)?;
        if self.stopped.is_cancelled()
            || envelope["instance"].as_str() != Some(self.instance.as_str())
            || !same(envelope["token"].as_str().unwrap_or_default(), &self.token)
        {
            return Ok(()); // 要求本文・tokenを診断に出さない
        }
        let Some(client) = envelope["client"]
            .as_str()
            .filter(|c| {
                c.len() == 32
                    && c.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let message = envelope["message"].clone();
        let dispatch = self.dispatch(&client, &message);
        tokio::pin!(dispatch);
        let mut closed = false;
        let mut probe = [0u8; 1];
        // 実行中にstdioヘルパーが切れたら要求を取り消す。Futureは捨てずに完了まで待つ。
        let reply = loop {
            tokio::select! {
                reply = &mut dispatch => break reply,
                read = reader.read(&mut probe), if !closed => {
                    if matches!(read, Ok(0) | Err(_)) {
                        closed = true;
                        self.disconnect(&client);
                    }
                }
            }
        };
        if !closed {
            let mut raw = serde_json::to_vec(&json!({"response": reply}))?;
            raw.push(b'\n');
            writer.write_all(&raw).await?;
        }
        Ok(())
    }
    pub fn disconnect(&self, client: &str) {
        if let Some(state) = self.clients.lock().unwrap().remove(client) {
            state.cancel.cancel();
        }
        self.broker.cancel_actor(&self.actor(client));
    }
    fn actor(&self, client: &str) -> String {
        format!("mcp:{}:{client}", self.instance)
    }

    pub async fn dispatch(&self, client: &str, message: &Value) -> Value {
        let invalid = || json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Invalid request"}});
        if !message.is_object() || message["jsonrpc"] != "2.0" {
            return invalid();
        }
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message["method"].as_str().unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() || !(id.is_null() || id.is_string() || id.is_i64() || id.is_u64()) {
            return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600, "message": "Invalid request"}});
        }
        let respond = |result: Result<Value>| -> Value {
            if id.is_null() {
                return Value::Null;
            }
            match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32000, "message": observe::redact(&error.to_string())}}),
            }
        };
        let notification = method.starts_with("notifications/");
        let cancel = {
            let mut clients = self.clients.lock().unwrap();
            if self.stopped.is_cancelled() {
                return respond(Err(anyhow::anyhow!("Hashi MCPは停止しています")));
            }
            if method == "notifications/hashi-disconnect" {
                drop(clients);
                self.disconnect(client);
                return Value::Null;
            }
            if id.is_null() && !notification {
                return Value::Null; // notificationからツールを実行しない
            }
            if method == "initialize" {
                if clients.len() >= 64 && !clients.contains_key(client) {
                    return respond(Err(anyhow::anyhow!("接続数の上限です")));
                }
                if clients.contains_key(client) {
                    return respond(Err(anyhow::anyhow!("このクライアントは初期化済みです")));
                }
                let requested = params["protocolVersion"].as_str().unwrap_or_default();
                let version = VERSIONS
                    .iter()
                    .find(|v| **v == requested)
                    .copied()
                    .unwrap_or(VERSIONS[0]);
                clients.insert(
                    client.to_owned(),
                    Client {
                        ready: false,
                        cancel: self.stopped.child_token(),
                    },
                );
                return respond(Ok(json!({"protocolVersion": version,
                    "capabilities": {"tools": {}}, "serverInfo": {"name": "Hashi", "version": "1"}})));
            }
            let Some(state) = clients.get_mut(client) else {
                return respond(Err(anyhow::anyhow!("初期化してください")));
            };
            if method == "notifications/initialized" {
                state.ready = true;
                return Value::Null;
            }
            if method == "notifications/cancelled" {
                let request = match &params["requestId"] {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                drop(clients);
                self.broker.cancel_operation(&self.actor(client), &request);
                return Value::Null;
            }
            if !state.ready {
                return respond(Err(anyhow::anyhow!("初期化が完了していません")));
            }
            state.cancel.clone()
        };
        if notification {
            return Value::Null;
        }
        let result = match method {
            "ping" => Ok(json!({})),
            "tools/list" => Ok(
                json!({"tools": tools::definitions().into_iter().map(|d| json!({
                "name": d["name"], "description": d["description"], "inputSchema": d["parameters"]})).collect::<Vec<_>>()}),
            ),
            "tools/call" => {
                let tools = tools::TerminalTools {
                    broker: self.broker.clone(),
                    actor: self.actor(client),
                    cancel,
                };
                let request = match &id {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                let name = params["name"].as_str().unwrap_or_default();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                Ok(match tools.call(name, &arguments, Some(request)).await {
                    Ok(result) => json!({"content": [{"type": "text",
                        "text": observe::observation_json(&result, 32000)}], "isError": false}),
                    Err(error) => json!({"content": [{"type": "text",
                        "text": observe::redact(&error.to_string())}], "isError": true}),
                })
            }
            _ => {
                return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})
            }
        };
        respond(result)
    }
    /// 停止。操作許可を外し、待受を閉じ、接続情報を削除する。
    pub fn close(&self) {
        if self.stopped.is_cancelled() {
            return;
        }
        self.stopped.cancel();
        self.broker.stop();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Drop for McpBridge {
    fn drop(&mut self) {
        self.close();
    }
}

/// stdioヘルパー側: 1要求を1接続で送り、応答を返す。
pub fn request(endpoint: &Value, client: &str, message: &Value) -> Result<Value> {
    use std::io::{BufRead, Read, Write};
    let mut envelope = endpoint.clone();
    envelope["client"] = json!(client);
    envelope["message"] = message.clone();
    let mut raw = serde_json::to_vec(&envelope)?;
    raw.push(b'\n');
    if raw.len() > MAX_MESSAGE {
        bail!("MCP要求が大きすぎます");
    }
    let port = endpoint["port"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= 65535)
        .context("接続情報が不正です")?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port as u16));
    let mut connection = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    connection.set_read_timeout(Some(Duration::from_secs(180)))?;
    connection.write_all(&raw)?;
    let mut line = Vec::new();
    std::io::BufReader::new(connection)
        .take(MAX_RESPONSE as u64 + 1)
        .read_until(b'\n', &mut line)?;
    if line.is_empty() || line.len() > MAX_RESPONSE {
        bail!("起動中のHashiへ接続できません");
    }
    let response: Value = serde_json::from_slice(&line)?;
    Ok(response["response"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashi_core::broker::{Kind, Mode, Registration, Registry};

    async fn bridge() -> (
        Arc<McpBridge>,
        Arc<Broker>,
        String,
        u64,
        tokio::sync::mpsc::Receiver<Vec<u8>>,
    ) {
        let registry = Arc::new(Registry::default());
        let (input, received) = tokio::sync::mpsc::channel(8);
        let (id, generation) = registry.register(
            None,
            Registration {
                label: "CMD".into(),
                kind: Kind::Local,
                shell: "unknown".into(),
                cwd: None,
                shareable: true,
                input,
                ssh: None,
            },
        );
        let broker = Arc::new(Broker::new(registry, None));
        (
            McpBridge::start(broker.clone()).await.unwrap(),
            broker,
            id,
            generation,
            received,
        )
    }

    async fn call(endpoint: &Value, client: &str, message: Value) -> Value {
        let endpoint = endpoint.clone();
        let client = client.to_owned();
        tokio::task::spawn_blocking(move || request(&endpoint, &client, &message).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn authenticated_protocol_flow_and_endpoint_privacy() {
        let (bridge, broker, id, generation, mut received) = bridge().await;
        let endpoint: Value =
            serde_json::from_slice(&std::fs::read(bridge.endpoint()).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(bridge.endpoint().parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let client = random_hex();
        // 初期化前の操作・notificationからの実行・誤tokenを拒否する。
        let early = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
        assert!(early["error"]["message"]
            .as_str()
            .unwrap()
            .contains("初期化"));
        let mut forged = endpoint.clone();
        forged["token"] = json!("wrong");
        let forged_result = tokio::task::spawn_blocking({
            let client = client.clone();
            move || {
                request(
                    &forged,
                    &client,
                    &json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
                )
            }
        })
        .await
        .unwrap();
        assert!(forged_result.is_err(), "誤tokenには応答しない");
        let init = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18"}}),
        )
        .await;
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let pending = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        )
        .await;
        assert!(pending["error"].is_object());
        assert!(call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .is_null());
        let list = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        )
        .await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);
        let args = json!({"session_id": id, "generation": generation, "text": "ls", "enter": true});
        let note = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","method":"tools/call",
            "params":{"name":"send_input","arguments":args}}),
        )
        .await;
        assert!(note.is_null());
        assert!(received.try_recv().is_err(), "notificationから実行しない");
        let denied = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"send_input","arguments":args}}),
        )
        .await;
        assert_eq!(
            denied["result"]["isError"], true,
            "利用者が許可するまで操作しない"
        );
        broker
            .configure(
                Mode::Auto,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let sent = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call",
            "params":{"name":"send_input","arguments":args}}),
        )
        .await;
        assert_eq!(sent["result"]["isError"], false);
        assert_eq!(received.recv().await.unwrap(), b"ls\r");
        let unknown = call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":6,"method":"resources/list"}),
        )
        .await;
        assert_eq!(unknown["error"]["code"], -32601);
        let directory = bridge.endpoint().parent().unwrap().to_owned();
        bridge.close();
        assert!(!directory.exists(), "停止で接続情報を削除する");
        assert_eq!(broker.permission_snapshot()["mode"], "consult");
    }

    #[tokio::test]
    async fn client_disconnect_cancels_pending_approval() {
        let registry = Arc::new(Registry::default());
        let (input, _received) = tokio::sync::mpsc::channel(8);
        let (id, generation) = registry.register(
            None,
            Registration {
                label: "CMD".into(),
                kind: Kind::Local,
                shell: "unknown".into(),
                cwd: None,
                shareable: true,
                input,
                ssh: None,
            },
        );
        let (approvals, mut asks) = tokio::sync::mpsc::channel(4);
        let broker = Arc::new(Broker::new(registry, Some(approvals)));
        broker
            .configure(
                Mode::Confirm,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let bridge = McpBridge::start(broker.clone()).await.unwrap();
        let endpoint: Value =
            serde_json::from_slice(&std::fs::read(bridge.endpoint()).unwrap()).unwrap();
        let client = random_hex();
        call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await;
        call(
            &endpoint,
            &client,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        // 生の接続で要求を送り、承認待ちの間に接続を切る。
        let mut envelope = endpoint.clone();
        envelope["client"] = json!(client);
        envelope["message"] = json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"send_input",
            "arguments":{"session_id": id, "generation": generation, "text": "rm -rf /", "enter": true}}});
        let mut raw = serde_json::to_vec(&envelope).unwrap();
        raw.push(b'\n');
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", bridge.port()))
            .await
            .unwrap();
        stream.write_all(&raw).await.unwrap();
        let mut ask = asks.recv().await.unwrap();
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), ask.reply.closed())
            .await
            .unwrap();
        for _ in 0..50 {
            if broker.audit_snapshot()["active"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(broker.audit_snapshot()["active"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(broker.audit_snapshot()["finished"][0]["cancelled"], true);
    }
}
