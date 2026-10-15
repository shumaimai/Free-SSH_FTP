//! SSH端末の受信出力をプレーンテキストで追記する（Python版`sessionlog.py`相当）。
//! GUIスレッドでファイルI/Oを待たないよう、専用スレッドへ有界キューで渡す。
use anyhow::{Context, Result};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::Duration,
};

const QUEUE: usize = 256;

pub struct SessionLog {
    path: PathBuf,
    sender: Option<SyncSender<Vec<String>>>,
    done: Option<Receiver<()>>,
    error: Arc<Mutex<Option<String>>>,
}

/// Python版と同じく`\/:*?"<>|`を`_`へ置換する。制御文字と長すぎる名前も避ける。
pub fn safe_name(profile_name: &str) -> String {
    let name: String = profile_name
        .chars()
        .map(|c| {
            if r#"\/:*?"<>|"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    if name.trim().is_empty() {
        "default".into()
    } else {
        name
    }
}

pub fn file_name(profile_name: &str, now: chrono::DateTime<chrono::Local>) -> String {
    format!(
        "hashi-{}-{}.log",
        safe_name(profile_name),
        now.format("%Y%m%d-%H%M%S")
    )
}

fn open_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    // 受信出力に秘密が含まれ得るため、新規作成は本人だけが読めるmodeにする。
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

impl SessionLog {
    /// `directory`が空なら`default_dir`（各版の設定ルート/logs）を使う。
    pub fn open(directory: Option<&Path>, default_dir: &Path, profile_name: &str) -> Result<Self> {
        let directory = directory.unwrap_or(default_dir);
        std::fs::create_dir_all(directory)
            .with_context(|| format!("ログの保存先を作成できません: {}", directory.display()))?;
        let path = directory.join(file_name(profile_name, chrono::Local::now()));
        let file = open_file(&path)
            .with_context(|| format!("ログファイルを開けません: {}", path.display()))?;
        let (sender, receiver) = mpsc::sync_channel::<Vec<String>>(QUEUE);
        let (done_tx, done) = mpsc::sync_channel(1);
        let error = Arc::new(Mutex::new(None));
        let failed = error.clone();
        std::thread::Builder::new()
            .name("hashi-session-log".into())
            .spawn(move || {
                let mut output = std::io::BufWriter::new(file);
                for lines in receiver {
                    let result = lines
                        .iter()
                        .try_for_each(|line| writeln!(output, "{line}"))
                        .and_then(|()| output.flush());
                    if let Err(error) = result {
                        if let Ok(mut slot) = failed.lock() {
                            *slot =
                                Some(format!("ログの書込みに失敗したため停止しました: {error}"));
                        }
                        break;
                    }
                }
                let _ = output.flush();
                let _ = done_tx.send(());
            })
            .context("ログの書込みスレッドを開始できません")?;
        Ok(Self {
            path,
            sender: Some(sender),
            done: Some(done),
            error,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// 行を書込みスレッドへ渡す。待たずに返り、満杯/停止は理由をErrで返す。
    pub fn write_lines(&mut self, lines: Vec<String>) -> Result<(), String> {
        if let Some(error) = self.error.lock().ok().and_then(|slot| slot.clone()) {
            self.sender = None;
            return Err(error);
        }
        if lines.is_empty() {
            return Ok(());
        }
        let Some(sender) = &self.sender else {
            return Err("ログは停止しています".into());
        };
        match sender.try_send(lines) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.sender = None;
                Err("ログの書込みが追いつかないため停止しました。以降の出力は記録しません".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.sender = None;
                Err(self
                    .error
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone())
                    .unwrap_or_else(|| "ログは停止しています".into()))
            }
        }
    }
    /// 残りを書き終えるまで最大`wait`待つ。超えた場合は書込みを保証しない。
    pub fn close(&mut self, wait: Duration) -> Result<(), String> {
        self.sender = None;
        let finished = match self.done.take() {
            Some(done) => done.recv_timeout(wait).is_ok(),
            None => true,
        };
        if let Some(error) = self.error.lock().ok().and_then(|slot| slot.clone()) {
            return Err(error);
        }
        if finished {
            Ok(())
        } else {
            Err("ログの書込み完了を確認できませんでした".into())
        }
    }
}
impl Drop for SessionLog {
    fn drop(&mut self) {
        self.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_python_replacement() {
        assert_eq!(safe_name(r#"a/b\c:d*e?f"g<h>i|j"#), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(safe_name(""), "default");
        assert_eq!(safe_name("本番\n"), "本番_");
        let time =
            chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 10, 15, 9, 8, 7).unwrap();
        assert_eq!(file_name("web", time), "hashi-web-20261015-090807.log");
    }

    #[test]
    fn writes_lines_in_order_and_closes() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = SessionLog::open(None, directory.path(), "試験").unwrap();
        log.write_lines(vec!["一行目".into(), "two".into()])
            .unwrap();
        log.write_lines(vec![]).unwrap();
        log.write_lines(vec!["三".into()]).unwrap();
        let path = log.path().to_owned();
        log.close(Duration::from_secs(5)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "一行目\ntwo\n三\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(log.write_lines(vec!["after".into()]).is_err());
    }

    /// 実PTYの出力を端末モデルで確定行にし、ログへ書くまでの経路。
    #[cfg(unix)]
    #[tokio::test]
    async fn real_pty_output_is_logged_as_committed_lines() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let mut local = crate::local::LocalTerminal::spawn(
            "/bin/sh",
            &[
                "-c".into(),
                "printf '日本語\\r進捗\\rdone\\n'; printf '\\033[?1049hTUI\\n\\033[?1049l'; printf 'last\\n'".into(),
            ],
            None,
            crate::interaction::Stamp::default(),
            tx,
        )
        .unwrap();
        let mut terminal = hashi_terminal::Terminal::new(80, 24);
        terminal.set_line_log(true);
        let mut log = SessionLog::open(None, directory.path(), "pty").unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = rx.recv().await {
                match event {
                    crate::local::TerminalEvent::Data { bytes, .. } => {
                        terminal.feed(&bytes);
                        let (lines, dropped) = terminal.take_log_lines();
                        assert_eq!(dropped, 0);
                        log.write_lines(lines).unwrap();
                    }
                    crate::local::TerminalEvent::Closed { .. } => break,
                }
            }
        })
        .await
        .unwrap();
        local.close().unwrap();
        let path = log.path().to_owned();
        log.close(Duration::from_secs(5)).unwrap();
        // CRの上書きは画面表示と同じ結果（6セルの日本語に4セルのdoneを重ねる）を記録する。
        assert_eq!(std::fs::read_to_string(path).unwrap(), "done語\nlast\n");
    }

    #[test]
    fn custom_directory_is_used_and_created() {
        let directory = tempfile::tempdir().unwrap();
        let custom = directory.path().join("nested").join("logs");
        let mut log = SessionLog::open(Some(&custom), directory.path(), "x").unwrap();
        assert!(log.path().starts_with(&custom));
        log.close(Duration::from_secs(5)).unwrap();
    }
}
