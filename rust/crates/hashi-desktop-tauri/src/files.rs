//! リモートのファイル操作（SFTP）。一覧・転送・削除・検索はhashi-coreの規則をそのまま使う。
//! 転送はキューに積み、同じ接続では1件ずつ、全体で4件まで走らせる。
//! 画面が渡すのは接続のidとリモートのパスだけで、ローカルのパスはRust側が選択ダイアログ/ドロップから得る。
use crate::{app::AppState, session::Session};
use hashi_core::{
    interaction::Stamp,
    permissions::{self, Manager as PermManager},
    ssh::SshSession,
    transfer::{self, Entry, Mode, Progress},
    transfer_tree::{self, Existing, TaskContext},
    work::Work,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{async_runtime::RuntimeHandle, AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub type Sftp = Arc<tokio::sync::Mutex<russh_sftp::client::SftpSession>>;

const MAX_JOBS: usize = 128;
const MAX_RUNNING: usize = 4;

/// 1つのSSH接続のファイル操作。nav（一覧・削除・検索）とxfer（転送）は別のSFTPチャネル。
pub struct Files {
    pub ssh: Arc<SshSession>,
    nav: Sftp,
    xfer: Sftp,
    pub work: Work,
    work_cancel: CancellationToken,
    navigation: Mutex<CancellationToken>,
    initial_path: String,
    bookmark_key: String,
    /// 権限補助（利用者が接続ごとに有効にしたときだけ入る）。
    permissions: Mutex<Option<Arc<PermManager>>>,
    permissions_busy: std::sync::atomic::AtomicBool,
}

impl Files {
    pub fn new(
        ssh: Arc<SshSession>,
        nav: russh_sftp::client::SftpSession,
        xfer: russh_sftp::client::SftpSession,
        session_cancel: &CancellationToken,
        initial_path: String,
        bookmark_key: String,
    ) -> Self {
        Self {
            ssh,
            nav: Arc::new(tokio::sync::Mutex::new(nav)),
            xfer: Arc::new(tokio::sync::Mutex::new(xfer)),
            work: Work::default(),
            work_cancel: session_cancel.child_token(),
            navigation: Mutex::new(CancellationToken::new()),
            initial_path,
            bookmark_key,
            permissions: Mutex::new(None),
            permissions_busy: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn permissions(&self) -> Option<Arc<PermManager>> {
        self.permissions.lock().ok().and_then(|p| p.clone())
    }

    /// 新しい受付を閉じ、進行中の操作（確定ACK・復元を含む）の終了を待つ。待ちには上限を置く。
    /// 権限補助の復元が済んでいない場合などは、利用者へ伝える警告を返す（復旧記録は残す）。
    pub async fn shutdown(&self) -> Vec<String> {
        self.work_cancel.cancel();
        if let Ok(token) = self.navigation.lock() {
            token.cancel();
        }
        self.work.close();
        let mut warnings = vec![];
        if tokio::time::timeout(Duration::from_secs(30), self.work.wait())
            .await
            .is_err()
        {
            warnings.push("ファイル操作の終了を30秒待ちましたが完了を確認できません".to_owned());
        }
        if let Some(manager) = self.permissions() {
            match manager.shutdown().await {
                Ok(0) => {}
                Ok(pending) => warnings.push(format!(
                    "{pending}件の権限復元が未完了です。復旧記録を保持しています（次回、この接続で復元できます）"
                )),
                Err(error) => warnings.push(format!(
                    "権限復元の確認が未完了です。復旧記録を保持: {error}"
                )),
            }
        }
        warnings
    }

    /// 公開鍵を接続先のauthorized_keysへ追記する（hashi-coreの規則）。登録済みならfalse。
    pub async fn register_public_key(&self, public_line: &str) -> anyhow::Result<bool> {
        let nav = self.nav.lock().await;
        hashi_core::keygen::register_public_key(&self.ssh, &nav, public_line).await
    }

    /// 一覧・削除・エディタ用のSFTPチャネル（転送のxferとは別）。
    pub fn nav(&self) -> &Sftp {
        &self.nav
    }

    /// 接続の終了で取り消されるtoken（エディタの読み書きに渡す）。
    pub fn work_token(&self) -> CancellationToken {
        self.work_cancel.child_token()
    }

    /// 古い一覧・検索を取り消し、新しい要求のtokenを返す。取消済みtokenは次の要求へ流用しない。
    fn begin_navigation(&self) -> CancellationToken {
        let token = self.work_cancel.child_token();
        if let Ok(mut current) = self.navigation.lock() {
            current.cancel();
            *current = token.clone();
        }
        token
    }
}

#[derive(Clone)]
pub enum Operation {
    Upload {
        local: PathBuf,
        remote: String,
    },
    Download {
        remote: String,
        local: PathBuf,
    },
    Delete {
        remote: String,
        confirmation: String,
    },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    id: String,
    session_id: String,
    label: String,
    /// queued / running / done / failed / cancelled
    phase: &'static str,
    done: u64,
    total: u64,
    message: String,
}

struct Job {
    view: JobView,
    operation: Operation,
    policy: Existing,
    mode: Mode,
    cancel: CancellationToken,
}

pub struct FileJobs {
    jobs: Mutex<Vec<Job>>,
    progress_tx: mpsc::Sender<Progress>,
    progress_rx: Mutex<Option<mpsc::Receiver<Progress>>>,
    /// ウィンドウへドロップされたローカルのパス。画面へは件数だけ伝え、パスは渡さない。
    dropped: Mutex<Vec<PathBuf>>,
}

impl FileJobs {
    pub fn new() -> Self {
        let (progress_tx, progress_rx) = mpsc::channel(256);
        Self {
            jobs: Mutex::new(vec![]),
            progress_tx,
            progress_rx: Mutex::new(Some(progress_rx)),
            dropped: Mutex::new(vec![]),
        }
    }

    pub fn set_dropped(&self, paths: Vec<PathBuf>) -> usize {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| p.is_absolute())
            .take(256)
            .collect();
        let count = paths.len();
        if let Ok(mut dropped) = self.dropped.lock() {
            *dropped = paths;
        }
        count
    }

    /// 接続が終わったので、その接続の待機中・実行中の転送を取り消す。
    pub fn cancel_session(&self, session_id: &str) {
        if let Ok(jobs) = self.jobs.lock() {
            for job in jobs.iter().filter(|j| j.view.session_id == session_id) {
                job.cancel.cancel();
            }
        }
    }
}

/// 進捗を画面へ流す（100msごとにまとめる）。
pub fn start_pumps(app: AppHandle) {
    let state = app.state::<AppState>();
    let Some(mut progress) = state
        .files
        .progress_rx
        .lock()
        .ok()
        .and_then(|mut r| r.take())
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<String, Instant> = HashMap::new();
        while let Some(update) = progress.recv().await {
            let state = app.state::<AppState>();
            let view = {
                let Ok(mut jobs) = state.files.jobs.lock() else {
                    continue;
                };
                let Some(job) = jobs.iter_mut().find(|j| j.view.id == update.job_id) else {
                    continue;
                };
                job.view.done = update.done;
                job.view.total = update.total;
                job.view.clone()
            };
            let due = last
                .get(&view.id)
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
            if due || view.done >= view.total {
                last.insert(view.id.clone(), Instant::now());
                let _ = app.emit("file-job", &view);
            }
            if last.len() > 512 {
                last.clear();
            }
        }
    });
}

pub fn parse_policy(policy: &str) -> Result<Existing, String> {
    match policy {
        "refuse" => Ok(Existing::Refuse),
        "overwrite" => Ok(Existing::Overwrite),
        "resume" => Ok(Existing::Resume),
        "skip" => Ok(Existing::Skip),
        _ => Err("既存ファイルの扱いが不正です".into()),
    }
}

pub fn parse_mode(mode: &str) -> Result<Mode, String> {
    match mode {
        "atomic" => Ok(Mode::Atomic),
        "in_place" => Ok(Mode::InPlace),
        _ => Err("転送方式が不正です".into()),
    }
}

pub fn files_of(state: &AppState, id: &str) -> Result<(Arc<Session>, Arc<Files>), String> {
    let session = state
        .sessions
        .lock()
        .map_err(|_| "セッション台帳が不正です")?
        .get(id)
        .cloned()
        .ok_or("セッションが見つかりません")?;
    let files = session.files().ok_or("この接続ではファイルを使えません")?;
    Ok((session, files))
}

#[derive(Serialize)]
pub struct Listing {
    path: String,
    entries: Vec<Entry>,
    /// 同期ブラウズが有効なとき、ローカル側を追随させる指示（絶対パスは写さず相対移動だけ）。
    follow: Option<crate::local::FollowView>,
}

/// 一覧。pathが空なら開始フォルダ（未設定ならホーム）。新しい一覧要求は古い要求を取り消す。
#[tauri::command]
pub async fn files_list(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
    follow: Option<bool>,
) -> Result<Listing, String> {
    let (_, files) = files_of(&state, &id)?;
    let _work = files.work.start().map_err(|e| e.to_string())?;
    let token = files.begin_navigation();
    let target = match path.filter(|p| !p.is_empty()) {
        Some(path) => path,
        None if files.initial_path.is_empty() => ".".to_owned(),
        None => files.initial_path.clone(),
    };
    // 同期ブラウズの追随で開始した要求だけ、結果の跳ね返りを読み飛ばす（要求ごとのStampで判定）。
    let navigation = Stamp::default();
    let tab = crate::local::remote_stamp(&id);
    if follow.unwrap_or(false) {
        state.local.expect_remote(navigation.clone());
    }
    let result = async {
        let nav = files.nav.lock().await;
        let path = nav.canonicalize(target).await?;
        // 権限補助が有効なら、権限不足のときだけ一時的に緩めて読み、終わったら戻す。
        let entries = permissions::with_access(files.permissions().as_ref(), &path, 0o500, || {
            transfer::list_bounded(&files.ssh, &path, &token)
        })
        .await?;
        anyhow::Ok((path, entries))
    }
    .await;
    let (path, entries) = match result {
        Ok(listing) => listing,
        Err(error) => {
            let message = error.to_string();
            // 追随の失敗はエラーにせず、理由を添えた通知にする（左右でフォルダ構成が違うのは当たり前）。
            return Err(state
                .local
                .remote_failed(&navigation, &message)
                .unwrap_or(message));
        }
    };
    if token.is_cancelled() {
        return Err("別の一覧要求に置き換えられました".into());
    }
    let follow = state.local.on_remote(&tab, &navigation, &path);
    Ok(Listing {
        path,
        entries,
        follow,
    })
}

#[tauri::command]
pub async fn files_search(
    state: State<'_, AppState>,
    id: String,
    root: String,
    query: String,
) -> Result<Vec<Entry>, String> {
    let (_, files) = files_of(&state, &id)?;
    let _work = files.work.start().map_err(|e| e.to_string())?;
    let token = files.begin_navigation();
    let nav = files.nav.lock().await;
    transfer_tree::search(&files.ssh, &nav, &root, &query, &token)
        .await
        .map_err(|e| e.to_string())
}

pub fn enqueue(
    app: &AppHandle,
    state: &AppState,
    session_id: &str,
    label: String,
    operation: Operation,
    policy: Existing,
    mode: Mode,
) -> Result<(), String> {
    let (session, files) = files_of(state, session_id)?;
    if files.work.is_closed() || session.is_closing() {
        return Err("終了処理中です".into());
    }
    {
        let mut jobs = state.files.jobs.lock().map_err(|_| "状態が不正です")?;
        if jobs.len() >= MAX_JOBS {
            jobs.retain(|job| matches!(job.view.phase, "queued" | "running"));
        }
        if jobs.len() >= MAX_JOBS {
            return Err("転送待ち行列が満杯です".into());
        }
        let view = JobView {
            id: Stamp::default().terminal_id,
            session_id: session_id.to_owned(),
            label,
            phase: "queued",
            done: 0,
            total: 0,
            message: String::new(),
        };
        let _ = app.emit("file-job", &view);
        jobs.push(Job {
            view,
            operation,
            policy,
            mode,
            cancel: files.work_cancel.child_token(),
        });
    }
    schedule(app);
    Ok(())
}

/// 待機中の転送を、同じ接続で1件・全体で4件までの範囲で開始する。
fn schedule(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut start: Vec<(String, String, Operation, Existing, Mode, CancellationToken)> = vec![];
    {
        let Ok(mut jobs) = state.files.jobs.lock() else {
            return;
        };
        for index in 0..jobs.len() {
            if jobs[index].view.phase != "queued" {
                continue;
            }
            if jobs[index].cancel.is_cancelled() {
                jobs[index].view.phase = "cancelled";
                let _ = app.emit("file-job", &jobs[index].view);
                continue;
            }
            let session_id = jobs[index].view.session_id.clone();
            if jobs
                .iter()
                .any(|j| j.view.phase == "running" && j.view.session_id == session_id)
                || jobs.iter().filter(|j| j.view.phase == "running").count() >= MAX_RUNNING
            {
                continue;
            }
            jobs[index].view.phase = "running";
            let _ = app.emit("file-job", &jobs[index].view);
            start.push((
                jobs[index].view.id.clone(),
                session_id,
                jobs[index].operation.clone(),
                jobs[index].policy,
                jobs[index].mode,
                jobs[index].cancel.clone(),
            ));
        }
    }
    for (job_id, session_id, operation, policy, mode, cancel) in start {
        let Ok((_, files)) = files_of(&state, &session_id) else {
            finish(
                app,
                &job_id,
                Err("接続が終了しています".into()),
                &cancel,
                &session_id,
            );
            continue;
        };
        let task_app = app.clone();
        let progress = state.files.progress_tx.clone();
        let RuntimeHandle::Tokio(handle) = tauri::async_runtime::handle();
        let task_files = files.clone();
        let permissions = files.permissions();
        let spawned = files.work.spawn(&handle, async move {
            let context = TaskContext {
                job: &job_id,
                progress: &progress,
                cancel: &cancel,
                // 有効にした接続だけ。無効の間は何も緩めない。
                permission: permissions.as_ref(),
                mode,
            };
            let result = match operation {
                Operation::Upload { local, remote } => {
                    let sftp = task_files.xfer.lock().await;
                    transfer_tree::upload_tree(
                        &task_files.ssh,
                        &sftp,
                        &local,
                        &remote,
                        policy,
                        &context,
                    )
                    .await
                    .map(|report| {
                        format!(
                            "{}ファイル / {}フォルダ / {}スキップ",
                            report.files, report.directories, report.skipped
                        )
                    })
                }
                Operation::Download { remote, local } => {
                    let sftp = task_files.xfer.lock().await;
                    transfer_tree::download_tree(
                        &task_files.ssh,
                        &sftp,
                        &remote,
                        &local,
                        policy,
                        &context,
                    )
                    .await
                    .map(|report| {
                        format!(
                            "{}ファイル / {}フォルダ / {}スキップ",
                            report.files, report.directories, report.skipped
                        )
                    })
                }
                Operation::Delete {
                    remote,
                    confirmation,
                } => {
                    let sftp = task_files.nav.lock().await;
                    transfer::delete_tree(&sftp, &remote, &confirmation, &cancel)
                        .await
                        .map(|_| "削除しました".to_owned())
                }
            };
            // tokenは各操作へ渡してある。確定ACK/復元中のFutureを取消で捨てない。
            finish(
                &task_app,
                &job_id,
                result.map_err(|e| e.to_string()),
                &cancel,
                &session_id,
            );
        });
        if spawned.is_err() {
            // 終了処理中で受け付けられなかった。
        }
    }
}

fn finish(
    app: &AppHandle,
    job_id: &str,
    result: Result<String, String>,
    cancel: &CancellationToken,
    session_id: &str,
) {
    let state = app.state::<AppState>();
    let mut changed = false;
    if let Ok(mut jobs) = state.files.jobs.lock() {
        if let Some(job) = jobs.iter_mut().find(|j| j.view.id == job_id) {
            match result {
                Ok(message) => {
                    job.view.phase = "done";
                    job.view.message = message;
                    changed = true;
                }
                Err(error) => {
                    job.view.phase = if cancel.is_cancelled() {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    job.view.message = error;
                }
            }
            let _ = app.emit("file-job", &job.view);
        }
    }
    if changed {
        let _ = app.emit("files-changed", session_id);
        // 受信でローカルのフォルダが変わったので、ローカルペインも読み直す。
        let _ = app.emit("local-changed", ());
    }
    schedule(app);
}

#[tauri::command]
pub fn files_jobs(state: State<'_, AppState>) -> Vec<JobView> {
    state
        .files
        .jobs
        .lock()
        .map(|jobs| jobs.iter().map(|j| j.view.clone()).collect())
        .unwrap_or_default()
}

#[tauri::command]
pub fn files_cancel(app: AppHandle, state: State<'_, AppState>, job_id: String) {
    if let Ok(mut jobs) = state.files.jobs.lock() {
        if let Some(job) = jobs.iter_mut().find(|j| j.view.id == job_id) {
            job.cancel.cancel();
            if job.view.phase == "queued" {
                job.view.phase = "cancelled";
                let _ = app.emit("file-job", &job.view);
            }
        }
    }
}

/// 終わった（完了・失敗・取消済み）転送を一覧から外す。
#[tauri::command]
pub fn files_clear_jobs(state: State<'_, AppState>) {
    if let Ok(mut jobs) = state.files.jobs.lock() {
        jobs.retain(|job| matches!(job.view.phase, "queued" | "running"));
    }
}

pub fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn remote_join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// 送信: OSのダイアログで選んだファイル/フォルダを、リモートのフォルダへ送る。
#[tauri::command]
pub async fn files_upload_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_dir: String,
    folder: bool,
    policy: String,
    mode: String,
) -> Result<(), String> {
    let (policy, mode) = (parse_policy(&policy)?, parse_mode(&mode)?);
    files_of(&state, &id)?;
    let picked = tokio::task::spawn_blocking(move || {
        if folder {
            rfd::FileDialog::new().pick_folder()
        } else {
            rfd::FileDialog::new().pick_file()
        }
    })
    .await
    .map_err(|_| "ファイル選択に失敗しました".to_owned())?;
    let Some(local) = picked else { return Ok(()) };
    let name = file_name(&local);
    let remote = remote_join(&remote_dir, &name);
    enqueue(
        &app,
        &state,
        &id,
        format!("送信 {name}"),
        Operation::Upload { local, remote },
        policy,
        mode,
    )
}

/// ウィンドウへドロップされたファイル/フォルダを、リモートのフォルダへ送る。
#[tauri::command]
pub fn files_upload_dropped(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_dir: String,
    policy: String,
    mode: String,
) -> Result<usize, String> {
    let (policy, mode) = (parse_policy(&policy)?, parse_mode(&mode)?);
    let paths = state
        .files
        .dropped
        .lock()
        .map_err(|_| "状態が不正です")?
        .drain(..)
        .collect::<Vec<_>>();
    let mut queued = 0;
    for local in paths {
        let name = file_name(&local);
        if name.is_empty() {
            continue;
        }
        let remote = remote_join(&remote_dir, &name);
        enqueue(
            &app,
            &state,
            &id,
            format!("送信 {name}"),
            Operation::Upload { local, remote },
            policy,
            mode,
        )?;
        queued += 1;
    }
    Ok(queued)
}

/// 受信: 保存先はOSのダイアログで選ぶ（フォルダはその中に同名で作る）。
// commandの引数は画面の呼び出しと1対1に対応させる（束ねると画面側の型も増える）。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn files_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_path: String,
    name: String,
    directory: bool,
    policy: String,
    mode: String,
) -> Result<(), String> {
    let (policy, mode) = (parse_policy(&policy)?, parse_mode(&mode)?);
    files_of(&state, &id)?;
    if name.is_empty() || name.contains(['/', '\\']) || name == ".." || name == "." {
        return Err("ファイル名が不正です".into());
    }
    let label_name = name.clone();
    let local = tokio::task::spawn_blocking(move || {
        if directory {
            rfd::FileDialog::new()
                .pick_folder()
                .map(|dir| dir.join(&name))
        } else {
            rfd::FileDialog::new().set_file_name(&name).save_file()
        }
    })
    .await
    .map_err(|_| "保存先の選択に失敗しました".to_owned())?;
    let Some(local) = local else { return Ok(()) };
    enqueue(
        &app,
        &state,
        &id,
        format!("受信 {label_name}"),
        Operation::Download {
            remote: remote_path,
            local,
        },
        policy,
        mode,
    )
}

/// 削除。再帰削除は、画面で `delete` と入力された場合だけ（確認文字列はhashi-coreが照合する）。
#[tauri::command]
pub fn files_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_path: String,
    name: String,
    confirmation: String,
) -> Result<(), String> {
    enqueue(
        &app,
        &state,
        &id,
        format!("削除 {name}"),
        Operation::Delete {
            remote: remote_path,
            confirmation,
        },
        Existing::Refuse,
        Mode::Atomic,
    )
}

fn bookmarks_of(state: &AppState, key: &str) -> Vec<String> {
    state
        .snapshot
        .lock()
        .ok()
        .and_then(|snapshot| {
            snapshot
                .settings
                .get("bookmarks")
                .and_then(|v| v.get(key))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn files_bookmarks(state: State<'_, AppState>, id: String) -> Result<Vec<String>, String> {
    let (_, files) = files_of(&state, &id)?;
    Ok(bookmarks_of(&state, &files.bookmark_key))
}

#[tauri::command]
pub fn files_bookmark_add(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<Vec<String>, String> {
    state.writable()?;
    let (_, files) = files_of(&state, &id)?;
    if path.is_empty() || path.len() > 4096 {
        return Err("ブックマークのパスが不正です".into());
    }
    let key = files.bookmark_key.clone();
    let snapshot = state
        .store
        .transact(|snapshot| {
            let bookmarks = snapshot
                .settings
                .entry("bookmarks")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or_else(|| std::io::Error::other("ブックマークの形式が不正です"))?;
            let entries = bookmarks
                .entry(key)
                .or_insert_with(|| serde_json::json!([]))
                .as_array_mut()
                .ok_or_else(|| std::io::Error::other("ブックマークの形式が不正です"))?;
            if !entries.iter().any(|value| value.as_str() == Some(&path)) {
                if entries.len() >= 128 {
                    return Err(std::io::Error::other("ブックマークは128件までです"));
                }
                entries.push(serde_json::json!(path));
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    *state.snapshot.lock().map_err(|_| "状態が不正です")? = snapshot;
    Ok(bookmarks_of(&state, &files.bookmark_key))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    enabled: bool,
    busy: bool,
    note: Option<String>,
}

#[tauri::command]
pub fn permissions_status(
    state: State<'_, AppState>,
    id: String,
) -> Result<PermissionStatus, String> {
    let (_, files) = files_of(&state, &id)?;
    let manager = files.permissions();
    Ok(PermissionStatus {
        enabled: manager.is_some(),
        busy: files
            .permissions_busy
            .load(std::sync::atomic::Ordering::SeqCst),
        note: manager.and_then(|m| m.migration_note()),
    })
}

/// 権限補助をこの接続で有効にする。旧Python版の停止確認と、旧版の未復元記録が無いことが前提。
#[tauri::command]
pub async fn permissions_enable(
    state: State<'_, AppState>,
    id: String,
    legacy_stopped: bool,
) -> Result<PermissionStatus, String> {
    use std::sync::atomic::Ordering;
    let (_, files) = files_of(&state, &id)?;
    if !legacy_stopped {
        return Err("権限操作の前に、旧Python版をすべて終了したことの確認が必要です".into());
    }
    let _work = files.work.start().map_err(|e| e.to_string())?;
    if files.permissions().is_some() {
        return Err("この接続ではすでに有効です".into());
    }
    if files.permissions_busy.swap(true, Ordering::SeqCst) {
        return Err("処理中です".into());
    }
    let result = async {
        let key = files
            .ssh
            .accepted
            .lock()
            .map_err(|_| anyhow::anyhow!("確認済み鍵を読めません"))?
            .get(&format!(
                "{}:{}",
                files.ssh.scope.host, files.ssh.scope.port
            ))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("接続先の確認済み鍵がありません"))?;
        // username と検証済み鍵まで束縛し、異なる相手の復元記録を適用しない。
        let connection = serde_json::to_string(&(
            files.ssh.scope.host.as_str(),
            files.ssh.scope.port,
            files.ssh.scope.username.as_str(),
            key.fingerprint,
        ))?;
        let backend = Arc::new(permissions::SftpBackend::new(files.ssh.sftp().await?));
        PermManager::scoped(backend, connection, true)
    }
    .await;
    files.permissions_busy.store(false, Ordering::SeqCst);
    let manager = result.map_err(|e| e.to_string())?;
    let note = manager.migration_note();
    *files.permissions.lock().map_err(|_| "状態が不正です")? = Some(manager);
    Ok(PermissionStatus {
        enabled: true,
        busy: false,
        note,
    })
}

/// この接続の未復元記録を元の権限へ戻す。
#[tauri::command]
pub async fn permissions_recover(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let (_, files) = files_of(&state, &id)?;
    let manager = files.permissions().ok_or("権限補助が有効ではありません")?;
    let _work = files.work.start().map_err(|e| e.to_string())?;
    match manager.recover().await {
        Ok((restored, 0)) => Ok(format!("権限復旧: {restored}件復元しました")),
        Ok((restored, pending)) => Err(format!(
            "権限復旧: {restored}件復元 / {pending}件未完了（記録は保持しています）"
        )),
        Err(error) => Err(format!("権限復旧できませんでした。記録を保持: {error}")),
    }
}

/// 受信: ローカルペインで開いているフォルダへ直接置く（保存先はRust側が持つフォルダだけ）。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn files_download_here(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_path: String,
    name: String,
    policy: String,
    mode: String,
) -> Result<(), String> {
    let (policy, mode) = (parse_policy(&policy)?, parse_mode(&mode)?);
    files_of(&state, &id)?;
    transfer::safe_name(&name).map_err(|e| e.to_string())?;
    let local = state
        .local
        .directory()
        .ok_or("ローカルのフォルダがまだ開かれていません")?
        .join(&name);
    enqueue(
        &app,
        &state,
        &id,
        format!("受信 {name}"),
        Operation::Download {
            remote: remote_path,
            local,
        },
        policy,
        mode,
    )
}
