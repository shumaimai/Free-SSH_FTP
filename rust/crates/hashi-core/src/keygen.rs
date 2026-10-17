//! SSH鍵ペアの生成と公開鍵のサーバー登録（Python版`keygen.py`相当）。
//! 秘密鍵は全種別をOpenSSH形式で保存する（Python版のRSA/ECDSAは従来PEM。Python版もOpenSSH形式を読める）。
use anyhow::{bail, Context, Result};
use russh::keys::ssh_key::{
    private::{KeypairData, RsaKeypair},
    LineEnding, PrivateKey,
};
use russh::keys::{Algorithm, EcdsaCurve};
use russh_sftp::{
    client::{fs::Metadata, SftpSession},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub const ECDSA_BITS: [u16; 3] = [256, 384, 521];
pub const RSA_BITS: [usize; 3] = [2048, 3072, 4096];
const AUTHORIZED_KEYS_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    Ecdsa(u16),
    Rsa(usize),
}

/// 鍵を生成する。RSAは時間がかかるのでGUIスレッドで呼ばない。
pub fn generate(kind: KeyKind, comment: &str) -> Result<PrivateKey> {
    let mut rng = rand::rng();
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
        KeyKind::Ecdsa(bits) => {
            let curve = match bits {
                256 => EcdsaCurve::NistP256,
                384 => EcdsaCurve::NistP384,
                521 => EcdsaCurve::NistP521,
                _ => bail!("ECDSA のビット数が不正です。"),
            };
            PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve })?
        }
        KeyKind::Rsa(bits) => {
            if !RSA_BITS.contains(&bits) {
                bail!("RSA のビット数が不正です。");
            }
            PrivateKey::new(KeypairData::from(RsaKeypair::random(&mut rng, bits)?), "")?
        }
    };
    key.set_comment(comment.trim());
    Ok(key)
}

/// OpenSSHの公開鍵1行。コメントが空なら付けない（Python版と同じ）。
pub fn public_line(key: &PrivateKey) -> Result<String> {
    Ok(key.public_key().to_openssh()?.trim_end().to_owned())
}

/// パスフレーズが空でなければbcrypt-pbkdf/AES-256-CTRで暗号化したOpenSSH形式。
pub fn private_openssh(key: &PrivateKey, passphrase: &str) -> Result<String> {
    let encoded = if passphrase.is_empty() {
        key.to_openssh(LineEnding::LF)?
    } else {
        key.encrypt(&mut rand::rng(), passphrase)?
            .to_openssh(LineEnding::LF)?
    };
    Ok(encoded.to_string())
}

/// 同じフォルダの一時ファイルへ書いてから置き換える。`overwrite`でなければ既存を拒否する。
/// Unixは0600。Windowsは保存先フォルダのACLを継承する（`location_warning`で知らせる）。
pub fn write_private_key(path: &Path, content: &str, overwrite: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // 新しく作るフォルダ（~/.ssh等）は本人だけが使えるようにする。既存フォルダのmodeは変えない。
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(parent)
        .with_context(|| format!("保存先フォルダを作成できません: {}", parent.display()))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".hashi-key-")
        .tempfile_in(parent)
        .context("一時ファイルを作成できません")?;
    // tempfileはUnixで0600を作る。念のため明示する。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    std::io::Write::write_all(&mut temp, content.as_bytes())?;
    temp.as_file().sync_all()?;
    let result = if overwrite {
        temp.persist(path).map(|_| ())
    } else {
        temp.persist_noclobber(path).map(|_| ())
    };
    result.map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("既存の秘密鍵ファイルがあります。上書きを確認してください")
        } else {
            anyhow::anyhow!("秘密鍵を保存できません: {}", error.error)
        }
    })
}

/// Windowsで保存先が ~/.ssh 外なら注意文を返す（Python版 #135 案3）。
pub fn location_warning(path: &Path, is_windows: bool, home: &Path) -> Option<&'static str> {
    if !is_windows {
        return None;
    }
    let normalize = |path: &Path| {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        PathBuf::from(absolute.to_string_lossy().to_lowercase())
    };
    let ssh = normalize(&home.join(".ssh"));
    let destination = normalize(path);
    if destination.starts_with(&ssh) && destination != ssh {
        return None;
    }
    Some(
        "保存先が ~/.ssh 以外です。Windows ではファイルの権限を変更しないため、\
         秘密鍵は保存先フォルダのアクセス権をそのまま引き継ぎます。\n\
         他のユーザーが読める場所の可能性があります。保存先のアクセス権を確認してください。",
    )
}

/// authorized_keysの有効行に、同じ「種別 base64」の並びがあるか（オプション付き行も含む）。
pub fn contains_key(existing: &str, public_line: &str) -> bool {
    let identity: Vec<&str> = public_line.split_whitespace().take(2).collect();
    if identity.len() < 2 {
        return false;
    }
    existing
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .any(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            fields
                .windows(identity.len())
                .any(|window| window == identity)
        })
}

fn not_found(error: &russh_sftp::client::error::Error) -> bool {
    matches!(error, russh_sftp::client::error::Error::Status(status)
        if status.status_code == StatusCode::NoSuchFile)
}

/// 接続中のサーバーへ公開鍵を重複なく登録する。戻り値は新規追加したか。
/// 公開鍵をシェルへ埋め込まず、SFTPで ~/.ssh と authorized_keys を操作する。
/// 既存内容は書き換えず、末尾へ追記する（途中で切れても既存の鍵を失わない）。
pub async fn register_public_key(
    ssh: &crate::ssh::SshSession,
    sftp: &SftpSession,
    public_line: &str,
) -> Result<bool> {
    let public_line = public_line.trim();
    if public_line.is_empty() || public_line.contains(['\n', '\r']) {
        bail!("公開鍵が空か、1行ではありません。");
    }
    let (status, home, error) = ssh.execute("printf \"%s\" \"$HOME\"").await?;
    let home = String::from_utf8_lossy(&home).trim().to_owned();
    if status != 0 || !home.starts_with('/') {
        let detail = String::from_utf8_lossy(&error).trim().to_owned();
        bail!(
            "{}",
            if detail.is_empty() {
                "ホームディレクトリを取得できませんでした。".to_owned()
            } else {
                detail
            }
        );
    }
    let ssh_dir = format!("{}/.ssh", home.trim_end_matches('/'));
    register_in(sftp, &ssh_dir, public_line).await
}

/// `ssh_dir`（通常は ~/.ssh）の authorized_keys へ登録する。試験では一時フォルダを渡す。
pub async fn register_in(sftp: &SftpSession, ssh_dir: &str, public_line: &str) -> Result<bool> {
    let public_line = public_line.trim();
    if public_line.is_empty() || public_line.contains(['\n', '\r']) {
        bail!("公開鍵が空か、1行ではありません。");
    }
    let ssh_dir = ssh_dir.to_owned();
    let authorized_keys = format!("{ssh_dir}/authorized_keys");
    match sftp.metadata(ssh_dir.clone()).await {
        Ok(_) => {}
        Err(error) if not_found(&error) => sftp.create_dir(ssh_dir.clone()).await?,
        Err(error) => bail!("~/.ssh の状態を確認できませんでした: {error}"),
    }
    sftp.set_metadata(
        ssh_dir.clone(),
        Metadata {
            permissions: Some(0o700),
            ..Default::default()
        },
    )
    .await
    .context("~/.ssh の権限を設定できません")?;
    let mut file = sftp
        .open_with_flags_and_attributes(
            authorized_keys.clone(),
            OpenFlags::READ | OpenFlags::WRITE | OpenFlags::CREATE,
            FileAttributes {
                permissions: Some(0o600),
                ..Default::default()
            },
        )
        .await
        .context("authorized_keys を開けませんでした")?;
    let result = async {
        let size = file.metadata().await?.len();
        if size > AUTHORIZED_KEYS_LIMIT {
            bail!("authorized_keys が大きすぎます（1MiB超）。手動で確認してください");
        }
        let mut bytes = Vec::with_capacity(size as usize);
        (&mut file)
            .take(AUTHORIZED_KEYS_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() as u64 != size {
            bail!("authorized_keys が読込み中に変更されました");
        }
        let existing = String::from_utf8_lossy(&bytes);
        if contains_key(&existing, public_line) {
            return anyhow::Ok(false);
        }
        let separator = if bytes.is_empty() || bytes.ends_with(b"\n") {
            ""
        } else {
            "\n"
        };
        file.seek(std::io::SeekFrom::Start(size)).await?;
        file.write_all(format!("{separator}{public_line}\n").as_bytes())
            .await?;
        file.flush().await?;
        anyhow::Ok(true)
    }
    .await;
    let closed = file.close().await;
    let added = result?;
    closed.context("authorized_keys の保存を確認できません")?;
    sftp.set_metadata(
        authorized_keys,
        Metadata {
            permissions: Some(0o600),
            ..Default::default()
        },
    )
    .await
    .context("authorized_keys の権限を設定できません")?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed25519_and_ecdsa_round_trip_with_passphrase() {
        for kind in [KeyKind::Ed25519, KeyKind::Ecdsa(256), KeyKind::Ecdsa(521)] {
            let key = generate(kind, " 試験 ").unwrap();
            let line = public_line(&key).unwrap();
            assert!(line.ends_with(" 試験"), "{line}");
            let encrypted = private_openssh(&key, "日本語の合言葉").unwrap();
            assert!(encrypted.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
            let loaded = PrivateKey::from_openssh(&encrypted).unwrap();
            assert!(loaded.is_encrypted());
            let decrypted = loaded.decrypt("日本語の合言葉").unwrap();
            assert_eq!(decrypted.public_key(), key.public_key());
            assert!(loaded.decrypt("違う").is_err());
            let plain = PrivateKey::from_openssh(private_openssh(&key, "").unwrap()).unwrap();
            assert!(!plain.is_encrypted());
        }
        let bare = generate(KeyKind::Ed25519, "").unwrap();
        assert_eq!(public_line(&bare).unwrap().split(' ').count(), 2);
        assert!(generate(KeyKind::Ecdsa(128), "").is_err());
        assert!(generate(KeyKind::Rsa(1024), "").is_err());
    }

    #[test]
    fn private_key_is_written_without_clobbering() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("id_ed25519");
        write_private_key(&path, "first", false).unwrap();
        let error = write_private_key(&path, "second", false).unwrap_err();
        assert!(error.to_string().contains("上書き"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        write_private_key(&path, "second", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let parent = std::fs::metadata(path.parent().unwrap()).unwrap();
            assert_eq!(parent.permissions().mode() & 0o777, 0o700);
        }
        let leftovers = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().file_name() != "id_ed25519")
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn windows_location_warning_follows_python() {
        let home = Path::new("/home/user");
        assert!(location_warning(Path::new("/tmp/key"), false, home).is_none());
        assert!(location_warning(Path::new("/home/user/.ssh/id"), true, home).is_none());
        assert!(location_warning(Path::new("/home/User/.SSH/id"), true, home).is_none());
        assert!(location_warning(Path::new("/home/user/.ssh"), true, home).is_some());
        assert!(location_warning(Path::new("/home/user/.sshx/id"), true, home).is_some());
        assert!(location_warning(Path::new("/tmp/key"), true, home).is_some());
    }

    #[test]
    fn duplicate_detection_ignores_comments_and_accepts_options() {
        let line = "ssh-ed25519 AAAAkey new-comment";
        assert!(contains_key("ssh-ed25519 AAAAkey old\n", line));
        assert!(contains_key(
            "from=\"10.0.0.1\" ssh-ed25519 AAAAkey\n",
            line
        ));
        assert!(!contains_key("# ssh-ed25519 AAAAkey\n", line));
        assert!(!contains_key("ssh-ed25519 AAAAother\n", line));
        assert!(!contains_key("ssh-ed25519 AAAAkey\n", "garbage"));
    }
}

/// OpenSSHの`ssh-keygen -y`で、生成した秘密鍵（暗号化あり/なし）から同じ公開鍵を復元できるか。
#[cfg(all(test, unix))]
#[test]
fn openssh_reads_generated_keys() {
    let Some(keygen) = ["/usr/bin/ssh-keygen", "/bin/ssh-keygen"]
        .into_iter()
        .find(|path| Path::new(path).exists())
    else {
        eprintln!("ssh-keygenが無いため省略");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    for (index, (kind, passphrase)) in [
        (KeyKind::Ed25519, "合言葉"),
        (KeyKind::Ecdsa(384), ""),
        (KeyKind::Rsa(2048), "rsa-pass"),
    ]
    .into_iter()
    .enumerate()
    {
        let key = generate(kind, "c").unwrap();
        let path = directory.path().join(format!("key{index}"));
        write_private_key(&path, &private_openssh(&key, passphrase).unwrap(), false).unwrap();
        let output = std::process::Command::new(keygen)
            .args(["-y", "-P", passphrase, "-f"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        let restored = String::from_utf8(output.stdout).unwrap();
        let expected = public_line(&key).unwrap();
        assert_eq!(
            restored.split_whitespace().take(2).collect::<Vec<_>>(),
            expected.split_whitespace().take(2).collect::<Vec<_>>()
        );
    }
}
