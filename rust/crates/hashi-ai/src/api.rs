//! APIキー方式のResponses / Anthropic Messages / 互換Chat Completions（Python版`ai_api.py`相当）。
//! 完了イベントのない応答のツールは実行しない。相談専用ではtoolsを送らず、返されたツール要求も実行しない。
use crate::http;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

pub const COMMANDCODE_BASE: &str = "https://api.commandcode.ai/provider/v1";
pub const KINDS: [(&str, &str); 4] = [
    ("openai", "OpenAI"),
    ("anthropic", "Anthropic"),
    ("compatible", "その他の互換API"),
    ("commandcode", "Command Code"),
];

pub fn default_base(kind: &str) -> &'static str {
    match kind {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        "commandcode" => COMMANDCODE_BASE,
        _ => "http://127.0.0.1:1234/v1",
    }
}

/// 会話の1件。Python版と同じ形（role/content/tool_calls/tool_call_id/name/raw_output）のJSON。
pub type Message = Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    pub tool_calls: Vec<Value>,
    /// Responsesのreasoning等を含む出力。次の要求へそのまま返す。
    pub raw_output: Option<Value>,
}

/// Python版`responses_input`。
pub fn responses_input(messages: &[Message]) -> Vec<Value> {
    let mut items = vec![];
    for message in messages {
        if message["role"] == "tool" {
            items.push(
                json!({"type": "function_call_output", "call_id": message["tool_call_id"],
                              "output": message["content"]}),
            );
        } else if let Some(Value::Array(raw)) = message.get("raw_output") {
            items.extend(raw.iter().cloned());
        } else {
            if message["content"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
            {
                items.push(json!({"role": message["role"], "content": message["content"]}));
            }
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                items.push(
                    json!({"type": "function_call", "call_id": call["id"], "name": call["name"],
                                  "arguments": call["arguments"].to_string()}),
                );
            }
        }
    }
    items
}

fn parse_arguments(text: &str) -> Result<Value> {
    serde_json::from_str(if text.is_empty() { "{}" } else { text })
        .map_err(|_| anyhow::anyhow!("AIのツール引数がJSONではありません。実行していません"))
}

#[derive(Clone)]
pub struct ApiProvider {
    pub kind: String,
    pub model: String,
    key: String,
    pub base: String,
    pub use_tools: bool,
    pub protocol: String,
    /// 通信方式を「自動」で作ったか。モデルが決まった後に、そのモデルに合わせて選び直すために持つ。
    pub auto_protocol: bool,
}
impl ApiProvider {
    pub fn new(
        kind: &str,
        model: &str,
        key: &str,
        base: Option<&str>,
        use_tools: bool,
        protocol: &str,
    ) -> Result<Self> {
        if !KINDS.iter().any(|(k, _)| *k == kind) || model.trim().is_empty() {
            bail!("プロバイダーとモデルを指定してください");
        }
        let base = http::normalize_api_base(base.unwrap_or(default_base(kind)))?;
        if matches!(kind, "openai" | "anthropic") && base != default_base(kind) {
            bail!("公式API方式は公式接続先を使います。独自URLは互換方式を選択してください");
        }
        if !matches!(protocol, "auto" | "chat" | "responses" | "messages") {
            bail!("APIの形式を選んでください");
        }
        let model = model.trim().to_owned();
        let auto_protocol = protocol == "auto";
        let protocol = if protocol == "auto" {
            if kind == "openai" {
                "responses"
            } else if kind == "anthropic"
                || (base == COMMANDCODE_BASE && model.to_lowercase().contains("claude"))
            {
                "messages"
            } else {
                "chat"
            }
        } else {
            protocol
        };
        Ok(Self {
            kind: kind.into(),
            model,
            key: key.to_owned(),
            base,
            use_tools,
            protocol: protocol.into(),
            auto_protocol,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    fn headers(&self) -> Vec<(String, String)> {
        if self.kind == "anthropic" {
            vec![
                ("x-api-key".into(), self.key.clone()),
                ("anthropic-version".into(), "2023-06-01".into()),
            ]
        } else if self.key.is_empty() {
            vec![]
        } else {
            vec![("Authorization".into(), format!("Bearer {}", self.key))]
        }
    }
    /// モデル一覧。ネットワーク待ちがあるのでGUIスレッドから直接呼ばない。
    pub fn models(&self, cancel: &CancellationToken) -> Result<Vec<Value>> {
        if cancel.is_cancelled() {
            bail!("接続確認を停止しました");
        }
        let data = http::get_json(&format!("{}/models", self.base), &self.headers())?;
        if cancel.is_cancelled() {
            bail!("接続確認を停止しました");
        }
        Ok(data["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|model| model["id"].is_string())
            .cloned()
            .collect())
    }
    pub fn respond(
        &self,
        messages: &[Message],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Reply> {
        let enabled: &[Value] = if self.use_tools { tools } else { &[] };
        let reply = match self.protocol.as_str() {
            "responses" => self.responses(messages, instructions, enabled, cancel, on_text),
            "messages" => self.anthropic(messages, instructions, enabled, cancel, on_text),
            _ => self.chat(messages, instructions, enabled, cancel, on_text),
        }?;
        if !self.use_tools && !reply.tool_calls.is_empty() {
            bail!("相談専用の接続がツールを要求しました。実行していません");
        }
        Ok(reply)
    }
    pub fn responses_payload(
        &self,
        messages: &[Message],
        instructions: &str,
        tools: &[Value],
    ) -> Value {
        let mut payload = json!({"model": self.model, "store": false, "stream": true,
            "include": ["reasoning.encrypted_content"], "instructions": instructions,
            "input": responses_input(messages)});
        if self.use_tools {
            payload["tools"] = tools
                .iter()
                .map(|tool| {
                    let mut item = tool.clone();
                    item["type"] = json!("function");
                    item["strict"] = json!(true);
                    item
                })
                .collect();
        }
        payload
    }
    fn responses(
        &self,
        messages: &[Message],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Reply> {
        let (mut completed, mut text) = (None, String::new());
        http::events(
            &format!("{}/responses", self.base),
            &self.responses_payload(messages, instructions, tools),
            &self.headers(),
            cancel,
            |event| {
                match event["type"].as_str().unwrap_or_default() {
                    "response.output_text.delta" => {
                        let delta = event["delta"].as_str().unwrap_or_default();
                        text.push_str(delta);
                        on_text(delta);
                    }
                    "response.completed" => completed = Some(event["response"].clone()),
                    "error" | "response.failed" | "response.incomplete" => {
                        bail!("AI応答が完了しませんでした。利用上限と接続設定を確認してください")
                    }
                    _ => {}
                }
                Ok(())
            },
        )?;
        let Some(completed) = completed else {
            bail!("AI応答が途中で切れました。ツールは実行していません");
        };
        let output = completed["output"].as_array().cloned().unwrap_or_default();
        let mut calls = vec![];
        for item in output.iter().filter(|item| item["type"] == "function_call") {
            calls.push(json!({"id": item["call_id"], "name": item["name"],
                "arguments": parse_arguments(item["arguments"].as_str().unwrap_or_default())?}));
        }
        if text.is_empty() {
            for item in output.iter().filter(|item| item["type"] == "message") {
                for block in item["content"].as_array().into_iter().flatten() {
                    if block["type"] == "output_text" {
                        text.push_str(block["text"].as_str().unwrap_or_default());
                    }
                }
            }
        }
        Ok(Reply {
            text,
            tool_calls: calls,
            raw_output: Some(Value::Array(output)),
        })
    }
    pub fn anthropic_history(messages: &[Message]) -> Vec<Value> {
        let mut history: Vec<Value> = vec![];
        for message in messages {
            let (role, blocks) = if message["role"] == "tool" {
                (
                    "user".to_owned(),
                    vec![
                        json!({"type": "tool_result", "tool_use_id": message["tool_call_id"],
                                               "content": message["content"]}),
                    ],
                )
            } else {
                let mut blocks = vec![];
                if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    blocks.push(json!({"type": "tool_use", "id": call["id"], "name": call["name"], "input": call["arguments"]}));
                }
                (
                    message["role"].as_str().unwrap_or("user").to_owned(),
                    blocks,
                )
            };
            match history.last_mut() {
                Some(last) if last["role"] == role.as_str() => {
                    last["content"].as_array_mut().unwrap().extend(blocks);
                }
                _ if !blocks.is_empty() => history.push(json!({"role": role, "content": blocks})),
                _ => {}
            }
        }
        history
    }
    fn anthropic(
        &self,
        messages: &[Message],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Reply> {
        let mut payload = json!({"model": self.model, "system": instructions, "max_tokens": 4096,
            "stream": true, "messages": Self::anthropic_history(messages)});
        if self.use_tools {
            payload["tools"] = tools
                .iter()
                .map(|tool| {
                    json!({"name": tool["name"],
                "description": tool["description"], "input_schema": tool["parameters"]})
                })
                .collect();
        }
        let mut headers = self.headers();
        if !headers.iter().any(|(name, _)| name == "anthropic-version") {
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        let (mut text, mut calls, mut ended, mut reason) = (
            String::new(),
            BTreeMap::<u64, (Value, Value, String)>::new(),
            false,
            None,
        );
        http::events(
            &format!("{}/messages", self.base),
            &payload,
            &headers,
            cancel,
            |event| {
                match event["type"].as_str().unwrap_or_default() {
                    "error" => bail!("Anthropicの応答エラー。利用上限を確認してください"),
                    "content_block_start" if event["content_block"]["type"] == "tool_use" => {
                        let block = &event["content_block"];
                        calls.insert(
                            event["index"].as_u64().unwrap_or_default(),
                            (block["id"].clone(), block["name"].clone(), String::new()),
                        );
                    }
                    "content_block_delta" => {
                        let delta = &event["delta"];
                        match delta["type"].as_str().unwrap_or_default() {
                            "text_delta" => {
                                let chunk = delta["text"].as_str().unwrap_or_default();
                                text.push_str(chunk);
                                on_text(chunk);
                            }
                            "input_json_delta" => {
                                if let Some(call) =
                                    calls.get_mut(&event["index"].as_u64().unwrap_or_default())
                                {
                                    call.2.push_str(
                                        delta["partial_json"].as_str().unwrap_or_default(),
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        reason = event["delta"]["stop_reason"].as_str().map(str::to_owned)
                    }
                    "message_stop" => ended = true,
                    _ => {}
                }
                Ok(())
            },
        )?;
        if !ended
            || !matches!(
                reason.as_deref(),
                Some("end_turn" | "tool_use" | "stop_sequence")
            )
        {
            bail!("Anthropic応答が未完了です。ツールは実行していません");
        }
        let tool_calls = calls
            .into_values()
            .map(|(id, name, arguments)| {
                Ok(json!({"id": id, "name": name, "arguments": parse_arguments(&arguments)?}))
            })
            .collect::<Result<_>>()?;
        Ok(Reply {
            text,
            tool_calls,
            raw_output: None,
        })
    }
    pub fn chat_history(messages: &[Message], instructions: &str) -> Vec<Value> {
        let mut history = vec![json!({"role": "system", "content": instructions})];
        for message in messages {
            let mut item = json!({"role": message["role"], "content": message["content"]});
            if message["role"] == "tool" {
                item["tool_call_id"] = message["tool_call_id"].clone();
            }
            if let Some(calls) = message["tool_calls"]
                .as_array()
                .filter(|calls| !calls.is_empty())
            {
                item["tool_calls"] = calls
                    .iter()
                    .map(|call| {
                        json!({"id": call["id"], "type": "function",
                    "function": {"name": call["name"], "arguments": call["arguments"].to_string()}})
                    })
                    .collect();
            }
            history.push(item);
        }
        history
    }
    fn chat(
        &self,
        messages: &[Message],
        instructions: &str,
        tools: &[Value],
        cancel: &CancellationToken,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Reply> {
        let mut payload = json!({"model": self.model, "messages": Self::chat_history(messages, instructions), "stream": true});
        if self.use_tools {
            payload["tools"] = tools
                .iter()
                .map(|tool| json!({"type": "function", "function": tool}))
                .collect();
        }
        let (mut text, mut calls, mut ended) = (
            String::new(),
            BTreeMap::<u64, (String, String, String)>::new(),
            false,
        );
        let use_tools = self.use_tools;
        http::events(
            &format!("{}/chat/completions", self.base),
            &payload,
            &self.headers(),
            cancel,
            |event| {
                if event.get("error").is_some_and(|error| !error.is_null()) {
                    bail!("互換APIの応答エラー");
                }
                for choice in event["choices"].as_array().into_iter().flatten() {
                    if matches!(
                        choice["finish_reason"].as_str(),
                        Some("stop" | "tool_calls")
                    ) {
                        ended = true;
                    }
                    let delta = &choice["delta"];
                    if let Some(content) = delta["content"].as_str().filter(|c| !c.is_empty()) {
                        text.push_str(content);
                        on_text(content);
                    }
                    for call in delta["tool_calls"].as_array().into_iter().flatten() {
                        if !use_tools {
                            bail!("相談専用の互換APIがツールを要求しました。実行していません");
                        }
                        let entry = calls
                            .entry(call["index"].as_u64().unwrap_or_default())
                            .or_default();
                        entry.0.push_str(call["id"].as_str().unwrap_or_default());
                        entry
                            .1
                            .push_str(call["function"]["name"].as_str().unwrap_or_default());
                        entry
                            .2
                            .push_str(call["function"]["arguments"].as_str().unwrap_or_default());
                    }
                }
                Ok(())
            },
        )?;
        if !ended {
            bail!("互換API応答が未完了です。ツールは実行していません");
        }
        let tool_calls = calls
            .into_values()
            .map(|(id, name, arguments)| {
                Ok(json!({"id": id, "name": name, "arguments": parse_arguments(&arguments)?}))
            })
            .collect::<Result<_>>()?;
        Ok(Reply {
            text,
            tool_calls,
            raw_output: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_validation_and_protocol_selection() {
        assert!(ApiProvider::new("openai", "", "k", None, true, "auto").is_err());
        assert!(ApiProvider::new("unknown", "m", "k", None, true, "auto").is_err());
        assert!(ApiProvider::new(
            "openai",
            "m",
            "k",
            Some("https://proxy.example/v1"),
            true,
            "auto"
        )
        .is_err());
        assert_eq!(
            ApiProvider::new("openai", "gpt", "k", None, true, "auto")
                .unwrap()
                .protocol,
            "responses"
        );
        assert_eq!(
            ApiProvider::new("anthropic", "c", "k", None, true, "auto")
                .unwrap()
                .protocol,
            "messages"
        );
        assert_eq!(
            ApiProvider::new("commandcode", "Claude-x", "k", None, true, "auto")
                .unwrap()
                .protocol,
            "messages"
        );
        assert_eq!(
            ApiProvider::new("commandcode", "gpt", "k", None, true, "auto")
                .unwrap()
                .protocol,
            "chat"
        );
        let local = ApiProvider::new(
            "compatible",
            "m",
            "",
            Some("http://127.0.0.1:1234/v1/chat/completions"),
            false,
            "auto",
        )
        .unwrap();
        assert_eq!(
            (local.base.as_str(), local.protocol.as_str()),
            ("http://127.0.0.1:1234/v1", "chat")
        );
        assert!(local.headers().is_empty());
    }

    #[test]
    fn history_formats_match_python() {
        let messages = vec![
            json!({"role": "user", "content": "q"}),
            json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "name": "list_sessions", "arguments": {}}]}),
            json!({"role": "tool", "tool_call_id": "c1", "name": "list_sessions", "content": "[]"}),
            json!({"role": "assistant", "content": "a", "tool_calls": [], "raw_output": [{"type": "reasoning", "id": "r"}]}),
        ];
        let input = responses_input(&messages);
        assert_eq!(input[0], json!({"role": "user", "content": "q"}));
        assert_eq!(
            input[1],
            json!({"type": "function_call", "call_id": "c1", "name": "list_sessions", "arguments": "{}"})
        );
        assert_eq!(
            input[2],
            json!({"type": "function_call_output", "call_id": "c1", "output": "[]"})
        );
        assert_eq!(input[3], json!({"type": "reasoning", "id": "r"}));
        let anthropic = ApiProvider::anthropic_history(&messages[..3]);
        assert_eq!(anthropic.len(), 3);
        assert_eq!(anthropic[1]["content"][0]["type"], "tool_use");
        assert_eq!(anthropic[2]["role"], "user");
        assert_eq!(anthropic[2]["content"][0]["type"], "tool_result");
        let chat = ApiProvider::chat_history(&messages[..3], "sys");
        assert_eq!(chat[0]["role"], "system");
        assert_eq!(chat[2]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(chat[3]["tool_call_id"], "c1");
    }
}

/// 3形式のSSEをローカルの偽サーバーで受け、文字列・ツール要求・未完了の扱いを確認する。
#[cfg(test)]
mod stream_tests {
    use super::*;
    use std::io::{Read, Write};

    fn serve(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![];
            let mut buffer = [0; 65536];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length: usize = text
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (format!("http://{address}/v1"), handle)
    }

    fn run(protocol: &str, body: &'static str) -> (Result<Reply>, String, String) {
        let (base, server) = serve(body);
        let provider =
            ApiProvider::new("compatible", "m", "sk-test", Some(&base), true, protocol).unwrap();
        let mut streamed = String::new();
        let tools = crate::tools::definitions();
        let reply = provider.respond(
            &[json!({"role": "user", "content": "q"})],
            "sys",
            &tools,
            &CancellationToken::new(),
            &mut |text| streamed.push_str(text),
        );
        (reply, streamed, server.join().unwrap())
    }

    #[test]
    fn chat_stream_with_tool_call() {
        let (reply, streamed, request) = run("chat", concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"確認\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_sessions\",\"arguments\":\"{\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"));
        let reply = reply.unwrap();
        assert_eq!((streamed.as_str(), reply.text.as_str()), ("確認", "確認"));
        assert_eq!(
            reply.tool_calls,
            vec![json!({"id": "c1", "name": "list_sessions", "arguments": {}})]
        );
        assert!(
            request.contains("POST /v1/chat/completions") && request.contains("Bearer sk-test")
        );
        assert!(request.contains("\"tools\""));
    }

    #[test]
    fn incomplete_streams_never_return_tool_calls() {
        let (reply, _, _) = run("chat", "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"interrupt\",\"arguments\":\"{}\"}}]}}]}\n\n");
        assert!(reply.unwrap_err().to_string().contains("未完了"));
        let (reply, _, _) = run(
            "responses",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}\n\n",
        );
        assert!(reply.unwrap_err().to_string().contains("途中で切れ"));
        let (reply, _, _) = run("messages", "data: {\"type\":\"message_start\"}\n\n");
        assert!(reply.unwrap_err().to_string().contains("未完了"));
    }

    #[test]
    fn responses_and_messages_complete() {
        let (reply, streamed, request) = run("responses", concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"や\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"reasoning\",\"id\":\"r\"},",
            "{\"type\":\"function_call\",\"call_id\":\"f\",\"name\":\"list_sessions\",\"arguments\":\"{}\"}]}}\n\n"));
        let reply = reply.unwrap();
        assert_eq!(streamed, "や");
        assert_eq!(reply.tool_calls[0]["id"], "f");
        assert_eq!(reply.raw_output.unwrap()[0]["type"], "reasoning");
        assert!(request.contains("\"store\":false") && request.contains("\"strict\":true"));
        let (reply, streamed, request) = run("messages", concat!(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"u\",\"name\":\"interrupt\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"));
        let reply = reply.unwrap();
        assert_eq!(
            (streamed.as_str(), reply.tool_calls[0]["name"].as_str()),
            ("ok", Some("interrupt"))
        );
        assert!(request.contains("POST /v1/messages") && request.contains("anthropic-version"));
    }

    #[test]
    fn consult_only_rejects_returned_tool_calls_and_sends_no_tools() {
        let (base, server) = serve("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"interrupt\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n");
        let provider = ApiProvider::new("compatible", "m", "", Some(&base), false, "chat").unwrap();
        let error = provider
            .respond(
                &[json!({"role": "user", "content": "q"})],
                "sys",
                &crate::tools::definitions(),
                &CancellationToken::new(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(error.to_string().contains("相談専用"));
        assert!(!server.join().unwrap().contains("\"tools\""));
    }
}
