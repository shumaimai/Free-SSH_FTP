//! AI/MCPからの端末操作を、端末ID・接続世代・利用者の許可で仲介する（Python版`command_broker.py`/
//! `session_registry.py`相当）。許可はメモリ内だけで、モデルやMCPツールから実行モードを変更できない。
//! 本文・出力・認証情報を監査記録へ入れない。同じactor/要求IDは二重実行しない。
use anyhow::{bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 64 * 1024;
const TEXT_LIMIT: usize = 16000;
const HISTORY: usize = 500;
const RUN_TAIL: usize = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Local,
    Ssh,
}

/// 末尾`limit`文字を残す（文字境界で切る）。
fn keep_chars(text: &mut String, limit: usize) {
    let count = text.chars().count();
    if count > limit {
        let cut = text
            .char_indices()
            .nth(count - limit)
            .map_or(0, |(index, _)| index);
        text.drain(..cut);
    }
}

/// 受信バイト列からCSI/OSCを除いたテキストにする。UTF-8や制御列がチャンク境界で分かれても崩さない。
#[derive(Default)]
pub struct OutputCapture {
    pending: Vec<u8>,
    state: Escape,
}
#[derive(Default, Clone, Copy, PartialEq)]
enum Escape {
    #[default]
    Text,
    Esc,
    Csi,
    Osc,
    OscEsc,
}
impl OutputCapture {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        let mut text = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (Escape::Text, 0x1b) => Escape::Esc,
                (Escape::Text, _) => {
                    text.push(byte);
                    Escape::Text
                }
                (Escape::Esc, b'[') => Escape::Csi,
                (Escape::Esc, b']') => Escape::Osc,
                (Escape::Esc, _) => Escape::Text,
                (Escape::Csi, 0x40..=0x7e) => Escape::Text,
                (Escape::Csi, _) => Escape::Csi,
                (Escape::Osc, 0x07) => Escape::Text,
                (Escape::Osc, 0x1b) => Escape::OscEsc,
                (Escape::Osc, _) => Escape::Osc,
                (Escape::OscEsc, b'\\') => Escape::Text,
                (Escape::OscEsc, _) => Escape::Osc,
            };
        }
        self.pending.extend_from_slice(&text);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let rest = self.pending.split_off(valid);
        String::from_utf8_lossy(&std::mem::replace(&mut self.pending, rest)).into_owned()
    }
}

struct Entry {
    generation: u64,
    label: String,
    kind: Kind,
    shell: String,
    cwd: Option<String>,
    connected: bool,
    shareable: bool,
    output: String,
    screen: String,
    input: mpsc::Sender<Vec<u8>>,
    ssh: Option<Arc<crate::ssh::SshSession>>,
    busy: Arc<tokio::sync::Mutex<()>>,
}
impl Entry {
    fn public(&self, id: &str) -> Value {
        json!({"session_id": id, "generation": self.generation, "label": self.label,
               "kind": self.kind, "shell": self.shell, "cwd": self.cwd,
               "connected": self.connected})
    }
}
#[derive(Clone)]
struct Resolved {
    input: mpsc::Sender<Vec<u8>>,
    kind: Kind,
    ssh: Option<Arc<crate::ssh::SshSession>>,
    busy: Arc<tokio::sync::Mutex<()>>,
    label: String,
}

/// 端末台帳。認証情報やGUIに依存しない。
#[derive(Default)]
pub struct Registry {
    inner: Mutex<(HashMap<String, Entry>, HashMap<String, u64>)>,
}
pub struct Registration {
    pub label: String,
    pub kind: Kind,
    /// 推測できないものは"unknown"（SSHのPTY種別からshellを推測しない）。
    pub shell: String,
    pub cwd: Option<String>,
    pub shareable: bool,
    pub input: mpsc::Sender<Vec<u8>>,
    pub ssh: Option<Arc<crate::ssh::SshSession>>,
}
impl Registry {
    /// 同じIDを再登録すると世代が上がり、再接続前の要求は世代不一致で拒否される。
    pub fn register(
        &self,
        session_id: Option<String>,
        registration: Registration,
    ) -> (String, u64) {
        let mut inner = self.inner.lock().unwrap();
        let id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let generation = inner.1.get(&id).copied().unwrap_or(0) + 1;
        inner.1.insert(id.clone(), generation);
        inner.0.insert(
            id.clone(),
            Entry {
                generation,
                label: registration.label,
                kind: registration.kind,
                shell: registration.shell,
                cwd: registration.cwd,
                connected: true,
                shareable: registration.shareable,
                output: String::new(),
                screen: String::new(),
                input: registration.input,
                ssh: registration.ssh,
                busy: Default::default(),
            },
        );
        (id, generation)
    }
    fn resolve(&self, id: &str, generation: u64) -> Result<(Resolved, bool)> {
        let inner = self.inner.lock().unwrap();
        let Some(entry) = inner.0.get(id).filter(|entry| entry.connected) else {
            bail!("端末は閉じているか、接続が切れています");
        };
        if entry.generation != generation {
            bail!("再接続前の端末への要求は実行できません");
        }
        Ok((
            Resolved {
                input: entry.input.clone(),
                kind: entry.kind,
                ssh: entry.ssh.clone(),
                busy: entry.busy.clone(),
                label: entry.label.clone(),
            },
            entry.shareable,
        ))
    }
    pub fn disconnect(&self, id: &str, generation: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.0.get_mut(id) {
            if entry.generation == generation {
                entry.connected = false;
            }
        }
    }
    /// 世代は残す。同じIDを再登録しても古い要求を受け付けない。
    pub fn remove(&self, id: &str) {
        self.inner.lock().unwrap().0.remove(id);
    }
    pub fn list(&self, shareable_only: bool) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        let mut list: Vec<_> = inner
            .0
            .iter()
            .filter(|(_, entry)| entry.connected && (entry.shareable || !shareable_only))
            .map(|(id, entry)| entry.public(id))
            .collect();
        list.sort_by(|a, b| a["label"].as_str().cmp(&b["label"].as_str()));
        list
    }
    pub fn update_output(&self, id: &str, generation: u64, text: &str, screen: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.0.get_mut(id) {
            if entry.generation == generation && entry.connected {
                entry.output.push_str(text);
                keep_chars(&mut entry.output, OUTPUT_LIMIT);
                entry.screen = screen.to_owned();
                keep_chars(&mut entry.screen, OUTPUT_LIMIT);
            }
        }
    }
    pub fn read_output(&self, id: &str, generation: u64, limit: usize) -> Result<Value> {
        self.resolve(id, generation)?;
        let inner = self.inner.lock().unwrap();
        let entry = &inner.0[id];
        let limit = limit.clamp(1, OUTPUT_LIMIT);
        let mut output = entry.output.clone();
        keep_chars(&mut output, limit);
        let mut screen = entry.screen.clone();
        keep_chars(&mut screen, limit);
        let mut value = entry.public(id);
        value["output"] = json!(output);
        value["screen"] = json!(screen);
        value["completion"] = json!("unknown");
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 相談のみ。実行しない。
    Consult,
    /// 毎回利用者が確認する。
    Confirm,
    /// 指定端末で自動実行する。
    Auto,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SendInput,
    /// Ctrl+X・Esc・矢印などの特殊キー(vi/nano等の操作用)。
    SendKeys,
    RunCommand,
    Interrupt,
}
impl Action {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "send_input" => Self::SendInput,
            "send_keys" => Self::SendKeys,
            "run_command" => Self::RunCommand,
            "interrupt" => Self::Interrupt,
            _ => bail!("不明な端末操作"),
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::SendInput => "send_input",
            Self::SendKeys => "send_keys",
            Self::RunCommand => "run_command",
            Self::Interrupt => "interrupt",
        }
    }
}

/// 1回に送れるキー数。
const KEYS_LIMIT: usize = 32;

/// 空白区切りのキー名を端末へ送るバイト列にする。未知のキー名は丸ごと拒否する。
/// 例: `Ctrl+X`, `Esc`, `Up`, `Alt+x`, `Shift+Tab`, `F5`。
/// 通常の文字入力はsend_inputで行い、ここでは名前のあるキーだけを扱う。
pub fn keys_to_bytes(spec: &str) -> Result<Vec<u8>> {
    let names: Vec<&str> = spec.split_whitespace().collect();
    if names.is_empty() || names.len() > KEYS_LIMIT {
        bail!("キーは1〜{KEYS_LIMIT}個まで指定できます");
    }
    let mut out = Vec::new();
    for name in names {
        out.extend(key_bytes(name)?);
    }
    Ok(out)
}

fn key_bytes(name: &str) -> Result<Vec<u8>> {
    let lower = name.to_ascii_lowercase();
    let fixed: Option<&[u8]> = match lower.as_str() {
        "enter" | "return" => Some(b"\r"),
        "esc" | "escape" => Some(b"\x1b"),
        "tab" => Some(b"\t"),
        "shift+tab" | "backtab" => Some(b"\x1b[Z"),
        "backspace" => Some(b"\x7f"),
        "space" => Some(b" "),
        "delete" | "del" => Some(b"\x1b[3~"),
        "insert" => Some(b"\x1b[2~"),
        "up" => Some(b"\x1b[A"),
        "down" => Some(b"\x1b[B"),
        "right" => Some(b"\x1b[C"),
        "left" => Some(b"\x1b[D"),
        "home" => Some(b"\x1b[H"),
        "end" => Some(b"\x1b[F"),
        "pageup" | "pgup" => Some(b"\x1b[5~"),
        "pagedown" | "pgdn" => Some(b"\x1b[6~"),
        "f1" => Some(b"\x1bOP"),
        "f2" => Some(b"\x1bOQ"),
        "f3" => Some(b"\x1bOR"),
        "f4" => Some(b"\x1bOS"),
        "f5" => Some(b"\x1b[15~"),
        "f6" => Some(b"\x1b[17~"),
        "f7" => Some(b"\x1b[18~"),
        "f8" => Some(b"\x1b[19~"),
        "f9" => Some(b"\x1b[20~"),
        "f10" => Some(b"\x1b[21~"),
        "f11" => Some(b"\x1b[23~"),
        "f12" => Some(b"\x1b[24~"),
        _ => None,
    };
    if let Some(bytes) = fixed {
        return Ok(bytes.to_vec());
    }
    if let Some(rest) = lower.strip_prefix("ctrl+") {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return match c {
                'a'..='z' => Ok(vec![c as u8 - b'a' + 1]),
                '[' => Ok(vec![0x1b]),
                '\\' => Ok(vec![0x1c]),
                ']' => Ok(vec![0x1d]),
                '^' => Ok(vec![0x1e]),
                '_' | '/' => Ok(vec![0x1f]),
                '@' | ' ' => Ok(vec![0x00]),
                _ => bail!("未対応のキーです: {name}"),
            };
        }
    }
    if let Some(rest) = name
        .strip_prefix("Alt+")
        .or_else(|| name.strip_prefix("alt+"))
    {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_graphic() {
                return Ok(vec![0x1b, c as u8]);
            }
        }
    }
    bail!("未対応のキーです: {name}")
}

/// 確認モードで画面へ出す依頼。返答がないまま破棄されたら拒否扱い。
pub struct ApprovalRequest {
    pub actor: String,
    pub operation_id: String,
    pub action: Action,
    pub session_id: String,
    pub generation: u64,
    pub label: String,
    pub text: String,
    pub cwd: Option<String>,
    pub reply: oneshot::Sender<bool>,
}

pub struct Request {
    pub actor: String,
    pub action: Action,
    pub session_id: String,
    pub generation: u64,
    pub text: String,
    pub cwd: Option<String>,
    pub request_id: Option<String>,
    pub timeout: Duration,
    pub cancel: Option<CancellationToken>,
}

struct Active {
    action: Action,
    session_id: String,
    generation: u64,
    cancel: CancellationToken,
}
struct State {
    mode: Mode,
    targets: HashSet<(String, u64)>,
    expires: Instant,
    epoch: u64,
    active: HashMap<(String, String), Active>,
    results: HashMap<(String, String), (String, Value)>,
    order: VecDeque<(String, String)>,
    audit: VecDeque<Value>,
}

pub struct Broker {
    pub registry: Arc<Registry>,
    approval: Option<mpsc::Sender<ApprovalRequest>>,
    state: Mutex<State>,
}
impl Broker {
    pub fn new(registry: Arc<Registry>, approval: Option<mpsc::Sender<ApprovalRequest>>) -> Self {
        Self {
            registry,
            approval,
            state: Mutex::new(State {
                mode: Mode::Consult,
                targets: HashSet::new(),
                expires: Instant::now(),
                epoch: 0,
                active: HashMap::new(),
                results: HashMap::new(),
                order: VecDeque::new(),
                audit: VecDeque::new(),
            }),
        }
    }
    /// 利用者が画面で選んだ対象と期限（1秒〜1時間）。進行中の要求は取り消す。
    pub fn configure(
        &self,
        mode: Mode,
        targets: &[(String, u64)],
        lifetime: Duration,
    ) -> Result<()> {
        let mut verified = HashSet::new();
        for (id, generation) in targets {
            let (_, shareable) = self.registry.resolve(id, *generation)?;
            if !shareable {
                bail!("この端末は外部操作を許可していません");
            }
            verified.insert((id.clone(), *generation));
        }
        self.stop();
        let mut state = self.state.lock().unwrap();
        state.mode = mode;
        state.targets = verified;
        state.expires =
            Instant::now() + lifetime.clamp(Duration::from_secs(1), Duration::from_secs(3600));
        Ok(())
    }
    /// 停止。相談のみへ戻し、対象を外し、進行中・承認待ちの要求をすべて取り消す。
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.epoch += 1;
        state.mode = Mode::Consult;
        state.targets.clear();
        for active in state.active.values() {
            active.cancel.cancel();
        }
    }
    pub fn cancel_operation(&self, actor: &str, operation_id: &str) -> Value {
        let state = self.state.lock().unwrap();
        match state
            .active
            .get(&(actor.to_owned(), operation_id.to_owned()))
        {
            Some(active) => {
                active.cancel.cancel();
                json!({"status": "cancel_requested", "completion": "unknown"})
            }
            None => json!({"status": "not_running", "completion": "unknown"}),
        }
    }
    /// MCP切断などで、そのactorの承認待ち・実行中を取り消す。
    pub fn cancel_actor(&self, actor: &str) {
        let state = self.state.lock().unwrap();
        for ((owner, _), active) in &state.active {
            if owner == actor {
                active.cancel.cancel();
            }
        }
    }
    pub fn audit_snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        let active: Vec<_> = state
            .active
            .iter()
            .map(|((actor, id), active)| {
                json!({"id": id, "actor": actor, "action": active.action.name(),
                       "session_id": active.session_id, "generation": active.generation,
                       "status": if active.cancel.is_cancelled() {"cancel_requested"} else {"pending"}})
            })
            .collect();
        json!({"active": active, "finished": state.audit.iter().collect::<Vec<_>>()})
    }
    fn check(&self, state: &State, id: &str, generation: u64) -> Result<Resolved> {
        let (resolved, shareable) = self.registry.resolve(id, generation)?;
        if !shareable {
            bail!("この端末は外部操作の対象外です");
        }
        if !state.targets.contains(&(id.to_owned(), generation)) || Instant::now() >= state.expires
        {
            bail!("端末の共有許可がありません。対象端末を選び直してください");
        }
        Ok(resolved)
    }
    pub fn list_sessions(&self) -> Vec<Value> {
        let state = self.state.lock().unwrap();
        if Instant::now() >= state.expires {
            return vec![];
        }
        self.registry
            .list(true)
            .into_iter()
            .filter(|session| {
                let key = (
                    session["session_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    session["generation"].as_u64().unwrap_or_default(),
                );
                state.targets.contains(&key)
            })
            .collect()
    }
    pub fn permission_snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        let mode = match state.mode {
            Mode::Consult => "consult",
            Mode::Confirm => "confirm",
            Mode::Auto => "auto",
        };
        json!({"mode": mode, "seconds": state.expires.saturating_duration_since(Instant::now()).as_secs()})
    }
    pub fn read_output(&self, id: &str, generation: u64, limit: usize) -> Result<Value> {
        let state = self.state.lock().unwrap();
        self.check(&state, id, generation)?;
        self.registry.read_output(id, generation, limit)
    }

    pub async fn perform(&self, request: Request) -> Result<Value> {
        if request.text.chars().count() > TEXT_LIMIT || request.text.contains('\0') {
            bail!("入力が長すぎるか不正です");
        }
        // キー名の誤りは、利用者へ確認を出す前に拒否する。
        if request.action == Action::SendKeys {
            keys_to_bytes(&request.text)?;
        }
        let operation_id = request
            .request_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let key = (request.actor.clone(), operation_id.clone());
        let fingerprint = hex(&Sha256::digest(serde_json::to_vec(&json!([
            request.action.name(),
            request.session_id,
            request.generation,
            request.text,
            request.cwd
        ]))?));
        let cancel = request
            .cancel
            .as_ref()
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let (resolved, epoch, mode) = {
            let mut state = self.state.lock().unwrap();
            let resolved = self.check(&state, &request.session_id, request.generation)?;
            if state.mode == Mode::Consult {
                bail!("相談のみモードでは実行できません");
            }
            if let Some((previous, result)) = state.results.get(&key) {
                if previous != &fingerprint {
                    bail!("同じ要求IDの内容が変わっています");
                }
                return Ok(result.clone());
            }
            if state.active.contains_key(&key) {
                bail!("同じ要求は実行中です");
            }
            state.active.insert(
                key.clone(),
                Active {
                    action: request.action,
                    session_id: request.session_id.clone(),
                    generation: request.generation,
                    cancel: cancel.clone(),
                },
            );
            (resolved, state.epoch, state.mode)
        };
        // Futureが途中で破棄されても実行中の記録を残さない。
        struct ActiveGuard<'a>(&'a Broker, (String, String));
        impl Drop for ActiveGuard<'_> {
            fn drop(&mut self) {
                if let Ok(mut state) = self.0.state.lock() {
                    state.active.remove(&self.1);
                }
            }
        }
        let guard = ActiveGuard(self, key.clone());
        let result = self
            .execute(&request, &operation_id, resolved, epoch, mode, &cancel)
            .await;
        drop(guard);
        let mut state = self.state.lock().unwrap();
        if let Ok(value) = &result {
            state
                .results
                .insert(key.clone(), (fingerprint, value.clone()));
            state.order.push_back(key);
            if state.order.len() > HISTORY {
                if let Some(old) = state.order.pop_front() {
                    state.results.remove(&old);
                }
            }
        }
        // コマンド本文・出力・認証情報を監査ログへ保存しない。
        let value = result.as_ref().ok();
        state.audit.push_back(json!({
            "id": operation_id, "actor": request.actor, "action": request.action.name(),
            "session_id": request.session_id, "generation": request.generation,
            "cancelled": cancel.is_cancelled(),
            "status": value.and_then(|v| v["status"].as_str()).unwrap_or("not_completed"),
            "completion": value.and_then(|v| v["completion"].as_str()).unwrap_or("unknown"),
            "exit_code": value.map(|v| v["exit_code"].clone()).unwrap_or(Value::Null),
        }));
        if state.audit.len() > HISTORY {
            state.audit.pop_front();
        }
        result
    }

    async fn execute(
        &self,
        request: &Request,
        operation_id: &str,
        resolved: Resolved,
        epoch: u64,
        mode: Mode,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        if mode == Mode::Confirm {
            let Some(approval) = &self.approval else {
                bail!("ユーザーが実行を許可しませんでした");
            };
            let (reply, answer) = oneshot::channel();
            let ask = ApprovalRequest {
                actor: request.actor.clone(),
                operation_id: operation_id.to_owned(),
                action: request.action,
                session_id: request.session_id.clone(),
                generation: request.generation,
                label: resolved.label.clone(),
                text: request.text.clone(),
                cwd: request.cwd.clone(),
                reply,
            };
            let approved = tokio::select! {
                biased;
                _ = cancel.cancelled() => false,
                sent = approval.send(ask) => sent.is_ok() && tokio::select! {
                    biased;
                    _ = cancel.cancelled() => false,
                    answer = answer => answer.unwrap_or(false),
                },
            };
            if !approved {
                bail!("ユーザーが実行を許可しませんでした");
            }
        }
        let busy = resolved.busy.clone();
        let Ok(_busy) = busy.try_lock() else {
            bail!("この端末は別の操作を実行中です");
        };
        {
            let state = self.state.lock().unwrap();
            self.check(&state, &request.session_id, request.generation)?;
            if epoch != state.epoch || cancel.is_cancelled() {
                bail!("停止または許可変更により取り消されました");
            }
        }
        // 対話入力は送信成功だけを返し、終了コードやコマンド完了を推測しない。
        let send = |bytes: Vec<u8>| {
            resolved.input.try_send(bytes).map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => anyhow::anyhow!("端末の入力待ちが満杯です"),
                mpsc::error::TrySendError::Closed(_) => anyhow::anyhow!("端末は閉じています"),
            })
        };
        match request.action {
            Action::SendInput => {
                send(request.text.replace('\n', "\r").into_bytes())?;
                Ok(json!({"status": "sent", "completion": "unknown", "exit_code": null}))
            }
            Action::SendKeys => {
                send(keys_to_bytes(&request.text)?)?;
                Ok(json!({"status": "sent", "completion": "unknown", "exit_code": null}))
            }
            Action::Interrupt => {
                send(vec![0x03])?;
                Ok(json!({"status": "interrupt_sent", "completion": "unknown"}))
            }
            Action::RunCommand => {
                let timeout = request
                    .timeout
                    .clamp(Duration::from_secs(1), Duration::from_secs(120));
                self.run(request, resolved, timeout, cancel).await
            }
        }
    }

    async fn run(
        &self,
        request: &Request,
        resolved: Resolved,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        match resolved.kind {
            Kind::Ssh => {
                let Some(ssh) = resolved.ssh else {
                    bail!("独立コマンド実行に対応しないSSH端末です");
                };
                let run = ssh
                    .run_command(&request.text, timeout, cancel, RUN_TAIL)
                    .await?;
                let known = run.status == "completed";
                Ok(
                    json!({"status": run.status, "completion": if known {"known"} else {"unknown"},
                          "exit_code": run.exit_code.filter(|_| known), "output": text(&run.output),
                          "error": text(&run.error)}),
                )
            }
            Kind::Local => {
                let Some(cwd) = request.cwd.as_deref().filter(|cwd| !cwd.is_empty()) else {
                    bail!(
                        "独立実行には明示した開始フォルダが必要です。対話端末のcwdは引き継ぎません"
                    );
                };
                let run = crate::local_command::run(
                    &request.text,
                    &PathBuf::from(cwd),
                    timeout,
                    cancel,
                    RUN_TAIL,
                )
                .await?;
                let known = run.status == "completed" && run.output_complete;
                Ok(
                    json!({"status": run.status, "completion": if known {"known"} else {"unknown"},
                          "exit_code": if run.status == "completed" {json!(run.exit_code)} else {Value::Null},
                          "output_complete": run.output_complete,
                          "output": text(&run.output), "error": text(&run.error)}),
                )
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_names_become_terminal_bytes_and_unknown_names_are_rejected() {
        assert_eq!(keys_to_bytes("Ctrl+X").unwrap(), vec![0x18]);
        assert_eq!(keys_to_bytes("ctrl+o Enter Ctrl+X").unwrap(), b"\x0f\r\x18");
        assert_eq!(
            keys_to_bytes("Esc Up Shift+Tab").unwrap(),
            b"\x1b\x1b[A\x1b[Z"
        );
        assert_eq!(keys_to_bytes("Alt+x F5").unwrap(), b"\x1bx\x1b[15~");
        for bad in [
            "",
            "   ",
            "Ctrl+",
            "Ctrl+Shift+X",
            "Hyper",
            "ls",
            "Alt+\u{3042}",
        ] {
            assert!(keys_to_bytes(bad).is_err(), "{bad}");
        }
        assert!(keys_to_bytes(&"Tab ".repeat(33)).is_err());
    }

    use super::*;

    fn setup(
        approval: Option<mpsc::Sender<ApprovalRequest>>,
    ) -> (Arc<Broker>, String, u64, mpsc::Receiver<Vec<u8>>) {
        let registry = Arc::new(Registry::default());
        let (input, received) = mpsc::channel(8);
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
        (
            Arc::new(Broker::new(registry, approval)),
            id,
            generation,
            received,
        )
    }
    fn request(id: &str, generation: u64, action: Action, text: &str) -> Request {
        Request {
            actor: "mcp:1".into(),
            action,
            session_id: id.into(),
            generation,
            text: text.into(),
            cwd: None,
            request_id: Some("r1".into()),
            timeout: Duration::from_secs(5),
            cancel: None,
        }
    }

    #[tokio::test]
    async fn consult_targets_generation_and_expiry_are_enforced() {
        let (broker, id, generation, _rx) = setup(None);
        let err = broker
            .perform(request(&id, generation, Action::SendInput, "ls"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("共有許可"));
        broker
            .configure(
                Mode::Consult,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let err = broker
            .perform(request(&id, generation, Action::SendInput, "ls"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("相談のみ"));
        broker
            .configure(
                Mode::Auto,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        // 再接続（再登録）後の古い世代は拒否する。
        let (input, _rx2) = mpsc::channel(1);
        let (_, newer) = broker.registry.register(
            Some(id.clone()),
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
        assert_eq!(newer, generation + 1);
        let err = broker
            .perform(request(&id, generation, Action::SendInput, "ls"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("再接続前"));
        let err = broker
            .perform(request(&id, newer, Action::SendInput, "ls"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("共有許可"),
            "新しい世代も明示選択が必要"
        );
        assert!(broker.list_sessions().is_empty());
    }

    #[tokio::test]
    async fn auto_send_is_deduplicated_and_audit_has_no_body() {
        let (broker, id, generation, mut rx) = setup(None);
        broker
            .configure(
                Mode::Auto,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let first = broker
            .perform(request(&id, generation, Action::SendInput, "echo 秘密\n"))
            .await
            .unwrap();
        assert_eq!(first["status"], "sent");
        assert_eq!(rx.recv().await.unwrap(), "echo 秘密\r".as_bytes());
        let again = broker
            .perform(request(&id, generation, Action::SendInput, "echo 秘密\n"))
            .await
            .unwrap();
        assert_eq!(again, first);
        assert!(rx.try_recv().is_err(), "同じ要求IDを二重送信しない");
        let err = broker
            .perform(request(&id, generation, Action::SendInput, "other"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("内容が変わって"));
        let audit = broker.audit_snapshot().to_string();
        assert!(!audit.contains("秘密") && audit.contains("send_input"));
        assert_eq!(broker.list_sessions().len(), 1);
        broker.stop();
        assert_eq!(broker.permission_snapshot()["mode"], "consult");
        let mut next = request(&id, generation, Action::Interrupt, "");
        next.request_id = Some("r2".into());
        assert!(broker.perform(next).await.is_err());
    }

    #[tokio::test]
    async fn confirm_mode_waits_for_user_and_stop_cancels_pending() {
        let (approvals, mut asks) = mpsc::channel(4);
        let (broker, id, generation, mut rx) = setup(Some(approvals));
        broker
            .configure(
                Mode::Confirm,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let worker = broker.clone();
        let (sid, sgen) = (id.clone(), generation);
        let task = tokio::spawn(async move {
            worker
                .perform(request(&sid, sgen, Action::SendInput, "rm -rf /tmp/x"))
                .await
        });
        let ask = asks.recv().await.unwrap();
        assert_eq!(ask.text, "rm -rf /tmp/x");
        assert_eq!(broker.audit_snapshot()["active"][0]["status"], "pending");
        ask.reply.send(false).unwrap();
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("許可しません"));
        assert!(rx.try_recv().is_err());
        // 承認待ち中の停止は要求を取り消す（承認画面の返答を待ち続けない）。
        let worker = broker.clone();
        let task = tokio::spawn(async move {
            let mut next = request(&id, generation, Action::SendInput, "ls");
            next.request_id = Some("r3".into());
            worker.perform(next).await
        });
        let _pending = asks.recv().await.unwrap();
        broker.stop();
        assert!(task.await.unwrap().is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_run_requires_explicit_cwd_and_returns_exit_code() {
        let (broker, id, generation, _rx) = setup(None);
        broker
            .configure(
                Mode::Auto,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let err = broker
            .perform(request(&id, generation, Action::RunCommand, "true"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("開始フォルダ"));
        let directory = tempfile::tempdir().unwrap();
        let mut run = request(&id, generation, Action::RunCommand, "echo out; exit 7");
        run.request_id = Some("run".into());
        run.cwd = Some(directory.path().to_string_lossy().into_owned());
        let result = broker.perform(run).await.unwrap();
        assert_eq!(
            (result["status"].as_str(), result["exit_code"].as_i64()),
            (Some("completed"), Some(7))
        );
        assert_eq!(result["completion"], "known");
        assert_eq!(result["output"], "out\n");
    }

    #[tokio::test]
    async fn dropped_request_does_not_leave_active_operation() {
        let (approvals, mut asks) = mpsc::channel(4);
        let (broker, id, generation, _rx) = setup(Some(approvals));
        broker
            .configure(
                Mode::Confirm,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let worker = broker.clone();
        let (sid, sgen) = (id.clone(), generation);
        let task = tokio::spawn(async move {
            worker
                .perform(request(&sid, sgen, Action::SendInput, "ls"))
                .await
        });
        let _ask = asks.recv().await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(broker.audit_snapshot()["active"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn output_capture_strips_split_escape_and_utf8() {
        let mut capture = OutputCapture::default();
        let bytes = "a\x1b[31m赤\x1b]0;title\x07b\x1b]8;;x\x1b\\c".as_bytes();
        let mut text = String::new();
        for byte in bytes {
            text.push_str(&capture.push(std::slice::from_ref(byte)));
        }
        assert_eq!(text, "a赤bc");
        let registry = Registry::default();
        let (input, _rx) = mpsc::channel(1);
        let (id, generation) = registry.register(
            None,
            Registration {
                label: "x".into(),
                kind: Kind::Ssh,
                shell: "unknown".into(),
                cwd: None,
                shareable: false,
                input,
                ssh: None,
            },
        );
        registry.update_output(&id, generation, &"あ".repeat(OUTPUT_LIMIT + 5), "画面");
        let read = registry.read_output(&id, generation, 3).unwrap();
        assert_eq!(read["output"], "あああ");
        assert!(
            registry.list(true).is_empty(),
            "共有しない端末は一覧に出さない"
        );
        registry.disconnect(&id, generation);
        assert!(registry.read_output(&id, generation, 3).is_err());
    }
}
