use crate::interaction::Stamp;
use anyhow::{bail, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub enum TerminalEvent {
    Data { stamp: Stamp, bytes: Vec<u8> },
    Closed { stamp: Stamp },
}

pub struct LocalTerminal {
    pub stamp: Stamp,
    master: Option<Box<dyn MasterPty + Send>>,
    input: mpsc::Sender<Vec<u8>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    cancel: CancellationToken,
    closed: bool,
    child_reaped: bool,
    #[cfg(windows)]
    job: Option<Job>,
}

#[cfg(windows)]
struct Job(usize);
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
#[cfg(windows)]
fn attach_job(child: &dyn Child) -> Result<Job> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };
    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
        );
        if ok == 0
            || AssignProcessToJobObject(
                handle,
                child
                    .as_raw_handle()
                    .ok_or_else(|| anyhow::anyhow!("子プロセスのhandleがありません"))?,
            ) == 0
        {
            let error = std::io::Error::last_os_error();
            CloseHandle(handle);
            return Err(error.into());
        }
        Ok(Job(handle as usize))
    }
}

impl LocalTerminal {
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        stamp: Stamp,
        output: mpsc::Sender<TerminalEvent>,
    ) -> Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize::default())?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        let child = pair.slave.spawn_command(command)?;
        #[cfg(windows)]
        let (child, job) = {
            let mut child = child;
            let job = match attach_job(child.as_ref()) {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    return Err(error);
                }
            };
            (child, job)
        };
        let mut writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;
        drop(pair.slave);
        let cancel = CancellationToken::new();
        let (input, mut input_rx) = mpsc::channel::<Vec<u8>>(32);
        let write_cancel = cancel.clone();
        let writer_thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("入力ワーカー");
            loop {
                let bytes = runtime.block_on(async {
                    tokio::select! {
                        _ = write_cancel.cancelled() => None,
                        bytes = input_rx.recv() => bytes,
                    }
                });
                let Some(bytes) = bytes else {
                    break;
                };
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let stopped = cancel.clone();
        let id = stamp.clone();
        let reader_thread = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().build() {
                Ok(runtime) => runtime,
                Err(_) => return,
            };
            let mut buffer = vec![0; 32768];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if stopped.is_cancelled() {
                            continue;
                        }
                        let result=runtime.block_on(async{tokio::select!{
                            _=stopped.cancelled()=>None,
                            result=output.send(TerminalEvent::Data{stamp:id.clone(),bytes:buffer[..count].to_vec()})=>Some(result),
                        }});
                        if matches!(result, Some(Err(_))) {
                            break;
                        }
                    }
                }
            }
            runtime.block_on(async{tokio::select!{_=stopped.cancelled()=>{},_=output.send(TerminalEvent::Closed{stamp:id})=>{}}});
        });
        Ok(Self {
            stamp,
            master: Some(pair.master),
            input,
            writer_thread: Some(writer_thread),
            reader_thread: Some(reader_thread),
            child: Some(child),
            cancel,
            closed: false,
            child_reaped: false,
            #[cfg(windows)]
            job: Some(job),
        })
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 1024 * 1024 {
            bail!("端末入力が大きすぎます");
        }
        self.input
            .try_send(bytes.to_vec())
            .map_err(|e| anyhow::anyhow!("端末入力の送信待ち: {e}"))?;
        Ok(())
    }
    pub fn input_sender(&self) -> mpsc::Sender<Vec<u8>> {
        self.input.clone()
    }
    pub fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        if columns == 0 || rows == 0 || columns > 1024 || rows > 512 {
            bail!("端末サイズが不正です");
        }
        self.master
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ローカル端末は終了しています"))?
            .resize(PtySize {
                cols: columns,
                rows,
                ..PtySize::default()
            })?;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.cancel.cancel();
        // try_waitで回収済みならUnixのPID/PGIDが別プロセスに再利用され得る。
        #[cfg(unix)]
        if self.child_reaped {
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(group) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
        {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        #[cfg(unix)]
        if let Some(pid) = self.child.as_ref().and_then(|child| child.process_id()) {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        unsafe {
            if self.job.as_ref().is_some_and(|job| {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0 as _, 1) == 0
            }) {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        if let Some(child) = self.child.as_mut().filter(|_| !self.child_reaped) {
            let _ = child.kill();
        }
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let stopped = self.stop();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        if let Some(child) = self.child.as_mut() {
            while child.try_wait()?.is_none() {
                if std::time::Instant::now() >= deadline {
                    bail!("ローカル端末のプロセス終了を確認できません");
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            self.child_reaped = true;
        }
        // readerが排出し続けている間にConPTY/masterを閉じ、read/writeを起こしてからjoinする。
        self.master.take();
        for thread in [&self.writer_thread, &self.reader_thread]
            .into_iter()
            .flatten()
        {
            while !thread.is_finished() {
                if std::time::Instant::now() >= deadline {
                    bail!("ローカル端末の入出力ワーカーの終了を確認できません");
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let written = self.writer_thread.take().map(|thread| thread.join());
        let read = self.reader_thread.take().map(|thread| thread.join());
        self.closed = true;
        stopped?;
        if matches!(written, Some(Err(_))) || matches!(read, Some(Err(_))) {
            bail!("ローカル端末の入出力ワーカーが異常終了しました");
        }
        Ok(())
    }
}
impl Drop for LocalTerminal {
    fn drop(&mut self) {
        self.cancel.cancel();
        if self.closed {
            return;
        }
        // 強制終了のGUIスレッドでPTY/pipeの終了を待たない。通常はclose()で回収済み。
        let (master, mut child, writer, reader) = (
            self.master.take(),
            self.child.take(),
            self.writer_thread.take(),
            self.reader_thread.take(),
        );
        let child_reaped = self.child_reaped;
        #[cfg(windows)]
        let job = self.job.take();
        std::thread::spawn(move || {
            #[cfg(unix)]
            if let Some(group) = master
                .as_ref()
                .filter(|_| !child_reaped)
                .and_then(|master| master.process_group_leader())
            {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
            // WindowsのJob handleは回収済みPIDと異なり有効な所有資源。
            // shellだけ先に終了していても、残る子孫を停止してからPTYを閉じる。
            #[cfg(windows)]
            if let Some(job) = &job {
                unsafe {
                    windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0 as _, 1);
                }
            }
            if let Some(child) = child.as_mut().filter(|_| !child_reaped) {
                #[cfg(unix)]
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            }
            drop(master);
            if let Some(writer) = writer {
                let _ = writer.join();
            }
            if let Some(reader) = reader {
                let _ = reader.join();
            }
        });
    }
}
pub type SharedLocalTerminal = Arc<Mutex<LocalTerminal>>;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn escaped_pty_holder_has_bounded_close_and_nonblocking_drop() {
        struct Escaped(i32);
        impl Drop for Escaped {
            fn drop(&mut self) {
                unsafe {
                    libc::kill(-self.0, libc::SIGKILL);
                }
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("escaped.pid");
        let (tx, _rx) = mpsc::channel(1);
        let mut terminal = LocalTerminal::spawn(
            "/bin/sh",
            &[
                "-c".into(),
                "setsid /bin/sh -c 'trap \"\" HUP; echo $$ > \"$1\"; sleep 30' _ \"$1\" & wait"
                    .into(),
                "_".into(),
                pid_file.to_string_lossy().into_owned(),
            ],
            None,
            Stamp::default(),
            tx,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid.trim().parse() {
                    break pid;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "PTYを保持する子が起動しません"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let _escaped = Escaped(pid);
        let started = std::time::Instant::now();
        assert!(
            terminal.close().is_err(),
            "回収できないreaderを成功扱いしない"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        assert!(
            terminal.child_reaped,
            "I/O待ち失敗でも子の回収済み状態を保つ"
        );
        assert!(terminal.stop().is_ok(), "回収済みPIDへ再送信しない");
        let started = std::time::Instant::now();
        drop(terminal);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "Dropがjoinを待っています"
        );
    }

    #[tokio::test]
    async fn local_pty_echo_and_resize_close() {
        let (tx, mut rx) = mpsc::channel(16);
        let stamp = Stamp::default();
        let program = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let mut terminal = LocalTerminal::spawn(program, &[], None, stamp.clone(), tx).unwrap();
        terminal.resize(100, 30).unwrap();
        let mut screen = hashi_terminal::Terminal::new(100, 30);
        let mut sent = !cfg!(windows);
        if sent {
            terminal.send(b"echo HASHI_PTY_TEST\r").unwrap();
        }
        let found = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(event) = rx.recv().await {
                if let TerminalEvent::Data { stamp: id, bytes } = event {
                    assert_eq!(id, stamp);
                    screen.feed(&bytes);
                    let replies = screen.replies();
                    let handshake = !replies.is_empty();
                    for reply in replies {
                        terminal.send(&reply).unwrap();
                    }
                    // Windows ConPTYのINHERIT_CURSORはDSR応答を待ってから起動する。
                    if cfg!(windows) && !sent && handshake {
                        terminal.send(b"echo HASHI_PTY_TEST\r").unwrap();
                        sent = true;
                    }
                    if screen.visible_text().iter().any(|line| {
                        line.trim_end().ends_with("HASHI_PTY_TEST") && !line.contains("echo ")
                    }) {
                        return true;
                    }
                }
            }
            false
        })
        .await;
        assert!(
            matches!(found, Ok(true)),
            "CMD/shell実行結果: {:?}",
            screen.visible_text()
        );
        terminal.close().unwrap();
    }
}
