//! 外部操作の共有（AIと同じBroker）とローカルMCP。端末の閲覧・操作は必ずBrokerを通す。
//! 許可はメモリ内だけで15分。停止・対象変更で進行中の要求も取り消す。
//! MCPは127.0.0.1の認証つきIPCで、起動中のこのHashiへだけつながる（外部へ公開しない）。
use crate::app::AppState;
use hashi_core::broker::Mode;
use hashi_mcp::McpBridge;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::State;

const SHARE_LIFETIME: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct External {
    bridge: Mutex<Option<Arc<McpBridge>>>,
    starting: Mutex<bool>,
}

impl External {
    /// 終了時にMCPを閉じる（接続中のクライアントの承認待ちも取り消される）。
    pub fn close(&self) {
        if let Some(bridge) = self.bridge.lock().ok().and_then(|mut b| b.take()) {
            bridge.close();
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalStatus {
    /// 共有している端末の数。
    shared: usize,
    /// consult / confirm / auto
    mode: String,
    seconds: u64,
    /// 実行中・承認待ちの要求数。
    active: usize,
    mcp_running: bool,
    mcp_starting: bool,
    /// MCPクライアントへ登録する設定（JSON文字列）。起動していなければ空。
    mcp_config: String,
}

fn status(state: &AppState) -> ExternalStatus {
    let broker = &state.ai.broker;
    let permission = broker.permission_snapshot();
    let audit = broker.audit_snapshot();
    let bridge = state.external.bridge.lock().ok().and_then(|b| b.clone());
    let mcp_config = bridge
        .as_ref()
        .map(|bridge| match bridge.config() {
            Ok(config) => serde_json::to_string_pretty(&config).unwrap_or_default(),
            Err(error) => format!("{error:#}"),
        })
        .unwrap_or_default();
    ExternalStatus {
        shared: broker.list_sessions().len(),
        mode: permission["mode"].as_str().unwrap_or("consult").to_owned(),
        seconds: permission["seconds"].as_u64().unwrap_or(0),
        active: audit["active"].as_array().map_or(0, Vec::len),
        mcp_running: bridge.is_some(),
        mcp_starting: state.external.starting.lock().map(|s| *s).unwrap_or(false),
        mcp_config,
    }
}

#[tauri::command]
pub fn external_status(state: State<'_, AppState>) -> ExternalStatus {
    status(&state)
}

fn parse_mode(mode: &str) -> Result<Mode, String> {
    match mode {
        "consult" => Ok(Mode::Consult),
        "confirm" => Ok(Mode::Confirm),
        "auto" => Ok(Mode::Auto),
        _ => Err("共有の種類が不正です".into()),
    }
}

/// 選んだ端末（接続のid）を15分だけ共有する。選び直すと以前の許可と進行中の要求は取り消される。
#[tauri::command]
pub fn external_share(
    state: State<'_, AppState>,
    mode: String,
    targets: Vec<String>,
) -> Result<ExternalStatus, String> {
    let ids = crate::session::bindings_for(&state, &targets);
    state
        .ai
        .broker
        .configure(parse_mode(&mode)?, &ids, SHARE_LIFETIME)
        .map_err(|error| error.to_string())?;
    Ok(status(&state))
}

#[tauri::command]
pub fn external_stop(state: State<'_, AppState>) -> ExternalStatus {
    state.ai.broker.stop();
    status(&state)
}

#[tauri::command]
pub async fn mcp_start(state: State<'_, AppState>) -> Result<ExternalStatus, String> {
    if state.is_exiting() {
        return Err("終了処理中です".into());
    }
    {
        let mut starting = state
            .external
            .starting
            .lock()
            .map_err(|_| "状態が不正です")?;
        if *starting
            || state
                .external
                .bridge
                .lock()
                .map_err(|_| "状態が不正です")?
                .is_some()
        {
            return Err("MCPはすでに開始しています".into());
        }
        *starting = true;
    }
    let result = McpBridge::start(state.ai.broker.clone()).await;
    if let Ok(mut starting) = state.external.starting.lock() {
        *starting = false;
    }
    let bridge = result.map_err(|error| format!("MCPを開始できません: {error:#}"))?;
    if state.is_exiting() {
        bridge.close();
        return Err("終了処理中です".into());
    }
    *state.external.bridge.lock().map_err(|_| "状態が不正です")? = Some(bridge);
    Ok(status(&state))
}

#[tauri::command]
pub fn mcp_stop(state: State<'_, AppState>) -> ExternalStatus {
    state.external.close();
    status(&state)
}
