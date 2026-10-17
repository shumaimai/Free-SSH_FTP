//! Rust版が保存するSSHの秘密（パスワード/パスフレーズ）。
//! Windowsは資格情報マネージャ（Rust版専用の名前）、他のOSは旧版と同じ形式の暗号化ファイル。
//! 旧版の保存先・資格情報には書かない。名前は旧版と同じ`user@host:port:kind`。
use crate::{
    paths::Channel,
    secrets::{EncryptedSecrets, Namespace},
};
use std::{io, path::PathBuf};

/// 保存する種類。sudoはRust版が使わないので保存しない。
pub const KINDS: [&str; 2] = ["password", "passphrase"];

pub fn key(profile_id: &str, kind: &str) -> String {
    format!("{profile_id}:{kind}")
}

enum Backend {
    File(EncryptedSecrets),
    #[cfg(windows)]
    Windows(Channel),
}

pub struct Vault {
    backend: Backend,
}

impl Vault {
    pub fn open(channel: Channel) -> io::Result<Self> {
        if channel == Channel::Python {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "旧版の認証情報へ書き込みできません",
            ));
        }
        #[cfg(windows)]
        {
            Ok(Self {
                backend: Backend::Windows(channel),
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                backend: Backend::File(EncryptedSecrets::new(channel, Namespace::Ssh)?),
            })
        }
    }

    /// 暗号化ファイルを使う保存先（試験用・非Windows）。
    pub fn file_at(root: PathBuf) -> io::Result<Self> {
        Ok(Self {
            backend: Backend::File(EncryptedSecrets::with_root(root, Namespace::Ssh)?),
        })
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            Backend::File(_) => "暗号化ファイル",
            #[cfg(windows)]
            Backend::Windows(_) => "Windows資格情報マネージャ",
        }
    }

    pub fn get(&self, name: &str) -> io::Result<Option<String>> {
        match &self.backend {
            Backend::File(file) => file.get(name),
            #[cfg(windows)]
            Backend::Windows(channel) => crate::winvault::get(channel.credential_service(), name),
        }
    }

    pub fn set(&self, name: &str, value: Option<&str>) -> io::Result<()> {
        match &self.backend {
            Backend::File(file) => file.set(name, value),
            #[cfg(windows)]
            Backend::Windows(channel) => crate::winvault::set(*channel, false, name, value),
        }
    }

    /// そのプロファイルの保存済みの種類。
    pub fn kinds_present(&self, profile_id: &str) -> io::Result<Vec<&'static str>> {
        let mut found = vec![];
        for kind in KINDS {
            if self.get(&key(profile_id, kind))?.is_some() {
                found.push(kind);
            }
        }
        Ok(found)
    }

    pub fn forget(&self, profile_id: &str) -> io::Result<()> {
        for kind in KINDS {
            self.set(&key(profile_id, kind), None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_forget_touch_only_that_profile() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::file_at(dir.path().to_owned()).unwrap();
        assert!(vault.kinds_present("a@h:22").unwrap().is_empty());
        vault
            .set(&key("a@h:22", "password"), Some("秘密1"))
            .unwrap();
        vault
            .set(&key("a@h:22", "passphrase"), Some("秘密2"))
            .unwrap();
        vault.set(&key("b@h:22", "password"), Some("別")).unwrap();
        assert_eq!(
            vault.kinds_present("a@h:22").unwrap(),
            vec!["password", "passphrase"]
        );
        vault.forget("a@h:22").unwrap();
        assert!(vault.kinds_present("a@h:22").unwrap().is_empty());
        assert_eq!(
            vault.get(&key("b@h:22", "password")).unwrap().as_deref(),
            Some("別")
        );
    }

    #[test]
    fn python_channel_is_refused() {
        assert!(Vault::open(Channel::Python).is_err());
    }
}
