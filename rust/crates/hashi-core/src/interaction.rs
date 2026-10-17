use anyhow::{bail, Context, Result};
use hashi_storage::config::HostKey;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Stamp {
    pub terminal_id: String,
    pub generation: String,
}
impl Default for Stamp {
    fn default() -> Self {
        Self {
            terminal_id: uuid::Uuid::new_v4().to_string(),
            generation: uuid::Uuid::new_v4().to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Scope {
    pub stamp: Stamp,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub hop: Option<u16>,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Challenge {
    HostKey { key: HostKey, old: Option<HostKey> },
    Secret { prompt: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub challenge_id: String,
    pub scope: Scope,
    pub challenge: Challenge,
}

// 秘密をDebugや全ウィンドウ向けイベントへ載せない。
pub enum Reply {
    HostTrust { accept: bool, confirmation: String },
    Secret(String),
    Cancel,
}
struct Pending {
    scope: Scope,
    tx: oneshot::Sender<Reply>,
    cancel: CancellationToken,
    expires: Instant,
}

#[derive(Clone)]
pub struct Interaction {
    pub events: mpsc::Sender<Request>,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}
impl Interaction {
    pub fn new() -> (Self, mpsc::Receiver<Request>) {
        let (events, rx) = mpsc::channel(32);
        (
            Self {
                events,
                pending: Arc::new(Mutex::new(HashMap::new())),
            },
            rx,
        )
    }
    pub async fn ask(
        &self,
        scope: Scope,
        challenge: Challenge,
        cancel: &CancellationToken,
    ) -> Result<Reply> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("認証要求の台帳が不正です"))?;
            if pending.len() >= 32 {
                bail!("認証要求が多すぎます");
            }
            pending.insert(
                id.clone(),
                Pending {
                    scope: scope.clone(),
                    tx,
                    cancel: cancel.clone(),
                    expires: Instant::now() + Duration::from_secs(180),
                },
            );
        }
        struct Cleanup {
            id: String,
            pending: Arc<Mutex<HashMap<String, Pending>>>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&self.id);
                }
            }
        }
        let _cleanup = Cleanup {
            id: id.clone(),
            pending: self.pending.clone(),
        };
        tokio::select! {
            _=cancel.cancelled()=>bail!("認証を取り消しました"),
            result=tokio::time::timeout(Duration::from_secs(180),async {
                self.events.send(Request { challenge_id:id,scope,challenge }).await.context("認証画面が終了しました")?;
                rx.await.context("認証画面が終了しました")
            })=> result.context("認証要求がタイムアウトしました")?,
        }
    }
    pub fn is_pending(&self, id: &str, scope: &Scope) -> bool {
        self.pending.lock().is_ok_and(|p| {
            p.get(id).is_some_and(|item| {
                &item.scope == scope
                    && !item.tx.is_closed()
                    && !item.cancel.is_cancelled()
                    && Instant::now() < item.expires
            })
        })
    }
    pub fn reply(&self, id: &str, scope: &Scope, answer: Reply) -> Result<()> {
        self.reply_with(id, scope, answer, || Ok(()))
    }
    // 有効な認証要求を台帳ロックの中で確認してから、信頼情報の保存を行う。
    pub fn reply_with(
        &self,
        id: &str,
        scope: &Scope,
        answer: Reply,
        before_send: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("認証台帳が不正です"))?;
        let expected = pending.get(id).context("認証要求が終了しています")?;
        if expected.tx.is_closed()
            || expected.cancel.is_cancelled()
            || Instant::now() >= expected.expires
        {
            bail!("認証要求が終了しています");
        }
        if &expected.scope != scope {
            bail!("認証先または接続世代が一致しません");
        }
        let answer_kind = match &answer {
            Reply::Secret(_) => "secret",
            Reply::HostTrust { .. } => "host_key",
            Reply::Cancel => "cancel",
        };
        if answer_kind != "cancel" && ((scope.kind == "host_key") != (answer_kind == "host_key")) {
            bail!("認証応答の種類が違います");
        }
        before_send()?;
        pending
            .remove(id)
            .context("認証要求が終了しています")?
            .tx
            .send(answer)
            .map_err(|_| anyhow::anyhow!("接続が終了しています"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_challenge_cannot_persist_trust() {
        let (interaction, mut rx) = Interaction::new();
        let scope = Scope {
            stamp: Stamp::default(),
            host: "test.invalid".into(),
            port: 22,
            username: "test".into(),
            hop: None,
            kind: "password".into(),
        };
        let cancel = CancellationToken::new();
        let worker = interaction.clone();
        let token = cancel.clone();
        let target = scope.clone();
        let task = tokio::spawn(async move {
            worker
                .ask(
                    target,
                    Challenge::Secret {
                        prompt: "試験".into(),
                    },
                    &token,
                )
                .await
        });
        let request = rx.recv().await.unwrap();
        cancel.cancel();
        let mut saved = false;
        assert!(interaction
            .reply_with(&request.challenge_id, &scope, Reply::Cancel, || {
                saved = true;
                Ok(())
            })
            .is_err());
        assert!(!saved);
        assert!(task.await.unwrap().is_err());
        assert!(!interaction.is_pending(&request.challenge_id, &scope));
    }
    #[tokio::test]
    async fn stale_or_wrong_hop_reply_does_not_consume_challenge() {
        let (interaction, mut rx) = Interaction::new();
        let scope = Scope {
            stamp: Stamp::default(),
            host: "test.invalid".into(),
            port: 22,
            username: "test".into(),
            hop: Some(1),
            kind: "password".into(),
        };
        let worker = interaction.clone();
        let expected = scope.clone();
        let task = tokio::spawn(async move {
            worker
                .ask(
                    expected,
                    Challenge::Secret {
                        prompt: "試験".into(),
                    },
                    &CancellationToken::new(),
                )
                .await
        });
        let request = rx.recv().await.unwrap();
        let mut wrong = scope.clone();
        wrong.hop = None;
        assert!(interaction
            .reply(
                &request.challenge_id,
                &wrong,
                Reply::Secret("secret".into())
            )
            .is_err());
        interaction
            .reply(&request.challenge_id, &scope, Reply::Secret("ok".into()))
            .unwrap();
        assert!(matches!(task.await.unwrap().unwrap(),Reply::Secret(s) if s=="ok"));
        assert!(interaction
            .reply(&request.challenge_id, &scope, Reply::Cancel)
            .is_err());
    }
}
