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

/// MCPクライアントに登録するサーバー名。
pub const SERVER_NAME: &str = "hashi-rust-preview";
/// 固定フォルダを差し替える環境変数(試験用)。
pub const DIRECTORY_ENV: &str = "HASHI_MCP_DIR";

/// 接続情報の固定フォルダ。Windowsは%LOCALAPPDATA%、他はXDG_RUNTIME_DIRか~/.local/state。
pub fn default_directory() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(DIRECTORY_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    let home = || {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .context("ホームフォルダが不明です")
    };
    Ok(if cfg!(windows) {
        match std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            Some(value) => PathBuf::from(value),
            None => home()?.join("AppData/Local"),
        }
        .join("Hashi-RustPreview/mcp")
    } else {
        match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(value) => PathBuf::from(value).join("hashi-rust-preview-mcp"),
            None => home()?.join(".local/state/hashi-rust-preview/mcp"),
        }
    })
}

/// 固定フォルダの接続情報ファイル。
pub fn default_endpoint() -> Result<PathBuf> {
    Ok(default_directory()?.join("endpoint.json"))
}

/// Hashi本体と同じフォルダにあるstdioヘルパー。
pub fn helper_path() -> Result<PathBuf> {
    let executable = std::env::current_exe()?.with_file_name(HELPER_NAME);
    if !executable.is_file() {
        bail!("{HELPER_NAME}をHashiと同じフォルダに配置してください");
    }
    Ok(executable)
}

/// Hashiに接続できない間のstdioヘルパーの応答。起動の成否をMCPクライアントに見せず、
/// ツールの呼び出し時に原因と対処を返す。操作は一切実行しない。
pub fn offline_reply(message: &Value) -> Value {
    const OFFLINE: &str = "Hashiに接続できません。Hashiを起動し、「外部連携(MCP)」で「MCPを開始」してから、もう一度お試しください。";
    let Some(id) = message.get("id").filter(|id| !id.is_null()) else {
        return Value::Null;
    };
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => initialize_result(&message["params"]),
        "ping" => json!({}),
        "tools/list" => tool_list(),
        "tools/call" => json!({"content": [{"type": "text", "text": OFFLINE}], "isError": true}),
        _ => {
            return json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}})
        }
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn initialize_result(params: &Value) -> Value {
    let requested = params["protocolVersion"].as_str().unwrap_or_default();
    let version = VERSIONS
        .iter()
        .find(|v| **v == requested)
        .copied()
        .unwrap_or(VERSIONS[0]);
    json!({"protocolVersion": version,
        "capabilities": {"tools": {}}, "serverInfo": {"name": "Hashi", "version": "1"}})
}

fn tool_list() -> Value {
    json!({"tools": tools::definitions().into_iter().map(|d| json!({
        "name": d["name"], "description": d["description"], "inputSchema": d["parameters"]})).collect::<Vec<_>>()})
}

struct Client {
    ready: bool,
    cancel: CancellationToken,
}

pub struct McpBridge {
    broker: Arc<Broker>,
    instance: String,
    token: String,
    directory: PathBuf,
    /// 固定フォルダ(利用者ごとに1つ)を使うとき真。停止時はフォルダではなく自分のendpointだけ消す。
    fixed: bool,
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
    if let Err(error) = restrict_directory(&directory) {
        let _ = std::fs::remove_dir_all(&directory);
        return Err(error);
    }
    Ok(directory)
}

/// 接続情報の固定フォルダ。リンクは辿らず、本人だけが使える権限にしてから使う。
fn fixed_directory(directory: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let meta = std::fs::symlink_metadata(directory)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("MCPの接続情報フォルダが不正です");
    }
    restrict_directory(directory)?;
    Ok(directory.to_owned())
}

fn restrict_directory(directory: &Path) -> Result<()> {
    (|| -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            let mode = std::fs::metadata(directory)?.permissions().mode();
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
                .arg(directory)
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
    })()
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
    /// 接続情報は起動ごとの一時フォルダへ置く(主にテスト用)。
    pub async fn start(broker: Arc<Broker>) -> Result<Arc<Self>> {
        Self::start_in(broker, private_directory()?, false).await
    }

    /// 利用者ごとの固定フォルダへ接続情報を置いて開始する。MCPクライアントへの登録は
    /// ヘルパーのパスだけで済み、Hashiを再起動しても登録し直さなくてよい。
    pub async fn start_fixed(broker: Arc<Broker>) -> Result<Arc<Self>> {
        Self::start_in(broker, fixed_directory(&default_directory()?)?, true).await
    }

    /// 指定した固定フォルダで開始する(`start_fixed`と同じ扱い。フォルダは終了後も残す)。
    pub async fn start_at(broker: Arc<Broker>, directory: &Path) -> Result<Arc<Self>> {
        Self::start_in(broker, fixed_directory(directory)?, true).await
    }

    async fn start_in(broker: Arc<Broker>, directory: PathBuf, fixed: bool) -> Result<Arc<Self>> {
        let started = async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let port = listener.local_addr()?.port();
            let bridge = Arc::new(Self {
                broker,
                instance: random_hex(),
                token: random_hex() + &random_hex(),
                endpoint: directory.join("endpoint.json"),
                directory: directory.clone(),
                fixed,
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
        if started.is_err() && !fixed {
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
    /// 固定フォルダ方式ではヘルパーが接続情報の場所を知っているので、引数は要らない。
    pub fn config(&self) -> Result<Value> {
        let executable = helper_path()?;
        let args: Vec<&Path> = if self.fixed {
            vec![]
        } else {
            vec![&self.endpoint]
        };
        Ok(json!({"mcpServers": {SERVER_NAME: {"type": "stdio",
            "command": executable, "args": args}}}))
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
                clients.insert(
                    client.to_owned(),
                    Client {
                        ready: false,
                        cancel: self.stopped.child_token(),
                    },
                );
                return respond(Ok(initialize_result(&params)));
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
            "tools/list" => Ok(tool_list()),
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
        if !self.fixed {
            let _ = std::fs::remove_dir_all(&self.directory);
        } else if let Ok(raw) = std::fs::read(&self.endpoint) {
            // 別のHashiが書いた接続情報は消さない(自分の起動分だけ消す)。
            let mine = serde_json::from_slice::<Value>(&raw)
                .ok()
                .is_some_and(|v| v["instance"].as_str() == Some(self.instance.as_str()));
            if mine {
                let _ = std::fs::remove_file(&self.endpoint);
            }
        }
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

    #[test]
    fn offline_replies_never_run_tools_and_explain_what_to_do() {
        let init = offline_reply(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let list = offline_reply(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);
        let call = offline_reply(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"send_input","arguments":{}}}));
        assert_eq!(call["result"]["isError"], true);
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("MCPを開始"));
        // notificationには何も返さない。
        assert!(
            offline_reply(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_null()
        );
        assert_eq!(
            offline_reply(&json!({"jsonrpc":"2.0","id":4,"method":"nope"}))["error"]["code"],
            -32601
        );
    }

    #[tokio::test]
    async fn fixed_directory_is_private_reused_and_only_removes_its_own_endpoint() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("mcp");
        let broker = || Arc::new(Broker::new(Arc::new(Registry::default()), None));
        let first = McpBridge::start_at(broker(), &directory).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&directory).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "本人以外に権限を与えない");
        }
        let config = first.config().ok();
        if let Some(config) = config {
            assert_eq!(config["mcpServers"][SERVER_NAME]["args"], json!([]));
        }
        // 後から起動した別のHashiが接続情報を書き換える。先に終わった方は消さない。
        let second = McpBridge::start_at(broker(), &directory).await.unwrap();
        first.close();
        assert!(second.endpoint().is_file(), "他のHashiの接続情報は残す");
        second.close();
        assert!(!second.endpoint().exists());
        assert!(directory.is_dir(), "固定フォルダ自体は残す");
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
