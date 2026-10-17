//! fsync拡張のない管理下SFTPで、直接書込みの互換を確認する。
use anyhow::Result;
use hashi_core::{
    ssh::SshSession,
    transfer::{self, Mode},
    transfer_tree::TaskContext,
};
use std::path::Path;
use tokio_util::sync::CancellationToken;
pub async fn verify(session: &SshSession, root: &Path) -> Result<()> {
    let sftp = session.sftp().await?;
    let source = root.join("compat-source");
    let remote = root.join("read-only/existing");
    let remote = remote.to_string_lossy();
    let bytes = vec![b'x'; 1024 * 1024];
    tokio::fs::write(&source, &bytes).await?;
    let token = CancellationToken::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut context = TaskContext {
        job: "compat",
        progress: &tx,
        cancel: &token,
        permission: None,
        mode: Mode::Atomic,
    };
    anyhow::ensure!(
        transfer::upload(session, &sftp, &source, &remote, true, &context)
            .await
            .is_err(),
        "拡張なしの原本保持を拒否しません"
    );
    anyhow::ensure!(
        tokio::fs::read(remote.as_ref()).await? == b"original",
        "原本保持の拒否で内容が変わりました"
    );
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(remote.as_ref())?.ino()
    };
    context.mode = Mode::InPlace;
    let journal = std::sync::Arc::new(hashi_core::permission_journal::Journal::at(
        root.join("race-journal"),
    )?);
    let manager = hashi_core::permissions::Manager::new(
        std::sync::Arc::new(hashi_core::permissions::SftpBackend::new(
            session.sftp().await?,
        )),
        journal.clone(),
        "fixture".into(),
    );
    for (name, mode) in [("atomic", Mode::Atomic), ("direct", Mode::InPlace)] {
        let destination = root.join(format!("read-only/race-{name}"));
        let raced = TaskContext {
            job: "raced-destination",
            progress: &tx,
            cancel: &token,
            permission: Some(&manager),
            mode,
        };
        anyhow::ensure!(
            hashi_core::transfer_tree::upload_tree(
                session,
                &sftp,
                &source,
                &destination.to_string_lossy(),
                hashi_core::transfer_tree::Existing::Overwrite,
                &raced
            )
            .await
            .is_err(),
            "新規観測後に現れた宛先を上書きしました"
        );
        anyhow::ensure!(
            tokio::fs::read(&destination).await? == b"race-original",
            "競合した原本を変えました"
        );
        anyhow::ensure!(
            journal.pending("fixture")?.is_empty(),
            "競合で親の権限補助を試みました"
        );
    }
    anyhow::ensure!(
        manager.shutdown().await? == 0,
        "競合で権限復元記録が残りました"
    );
    transfer::upload(session, &sftp, &source, &remote, true, &context).await?;
    anyhow::ensure!(
        tokio::fs::read(remote.as_ref()).await? == bytes,
        "直接上書きの内容が違います"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        anyhow::ensure!(
            std::fs::metadata(remote.as_ref())?.ino() == identity,
            "直接上書きでinodeが変わりました"
        );
        anyhow::ensure!(
            std::fs::metadata(root.join("read-only"))?
                .permissions()
                .mode()
                & 0o777
                == 0o500,
            "親のmodeが変わりました"
        );
    }
    let new = root.join("new-direct");
    transfer::upload(
        session,
        &sftp,
        &source,
        &new.to_string_lossy(),
        false,
        &context,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&new).await? == bytes,
        "拡張なし新規uploadが違います"
    );
    let local = root.join("local-direct");
    tokio::fs::write(&local, b"local-original").await?;
    #[cfg(unix)]
    let local_identity = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&local)?.ino()
    };
    transfer::download(
        &sftp,
        &remote,
        &local,
        transfer::DownloadPolicy {
            overwrite: true,
            resume: false,
            mode: Mode::InPlace,
        },
        "compat-download",
        &tx,
        &token,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&local).await? == bytes,
        "直接downloadが違います"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            std::fs::metadata(&local)?.ino() == local_identity,
            "直接downloadでinodeが変わりました"
        );
    }
    tokio::fs::write(&local, b"keep-cancelled-download").await?;
    let late = CancellationToken::new();
    let cancellation = late.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancellation.cancel();
    });
    anyhow::ensure!(
        transfer::download(
            &sftp,
            &root.join("slow-source").to_string_lossy(),
            &local,
            transfer::DownloadPolicy {
                overwrite: true,
                resume: false,
                mode: Mode::InPlace
            },
            "cancel-before-truncate",
            &tx,
            &late
        )
        .await
        .is_err(),
        "取消が効きません"
    );
    timer.await?;
    anyhow::ensure!(
        tokio::fs::read(&local).await? == b"keep-cancelled-download",
        "open応答待ち中の取消で原本を切り詰めました"
    );
    let cancel = CancellationToken::new();
    let (updates, mut receiver) = tokio::sync::mpsc::channel(1);
    let cancellation = TaskContext {
        job: "cancel-direct",
        progress: &updates,
        cancel: &cancel,
        permission: None,
        mode: Mode::InPlace,
    };
    {
        let upload = transfer::upload(session, &sftp, &source, &remote, true, &cancellation);
        tokio::pin!(upload);
        tokio::select! {
            update=receiver.recv()=>{
                anyhow::ensure!(update.is_some(),"取消前の進捗がありません");cancel.cancel();
                let error=upload.await.unwrap_err();
                anyhow::ensure!(error.to_string().contains("部分更新"),"部分更新の結果がありません: {error}");
                anyhow::ensure!(!hashi_core::permissions::is_permission_error(&error),"直接更新後の失敗を再試行してしまいます");
            },
            result=&mut upload=>{result?;anyhow::bail!("取消前に完了しました");},
        }
    }
    anyhow::ensure!(
        tokio::fs::try_exists(remote.as_ref()).await?,
        "取消で部分ファイルを削除しました"
    );
    drop(tx);
    drain.await?;
    Ok(())
}
