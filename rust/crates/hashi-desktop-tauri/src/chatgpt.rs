//! ChatGPTのサインイン(`hashi_ai::chatgpt`)を画面から使うコマンド。
//! トークンはRust側のAI専用ストアにだけあり、画面へは登録の名前・サインイン中かどうかだけ返す。
//! 認証URLはRust側で既定のブラウザに渡し、画面からURLを受け取って開くことはしない。
use crate::ai::secrets;
use hashi_ai::chatgpt::{Accounts, Endpoints};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// 進行中のサインイン(1つだけ)。画面の「中止」で取り消す。
static SIGNING_IN: Mutex<Option<CancellationToken>> = Mutex::new(None);

pub(crate) fn accounts() -> Result<Arc<Accounts>, String> {
    Ok(Arc::new(Accounts::new(secrets()?, Endpoints::official())))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| "処理に失敗しました".to_owned())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    id: String,
    label: String,
    connected: bool,
}

#[tauri::command]
pub async fn chatgpt_accounts() -> Result<Vec<AccountInfo>, String> {
    blocking(|| {
        Ok(accounts()?
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| AccountInfo {
                id: a.id,
                label: a.label,
                connected: a.connected,
            })
            .collect())
    })
    .await
}

struct Clear;
impl Drop for Clear {
    fn drop(&mut self) {
        if let Ok(mut slot) = SIGNING_IN.lock() {
            *slot = None;
        }
    }
}

/// ブラウザでサインインする。`registration`を渡すとその登録で再認証する。登録のIDを返す。
#[tauri::command]
pub async fn chatgpt_sign_in(registration: Option<String>) -> Result<String, String> {
    let cancel = CancellationToken::new();
    {
        let mut slot = SIGNING_IN.lock().map_err(|_| "状態が不正です")?;
        if slot.is_some() {
            return Err("サインインの途中です。ブラウザで続けるか、中止してください".into());
        }
        *slot = Some(cancel.clone());
    }
    let _clear = Clear;
    blocking(move || {
        accounts()?
            .sign_in(&cancel, registration.as_deref(), &mut |url| {
                webbrowser::open(url).is_ok()
            })
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub fn chatgpt_cancel() {
    if let Ok(slot) = SIGNING_IN.lock() {
        if let Some(cancel) = slot.as_ref() {
            cancel.cancel();
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutResult {
    /// 遠隔の失効を確認できた。偽なら、ローカルのトークンだけ消した。
    revoked: bool,
}

#[tauri::command]
pub async fn chatgpt_sign_out(id: String) -> Result<SignOutResult, String> {
    blocking(move || {
        let revoked = accounts()?.sign_out(&id).map_err(|e| e.to_string())?;
        Ok(SignOutResult { revoked })
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_forget(id: String) -> Result<(), String> {
    blocking(move || accounts()?.forget(&id).map_err(|e| e.to_string())).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    slug: String,
    display_name: String,
}

#[tauri::command]
pub async fn chatgpt_models(id: String) -> Result<Vec<ModelInfo>, String> {
    blocking(move || {
        Ok(accounts()?
            .models(&id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| ModelInfo {
                slug: m.slug,
                display_name: m.display_name,
            })
            .collect())
    })
    .await
}
