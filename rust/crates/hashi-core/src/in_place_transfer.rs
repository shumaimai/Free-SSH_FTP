//! Python互換の直接書込み。取消/失敗では部分データを保持し、原本保全を約束しない。
use crate::{ssh::SshSession, transfer::Progress, transfer_tree::TaskContext};
use anyhow::{bail, Context, Result};
use russh_sftp::{
    client::{fs::Metadata, SftpSession},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn upload(
    ssh: &SshSession,
    session: &SftpSession,
    local: &Path,
    remote: &str,
    overwrite: bool,
    context: &TaskContext<'_>,
) -> Result<()> {
    let _work = ssh.background.start()?;
    let mut input = tokio::fs::File::open(local)
        .await
        .map_err(crate::atomic_transfer::local_error)?;
    let source = input
        .metadata()
        .await
        .map_err(crate::atomic_transfer::local_error)?;
    if !source.is_file() {
        bail!("通常ファイルを選択してください");
    }
    let existing = match session.symlink_metadata(remote).await {
        Ok(meta) if meta.is_regular() => Some(meta),
        Ok(_) => bail!("通常ファイル以外へ直接書込みできません"),
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == StatusCode::NoSuchFile =>
        {
            None
        }
        Err(error) => bail!("転送先の状態を確認できません。権限の自動再試行はしません: {error}"),
    };
    if existing.is_some() && !overwrite {
        bail!("既存ファイルの上書き確認が必要です");
    }
    if context.cancel.is_cancelled() {
        bail!("直接書込み開始前に取り消しました");
    }
    let flags = if existing.is_some() {
        OpenFlags::WRITE
    } else {
        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE
    };
    // 既存ファイルへmode属性を送ると、open時に属性を適用するサーバーで権限が変わる。
    let attributes = if existing.is_some() {
        FileAttributes::default()
    } else {
        FileAttributes {
            permissions: Some(0o666),
            ..Default::default()
        }
    };
    let mut output = session
        .open_with_flags_and_attributes(remote, flags, attributes)
        .await?;
    // 新規作成はopen成功で転送先が変わる。既存ファイルは切り詰め直前から変更扱い。
    let modified = std::sync::atomic::AtomicBool::new(existing.is_none());
    let result=async {
        let opened=output.metadata().await?;
        if !opened.is_regular(){bail!("書込み先の種類が変わりました");}
        if let Some(expected)=&existing {
            let current=session.symlink_metadata(remote).await?;
            if !current.is_regular() || !crate::atomic_transfer::same_remote(expected,&current)
                || !crate::atomic_transfer::same_remote(&current,&opened) {
                bail!("書込み先が外部で変更されました");
            }
        }
        if context.cancel.is_cancelled(){bail!("転送を取り消しました");}
        modified.store(true, std::sync::atomic::Ordering::Relaxed);
        if existing.is_some(){output.set_metadata(Metadata{size:Some(0),..Default::default()}).await?;}
        let mut buffer=vec![0;32768];
        let mut done=0;
        loop {
            let size=tokio::select!{biased;_=context.cancel.cancelled()=>bail!("転送を取り消しました"),size=input.read(&mut buffer)=>size?};
            if size==0 {break;}
            tokio::select!{biased;_=context.cancel.cancelled()=>bail!("転送を取り消しました"),result=output.write_all(&buffer[..size])=>result?};
            done+=size as u64;
            tokio::select!{biased;_=context.cancel.cancelled()=>bail!("転送を取り消しました"),result=context.progress.send(Progress{job_id:context.job.into(),done,total:source.len()})=>result.context("転送画面が終了しました")?};
        }
        // flushは全write ACKを待つ。サーバーが持たないfsync拡張を必須にしない。
        output.flush().await?;
        if done!=source.len() || output.metadata().await?.len()!=done {bail!("転送サイズが一致しません");}
        anyhow::Ok(())
    }.await;
    let closed = output.close().await.map_err(anyhow::Error::from);
    // 変更後の失敗を権限補助の自動再試行へ流さず、部分ファイルを残す。
    complete(
        result,
        closed,
        "直接書込み",
        modified.load(std::sync::atomic::Ordering::Relaxed),
    )
}
/// `modified`は転送先へ書込み/切り詰め/新規作成を始めたか。始める前の停止は原本のままと示す。
pub(crate) fn complete(
    result: Result<()>,
    closed: Result<()>,
    kind: &str,
    modified: bool,
) -> Result<()> {
    let state = if modified {
        "転送先が変更/部分更新された可能性があります（削除していません）"
    } else {
        "転送先へ書き込む前に停止したため、転送先は変更していません"
    };
    match (result, closed) {
        (Ok(()), Ok(())) => Ok(()),
        (operation, close) => bail!(
            "{kind}は未完了です。{state}。操作: {}; close: {}",
            operation
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "完了".into()),
            close
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "完了".into())
        ),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn partial_update_and_close_failure_are_kept_without_privilege_retry() {
        let error = super::complete(
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
            Err(anyhow::anyhow!("close-ACK喪失")),
            "再開",
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("部分更新"));
        assert!(error.to_string().contains("close-ACK喪失"));
        assert!(!crate::permissions::is_permission_error(&error));
    }

    #[test]
    fn failure_before_modification_reports_unchanged_target() {
        let error = super::complete(
            Err(anyhow::anyhow!("書込み先が外部で変更されました")),
            Ok(()),
            "直接書込み",
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("変更していません"));
        assert!(!error.contains("部分更新"));
    }
}
