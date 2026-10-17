//! chmod前の永続記録とパスごとのプロセス間lease。復元失敗の記録を消さない。
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub schema_version: u32,
    pub id: String,
    pub connection: String,
    pub path: String,
    pub original_mode: u32,
    pub owner: String,
}
pub struct Journal {
    root: PathBuf,
}
pub struct PathLease {
    _file: File,
    connection: String,
    path: String,
}
fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 16384
        && !path.contains('\0')
        && (path == "/"
            || !path
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == ".."))
}
fn reject_link(path: &Path) -> Result<()> {
    let info = fs::symlink_metadata(path)?;
    if info.file_type().is_symlink() {
        bail!("権限記録にリンクを使えません");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            bail!("権限記録にreparse pointを使えません");
        }
    }
    Ok(())
}
fn validate_file(file: &File) -> Result<()> {
    let info = file.metadata()?;
    if !info.is_file() {
        bail!("権限記録/leaseが通常ファイルではありません");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if info.uid() != unsafe { libc::geteuid() } || info.nlink() != 1 {
            bail!("権限記録/leaseの所有者またはリンク数が不正です");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            bail!("権限記録/leaseにreparse pointは使えません");
        }
    }
    Ok(())
}
impl Journal {
    pub fn at(root: PathBuf) -> Result<Self> {
        if root.try_exists()? {
            reject_link(&root)?;
        }
        // 既存の新保存領域と同じ所有者/リンク検証を使う。旧Python rootは許可しない。
        let _ = hashi_storage::config::Store::at(root.clone())?;
        reject_link(&root)?;
        let preparing = root.join("preparing");
        if preparing.try_exists()? {
            reject_link(&preparing)?;
        }
        let _ = hashi_storage::config::Store::at(preparing.clone())?;
        reject_link(&preparing)?;
        Ok(Self { root })
    }
    fn file_guard(&self) -> Result<File> {
        let path = self.root.join("journal.guard");
        if path.try_exists()? {
            reject_link(&path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options.open(path)?;
        validate_file(&file)?;
        // fsyncや同時プロセス起動による短い競合を待つ。外部所有者で無期限停止しない。
        let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => break,
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                        && std::time::Instant::now() < until =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(2))
                }
                Err(error) => return Err(error).context("別の処理が権限記録を保存しています"),
            }
        }
        Ok(file)
    }
    fn key(_connection: &str, path: &str) -> String {
        let mut hash = Sha256::new();
        // 別利用者/host aliasで同じ対象を重ねて緩めない。現段階は全サーバーで同名パスを排他。
        hash.update(path.as_bytes());
        format!("{:x}", hash.finalize())
    }
    pub fn try_lease(&self, connection: &str, path: &str) -> Result<PathLease> {
        if connection.is_empty()
            || connection.len() > 2048
            || connection.contains('\0')
            || !valid_path(path)
        {
            bail!("権限対象の接続/パスが不正です");
        }
        let resource_path = path.to_owned();
        let path = self
            .root
            .join(format!("{}.lock", Self::key(connection, path)));
        if path.try_exists()? {
            reject_link(&path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options.open(path)?;
        validate_file(&file)?;
        FileExt::try_lock_exclusive(&file).context("別の権限操作がこのパスを保持しています")?;
        Ok(PathLease {
            _file: file,
            connection: connection.into(),
            path: resource_path,
        })
    }
    pub fn record(
        &self,
        connection: &str,
        path: &str,
        original_mode: u32,
        owner: &str,
        lease: &PathLease,
    ) -> Result<Entry> {
        if lease.connection != connection || lease.path != path {
            bail!("権限leaseの対象が一致しません");
        }
        let _guard = self.file_guard()?;
        self.pending_locked(connection)?;
        if !valid_path(path) || original_mode > 0o7777 || owner.is_empty() || owner.len() > 256 {
            bail!("元の権限記録が不正です");
        }
        let entry = Entry {
            schema_version: 2,
            id: uuid::Uuid::new_v4().to_string(),
            connection: connection.into(),
            path: path.into(),
            original_mode,
            owner: owner.into(),
        };
        // 未commitのtempはchmod前の準備データ。クラッシュ後も正常な記録の復元を妨げない。
        let mut temp = tempfile::NamedTempFile::new_in(self.root.join("preparing"))?;
        temp.write_all(&serde_json::to_vec(&entry)?)?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(self.root.join(format!("{}.json", entry.id)))
            .map_err(|error| error.error)?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(entry)
    }
    pub fn pending(&self, connection: &str) -> Result<Vec<Entry>> {
        let _guard = self.file_guard()?;
        self.pending_locked(connection)
    }
    pub fn pending_all(&self) -> Result<Vec<Entry>> {
        self.pending("")
    }
    fn pending_locked(&self, connection: &str) -> Result<Vec<Entry>> {
        let mut result = vec![];
        let mut total = 0usize;
        for file in fs::read_dir(&self.root)? {
            let path = file?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                bail!("権限記録の名前が不正です");
            };
            if name == "journal.guard" || name.ends_with(".lock") {
                continue;
            }
            if name == "preparing" {
                reject_link(&path)?;
                if !fs::metadata(&path)?.is_dir() {
                    bail!("権限記録の準備領域が不正です");
                }
                continue;
            }
            if !name.ends_with(".json") {
                bail!("未完了/未知の権限記録があります");
            }
            reject_link(&path)?;
            let mut bytes = vec![];
            let file = File::open(&path)?;
            validate_file(&file)?;
            file.take(65537).read_to_end(&mut bytes)?;
            if bytes.len() > 65536 {
                bail!("権限記録が大きすぎます");
            }
            let entry: Entry = serde_json::from_slice(&bytes)
                .context("権限記録が破損しています（元の記録を保持）")?;
            if entry.schema_version != 2
                || name != format!("{}.json", entry.id)
                || uuid::Uuid::parse_str(&entry.id).is_err()
                || !valid_path(&entry.path)
                || entry.original_mode > 0o7777
                || entry.connection.is_empty()
                || entry.connection.len() > 2048
                || entry.connection.contains('\0')
                || entry.owner.is_empty()
                || entry.owner.len() > 256
            {
                bail!("権限記録の版/内容が不正です");
            }
            total += 1;
            if total > 10000 {
                bail!("権限記録が多すぎます");
            }
            if connection.is_empty() || entry.connection == connection {
                result.push(entry);
            }
        }
        result.sort_by_key(|entry| std::cmp::Reverse(entry.path.matches('/').count()));
        Ok(result)
    }
    pub fn clear_restored(&self, entry: &Entry, lease: &PathLease) -> Result<()> {
        let _guard = self.file_guard()?;
        if lease.connection != entry.connection || lease.path != entry.path {
            bail!("権限leaseの対象が一致しません");
        }
        if uuid::Uuid::parse_str(&entry.id).is_err() {
            bail!("権限記録IDが不正です");
        }
        let path = self.root.join(format!("{}.json", entry.id));
        reject_link(&path)?;
        let file = File::open(&path)?;
        validate_file(&file)?;
        let current: Entry = serde_json::from_reader(file.take(65537))?;
        if &current != entry {
            bail!("権限記録の内容が変わったため削除できません");
        }
        fs::remove_file(path)?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_blocks_other_owner_and_records_survive_reopen() {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::at(root.path().join("journal")).unwrap();
        let lease = journal.try_lease("conn", "/file").unwrap();
        assert!(journal.try_lease("conn", "/file").is_err());
        let entry = journal
            .record("conn", "/file", 0o600, "owner", &lease)
            .unwrap();
        assert_eq!(journal.pending("conn").unwrap().len(), 1);
        drop(lease);
        let second = Journal::at(root.path().join("journal")).unwrap();
        let lease = second.try_lease("conn", "/file").unwrap();
        second.clear_restored(&entry, &lease).unwrap();
        assert!(second.pending("conn").unwrap().is_empty());
    }
    #[test]
    fn corrupt_journal_is_never_overwritten_and_order_is_deep_first() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal");
        let journal = Journal::at(path.clone()).unwrap();
        for target in ["/parent", "/parent/child"] {
            let lease = journal.try_lease("conn", target).unwrap();
            journal
                .record("conn", target, 0o000, "owner", &lease)
                .unwrap();
        }
        assert_eq!(journal.pending("conn").unwrap()[0].path, "/parent/child");
        fs::write(path.join("preparing/crashed-temp"), b"{partial").unwrap();
        assert_eq!(journal.pending("conn").unwrap().len(), 2);
        fs::write(path.join("broken.json"), b"{broken").unwrap();
        assert!(journal.pending("conn").is_err());
        assert_eq!(fs::read(path.join("broken.json")).unwrap(), b"{broken");
    }
}
