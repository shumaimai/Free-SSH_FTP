//! AIモデルの管理。接続先（サービス・URL・キー）とモデルを複数登録でき、AIドックでは選ぶだけにする。
//! 登録内容は設定の`ai_models`（配列）と`ai_active`（使用中のid）。キーは同じサービス・URLで共有し、
//! AI専用の暗号化ストアにだけ保存する。キーは画面へ返さない（保存済みかどうかだけ伝える）。
use crate::{
    ai::{adopt, load_saved_key, save_connection, secrets, CHATGPT},
    app::AppState,
};
use hashi_ai::{
    api::ApiProvider,
    connect::{self, Outcome, PROTOCOLS},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use tauri::{AppHandle, Emitter, Manager, State};

const MAX_MODELS: usize = 32;

/// 登録されたAIモデル1件。画面とは同じ形でやり取りする。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelEntry {
    pub id: String,
    /// 表示名。空ならサービスとモデル名から作る。
    pub name: String,
    pub service: String,
    /// 接続先URL。「その他」「自分のPC」だけで使い、公式サービスでは無視する。
    pub base: String,
    /// 空なら使うときに一覧から選ぶ。
    pub model: String,
    pub protocol: String,
    pub use_tools: bool,
    /// 通信方式が「自動」のとき、前回モデルの一覧から決めた方式。変更したら消える。
    pub resolved: String,
    /// ChatGPTサインインの登録ID(`service`が`chatgpt`のときだけ)。
    pub account: String,
}

impl Default for ModelEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            service: "openai".into(),
            base: String::new(),
            model: String::new(),
            protocol: "auto".into(),
            use_tools: true,
            resolved: String::new(),
            account: String::new(),
        }
    }
}

fn is_chatgpt(entry: &ModelEntry) -> bool {
    entry.service == CHATGPT
}

/// サインイン中のChatGPT登録のID。
fn connected_accounts() -> Result<Vec<String>, String> {
    Ok(crate::chatgpt::accounts()?
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|a| a.connected)
        .map(|a| a.id)
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    #[serde(flatten)]
    entry: ModelEntry,
    /// 画面に出す名前。
    label: String,
    /// キーが要らない、または保存済み。
    has_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsList {
    entries: Vec<ModelView>,
    active: Option<String>,
    /// 保存はできたが、使用中のモデルの読み直しに失敗したときの知らせ。
    warning: Option<String>,
}

fn label_of(entry: &ModelEntry) -> String {
    if !entry.name.trim().is_empty() {
        return entry.name.trim().to_owned();
    }
    let service = if is_chatgpt(entry) {
        "ChatGPT"
    } else {
        connect::service(&entry.service).map_or("AI", |s| s.label)
    };
    if entry.model.is_empty() {
        format!("{service}（モデルは自動）")
    } else {
        format!("{service} / {}", entry.model)
    }
}

fn read(state: &AppState) -> Result<(Vec<ModelEntry>, Option<String>), String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    let entries = match snapshot.settings.get("ai_models") {
        None => vec![],
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "AIモデルの保存データを読めません。設定を確認してください")?,
    };
    let active = snapshot
        .settings
        .get("ai_active")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|id| entries.iter().any(|e: &ModelEntry| &e.id == id));
    Ok((entries, active))
}

fn write(state: &AppState, entries: &[ModelEntry], active: Option<&str>) -> Result<(), String> {
    let value = serde_json::to_value(entries).map_err(|e| e.to_string())?;
    let active = active.map(str::to_owned);
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot.settings.insert("ai_models".into(), value);
            match active {
                Some(id) => snapshot
                    .settings
                    .insert("ai_active".into(), Value::String(id)),
                None => snapshot.settings.remove("ai_active"),
            };
            Ok(())
        })
        .map_err(|error: io::Error| format!("AIモデルを保存できませんでした: {error}"))?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(())
}

/// 旧版の単一の接続設定（`ai_connection`）を、登録の1件目へ移す。登録がすでにあれば何もしない。
fn migrate(state: &AppState) -> Result<(), String> {
    let saved = {
        let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
        if snapshot.settings.contains_key("ai_models") {
            return Ok(());
        }
        snapshot.settings.get("ai_connection").cloned()
    };
    let Some(saved) = saved else { return Ok(()) };
    let (Some(service), Some(model)) = (saved["service"].as_str(), saved["model"].as_str()) else {
        return Ok(());
    };
    if connect::service(service).is_none() {
        return Ok(());
    }
    let auto = saved["protocol_auto"].as_bool().unwrap_or(false);
    let protocol = saved["protocol"].as_str().unwrap_or("auto");
    let entry = ModelEntry {
        id: uuid::Uuid::new_v4().to_string(),
        service: service.to_owned(),
        base: saved["base"].as_str().unwrap_or_default().to_owned(),
        model: model.to_owned(),
        protocol: if auto { "auto" } else { protocol }.to_owned(),
        resolved: if auto {
            protocol.to_owned()
        } else {
            String::new()
        },
        use_tools: saved["use_tools"].as_bool().unwrap_or(true),
        ..ModelEntry::default()
    };
    let id = entry.id.clone();
    write(state, &[entry], Some(&id))
}

/// 入力を整えて検証する。通信はしない。
fn validate(entry: &mut ModelEntry) -> Result<(), String> {
    entry.name = entry.name.trim().to_owned();
    entry.base = entry.base.trim().to_owned();
    entry.model = entry.model.trim().to_owned();
    if entry.name.chars().count() > 100 {
        return Err("名前は100文字までです".into());
    }
    if entry.model.len() > 200 || entry.model.chars().any(char::is_control) {
        return Err("モデル名が不正です".into());
    }
    if is_chatgpt(entry) {
        // 接続先は公式で固定。キーの代わりにサインインした登録を使い、モデルは一覧から選ぶ。
        entry.base.clear();
        entry.protocol = "auto".into();
        entry.resolved.clear();
        if entry.account.is_empty()
            || entry.account.len() > 512
            || entry.account.chars().any(char::is_control)
        {
            return Err("ChatGPTにサインインして、アカウントを選んでください".into());
        }
        if entry.model.is_empty() {
            return Err("モデルを一覧から選んでください".into());
        }
        return Ok(());
    }
    entry.account.clear();
    let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
    if entry.base.len() > 2048 {
        return Err("接続先URLが長すぎます".into());
    }
    if !matches!(service.id, "local" | "custom") {
        // 公式サービスの接続先は固定。入力されたURLを持たない。
        entry.base.clear();
    } else if service.id == "custom" && entry.base.is_empty() {
        return Err("接続先URLを入力してください".into());
    }
    if !PROTOCOLS.iter().any(|(id, _)| *id == entry.protocol) {
        return Err("通信の形式が不正です".into());
    }
    // 形式・URLの検証だけ。キーは入れないので仮の値で組み立てる。
    connect::build(
        service,
        &entry.base,
        "x",
        &entry.model,
        &entry.protocol,
        entry.use_tools,
    )?;
    Ok(())
}

fn key_for(entry: &ModelEntry) -> Result<String, String> {
    if is_chatgpt(entry) {
        return Ok(String::new());
    }
    let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
    if !service.needs_key {
        return Ok(String::new());
    }
    load_saved_key(service, &connect::effective_base(service, &entry.base))
}

fn key_id_for(entry: &ModelEntry) -> Result<String, String> {
    let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
    let base = connect::effective_base(service, &entry.base);
    connect::key_id(connect::kind_for(service, &base), &base)
}

fn list_of(
    entries: Vec<ModelEntry>,
    active: Option<String>,
    warning: Option<String>,
) -> Result<ModelsList, String> {
    let secrets = secrets()?;
    let signed_in = if entries.iter().any(is_chatgpt) {
        connected_accounts().unwrap_or_default()
    } else {
        vec![]
    };
    let views = entries
        .into_iter()
        .map(|entry| {
            let needs_key = connect::service(&entry.service).is_some_and(|s| s.needs_key);
            let has_key = if is_chatgpt(&entry) {
                signed_in.contains(&entry.account)
            } else {
                !needs_key
                    || key_id_for(&entry)
                        .ok()
                        .and_then(|id| secrets.get(&id).ok().flatten())
                        .is_some_and(|key| !key.is_empty())
            };
            ModelView {
                label: label_of(&entry),
                has_key,
                entry,
            }
        })
        .collect();
    Ok(ModelsList {
        entries: views,
        active,
        warning,
    })
}

/// 登録の一覧と使用中のid。通信はしない。
#[tauri::command]
pub async fn ai_models_list(state: State<'_, AppState>) -> Result<ModelsList, String> {
    migrate(&state)?;
    let (entries, active) = read(&state)?;
    tokio::task::spawn_blocking(move || list_of(entries, active, None))
        .await
        .map_err(|_| "処理に失敗しました".to_owned())?
}

struct Busy<'a>(&'a AppState);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.0.ai.connecting.lock() {
            *busy = false;
        }
    }
}
fn busy(state: &AppState) -> Result<Busy<'_>, String> {
    let mut flag = state.ai.connecting.lock().map_err(|_| "状態が不正です")?;
    if *flag {
        return Err("接続の確認中です。終わるまでお待ちください".into());
    }
    *flag = true;
    Ok(Busy(state))
}

/// 登録内容から、会話に使うproviderを作る。通信方式が「自動」なら、一覧から決めた方式を使う。
/// `network`が真で未決定なら、一覧を取得して決める（取得できなければ名前から決める）。
fn provider_for(
    entry: &ModelEntry,
    key: &str,
    network: bool,
) -> Result<(ApiProvider, String), String> {
    if is_chatgpt(entry) {
        // 通信しない。トークンは最初の要求のときに取得・更新する。
        let provider = hashi_ai::chatgpt::provider(
            crate::chatgpt::accounts()?,
            &entry.account,
            &entry.model,
            entry.use_tools,
        )
        .map_err(|e| e.to_string())?;
        return Ok((provider, String::new()));
    }
    let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
    let auto = entry.protocol == "auto";
    let mut resolved = if auto {
        entry.resolved.clone()
    } else {
        String::new()
    };
    let protocol = if resolved.is_empty() {
        entry.protocol.as_str()
    } else {
        resolved.as_str()
    };
    let mut provider = connect::build(
        service,
        &entry.base,
        key,
        &entry.model,
        protocol,
        entry.use_tools,
    )?;
    if network
        && auto
        && resolved.is_empty()
        && matches!(provider.kind.as_str(), "compatible" | "commandcode")
    {
        let listed = provider
            .models(&tokio_util::sync::CancellationToken::new())
            .unwrap_or_default();
        if let Some(found) = connect::protocol_from_endpoints(&entry.model, &listed) {
            resolved = found.to_owned();
            provider = connect::build(
                service,
                &entry.base,
                key,
                &entry.model,
                &resolved,
                entry.use_tools,
            )?;
        }
    }
    provider.auto_protocol = auto;
    Ok((provider, resolved))
}

fn changed(app: &AppHandle) {
    let _ = app.emit("ai-models-changed", ());
}

/// 登録を追加・更新する。通信はしない。キーが空なら保存済みのキーをそのまま使う。
/// 使用中のモデルを更新したときは、新しい内容で読み直す（会話は新しくなる）。
#[tauri::command]
pub async fn ai_model_save(
    app: AppHandle,
    state: State<'_, AppState>,
    entry: ModelEntry,
    key: String,
) -> Result<ModelsList, String> {
    state.writable()?;
    migrate(&state)?;
    let mut entry = entry;
    validate(&mut entry)?;
    let key = key.trim().to_owned();
    let (mut entries, mut active) = read(&state)?;
    let previous = entries.iter().find(|e| e.id == entry.id).cloned();
    if previous.is_none() {
        if entries.len() >= MAX_MODELS {
            return Err(format!("登録は{MAX_MODELS}件までです"));
        }
        entry.id = uuid::Uuid::new_v4().to_string();
    }
    // 接続先・モデル・形式を変えたら、前に決めた通信方式は使わない。
    entry.resolved = previous
        .as_ref()
        .filter(|p| {
            p.service == entry.service
                && p.account == entry.account
                && p.base == entry.base
                && p.model == entry.model
                && p.protocol == entry.protocol
        })
        .map(|p| p.resolved.clone())
        .unwrap_or_default();
    if !key.is_empty() && connect::service(&entry.service).is_some_and(|s| s.needs_key) {
        let id = key_id_for(&entry)?;
        let value = key.clone();
        tokio::task::spawn_blocking(move || {
            secrets()?
                .set(&id, Some(&value))
                .map_err(|_| "APIキーを保存できませんでした。もう一度お試しください。".to_owned())
        })
        .await
        .map_err(|_| "処理に失敗しました".to_owned())??;
    }
    match entries.iter_mut().find(|e| e.id == entry.id) {
        Some(slot) => *slot = entry.clone(),
        None => entries.push(entry.clone()),
    }
    if active.is_none() {
        active = Some(entry.id.clone());
    }
    write(&state, &entries, active.as_deref())?;
    let mut warning = None;
    if active.as_deref() == Some(entry.id.as_str()) {
        // 使用中の登録を変えた（または初めての登録）ので、新しい内容で読み直す。
        if let Err(error) = activate(&app, &state, &entry.id, false).await {
            warning = Some(format!(
                "保存しましたが、このモデルを使える状態にできませんでした: {error}"
            ));
        }
    }
    changed(&app);
    let (entries, active) = read(&state)?;
    tokio::task::spawn_blocking(move || list_of(entries, active, warning))
        .await
        .map_err(|_| "処理に失敗しました".to_owned())?
}

/// 登録を1件削除する。使用中なら接続を外す。キーは消さない（同じサービス・URLの他の登録が使うため）。
#[tauri::command]
pub async fn ai_model_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<ModelsList, String> {
    state.writable()?;
    migrate(&state)?;
    let (mut entries, active) = read(&state)?;
    let before = entries.len();
    entries.retain(|e| e.id != id);
    if entries.len() == before {
        return Err("登録が見つかりません".into());
    }
    let was_active = active.as_deref() == Some(id.as_str());
    let active = if was_active { None } else { active };
    write(&state, &entries, active.as_deref())?;
    if was_active {
        disconnect(&state);
        // 次の起動で消した接続を読み戻さない。
        let snapshot = state
            .store
            .transact(|snapshot| {
                snapshot.settings.remove("ai_connection");
                Ok(())
            })
            .map_err(|_| "接続設定を保存できませんでした".to_owned())?;
        *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    }
    changed(&app);
    let (entries, active) = read(&state)?;
    tokio::task::spawn_blocking(move || list_of(entries, active, None))
        .await
        .map_err(|_| "処理に失敗しました".to_owned())?
}

fn disconnect(state: &AppState) {
    state.ai.cancel_running();
    state.ai.broker.stop();
    if let Ok(mut preview) = state.ai.preview.lock() {
        *preview = None;
    }
    if let Ok(mut slot) = state.ai.provider.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state.ai.conn.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state.ai.conversation.lock() {
        *slot = None;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UseResult {
    model: String,
}

/// 登録を使う状態にする。`verify`が真でモデルが空のときは、一覧から選ぶために通信して確認する。
async fn activate(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    announce: bool,
) -> Result<UseResult, String> {
    let (mut entries, _) = read(state)?;
    let entry = entries
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or("登録が見つかりません")?;
    let job = entry.clone();
    let (provider, models, resolved) = tokio::task::spawn_blocking(move || {
        if is_chatgpt(&job) {
            if !connected_accounts()?.contains(&job.account) {
                return Err(
                    "このChatGPTアカウントはサインアウトしています。AIモデルの管理でサインインしてください"
                        .to_owned(),
                );
            }
            return provider_for(&job, "", false)
                .map(|(provider, resolved)| (provider, vec![], resolved));
        }
        let key = key_for(&job)?;
        let needs_key = connect::service(&job.service).is_some_and(|s| s.needs_key);
        if needs_key && key.is_empty() {
            return Err(
                "このモデルのAPIキーが保存されていません。AIモデルの管理でキーを入力してください"
                    .to_owned(),
            );
        }
        if job.model.is_empty() {
            // モデルが空のときだけ通信して、使えるモデルを決める。
            let service = connect::service(&job.service).ok_or("サービスが不正です")?;
            let probe = connect::build(service, &job.base, &key, "", &job.protocol, job.use_tools)?;
            return match connect::verify(probe, "")? {
                Outcome::Connected(provider, models) => {
                    let resolved = if provider.auto_protocol {
                        provider.protocol.clone()
                    } else {
                        String::new()
                    };
                    Ok((*provider, models, resolved))
                }
                Outcome::NeedModel(message, _) => Err(message),
            };
        }
        provider_for(&job, &key, true).map(|(provider, resolved)| (provider, vec![], resolved))
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())??;
    let model = provider.model.clone();
    // 決まったモデルと通信方式を登録へ戻し、次からは通信なしで使えるようにする。
    if let Some(slot) = entries.iter_mut().find(|e| e.id == id) {
        slot.model = model.clone();
        slot.resolved = resolved;
    }
    write(state, &entries, Some(id))?;
    save_connection(state, &entry.service, &entry.base, &provider)?;
    adopt(state, &entry.service, provider, models);
    if announce {
        changed(app);
    }
    Ok(UseResult { model })
}

/// 登録したモデルを使う（AIドックの選択）。会話は新しくなる。
#[tauri::command]
pub async fn ai_model_use(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<UseResult, String> {
    state.writable()?;
    migrate(&state)?;
    let _busy = busy(&state)?;
    activate(&app, &state, &id, true).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    ok: bool,
    message: String,
    /// 確認で使われた（選ばれた）モデル。
    model: Option<String>,
    models: Vec<String>,
}

/// フォームの内容で接続を確認する。保存もキーの書き換えもしない。
/// キーが空なら保存済みのキーを使う。モデルが空なら自動で選ぶ。
#[tauri::command]
pub async fn ai_model_check(
    state: State<'_, AppState>,
    entry: ModelEntry,
    key: String,
) -> Result<CheckResult, String> {
    let _busy = busy(&state)?;
    let mut entry = entry;
    validate(&mut entry)?;
    let typed = key.trim().to_owned();
    tokio::task::spawn_blocking(move || {
        if is_chatgpt(&entry) {
            // toolsを送らずに短い応答だけ確かめる。登録もトークン以外の保存内容も変えない。
            let (mut provider, _) = provider_for(&entry, "", false)?;
            provider.use_tools = false;
            provider
                .respond(
                    &[serde_json::json!({"role": "user", "content": "OKとだけ答えてください。"})],
                    "接続確認です。端末操作をせず短く回答してください。",
                    &[],
                    &tokio_util::sync::CancellationToken::new(),
                    &mut |_| {},
                )
                .map_err(|e| e.to_string())?;
            return Ok(CheckResult {
                ok: true,
                message: "接続できました".into(),
                model: Some(entry.model.clone()),
                models: vec![],
            });
        }
        let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
        let key = if typed.is_empty() {
            key_for(&entry)?
        } else {
            typed
        };
        if service.needs_key && key.is_empty() {
            return Err("APIキーを入力してください".to_owned());
        }
        let probe = connect::build(
            service,
            &entry.base,
            &key,
            &entry.model,
            &entry.protocol,
            entry.use_tools,
        )?;
        Ok(match connect::verify(probe, &entry.model)? {
            Outcome::Connected(provider, models) => CheckResult {
                ok: true,
                message: "接続できました".into(),
                model: Some(provider.model.clone()),
                models,
            },
            Outcome::NeedModel(message, models) => CheckResult {
                ok: false,
                message,
                model: None,
                models,
            },
        })
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())?
}

/// 接続先が公開しているモデルの一覧だけを取得する（質問は送らない）。
#[tauri::command]
pub async fn ai_model_candidates(
    state: State<'_, AppState>,
    entry: ModelEntry,
    key: String,
) -> Result<Vec<String>, String> {
    let _busy = busy(&state)?;
    let mut entry = entry;
    validate(&mut entry)?;
    let typed = key.trim().to_owned();
    tokio::task::spawn_blocking(move || {
        let service = connect::service(&entry.service).ok_or("サービスが不正です")?;
        let key = if typed.is_empty() {
            key_for(&entry)?
        } else {
            typed
        };
        if service.needs_key && key.is_empty() {
            return Err("APIキーを入力してください".to_owned());
        }
        let probe = connect::build(service, &entry.base, &key, "", &entry.protocol, true)?;
        let listed = probe
            .models(&tokio_util::sync::CancellationToken::new())
            .map_err(|error| error.to_string())?;
        Ok(listed
            .iter()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect())
    })
    .await
    .map_err(|_| "処理に失敗しました".to_owned())?
}

/// 起動時に、使用中の登録を通信なしで読み戻す。旧版の単一設定は、先に登録の1件目へ移す。
pub fn restore_active(app: &AppHandle) {
    let state = app.state::<AppState>();
    if migrate(&state).is_err() {
        return;
    }
    let Ok((entries, Some(active))) = read(&state) else {
        return;
    };
    let Some(entry) = entries.iter().find(|e| e.id == active) else {
        return;
    };
    if entry.model.is_empty() {
        return;
    }
    let Ok(key) = key_for(entry) else { return };
    if connect::service(&entry.service).is_some_and(|s| s.needs_key) && key.is_empty() {
        return;
    }
    if let Ok((provider, _)) = provider_for(entry, &key, false) {
        adopt(&state, &entry.service, provider, vec![]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(service: &str) -> ModelEntry {
        ModelEntry {
            service: service.into(),
            model: "m".into(),
            ..ModelEntry::default()
        }
    }

    #[test]
    fn official_services_drop_the_typed_url_and_custom_requires_one() {
        let mut official = ModelEntry {
            base: "https://evil.example/v1".into(),
            ..entry("openai")
        };
        validate(&mut official).unwrap();
        assert!(official.base.is_empty());
        assert!(validate(&mut entry("custom")).is_err());
        let mut custom = ModelEntry {
            base: " http://127.0.0.1:1234/v1 ".into(),
            ..entry("custom")
        };
        validate(&mut custom).unwrap();
        assert_eq!(custom.base, "http://127.0.0.1:1234/v1");
        assert!(validate(&mut entry("nope")).is_err());
    }

    #[test]
    fn bad_protocol_name_and_model_are_rejected() {
        let mut bad = ModelEntry {
            protocol: "weird".into(),
            ..entry("openai")
        };
        assert!(validate(&mut bad).is_err());
        let mut bad = ModelEntry {
            name: "あ".repeat(101),
            ..entry("openai")
        };
        assert!(validate(&mut bad).is_err());
        let mut bad = ModelEntry {
            model: "a\nb".into(),
            ..entry("openai")
        };
        assert!(validate(&mut bad).is_err());
    }

    #[test]
    fn chatgpt_entries_need_an_account_and_a_model_and_keep_no_url() {
        let mut ok = ModelEntry {
            service: CHATGPT.into(),
            account: "issued-1".into(),
            base: "https://evil.example/v1".into(),
            protocol: "chat".into(),
            ..entry("x")
        };
        validate(&mut ok).unwrap();
        assert!(ok.base.is_empty());
        assert_eq!(ok.protocol, "auto");
        assert_eq!(label_of(&ok), "ChatGPT / m");
        let mut no_account = ModelEntry {
            service: CHATGPT.into(),
            ..entry("x")
        };
        assert!(validate(&mut no_account).is_err());
        let mut no_model = ModelEntry {
            model: String::new(),
            ..ok.clone()
        };
        assert!(validate(&mut no_model).is_err());
        // 他のサービスへ替えたらアカウントは持ち越さない。
        let mut other = ModelEntry {
            service: "openai".into(),
            ..ok
        };
        validate(&mut other).unwrap();
        assert!(other.account.is_empty());
    }

    #[test]
    fn labels_use_the_name_or_the_service_and_model() {
        assert_eq!(label_of(&entry("anthropic")), "Claude / m");
        assert_eq!(
            label_of(&ModelEntry {
                name: " 仕事用 ".into(),
                ..entry("anthropic")
            }),
            "仕事用"
        );
        assert!(label_of(&ModelEntry {
            model: String::new(),
            ..entry("openai")
        })
        .contains("自動"));
    }

    #[test]
    fn provider_uses_the_resolved_protocol_only_for_auto() {
        let key = "k";
        let auto = ModelEntry {
            service: "custom".into(),
            base: "http://127.0.0.1:1/v1".into(),
            resolved: "messages".into(),
            ..entry("custom")
        };
        let (provider, resolved) = provider_for(&auto, key, false).unwrap();
        assert_eq!(
            (provider.protocol.as_str(), resolved.as_str()),
            ("messages", "messages")
        );
        assert!(provider.auto_protocol);
        let explicit = ModelEntry {
            protocol: "chat".into(),
            ..auto
        };
        let (provider, resolved) = provider_for(&explicit, key, false).unwrap();
        assert_eq!(
            (provider.protocol.as_str(), resolved.as_str()),
            ("chat", "")
        );
        assert!(!provider.auto_protocol);
    }
}
