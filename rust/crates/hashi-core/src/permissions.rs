//! nav/xferで共有する参照数付き権限管理。専用backendを直列化し、未復元記録を保つ。
use crate::permission_journal::{Entry, Journal, PathLease};
use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub trait Backend: Send + Sync {
    fn canonical_path<'a>(&'a self, path: &'a str) -> BackendFuture<'a, String>;
    fn mode<'a>(&'a self, path: &'a str) -> BackendFuture<'a, u32>;
    fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> BackendFuture<'a, ()>;
}
struct Active {
    original: u32,
    added: u32,
    count: usize,
    entry: Option<Entry>,
    lease: PathLease,
}
pub struct Manager {
    work: crate::work::Work,
    backend: Arc<dyn Backend>,
    journal: Arc<Journal>,
    connection: String,
    owner: String,
    active: Mutex<BTreeMap<String, Active>>,
    legacy_guard: Option<Arc<hashi_storage::migration::MigrationLease>>,
}
fn operation_gate() -> Arc<Mutex<()>> {
    static GATE: std::sync::OnceLock<Arc<Mutex<()>>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}
pub struct Grant {
    manager: Arc<Manager>,
    path: String,
    released: bool,
}
impl Manager {
    pub fn new(backend: Arc<dyn Backend>, journal: Arc<Journal>, connection: String) -> Arc<Self> {
        Arc::new(Self {
            work: crate::work::Work::default(),
            backend,
            journal,
            connection,
            owner: uuid::Uuid::new_v4().to_string(),
            active: Mutex::new(BTreeMap::new()),
            legacy_guard: None,
        })
    }
    pub fn scoped(
        backend: Arc<dyn Backend>,
        connection: String,
        legacy_stopped_confirmed: bool,
    ) -> Result<Arc<Self>> {
        if !legacy_stopped_confirmed {
            bail!("権限操作の前に旧Python版の停止確認が必要です");
        }
        static GUARD: std::sync::OnceLock<
            std::sync::Mutex<std::sync::Weak<hashi_storage::migration::MigrationLease>>,
        > = std::sync::OnceLock::new();
        let runtime = hashi_storage::paths::runtime_root()?;
        let mut guard = GUARD
            .get_or_init(|| std::sync::Mutex::new(std::sync::Weak::new()))
            .lock()
            .map_err(|_| anyhow::anyhow!("旧版停止ガードが不正です"))?;
        let lease = match guard.upgrade() {
            Some(lease) => lease,
            None => {
                let lease = Arc::new(hashi_storage::migration::acquire(&runtime)?);
                *guard = Arc::downgrade(&lease);
                lease
            }
        };
        let journal = Arc::new(Journal::at(
            hashi_storage::paths::permission_journal_root()?
        )?);
        Ok(Arc::new(Self {
            work: crate::work::Work::default(),
            backend,
            journal,
            connection,
            owner: uuid::Uuid::new_v4().to_string(),
            active: Mutex::new(BTreeMap::new()),
            legacy_guard: Some(lease),
        }))
    }
    pub async fn shutdown(&self) -> Result<usize> {
        self.work.close();
        self.work.wait().await;
        if !self.active.lock().await.is_empty() {
            bail!("使用中の権限参照が残っています。復旧記録を保持しています");
        }
        Ok(self.journal.pending(&self.connection)?.len())
    }
    pub fn coordinated(&self) -> bool {
        self.legacy_guard.is_some()
    }
    async fn join_existing(
        self: &Arc<Self>,
        path: &str,
        excluded: &std::collections::BTreeSet<String>,
    ) -> Vec<Grant> {
        let mut active = self.active.lock().await;
        let mut grants = vec![];
        for (target, record) in active.iter_mut() {
            if !excluded.contains(target)
                && (target == path
                    || path.starts_with(&format!("{}/", target.trim_end_matches('/'))))
            {
                record.count += 1;
                grants.push(Grant {
                    manager: self.clone(),
                    path: target.clone(),
                    released: false,
                });
            }
        }
        grants
    }
    pub async fn acquire(self: &Arc<Self>, path: &str, bits: u32) -> Result<Grant> {
        // 呼出し元が取消されてもchmodのACK処理とGrantのDrop復元を完結させる。
        let manager = self.clone();
        let path = path.to_owned();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.work
            .spawn(&tokio::runtime::Handle::current(), async move {
                let result = manager.acquire_inner(&path, bits).await;
                if let Err(Ok(grant)) = sender.send(result) {
                    let _ = grant.release().await;
                }
            })?;
        receiver.await.context("権限付与タスクが終了しました")?
    }
    async fn acquire_inner(self: &Arc<Self>, path: &str, bits: u32) -> Result<Grant> {
        if bits == 0 || bits & !0o777 != 0 {
            bail!("追加する権限が不正です");
        }
        if self.backend.canonical_path(path).await? != path {
            bail!("権限操作に別表記/親リンクのパスを使えません。正規パスで再試行してください");
        }
        let mut active = self.active.lock().await;
        if let Some(record) = active.get_mut(path) {
            let additional = bits & !record.original & !record.added;
            if additional != 0 {
                if record.entry.is_none() {
                    record.entry = Some(self.journal.record(
                        &self.connection,
                        path,
                        record.original,
                        &self.owner,
                        &record.lease,
                    )?);
                }
                // ACK喪失もあり得るため、chmod失敗でも永続記録を消さない。
                self.backend
                    .chmod(path, record.original | record.added | additional)
                    .await?;
                record.added |= additional;
            }
            record.count += 1;
        } else {
            let lease = self.journal.try_lease(&self.connection, path)?;
            if self
                .journal
                .pending_all()?
                .iter()
                .any(|record| record.path == path)
            {
                bail!("このパスには未復元の権限記録があります。復元後に再試行してください");
            }
            let original = self.backend.mode(path).await? & 0o7777;
            let added = bits & !original;
            let entry = if added != 0 {
                let entry =
                    self.journal
                        .record(&self.connection, path, original, &self.owner, &lease)?;
                Some(entry)
            } else {
                None
            };
            active.insert(
                path.to_owned(),
                Active {
                    original,
                    added,
                    count: 1,
                    entry,
                    lease,
                },
            );
            if added != 0 {
                if let Err(error) = self.backend.chmod(path, original | added).await {
                    drop(active);
                    let restored = self.release(path).await;
                    return Err(error.context(match restored {
                        Ok(()) => "権限付与に失敗したため元の権限へ復元しました".to_owned(),
                        Err(error) => format!("権限付与/復元が未完了です。記録を保持: {error}"),
                    }));
                }
            }
        }
        Ok(Grant {
            manager: self.clone(),
            path: path.into(),
            released: false,
        })
    }
    async fn release(&self, path: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        let record = active.get_mut(path).context("権限の参照記録がありません")?;
        if record.count > 1 {
            record.count -= 1;
            return Ok(());
        }
        let record = active.remove(path).unwrap();
        if let Some(entry) = record.entry {
            if self.journal.pending_all()?.iter().any(|child| {
                child
                    .path
                    .starts_with(&format!("{}/", path.trim_end_matches('/')))
            }) {
                bail!("子パスの未復元記録があるため、親の復元を延期しました。記録を保持しています");
            }
            self.backend.chmod(path, record.original).await?;
            if self.backend.mode(path).await? & 0o7777 != record.original {
                bail!("元のmodeへの復元を確認できません。記録を保持しています");
            }
            self.journal.clear_restored(&entry, &record.lease)?;
        }
        Ok(())
    }
    pub async fn recover(&self) -> Result<(usize, usize)> {
        let _work = self.work.start()?;
        let _operation = operation_gate().lock_owned().await;
        let _active = self.active.lock().await;
        let mut restored = 0;
        let mut pending = 0;
        for entry in self.journal.pending(&self.connection)? {
            // leaseが取れなければ、生存中の所有者をPID照会だけで誤って復元しない。
            let lease = match self.journal.try_lease(&self.connection, &entry.path) {
                Ok(lease) => lease,
                Err(_) => {
                    pending += 1;
                    continue;
                }
            };
            let current = self
                .journal
                .pending(&self.connection)?
                .into_iter()
                .find(|current| current.id == entry.id);
            if current.as_ref() != Some(&entry) {
                continue;
            }
            if self.journal.pending_all()?.iter().any(|child| {
                child
                    .path
                    .starts_with(&format!("{}/", entry.path.trim_end_matches('/')))
            }) {
                pending += 1;
                continue;
            }
            if !self
                .backend
                .canonical_path(&entry.path)
                .await
                .is_ok_and(|path| path == entry.path)
            {
                pending += 1;
                continue;
            }
            match self.backend.chmod(&entry.path, entry.original_mode).await {
                Ok(()) => {
                    if !self
                        .backend
                        .mode(&entry.path)
                        .await
                        .is_ok_and(|mode| mode & 0o7777 == entry.original_mode)
                    {
                        pending += 1;
                        continue;
                    }
                    self.journal.clear_restored(&entry, &lease)?;
                    restored += 1;
                }
                Err(_) => pending += 1,
            }
        }
        Ok((restored, pending))
    }
}
impl Grant {
    pub async fn release(mut self) -> Result<()> {
        self.released = true;
        let manager = self.manager.clone();
        let path = self.path.clone();
        self.manager
            .work
            .cleanup(&tokio::runtime::Handle::current(), async move {
                manager.release(&path).await
            })
            .await
            .context("権限復元の処理が終了しました")?
    }
}
impl Drop for Grant {
    fn drop(&mut self) {
        if !self.released {
            let manager = self.manager.clone();
            let path = self.path.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                self.manager.work.cleanup(&runtime, async move {
                    let _ = manager.release(&path).await;
                });
            }
        }
    }
}

pub fn is_permission_error(error: &anyhow::Error) -> bool {
    error.chain().any(|error| error.downcast_ref::<std::io::Error>().is_some_and(|error|error.kind()==std::io::ErrorKind::PermissionDenied)
        || error.downcast_ref::<russh_sftp::client::error::Error>().is_some_and(|error|matches!(error,russh_sftp::client::error::Error::Status(status) if status.status_code==russh_sftp::protocol::StatusCode::PermissionDenied)))
}
/// 確定結果の通知を権限復元エラーで消さず、処理完了後の自動再試行も防ぐ。
pub fn complete<T>(operation: Result<T>, restoration: Result<()>) -> Result<T> {
    match (operation, restoration) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => {
            bail!("ファイル処理は完了しましたが、権限復元が未完了です。記録を保持: {error}")
        }
        (Err(operation), Err(restoration)) => {
            bail!("ファイル処理: {operation} / 権限復元も未完了です。記録を保持: {restoration}")
        }
    }
}
pub async fn with_access<T, F, U>(
    manager: Option<&Arc<Manager>>,
    path: &str,
    bits: u32,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> U,
    U: Future<Output = Result<T>>,
{
    let mut batch = Batch::new(manager);
    let result = batch.run(path, bits, operation).await;
    complete(result, batch.finish().await)
}

/// 再帰処理中は親の権限を保ち、取消/失敗でも一つの所有タスクで深い順に復元する。
pub struct Batch {
    manager: Option<Arc<Manager>>,
    grants: Vec<Grant>,
    operation_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    work: crate::work::Work,
}
impl Batch {
    pub fn new(manager: Option<&Arc<Manager>>) -> Self {
        Self {
            manager: manager.cloned(),
            grants: vec![],
            operation_guard: None,
            work: manager
                .map(|manager| manager.work.clone())
                .unwrap_or_default(),
        }
    }
    pub async fn join(&mut self, path: &str) {
        if let Some(manager) = &self.manager {
            // 異なるタブも含め権限補助の取引を直列化し、参加snapshot直後の新規grantを防ぐ。
            if self.operation_guard.is_none() {
                self.operation_guard = Some(operation_gate().lock_owned().await);
            }
            let joined = self.grants.iter().map(|grant| grant.path.clone()).collect();
            self.grants
                .extend(manager.join_existing(path, &joined).await);
        }
    }
    pub async fn run<T, F, U>(&mut self, path: &str, bits: u32, mut operation: F) -> Result<T>
    where
        F: FnMut() -> U,
        U: Future<Output = Result<T>>,
    {
        self.join(path).await;
        let first = operation().await;
        if !first.as_ref().is_err_and(is_permission_error) {
            return first;
        }
        let Some(manager) = &self.manager else {
            return first;
        };
        self.grants.push(manager.acquire(path, bits).await?);
        operation().await
    }
    pub async fn finish(mut self) -> Result<()> {
        let grants = std::mem::take(&mut self.grants);
        let operation = self.operation_guard.take();
        self.work
            .cleanup(&tokio::runtime::Handle::current(), async move {
                let _operation = operation;
                restore_batch(grants).await
            })
            .await
            .context("権限復元タスクが終了しました")?
    }
}
async fn restore_batch(mut grants: Vec<Grant>) -> Result<()> {
    grants.sort_by_key(|grant| std::cmp::Reverse(grant.path.matches('/').count()));
    let mut failures = 0;
    for grant in grants {
        if grant.release().await.is_err() {
            failures += 1;
        }
    }
    if failures != 0 {
        bail!("{failures}件の権限復元が未完了です。永続記録を保持しています");
    }
    Ok(())
}
impl Drop for Batch {
    fn drop(&mut self) {
        let grants = std::mem::take(&mut self.grants);
        let operation = self.operation_guard.take();
        if !grants.is_empty() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                self.work.cleanup(&runtime, async move {
                    let _operation = operation;
                    restore_batch(grants).await
                });
            }
        }
    }
}

pub struct SftpBackend {
    session: Mutex<russh_sftp::client::SftpSession>,
}
impl SftpBackend {
    pub fn new(session: russh_sftp::client::SftpSession) -> Self {
        Self {
            session: Mutex::new(session),
        }
    }
}
impl Backend for SftpBackend {
    fn canonical_path<'a>(&'a self, path: &'a str) -> BackendFuture<'a, String> {
        Box::pin(async move {
            let session = self.session.lock().await;
            if session.symlink_metadata(path).await?.is_symlink() {
                bail!("リンクの権限を変更できません");
            }
            Ok(session.canonicalize(path).await?)
        })
    }
    fn mode<'a>(&'a self, path: &'a str) -> BackendFuture<'a, u32> {
        Box::pin(async move {
            let metadata = self.session.lock().await.symlink_metadata(path).await?;
            if metadata.is_symlink() {
                bail!("リンクの権限を変更できません");
            }
            metadata.permissions.context("権限情報がありません")
        })
    }
    fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let session = self.session.lock().await;
            if session.symlink_metadata(path).await?.is_symlink() {
                bail!("リンクの権限を変更できません");
            }
            if session.canonicalize(path).await? != path {
                bail!("権限対象の正規パスが変わりました");
            }
            session
                .set_metadata(
                    path,
                    russh_sftp::client::fs::Metadata {
                        permissions: Some(mode),
                        ..Default::default()
                    },
                )
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake {
        modes: Mutex<BTreeMap<String, u32>>,
        fail_restore: std::sync::atomic::AtomicBool,
        changes: Mutex<Vec<(String, u32)>>,
        pause_grant: std::sync::atomic::AtomicBool,
        fail_grant: std::sync::atomic::AtomicBool,
        strip_special: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        proceed: tokio::sync::Notify,
    }
    impl Backend for Fake {
        fn canonical_path<'a>(&'a self, path: &'a str) -> BackendFuture<'a, String> {
            Box::pin(async move { Ok(path.into()) })
        }
        fn mode<'a>(&'a self, path: &'a str) -> BackendFuture<'a, u32> {
            Box::pin(async move { Ok(*self.modes.lock().await.get(path).unwrap_or(&0)) })
        }
        fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> BackendFuture<'a, ()> {
            Box::pin(async move {
                if mode == 0 && self.fail_restore.load(std::sync::atomic::Ordering::SeqCst) {
                    bail!("合成した復元失敗");
                }
                self.changes.lock().await.push((path.into(), mode));
                let actual = if self.strip_special.load(std::sync::atomic::Ordering::SeqCst) {
                    mode & !0o6000
                } else {
                    mode
                };
                self.modes.lock().await.insert(path.into(), actual);
                if mode != 0
                    && self
                        .pause_grant
                        .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    self.entered.notify_one();
                    self.proceed.notified().await;
                }
                if mode != 0
                    && self
                        .fail_grant
                        .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    bail!("適用後のACK喪失を合成");
                }
                Ok(())
            })
        }
    }
    #[tokio::test]
    async fn shared_references_restore_only_after_last_release_and_failures_remain() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        let first = manager.acquire("/file", 0o444).await.unwrap();
        let second = manager.acquire("/file", 0o222).await.unwrap();
        assert_eq!(backend.mode("/file").await.unwrap(), 0o666);
        first.release().await.unwrap();
        assert_eq!(backend.mode("/file").await.unwrap(), 0o666);
        backend
            .fail_restore
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(second.release().await.is_err());
        assert_eq!(journal.pending("conn").unwrap().len(), 1);
        assert!(manager.acquire("/file", 0o444).await.is_err());
        backend
            .fail_restore
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(manager.recover().await.unwrap(), (1, 0));
        assert!(journal.pending("conn").unwrap().is_empty());
        assert_eq!(backend.mode("/file").await.unwrap(), 0);
    }
    #[tokio::test]
    async fn persistence_failure_prevents_chmod_and_recovery_is_deep_first() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal");
        let journal = Arc::new(Journal::at(path.clone()).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        std::fs::write(path.join("broken.json"), b"{bad").unwrap();
        assert!(manager.acquire("/file", 0o444).await.is_err());
        assert!(backend.changes.lock().await.is_empty());
        std::fs::remove_file(path.join("broken.json")).unwrap();
        for target in ["/parent", "/parent/child"] {
            let lease = journal.try_lease("conn", target).unwrap();
            journal
                .record("conn", target, 0, "dead-owner", &lease)
                .unwrap();
        }
        assert_eq!(manager.recover().await.unwrap(), (2, 0));
        assert_eq!(backend.changes.lock().await[0].0, "/parent/child");
    }
    #[tokio::test]
    async fn cancelled_recursive_scope_restores_children_before_parent() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        let mut batch = Batch::new(Some(&manager));
        for path in ["/parent", "/parent/child"] {
            batch
                .run(path, 0o555, || async {
                    if backend.mode(path).await? == 0 {
                        return Err(
                            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
                        );
                    }
                    Ok(())
                })
                .await
                .unwrap();
        }
        assert_eq!(journal.pending("conn").unwrap().len(), 2);
        backend.changes.lock().await.clear();
        drop(batch);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if backend.changes.lock().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(backend.changes.lock().await[0], ("/parent/child".into(), 0));
        assert_eq!(backend.changes.lock().await[1], ("/parent".into(), 0));
        assert_eq!(manager.shutdown().await.unwrap(), 0);
    }
    #[tokio::test]
    async fn connection_alias_cannot_overlap_an_existing_or_unrestored_path() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let first = Manager::new(backend.clone(), journal.clone(), "first".into());
        let second = Manager::new(backend.clone(), journal.clone(), "alias".into());
        let grant = first.acquire("/file", 0o444).await.unwrap();
        assert!(second.acquire("/file", 0o222).await.is_err());
        backend
            .fail_restore
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(grant.release().await.is_err());
        assert!(second.acquire("/file", 0o222).await.is_err());
        assert_eq!(journal.pending_all().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn cancellation_during_chmod_ack_restores_and_ack_failure_rolls_back() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        backend
            .pause_grant
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let copy = manager.clone();
        let task = tokio::spawn(async move { copy.acquire("/file", 0o444).await });
        backend.entered.notified().await;
        assert_eq!(backend.mode("/file").await.unwrap(), 0o444);
        task.abort();
        let _ = task.await;
        backend.proceed.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if backend.mode("/file").await.unwrap() == 0
                    && journal
                        .pending("conn")
                        .is_ok_and(|entries| entries.is_empty())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        backend
            .fail_grant
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(manager.acquire("/file", 0o222).await.is_err());
        assert_eq!(backend.mode("/file").await.unwrap(), 0);
        assert!(journal.pending("conn").unwrap().is_empty());
        assert!(manager.acquire("/./file", 0o444).await.is_err());
        assert!(manager.acquire("//file", 0o444).await.is_err());
    }
    #[tokio::test]
    async fn failed_child_restore_defers_parent_and_special_bits_are_verified() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        let parent = manager.acquire("/parent", 0o555).await.unwrap();
        let child = manager.acquire("/parent/child", 0o444).await.unwrap();
        backend
            .fail_restore
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(child.release().await.is_err());
        assert!(parent.release().await.is_err());
        assert_eq!(backend.mode("/parent").await.unwrap(), 0o555);
        assert_eq!(journal.pending("conn").unwrap().len(), 2);
        backend
            .fail_restore
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(manager.recover().await.unwrap(), (2, 0));
        backend.modes.lock().await.insert("/special".into(), 0o2600);
        backend
            .strip_special
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let special = manager.acquire("/special", 0o444).await.unwrap();
        assert!(special.release().await.is_err());
        assert_eq!(journal.pending("conn").unwrap().len(), 1);
        assert_eq!(manager.recover().await.unwrap(), (0, 1));
        backend
            .strip_special
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(manager.recover().await.unwrap(), (1, 0));
    }
    #[tokio::test]
    async fn normal_operation_joins_an_existing_ancestor_until_it_finishes() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        let first = manager.acquire("/parent", 0o555).await.unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let proceed = Arc::new(tokio::sync::Notify::new());
        let (entered_copy, proceed_copy, manager_copy) =
            (entered.clone(), proceed.clone(), manager.clone());
        let task = tokio::spawn(async move {
            with_access(Some(&manager_copy), "/parent/file", 0o444, || async {
                entered_copy.notify_one();
                proceed_copy.notified().await;
                Ok(())
            })
            .await
        });
        entered.notified().await;
        first.release().await.unwrap();
        assert_eq!(backend.mode("/parent").await.unwrap(), 0o555);
        proceed.notify_one();
        task.await.unwrap().unwrap();
        assert_eq!(backend.mode("/parent").await.unwrap(), 0);
        assert!(journal.pending("conn").unwrap().is_empty());
    }
    #[tokio::test]
    async fn shutdown_waits_for_cancelled_acquire_ack_and_restoration() {
        let root = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::at(root.path().join("journal")).unwrap());
        let backend = Arc::new(Fake::default());
        let manager = Manager::new(backend.clone(), journal.clone(), "conn".into());
        backend
            .pause_grant
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let copy = manager.clone();
        let acquire = tokio::spawn(async move { copy.acquire("/file", 0o444).await });
        backend.entered.notified().await;
        acquire.abort();
        let _ = acquire.await;
        let shutdown = manager.shutdown();
        tokio::pin!(shutdown);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut shutdown)
                .await
                .is_err()
        );
        backend.proceed.notify_one();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), shutdown)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(backend.mode("/file").await.unwrap(), 0);
        assert!(journal.pending("conn").unwrap().is_empty());
        assert!(manager.acquire("/file", 0o222).await.is_err());
        assert!(manager.recover().await.is_err());
    }
}
