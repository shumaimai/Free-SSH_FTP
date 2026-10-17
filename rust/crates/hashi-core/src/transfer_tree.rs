//! 再帰転送。リンクを追わず、件数・深さ・待ち行列を制限する。
use crate::{
    ssh::SshSession,
    transfer::{self, DownloadPolicy, Progress},
};
use anyhow::{bail, Context, Result};
use russh_sftp::{
    client::{fs::Metadata, SftpSession},
    protocol::StatusCode,
};
use std::{collections::VecDeque, path::Path};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    Refuse,
    Overwrite,
    Resume,
    Skip,
}
#[derive(Default)]
pub struct Report {
    pub files: u64,
    pub directories: u64,
    pub skipped: u64,
}
struct Budget {
    visited: usize,
    queued_bytes: usize,
}
impl Budget {
    fn enqueue(&mut self, bytes: usize) -> Result<()> {
        self.queued_bytes = self.queued_bytes.saturating_add(bytes);
        if self.queued_bytes > 16 * 1024 * 1024 {
            bail!("再帰処理のパス待ち行列が大きすぎます");
        }
        Ok(())
    }
    fn dequeue(&mut self, bytes: usize) {
        self.queued_bytes = self.queued_bytes.saturating_sub(bytes);
    }

    fn visit(&mut self, depth: usize, pending: usize) -> Result<()> {
        self.visited += 1;
        if self.visited > 100_000 || depth > 256 || pending > 100_000 {
            bail!("再帰転送の件数/深さ/待ち行列が上限を超えました");
        }
        Ok(())
    }
}
async fn remote_metadata(sftp: &SftpSession, path: &str) -> Result<Option<Metadata>> {
    match sftp.symlink_metadata(path).await {
        Ok(meta) => Ok(Some(meta)),
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == StatusCode::NoSuchFile =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
async fn local_metadata(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
pub struct TaskContext<'a> {
    pub job: &'a str,
    pub progress: &'a mpsc::Sender<Progress>,
    pub cancel: &'a CancellationToken,
    pub permission: Option<&'a std::sync::Arc<crate::permissions::Manager>>,
    pub mode: transfer::Mode,
}
pub async fn upload_tree(
    ssh: &SshSession,
    sftp: &SftpSession,
    local: &Path,
    remote: &str,
    policy: Existing,
    context: &TaskContext<'_>,
) -> Result<Report> {
    let mut access = crate::permissions::Batch::new(context.permission);
    let result = upload_tree_inner(ssh, sftp, local, remote, policy, context, &mut access).await;
    crate::permissions::complete(result, access.finish().await)
}
async fn upload_tree_inner(
    ssh: &SshSession,
    sftp: &SftpSession,
    local: &Path,
    remote: &str,
    policy: Existing,
    context: &TaskContext<'_>,
    access: &mut crate::permissions::Batch,
) -> Result<Report> {
    let (job, progress, cancel) = (context.job, context.progress, context.cancel);
    let mut pending = VecDeque::from([(local.to_path_buf(), remote.to_owned(), 0)]);
    let mut budget = Budget {
        visited: 0,
        queued_bytes: 0,
    };
    let mut report = Report::default();
    while let Some((local, remote, depth)) = pending.pop_front() {
        if cancel.is_cancelled() {
            bail!("再帰転送を取り消しました");
        }
        budget.dequeue(local.as_os_str().len() + remote.len() + 128);
        budget.visit(depth, pending.len())?;
        access.join(&remote).await;
        let meta = tokio::fs::symlink_metadata(&local).await?;
        if is_link(&meta) {
            report.skipped += 1;
            continue;
        }
        let parent = Path::new(&remote)
            .parent()
            .and_then(|p| p.to_str())
            .context("親パスが不正です")?;
        let existing = access
            .run(parent, 0o100, || remote_metadata(sftp, &remote))
            .await?;
        if existing.as_ref().is_some_and(|meta| meta.is_symlink()) {
            bail!("転送先のリンクを上書きできません");
        }
        if meta.is_dir() {
            match existing {
                Some(meta) if meta.is_dir() => {}
                Some(_) => bail!("転送先に同名の通常ファイルがあります"),
                None => {
                    let parent = Path::new(&remote)
                        .parent()
                        .and_then(|p| p.to_str())
                        .context("親パスが不正です")?;
                    access
                        .run(parent, 0o300, || async {
                            Ok(sftp.create_dir(&remote).await?)
                        })
                        .await?;
                }
            };
            report.directories += 1;
            let mut children = tokio::fs::read_dir(&local).await?;
            while let Some(child) = children.next_entry().await? {
                let name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("UTF-8で表現できないファイル名です"))?;
                transfer::safe_name(&name)?;
                budget.enqueue(child.path().as_os_str().len() + remote.len() + name.len() + 129)?;
                pending.push_back((
                    child.path(),
                    format!("{}/{}", remote.trim_end_matches('/'), name),
                    depth + 1,
                ));
                if pending.len() > 100_000 {
                    bail!("転送の待ち行列が上限を超えました");
                }
            }
        } else if meta.is_file() {
            if existing.is_some() && policy == Existing::Skip {
                report.skipped += 1;
                continue;
            }
            if existing.is_some() && policy == Existing::Resume {
                access
                    .run(&remote, 0o600, || {
                        transfer::upload_resume(sftp, &local, &remote, job, progress, cancel)
                    })
                    .await?;
                report.files += 1;
                continue;
            }
            let cleanup = access.cleanup();
            let upload = || {
                transfer::upload_scoped(
                    ssh,
                    sftp,
                    &local,
                    &remote,
                    existing.is_some() && policy == Existing::Overwrite,
                    context,
                    Some(&cleanup),
                )
            };
            if existing.is_some() {
                if context.mode == transfer::Mode::InPlace {
                    access.run(&remote, 0o200, upload).await?;
                } else {
                    // 既存原本の置換のために親の書込み権限を自動拡大しない。
                    access.require_original_parent_write(parent)?;
                    upload().await?;
                }
            } else {
                access.run(parent, 0o300, upload).await?;
            }
            report.files += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}
pub async fn download_tree(
    ssh: &SshSession,
    sftp: &SftpSession,
    remote: &str,
    local: &Path,
    policy: Existing,
    context: &TaskContext<'_>,
) -> Result<Report> {
    let mut access = crate::permissions::Batch::new(context.permission);
    let result = download_tree_inner(ssh, sftp, remote, local, policy, context, &mut access).await;
    crate::permissions::complete(result, access.finish().await)
}
async fn download_tree_inner(
    ssh: &SshSession,
    sftp: &SftpSession,
    remote: &str,
    local: &Path,
    policy: Existing,
    context: &TaskContext<'_>,
    access: &mut crate::permissions::Batch,
) -> Result<Report> {
    let (job, progress, cancel) = (context.job, context.progress, context.cancel);
    let mut pending = VecDeque::from([(remote.to_owned(), local.to_path_buf(), 0)]);
    let mut budget = Budget {
        visited: 0,
        queued_bytes: 0,
    };
    let mut report = Report::default();
    while let Some((remote, local, depth)) = pending.pop_front() {
        if cancel.is_cancelled() {
            bail!("再帰転送を取り消しました");
        }
        budget.dequeue(local.as_os_str().len() + remote.len() + 128);
        budget.visit(depth, pending.len())?;
        access.join(&remote).await;
        let meta = sftp.symlink_metadata(&remote).await?;
        if meta.is_symlink() {
            report.skipped += 1;
            continue;
        }
        let existing = local_metadata(&local).await?;
        if existing.as_ref().is_some_and(is_link) {
            bail!("転送先のリンクを上書きできません");
        }
        if meta.is_dir() {
            match existing {
                Some(meta) if meta.is_dir() => {}
                Some(_) => bail!("転送先に同名の通常ファイルがあります"),
                None => tokio::fs::create_dir(&local).await?,
            };
            report.directories += 1;
            for child in access
                .run(&remote, 0o500, || {
                    transfer::list_bounded(ssh, &remote, cancel)
                })
                .await?
            {
                transfer::safe_name(&child.name)?;
                budget
                    .enqueue(child.path.len() + local.as_os_str().len() + child.name.len() + 129)?;
                pending.push_back((child.path, local.join(child.name), depth + 1));
            }
            if pending.len() > 100_000 {
                bail!("転送の待ち行列が上限を超えました");
            }
        } else if meta.is_regular() {
            if existing.is_some() && policy == Existing::Skip {
                report.skipped += 1;
                continue;
            }
            access
                .run(&remote, 0o400, || {
                    transfer::download(
                        sftp,
                        &remote,
                        &local,
                        DownloadPolicy {
                            overwrite: policy == Existing::Overwrite,
                            resume: policy == Existing::Resume,
                            mode: context.mode,
                        },
                        job,
                        progress,
                        cancel,
                    )
                })
                .await?;
            report.files += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}
pub async fn search(
    ssh: &SshSession,
    sftp: &SftpSession,
    root: &str,
    query: &str,
    cancel: &CancellationToken,
) -> Result<Vec<transfer::Entry>> {
    if query.is_empty() || query.len() > 1024 {
        bail!("検索文字列が空/長すぎます");
    }
    let mut pending = VecDeque::from([(root.to_owned(), 0)]);
    let mut budget = Budget {
        visited: 0,
        queued_bytes: 0,
    };
    let mut hits = vec![];
    let mut hit_bytes = 0usize;
    let query = query.to_lowercase();
    while let Some((path, depth)) = pending.pop_front() {
        budget.dequeue(path.len() + 128);
        budget.visit(depth, pending.len())?;
        let metadata = sftp.symlink_metadata(&path).await?;
        if metadata.is_symlink() {
            continue;
        }
        for entry in transfer::list_bounded(ssh, &path, cancel).await? {
            budget.visit(depth, pending.len())?;
            if entry.name.to_lowercase().contains(&query) {
                hit_bytes = hit_bytes.saturating_add(entry.name.len() + entry.path.len() + 128);
                if hit_bytes > 16 * 1024 * 1024 {
                    bail!("検索結果が大きすぎます");
                }
                hits.push(entry.clone());
                if hits.len() > 10_000 {
                    bail!("検索結果が上限を超えました");
                }
            }
            if entry.directory && !entry.symlink {
                budget.enqueue(entry.path.len() + 128)?;
                pending.push_back((entry.path, depth + 1));
            }
        }
    }
    Ok(hits)
}
pub async fn list_local(path: &Path) -> Result<Vec<transfer::Entry>> {
    let mut directory = tokio::fs::read_dir(path).await?;
    let mut entries = vec![];
    let mut bytes = 0usize;
    while let Some(entry) = directory.next_entry().await? {
        let path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("UTF-8で表現できないファイル名です"))?;
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        bytes = bytes.saturating_add(name.len() + path.as_os_str().len() + 128);
        if entries.len() >= 100_000 || bytes > 16 * 1024 * 1024 {
            bail!("ローカル一覧の表示上限を超えました");
        }
        entries.push(transfer::Entry {
            name,
            path: path
                .to_str()
                .context("パスがUTF-8ではありません")?
                .to_owned(),
            directory: metadata.is_dir(),
            symlink: is_link(&metadata),
            size: metadata.len(),
            permissions: None,
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_walks_have_a_depth_and_item_budget() {
        let mut budget = Budget {
            visited: 0,
            queued_bytes: 0,
        };
        assert!(budget.visit(257, 0).is_err());
        let mut budget = Budget {
            visited: 100_000,
            queued_bytes: 0,
        };
        assert!(budget.visit(0, 0).is_err());
    }
    #[tokio::test]
    async fn local_listing_preserves_japanese_names_and_links() {
        let root = tempfile::tempdir().unwrap();
        tokio::fs::write(root.path().join("日本語.txt"), b"test")
            .await
            .unwrap();
        tokio::fs::create_dir(root.path().join("dir"))
            .await
            .unwrap();
        let entries = list_local(root.path()).await.unwrap();
        assert!(entries[0].directory);
        assert!(entries
            .iter()
            .any(|entry| entry.name == "日本語.txt" && entry.size == 4));
    }
}
