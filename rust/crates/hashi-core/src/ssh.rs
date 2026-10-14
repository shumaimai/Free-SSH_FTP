use crate::interaction::{Challenge, Interaction, Reply, Scope, Stamp};
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
    scope: Scope,
    interaction: Interaction,
    known: Arc<Mutex<KnownHosts>>,
    cancel: CancellationToken,
    accepted: Arc<Mutex<KnownHosts>>,
}
impl client::Handler for Handler {
    type Error = anyhow::Error;
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
                self.accepted
                    .lock()
                    .map_err(|_| anyhow::anyhow!("信頼情報を更新できません"))?
                    .insert(format!("{}:{}", self.scope.host, self.scope.port), key);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

pub struct SshSession {
    pub stamp: Stamp,
    pub scope: Scope,
    pub connection: client::Handle<Handler>,
    hops: Vec<client::Handle<Handler>>,
    pub accepted: Arc<Mutex<KnownHosts>>,
    pub cancel: CancellationToken,
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
    if host.is_empty() || user.is_empty() || port == 0 {
        bail!("踏み台の接続先が不正です");
    }
    Ok(Target {
        host,
        port,
        username: user.into(),
        auth_method: "agent".into(),
        key_path: String::new(),
    })
}

fn config(scope: &Scope, known: &KnownHosts) -> client::Config {
    let mut preferred = russh::Preferred::default();
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
        _ => bail!("認証を取り消しました"),
    }
}

async fn authenticate(
    connection: &mut client::Handle<Handler>,
    target: &Target,
    scope: &Scope,
    interaction: &Interaction,
    cancel: &CancellationToken,
) -> Result<()> {
    let accepted = match target.auth_method.as_str() {
        "password" => {
            let password = secret(target, scope, "password", interaction, cancel).await?;
            connection
                .authenticate_password(&target.username, password)
                .await?
                .success()
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
        profile.validate()?;
        if profile.host.is_empty() || profile.username.is_empty() {
            bail!("ホストと利用者名を指定してください");
        }
        let known = Arc::new(Mutex::new(known));
        let accepted = Arc::new(Mutex::new(BTreeMap::new()));
        let mut targets = vec![];
        for hop in profile
            .proxy_jump
            .split(',')
            .filter(|s| !s.trim().is_empty())
        {
            targets.push(parse_hop(hop.trim(), &profile.username)?);
        }
        targets.push(Target::from(profile));
        let last = targets.len() - 1;
        let mut hops: Vec<client::Handle<Handler>> = vec![];
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
                scope: scope.clone(),
                interaction: interaction.clone(),
                known: known.clone(),
                cancel: cancel.clone(),
                accepted: accepted.clone(),
            };
            let future = async {
                let mut connection = if let Some(previous) = hops.last() {
                    let channel = previous
                        .channel_open_direct_tcpip(&target.host, target.port as u32, "127.0.0.1", 0)
                        .await?;
                    client::connect_stream(config, channel.into_stream(), handler).await?
                } else {
                    client::connect(config, (&*target.host, target.port), handler).await?
                };
                authenticate(&mut connection, target, &scope, &interaction, &cancel).await?;
                anyhow::Ok(connection)
            };
            let connection = tokio::select! { _=cancel.cancelled()=>bail!("接続を取り消しました"),result=tokio::time::timeout(Duration::from_secs(180),future)=>result.context("接続がタイムアウトしました")?? };
            if index == last {
                return Ok(Self {
                    stamp,
                    scope,
                    connection,
                    hops,
                    accepted,
                    cancel,
                });
            }
            hops.push(connection);
        }
        bail!("接続先がありません")
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
        channel.request_shell(true).await?;
        Ok(channel)
    }
    pub async fn execute(&self, command: &str) -> Result<(u32, Vec<u8>, Vec<u8>)> {
        if command.len() > 256 * 1024 || command.contains('\0') {
            bail!("コマンドの長さ/形式が不正です");
        }
        let mut channel = self.connection.channel_open_session().await?;
        channel.exec(true, command).await?;
        let mut stdout = vec![];
        let mut stderr = vec![];
        let mut code = None;
        loop {
            tokio::select! {
                _=self.cancel.cancelled()=> { let _=channel.close().await;bail!("実行を取り消しました"); },
                message=channel.wait()=>match message {
                    Some(ChannelMsg::Data { data })=>stdout.extend_from_slice(&data),
                    Some(ChannelMsg::ExtendedData { data,.. })=>stderr.extend_from_slice(&data),
                    Some(ChannelMsg::ExitStatus { exit_status })=>code=Some(exit_status),
                    Some(ChannelMsg::Close)|None=>break,
                    _=>{},
                },
            }
            if stdout.len() + stderr.len() > 8 * 1024 * 1024 {
                let _ = channel.close().await;
                bail!("実行出力が上限を超えました");
            }
        }
        Ok((code.context("終了状態が届きませんでした")?, stdout, stderr))
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
        assert_eq!(hop.auth_method, "agent");
        assert!(hop.key_path.is_empty());
        assert!(parse_hop("host:0", "target").is_err());
    }
}
