//! 旧Python版からの取り込み。取り込み元は「旧版の保存先を直接読む」か「旧版が書き出したファイル」。
//! 旧版のファイル・資格情報は読むだけで書かない。秘密（パスワード等）はこのファイルの外へ出さず、
//! 画面へ返すのは名前・件数・有無だけ。保存は利用者が選んだ項目だけを、確認後に行う。
use crate::app::AppState;
use hashi_core::{permissions::shared_legacy_lease, ssh::SavedSecrets};
use hashi_storage::{
    config::{merge_hosts, KnownHosts, Profile},
    legacy_import::{self, Imported},
    paths::{config_root, Channel},
    snippets::{self, Snippet},
    vault::{self, Vault},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::State;

const VALID_FOR: Duration = Duration::from_secs(15 * 60);
const MAX_SNIPPETS: usize = 256;
const MAX_BODY: usize = 16 * 1024;
const MAX_BUNDLE: u64 = 8 * 1024 * 1024;

struct Prepared {
    token: String,
    source: &'static str,
    imported: Imported,
    /// 書き出しファイルの元バイト列。パスフレーズ入力後に秘密を復号するために保持する。
    bundle: Option<Vec<u8>>,
    needs_passphrase: bool,
    unlocked: bool,
    created: Instant,
}

/// 準備済みの取り込みは1件だけ。新しく準備すると前のものは捨てる（秘密を長く持たない）。
#[derive(Default)]
pub struct Imports {
    slot: Mutex<Option<Prepared>>,
}

#[derive(Serialize)]
pub struct PreviewProfile {
    index: usize,
    label: String,
    id: String,
    auth_method: String,
    proxy_jump: bool,
    exists: bool,
    password: bool,
    passphrase: bool,
}

#[derive(Serialize)]
pub struct Preview {
    token: String,
    source: &'static str,
    needs_passphrase: bool,
    unlocked: bool,
    profiles: Vec<PreviewProfile>,
    hosts_new: usize,
    hosts_same: usize,
    hosts_conflict: usize,
    snippets_new: Option<usize>,
    notes: Vec<String>,
}

fn preview(prepared: &Prepared, state: &AppState) -> Result<Preview, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    let imported = &prepared.imported;
    let profiles = imported
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let id = profile.id();
            let secrets = imported.secrets.get(&id);
            PreviewProfile {
                index,
                label: if profile.name.is_empty() {
                    profile.host.clone()
                } else {
                    profile.name.clone()
                },
                auth_method: profile.auth_method.clone(),
                proxy_jump: !profile.proxy_jump.is_empty(),
                exists: snapshot.profiles.iter().any(|p| p.id() == id),
                password: secrets.is_some_and(|s| s.contains_key("password")),
                passphrase: secrets.is_some_and(|s| s.contains_key("passphrase")),
                id,
            }
        })
        .collect();
    let (mut hosts_new, mut hosts_same, mut hosts_conflict) = (0, 0, 0);
    for (host, key) in &imported.known_hosts {
        match snapshot.known_hosts.get(host) {
            None => hosts_new += 1,
            Some(old) if old == key => hosts_same += 1,
            Some(_) => hosts_conflict += 1,
        }
    }
    let mut notes = imported.notes.clone();
    let snippets_new = match &imported.snippets {
        None => None,
        Some(value) => match (snippets::parse(value), snippets::parse(&snapshot.snippets)) {
            (Ok(incoming), Ok(current)) => Some(new_snippets(&current, &incoming).len()),
            _ => {
                notes.push("スニペットを解析できないため、スニペットは取り込みません".into());
                None
            }
        },
    };
    Ok(Preview {
        token: prepared.token.clone(),
        source: prepared.source,
        needs_passphrase: prepared.needs_passphrase,
        unlocked: prepared.unlocked,
        profiles,
        hosts_new,
        hosts_same,
        hosts_conflict,
        snippets_new,
        notes,
    })
}

fn new_snippets(current: &[Snippet], incoming: &[Snippet]) -> Vec<Snippet> {
    incoming
        .iter()
        .filter(|s| {
            !s.name.trim().is_empty()
                && s.name.len() <= 200
                && s.body.len() <= MAX_BODY
                && !current.iter().any(|c| c.name == s.name && c.body == s.body)
        })
        .cloned()
        .collect()
}

fn lock<'a>(imports: &'a Imports) -> Result<std::sync::MutexGuard<'a, Option<Prepared>>, String> {
    imports
        .slot
        .lock()
        .map_err(|_| "取り込みの状態が不正です".into())
}

fn token() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn place(
    state: &AppState,
    source: &'static str,
    imported: Imported,
    bundle: Option<Vec<u8>>,
    needs_passphrase: bool,
) -> Result<Preview, String> {
    let prepared = Prepared {
        token: token(),
        source,
        imported,
        bundle,
        needs_passphrase,
        unlocked: false,
        created: Instant::now(),
    };
    let result = preview(&prepared, state)?;
    *lock(&state.imports)? = Some(prepared);
    Ok(result)
}

/// 旧版の保存先を読む。旧版が動いていないという利用者の確認と、排他ガードの取得が前提。
#[tauri::command]
pub async fn import_legacy_inspect(
    state: State<'_, AppState>,
    legacy_stopped: bool,
) -> Result<Preview, String> {
    state.writable()?;
    if !legacy_stopped {
        return Err("旧Python版をすべて終了したことの確認が必要です".into());
    }
    let imported = tokio::task::spawn_blocking(|| -> Result<Imported, String> {
        let lease = shared_legacy_lease().map_err(|error| {
            format!("旧版が起動していないことを確認できませんでした。旧版をすべて終了してからやり直してください（{error}）")
        })?;
        let root = config_root(Channel::Python).map_err(|error| error.to_string())?;
        legacy_import::read_legacy(&root, &lease).map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "旧データの読み取りに失敗しました".to_owned())??;
    place(&state, "legacy", imported, None, false)
}

/// 旧版が書き出した`hashi-export`ファイルをOSのファイル選択で選ぶ。パスは画面へ渡さない。
#[tauri::command]
pub async fn import_bundle_pick(state: State<'_, AppState>) -> Result<Option<Preview>, String> {
    state.writable()?;
    let picked = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_file())
        .await
        .map_err(|_| "ファイル選択に失敗しました".to_owned())?;
    let Some(path) = picked else { return Ok(None) };
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        use std::io::Read;
        let file = std::fs::File::open(&path).map_err(|e| format!("読み込めませんでした: {e}"))?;
        let mut bytes = vec![];
        file.take(MAX_BUNDLE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("読み込めませんでした: {e}"))?;
        if bytes.len() as u64 > MAX_BUNDLE {
            return Err("ファイルが大きすぎます".into());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| "読み込みに失敗しました".to_owned())??;
    let (imported, encrypted) =
        legacy_import::read_bundle(&bytes).map_err(|error| error.to_string())?;
    place(&state, "bundle", imported, Some(bytes), encrypted).map(Some)
}

/// 書き出しファイルの秘密を、書き出したときのパスフレーズで読めるようにする。違えば何も変えない。
#[tauri::command]
pub async fn import_bundle_unlock(
    state: State<'_, AppState>,
    token: String,
    passphrase: String,
) -> Result<Preview, String> {
    let bytes = {
        let slot = lock(&state.imports)?;
        let prepared = slot
            .as_ref()
            .filter(|p| p.token == token)
            .ok_or("取り込みの準備が見つかりません。やり直してください")?;
        prepared
            .bundle
            .clone()
            .ok_or("書き出しファイルではありません")?
    };
    let secrets = tokio::task::spawn_blocking(move || {
        legacy_import::decrypt_bundle_secrets(&bytes, &passphrase)
    })
    .await
    .map_err(|_| "復号に失敗しました".to_owned())?
    .map_err(|error| error.to_string())?;
    let mut slot = lock(&state.imports)?;
    let prepared = slot
        .as_mut()
        .filter(|p| p.token == token)
        .ok_or("取り込みの準備が見つかりません。やり直してください")?;
    prepared.imported.secrets = secrets;
    prepared.unlocked = true;
    preview(prepared, &state)
}

#[tauri::command]
pub fn import_cancel(state: State<'_, AppState>) -> Result<(), String> {
    *lock(&state.imports)? = None;
    Ok(())
}

#[derive(Deserialize)]
pub struct ApplyOptions {
    token: String,
    selected: Vec<usize>,
    overwrite: bool,
    known_hosts: bool,
    snippets: bool,
    secrets: bool,
}

#[derive(Serialize, Default)]
pub struct ApplyResult {
    added: usize,
    updated: usize,
    skipped: usize,
    hosts_added: usize,
    host_conflicts: Vec<String>,
    snippets_added: usize,
    secrets_saved: usize,
    secrets_failed: usize,
    notes: Vec<String>,
}

/// 選ばれた項目を保存する。ホスト鍵は無いものだけ追加（既存・食い違いは上書きしない）。
#[tauri::command]
pub fn import_apply(
    state: State<'_, AppState>,
    options: ApplyOptions,
) -> Result<(ApplyResult, Vec<Profile>), String> {
    state.writable()?;
    let slot = lock(&state.imports)?;
    let prepared = slot
        .as_ref()
        .filter(|p| p.token == options.token)
        .ok_or("取り込みの準備が見つかりません。やり直してください")?;
    if prepared.created.elapsed() > VALID_FOR {
        return Err("取り込みの準備が古くなりました。やり直してください".into());
    }
    let imported = &prepared.imported;
    let mut chosen: Vec<&Profile> = vec![];
    for &index in &options.selected {
        let profile = imported.profiles.get(index).ok_or("選択が不正です")?;
        if !chosen.iter().any(|p| p.id() == profile.id()) {
            chosen.push(profile);
        }
    }
    let mut result = ApplyResult::default();
    let incoming_snippets = match (&imported.snippets, options.snippets) {
        (Some(value), true) => match snippets::parse(value) {
            Ok(list) => Some(list),
            Err(_) => {
                result
                    .notes
                    .push("スニペットを解析できないため取り込みませんでした".into());
                None
            }
        },
        _ => None,
    };
    let mut to_store: Vec<(String, bool)> = vec![];
    let mut snippet_note = None;
    let snapshot = state
        .store
        .transact(|snapshot| {
            for profile in &chosen {
                if profile.host.trim().is_empty() || profile.username.trim().is_empty() {
                    result.skipped += 1;
                    continue;
                }
                let id = profile.id();
                match snapshot.profiles.iter().position(|p| p.id() == id) {
                    None => {
                        snapshot.profiles.push((*profile).clone());
                        result.added += 1;
                    }
                    Some(index) if options.overwrite => {
                        snapshot.profiles[index] = (*profile).clone();
                        result.updated += 1;
                    }
                    Some(_) => {
                        result.skipped += 1;
                        continue;
                    }
                }
                to_store.push((id, profile.save_secrets));
            }
            if options.known_hosts {
                let before = snapshot.known_hosts.len();
                let incoming: KnownHosts = imported.known_hosts.clone();
                result.host_conflicts = merge_hosts(&mut snapshot.known_hosts, &incoming);
                result.hosts_added = snapshot.known_hosts.len() - before;
            }
            if let Some(incoming) = &incoming_snippets {
                match snippets::parse(&snapshot.snippets) {
                    Ok(mut current) => {
                        let fresh = new_snippets(&current, incoming);
                        let room = MAX_SNIPPETS.saturating_sub(current.len());
                        if fresh.len() > room {
                            snippet_note = Some(format!(
                                "スニペットは{MAX_SNIPPETS}件までのため、{}件は取り込めませんでした",
                                fresh.len() - room
                            ));
                        }
                        let take: Vec<_> = fresh.into_iter().take(room).collect();
                        result.snippets_added = take.len();
                        current.extend(take);
                        snapshot.snippets = snippets::to_value(&current);
                    }
                    Err(_) => {
                        snippet_note =
                            Some("保存済みのスニペットを解析できないため、追加しませんでした".into())
                    }
                }
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    result.notes.extend(snippet_note);
    let profiles = snapshot.profiles.clone();
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    // 接続設定の保存が済んでから秘密を保存する。失敗しても接続設定は残し、件数で正直に知らせる。
    if options.secrets {
        match Vault::open(Channel::Preview) {
            Ok(vault) => {
                for (id, allowed) in &to_store {
                    let Some(kinds) = imported.secrets.get(id) else {
                        continue;
                    };
                    if !allowed {
                        continue;
                    }
                    for (kind, value) in kinds {
                        match vault.set(&vault::key(id, kind), Some(value)) {
                            Ok(()) => result.secrets_saved += 1,
                            Err(_) => result.secrets_failed += 1,
                        }
                    }
                }
                if result.secrets_saved > 0 {
                    result
                        .notes
                        .push(format!("秘密の保存先: {}", vault.backend_name()));
                }
            }
            Err(error) => {
                result.secrets_failed = to_store
                    .iter()
                    .map(|(id, _)| imported.secrets.get(id).map_or(0, |s| s.len()))
                    .sum();
                result
                    .notes
                    .push(format!("秘密の保存先を開けませんでした: {error}"));
            }
        }
    }
    drop(slot);
    *lock(&state.imports)? = None;
    Ok((result, profiles))
}

#[derive(Serialize)]
pub struct SecretsStatus {
    saved: Vec<String>,
    backend: &'static str,
}

fn profile_id(state: &AppState, index: usize) -> Result<String, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    snapshot
        .profiles
        .get(index)
        .map(Profile::id)
        .ok_or_else(|| "接続設定がありません".into())
}

/// その接続先に保存されている秘密の種類（値は返さない）。
#[tauri::command]
pub fn secrets_status(state: State<'_, AppState>, index: usize) -> Result<SecretsStatus, String> {
    let id = profile_id(&state, index)?;
    let vault = Vault::open(Channel::Preview).map_err(|error| error.to_string())?;
    let saved = vault
        .kinds_present(&id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(str::to_owned)
        .collect();
    Ok(SecretsStatus {
        saved,
        backend: vault.backend_name(),
    })
}

#[tauri::command]
pub fn secrets_forget(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    let id = profile_id(&state, index)?;
    Vault::open(Channel::Preview)
        .and_then(|vault| vault.forget(&id))
        .map_err(|error| error.to_string())
}

/// 接続先の削除時に保存済みの秘密も消す。
pub fn forget_id(profile_id: &str) -> std::io::Result<()> {
    Vault::open(Channel::Preview)?.forget(profile_id)
}

/// 接続時に使う保存済みの秘密。最終の接続先だけに使われる（踏み台は`hashi-core`側で除外）。
/// 接続設定の`save_secrets`が偽なら何も返さない。
pub struct ProfileSecrets {
    vault: Vault,
    profile_id: String,
}

impl ProfileSecrets {
    pub fn for_profile(profile: &Profile) -> Option<std::sync::Arc<dyn SavedSecrets>> {
        if !profile.save_secrets {
            return None;
        }
        let vault = Vault::open(Channel::Preview).ok()?;
        Some(std::sync::Arc::new(Self {
            vault,
            profile_id: profile.id(),
        }))
    }
}

impl SavedSecrets for ProfileSecrets {
    fn get(&self, _host: &str, _port: u16, _username: &str, kind: &str) -> Option<String> {
        // ~/.ssh/configで接続先が書き換わっても、選んだ接続設定の秘密を引く。
        vault::KINDS
            .contains(&kind)
            .then(|| {
                self.vault
                    .get(&vault::key(&self.profile_id, kind))
                    .ok()
                    .flatten()
            })
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str, body: &str) -> Snippet {
        Snippet {
            name: name.into(),
            body: body.into(),
            send_enter: true,
        }
    }

    #[test]
    fn only_new_valid_snippets_are_added() {
        let current = vec![snippet("稼働", "uptime")];
        let incoming = vec![
            snippet("稼働", "uptime"),
            snippet("稼働", "uptime -p"),
            snippet("  ", "x"),
            snippet(&"長".repeat(100), "x"),
            snippet("巨大", &"a".repeat(MAX_BODY + 1)),
            snippet("新規", "ls"),
        ];
        let fresh = new_snippets(&current, &incoming);
        let names: Vec<_> = fresh
            .iter()
            .map(|s| (s.name.as_str(), s.body.as_str()))
            .collect();
        assert_eq!(names, vec![("稼働", "uptime -p"), ("新規", "ls")]);
    }

    #[test]
    fn profile_secrets_are_off_when_the_profile_does_not_save_them() {
        let profile = Profile {
            save_secrets: false,
            ..Profile::default()
        };
        assert!(ProfileSecrets::for_profile(&profile).is_none());
    }
}
