//! 同期ブラウズの画面側の配線。状態機械（SyncState）はhashi-coreにあり、Tauri版と共有する。
use super::*;
use hashi_core::sync_browse::Follow;

impl App {
    /// 表示中のタブが変わったら、そのタブを基準に取り直す。
    pub(super) fn sync_track_active(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !self.browser.sync.tracks(&tab.stamp) {
            let (stamp, path) = (tab.stamp.clone(), tab.path.clone());
            let local = self.browser.local_path().to_owned();
            self.browser.sync.rebase(&local, Some((&stamp, &path)));
        }
    }
    pub(super) fn set_sync_browse(&mut self, enabled: bool) {
        self.browser.sync.enabled = enabled;
        let tab = self
            .tabs
            .get(self.active)
            .map(|tab| (tab.stamp.clone(), tab.path.clone()));
        let local = self.browser.local_path().to_owned();
        self.browser
            .sync
            .rebase(&local, tab.as_ref().map(|(s, p)| (s, p.as_str())));
        if let Err(error) = self.store.transact(|snapshot| {
            snapshot
                .settings
                .insert("sync_browse".into(), serde_json::Value::Bool(enabled));
            Ok(())
        }) {
            self.notice = format!("同期ブラウズの設定を保存できません: {error}");
        }
    }
    pub(super) fn apply_follow(&mut self, follow: Option<Follow>) {
        match follow {
            None => {}
            Some(Follow::Failed(message)) => self.notice = message,
            Some(Follow::Remote(target)) => {
                if let Some(navigation) = self.browse(self.active, target) {
                    self.browser.sync.expect_remote(navigation);
                }
            }
            Some(Follow::Local(target)) => {
                if !target.is_dir() {
                    self.notice = "同期ブラウズ: ローカル側に対応するフォルダがありません".into();
                } else if let Some(revision) = self.refresh_local(target) {
                    self.browser.sync.expect_local(revision);
                }
            }
        }
    }
}
