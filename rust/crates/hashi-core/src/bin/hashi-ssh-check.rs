//! 管理下OpenSSHとの接続・転送を確認する開発用CLI。
use anyhow::{Context, Result};
use hashi_core::{
    interaction::{Interaction, Stamp},
    ssh::SshSession,
    transfer,
};
use hashi_storage::config::{HostKey, KnownHosts, Profile};
use serde::Deserialize;
use std::{io::Read, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Request {
    profile: Profile,
    expected_key: HostKey,
    directory: PathBuf,
    #[serde(default)]
    ssh_config: Option<PathBuf>,
    #[serde(default)]
    expected_agent_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input)?;
    let req: Request = serde_json::from_str(&input)?;
    let resolved = match req.ssh_config.as_deref() {
        Some(path) => hashi_core::ssh_config::resolve_at(&req.profile, path)?,
        None => req.profile.clone(),
    };
    let known: KnownHosts = [(
        format!("{}:{}", resolved.host, resolved.port),
        req.expected_key,
    )]
    .into_iter()
    .collect();
    let (interaction, _requests) = Interaction::new();
    let cancel = CancellationToken::new();
    let session = Arc::new(
        SshSession::connect_with_config(
            &req.profile,
            Stamp::default(),
            known,
            interaction,
            cancel.clone(),
            req.ssh_config.as_deref(),
        )
        .await?,
    );
    if let Some(key) = req.expected_agent_key.as_ref() {
        let mut channel = session.shell(80, 24).await?;
        channel.data(&b"ssh-add -L\r"[..]).await?;
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let mut output = vec![];
            while let Some(message) = channel.wait().await {
                if let russh::ChannelMsg::Data { data } = message {
                    output.extend_from_slice(&data);
                }
                anyhow::ensure!(output.len() <= 1024 * 1024, "agent試験出力が大きすぎます");
                if String::from_utf8_lossy(&output).contains(key) {
                    return anyhow::Ok(());
                }
            }
            anyhow::bail!("転送された合成agent鍵を確認できません")
        })
        .await??;
        channel.close().await?;
    }
    let (status, stdout, _) = session.execute("printf 'HASHI_SSH_TEST\\n'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"HASHI_SSH_TEST\n",
        "SSHコマンドの結果が一致しません"
    );
    // 締切を超えたexecの後も、同じSSH Transportを使えることを実サーバーで確認。
    let expired = session
        .execute_with_timeout("sleep 2", std::time::Duration::from_millis(50))
        .await;
    anyhow::ensure!(expired.is_err(), "execの締切が効きません");
    let (status, stdout, _) = session.execute("printf 'STILL_ALIVE'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"STILL_ALIVE",
        "execの締切で接続全体が閉じられました"
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
    verify_forwards(session.clone()).await?;
    session.close().await?;
    println!("ssh-sftp-verified");
    Ok(())
}

async fn echo(socket: &mut tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    socket.write_all(b"PING").await?;
    let mut bytes = [0; 4];
    socket.read_exact(&mut bytes).await?;
    anyhow::ensure!(&bytes == b"PING", "フォワードの内容が一致しません");
    Ok(())
}
async fn verify_forwards(session: Arc<SshSession>) -> Result<()> {
    use hashi_core::forward::{Destination, LocalForward, RemoteForward};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let target = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await?;
            tasks.spawn(async move {
                let mut bytes = [0; 4];
                socket.read_exact(&mut bytes).await?;
                socket.write_all(&bytes).await?;
                let mut byte = [0];
                anyhow::ensure!(
                    socket.read(&mut byte).await? == 0,
                    "転送後の停止を確認できません"
                );
                anyhow::Ok(())
            });
        }
        while let Some(done) = tasks.join_next().await {
            done??;
        }
        anyhow::Ok(())
    });
    let mut local = LocalForward::start(
        session.clone(),
        "127.0.0.1:0",
        Destination::Fixed {
            host: "127.0.0.1".into(),
            port: target.port(),
        },
    )
    .await?;
    let mut socket = TcpStream::connect(local.address).await?;
    echo(&mut socket).await?;
    local.stop().await;
    let mut byte = [0];
    anyhow::ensure!(
        tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut byte)).await??
            == 0,
        "停止後の-Lチャネルが残っています"
    );
    let mut dynamic =
        LocalForward::start(session.clone(), "127.0.0.1:0", Destination::Socks5).await?;
    let mut socket = TcpStream::connect(dynamic.address).await?;
    socket.write_all(&[5, 1, 0]).await?;
    let mut response = [0; 2];
    socket.read_exact(&mut response).await?;
    anyhow::ensure!(response == [5, 0], "SOCKS5方式が違います");
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend_from_slice(b"localhost");
    request.extend_from_slice(&target.port().to_be_bytes());
    socket.write_all(&request).await?;
    let mut response = [0; 10];
    socket.read_exact(&mut response).await?;
    anyhow::ensure!(response[1] == 0, "SOCKS5接続を拒否されました");
    echo(&mut socket).await?;
    dynamic.stop().await;
    let remote = RemoteForward::start(
        session.clone(),
        "127.0.0.1".into(),
        0,
        "127.0.0.1".into(),
        target.port(),
    )
    .await?;
    let mut socket = TcpStream::connect(("127.0.0.1", remote.port as u16)).await?;
    echo(&mut socket).await?;
    remote.stop().await?;
    server.await??;
    let (status, stdout, _) = session.execute("printf 'FORWARDS_STOPPED'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"FORWARDS_STOPPED",
        "フォワード停止がSSH接続を巻き込みました"
    );
    Ok(())
}
