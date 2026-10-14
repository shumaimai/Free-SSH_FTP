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
    master: Box<dyn MasterPty + Send>,
    input: mpsc::Sender<Vec<u8>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    child: Box<dyn Child + Send + Sync>,
    cancel: CancellationToken,
    closed: bool,
    #[cfg(windows)]
    job: Job,
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
        std::thread::spawn(move || {
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
            master: pair.master,
            input,
            writer_thread: Some(writer_thread),
            child,
            cancel,
            closed: false,
            #[cfg(windows)]
            job,
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
        self.master.resize(PtySize {
            cols: columns,
            rows,
            ..PtySize::default()
        })?;
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.cancel.cancel();
        #[cfg(unix)]
        if let Some(pid) = self.child.process_id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        unsafe {
            if windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job.0 as _, 1) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let _ = self.child.kill();
        self.child.wait()?;
        self.closed = true;
        if let Some(thread) = self.writer_thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }
}
impl Drop for LocalTerminal {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
pub type SharedLocalTerminal = Arc<Mutex<LocalTerminal>>;

#[cfg(test)]
mod tests {
    use super::*;
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
