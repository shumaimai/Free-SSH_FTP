//! 新しい仕事の受付を閉じ、子のcleanupも終わるまで待つ。
use anyhow::{bail, Result};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::{runtime::Handle, task::JoinHandle};
use tokio_util::task::TaskTracker;

#[derive(Clone, Default)]
pub struct Work(Arc<Inner>);
#[derive(Default)]
struct Inner {
    closing: Mutex<bool>,
    tracker: TaskTracker,
    errors: Mutex<Vec<String>>,
}
impl Work {
    pub fn start(&self) -> Result<tokio_util::task::task_tracker::TaskTrackerToken> {
        let closing = self
            .0
            .closing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *closing {
            bail!("終了処理中のため新しい仕事を開始できません");
        }
        Ok(self.0.tracker.token())
    }
    pub fn spawn<F>(&self, handle: &Handle, future: F) -> Result<JoinHandle<F::Output>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let closing = self
            .0
            .closing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *closing {
            bail!("終了処理中のため新しい仕事を開始できません");
        }
        Ok(self.0.tracker.spawn_on(future, handle))
    }
    pub fn spawn_blocking<F, T>(&self, handle: &Handle, job: F) -> Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let closing = self
            .0
            .closing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *closing {
            bail!("終了処理中のため新しい仕事を開始できません");
        }
        Ok(self.0.tracker.spawn_blocking_on(job, handle))
    }
    /// 受付終了後も、すでに開始した仕事から生まれる復元/cleanupは受け付ける。
    pub fn cleanup<F>(&self, handle: &Handle, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.0.tracker.spawn_on(future, handle)
    }
    pub fn close(&self) {
        let mut closing = self
            .0
            .closing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *closing = true;
        self.0.tracker.close();
    }
    pub fn is_closed(&self) -> bool {
        *self
            .0
            .closing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub fn is_finished(&self) -> bool {
        self.0.tracker.is_closed() && self.0.tracker.is_empty()
    }
    pub async fn wait(&self) {
        self.0.tracker.wait().await;
    }
    pub fn report(&self, error: String) {
        self.0
            .errors
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(error);
    }
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .0
                .errors
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn closure_refuses_new_work_and_waits_for_descendant_cleanup() {
        let work = Work::default();
        let handle = Handle::current();
        let (started, start) = tokio::sync::oneshot::channel();
        let (finish, finished) = tokio::sync::oneshot::channel();
        let child = work.clone();
        work.spawn(&handle, async move {
            start.await.unwrap();
            child.cleanup(&Handle::current(), async move {
                finished.await.unwrap();
            });
        })
        .unwrap();
        work.close();
        assert!(work.spawn(&handle, async {}).is_err());
        assert!(work.start().is_err());
        started.send(()).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), work.wait())
                .await
                .is_err()
        );
        finish.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), work.wait())
            .await
            .unwrap();
    }
}
