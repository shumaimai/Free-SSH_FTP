//! AIサービスの選択と「接続する」の一連の処理。egui版とWeb画面版が同じ規則を使う。
//! 利用者に見せる選択肢はサービスだけで、種別・API形式・URL検証はここで決める。
use crate::api::ApiProvider;
use anyhow::Result;
use hashi_storage::secrets::EncryptedSecrets;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// 利用者が選ぶ接続先。
pub struct Service {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub base: &'static str,
    pub needs_key: bool,
    pub key_page: &'static str,
    pub blurb: &'static str,
}

pub const SERVICES: [Service; 4] = [
    Service {
        id: "openai",
        label: "OpenAI",
        kind: "openai",
        base: "https://api.openai.com/v1",
        needs_key: true,
        key_page: "https://platform.openai.com/api-keys",
        blurb: "ChatGPTのAPI。利用料はOpenAIの料金に従います。",
    },
    Service {
        id: "anthropic",
        label: "Claude",
        kind: "anthropic",
        base: "https://api.anthropic.com/v1",
        needs_key: true,
        key_page: "https://console.anthropic.com/settings/keys",
        blurb: "AnthropicのAPI。利用料はAnthropicの料金に従います。",
    },
    Service {
        id: "local",
        label: "自分のPC",
        kind: "compatible",
        base: "http://127.0.0.1:1234/v1",
        needs_key: false,
        key_page: "",
        blurb: "LM StudioやOllamaなど、このPCで動くAI。キーは要りません（Ollamaは http://127.0.0.1:11434/v1）。",
    },
    Service {
        id: "custom",
        label: "その他",
        kind: "compatible",
        base: "",
        needs_key: true,
        key_page: "",
        blurb: "OpenAI互換のAPIを出している他社サービス。接続先URLを入力してください。",
    },
];

pub const PROTOCOLS: [(&str, &str); 4] = [
    ("auto", "自動"),
    ("chat", "Chat Completions"),
    ("responses", "Responses"),
    ("messages", "Anthropic Messages"),
];

pub fn service(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|service| service.id == id)
}

/// 接続先のURL。公式サービスは固定で、入力されたURLを使わない。
pub fn effective_base(service: &Service, typed: &str) -> String {
    match service.id {
        "local" | "custom" if !typed.trim().is_empty() => typed.trim().to_owned(),
        _ => service.base.to_owned(),
    }
}

/// 公式でない接続先の種別。Command CodeのURLだけは専用の扱いにする。
pub fn kind_for(service: &Service, base: &str) -> &'static str {
    if service.kind == "compatible" {
        if let Ok(base) = crate::http::normalize_api_base(base) {
            if base == crate::api::default_base("commandcode") {
                return "commandcode";
            }
        }
    }
    service.kind
}

/// 一覧から最初に使うモデル。各社の主力に当たりやすい名前を優先し、なければ先頭。
pub fn default_model(kind: &str, models: &[String]) -> Option<String> {
    let preferred: &[&str] = match kind {
        "openai" => &["gpt-5", "gpt-4.1", "gpt-4o"],
        "anthropic" => &["claude-sonnet", "claude-opus", "claude"],
        _ => &[],
    };
    for prefix in preferred {
        if let Some(model) = models
            .iter()
            .filter(|m| m.starts_with(prefix) && !m.contains("mini") && !m.contains("nano"))
            .min_by_key(|m| m.len())
        {
            return Some(model.clone());
        }
    }
    models.first().cloned()
}

/// モデル一覧が示す対応エンドポイント（Command Code等）から、そのモデルで使う通信方式を選ぶ。
/// 一覧に情報がなければ`None`（従来どおりモデル名とURLで決める）。
pub fn protocol_from_endpoints(model: &str, listed: &[serde_json::Value]) -> Option<&'static str> {
    let entry = listed.iter().find(|m| m["id"].as_str() == Some(model))?;
    let endpoints: Vec<&str> = entry["supported_endpoints"]
        .as_array()?
        .iter()
        .filter_map(|e| e.as_str())
        .collect();
    let has = |name: &str| {
        endpoints
            .iter()
            .any(|e| e.trim_end_matches('/').ends_with(name))
    };
    if has("/messages") && model.to_lowercase().contains("claude") {
        Some("messages")
    } else if has("/chat/completions") {
        Some("chat")
    } else if has("/responses") {
        Some("responses")
    } else if has("/messages") {
        Some("messages")
    } else {
        None
    }
}

/// 保存キーの名前。`api:{種別}:{正規化URLのSHA-256}`。
pub fn key_id(kind: &str, base: &str) -> Result<String, String> {
    use sha2::Digest;
    let base = crate::http::normalize_api_base(base).map_err(|e| e.to_string())?;
    let digest = sha2::Sha256::digest(base.as_bytes());
    Ok(format!(
        "api:{kind}:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

/// 画面の入力から接続用のproviderを作る。通信はしない。
/// 公式サービスは入力URLを無視し、キー必須のサービスで空のキーは受け付けない。
pub fn build(
    service: &Service,
    typed_base: &str,
    key: &str,
    model: &str,
    protocol: &str,
    use_tools: bool,
) -> Result<ApiProvider, String> {
    let base = effective_base(service, typed_base);
    let base = crate::http::normalize_api_base(&base).map_err(|e| e.to_string())?;
    let kind = kind_for(service, &base);
    if service.needs_key && key.trim().is_empty() {
        return Err("APIキーを入力してください。".into());
    }
    if !PROTOCOLS.iter().any(|(id, _)| *id == protocol) {
        return Err("API形式が不正です".into());
    }
    let protocol = if matches!(kind, "compatible" | "commandcode") {
        protocol
    } else {
        "auto"
    };
    ApiProvider::new(
        kind,
        if model.is_empty() {
            "model-list"
        } else {
            model
        },
        key.trim(),
        Some(&base),
        use_tools,
        protocol,
    )
    .map_err(|e| e.to_string())
}

pub enum Outcome {
    Connected(Box<ApiProvider>, Vec<String>),
    /// モデルを自動で決められなかった。画面は入力欄を出す。
    NeedModel(String, Vec<String>),
}

/// 「接続する」: モデル一覧の取得、モデルの決定、toolsなしの応答確認、キーの保存。
/// 確認で失敗したら何も保存しない。キーやURLの誤り（401/403）は原因をそのまま返す。
pub fn connect(
    probe: ApiProvider,
    typed_model: &str,
    key_id: &str,
    persist: bool,
    secrets: &EncryptedSecrets,
) -> Result<Outcome, String> {
    let cancel = CancellationToken::new();
    let raw = probe.models(&cancel);
    let raw_models = raw.as_ref().cloned().unwrap_or_default();
    let listed = raw.map(|models| {
        models
            .iter()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    });
    let models = listed.as_ref().cloned().unwrap_or_default();
    let typed = typed_model.trim();
    let model = if !typed.is_empty() {
        typed.to_owned()
    } else {
        match (&listed, default_model(&probe.kind, &models)) {
            (Ok(_), Some(model)) => model,
            (Ok(_), None) => {
                return Ok(Outcome::NeedModel(
                    "使えるモデルが見つかりません。モデル名を入力してください。".into(),
                    models,
                ))
            }
            (Err(error), _) => {
                let message = error.to_string();
                return if message.contains("HTTP 401") || message.contains("HTTP 403") {
                    Err(message)
                } else {
                    Ok(Outcome::NeedModel(
                        format!("モデル一覧を取得できませんでした（{message}）。モデル名を入力して、もう一度「接続する」を押してください。"),
                        models,
                    ))
                };
            }
        }
    };
    // 通信方式は「自動」のままなら、決まったモデルに合わせて選び直す（仮の名前で決めた方式を引き継がない）。
    let protocol = if probe.auto_protocol {
        protocol_from_endpoints(&model, &raw_models).unwrap_or("auto")
    } else {
        probe.protocol.as_str()
    };
    let provider = ApiProvider::new(
        &probe.kind,
        &model,
        probe.key(),
        Some(&probe.base),
        probe.use_tools,
        protocol,
    )
    .map_err(|e| e.to_string())?;
    let mut provider = provider;
    // 利用者が方式を選んでいないことを覚えておく（モデルを切り替えたとき、そのモデルに合わせて選び直す）。
    provider.auto_protocol = probe.auto_protocol;
    // 接続確認ではtoolsを送らない。
    let mut check = provider.clone();
    check.use_tools = false;
    if let Err(error) = check.respond(
        &[json!({"role": "user", "content": "OKとだけ答えてください。"})],
        "接続確認です。端末操作をせず短く回答してください。",
        &[],
        &cancel,
        &mut |_| {},
    ) {
        let message = error.to_string();
        // 自動で選んだモデルが使えなかっただけなら、モデルを選び直せるようにする。キーやURLの誤りはそのまま返す。
        let auth = message.contains("HTTP 401") || message.contains("HTTP 403");
        return if typed.is_empty() && !auth {
            Ok(Outcome::NeedModel(
                format!("自動で選んだモデル「{model}」で確認に失敗しました（{message}）。別のモデル名を選ぶか入力して、もう一度「接続する」を押してください。"),
                models,
            ))
        } else {
            Err(message)
        };
    }
    secrets
        .set(
            key_id,
            persist
                .then(|| provider.key().to_owned())
                .as_deref()
                .filter(|v| !v.is_empty()),
        )
        .map_err(|_| "APIキーを保存できませんでした。もう一度お試しください。".to_owned())?;
    Ok(Outcome::Connected(Box::new(provider), models))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_model_prefers_main_models_over_small_variants() {
        let models = names(&["gpt-4o-mini", "gpt-5-mini", "gpt-5", "gpt-5-2025", "o3"]);
        assert_eq!(default_model("openai", &models).as_deref(), Some("gpt-5"));
        let claude = names(&["claude-haiku-4", "claude-sonnet-4-5", "claude-opus-4"]);
        assert_eq!(
            default_model("anthropic", &claude).as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            default_model("compatible", &names(&["local-model", "b"])).as_deref(),
            Some("local-model")
        );
        assert_eq!(default_model("openai", &[]), None);
    }

    #[test]
    fn only_command_code_url_changes_the_kind_of_a_custom_service() {
        let custom = &SERVICES[3];
        assert_eq!(kind_for(custom, "https://example.com/v1"), "compatible");
        assert_eq!(
            kind_for(custom, "https://api.commandcode.ai/v1"),
            "commandcode"
        );
        assert_eq!(
            kind_for(&SERVICES[0], "https://api.commandcode.ai/v1"),
            "openai"
        );
        assert_eq!(kind_for(custom, "not a url"), "compatible");
    }

    #[test]
    fn official_services_ignore_a_typed_base_url() {
        assert_eq!(
            effective_base(&SERVICES[0], "https://evil.example.com/v1"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            effective_base(&SERVICES[3], " https://evil.example.com/v1 "),
            "https://evil.example.com/v1"
        );
        assert_eq!(effective_base(&SERVICES[3], ""), "");
    }

    #[test]
    fn protocol_follows_the_chosen_models_supported_endpoints() {
        let listed = vec![
            json!({"id": "claude-sonnet-5-5", "supported_endpoints": ["/messages"]}),
            json!({"id": "gpt-x", "supported_endpoints": ["/chat/completions", "/responses"]}),
            json!({"id": "resp-only", "supported_endpoints": ["/responses"]}),
            json!({"id": "no-info"}),
        ];
        assert_eq!(
            protocol_from_endpoints("claude-sonnet-5-5", &listed),
            Some("messages")
        );
        assert_eq!(protocol_from_endpoints("gpt-x", &listed), Some("chat"));
        assert_eq!(
            protocol_from_endpoints("resp-only", &listed),
            Some("responses")
        );
        assert_eq!(protocol_from_endpoints("no-info", &listed), None);
        assert_eq!(protocol_from_endpoints("unknown", &listed), None);
    }

    #[test]
    fn auto_protocol_is_remembered_so_it_can_follow_the_chosen_model() {
        let placeholder =
            ApiProvider::new("commandcode", "model-list", "k", None, true, "auto").unwrap();
        assert!(placeholder.auto_protocol);
        assert_eq!(placeholder.protocol, "chat");
        let explicit =
            ApiProvider::new("commandcode", "claude-x", "k", None, true, "chat").unwrap();
        assert!(!explicit.auto_protocol);
    }
}
