//! 接続先の書き出し（`hashi-export` v1。旧Python版の「接続情報の書き出し」と同じ形式）。
//! 書き出したファイルは、このアプリと旧版のどちらの「取り込み」でも読める。
//! パスワード等は平文で書かない。含めるときは利用者が決めたパスフレーズ（scrypt＋Fernet）で暗号化する。
use crate::app::AppState;
use hashi_storage::{
    config::{Bundle, KnownHosts, Profile},
    crypto,
    paths::Channel,
    vault::{self, Vault},
};
use serde::Serialize;
use std::{collections::BTreeMap, io::Write, path::Path};
use tauri::State;

/// 書き出しファイルを守るパスフレーズの最短の長さ（文字数）。
const MIN_PASSPHRASE: usize = 8;

#[derive(Serialize)]
pub struct ExportResult {
    profiles: usize,
    hosts: usize,
    secrets: usize,
    file_name: String,
}

/// `host:port`形式の踏み台（`user@host:port`・`[v6]:port`）から、ホスト鍵の記録名を作る。
fn hop_key(hop: &str) -> Option<String> {
    let host = hop.trim().rsplit_once('@').map_or(hop.trim(), |(_, h)| h);
    let (host, port) = if let Some(rest) = host.strip_prefix('[') {
        let (address, suffix) = rest.split_once(']')?;
        let port = match suffix.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None => 22u16,
        };
        (address.to_owned(), port)
    } else if host.matches(':').count() == 1 {
        let (h, p) = host.rsplit_once(':')?;
        (h.to_owned(), p.parse().ok()?)
    } else {
        (host.to_owned(), 22)
    };
    (!host.is_empty()).then(|| format!("{host}:{port}"))
}

/// 選んだ接続先（と踏み台）の承認済みホスト鍵だけを持ち出す。ほかのサーバーの記録は含めない。
fn hosts_for(profiles: &[Profile], known: &KnownHosts) -> KnownHosts {
    let mut names = vec![];
    for profile in profiles {
        names.push(format!("{}:{}", profile.host, profile.port));
        if !profile.proxy_jump.trim().eq_ignore_ascii_case("none") {
            names.extend(profile.proxy_jump.split(',').filter_map(hop_key));
        }
    }
    known
        .iter()
        .filter(|(name, _)| names.contains(name))
        .map(|(name, key)| (name.clone(), key.clone()))
        .collect()
}

/// 書き出す内容を組み立てる（ファイル選択とは別にして試験できるようにする）。
fn build(
    profiles: Vec<Profile>,
    known: &KnownHosts,
    secrets: Option<(&str, BTreeMap<String, BTreeMap<String, String>>)>,
) -> Result<(Bundle, usize), String> {
    let mut bundle = Bundle {
        format: "hashi-export".into(),
        version: 1,
        known_hosts: hosts_for(&profiles, known),
        profiles,
        secrets: None,
    };
    let mut count = 0;
    if let Some((passphrase, secrets)) = secrets {
        count = secrets.values().map(BTreeMap::len).sum();
        if count > 0 {
            let plain = serde_json::to_vec(&secrets).map_err(|e| e.to_string())?;
            bundle.secrets =
                Some(crypto::encrypt(&plain, passphrase).map_err(|error| error.to_string())?);
        }
    }
    Ok((bundle, count))
}

fn default_name(profiles: &[Profile]) -> String {
    let base = match profiles {
        [one] if !one.name.trim().is_empty() => one.name.trim().to_owned(),
        [one] => one.host.clone(),
        _ => "connections".into(),
    };
    // Windowsでファイル名に使えない文字を置き換える。
    let safe: String = base
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(60)
        .collect();
    format!("hashi-export-{}.json", safe.trim_matches(['.', ' ']))
}

/// 同じフォルダの一時ファイルへ書いてから置き換える（書きかけのファイルを残さない）。
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// 選んだ接続先を書き出す。保存先はOSのファイル選択で決め、パスは画面へ返さない（名前だけ）。
/// `passphrase`があるときだけ、保存済みのパスワード・パスフレーズを暗号化して含める。
#[tauri::command]
pub async fn export_bundle(
    state: State<'_, AppState>,
    indices: Vec<usize>,
    passphrase: Option<String>,
) -> Result<Option<ExportResult>, String> {
    let (profiles, known) = {
        let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
        let mut chosen: Vec<Profile> = vec![];
        for index in indices {
            let profile = snapshot.profiles.get(index).ok_or("接続設定がありません")?;
            if !chosen.iter().any(|p| p.id() == profile.id()) {
                chosen.push(profile.clone());
            }
        }
        (chosen, snapshot.known_hosts.clone())
    };
    if profiles.is_empty() {
        return Err("書き出す接続先を選んでください".into());
    }
    let passphrase = passphrase.filter(|p| !p.is_empty());
    if passphrase
        .as_ref()
        .is_some_and(|p| p.chars().count() < MIN_PASSPHRASE)
    {
        return Err(format!(
            "パスフレーズは{MIN_PASSPHRASE}文字以上にしてください"
        ));
    }
    let name = default_name(&profiles);
    let picked = tokio::task::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("Hashi", &["json"])
            .save_file()
    })
    .await
    .map_err(|_| "ファイル選択に失敗しました".to_owned())?;
    let Some(path) = picked else { return Ok(None) };
    tokio::task::spawn_blocking(move || -> Result<ExportResult, String> {
        let secrets = match &passphrase {
            None => None,
            Some(passphrase) => {
                let vault = Vault::open(Channel::Preview)
                    .map_err(|error| format!("保存済みの秘密を読めません: {error}"))?;
                let mut secrets: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
                for profile in profiles.iter().filter(|p| p.save_secrets) {
                    let id = profile.id();
                    for kind in vault::KINDS {
                        let value = vault
                            .get(&vault::key(&id, kind))
                            .map_err(|error| format!("保存済みの秘密を読めません: {error}"))?;
                        if let Some(value) = value.filter(|v| !v.is_empty()) {
                            secrets
                                .entry(id.clone())
                                .or_default()
                                .insert(kind.to_owned(), value);
                        }
                    }
                }
                Some((passphrase.as_str(), secrets))
            }
        };
        let count = profiles.len();
        let (bundle, secret_count) = build(profiles, &known, secrets)?;
        let bytes = serde_json::to_vec_pretty(&bundle).map_err(|error| error.to_string())?;
        write_atomic(&path, &bytes).map_err(|error| format!("書き出せませんでした: {error}"))?;
        Ok(ExportResult {
            profiles: count,
            hosts: bundle.known_hosts.len(),
            secrets: secret_count,
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        })
    })
    .await
    .map_err(|_| "書き出しに失敗しました".to_owned())?
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashi_storage::{config::HostKey, legacy_import};

    fn profile(user: &str, host: &str, port: u16, jump: &str) -> Profile {
        Profile {
            name: String::new(),
            host: host.into(),
            port,
            username: user.into(),
            auth_method: "password".into(),
            proxy_jump: jump.into(),
            ..Profile::default()
        }
    }

    fn key(fp: &str) -> HostKey {
        HostKey {
            key_type: "ssh-ed25519".into(),
            fingerprint: fp.into(),
        }
    }

    #[test]
    fn hop_names_follow_the_known_hosts_format() {
        assert_eq!(hop_key("bastion").as_deref(), Some("bastion:22"));
        assert_eq!(
            hop_key(" me@b.example:2200").as_deref(),
            Some("b.example:2200")
        );
        assert_eq!(hop_key("[::1]:2222").as_deref(), Some("::1:2222"));
        assert_eq!(hop_key("[::1]").as_deref(), Some("::1:22"));
        assert_eq!(hop_key("x:notaport"), None);
    }

    #[test]
    fn only_the_chosen_servers_and_their_jumps_are_exported() {
        let mut known = KnownHosts::new();
        known.insert("a.example:22".into(), key("A"));
        known.insert("jump.example:2200".into(), key("J"));
        known.insert("other.example:22".into(), key("O"));
        let chosen = vec![profile("me", "a.example", 22, "u@jump.example:2200")];
        let (bundle, count) = build(chosen, &known, None).unwrap();
        assert_eq!(count, 0);
        assert!(bundle.secrets.is_none());
        let names: Vec<_> = bundle.known_hosts.keys().cloned().collect();
        assert_eq!(names, vec!["a.example:22", "jump.example:2200"]);
    }

    #[test]
    fn exported_file_round_trips_through_the_importer() {
        let chosen = vec![profile("me", "a.example", 2222, "")];
        let id = chosen[0].id();
        let mut secrets = BTreeMap::new();
        secrets.insert(
            id.clone(),
            BTreeMap::from([("password".to_owned(), "秘密のパスワード".to_owned())]),
        );
        let (bundle, count) = build(
            chosen,
            &KnownHosts::new(),
            Some(("長いパスフレーズ", secrets)),
        )
        .unwrap();
        assert_eq!(count, 1);
        let bytes = serde_json::to_vec_pretty(&bundle).unwrap();
        // 平文の秘密はファイルに現れない。
        assert!(!String::from_utf8_lossy(&bytes).contains("秘密のパスワード"));
        let (imported, encrypted) = legacy_import::read_bundle(&bytes).unwrap();
        assert!(encrypted);
        assert_eq!(imported.profiles[0].id(), id);
        let decrypted = legacy_import::decrypt_bundle_secrets(&bytes, "長いパスフレーズ").unwrap();
        assert_eq!(decrypted[&id]["password"], "秘密のパスワード");
        assert!(legacy_import::decrypt_bundle_secrets(&bytes, "違うパスフレーズ").is_err());
    }

    #[test]
    fn file_names_are_safe_on_windows() {
        let mut p = profile("me", "a.example", 22, "");
        p.name = "本番: web/1?".into();
        assert_eq!(default_name(&[p]), "hashi-export-本番_ web_1_.json");
        let two = vec![profile("a", "h", 22, ""), profile("b", "h", 22, "")];
        assert_eq!(default_name(&two), "hashi-export-connections.json");
    }

    #[test]
    fn writes_replace_the_target_in_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
