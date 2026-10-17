//! 旧Pythonの復旧記録は読み取りだけ行い、未復元/判定不能なら新しい権限変更を止める。
use crate::migration::PythonMarker;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct Inspection {
    pub paths: Vec<PathBuf>,
    pub pending: usize,
}
impl Inspection {
    pub fn require_clear(&self) -> io::Result<()> {
        if self.pending != 0 {
            return Err(io::Error::other(format!(
                "旧Python版に{}件の未復元記録があります。旧版で権限を復旧して終了後、やり直してください。記録は変更していません: {}",
                self.pending, self.paths.iter().map(|path|path.display().to_string()).collect::<Vec<_>>().join(", ")
            )));
        }
        Ok(())
    }
}
pub fn inspect(default_root: &Path, markers: &[PythonMarker]) -> io::Result<Inspection> {
    let roots: BTreeSet<_> = std::iter::once(default_root.to_path_buf())
        .chain(
            markers
                .iter()
                .map(|marker| PathBuf::from(&marker.data_root)),
        )
        .collect();
    let mut result = Inspection::default();
    for root in roots {
        let path = root.join("perm_journal.json");
        let count = read_pending(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "旧Python版の復旧記録を確認できません。権限補助を開始しません ({}): {error}",
                    path.display()
                ),
            )
        })?;
        result.pending += count;
        result.paths.push(path);
    }
    Ok(result)
}
fn read_pending(path: &Path) -> io::Result<usize> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > LIMIT {
        return Err(io::Error::other("旧復旧記録の種類/サイズが不正です"));
    }
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other(
                "旧復旧記録のreparse pointは確認できません",
            ));
        }
    }
    if !metadata.is_file() {
        return Err(io::Error::other("旧復旧記録が通常ファイルではありません"));
    }
    let mut bytes = vec![];
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("旧復旧記録が大きすぎます"));
    }
    let data: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::other("旧復旧記録の形式が不正です"))?;
    // 未知のentryも未完了として保ち、PID死亡を根拠に無視/削除しない。
    Ok(data.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_and_unknown_entries_block_without_changing_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("perm_journal.json");
        for bytes in [
            br#"{"1":{"conn":"host","path":"/file","orig":0,"pid":999999}}"#.as_slice(),
            br#"{"future":{}}"#,
        ] {
            std::fs::write(&path, bytes).unwrap();
            let status = inspect(root.path(), &[]).unwrap();
            assert_eq!(status.pending, 1);
            assert!(status.require_clear().is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        for bytes in [b"{".as_slice(), b"[]"] {
            std::fs::write(&path, bytes).unwrap();
            assert!(inspect(root.path(), &[]).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(&path, b"{}").unwrap();
        inspect(root.path(), &[]).unwrap().require_clear().unwrap();
    }
    #[test]
    fn absent_journal_is_only_a_read_check() {
        let root = tempfile::tempdir().unwrap();
        inspect(root.path(), &[]).unwrap().require_clear().unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn linked_journal_is_not_assumed_clear() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("real"), b"{}").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("real"),
            root.path().join("perm_journal.json"),
        )
        .unwrap();
        assert!(inspect(root.path(), &[]).is_err());
    }
}
