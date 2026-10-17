//! 完了するまで宛先の原本を保ち、同じディレクトリの一時ファイルから確定する。
use crate::{
    ssh::SshSession,
    transfer::{DownloadPolicy, Progress},
    transfer_tree::TaskContext,
};
use anyhow::{bail, Context, Result};
use russh_sftp::{
    client::{fs::Metadata, RawSftpSession, SftpSession},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use std::{path::Path, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
enum TransferFailure {
    Local(String),
    Committed(String),
    Unknown(String),
    Cleanup(String),
    ExistingAtomic(String),
}
impl std::fmt::Display for TransferFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(message) => write!(f, "ローカルファイル操作に失敗しました: {message}"),
            Self::Committed(message) => write!(
                f,
                "ファイルの置換は完了しましたが、永続化/属性確認が未完了です: {message}"
            ),
            Self::Unknown(message) => write!(
                f,
                "転送確定の結果が不明です。宛先を確認してください: {message}"
            ),
            Self::Cleanup(message) => write!(f, "転送の後始末が未完了です: {message}"),
            Self::ExistingAtomic(message) => write!(f,"既存原本の保持に必要な権限を確認できません。親の書込み権限は自動変更しません: {message}"),
        }
    }
}
// ローカル権限不足と確定後の失敗を、リモートchmodの自動再試行へ流さない。
impl std::error::Error for TransferFailure {}
pub(crate) fn local_error(error: impl std::fmt::Display) -> anyhow::Error {
    TransferFailure::Local(error.to_string()).into()
}
fn committed_error(error: impl std::fmt::Display) -> anyhow::Error {
    TransferFailure::Committed(error.to_string()).into()
}
fn outcome_error(error: impl std::fmt::Display) -> anyhow::Error {
    TransferFailure::Unknown(error.to_string()).into()
}

#[derive(Debug, PartialEq, Eq)]
struct LocalVersion {
    size: u64,
    modified: Option<std::time::SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    identity: (u64, u64, u32, u32, u32),
    #[cfg(windows)]
    identity: (u64, u32),
}
fn local_version(path: &Path) -> Result<Option<(LocalVersion, std::fs::Permissions)>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("通常ファイル以外へ転送できません");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            bail!("reparse pointへ転送できません");
        }
    }
    Ok(Some((
        LocalVersion {
            size: meta.len(),
            modified: meta.modified().ok(),
            readonly: meta.permissions().readonly(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino(), meta.mode(), meta.uid(), meta.gid())
            },
            #[cfg(windows)]
            identity: {
                use std::os::windows::fs::MetadataExt;
                (meta.creation_time(), meta.file_attributes())
            },
        },
        meta.permissions(),
    )))
}
fn commit_local(
    temp: tempfile::NamedTempFile,
    path: &Path,
    version: &Option<(LocalVersion, std::fs::Permissions)>,
) -> Result<()> {
    let current = local_version(path).map_err(local_error)?;
    if current.as_ref().map(|v| &v.0) != version.as_ref().map(|v| &v.0) {
        bail!("転送中に保存先が変更されました。原本を保持しました");
    }
    #[cfg(unix)]
    let parent = std::fs::File::open(
        path.parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )
    .map_err(local_error)?;
    let committed = if version.is_some() {
        temp.persist(path)
            .map_err(|error| local_error(error.error))?
    } else {
        temp.persist_noclobber(path)
            .map_err(|error| local_error(error.error))?
    };
    #[cfg(windows)]
    if let Some((_, permissions)) = version {
        // tempfileの移動準備がreadonlyを落とすため、確定した同じhandleで再適用する。
        committed
            .set_permissions(permissions.clone())
            .map_err(committed_error)?;
        if committed
            .metadata()
            .map_err(committed_error)?
            .permissions()
            .readonly()
            != permissions.readonly()
        {
            return Err(committed_error("readonly属性を確認できません"));
        }
        committed.sync_all().map_err(committed_error)?;
    }
    #[cfg(unix)]
    parent.sync_all().map_err(committed_error)?;
    drop(committed);
    Ok(())
}
pub(crate) fn same_remote(a: &Metadata, b: &Metadata) -> bool {
    (a.size, a.permissions, a.uid, a.gid, a.mtime) == (b.size, b.permissions, b.uid, b.gid, b.mtime)
}
async fn remote_metadata(session: &SftpSession, path: &str) -> Result<Option<Metadata>> {
    match session.symlink_metadata(path).await {
        Ok(meta) if meta.is_regular() => Ok(Some(meta)),
        Ok(_) => bail!("通常ファイル以外へ転送できません"),
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == StatusCode::NoSuchFile =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}
async fn progress(
    sender: &mpsc::Sender<Progress>,
    job: &str,
    done: u64,
    total: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    tokio::select! {biased;_=cancel.cancelled()=>bail!("確定前に転送を取り消しました。原本は保持しました"),result=sender.send(Progress {job_id:job.into(),done,total})=>result.context("転送画面が終了しました。原本を保持しました")}
}

pub async fn download(
    session: &SftpSession,
    remote: &str,
    local: &Path,
    policy: DownloadPolicy,
    job: &str,
    sender: &mpsc::Sender<Progress>,
    cancel: &CancellationToken,
) -> Result<()> {
    let source = remote_metadata(session, remote)
        .await?
        .context("転送元がありません")?;
    let version = local_version(local).map_err(local_error)?;
    if version.is_some() && !policy.overwrite {
        bail!("既存ファイルの上書き確認が必要です");
    }
    let parent = local
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    #[cfg(unix)]
    let new_permissions = if version.is_none() {
        use std::os::unix::fs::PermissionsExt;
        // 空のprobeだけ通常作成modeで作り、process全体のumaskを変更せずに実測する。
        let probe = tempfile::Builder::new()
            .prefix(".hashi-mode-")
            .permissions(std::fs::Permissions::from_mode(0o666))
            .tempfile_in(parent)
            .map_err(local_error)?;
        let permissions = probe
            .as_file()
            .metadata()
            .map_err(local_error)?
            .permissions();
        probe.close().map_err(local_error)?;
        Some(permissions)
    } else {
        None
    };
    let temp = tempfile::Builder::new()
        .prefix(".hashi-transfer-")
        .tempfile_in(parent)
        .map_err(local_error)?;
    let mut output = tokio::fs::File::from_std(temp.reopen().map_err(local_error)?);
    let mut input = session.open(remote).await?;
    let mut buffer = vec![0; 32768];
    let mut done = 0;
    loop {
        let size = tokio::select! {biased;_=cancel.cancelled()=>bail!("確定前に転送を取り消しました。原本は保持しました"),size=input.read(&mut buffer)=>size?};
        if size == 0 {
            break;
        }
        output
            .write_all(&buffer[..size])
            .await
            .map_err(local_error)?;
        done += size as u64;
        progress(sender, job, done, source.len(), cancel).await?;
    }
    input.close().await?;
    if done != source.len()
        || !remote_metadata(session, remote)
            .await?
            .is_some_and(|current| same_remote(&source, &current))
    {
        bail!("転送元が途中で変わりました。原本を保持しました");
    }
    if let Some((_, permissions)) = &version {
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
            let expected = version.as_ref().unwrap().0.identity;
            let current = temp.as_file().metadata().map_err(local_error)?;
            if (current.uid(), current.gid()) != (expected.3, expected.4)
                && unsafe { libc::fchown(temp.as_file().as_raw_fd(), expected.3, expected.4) } != 0
            {
                return Err(local_error(std::io::Error::last_os_error()));
            }
        }
        output
            .set_permissions(permissions.clone())
            .await
            .map_err(local_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if output
                .metadata()
                .await
                .map_err(local_error)?
                .permissions()
                .mode()
                & 0o7777
                != permissions.mode() & 0o7777
            {
                bail!("元のmodeを保てません。原本を保持しました");
            }
        }
    }
    #[cfg(unix)]
    if let Some(permissions) = new_permissions {
        output
            .set_permissions(permissions)
            .await
            .map_err(local_error)?;
    }
    output.sync_all().await.map_err(local_error)?;
    drop(output);
    if cancel.is_cancelled() {
        bail!("確定前に転送を取り消しました。原本は保持しました");
    }
    // persist自体にはawaitを挟まず、確定後に取り消されたとの応答を作らない。
    commit_local(temp, local, &version)
}

struct RemoteTemp {
    work: crate::work::Work,
    raw: Arc<RawSftpSession>,
    path: Option<String>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    scope: Option<crate::work::Work>,
    scope_token: Option<tokio_util::task::task_tracker::TaskTrackerToken>,
}
impl RemoteTemp {
    async fn finish(mut self, result: Result<()>) -> Result<()> {
        if let Some(path) = self.path.clone() {
            // await途中でこのfuture自体がDropされても、fallbackへ確認済みpathを渡す。
            let cleanup = remove_temporary(&self.raw, &path).await;
            self.path = None;
            if let Err(error) = cleanup {
                let message = match result {
                    Ok(()) => format!("転送後の一時ファイル削除は未確認 ({path}): {error}"),
                    Err(operation) => {
                        format!("{operation}; 一時ファイル削除も未確認 ({path}): {error}")
                    }
                };
                // 削除失敗をchmodの自動再試行に変換しない。
                return Err(TransferFailure::Cleanup(message).into());
            }
        }
        result
    }
}
async fn remove_temporary(raw: &RawSftpSession, path: &str) -> Result<()> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        raw.remove(path.to_owned()),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(russh_sftp::client::error::Error::Status(status)))
            if status.status_code == StatusCode::NoSuchFile =>
        {
            Ok(())
        }
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(error.into()),
    }
}
impl Drop for RemoteTemp {
    fn drop(&mut self) {
        let raw = self.raw.clone();
        let path = self.path.take();
        let permit = self.permit.take();
        let scope_token = self.scope_token.take();
        let scope = self.scope.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let errors = self.work.clone();
            self.work.cleanup(&runtime, async move {
                let _permit = permit;
                // 権限scopeの所有tokenも削除完了まで保ち、Batchの親mode復元を先行させない。
                let _scope_token = scope_token;
                if let Some(path) = path {
                    if let Err(error) = remove_temporary(&raw, &path).await {
                        let message = format!("一時ファイル削除は未確認 ({path}): {error}");
                        if let Some(scope) = &scope {
                            scope.report(message.clone());
                        }
                        errors.report(message);
                    }
                }
                let _ = raw.close_session();
            });
        }
    }
}
pub async fn upload(
    ssh: &SshSession,
    session: &SftpSession,
    local: &Path,
    remote: &str,
    overwrite: bool,
    context: &TaskContext<'_>,
    cleanup: Option<&crate::work::Work>,
) -> Result<()> {
    let _work = ssh.background.start()?;
    let source = local_version(local)
        .map_err(local_error)?
        .context("転送元がありません")?;
    let original = remote_metadata(session, remote)
        .await
        .map_err(|error| TransferFailure::ExistingAtomic(error.to_string()))?;
    let replacing = original.is_some();
    if original.is_some() && !overwrite {
        bail!("既存ファイルの上書き確認が必要です");
    }
    let (parent, name) = remote.rsplit_once('/').context("転送先の親が不明です")?;
    crate::transfer::safe_name(name)?;
    let parent = if parent.is_empty() { "/" } else { parent };
    let permit = tokio::select! {biased;_=context.cancel.cancelled()=>bail!("確定前に転送を取り消しました"),permit=ssh.transfer_commits.clone().acquire_owned()=>permit?};
    let channel = ssh.connection.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let raw = Arc::new(RawSftpSession::new(channel.into_stream()));
    let work = ssh.background.clone();
    let mut temporary = Some(RemoteTemp {
        work: work.clone(),
        raw: raw.clone(),
        path: None,
        permit: Some(permit),
        scope: cleanup.cloned(),
        scope_token: cleanup.map(crate::work::Work::start).transpose()?,
    });
    let result = async {
        let version = raw.init().await?;
        if !version
            .extensions
            .get("fsync@openssh.com")
            .is_some_and(|value| value == "1")
        {
            bail!("原本を保持する転送に必要なfsync拡張がありません");
        }
        if original.is_some()
            && !version
                .extensions
                .get("posix-rename@openssh.com")
                .is_some_and(|v| v == "1")
        {
            bail!("原本を保持する上書きに必要なposix-rename拡張がありません");
        }
        let path = format!(
            "{}/.hashi-transfer-{}.tmp",
            parent.trim_end_matches('/'),
            uuid::Uuid::new_v4()
        );
        let new_mode=if original.is_none() {
            let probe_path=format!("{}/.hashi-mode-{}.tmp",parent.trim_end_matches('/'),uuid::Uuid::new_v4());
            let probe=session.open_with_flags_and_attributes(&probe_path,
                OpenFlags::WRITE|OpenFlags::CREATE|OpenFlags::EXCLUDE,
                FileAttributes{permissions:Some(0o666),..Default::default()}).await?;
            temporary.as_mut().expect("probeの所有権").path=Some(probe_path.clone());
            let mode=probe.metadata().await?.permissions.context("新規作成modeを取得できません")? & 0o777;
            probe.close().await?;
            remove_temporary(&raw,&probe_path).await?;
            temporary.as_mut().expect("probe削除済み").path=None;
            Some(mode)
        } else {None};
        let mut output = session
            .open_with_flags_and_attributes(
                &path,
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                FileAttributes {
                    permissions: Some(0o600),
                    ..Default::default()
                },
            )
            .await?;
        // EXCL openの成功を確認したファイルだけcleanupする。ACK不明/衝突時は削除しない。
        temporary.as_mut().expect("確定前の一時ファイル").path = Some(path.clone());
        let mut input = tokio::fs::File::open(local).await.map_err(local_error)?;
        let mut buffer = vec![0; 32768];
        let mut done = 0;
        loop {
            let size = tokio::select! {biased;_=context.cancel.cancelled()=>bail!("確定前に転送を取り消しました。原本は保持しました"),size=input.read(&mut buffer)=>size.map_err(local_error)?};
            if size == 0 {
                break;
            }
            tokio::select! {biased;_=context.cancel.cancelled()=>bail!("確定前に転送を取り消しました。原本は保持しました"),result=output.write_all(&buffer[..size])=>result?};
            done += size as u64;
            progress(
                context.progress,
                context.job,
                done,
                source.0.size,
                context.cancel,
            )
            .await?;
        }
        if done != source.0.size
            || local_version(local)
                .map_err(local_error)?
                .as_ref()
                .map(|v| &v.0)
                != Some(&source.0)
        {
            bail!("転送元が途中で変わりました。原本を保持しました");
        }
        output.flush().await?;
        if let Some(original) = &original {
            output
                .set_metadata(Metadata {
                    permissions: original.permissions,
                    uid: original.uid,
                    gid: original.gid,
                    ..Default::default()
                })
                .await?;
            let readback = output.metadata().await?;
            if (
                readback.permissions.map(|v| v & 0o7777),
                readback.uid,
                readback.gid,
            ) != (
                original.permissions.map(|v| v & 0o7777),
                original.uid,
                original.gid,
            ) {
                bail!("元のmode/所有者を保てません。原本を保持しました");
            }
        }
        if let Some(mode)=new_mode {
            output.set_metadata(Metadata{permissions:Some(mode),..Default::default()}).await?;
            if output.metadata().await?.permissions.map(|value|value & 0o777)!=Some(mode) {
                bail!("新規ファイルのmodeを確認できません");
            }
        }
        output.sync_all().await?;
        if output.metadata().await?.len() != done {
            bail!("一時ファイルのサイズが違います。原本を保持しました");
        }
        output.close().await?;
        let current = remote_metadata(session, remote).await?;
        let unchanged = match (&original, &current) {
            (None, None) => true,
            (Some(a), Some(b)) => same_remote(a, b),
            _ => false,
        };
        if !unchanged {
            bail!("転送中に宛先が変わりました。原本を保持しました");
        }
        if context.cancel.is_cancelled() {
            bail!("確定前に転送を取り消しました。原本は保持しました");
        }
        let remote = remote.to_owned();
        // 確定要求とcleanupの所有者を同じタスクへ移す。ACK待ちの取消で要求を捨てない。
        let mut temporary = temporary.take().expect("確定タスクへ所有権を渡す");
        work.cleanup(&tokio::runtime::Handle::current(), async move {
            let result = async {
                if original.is_some() {
                    let mut encoded = vec![];
                    for text in [&path, &remote] {
                        encoded.extend_from_slice(&(text.len() as u32).to_be_bytes());
                        encoded.extend_from_slice(text.as_bytes());
                    }
                    match raw
                        .extended("posix-rename@openssh.com", encoded)
                        .await
                        .map_err(outcome_error)?
                    {
                        Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
                        _ => Err(outcome_error("上書き確定の成功を確認できません")),
                    }
                } else {
                    raw.rename(path, remote).await.map_err(outcome_error)?;
                    Ok(())
                }
            }
            .await;
            if result.is_ok() {
                temporary.path = None;
            }
            temporary.finish(result).await
        })
        .await
        .context("転送確定タスクが終了しました。結果は不明です")?
    }
    .await;
    // token取消や通常の失敗でも親の権限を戻す前に削除を確認する。
    let result = if let Some(temporary) = temporary {
        temporary.finish(result).await
    } else {
        result
    };
    if replacing
        && result
            .as_ref()
            .is_err_and(crate::permissions::is_permission_error)
    {
        return Err(TransferFailure::ExistingAtomic(result.unwrap_err().to_string()).into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[tokio::test]
    async fn dropped_remove_ack_keeps_cleanup_scope_until_fallback_finishes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Server {
            entered: Arc<tokio::sync::Notify>,
            resume: Arc<tokio::sync::Notify>,
            calls: Arc<AtomicUsize>,
        }
        impl russh_sftp::server::Handler for Server {
            type Error = StatusCode;
            fn unimplemented(&self) -> Self::Error {
                StatusCode::OpUnsupported
            }
            async fn remove(
                &mut self,
                id: u32,
                filename: String,
            ) -> std::result::Result<russh_sftp::protocol::Status, Self::Error> {
                assert_eq!(filename, "/confirmed-temp");
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.entered.notify_one();
                    self.resume.notified().await;
                    Ok(russh_sftp::protocol::Status {
                        id,
                        status_code: StatusCode::Ok,
                        error_message: String::new(),
                        language_tag: String::new(),
                    })
                } else {
                    // 最初のremoveは完了したが、応答待ちがDropされたケース。
                    Err(StatusCode::NoSuchFile)
                }
            }
        }
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let (client, server) = tokio::io::duplex(4096);
        russh_sftp::server::run(
            server,
            Server {
                entered: entered.clone(),
                resume: resume.clone(),
                calls: calls.clone(),
            },
        )
        .await;
        let raw = Arc::new(RawSftpSession::new(client));
        raw.init().await.unwrap();
        let work = crate::work::Work::default();
        let scope = crate::work::Work::default();
        let temporary = RemoteTemp {
            work: work.clone(),
            raw,
            path: Some("/confirmed-temp".into()),
            permit: None,
            scope: Some(scope.clone()),
            scope_token: Some(scope.start().unwrap()),
        };
        let finish = tokio::spawn(temporary.finish(Ok(())));
        entered.notified().await;
        finish.abort();
        assert!(finish.await.unwrap_err().is_cancelled());
        scope.close();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), scope.wait())
                .await
                .is_err(),
            "削除ACK前に親modeを復元できてしまう"
        );
        resume.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), scope.wait())
            .await
            .unwrap();
        work.close();
        work.wait().await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(work.take_errors().is_empty());
    }

    #[test]
    fn local_commit_preserves_conflict_and_refuses_new_destination_races() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"original").unwrap();
        let version = local_version(&path).unwrap();
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(b"replacement").unwrap();
        std::fs::write(&path, b"externally-changed").unwrap();
        assert!(commit_local(temp, &path, &version).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"externally-changed");
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(b"replacement").unwrap();
        assert!(commit_local(temp, &path, &None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"externally-changed");
        let version = local_version(&path).unwrap();
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(b"replacement").unwrap();
        temp.as_file()
            .set_permissions(version.as_ref().unwrap().1.clone())
            .unwrap();
        commit_local(temp, &path, &version).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().readonly(),
            version.as_ref().unwrap().1.readonly()
        );
    }
    #[test]
    fn local_and_committed_failures_never_trigger_remote_privilege_retry() {
        for error in [
            local_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            committed_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            outcome_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            TransferFailure::Cleanup("permission denied".into()).into(),
            TransferFailure::ExistingAtomic("permission denied".into()).into(),
        ] {
            assert!(!crate::permissions::is_permission_error(
                &error.context("上位の操作文脈")
            ));
        }
        let combined: Result<()> = crate::permissions::complete(
            Err(outcome_error("ACK喪失")),
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
        );
        let error = combined.unwrap_err();
        assert!(error.to_string().contains("結果が不明"));
        assert!(error.to_string().contains("権限復元も未完了"));
        assert!(!crate::permissions::is_permission_error(&error));
    }
    #[cfg(unix)]
    #[test]
    fn unreadable_parent_fails_before_commit_and_original_survives() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"original").unwrap();
        let version = local_version(&path).unwrap();
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(b"replacement").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o300)).unwrap();
        let result = commit_local(temp, &path, &version);
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }
    #[cfg(windows)]
    #[test]
    fn readonly_destination_keeps_attributes_on_success_or_refusal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"original").unwrap();
        let initial = std::fs::metadata(&path).unwrap().permissions();
        let mut readonly = initial.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&path, readonly).unwrap();
        let version = local_version(&path).unwrap();
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(b"replacement").unwrap();
        let result = commit_local(temp, &path, &version);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            if result.is_ok() {
                &b"replacement"[..]
            } else {
                &b"original"[..]
            }
        );
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        std::fs::set_permissions(&path, initial).unwrap();
    }
}
