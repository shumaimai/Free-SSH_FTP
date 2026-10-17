//! MCPクライアント(Claude Desktop / Claude Code)へのHashiの登録を手伝う。
//! 触るのは`mcpServers`の`hashi-rust-preview`1項目だけ。ほかの設定は一切変えず、書く前に必ず
//! バックアップを残す。読めない/形式が違うファイルは上書きしない。
use hashi_mcp::SERVER_NAME;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const CONFIG_NAME: &str = "claude_desktop_config.json";
const BACKUP_KEEP: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// 設定ファイルにこのHashiのヘルパーが登録済み。
    Registered,
    /// 登録がない(ファイルが無い場合も含む)。
    NotRegistered,
    /// 登録はあるが、別の場所のヘルパーを指している。
    OtherPath,
    /// 設定ファイルがJSONとして読めない。上書きしない。
    Unreadable,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::NotRegistered => "notRegistered",
            Self::OtherPath => "otherPath",
            Self::Unreadable => "unreadable",
        }
    }
}

fn parse(raw: &[u8]) -> Result<Map<String, Value>, String> {
    // BOM付きUTF-8も読む。
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    if raw.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err("設定ファイルがJSONのオブジェクトとして読めません。手で確認してください".into()),
    }
}

/// 登録状況。`command`は現在のヘルパーのパス。
pub fn state_of(existing: Option<&[u8]>, command: &str) -> State {
    let Some(raw) = existing else {
        return State::NotRegistered;
    };
    let Ok(map) = parse(raw) else {
        return State::Unreadable;
    };
    match map
        .get("mcpServers")
        .and_then(|servers| servers.get(SERVER_NAME))
    {
        None => State::NotRegistered,
        Some(entry) => {
            let same = entry["command"].as_str() == Some(command)
                && entry["args"].as_array().is_none_or(|args| args.is_empty());
            if same {
                State::Registered
            } else {
                State::OtherPath
            }
        }
    }
}

/// `mcpServers.hashi-rust-preview`だけを入れた(または置き換えた)設定ファイルの内容。
pub fn merged(existing: Option<&[u8]>, command: &str) -> Result<Vec<u8>, String> {
    let mut map = match existing {
        Some(raw) => parse(raw)?,
        None => Map::new(),
    };
    let servers = map
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err("設定ファイルのmcpServersが想定と違う形です。手で確認してください".into());
    };
    servers.insert(
        SERVER_NAME.to_owned(),
        json!({"command": command, "args": []}),
    );
    let mut out = serde_json::to_vec_pretty(&Value::Object(map)).map_err(|e| e.to_string())?;
    out.push(b'\n');
    Ok(out)
}

/// Claude Desktopの設定ファイルの場所。既存のファイルを優先し、無ければ親フォルダがある場所。
pub fn config_path_in(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|path| path.is_file())
        .or_else(|| {
            candidates
                .iter()
                .find(|path| path.parent().is_some_and(Path::is_dir))
        })
        .cloned()
}

fn candidates() -> Vec<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let mut out = vec![];
    if cfg!(windows) {
        if let Some(appdata) = var("APPDATA") {
            out.push(appdata.join("Claude").join(CONFIG_NAME));
        }
        // Microsoft Store版は設定がパッケージ内の仮想領域に置かれる。
        if let Some(local) = var("LOCALAPPDATA") {
            if let Ok(entries) = std::fs::read_dir(local.join("Packages")) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with("Claude_") {
                        out.push(
                            entry
                                .path()
                                .join("LocalCache/Roaming/Claude")
                                .join(CONFIG_NAME),
                        );
                    }
                }
            }
        }
    } else if let Some(home) = var("HOME") {
        out.push(
            home.join("Library/Application Support/Claude")
                .join(CONFIG_NAME),
        );
        out.push(home.join(".config/Claude").join(CONFIG_NAME));
    }
    out
}

fn backup(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let target = path.with_file_name(format!("{name}.hashi-backup-{stamp}"));
    std::fs::copy(path, &target).map_err(|e| format!("バックアップを作れません: {e}"))?;
    // 古いバックアップは新しい3つだけ残す。
    let prefix = format!("{name}.hashi-backup-");
    if let Some(parent) = path.parent() {
        let mut old: Vec<PathBuf> = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
            })
            .collect();
        old.sort();
        let drop = old.len().saturating_sub(BACKUP_KEEP);
        for path in old.into_iter().take(drop) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(Some(target))
}

/// 設定ファイルへ登録する。成功したらバックアップの場所(あれば)を返す。
pub fn register_into(path: &Path, command: &str) -> Result<Option<PathBuf>, String> {
    let existing = match std::fs::read(path) {
        Ok(raw) => Some(raw),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("設定ファイルを読めません: {error}")),
    };
    // 先に内容を作る。読めない設定ファイルはここで止まり、何も書かない。
    let content = merged(existing.as_deref(), command)?;
    let saved = backup(path)?;
    let parent = path.parent().ok_or("設定ファイルの場所が不正です")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("設定ファイルを書けません: {e}"))?;
    std::io::Write::write_all(&mut temp, &content).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path)
        .map_err(|e| format!("設定ファイルを書けません: {}", e.error))?;
    Ok(saved)
}

/// Claude Code用の登録コマンド(利用者がターミナルへ貼って実行する)。
pub fn code_command(command: &str) -> String {
    format!("claude mcp add --scope user {SERVER_NAME} -- \"{command}\"")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterStatus {
    helper_found: bool,
    helper_path: String,
    desktop_config_path: Option<String>,
    /// registered / notRegistered / otherPath / unreadable / noConfig
    desktop_state: String,
    code_command: String,
    /// 手で登録するとき用のJSON。
    manual_config: String,
    /// 直前の登録で作ったバックアップ。
    backup: Option<String>,
}

fn status_with(path: Option<PathBuf>, backup: Option<PathBuf>) -> RegisterStatus {
    let helper = hashi_mcp::helper_path().ok();
    let command = helper
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let desktop_state = match &path {
        None => "noConfig",
        Some(path) => state_of(std::fs::read(path).ok().as_deref(), &command).name(),
    };
    RegisterStatus {
        helper_found: helper.is_some(),
        manual_config: serde_json::to_string_pretty(
            &json!({"mcpServers": {SERVER_NAME: {"command": command, "args": []}}}),
        )
        .unwrap_or_default(),
        code_command: code_command(&command),
        helper_path: command,
        desktop_config_path: path.map(|p| p.to_string_lossy().into_owned()),
        desktop_state: desktop_state.to_owned(),
        backup: backup.map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub fn mcp_register_status() -> RegisterStatus {
    status_with(config_path_in(&candidates()), None)
}

/// Claude Desktopの設定ファイルへHashiを登録する。Claude Desktopが動いていると
/// 終了時に上書きされて消えるため、画面で必ず終了を案内する。
#[tauri::command]
pub fn mcp_register_desktop() -> Result<RegisterStatus, String> {
    let command = hashi_mcp::helper_path()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    let path = config_path_in(&candidates()).ok_or(
        "Claude Desktopの設定フォルダが見つかりません。Claude Desktopを一度起動してから、もう一度お試しください",
    )?;
    let saved = register_into(&path, &command)?;
    Ok(status_with(Some(path), saved))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_adds_only_our_entry_and_keeps_everything_else() {
        let existing = br#"{
          "mcpServers": {"Blender-MCP": {"command": "blender.exe", "args": [], "env": {"A": "1"}}},
          "preferences": {"x": [1, 2, {"y": null}]},
          "coworkUserFilesPath": "C:\\Users\\a\\Claude"
        }"#;
        let out = merged(Some(existing), r"C:\Hashi\HashiMCP-RustPreview.exe").unwrap();
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            value["mcpServers"]["Blender-MCP"]["env"]["A"], "1",
            "ほかのMCPサーバーは変えない"
        );
        assert_eq!(
            value["mcpServers"][SERVER_NAME]["command"],
            r"C:\Hashi\HashiMCP-RustPreview.exe"
        );
        assert_eq!(value["preferences"]["x"][2]["y"], Value::Null);
        assert_eq!(value["coworkUserFilesPath"], r"C:\Users\a\Claude");
    }

    #[test]
    fn merge_creates_missing_sections_and_replaces_a_stale_entry() {
        let fresh = merged(None, "h.exe").unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fresh).unwrap()["mcpServers"][SERVER_NAME]["command"],
            "h.exe"
        );
        let stale = br#"{"mcpServers": {"hashi-rust-preview": {"command": "old.exe", "args": ["x.json"]}}}"#;
        let out = merged(Some(stale), "new.exe").unwrap();
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["mcpServers"][SERVER_NAME]["command"], "new.exe");
        assert_eq!(value["mcpServers"][SERVER_NAME]["args"], json!([]));
        assert!(merged(Some(b""), "h.exe").is_ok(), "空ファイルは新規扱い");
        assert!(
            merged(Some(b"\xEF\xBB\xBF{}"), "h.exe").is_ok(),
            "BOM付きも読む"
        );
    }

    #[test]
    fn unreadable_or_odd_files_are_never_overwritten() {
        for bad in [
            &b"{ broken"[..],
            b"[1,2]",
            b"\"text\"",
            br#"{"mcpServers": 3}"#,
        ] {
            assert!(merged(Some(bad), "h.exe").is_err(), "{:?}", bad);
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_NAME);
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(register_into(&path, "h.exe").is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{ broken",
            "壊れたファイルはそのまま"
        );
    }

    #[test]
    fn registering_backs_up_and_reports_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_NAME);
        std::fs::write(&path, br#"{"mcpServers": {"a": {"command": "a"}}}"#).unwrap();
        assert_eq!(
            state_of(std::fs::read(&path).ok().as_deref(), "h.exe"),
            State::NotRegistered
        );
        let backup = register_into(&path, "h.exe")
            .unwrap()
            .expect("既存ファイルは退避する");
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            br#"{"mcpServers": {"a": {"command": "a"}}}"#
        );
        assert_eq!(
            state_of(std::fs::read(&path).ok().as_deref(), "h.exe"),
            State::Registered
        );
        assert_eq!(
            state_of(std::fs::read(&path).ok().as_deref(), "moved.exe"),
            State::OtherPath
        );
        assert_eq!(state_of(Some(b"{ broken"), "h.exe"), State::Unreadable);
        // 新規作成(元のファイルが無い)ではバックアップを作らない。
        let fresh = directory.path().join("sub");
        std::fs::create_dir(&fresh).unwrap();
        assert_eq!(
            register_into(&fresh.join(CONFIG_NAME), "h.exe").unwrap(),
            None
        );
    }

    #[test]
    fn backups_keep_only_the_newest_few() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_NAME);
        std::fs::write(&path, b"{}").unwrap();
        for stamp in 1..=6 {
            std::fs::write(
                directory
                    .path()
                    .join(format!("{CONFIG_NAME}.hashi-backup-{stamp:010}")),
                b"{}",
            )
            .unwrap();
        }
        backup(&path).unwrap();
        let left = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("hashi-backup"))
            .count();
        assert_eq!(left, BACKUP_KEEP);
    }

    #[test]
    fn config_path_prefers_an_existing_file_then_an_existing_folder() {
        let directory = tempfile::tempdir().unwrap();
        let missing_folder = directory.path().join("none/x.json");
        let folder_only = directory.path().join("have").join(CONFIG_NAME);
        std::fs::create_dir(directory.path().join("have")).unwrap();
        assert_eq!(
            config_path_in(&[missing_folder.clone(), folder_only.clone()]),
            Some(folder_only.clone())
        );
        let existing = directory.path().join("have2");
        std::fs::create_dir(&existing).unwrap();
        let existing = existing.join(CONFIG_NAME);
        std::fs::write(&existing, b"{}").unwrap();
        assert_eq!(
            config_path_in(&[folder_only, existing.clone()]),
            Some(existing)
        );
        assert_eq!(config_path_in(&[missing_folder]), None);
    }

    #[test]
    fn code_command_quotes_the_path() {
        assert_eq!(
            code_command(r"C:\Program Files\Hashi\HashiMCP-RustPreview.exe"),
            r#"claude mcp add --scope user hashi-rust-preview -- "C:\Program Files\Hashi\HashiMCP-RustPreview.exe""#
        );
    }
}
