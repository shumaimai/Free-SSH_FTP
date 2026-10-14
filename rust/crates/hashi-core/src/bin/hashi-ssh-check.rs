//! 管理下OpenSSHとの接続・転送を確認する開発用CLI。
use anyhow::{Context, Result};
use hashi_core::{
    interaction::{Interaction, Stamp},
    ssh::SshSession,
    transfer,
};
use hashi_storage::config::{HostKey, KnownHosts, Profile};
use serde::Deserialize;
use std::{io::Read, path::PathBuf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Request {
    profile: Profile,
    expected_key: HostKey,
    directory: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input)?;
    let req: Request = serde_json::from_str(&input)?;
    let known: KnownHosts = [(
        format!("{}:{}", req.profile.host, req.profile.port),
        req.expected_key,
    )]
    .into_iter()
    .collect();
    let (interaction, _requests) = Interaction::new();
    let cancel = CancellationToken::new();
    let session = SshSession::connect(
        &req.profile,
        Stamp::default(),
        known,
        interaction,
        cancel.clone(),
    )
    .await?;
    let (status, stdout, _) = session.execute("printf 'HASHI_SSH_TEST\\n'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"HASHI_SSH_TEST\n",
        "SSHコマンドの結果が一致しません"
    );
    let nav = session.sftp().await?;
    let xfer = session.sftp().await?;
    let source = req.directory.join("source.bin");
    let restored = req.directory.join("restored.bin");
    let remote = req
        .directory
        .join("remote.bin")
        .to_string_lossy()
        .into_owned();
    let bytes: Vec<u8> = (0..1024 * 1024).map(|n| (n % 251) as u8).collect();
    tokio::fs::write(&source, &bytes).await?;
    let (tx, mut rx) = mpsc::channel(16);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let upload = transfer::upload(&xfer, &source, &remote, false, "upload", &tx, &cancel);
    let directory = req.directory.to_string_lossy();
    let list = transfer::list(&nav, &directory, &cancel);
    // navとxferの独立チャネルで、同時に要求を進める。
    let (uploaded, listed) = tokio::join!(upload, list);
    uploaded?;
    listed?;
    let entries = transfer::list(&nav, &req.directory.to_string_lossy(), &cancel).await?;
    anyhow::ensure!(
        entries.iter().any(|entry| entry.name == "remote.bin"),
        "転送中も一覧を読める必要があります"
    );
    transfer::download(
        &xfer,
        &remote,
        &restored,
        transfer::DownloadPolicy::default(),
        "download",
        &tx,
        &cancel,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&restored).await? == bytes,
        "転送内容が一致しません"
    );
    tokio::fs::write(&restored, &bytes[..65536]).await?;
    transfer::download(
        &xfer,
        &remote,
        &restored,
        transfer::DownloadPolicy {
            overwrite: false,
            resume: true,
        },
        "resume",
        &tx,
        &cancel,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&restored).await? == bytes,
        "再開した内容が一致しません"
    );
    transfer::delete_tree(&nav, &remote, "delete", &cancel).await?;
    drop(tx);
    drain.await.context("進捗処理が終了しませんでした")?;
    session.close().await?;
    println!("ssh-sftp-verified");
    Ok(())
}
