//! 旧Python版の接続情報の取り込み元。読み取り専用で、旧版のファイル・資格情報は書き換えない。
//! 取り込み元は2つ: 旧版の保存先（OSの資格情報マネージャ/暗号化ファイル込み）と、旧版が書き出した`hashi-export`ファイル。
use crate::{
    config::{read_python_snapshot, Bundle, KnownHosts, Profile},
    crypto,
    migration::MigrationLease,
    secrets::{read_legacy_file, Namespace},
    vault::{key, KINDS},
};
use serde_json::Value;
use std::{collections::BTreeMap, io, path::Path};

/// プロファイルid（`user@host:port`）→ 種類 → 秘密。
pub type Secrets = BTreeMap<String, BTreeMap<String, String>>;

pub struct Imported {
    pub profiles: Vec<Profile>,
    pub known_hosts: KnownHosts,
    /// 旧版のsnippets.json（解析は取り込み側で行う）。書き出しファイルには含まれない。
    pub snippets: Option<Value>,
    pub secrets: Secrets,
    /// 利用者へ見せる注意書き（読めなかった保管先など）。秘密は含めない。
    pub notes: Vec<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// 旧版の保存先を読む。呼出し側が`MigrationLease`と旧版停止の確認を保持していること。
pub fn read_legacy(root: &Path, lease: &MigrationLease) -> io::Result<Imported> {
    #[cfg(windows)]
    let os_vault =
        |name: &str| crate::winvault::get(crate::paths::Channel::Python.credential_service(), name);
    #[cfg(not(windows))]
    let os_vault = |_: &str| Ok(None);
    read_legacy_with(root, lease, &os_vault, cfg!(windows))
}

/// OSの資格情報保管の読み取りを差し替えられる形。`os_vault_readable`は注意書きの出し分け用。
pub fn read_legacy_with(
    root: &Path,
    lease: &MigrationLease,
    os_vault: &dyn Fn(&str) -> io::Result<Option<String>>,
    os_vault_readable: bool,
) -> io::Result<Imported> {
    if !root.is_dir() {
        return Err(invalid("旧版のデータが見つかりません"));
    }
    let snapshot = read_python_snapshot(root, lease)?;
    let mut notes = vec![];
    let file = match read_legacy_file(root, Namespace::Ssh, lease) {
        Ok(map) => map,
        Err(_) => {
            notes.push(
                "旧版の暗号化された認証情報ファイル（creds.dat）を復号できませんでした。保存済みの秘密は取り込めません"
                    .into(),
            );
            BTreeMap::new()
        }
    };
    let mut secrets = Secrets::new();
    let mut vault_failures = 0usize;
    for profile in &snapshot.profiles {
        let id = profile.id();
        for kind in KINDS {
            let name = key(&id, kind);
            let value = match os_vault(&name) {
                Ok(Some(value)) => Some(value),
                Ok(None) => file.get(&name).cloned(),
                Err(_) => {
                    vault_failures += 1;
                    file.get(&name).cloned()
                }
            };
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                secrets
                    .entry(id.clone())
                    .or_default()
                    .insert(kind.into(), value);
            }
        }
    }
    if vault_failures > 0 {
        notes.push(format!(
            "OSの資格情報マネージャから{vault_failures}件を読めませんでした"
        ));
    }
    if !os_vault_readable {
        notes.push(
            "この環境ではOSの資格情報保管（Secret Service・キーチェーン）は読めません。暗号化ファイルに保存されていた秘密だけ取り込めます"
                .into(),
        );
    }
    notes.push("sudoパスワードはこのアプリが使わないため取り込みません".into());
    Ok(Imported {
        profiles: snapshot.profiles,
        known_hosts: snapshot.known_hosts,
        snippets: Some(snapshot.snippets),
        secrets,
        notes,
    })
}

/// 書き出しファイルの中身（秘密は未復号）。
pub fn read_bundle(bytes: &[u8]) -> io::Result<(Imported, bool)> {
    let bundle = Bundle::parse(bytes)?;
    let encrypted = bundle.secrets.is_some();
    Ok((
        Imported {
            profiles: bundle.profiles,
            known_hosts: bundle.known_hosts,
            snippets: None,
            secrets: Secrets::new(),
            notes: vec![],
        },
        encrypted,
    ))
}

/// 書き出しファイルの秘密を、書き出し時に決めたパスフレーズで復号する。
pub fn decrypt_bundle_secrets(bytes: &[u8], passphrase: &str) -> io::Result<Secrets> {
    let bundle = Bundle::parse(bytes)?;
    let Some(envelope) = bundle.secrets else {
        return Ok(Secrets::new());
    };
    let plain = crypto::decrypt(&envelope, passphrase)
        .map_err(|_| invalid("パスフレーズが違うか、秘密情報が壊れています"))?;
    let parsed: BTreeMap<String, BTreeMap<String, Value>> =
        serde_json::from_slice(&plain).map_err(|_| invalid("秘密情報の形式が不正です"))?;
    let mut secrets = Secrets::new();
    for (id, kinds) in parsed {
        for (kind, value) in kinds {
            if let (true, Value::String(value)) = (KINDS.contains(&kind.as_str()), value) {
                if !value.is_empty() {
                    secrets.entry(id.clone()).or_default().insert(kind, value);
                }
            }
        }
    }
    Ok(secrets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/python-v1.json")).unwrap()
    }

    #[test]
    fn bundle_secrets_decrypt_with_the_export_passphrase_only() {
        let f = fixture();
        let mut bundle = f["bundle"].clone();
        bundle["secrets"] = f["encrypted_secrets"].clone();
        let bytes = serde_json::to_vec(&bundle).unwrap();
        let (imported, encrypted) = read_bundle(&bytes).unwrap();
        assert!(encrypted);
        assert_eq!(imported.profiles.len(), 1);
        assert!(imported.secrets.is_empty());
        let secrets = decrypt_bundle_secrets(&bytes, f["passphrase"].as_str().unwrap()).unwrap();
        let id = imported.profiles[0].id();
        assert_eq!(secrets[&id]["password"], "試験パスワード");
        assert_eq!(secrets[&id]["passphrase"], "試験秘密鍵");
        assert!(decrypt_bundle_secrets(&bytes, "間違い").is_err());
    }

    #[test]
    fn bundle_without_secrets_is_plain_and_unknown_kinds_are_dropped() {
        let f = fixture();
        let bytes = serde_json::to_vec(&f["bundle"]).unwrap();
        let (_, encrypted) = read_bundle(&bytes).unwrap();
        assert!(!encrypted);
        assert!(decrypt_bundle_secrets(&bytes, "x").unwrap().is_empty());
        let env = crypto::encrypt(
            br#"{"a@h:22":{"password":"p","sudo":"s","passphrase":"","x":1}}"#,
            "pw",
        )
        .unwrap();
        let mut bundle = f["bundle"].clone();
        bundle["secrets"] = serde_json::to_value(env).unwrap();
        let secrets = decrypt_bundle_secrets(&serde_json::to_vec(&bundle).unwrap(), "pw").unwrap();
        assert_eq!(secrets["a@h:22"].len(), 1);
        assert_eq!(secrets["a@h:22"]["password"], "p");
    }

    fn legacy_dir(f: &Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("profiles.json"),
            serde_json::to_vec(&f["profiles"]).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("known_hosts.json"),
            serde_json::to_vec(&f["known_hosts"]).unwrap(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn legacy_reads_file_vault_and_os_vault_without_writing() {
        let f = fixture();
        let dir = legacy_dir(&f);
        let runtime = tempfile::tempdir().unwrap();
        let lease = crate::migration::acquire(runtime.path()).unwrap();
        let id = "demo@server.invalid:2222";
        // 旧版と同じ配置: .credkey（Fernet鍵）とcreds.dat（暗号化JSON）。
        let fernet_key = f["fernet_key"].as_str().unwrap();
        let body = serde_json::json!({
            format!("{id}:password"): "ファイル内パスワード",
            format!("{id}:sudo"): "sudo",
        });
        fs::write(dir.path().join(".credkey"), fernet_key).unwrap();
        fs::write(
            dir.path().join("creds.dat"),
            crypto::encrypt_fernet(fernet_key, &serde_json::to_vec(&body).unwrap()).unwrap(),
        )
        .unwrap();
        let before: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (p.clone(), fs::read(&p).unwrap())
            })
            .collect();
        let os = |name: &str| {
            Ok((name == format!("{id}:passphrase")).then(|| "OS内パスフレーズ".to_owned()))
        };
        let imported = read_legacy_with(dir.path(), &lease, &os, true).unwrap();
        assert_eq!(imported.secrets[id]["password"], "ファイル内パスワード");
        assert_eq!(imported.secrets[id]["passphrase"], "OS内パスフレーズ");
        assert!(!imported.secrets[id].contains_key("sudo"));
        assert_eq!(imported.profiles.len(), 1);
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes, "旧ファイルを変更しない");
        }
    }

    #[test]
    fn os_vault_wins_and_failures_are_reported_without_stopping() {
        let f = fixture();
        let dir = legacy_dir(&f);
        let runtime = tempfile::tempdir().unwrap();
        let lease = crate::migration::acquire(runtime.path()).unwrap();
        let ok = |_: &str| Ok(Some("OS".to_owned()));
        let imported = read_legacy_with(dir.path(), &lease, &ok, true).unwrap();
        assert_eq!(imported.secrets["demo@server.invalid:2222"].len(), 2);
        let broken = |_: &str| Err(io::Error::other("拒否"));
        let imported = read_legacy_with(dir.path(), &lease, &broken, true).unwrap();
        assert!(imported.secrets.is_empty());
        assert!(imported
            .notes
            .iter()
            .any(|n| n.contains("読めませんでした")));
    }

    #[test]
    fn undecryptable_creds_file_is_a_note_and_missing_root_is_an_error() {
        let f = fixture();
        let dir = legacy_dir(&f);
        let runtime = tempfile::tempdir().unwrap();
        let lease = crate::migration::acquire(runtime.path()).unwrap();
        fs::write(dir.path().join("creds.dat"), b"broken").unwrap();
        let none = |_: &str| Ok(None);
        let imported = read_legacy_with(dir.path(), &lease, &none, false).unwrap();
        assert!(imported.secrets.is_empty());
        assert!(imported.notes.iter().any(|n| n.contains("creds.dat")));
        assert!(imported.notes.iter().any(|n| n.contains("Secret Service")));
        assert_eq!(fs::read(dir.path().join("creds.dat")).unwrap(), b"broken");
        assert!(read_legacy_with(&dir.path().join("none"), &lease, &none, false).is_err());
    }
}
