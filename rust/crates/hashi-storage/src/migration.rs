use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

pub const GUARD_NAME: &str = "migration.guard";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PythonMarker {
    pub schema_version: u32,
    pub implementation: String,
    pub application_version: String,
    pub pid: u32,
    pub process_start: String,
    pub instance_id: String,
    pub executable: String,
    pub data_root: String,
}

/// 保持中は対応Python版の起動を防ぐ。確認後も操作の終了まで保持する。
pub struct MigrationLease {
    _file: File,
    pub stale_markers: Vec<PythonMarker>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn reject_link(path: &Path) -> io::Result<()> {
    let info = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if info.file_attributes() & 0x400 != 0 {
            return Err(invalid("移行領域にreparse pointは使えません"));
        }
    }
    if info.file_type().is_symlink() {
        return Err(invalid("移行領域にリンクは使えません"));
    }
    Ok(())
}

pub fn acquire(directory: &Path) -> io::Result<MigrationLease> {
    fs::create_dir_all(directory)?;
    reject_link(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // 所有者の検証は呼出し元の利用者領域にも必要。
        if fs::metadata(directory)?.uid() != unsafe { libc::geteuid() } {
            return Err(invalid("移行領域の所有者が異なります"));
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    let guard_path = directory.join(GUARD_NAME);
    if guard_path.try_exists()? {
        reject_link(&guard_path)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&guard_path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("移行ガードは通常ファイルである必要があります"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(invalid("移行ガードにreparse pointは使えません"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata()?.uid() != unsafe { libc::geteuid() } {
            return Err(invalid("移行ガードの所有者が異なります"));
        }
    }
    // stdの新しい同名メソッドではなく、Pythonと互換なfs2を明示。
    FileExt::try_lock_exclusive(&file)?;
    let mut stale_markers = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !name.starts_with("python-") {
            continue;
        }
        if !name.ends_with(".json") {
            return Err(invalid("未完了の実行中マーカーが残っています"));
        }
        reject_link(&path)?;
        let info = fs::metadata(&path)?;
        if !info.is_file() || info.len() > 16 * 1024 {
            return Err(invalid("実行中マーカーの種類/サイズが不正です"));
        }
        let marker: PythonMarker = serde_json::from_slice(&fs::read(&path)?)
            .map_err(|_| invalid("実行中マーカーを解析できません"))?;
        if marker.schema_version != SCHEMA_VERSION
            || marker.implementation != "python"
            || marker.pid == 0
            || marker.instance_id.len() != 32
            || !marker.instance_id.bytes().all(|b| b.is_ascii_hexdigit())
            || name != format!("python-{}.json", marker.instance_id)
            || marker.process_start.is_empty()
            || !Path::new(&marker.executable).is_absolute()
            || !Path::new(&marker.data_root).is_absolute()
            || marker.application_version.is_empty()
        {
            return Err(invalid("実行中マーカーの形式が不正です"));
        }
        #[cfg(windows)]
        if marker
            .process_start
            .strip_prefix("windows:")
            .and_then(|s| s.parse::<u64>().ok())
            .is_none()
        {
            return Err(invalid("実行中マーカーの開始時刻が不正です"));
        }
        #[cfg(target_os = "linux")]
        if marker
            .process_start
            .strip_prefix("linux:")
            .and_then(|s| s.rsplit_once(':'))
            .and_then(|(boot, ticks)| ticks.parse::<u64>().ok().filter(|_| !boot.is_empty()))
            .is_none()
        {
            return Err(invalid("実行中マーカーの開始時刻が不正です"));
        }
        // ガードが空いていても、記録のプロセスが生存中/判定不明なら進まない。
        if process_start(marker.pid)?.as_deref() == Some(&marker.process_start) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "旧Python版が実行中です",
            ));
        }
        stale_markers.push(marker);
    }
    Ok(MigrationLease {
        _file: file,
        stale_markers,
    })
}

#[cfg(target_os = "linux")]
fn process_start(pid: u32) -> io::Result<Option<String>> {
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let ticks = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .ok_or_else(|| invalid("プロセス開始時刻を取得できません"))?;
    Ok(Some(format!("linux:{}:{ticks}", boot.trim())))
}

#[cfg(windows)]
fn process_start(pid: u32) -> io::Result<Option<String>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    // 読み取り専用のプロセス照会。アクセス拒否を死亡扱いしない。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let mut times: [FILETIME; 4] = std::mem::zeroed();
        let ptr = times.as_mut_ptr();
        let ok = GetProcessTimes(handle, ptr, ptr.add(1), ptr.add(2), ptr.add(3));
        let error = if ok == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(handle);
        if let Some(error) = error {
            return Err(error);
        }
        let created = times[0].dwLowDateTime as u64 | (times[0].dwHighDateTime as u64) << 32;
        Ok(Some(format!("windows:{created}")))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_start(_pid: u32) -> io::Result<Option<String>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "プロセス開始時刻は未対応です",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_python_lock_blocks_migration_and_crash_release_allows_it() {
        let directory = tempfile::tempdir().unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.path().join(GUARD_NAME))
            .unwrap();
        FileExt::try_lock_shared(&file).unwrap();
        assert!(acquire(directory.path()).is_err());
        drop(file);
        let lease = acquire(directory.path()).unwrap();
        assert!(FileExt::try_lock_shared(
            &OpenOptions::new()
                .read(true)
                .write(true)
                .open(directory.path().join(GUARD_NAME))
                .unwrap()
        )
        .is_err());
        drop(lease);
        assert!(acquire(directory.path()).is_ok());
    }

    #[test]
    fn corrupt_or_future_marker_is_not_ignored() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("python-corrupt.json"), b"{").unwrap();
        assert!(acquire(directory.path()).is_err());
    }
}
