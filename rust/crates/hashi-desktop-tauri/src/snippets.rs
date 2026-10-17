//! スニペット（よく使うコマンド）。保存形式はPython版と同じ配列で、解析できない記録は上書きしない。
use crate::{app::AppState, session};
use hashi_storage::snippets::{self, Snippet};
use std::collections::HashMap;
use tauri::State;

const MAX_SNIPPETS: usize = 256;
const MAX_BODY: usize = 16 * 1024;
const MAX_VALUE: usize = 4096;

fn read(state: &AppState) -> Result<Vec<Snippet>, String> {
    let snapshot = state.snapshot.lock().map_err(|_| "状態が不正です")?;
    snippets::parse(&snapshot.snippets).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn snippets_list(state: State<'_, AppState>) -> Result<Vec<Snippet>, String> {
    read(&state)
}

#[tauri::command]
pub fn snippets_save(
    state: State<'_, AppState>,
    list: Vec<Snippet>,
) -> Result<Vec<Snippet>, String> {
    state.writable()?;
    // 保存済みの記録が解析できないときは、上書きしない。
    read(&state)?;
    if list.len() > MAX_SNIPPETS {
        return Err(format!("スニペットは{MAX_SNIPPETS}件までです"));
    }
    for snippet in &list {
        if snippet.name.trim().is_empty() || snippet.name.len() > 200 {
            return Err("スニペットの名前を入力してください（200バイトまで）".into());
        }
        if snippet.body.len() > MAX_BODY {
            return Err("スニペットの本文が長すぎます".into());
        }
    }
    let value = snippets::to_value(&list);
    let snapshot = state
        .store
        .transact(|snapshot| {
            snapshot.snippets = value;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    read(&state)
}

/// 本文中の`{{変数名}}`。画面はこれを見て入力欄を出す。
#[tauri::command]
pub fn snippet_variables(body: String) -> Vec<String> {
    snippets::find_variables(&body)
}

/// スニペットを端末へ送る。送る内容はRust側が保存済みの記録から組み立てる（画面は番号と変数の値だけ渡す）。
#[tauri::command]
pub async fn snippet_send(
    state: State<'_, AppState>,
    id: String,
    index: usize,
    values: HashMap<String, String>,
) -> Result<(), String> {
    if values.values().any(|value| value.len() > MAX_VALUE) {
        return Err("変数の値が長すぎます".into());
    }
    let snippet = read(&state)?
        .into_iter()
        .nth(index)
        .ok_or("スニペットが見つかりません")?;
    let bytes = snippets::terminal_bytes(&snippet, &values);
    session::find(&state, &id)?.send(bytes).await
}
