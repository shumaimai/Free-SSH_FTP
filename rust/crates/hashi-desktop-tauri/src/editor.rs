//! 内蔵エディタ。文書の保持・保存はhashi-coreの`Document`をそのまま使う
//! （BOM・UTF-16・改行の保持、保存前後の外部変更の照合、HEXはサイズ不変）。
//! 画面が渡すのは編集後のテキストまたはHEXの変更点だけで、保存先のパスはRust側が持つ。
use crate::{app::AppState, files::Files};
use base64::Engine;
use hashi_core::editor::{self, Body, Document, Encoding, Newline};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::State;
use tokio_util::sync::CancellationToken;

const MAX_EDITORS: usize = 32;

#[derive(Clone)]
enum Target {
    Local(PathBuf),
    Remote { session_id: String, path: String },
}

pub struct EditorEntry {
    target: Mutex<Target>,
    doc: tokio::sync::Mutex<Document>,
    dirty: AtomicBool,
}

#[derive(Default)]
pub struct Editors {
    entries: Mutex<std::collections::HashMap<String, Arc<EditorEntry>>>,
}

impl Editors {
    fn insert(&self, entry: Arc<EditorEntry>) -> Result<String, String> {
        let mut entries = self.entries.lock().map_err(|_| "状態が不正です")?;
        if entries.len() >= MAX_EDITORS {
            return Err("開いているエディタが多すぎます。閉じてから開いてください".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        entries.insert(id.clone(), entry);
        Ok(id)
    }
    fn get(&self, id: &str) -> Result<Arc<EditorEntry>, String> {
        self.entries
            .lock()
            .map_err(|_| "状態が不正です")?
            .get(id)
            .cloned()
            .ok_or_else(|| "エディタが見つかりません".into())
    }
    /// 未保存の編集があるエディタの数（ウィンドウを閉じる前の確認に使う）。
    pub fn dirty_count(&self) -> usize {
        self.entries
            .lock()
            .map(|e| {
                e.values()
                    .filter(|e| e.dirty.load(Ordering::SeqCst))
                    .count()
            })
            .unwrap_or(0)
    }
    /// 接続が終わったエディタは残す（名前を付けて保存でローカルへ逃がせる）。
    pub fn remove(&self, id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(id);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    id: String,
    name: String,
    path: String,
    session_id: Option<String>,
    /// "text" / "hex"
    kind: &'static str,
    text: Option<String>,
    /// HEXのときの全バイト（base64）。
    hex: Option<String>,
    encoding: &'static str,
    newline: &'static str,
    size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    name: String,
    path: String,
    session_id: Option<String>,
    size: usize,
}

fn encoding_label(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf8Bom => "UTF-8 (BOM付き)",
        Encoding::Utf16Le => "UTF-16 LE",
        Encoding::Utf16Be => "UTF-16 BE",
        Encoding::Latin1 => "Latin-1",
    }
}

fn newline_label(newline: Newline) -> &'static str {
    match newline {
        Newline::Lf => "LF",
        Newline::CrLf => "CRLF",
        Newline::Cr => "CR",
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

fn opened(id: String, target: &Target, document: &Document) -> Opened {
    let (path, session_id) = match target {
        Target::Local(path) => (path.to_string_lossy().into_owned(), None),
        Target::Remote { session_id, path } => (path.clone(), Some(session_id.clone())),
    };
    let (kind, text, hex, size) = match &document.body {
        Body::Text(text) => ("text", Some(text.clone()), None, document.original().len()),
        Body::Hex(bytes) => (
            "hex",
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            bytes.len(),
        ),
    };
    Opened {
        id,
        name: file_name(&path),
        path,
        session_id,
        kind,
        text,
        hex,
        encoding: encoding_label(document.encoding),
        newline: newline_label(document.newline),
        size,
    }
}

fn register(state: &AppState, target: Target, document: Document) -> Result<Opened, String> {
    let view_target = target.clone();
    let view_doc = document.clone();
    let id = state.editors.insert(Arc::new(EditorEntry {
        target: Mutex::new(target),
        doc: tokio::sync::Mutex::new(document),
        dirty: AtomicBool::new(false),
    }))?;
    Ok(opened(id, &view_target, &view_doc))
}

fn files_of(state: &AppState, session_id: &str) -> Result<Arc<Files>, String> {
    let session = state
        .sessions
        .lock()
        .map_err(|_| "セッション台帳が不正です")?
        .get(session_id)
        .cloned()
        .ok_or("接続は終了しています。名前を付けてローカルへ保存できます")?;
    session
        .files()
        .ok_or_else(|| "この接続ではファイルを使えません".to_owned())
}

#[tauri::command]
pub async fn editor_open_remote(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<Opened, String> {
    let files = files_of(&state, &session_id)?;
    let _work = files.work.start().map_err(|e| e.to_string())?;
    let cancel = CancellationToken::new();
    let (path, document) = async {
        let sftp = files.nav().lock().await;
        let path = sftp.canonicalize(&path).await?;
        let document = editor::read_remote(&sftp, &path, &cancel).await?;
        anyhow::Ok((path, document))
    }
    .await
    .map_err(|e| e.to_string())?;
    register(&state, Target::Remote { session_id, path }, document)
}

/// OSのファイル選択ダイアログで選んだローカルのファイルを開く。取り消したらNone。
#[tauri::command]
pub async fn editor_open_local(state: State<'_, AppState>) -> Result<Option<Opened>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_file())
        .await
        .map_err(|e| e.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let read = path.clone();
    let document = tauri::async_runtime::spawn_blocking(move || editor::read_local(&read))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    register(&state, Target::Local(path), document).map(Some)
}

/// 画面の編集内容（テキスト、またはHEXの[位置, 値]の変更点）を文書へ反映した複製を作る。
fn apply(
    document: &Document,
    text: Option<String>,
    patches: Option<Vec<[u32; 2]>>,
) -> Result<Document, String> {
    let mut work = document.clone();
    match (&work.body, text, patches) {
        (Body::Text(_), Some(text), _) => work.body = Body::Text(text),
        (Body::Hex(_), _, Some(patches)) => {
            for [offset, value] in patches {
                let value = u8::try_from(value).map_err(|_| "バイト値が不正です")?;
                work.overwrite_byte(offset as usize, value)
                    .map_err(|e| e.to_string())?;
            }
        }
        (Body::Hex(_), _, None) => {}
        (Body::Text(_), None, _) => return Err("編集内容がありません".into()),
    }
    Ok(work)
}

#[tauri::command]
pub async fn editor_save(
    state: State<'_, AppState>,
    id: String,
    text: Option<String>,
    patches: Option<Vec<[u32; 2]>>,
) -> Result<Saved, String> {
    let entry = state.editors.get(&id)?;
    // 同じ文書の保存は1つずつ。重なった要求は待たせずに断る。
    let mut doc = entry
        .doc
        .try_lock()
        .map_err(|_| "保存中です。終わってからもう一度試してください")?;
    let mut work = apply(&doc, text, patches)?;
    let payload = work.payload().map_err(|e| e.to_string())?;
    let original = doc.original().to_vec();
    let target = entry.target.lock().map_err(|_| "状態が不正です")?.clone();
    match &target {
        Target::Local(path) => {
            let (path, payload, original) = (path.clone(), payload.clone(), original);
            tauri::async_runtime::spawn_blocking(move || {
                editor::save_local(&path, &original, &payload)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        }
        Target::Remote { session_id, path } => {
            let files = files_of(&state, session_id)?;
            let _work = files.work.start().map_err(|e| e.to_string())?;
            let cancel = files.work_token();
            let sftp = files.nav().lock().await;
            editor::save_remote(&files.ssh, &sftp, path, &original, &payload, &cancel)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    work.mark_saved(&payload).map_err(|e| e.to_string())?;
    let size = payload.len();
    *doc = work;
    // 未保存の印はここで下ろさない。保存中に足された編集があるかどうかは画面だけが知っており、
    // 画面が保存した内容と今の内容を比べて editor_set_dirty で知らせる。
    let (path, session_id) = match &target {
        Target::Local(path) => (path.to_string_lossy().into_owned(), None),
        Target::Remote { session_id, path } => (path.clone(), Some(session_id.clone())),
    };
    Ok(Saved {
        name: file_name(&path),
        path,
        session_id,
        size,
    })
}

/// ローカルへ名前を付けて保存する。保存先はOSの保存ダイアログで決める。取り消したらNone。
#[tauri::command]
pub async fn editor_save_as(
    state: State<'_, AppState>,
    id: String,
    text: Option<String>,
    patches: Option<Vec<[u32; 2]>>,
) -> Result<Option<Saved>, String> {
    let entry = state.editors.get(&id)?;
    let mut doc = entry
        .doc
        .try_lock()
        .map_err(|_| "保存中です。終わってからもう一度試してください")?;
    let mut work = apply(&doc, text, patches)?;
    let payload = work.payload().map_err(|e| e.to_string())?;
    let picked = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().save_file())
        .await
        .map_err(|e| e.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let (target_path, bytes) = (path.clone(), payload.clone());
    tauri::async_runtime::spawn_blocking(move || editor::save_as_local(&target_path, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    // 以後の保存先は新しいローカルのファイル。次の保存の照合は書いた内容を基準にする。
    work.mark_saved(&payload).map_err(|e| e.to_string())?;
    *doc = work;
    *entry.target.lock().map_err(|_| "状態が不正です")? = Target::Local(path.clone());
    // 未保存の印は画面が比べて知らせる(editor_saveと同じ)。
    let path = path.to_string_lossy().into_owned();
    Ok(Some(Saved {
        name: file_name(&path),
        path,
        session_id: None,
        size: payload.len(),
    }))
}

#[tauri::command]
pub fn editor_set_dirty(state: State<'_, AppState>, id: String, dirty: bool) {
    if let Ok(entry) = state.editors.get(&id) {
        entry.dirty.store(dirty, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn editor_close(state: State<'_, AppState>, id: String) {
    state.editors.remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_edit_keeps_bom_and_newline() {
        let mut raw = vec![239, 187, 191];
        raw.extend_from_slice("a\r\nb\r\n".as_bytes());
        let doc = Document::from_bytes(raw).unwrap();
        let mut work = apply(&doc, Some("a\nB\n".into()), None).unwrap();
        let payload = work.payload().unwrap();
        assert_eq!(payload, [&[239u8, 187, 191][..], b"a\r\nB\r\n"].concat());
        work.mark_saved(&payload).unwrap();
        assert!(!work.dirty());
    }

    #[test]
    fn hex_patches_never_change_size() {
        let doc = Document::from_bytes(vec![0, 1, 2, 3]).unwrap();
        assert!(matches!(doc.body, Body::Hex(_)));
        let work = apply(&doc, None, Some(vec![[1, 0xff], [3, 0x10]])).unwrap();
        assert_eq!(work.payload().unwrap(), vec![0, 0xff, 2, 0x10]);
        assert!(apply(&doc, None, Some(vec![[4, 1]])).is_err());
        assert!(apply(&doc, None, Some(vec![[0, 256]])).is_err());
        // テキスト文書へのHEX変更点は無視せず、本文の無い保存も受け付けない。
        let text = Document::from_bytes(b"abc".to_vec()).unwrap();
        assert!(apply(&text, None, Some(vec![[0, 1]])).is_err());
    }

    #[test]
    fn dirty_count_and_limit() {
        let editors = Editors::default();
        let make = || {
            Arc::new(EditorEntry {
                target: Mutex::new(Target::Local(PathBuf::from("/tmp/x"))),
                doc: tokio::sync::Mutex::new(Document::from_bytes(b"x".to_vec()).unwrap()),
                dirty: AtomicBool::new(false),
            })
        };
        let first = make();
        let id = editors.insert(first.clone()).unwrap();
        assert_eq!(editors.dirty_count(), 0);
        first.dirty.store(true, Ordering::SeqCst);
        assert_eq!(editors.dirty_count(), 1);
        editors.remove(&id);
        assert_eq!(editors.dirty_count(), 0);
        for _ in 0..MAX_EDITORS {
            editors.insert(make()).unwrap();
        }
        assert!(editors.insert(make()).is_err());
    }

    #[test]
    fn display_names() {
        assert_eq!(file_name("/srv/app/config.toml"), "config.toml");
        assert_eq!(file_name(r"C:\Users\me\a.txt"), "a.txt");
    }
}
