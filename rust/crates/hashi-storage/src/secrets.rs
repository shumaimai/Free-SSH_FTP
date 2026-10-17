//! 旧版と同じFernet形式を、新しい保存先で使う。
use crate::{crypto, paths::Channel};
use fs2::FileExt;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum Namespace {
    Ssh,
    Ai,
}
impl Namespace {
    fn names(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Ssh => (".credkey", "creds.dat", "ssh-secrets.guard"),
            Self::Ai => (".aikey", "ai-secrets.dat", "ai-secrets.guard"),
        }
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub struct EncryptedSecrets {
    root: PathBuf,
    namespace: Namespace,
}
impl EncryptedSecrets {
    pub fn new(channel: Channel, namespace: Namespace) -> io::Result<Self> {
        if channel == Channel::Python {
            return Err(invalid("旧認証ストアへ書き込みできません"));
        }
        Self::with_root(crate::paths::config_root(channel)?, namespace)
    }

    /// 保存先を指定して開く。旧版の保存先は`Store::at`が拒否する。
    pub fn with_root(root: PathBuf, namespace: Namespace) -> io::Result<Self> {
        crate::config::Store::at(root.clone())?;
        Ok(Self { root, namespace })
    }

    /// 保存先のフォルダ(同じ場所に別の取引用ロックを置くため)。
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    fn at(root: PathBuf, namespace: Namespace) -> Self {
        Self { root, namespace }
    }

    fn key(&self, create: bool) -> io::Result<Option<String>> {
        let (key_name, data_name, _) = self.namespace.names();
        let path = self.root.join(key_name);
        match fs::File::open(&path) {
            Ok(file) => {
                let mut key = String::new();
                file.take(65).read_to_string(&mut key)?;
                if key.len() > 64 {
                    return Err(invalid("保存鍵のサイズが不正です"));
                }
                if fernet::Fernet::new(&key).is_none() {
                    return Err(invalid("保存鍵の形式が不正です"));
                }
                Ok(Some(key))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.root.join(data_name).try_exists()? {
                    return Err(invalid("認証情報が存在しますが鍵がありません"));
                }
                if !create {
                    return Ok(None);
                }
                let key = fernet::Fernet::generate_key();
                let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
                temp.write_all(key.as_bytes())?;
                temp.as_file().sync_all()?;
                temp.persist_noclobber(&path).map_err(|e| e.error)?;
                Ok(Some(key))
            }
            Err(error) => Err(error),
        }
    }

    fn load(&self, key: Option<&str>) -> io::Result<BTreeMap<String, String>> {
        let (_, name, _) = self.namespace.names();
        match crate::config::read_optional(&self.root.join(name))? {
            Some(token) => {
                let token = std::str::from_utf8(&token)
                    .map_err(|_| invalid("認証ファイルを解析できません"))?;
                let raw = crypto::decrypt_fernet(
                    key.ok_or_else(|| invalid("保存鍵がありません"))?,
                    token,
                )?;
                serde_json::from_slice(&raw).map_err(|_| invalid("認証ファイルの内容が不正です"))
            }
            None => Ok(BTreeMap::new()),
        }
    }

    pub fn get(&self, name: &str) -> io::Result<Option<String>> {
        let guard = self.lock()?;
        let key = self.key(false)?;
        let result = self.load(key.as_deref())?.remove(name);
        drop(guard);
        Ok(result)
    }
    pub fn set(&self, name: &str, value: Option<&str>) -> io::Result<()> {
        let _guard = self.lock()?;
        let key = self.key(value.is_some())?;
        let mut values = self.load(key.as_deref())?;
        match value {
            Some(value) => {
                values.insert(name.into(), value.into());
            }
            None => {
                values.remove(name);
            }
        }
        let Some(key) = key else {
            return Ok(());
        };
        let token = crypto::encrypt_fernet(&key, &serde_json::to_vec(&values)?)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.write_all(token.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(self.root.join(self.namespace.names().1))
            .map_err(|e| e.error)?;
        #[cfg(unix)]
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }
    fn lock(&self) -> io::Result<fs::File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(self.namespace.names().2))?;
        FileExt::try_lock_exclusive(&file)?;
        Ok(file)
    }
}

/// 旧ストアは読み取りのみ。取り込み元を共有してRustから更新しない。
pub fn read_legacy_file(
    root: &Path,
    namespace: Namespace,
    _lease: &crate::migration::MigrationLease,
) -> io::Result<BTreeMap<String, String>> {
    EncryptedSecrets {
        root: root.into(),
        namespace,
    }
    .load(
        EncryptedSecrets {
            root: root.into(),
            namespace,
        }
        .key(false)?
        .as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_and_ai_are_isolated_and_corruption_does_not_reset_secrets() {
        let root = tempfile::tempdir().unwrap();
        let ssh = EncryptedSecrets::at(root.path().into(), Namespace::Ssh);
        let ai = EncryptedSecrets::at(root.path().into(), Namespace::Ai);
        assert!(ssh.get("test").unwrap().is_none());
        assert!(!root.path().join(".credkey").exists());
        ssh.set("test", Some("試験用SSH秘密")).unwrap();
        ai.set("test", Some("試験用AI秘密")).unwrap();
        assert_eq!(ssh.get("test").unwrap().as_deref(), Some("試験用SSH秘密"));
        assert_eq!(ai.get("test").unwrap().as_deref(), Some("試験用AI秘密"));
        let ai_bytes = fs::read(root.path().join("ai-secrets.dat")).unwrap();
        fs::write(root.path().join("creds.dat"), b"invalid").unwrap();
        assert!(ssh.set("second", Some("secret")).is_err());
        assert_eq!(fs::read(root.path().join("creds.dat")).unwrap(), b"invalid");
        assert_eq!(
            fs::read(root.path().join("ai-secrets.dat")).unwrap(),
            ai_bytes
        );
    }
    #[test]
    fn missing_key_does_not_overwrite_existing_ciphertext() {
        let root = tempfile::tempdir().unwrap();
        let ssh = EncryptedSecrets::at(root.path().into(), Namespace::Ssh);
        fs::write(root.path().join("creds.dat"), b"ciphertext").unwrap();
        assert!(ssh.set("test", Some("secret")).is_err());
        assert!(!root.path().join(".credkey").exists());
        assert_eq!(
            fs::read(root.path().join("creds.dat")).unwrap(),
            b"ciphertext"
        );
    }
}
