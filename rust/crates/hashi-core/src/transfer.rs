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

fn safe_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        bail!("転送先のファイル名が不正です");
    }
    #[cfg(windows)]
    {
        let first = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if name.ends_with(['.', ' '])
            || name.contains(':')
            || ["CON", "PRN", "AUX", "NUL"].contains(&first.as_str())
            || (first.len() == 4
                && (first.starts_with("COM") || first.starts_with("LPT"))
                && first.as_bytes()[3].is_ascii_digit())
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
    let total = session.metadata(remote).await?.len();
    let exists = tokio::fs::try_exists(local).await?;
    if exists && !policy.overwrite && !policy.resume {
        bail!("既存ファイルの上書き確認が必要です");
    }
    let mut input = session.open(remote).await?;
    let offset = if exists && policy.resume {
        tokio::fs::metadata(local).await?.len()
    } else {
        0
    };
    if offset > total {
        bail!("再開元がリモートファイルより大きいです");
    }
    // 同じ部分ファイルであることを確認し、別内容へ追記しない。
    if offset > 0 {
        let mut previous = tokio::fs::File::open(local).await?;
        let mut expected = vec![0; 32768];
        let mut actual = vec![0; 32768];
        let mut checked = 0;
        while checked < offset {
            let count = (offset - checked).min(32768) as usize;
            tokio::select! { _=cancel.cancelled()=>bail!("再開確認を取り消しました"),result=input.read_exact(&mut actual[..count])=>{result?;} }
            previous.read_exact(&mut expected[..count]).await?;
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
    let mut output = options.open(local).await?;
    let mut bytes = vec![0; 32768];
    let mut done = offset;
    loop {
        let count = tokio::select! { _=cancel.cancelled()=>bail!("転送を取り消しました"),result=input.read(&mut bytes)=>result? };
        if count == 0 {
            break;
        }
        output.write_all(&bytes[..count]).await?;
        done += count as u64;
        notify(progress, job, done, total, cancel).await?;
    }
    output.sync_all().await?;
    if done != total {
        bail!("転送したサイズが一致しません");
    }
    Ok(())
}

pub async fn upload(
    session: &SftpSession,
    local: &Path,
    remote: &str,
    overwrite: bool,
    job: &str,
    progress: &mpsc::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<()> {
    match session.symlink_metadata(remote).await {
        Ok(meta) => {
            if !overwrite {
                bail!("既存ファイルの上書き確認が必要です");
            }
            if meta.is_dir() {
                bail!("ディレクトリへファイルを上書きできません");
            }
        }
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == StatusCode::NoSuchFile => {}
        Err(error) => return Err(error.into()),
    }
    let mut input = tokio::fs::File::open(local).await?;
    let total = input.metadata().await?.len();
    let flags = if overwrite {
        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
    } else {
        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE
    };
    let mut output = session.open_with_flags(remote, flags).await?;
    let mut bytes = vec![0; 32768];
    let mut done = 0;
    loop {
        let count = tokio::select! { _=cancel.cancelled()=>bail!("転送を取り消しました"),result=input.read(&mut bytes)=>result? };
        if count == 0 {
            break;
        }
        tokio::select! { _=cancel.cancelled()=>bail!("転送を取り消しました"),result=output.write_all(&bytes[..count])=>result? };
        done += count as u64;
        notify(progress, job, done, total, cancel).await?;
    }
    output.flush().await?;
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
