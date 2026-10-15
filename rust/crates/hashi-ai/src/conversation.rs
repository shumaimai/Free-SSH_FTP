//! AI会話とツール呼出しの往復（Python版`AiConversation`相当）。会話と操作許可は保存しない。
use crate::{api::ApiProvider, observe::observation_json, tools::TerminalTools, INSTRUCTIONS};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// 応答の生成を担うもの。テストでは偽の応答を返す。
pub trait Provider: Send + Sync {
    fn respond(
        &self,
        messages: &[Value],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<crate::api::Reply>;
}
impl Provider for ApiProvider {
    fn respond(
        &self,
        messages: &[Value],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<crate::api::Reply> {
        ApiProvider::respond(self, messages, instructions, tools, cancel, on_text)
    }
}

pub struct Conversation {
    pub provider: Arc<dyn Provider>,
    pub tools: TerminalTools,
    pub messages: Vec<Value>,
}
impl Conversation {
    /// 質問と観測データを送り、ツール要求があれば実行して最大8往復まで続ける。
    /// `on_text`へ途中の文字列を渡す。取消は各段階の境目で確認する。
    pub async fn ask(
        &mut self,
        question: &str,
        context: &str,
        cancel: &CancellationToken,
        on_text: mpsc::Sender<String>,
    ) -> Result<String> {
        let content = if context.is_empty() {
            question.to_owned()
        } else {
            format!("{question}\n\n観測データ:\n{context}")
        };
        self.messages
            .push(json!({"role": "user", "content": content}));
        let stopped = || anyhow::anyhow!("AI処理を停止しました");
        for _ in 0..8 {
            if cancel.is_cancelled() {
                bail!(stopped());
            }
            if serde_json::to_string(&self.messages)?.chars().count() > 256000 {
                bail!("会話が長くなりました。会話を消去して続けてください");
            }
            let (provider, messages, token, sender) = (
                self.provider.clone(),
                self.messages.clone(),
                cancel.clone(),
                on_text.clone(),
            );
            let tools = crate::tools::definitions();
            let reply = tokio::task::spawn_blocking(move || {
                provider.respond(&messages, INSTRUCTIONS, &tools, &token, &mut |text| {
                    let _ = sender.blocking_send(text.to_owned());
                })
            })
            .await
            .map_err(|_| anyhow::anyhow!("AI処理が中断されました"))??;
            if cancel.is_cancelled() {
                bail!(stopped());
            }
            let mut message =
                json!({"role": "assistant", "content": reply.text, "tool_calls": reply.tool_calls});
            if let Some(raw) = &reply.raw_output {
                message["raw_output"] = raw.clone();
            }
            self.messages.push(message);
            if reply.tool_calls.is_empty() {
                return Ok(reply.text);
            }
            for call in &reply.tool_calls {
                if cancel.is_cancelled() {
                    bail!(stopped());
                }
                let id = call["id"].as_str().unwrap_or_default().to_owned();
                let name = call["name"].as_str().unwrap_or_default();
                let result = match self
                    .tools
                    .call(name, &call["arguments"], Some(id.clone()))
                    .await
                {
                    Ok(result) => observation_json(&result, 32000),
                    Err(error) => observation_json(&json!({"error": error.to_string()}), 32000),
                };
                self.messages.push(
                    json!({"role": "tool", "tool_call_id": id, "name": name, "content": result}),
                );
            }
        }
        bail!("AI操作の回数上限です。状況を確認して続けてください")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Reply;
    use hashi_core::broker::{Broker, Kind, Mode, Registration, Registry};
    use std::{sync::Mutex, time::Duration};

    struct Scripted(Mutex<Vec<Reply>>);
    impl Provider for Scripted {
        fn respond(
            &self,
            messages: &[Value],
            _: &str,
            tools: &[Value],
            _: &CancellationToken,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<Reply> {
            assert_eq!(tools.len(), 6);
            let reply = self.0.lock().unwrap().remove(0);
            if messages.last().unwrap()["role"] == "tool" {
                assert!(messages.last().unwrap()["content"]
                    .as_str()
                    .unwrap()
                    .contains("sent"));
            }
            on_text(&reply.text);
            Ok(reply)
        }
    }

    #[tokio::test]
    async fn tool_calls_go_through_broker_and_results_are_observations() {
        let registry = Arc::new(Registry::default());
        let (input, mut received) = mpsc::channel(4);
        let (id, generation) = registry.register(
            None,
            Registration {
                label: "SSH".into(),
                kind: Kind::Ssh,
                shell: "unknown".into(),
                cwd: None,
                shareable: true,
                input,
                ssh: None,
            },
        );
        let broker = Arc::new(Broker::new(registry, None));
        broker
            .configure(
                Mode::Auto,
                &[(id.clone(), generation)],
                Duration::from_secs(60),
            )
            .unwrap();
        let provider = Scripted(Mutex::new(vec![
            Reply {
                text: String::new(),
                tool_calls: vec![json!({"id": "t1", "name": "send_input",
                "arguments": {"session_id": id, "generation": generation, "text": "uptime\n"}})],
                raw_output: None,
            },
            Reply {
                text: "送信しました".into(),
                tool_calls: vec![],
                raw_output: None,
            },
        ]));
        let mut conversation = Conversation {
            provider: Arc::new(provider),
            tools: TerminalTools {
                broker,
                actor: "ai:test".into(),
                cancel: CancellationToken::new(),
            },
            messages: vec![],
        };
        let (tx, mut deltas) = mpsc::channel(16);
        let answer = conversation
            .ask("稼働時間は?", "", &CancellationToken::new(), tx)
            .await
            .unwrap();
        assert_eq!(answer, "送信しました");
        assert_eq!(received.recv().await.unwrap(), b"uptime\r");
        assert_eq!(conversation.messages.len(), 4);
        assert_eq!(conversation.messages[2]["role"], "tool");
        let mut streamed = String::new();
        while let Ok(text) = deltas.try_recv() {
            streamed.push_str(&text);
        }
        assert_eq!(streamed, "送信しました");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (tx, _rx) = mpsc::channel(1);
        assert!(conversation.ask("x", "", &cancel, tx).await.is_err());
    }
}
