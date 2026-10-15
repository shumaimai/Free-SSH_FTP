use crate::interaction::{Challenge, Interaction, Reply, Scope, Stamp};
use crate::ssh_config;
use anyhow::{bail, Context, Result};
use hashi_storage::config::{HostKey, KnownHosts, Profile};
use russh::{
    client,
    keys::{agent::client::AgentClient, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    ChannelMsg, Disconnect,
};
use russh_sftp::client::SftpSession;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct Handler {
    remote_routes: crate::forward::Routes,
    agent_forwarding: bool,
    agent_slots: Arc<tokio::sync::Semaphore>,
    scope: Scope,
    interaction: Interaction,
    known: Arc<Mutex<KnownHosts>>,
    cancel: CancellationToken,
    accepted: Arc<Mutex<KnownHosts>>,
}
impl client::Handler for Handler {
    type Error = anyhow::Error;
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<()> {
        if !self.agent_forwarding || self.cancel.is_cancelled() {
            return Ok(());
        }
        let Ok(permit) = self.agent_slots.clone().try_acquire_owned() else {
            return Ok(());
        };
        let agent = tokio::select! { _ = self.cancel.cancelled() => return Ok(()), result = tokio::time::timeout(Duration::from_secs(10), connect_agent()) => result?? };
        let mut stream = agent.into_inner();
        reply.accept().await;
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut channel = channel.into_stream();
            tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::io::copy_bidirectional(&mut channel, &mut stream) => {} }
        });
        Ok(())
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        address: &str,
        port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<()> {
        let route = self
            .remote_routes
            .lock()
            .map_err(|_| anyhow::anyhow!("フォワード台帳が不正です"))?
            .get(&(address.to_owned(), port))
            .cloned();
        let Some(route) = route else {
            return Ok(());
        };
        if self.cancel.is_cancelled() || route.cancel.is_cancelled() {
            return Ok(());
        }
        let Ok(slot) = self.agent_slots.clone().try_acquire_owned() else {
            return Ok(());
        };
        tokio::spawn(crate::forward::accept_remote(channel, reply, route, slot));
        Ok(())
    }
    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        if matches!(key, PublicKeyOrCertificate::Certificate(_)) {
            bail!("SSHホスト証明書は未対応です");
        }
        let key = HostKey::from_blob(&key.public_key().to_bytes()?)?;
        let old = {
            self.known
                .lock()
                .map_err(|_| anyhow::anyhow!("信頼情報を参照できません"))?
                .get(&format!("{}:{}", self.scope.host, self.scope.port))
                .cloned()
        };
        if old.as_ref() == Some(&key) {
            self.accepted
                .lock()
                .map_err(|_| anyhow::anyhow!("確認済み鍵の台帳が不正です"))?
                .insert(format!("{}:{}", self.scope.host, self.scope.port), key);
            return Ok(true);
        }
        let mut scope = self.scope.clone();
        scope.kind = "host_key".into();
        match self
            .interaction
            .ask(
                scope,
                Challenge::HostKey {
                    key: key.clone(),
                    old: old.clone(),
                },
                &self.cancel,
            )
            .await?
        {
            Reply::HostTrust {
                accept: true,
                confirmation,
            } if old.is_none() || confirmation == "trust" => {
                let host = format!("{}:{}", self.scope.host, self.scope.port);
                self.accepted
                    .lock()
                    .map_err(|_| anyhow::anyhow!("信頼情報を更新できません"))?
                    .insert(host.clone(), key.clone());
                self.known
                    .lock()
                    .map_err(|_| anyhow::anyhow!("信頼情報を更新できません"))?
                    .insert(host, key);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

pub struct SshSession {
    pub background: crate::work::Work,
    pub stamp: Stamp,
    pub scope: Scope,
    pub connection: client::Handle<Handler>,
    hops: Vec<client::Handle<Handler>>,
    pub accepted: Arc<Mutex<KnownHosts>>,
    pub cancel: CancellationToken,
    agent_forwarding: bool,
    pub(crate) remote_routes: crate::forward::Routes,
    pub(crate) forward_limits: Arc<tokio::sync::Semaphore>,
    pub(crate) transfer_commits: Arc<tokio::sync::Semaphore>,
}

struct ConnectingHops {
    connections: Vec<client::Handle<Handler>>,
    cancel: CancellationToken,
    finished: bool,
}
impl Drop for ConnectingHops {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.cancel.cancel();
        let mut connections = std::mem::take(&mut self.connections);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                for connection in connections.iter_mut().rev() {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        connection.disconnect(Disconnect::ByApplication, "Hashi: 接続中止", ""),
                    )
                    .await;
                }
            });
        }
    }
}

#[derive(Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: String,
}
impl From<&Profile> for Target {
    fn from(p: &Profile) -> Self {
        Self {
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
            auth_method: p.auth_method.clone(),
            key_path: p.key_path.clone(),
        }
    }
}

fn parse_hop(text: &str, username: &str) -> Result<Target> {
    let (user, host) = text.rsplit_once('@').unwrap_or((username, text));
    let (host, port) = if let Some(rest) = host.strip_prefix('[') {
        let (address, suffix) = rest.split_once(']').context("踏み台のIPv6形式が不正です")?;
        let port = if suffix.is_empty() {
            22
        } else {
            suffix
                .strip_prefix(':')
                .context("踏み台ポートが不正です")?
                .parse()?
        };
        (address.to_owned(), port)
    } else if host.matches(':').count() == 1 {
        let (h, p) = host.rsplit_once(':').context("踏み台が不正です")?;
        (h.into(), p.parse()?)
    } else {
        (host.into(), 22)
    };
    if host.is_empty() || port == 0 {
        bail!("踏み台の接続先が不正です");
    }
    Ok(Target {
        host,
        port,
        username: user.into(),
        auth_method: "key".into(),
        key_path: String::new(),
    })
}

fn config(scope: &Scope, known: &KnownHosts) -> client::Config {
    let mut preferred = russh::Preferred::default();
    // Python/Paramiko 5のCBC互換を保つ。現代的な暗号を先に交渉し、SHA-1署名/MACは追加しない。
    let mut ciphers = preferred.cipher.to_vec();
    ciphers.extend([
        russh::cipher::AES_128_CBC,
        russh::cipher::AES_192_CBC,
        russh::cipher::AES_256_CBC,
        russh::cipher::TRIPLE_DES_CBC,
    ]);
    preferred.cipher = Cow::Owned(ciphers);
    // Paramiko 5と同様にSHA-1署名へ自動で格下げしない。
    let mut keys = preferred.key.to_vec();
    keys.retain(|algorithm| {
        algorithm.to_string() != "ssh-rsa" && algorithm.to_string() != "ssh-dss"
    });
    if let Some(saved) = known.get(&format!("{}:{}", scope.host, scope.port)) {
        keys.sort_by_key(|algorithm| {
            let name = algorithm.to_string();
            !(name == saved.key_type
                || (saved.key_type == "ssh-rsa" && name.starts_with("rsa-sha2-")))
        });
    }
    preferred.key = Cow::Owned(keys);
    client::Config {
        preferred,
        keepalive_interval: Some(Duration::from_secs(30)),
        inactivity_timeout: None,
        ..Default::default()
    }
}

async fn secret(
    target: &Target,
    scope: &Scope,
    kind: &str,
    interaction: &Interaction,
    cancel: &CancellationToken,
) -> Result<String> {
    let mut scope = scope.clone();
    scope.kind = kind.into();
    let hop = if scope.hop.is_some() {
        "踏み台: "
    } else {
        ""
    };
    match interaction
        .ask(
            scope,
            Challenge::Secret {
                prompt: format!(
                    "{hop}{}@{}:{} の{kind}",
                    target.username, target.host, target.port
                ),
            },
            cancel,
        )
        .await?
    {
        Reply::Secret(secret) => Ok(secret),
        _ => {
            cancel.cancel();
            bail!("認証を取り消しました");
        }
    }
}

async fn authenticate_once(
    connection: &mut client::Handle<Handler>,
    target: &Target,
    scope: &Scope,
    interaction: &Interaction,
    cancel: &CancellationToken,
) -> Result<()> {
    let accepted = match target.auth_method.as_str() {
        "password" => {
            let mut accepted = false;
            for _ in 0..3 {
                let password = secret(target, scope, "password", interaction, cancel).await?;
                if connection
                    .authenticate_password(&target.username, password)
                    .await?
                    .success()
                {
                    accepted = true;
                    break;
                }
            }
            accepted
        }
        "key" => {
            let path = if let Some(rest) = target.key_path.strip_prefix("~/") {
                PathBuf::from(
                    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                        .context("ホームが不明です")?,
                )
                .join(rest)
            } else {
                PathBuf::from(&target.key_path)
            };
            if target.key_path.is_empty() || !path.is_file() {
                bail!("秘密鍵ファイルを指定してください");
            }
            let key = match russh::keys::load_secret_key(&path, None) {
                Ok(key) => key,
                Err(_) => {
                    let passphrase =
                        secret(target, scope, "passphrase", interaction, cancel).await?;
                    russh::keys::load_secret_key(&path, Some(&passphrase))
                        .context("秘密鍵を復号できません")?
                }
            };
            let hash = connection
                .best_supported_rsa_hash()
                .await?
                .flatten()
                .or(Some(russh::keys::HashAlg::Sha512));
            connection
                .authenticate_publickey(
                    &target.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await?
                .success()
        }
        "agent" => {
            let mut agent = connect_agent().await?;
            let hash = connection
                .best_supported_rsa_hash()
                .await?
                .flatten()
                .or(Some(russh::keys::HashAlg::Sha512));
            let mut accepted = false;
            for key in agent.request_identities().await? {
                if connection
                    .authenticate_publickey_with(
                        &target.username,
                        key.public_key().into_owned(),
                        hash,
                        &mut agent,
                    )
                    .await?
                    .success()
                {
                    accepted = true;
                    break;
                }
            }
            accepted
        }
        _ => bail!("未対応の認証方式です"),
    };
    if !accepted {
        bail!("SSH認証が拒否されました");
    }
    Ok(())
}

async fn authenticate(
    connection: &mut client::Handle<Handler>,
    target: &Target,
    scope: &Scope,
    interaction: &Interaction,
    cancel: &CancellationToken,
) -> Result<()> {
    if scope.hop.is_none() {
        return authenticate_once(connection, target, scope, interaction, cancel).await;
    }
    // 踏み台の設定鍵→agent→専用のpassword。接続先の秘密を使い回さない。
    if !target.key_path.is_empty() {
        let mut key = target.clone();
        key.auth_method = "key".into();
        if authenticate_once(connection, &key, scope, interaction, cancel)
            .await
            .is_ok()
        {
            return Ok(());
        }
        if cancel.is_cancelled() {
            bail!("認証を取り消しました");
        }
    }
    let mut agent = target.clone();
    agent.auth_method = "agent".into();
    if authenticate_once(connection, &agent, scope, interaction, cancel)
        .await
        .is_ok()
    {
        return Ok(());
    }
    if cancel.is_cancelled() {
        bail!("認証を取り消しました");
    }
    let mut password = target.clone();
    password.auth_method = "password".into();
    authenticate_once(connection, &password, scope, interaction, cancel).await
}

pub async fn connect_agent(
) -> Result<AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>> {
    #[cfg(unix)]
    {
        Ok(AgentClient::connect(
            AgentClient::connect_env().await?.into_inner(),
        ))
    }
    #[cfg(windows)]
    {
        let stream = match tokio::time::timeout(
            Duration::from_secs(5),
            AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent"),
        )
        .await
        {
            Ok(Ok(agent)) => agent.into_inner(),
            _ => AgentClient::connect_pageant().await?.into_inner(),
        };
        Ok(AgentClient::connect(stream))
    }
}

impl SshSession {
    pub async fn connect(
        profile: &Profile,
        stamp: Stamp,
        known: KnownHosts,
        interaction: Interaction,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let path = ssh_config::default_path();
        Self::connect_with_config(profile, stamp, known, interaction, cancel, path.as_deref()).await
    }
    pub async fn connect_with_config(
        profile: &Profile,
        stamp: Stamp,
        known: KnownHosts,
        interaction: Interaction,
        cancel: CancellationToken,
        path: Option<&std::path::Path>,
    ) -> Result<Self> {
        let profile = match path {
            Some(path) => ssh_config::resolve_at(profile, path)?,
            None => profile.clone(),
        };
        profile.validate()?;
        if profile.host.is_empty() || profile.username.is_empty() {
            bail!("ホストと利用者名を指定してください");
        }
        let known = Arc::new(Mutex::new(known));
        let accepted = Arc::new(Mutex::new(BTreeMap::new()));
        let remote_routes = Arc::new(Mutex::new(BTreeMap::new()));
        let mut targets = vec![];
        if !profile.proxy_jump.trim().eq_ignore_ascii_case("none") {
            for hop in profile
                .proxy_jump
                .split(',')
                .filter(|s| !s.trim().is_empty())
            {
                let target = parse_hop(hop.trim(), "")?;
                let hop = Profile {
                    host: target.host,
                    port: target.port,
                    username: target.username,
                    save_secrets: false,
                    ..Default::default()
                };
                let hop = match path {
                    Some(path) => ssh_config::resolve_at(&hop, path)?,
                    None => hop,
                };
                if !hop.proxy_jump.is_empty() && !hop.proxy_jump.eq_ignore_ascii_case("none") {
                    bail!("踏み台自身のProxyJumpは未対応です。カンマ区切りへ平坦化してください");
                }
                let mut target = Target::from(&hop);
                if target.username.is_empty() {
                    target.username = profile.username.clone();
                }
                targets.push(target);
                if targets.len() > 8 {
                    bail!("ProxyJumpは8段までです");
                }
            }
        }
        targets.push(Target::from(&profile));
        let last = targets.len() - 1;
        let mut hops = ConnectingHops {
            connections: vec![],
            cancel: cancel.clone(),
            finished: false,
        };
        for (index, target) in targets.iter().enumerate() {
            let scope = Scope {
                stamp: stamp.clone(),
                host: target.host.clone(),
                port: target.port,
                username: target.username.clone(),
                hop: if index == last {
                    None
                } else {
                    Some(index as u16)
                },
                kind: "password".into(),
            };
            let config = {
                let known_guard = known
                    .lock()
                    .map_err(|_| anyhow::anyhow!("信頼情報が不正です"))?;
                Arc::new(config(&scope, &known_guard))
            };
            let handler = Handler {
                remote_routes: if index == last {
                    remote_routes.clone()
                } else {
                    Arc::new(Mutex::new(BTreeMap::new()))
                },
                agent_slots: Arc::new(tokio::sync::Semaphore::new(32)),
                agent_forwarding: index == last && profile.agent_forwarding,
                scope: scope.clone(),
                interaction: interaction.clone(),
                known: known.clone(),
                cancel: cancel.clone(),
                accepted: accepted.clone(),
            };
            let future = async {
                let mut connection = if let Some(previous) = hops.connections.last() {
                    let channel = previous
                        .channel_open_direct_tcpip(&target.host, target.port as u32, "127.0.0.1", 0)
                        .await?;
                    client::connect_stream(config, channel.into_stream(), handler).await?
                } else {
                    client::connect(config, (&*target.host, target.port), handler).await?
                };
                if let Err(error) =
                    authenticate(&mut connection, target, &scope, &interaction, &cancel).await
                {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        connection.disconnect(Disconnect::ByApplication, "Hashi: 認証中止", ""),
                    )
                    .await;
                    return Err(error);
                }
                anyhow::Ok(connection)
            };
            let connection = tokio::select! { _=cancel.cancelled()=>bail!("接続を取り消しました"),result=tokio::time::timeout(Duration::from_secs(180),future)=>result.context("接続がタイムアウトしました")?? };
            if index == last {
                hops.finished = true;
                return Ok(Self {
                    background: crate::work::Work::default(),
                    stamp,
                    scope,
                    connection,
                    hops: std::mem::take(&mut hops.connections),
                    accepted,
                    cancel,
                    agent_forwarding: profile.agent_forwarding,
                    remote_routes,
                    forward_limits: Arc::new(tokio::sync::Semaphore::new(32)),
                    transfer_commits: Arc::new(tokio::sync::Semaphore::new(2)),
                });
            }
            hops.connections.push(connection);
        }
        bail!("接続先がありません")
    }

    pub async fn raw_sftp(&self) -> Result<Arc<russh_sftp::client::RawSftpSession>> {
        let channel = self.connection.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let session = Arc::new(russh_sftp::client::RawSftpSession::new(
            channel.into_stream(),
        ));
        session.init().await?;
        Ok(session)
    }
    pub async fn sftp(&self) -> Result<SftpSession> {
        let channel = self.connection.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        Ok(SftpSession::new_with_config(
            channel.into_stream(),
            russh_sftp::client::Config {
                max_concurrent_reads: 1,
                max_concurrent_writes: 1,
                ..Default::default()
            },
        )
        .await?)
    }
    pub async fn shell(&self, cols: u32, rows: u32) -> Result<russh::Channel<client::Msg>> {
        let channel = self.connection.channel_open_session().await?;
        channel
            .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
            .await?;
        if self.agent_forwarding {
            channel.agent_forward(true).await?;
        }
        channel.request_shell(true).await?;
        Ok(channel)
    }
    pub async fn execute(&self, command: &str) -> Result<(u32, Vec<u8>, Vec<u8>)> {
        self.execute_with_timeout(command, Duration::from_secs(120))
            .await
    }
    pub async fn execute_with_timeout(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>, Vec<u8>)> {
        if command.len() > 256 * 1024
            || command.contains('\0')
            || timeout.is_zero()
            || timeout > Duration::from_secs(3600)
        {
            bail!("コマンドの長さ/形式/締切が不正です");
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let mut channel = tokio::select! {
            _ = self.cancel.cancelled() => bail!("実行を取り消しました"),
            channel = tokio::time::timeout_at(deadline, self.connection.channel_open_session()) => channel.context("コマンド開始がタイムアウトしました")??,
        };
        let result = tokio::select! {
            _ = self.cancel.cancelled() => Err(anyhow::anyhow!("実行を取り消しました")),
            result = tokio::time::timeout_at(deadline, async {
                channel.exec(true, command).await?;
                let mut stdout = vec![]; let mut stderr = vec![]; let mut code = None;
                while let Some(message) = channel.wait().await {
                    match message {
                        ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                        ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                        ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                        ChannelMsg::Close => break,
                        _ => {},
                    }
                    if stdout.len() + stderr.len() > 8 * 1024 * 1024 { bail!("実行出力が上限を超えました"); }
                }
                anyhow::Ok((code.context("終了状態が届きませんでした")?, stdout, stderr))
            }) => match result { Ok(result) => result, Err(_) => Err(anyhow::anyhow!("コマンドがタイムアウトしました（遠隔プロセスの終了は未確認）")) },
        };
        // 独立execの専用チャネルだけを閉じ、対話PTY/SFTP/Transportは維持する。
        let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
        result
    }
    pub async fn close(&self) -> Result<()> {
        self.cancel.cancel();
        let result = self
            .connection
            .disconnect(Disconnect::ByApplication, "Hashi: 終了", "")
            .await;
        for hop in self.hops.iter().rev() {
            let _ = hop
                .disconnect(Disconnect::ByApplication, "Hashi: 終了", "")
                .await;
        }
        result.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hops_include_ipv6_and_do_not_reuse_target_secrets() {
        let hop = parse_hop("user@[::1]:2222", "target").unwrap();
        assert_eq!(
            (hop.host.as_str(), hop.username.as_str(), hop.port),
            ("::1", "user", 2222)
        );
        assert_eq!(hop.auth_method, "key");
        assert!(hop.key_path.is_empty());
        assert!(parse_hop("host:0", "target").is_err());
    }
}
