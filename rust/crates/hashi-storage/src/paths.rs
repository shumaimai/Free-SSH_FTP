use std::{env, io, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Python,
    Preview,
    Native,
}

impl Channel {
    pub fn directory_name(self) -> &'static str {
        match self {
            Self::Python => "Hashi",
            Self::Preview => "Hashi-RustPreview",
            Self::Native => "Hashi-Rust",
        }
    }

    pub fn credential_service(self) -> &'static str {
        match self {
            Self::Python => "Hashi",
            Self::Preview => "Hashi.RustPreview",
            Self::Native => "Hashi.Rust",
        }
    }

    pub fn ai_credential_service(self) -> &'static str {
        match self {
            Self::Python => "Hashi.AI",
            Self::Preview => "Hashi.RustPreview.AI",
            Self::Native => "Hashi.Rust.AI",
        }
    }
}

fn home() -> io::Result<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ホームが不明です"))
}

/// パス解決はディレクトリ作成や旧データの書き換えを行わない。
pub fn config_root(channel: Channel) -> io::Result<PathBuf> {
    let base = match env::var_os(if cfg!(windows) {
        "APPDATA"
    } else {
        "XDG_CONFIG_HOME"
    }) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home()?.join(if cfg!(windows) {
            "AppData/Roaming"
        } else {
            ".config"
        }),
    };
    Ok(base.join(channel.directory_name()))
}

pub fn runtime_root() -> io::Result<PathBuf> {
    let base = if cfg!(windows) {
        match env::var_os("LOCALAPPDATA") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => home()?.join("AppData/Local"),
        }
    } else {
        match env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .or_else(|| env::var_os("XDG_CONFIG_HOME"))
        {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => home()?.join(".config"),
        }
    };
    Ok(base.join("Hashi-Migration/runtime"))
}

/// 復旧記録はXDG_RUNTIME_DIRへ保存しない。ログアウト後もpreview/nativeで共用する。
pub fn permission_journal_root() -> io::Result<PathBuf> {
    let base = if cfg!(windows) {
        match env::var_os("LOCALAPPDATA") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => home()?.join("AppData/Local"),
        }
    } else {
        config_root(Channel::Native)?
            .parent()
            .ok_or_else(|| io::Error::other("永続保存先が不明です"))?
            .to_path_buf()
    };
    Ok(base.join("Hashi-Rust-Coordination/permission-journal-v2"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_channels_are_isolated() {
        let channels = [Channel::Python, Channel::Preview, Channel::Native];
        for (index, a) in channels.iter().enumerate() {
            for b in &channels[index + 1..] {
                assert_ne!(config_root(*a).unwrap(), config_root(*b).unwrap());
                assert_ne!(a.credential_service(), b.credential_service());
                assert_ne!(a.ai_credential_service(), b.ai_credential_service());
            }
        }
    }
}
