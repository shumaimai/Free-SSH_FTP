//! SSHの-L/-D/-R。各フォワードの停止は、その所有するチャネルだけを閉じる。
use crate::ssh::SshSession;
use anyhow::{bail, Context, Result};
use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub enum Destination {
    Fixed { host: String, port: u16 },
    Socks5,
}

pub struct LocalForward {
    pub address: SocketAddr,
    pub last_error: Arc<Mutex<Option<String>>>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}
impl LocalForward {
    pub async fn start(
        session: Arc<SshSession>,
        bind: &str,
        destination: Destination,
    ) -> Result<Self> {
        if session.cancel.is_cancelled() {
            bail!("SSH接続が終了しています");
        }
        if let Destination::Fixed { host, port } = &destination {
            validate(host, *port)?;
        }
        let listener = TcpListener::bind(bind)
            .await
            .context("フォワードの待受を開始できません")?;
        let address = listener.local_addr()?;
        let cancel = session.cancel.child_token();
        let stopped = cancel.clone();
        let errors = Arc::new(Mutex::new(None));
        let report = errors.clone();
        let task = tokio::spawn(async move {
            let mut tasks: JoinSet<Result<()>> = JoinSet::new();
            loop {
                tokio::select! {
                    _ = stopped.cancelled() => break,
                    done = tasks.join_next(), if !tasks.is_empty() => {
                        if let Some(Ok(Err(error))) = done { if let Ok(mut last) = report.lock() { *last = Some(error.to_string()); } }
                    },
                    accepted = listener.accept(), if tasks.len() < 32 => {
                        let (stream, peer) = match accepted { Ok(value) => value, Err(error) => { if let Ok(mut last) = report.lock() { *last = Some(error.to_string()); } break; } };
                        let session = session.clone(); let destination = destination.clone(); let cancel = stopped.clone();
                        tasks.spawn(async move {
                            tokio::select! {
                                _ = cancel.cancelled() => Ok(()),
                                result = connection(&session, stream, peer, destination, &cancel) => result,
                            }
                        });
                    },
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        Ok(Self {
            address,
            last_error: errors,
            cancel,
            task: Some(task),
        })
    }
    pub async fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for LocalForward {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn validate(host: &str, port: u16) -> Result<()> {
    if host.is_empty() || host.len() > 1024 || host.contains('\0') || port == 0 {
        bail!("フォワードの宛先が不正です");
    }
    Ok(())
}
async fn socks_destination(stream: &mut TcpStream) -> Result<(String, u16)> {
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting[0] != 5 || greeting[1] == 0 {
        bail!("SOCKS5 greetingが不正です");
    }
    let mut methods = vec![0; greeting[1] as usize];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        stream.write_all(&[5, 255]).await?;
        bail!("SOCKS5の認証方式が未対応です");
    }
    stream.write_all(&[5, 0]).await?;
    let mut request = [0; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != 5 || request[1] != 1 || request[2] != 0 {
        stream.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        bail!("SOCKS5はCONNECTだけに対応します");
    }
    let host = match request[3] {
        1 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            Ipv6Addr::from(bytes).to_string()
        }
        3 => {
            let length = stream.read_u8().await? as usize;
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await?;
            String::from_utf8(bytes).context("SOCKS5のホスト名が不正です")?
        }
        _ => {
            stream.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            bail!("SOCKS5のアドレス形式が未対応です");
        }
    };
    let port = stream.read_u16().await?;
    validate(&host, port)?;
    Ok((host, port))
}
async fn connection(
    session: &SshSession,
    mut stream: TcpStream,
    peer: SocketAddr,
    destination: Destination,
    cancel: &CancellationToken,
) -> Result<()> {
    let dynamic = matches!(destination, Destination::Socks5);
    let (host, port) = match destination {
        Destination::Fixed { host, port } => (host, port),
        Destination::Socks5 => {
            tokio::time::timeout(Duration::from_secs(10), socks_destination(&mut stream))
                .await
                .context("SOCKS5の開始がタイムアウトしました")??
        }
    };
    let channel = tokio::time::timeout(
        Duration::from_secs(15),
        session.connection.channel_open_direct_tcpip(
            &host,
            port as u32,
            peer.ip().to_string(),
            peer.port() as u32,
        ),
    )
    .await;
    let channel = match channel {
        Ok(Ok(channel)) => channel,
        result => {
            if dynamic {
                let _ = stream.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            }
            bail!("SSHフォワードの宛先へ接続できません: {:?}", result.err());
        }
    };
    if dynamic {
        stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    }
    let mut channel = channel.into_stream();
    tokio::select! {
        _ = cancel.cancelled() => {},
        result = tokio::io::copy_bidirectional_with_sizes(&mut stream,&mut channel,16384,16384) => {result?;},
    }
    Ok(())
}

pub struct RemoteForward {
    session: Arc<SshSession>,
    pub bind: String,
    pub port: u32,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
    stopped: AtomicBool,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
impl RemoteForward {
    pub async fn start(
        session: Arc<SshSession>,
        bind: String,
        port: u16,
        host: String,
        target_port: u16,
    ) -> Result<Self> {
        validate(&host, target_port)?;
        if bind.len() > 1024 || bind.contains('\0') {
            bail!("リモート待受が不正です");
        }
        let slot = session
            .forward_limits
            .clone()
            .try_acquire_owned()
            .context("リモートフォワードは32本までです")?;
        let assigned = tokio::time::timeout(
            Duration::from_secs(15),
            session.connection.tcpip_forward(bind.clone(), port as u32),
        )
        .await
        .context("リモートフォワード開始がタイムアウトしました")??;
        let actual = if port == 0 { assigned } else { port as u32 };
        let cancel = session.cancel.child_token();
        let registration = (|| {
            if cancel.is_cancelled() {
                bail!("SSH接続が終了しています");
            }
            let mut routes = session
                .remote_routes
                .lock()
                .map_err(|_| anyhow::anyhow!("フォワード台帳が不正です"))?;
            if routes.contains_key(&(bind.clone(), actual)) {
                bail!("同じ待受が登録済みです");
            }
            routes.insert(
                (bind.clone(), actual),
                RemoteRoute {
                    host,
                    port: target_port,
                    cancel: cancel.clone(),
                },
            );
            anyhow::Ok(())
        })();
        if let Err(error) = registration {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                session
                    .connection
                    .cancel_tcpip_forward(bind.clone(), actual),
            )
            .await;
            return Err(error);
        }
        Ok(Self {
            session,
            bind,
            port: actual,
            cancel,
            runtime: tokio::runtime::Handle::current(),
            stopped: AtomicBool::new(false),
            _slot: slot,
        })
    }
    pub async fn stop(&self) -> Result<()> {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.cancel.cancel();
        if let Ok(mut routes) = self.session.remote_routes.lock() {
            routes.remove(&(self.bind.clone(), self.port));
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            self.session
                .connection
                .cancel_tcpip_forward(self.bind.clone(), self.port),
        )
        .await
        .context("リモートフォワード停止がタイムアウトしました")??;
        Ok(())
    }
}
impl Drop for RemoteForward {
    fn drop(&mut self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        self.cancel.cancel();
        let session = self.session.clone();
        let bind = self.bind.clone();
        let port = self.port;
        self.runtime.spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                session.connection.cancel_tcpip_forward(bind, port),
            )
            .await;
        });
        if let Ok(mut routes) = self.session.remote_routes.lock() {
            routes.remove(&(self.bind.clone(), self.port));
        }
    }
}
#[derive(Clone)]
pub(crate) struct RemoteRoute {
    pub host: String,
    pub port: u16,
    pub cancel: CancellationToken,
}
pub(crate) type Routes = Arc<Mutex<std::collections::BTreeMap<(String, u32), RemoteRoute>>>;

pub(crate) async fn accept_remote(
    channel: russh::Channel<russh::client::Msg>,
    reply: russh::client::ChannelOpenHandle,
    route: RemoteRoute,
    slot: tokio::sync::OwnedSemaphorePermit,
) {
    let mut socket = tokio::select! {
        _ = route.cancel.cancelled() => return,
        socket = tokio::time::timeout(Duration::from_secs(10),TcpStream::connect((route.host.as_str(),route.port))) => match socket { Ok(Ok(socket)) => socket, _ => return },
    };
    reply.accept().await;
    let _slot = slot;
    let mut channel = channel.into_stream();
    tokio::select! { _ = route.cancel.cancelled() => {}, _ = tokio::io::copy_bidirectional_with_sizes(&mut socket,&mut channel,16384,16384) => {} }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn socks5_rejects_authentication_and_udp() {
        for udp in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                socks_destination(&mut socket).await
            });
            let mut socket = TcpStream::connect(address).await.unwrap();
            socket
                .write_all(if udp { &[5, 1, 0] } else { &[5, 1, 2] })
                .await
                .unwrap();
            let mut reply = [0; 2];
            socket.read_exact(&mut reply).await.unwrap();
            if udp {
                assert_eq!(reply, [5, 0]);
                socket.write_all(&[5, 3, 0, 1]).await.unwrap();
                let mut reply = [0; 10];
                socket.read_exact(&mut reply).await.unwrap();
                assert_eq!(reply[1], 7);
            } else {
                assert_eq!(reply, [5, 255]);
            }
            assert!(task.await.unwrap().is_err());
        }
    }
    #[test]
    fn targets_are_bounded() {
        assert!(validate("", 22).is_err());
        assert!(validate("localhost", 0).is_err());
    }
}
