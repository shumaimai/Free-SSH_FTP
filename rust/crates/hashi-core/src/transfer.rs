use crate::atomic_transfer::local_error;
use anyhow::{bail, Context, Result};
use russh_sftp::{
    client::SftpSession,
    protocol::{OpenFlags, StatusCode},
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub symlink: bool,
    pub size: u64,
    pub permissions: Option<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Progress {
    pub job_id: String,
    pub done: u64,
    pub total: u64,
}

pub async fn list(
    session: &SftpSession,
    path: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Entry>> {
    let entries = tokio::select! { _=cancel.cancelled()=>bail!("一覧取得を取り消しました"),result=session.read_dir(path)=>result? };
    let mut result = vec![];
    for entry in entries {
        let meta = entry.metadata();
        result.push(Entry {
            name: entry.file_name(),
            path: entry.path(),
            directory: meta.is_dir(),
            symlink: meta.is_symlink(),
            size: meta.len(),
            permissions: meta.permissions,
        });
        if result.len() > 100_000 {
            bail!("ディレクトリの項目数が上限を超えました");
        }
    }
    result.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(result)
}

// 全件を取得する高水準read_dirの前に、件数と文字列合計の上限を適用する。
pub async fn list_bounded(
    ssh: &crate::ssh::SshSession,
    path: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Entry>> {
    if path.len() > 16384
        || !(path.starts_with('/')
            || (path.as_bytes().get(1) == Some(&b':')
                && path.as_bytes().get(2) == Some(&b'/')
                && path.as_bytes()[0].is_ascii_alphabetic()))
        || path.contains('\0')
    {
        bail!("一覧取得のパスが不正です");
    }
    let raw = tokio::select! {_=cancel.cancelled()=>bail!("一覧取得を取り消しました"),raw=ssh.raw_sftp()=>raw?};
    let handle = tokio::select! {_=cancel.cancelled()=>bail!("一覧取得を取り消しました"),handle=raw.opendir(path)=>handle?.handle};
    struct Directory {
        raw: std::sync::Arc<russh_sftp::client::RawSftpSession>,
        handle: Option<String>,
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                let raw = self.raw.clone();
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(1),
                            raw.close(handle),
                        )
                        .await;
                        let _ = raw.close_session();
                    });
                }
            }
        }
    }
    let mut directory = Directory {
        raw: raw.clone(),
        handle: Some(handle.clone()),
    };
    let mut entries = vec![];
    let mut bytes = 0usize;
    loop {
        let response = tokio::select! {_=cancel.cancelled()=>bail!("一覧取得を取り消しました"),response=raw.readdir(&handle)=>response};
        let response = match response {
            Ok(value) => value,
            Err(russh_sftp::client::error::Error::Status(status))
                if status.status_code == StatusCode::Eof =>
            {
                break
            }
            Err(error) => return Err(error.into()),
        };
        if response.files.is_empty() {
            bail!("SFTP一覧の応答が進みません");
        }
        for file in response.files {
            if file.filename == "." || file.filename == ".." {
                continue;
            }
            remote_name(&file.filename)?;
            bytes = bytes.saturating_add(file.filename.len() * 2 + path.len() + 128);
            if entries.len() >= 100_000 || bytes > 16 * 1024 * 1024 {
                bail!("ディレクトリの表示上限を超えました");
            }
            let attrs = file.attrs;
            entries.push(Entry {
                path: format!("{}/{}", path.trim_end_matches('/'), file.filename),
                name: file.filename,
                directory: attrs.is_dir(),
                symlink: attrs.is_symlink(),
                size: attrs.size.unwrap_or(0),
                permissions: attrs.permissions,
            });
        }
    }
    raw.close(&handle).await?;
    directory.handle = None;
    let _ = raw.close_session();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}
fn remote_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 16384
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
    {
        bail!("リモートのファイル名が不正です");
    }
    Ok(())
}

pub(crate) fn safe_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        bail!("転送先のファイル名が不正です");
    }
    #[cfg(windows)]
    {
        let first = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        let device = first
            .strip_prefix("COM")
            .or_else(|| first.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && suffix
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'))
            });
        if name.ends_with(['.', ' '])
            || name.contains([':', '<', '>', '"', '|', '?', '*'])
            || name.chars().any(|c| c < ' ')
            || name.encode_utf16().count() > 255
            || ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&first.as_str())
            || device
        {
            bail!("Windowsへ保存できないファイル名です");
        }
    }
    Ok(())
}

async fn notify(
    sender: &mpsc::Sender<Progress>,
    job: &str,
    done: u64,
    total: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    tokio::select! { _=cancel.cancelled()=>bail!("転送を取り消しました"),result=sender.send(Progress{job_id:job.into(),done,total})=>result.context("転送画面が終了しました") }
}

#[derive(Clone, Copy, Default)]
pub struct DownloadPolicy {
    pub overwrite: bool,
    pub resume: bool,
}

pub async fn download(
    session: &SftpSession,
    remote: &str,
    local: &Path,
    policy: DownloadPolicy,
    job: &str,
    progress: &mpsc::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<()> {
    if !policy.resume {
        return crate::atomic_transfer::download(
            session, remote, local, policy, job, progress, cancel,
        )
        .await;
    }
    let total = session.metadata(remote).await?.len();
    let exists = tokio::fs::try_exists(local).await.map_err(local_error)?;
    if exists && !policy.overwrite && !policy.resume {
        bail!("既存ファイルの上書き確認が必要です");
    }
    let mut input = session.open(remote).await?;
    let offset = if exists && policy.resume {
        tokio::fs::metadata(local).await.map_err(local_error)?.len()
    } else {
        0
    };
    if offset > total {
        bail!("再開元がリモートファイルより大きいです");
    }
    // 同じ部分ファイルであることを確認し、別内容へ追記しない。
    if offset > 0 {
        let mut previous = tokio::fs::File::open(local).await.map_err(local_error)?;
        let mut expected = vec![0; 32768];
        let mut actual = vec![0; 32768];
        let mut checked = 0;
        while checked < offset {
            let count = (offset - checked).min(32768) as usize;
            tokio::select! { _=cancel.cancelled()=>bail!("再開確認を取り消しました"),result=input.read_exact(&mut actual[..count])=>{result?;} }
            previous
                .read_exact(&mut expected[..count])
                .await
                .map_err(local_error)?;
            if actual[..count] != expected[..count] {
                bail!("再開元の内容が一致しません");
            }
            checked += count as u64;
        }
    }
    input.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true);
    if policy.resume && exists {
        options.append(true);
    } else if exists {
        options.truncate(true);
    } else {
        options.create_new(true);
    }
    let mut output = options.open(local).await.map_err(local_error)?;
    let mut bytes = vec![0; 32768];
    let mut done = offset;
    loop {
        let count = tokio::select! { _=cancel.cancelled()=>bail!("転送を取り消しました"),result=input.read(&mut bytes)=>result? };
        if count == 0 {
            break;
        }
        output
            .write_all(&bytes[..count])
            .await
            .map_err(local_error)?;
        done += count as u64;
        notify(progress, job, done, total, cancel).await?;
    }
    output.sync_all().await.map_err(local_error)?;
    if done != total {
        bail!("転送したサイズが一致しません");
    }
    Ok(())
}

pub use crate::atomic_transfer::upload;

pub async fn upload_resume(
    session: &SftpSession,
    local: &Path,
    remote: &str,
    job: &str,
    progress: &mpsc::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<()> {
    let metadata = session.symlink_metadata(remote).await?;
    if !metadata.is_regular() {
        bail!("通常ファイル以外は再開できません");
    }
    let mut input = tokio::fs::File::open(local).await.map_err(local_error)?;
    let total = input.metadata().await.map_err(local_error)?.len();
    let offset = metadata.len();
    if offset > total {
        bail!("リモートの再開元がローカルより大きいです");
    }
    let mut previous = session.open(remote).await?;
    let mut expected = vec![0; 32768];
    let mut actual = vec![0; 32768];
    let mut checked = 0;
    while checked < offset {
        let size = (offset - checked).min(32768) as usize;
        tokio::select! {_=cancel.cancelled()=>bail!("再開確認を取り消しました"),result=previous.read_exact(&mut actual[..size])=>{result?;}}
        input
            .read_exact(&mut expected[..size])
            .await
            .map_err(local_error)?;
        if expected[..size] != actual[..size] {
            bail!("再開元の内容が一致しません");
        }
        checked += size as u64;
    }
    previous.close().await?;
    if session.metadata(remote).await?.len() != offset {
        bail!("再開確認中にリモートファイルが変わりました");
    }
    let mut output = session.open_with_flags(remote, OpenFlags::WRITE).await?;
    output.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut bytes = vec![0; 32768];
    let mut done = offset;
    loop {
        let size = tokio::select! {_=cancel.cancelled()=>bail!("転送を取り消しました"),size=input.read(&mut bytes)=>size.map_err(local_error)?};
        if size == 0 {
            break;
        }
        tokio::select! {_=cancel.cancelled()=>bail!("転送を取り消しました"),result=output.write_all(&bytes[..size])=>result?}
        done += size as u64;
        notify(progress, job, done, total, cancel).await?;
    }
    output.flush().await?;
    output.sync_all().await?;
    output.close().await?;
    if done != total {
        bail!("転送元のサイズが変わりました");
    }
    Ok(())
}

pub async fn delete_tree(
    session: &SftpSession,
    path: &str,
    confirmation: &str,
    cancel: &CancellationToken,
) -> Result<()> {
    if confirmation != "delete" {
        bail!("削除の確認文字が一致しません");
    }
    let mut stack = vec![(path.to_owned(), false)];
    while let Some((path, visited)) = stack.pop() {
        if cancel.is_cancelled() {
            bail!("削除を取り消しました");
        }
        let meta = session.symlink_metadata(&path).await?;
        if meta.is_dir() && !meta.is_symlink() {
            if visited {
                session.remove_dir(path).await?;
            } else {
                stack.push((path.clone(), true));
                for entry in session.read_dir(&path).await? {
                    safe_name(&entry.file_name())?;
                    stack.push((entry.path(), false));
                }
            }
        } else {
            session.remove_file(path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_names_cannot_escape_local_destination() {
        for name in ["..", "../file", "a/b", "a\\b", "\0"] {
            assert!(safe_name(name).is_err());
        }
        assert!(safe_name("日本語.txt").is_ok());
    }
}
