//! 同期ブラウズの調停（Python版SyncBrowse相当）。
//! Python版は「次のpath_changedを1回読み飛ばす」フラグで跳ね返りを防ぐが、Rust版は追随で開始した
//! 一覧要求のStampだけを読み飛ばす。追随が失敗/取消されてもフラグが残らず、次の本物の移動を取りこぼさない。
use super::*;
use hashi_core::sync_browse::{mirror_move, PathStyle};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct SyncState {
    pub(super) enabled: bool,
    local: Option<PathBuf>,
    remote_tab: Option<Stamp>,
    remote: Option<String>,
    follow_local: Option<Stamp>,
    follow_remote: Option<Stamp>,
}
#[derive(Debug, PartialEq)]
pub(super) enum Follow {
    Remote(String),
    Local(PathBuf),
    Failed(String),
}
fn absolute_remote(path: &str) -> Option<String> {
    path.starts_with('/').then(|| path.to_owned())
}
impl SyncState {
    /// 入/切や対象タブの変更時に現在位置を基準にする。この時点では動かさない。
    pub(super) fn rebase(&mut self, local: &Path, tab: Option<(&Stamp, &str)>) {
        self.local = Some(local.to_owned());
        self.remote_tab = tab.map(|(stamp, _)| stamp.clone());
        self.remote = tab.and_then(|(_, path)| absolute_remote(path));
        self.follow_local = None;
        self.follow_remote = None;
    }
    pub(super) fn tracks(&self, tab: &Stamp) -> bool {
        self.remote_tab.as_ref() == Some(tab)
    }
    pub(super) fn on_local(&mut self, revision: &Stamp, new: &Path) -> Option<Follow> {
        let old = self.local.replace(new.to_owned());
        if self.follow_local.as_ref() == Some(revision) {
            self.follow_local = None;
            return None;
        }
        let (old, remote) = (old?, self.remote.as_ref()?);
        if !self.enabled || old == new {
            return None;
        }
        let target = mirror_move(
            &old.to_string_lossy(),
            &new.to_string_lossy(),
            remote,
            PathStyle::local(),
            PathStyle::Posix,
        );
        Some(match target {
            Some(target) => Follow::Remote(target),
            None => Follow::Failed("同期ブラウズ: リモート側は同じ移動ができません".into()),
        })
    }
    pub(super) fn on_remote(
        &mut self,
        tab: &Stamp,
        navigation: &Stamp,
        new: &str,
    ) -> Option<Follow> {
        if !self.tracks(tab) {
            return None;
        }
        let old = std::mem::replace(&mut self.remote, absolute_remote(new));
        if self.follow_remote.as_ref() == Some(navigation) {
            self.follow_remote = None;
            return None;
        }
        let (old, local) = (old?, self.local.as_ref()?);
        if !self.enabled || old == new {
            return None;
        }
        let target = mirror_move(
            &old,
            new,
            &local.to_string_lossy(),
            PathStyle::Posix,
            PathStyle::local(),
        );
        Some(match target {
            Some(target) => Follow::Local(PathBuf::from(target)),
            None => Follow::Failed("同期ブラウズ: ローカル側に対応するフォルダがありません".into()),
        })
    }
    pub(super) fn expect_local(&mut self, revision: Stamp) {
        self.follow_local = Some(revision);
    }
    pub(super) fn expect_remote(&mut self, navigation: Stamp) {
        self.follow_remote = Some(navigation);
    }
    pub(super) fn local_failed(&mut self, revision: &Stamp) -> Option<String> {
        (self.follow_local.as_ref() == Some(revision)).then(|| {
            self.follow_local = None;
            "同期ブラウズ: ローカル側へ移動できません".to_owned()
        })
    }
    pub(super) fn remote_failed(&mut self, navigation: &Stamp, message: &str) -> Option<String> {
        (self.follow_remote.as_ref() == Some(navigation)).then(|| {
            self.follow_remote = None;
            format!("同期ブラウズ: リモート側へ移動できません ({message})")
        })
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (SyncState, Stamp) {
        let tab = Stamp::default();
        let mut sync = SyncState {
            enabled: true,
            ..Default::default()
        };
        sync.rebase(Path::new("/home/me/proj"), Some((&tab, "/srv/app")));
        (sync, tab)
    }

    #[test]
    fn local_move_follows_remote_and_echo_is_skipped_once() {
        let (mut sync, tab) = state();
        let follow = sync.on_local(&Stamp::default(), Path::new("/home/me/proj/logs"));
        assert_eq!(follow, Some(Follow::Remote("/srv/app/logs".into())));
        let navigation = Stamp::default();
        sync.expect_remote(navigation.clone());
        assert_eq!(sync.on_remote(&tab, &navigation, "/srv/app/logs"), None);
        // 次の本物のリモート移動はローカルへ写る。
        let follow = sync.on_remote(&tab, &Stamp::default(), "/srv/app");
        assert_eq!(follow, Some(Follow::Local(PathBuf::from("/home/me/proj"))));
    }

    #[test]
    fn failed_follow_does_not_swallow_the_next_real_move() {
        let (mut sync, tab) = state();
        let navigation = Stamp::default();
        sync.expect_remote(navigation.clone());
        assert!(sync.remote_failed(&Stamp::default(), "別要求").is_none());
        assert!(sync
            .remote_failed(&navigation, "権限")
            .unwrap()
            .contains("権限"));
        // 取消された追随要求の結果が来なくても、別Stampの移動は通常どおり写る。
        sync.expect_remote(Stamp::default());
        let follow = sync.on_remote(&tab, &Stamp::default(), "/srv/app/x");
        assert_eq!(
            follow,
            Some(Follow::Local(PathBuf::from("/home/me/proj/x")))
        );
    }

    #[test]
    fn disabled_other_tab_and_unknown_baseline_do_not_move() {
        let (mut sync, tab) = state();
        sync.enabled = false;
        assert_eq!(
            sync.on_local(&Stamp::default(), Path::new("/home/me")),
            None
        );
        sync.enabled = true;
        assert_eq!(
            sync.on_remote(&Stamp::default(), &Stamp::default(), "/other"),
            None
        );
        sync.rebase(Path::new("/home/me"), Some((&tab, ".")));
        // 初回一覧前の"."から実パスへの変化は基準の確定として扱う。
        assert_eq!(sync.on_remote(&tab, &Stamp::default(), "/home/user"), None);
        assert_eq!(
            sync.on_local(&Stamp::default(), Path::new("/home/me/x")),
            Some(Follow::Remote("/home/user/x".into()))
        );
        sync.rebase(Path::new("/a/b/c"), Some((&tab, "/srv")));
        assert_eq!(
            sync.on_local(&Stamp::default(), Path::new("/")),
            Some(Follow::Failed(
                "同期ブラウズ: リモート側は同じ移動ができません".into()
            ))
        );
    }
}
