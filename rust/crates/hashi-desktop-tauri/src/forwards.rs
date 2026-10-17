//! ポート転送（-L / -R / -D）。待受の公開範囲は利用者が確認して指定する。
//! 停止は所有する転送だけに作用し、停止通信の失敗は未確認として返す。
use crate::{app::AppState, session};
use hashi_core::forward::{Destination, LocalForward, RemoteForward};
use serde::Serialize;
use tauri::State;

pub enum Active {
    Local(LocalForward),
    Remote(RemoteForward),
}

pub struct Entry {
    pub id: String,
    pub label: String,
    pub handle: Active,
}

impl Active {
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Local(forward) => {
                forward.stop().await;
                Ok(())
            }
            Self::Remote(forward) => forward.stop().await,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ForwardInfo {
    id: String,
    label: String,
}

fn infos(entries: &[Entry]) -> Vec<ForwardInfo> {
    entries
        .iter()
        .map(|entry| ForwardInfo {
            id: entry.id.clone(),
            label: entry.label.clone(),
        })
        .collect()
}

#[tauri::command]
pub async fn forward_list(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<ForwardInfo>, String> {
    let session = session::find(&state, &id)?;
    let forwards = session.forwards.lock().await;
    Ok(infos(&forwards))
}

#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    id: String,
    kind: String,
    bind: String,
    port: u32,
    host: String,
    target_port: u32,
) -> Result<Vec<ForwardInfo>, String> {
    let session = session::find(&state, &id)?;
    let ssh = session.ssh().ok_or("接続が済んでいません")?;
    if session.is_closing() {
        return Err("接続の終了処理中です".into());
    }
    if bind.trim().is_empty() || bind.len() > 253 || port > 65535 {
        return Err("待受アドレス/ポートが不正です".into());
    }
    let bind = bind.trim().to_owned();
    let address = if bind.contains(':') {
        format!("[{bind}]:{port}")
    } else {
        format!("{bind}:{port}")
    };
    let fixed = |host: &str, target_port: u32| -> Result<Destination, String> {
        if host.trim().is_empty() || host.len() > 253 || !(1..=65535).contains(&target_port) {
            return Err("転送先ホスト/ポートが不正です".into());
        }
        Ok(Destination::Fixed {
            host: host.trim().to_owned(),
            port: target_port as u16,
        })
    };
    let (label, handle) = match kind.as_str() {
        "L" => {
            let destination = fixed(&host, target_port)?;
            let forward = LocalForward::start(ssh, &address, destination)
                .await
                .map_err(|e| e.to_string())?;
            (
                format!(
                    "ローカル {} → {}:{}",
                    forward.address,
                    host.trim(),
                    target_port
                ),
                Active::Local(forward),
            )
        }
        "D" => {
            let forward = LocalForward::start(ssh, &address, Destination::Socks5)
                .await
                .map_err(|e| e.to_string())?;
            (
                format!("SOCKS5 {}", forward.address),
                Active::Local(forward),
            )
        }
        "R" => {
            fixed(&host, target_port)?;
            let forward = RemoteForward::start(
                ssh,
                bind.clone(),
                port as u16,
                host.trim().to_owned(),
                target_port as u16,
            )
            .await
            .map_err(|e| e.to_string())?;
            (
                format!(
                    "リモート {}:{} → {}:{}",
                    forward.bind,
                    forward.port,
                    host.trim(),
                    target_port
                ),
                Active::Remote(forward),
            )
        }
        _ => return Err("転送の種類が不正です".into()),
    };
    let mut forwards = session.forwards.lock().await;
    forwards.push(Entry {
        id: uuid::Uuid::new_v4().to_string(),
        label,
        handle,
    });
    Ok(infos(&forwards))
}

#[tauri::command]
pub async fn forward_stop(
    state: State<'_, AppState>,
    id: String,
    forward_id: String,
) -> Result<Vec<ForwardInfo>, String> {
    let session = session::find(&state, &id)?;
    let mut forwards = session.forwards.lock().await;
    let index = forwards
        .iter()
        .position(|entry| entry.id == forward_id)
        .ok_or("転送が見つかりません")?;
    // 所有する転送だけを止める。停止通信の失敗は、止まったとは扱わず未確認として返す。
    let mut entry = forwards.remove(index);
    match entry.handle.stop().await {
        Ok(()) => Ok(infos(&forwards)),
        Err(error) => Err(format!("停止は未確認です: {error}")),
    }
}
