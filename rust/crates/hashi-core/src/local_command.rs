//! 操作仲介のローカル独立実行。対話CMD/シェルのcwd・環境・TUI状態を引き継がない別プロセスで動かす。
//! Windowsは旧版と同じく外側cmdをASCIIだけで起動し、本文は環境変数経由の遅延展開で内側cmdへ渡す
//! （本文中の`%`/`!`を外側で展開しない。引用符をCRT用に再エスケープしない）。
use anyhow::{bail, Context, Result};
use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
pub struct LocalRun {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub output: Vec<u8>,
    pub error: Vec<u8>,
    /// 出力のEOFを両方確認できたか。子孫がパイプを保持すると確認できない。
    pub output_complete: bool,
}

fn command(text: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let executable = std::path::PathBuf::from(root)
            .join("System32")
            .join("cmd.exe");
        let variable = format!("HASHI_COMMAND_{}", uuid::Uuid::new_v4().simple()).to_uppercase();
        let quoted = format!("\"{}\"", executable.display());
        let mut command = tokio::process::Command::new(&executable);
        command
            .env(&variable, text)
            .raw_arg(format!(
                "/D /V:ON /S /C \"chcp 65001>nul & {quoted} /D /V:OFF /S /C \"!{variable}!\"\""
            ))
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    }
    #[cfg(unix)]
    {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg(text).process_group(0);
        command
    }
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    mut pipe: R,
    buffer: Arc<Mutex<Vec<u8>>>,
    tail: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut chunk = vec![0; 4096];
        while let Ok(count) = pipe.read(&mut chunk).await {
            if count == 0 {
                break;
            }
            if let Ok(mut buffer) = buffer.lock() {
                crate::ssh::keep_tail(&mut buffer, &chunk[..count], tail);
            }
        }
    })
}

/// `cwd`は明示した既存フォルダだけ。対話端末の現在位置を推測して使わない。
pub async fn run(
    text: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
    tail: usize,
) -> Result<LocalRun> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        bail!("独立実行には明示した開始フォルダが必要です。対話端末のcwdは引き継ぎません");
    }
    let mut child = command(text)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("ローカルコマンドを開始できません")?;
    let pid = child.id().context("プロセスIDを取得できません")?;
    let output = Arc::new(Mutex::new(vec![]));
    let error = Arc::new(Mutex::new(vec![]));
    let mut readers = vec![
        reader(child.stdout.take().unwrap(), output.clone(), tail),
        reader(child.stderr.take().unwrap(), error.clone(), tail),
    ];
    let deadline = tokio::time::Instant::now() + timeout;
    let mut run = LocalRun::default();
    tokio::select! {
        biased;
        _ = cancel.cancelled() => run.status = "cancelled",
        _ = tokio::time::sleep_until(deadline) => run.status = "timeout",
        status = child.wait() => {
            let status = status?;
            run.status = "completed";
            run.exit_code = status.code();
        }
    }
    if run.status != "completed" {
        kill_tree(pid);
        let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
    }
    // 子孫が出力パイプを保持していてもEOF待ちで停止を塞がない。
    let drained = tokio::time::timeout(Duration::from_millis(500), async {
        for handle in &mut readers {
            let _ = handle.await;
        }
    })
    .await
    .is_ok();
    for handle in &readers {
        handle.abort();
    }
    run.output_complete = drained && run.status == "completed";
    run.output = std::mem::take(&mut *output.lock().unwrap());
    run.error = std::mem::take(&mut *error.lock().unwrap());
    Ok(run)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn completes_with_exit_code_and_separate_streams() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let done = super::run(
            "pwd; echo 日本語 >&2; exit 3",
            directory.path(),
            Duration::from_secs(10),
            &cancel,
            65536,
        )
        .await
        .unwrap();
        assert_eq!((done.status, done.exit_code), ("completed", Some(3)));
        assert!(done.output_complete);
        assert_eq!(
            String::from_utf8(done.output).unwrap().trim(),
            directory.path().canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(String::from_utf8(done.error).unwrap(), "日本語\n");
    }

    #[tokio::test]
    async fn timeout_cancel_tail_and_cwd_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let started = std::time::Instant::now();
        let done = super::run(
            "echo begin; sleep 30",
            directory.path(),
            Duration::from_millis(300),
            &cancel,
            64,
        )
        .await
        .unwrap();
        assert_eq!(done.status, "timeout");
        assert!(!done.output_complete && done.exit_code.is_none());
        assert_eq!(done.output, b"begin\n");
        assert!(started.elapsed() < Duration::from_secs(5));
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            trigger.cancel();
        });
        let done = super::run(
            "sleep 30",
            directory.path(),
            Duration::from_secs(30),
            &cancel,
            64,
        )
        .await
        .unwrap();
        assert_eq!(done.status, "cancelled");
        let long = super::run(
            "yes x | head -c 10000",
            directory.path(),
            Duration::from_secs(10),
            &CancellationToken::new(),
            64,
        )
        .await
        .unwrap();
        assert_eq!(long.output.len(), 64);
        assert!(super::run(
            "true",
            Path::new("relative"),
            Duration::from_secs(1),
            &cancel,
            64
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn detached_child_holding_pipe_does_not_block() {
        let directory = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let done = super::run(
            "(sleep 5 &) ; echo done",
            directory.path(),
            Duration::from_secs(10),
            &CancellationToken::new(),
            1024,
        )
        .await
        .unwrap();
        assert_eq!(done.status, "completed");
        assert!(!done.output_complete, "EOF未確認を完了扱いしない");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
