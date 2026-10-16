//! 同期ブラウズの相対移動（Python版`localbrowser.mirror_move`相当、#82第2段）。
//! 左右はパスの体系が違うため、絶対パスは写さず「共通の親までさかのぼって降りる」移動だけを写す。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    Posix,
    Windows,
}
impl PathStyle {
    /// このOSのローカルパス体系。
    pub fn local() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
    fn separator(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }
    fn is_separator(self, c: char) -> bool {
        c == '/' || (self == Self::Windows && c == '\\')
    }
}

/// `(ルート, 構成要素)`。ルートはドライブ/UNC共有を含む（"C:\\" / "/" / ""）。
pub fn components(path: &str, style: PathStyle) -> (String, Vec<String>) {
    let path = if path.is_empty() {
        style.separator().to_string()
    } else {
        path.to_owned()
    };
    let separator = style.separator();
    let normalized: String = path
        .chars()
        .map(|c| if style.is_separator(c) { separator } else { c })
        .collect();
    let (drive, rest) = match style {
        PathStyle::Posix => ("".to_owned(), normalized.as_str()),
        PathStyle::Windows => split_drive(&normalized),
    };
    let absolute = rest.starts_with(separator);
    let mut parts: Vec<String> = vec![];
    for part in rest.split(separator) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..".into());
                }
            }
            part => parts.push(part.to_owned()),
        }
    }
    let root = if absolute {
        format!("{drive}{separator}")
    } else {
        drive
    };
    (root, parts)
}

/// ntpath.splitdriveと同じく、"C:"またはUNCの"\\server\share"を分ける。
fn split_drive(path: &str) -> (String, &str) {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return (path[..2].to_owned(), &path[2..]);
    }
    if let Some(rest) = path.strip_prefix("\\\\") {
        if !rest.starts_with('\\') {
            if let Some(server_end) = rest.find('\\') {
                let after = &rest[server_end + 1..];
                if !after.is_empty() && !after.starts_with('\\') {
                    let share_end = after.find('\\').unwrap_or(after.len());
                    let end = 2 + server_end + 1 + share_end;
                    return (path[..end].to_owned(), &path[end..]);
                }
            }
        }
    }
    (String::new(), path)
}

pub fn join(root: &str, parts: &[String], style: PathStyle) -> String {
    let separator = style.separator().to_string();
    if parts.is_empty() {
        return if root.is_empty() {
            separator
        } else {
            root.to_owned()
        };
    }
    let mut path = if root.is_empty() {
        separator.clone()
    } else {
        root.to_owned()
    };
    // ntpath.join("C:", "x") は "C:x"（ドライブ相対）。Python版と同じく区切りを足さない。
    let bare_drive = style == PathStyle::Windows && path.len() == 2 && path.ends_with(':');
    if !path.ends_with(style.separator()) && !bare_drive {
        path.push_str(&separator);
    }
    path.push_str(&parts.join(&separator));
    path
}

/// 片側の`old`→`new`の移動を、もう片方の`other`へ相対的に写す。
/// ドライブ/ルートが変わる移動と、相手がルートを突き抜ける移動はNone（追随しない）。
pub fn mirror_move(
    old: &str,
    new: &str,
    other: &str,
    style: PathStyle,
    other_style: PathStyle,
) -> Option<String> {
    let (old_root, old_parts) = components(old, style);
    let (new_root, new_parts) = components(new, style);
    if old_root != new_root {
        return None;
    }
    let common = old_parts
        .iter()
        .zip(&new_parts)
        .take_while(|(left, right)| left == right)
        .count();
    let ups = old_parts.len() - common;
    let (other_root, other_parts) = components(other, other_style);
    if ups > other_parts.len() {
        return None;
    }
    let mut kept = other_parts[..other_parts.len() - ups].to_vec();
    kept.extend_from_slice(&new_parts[common..]);
    Some(join(&other_root, &kept, other_style))
}

// ---- 同期ブラウズの調停（Python版SyncBrowse相当）--------------------------------------
// Python版は「次のpath_changedを1回読み飛ばす」フラグで跳ね返りを防ぐが、こちらは追随で開始した
// 一覧要求のStampだけを読み飛ばす。追随が失敗/取消されてもフラグが残らず、次の本物の移動を取りこぼさない。
use crate::interaction::Stamp;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct SyncState {
    pub enabled: bool,
    local: Option<PathBuf>,
    remote_tab: Option<Stamp>,
    remote: Option<String>,
    follow_local: Option<Stamp>,
    follow_remote: Option<Stamp>,
}
#[derive(Debug, PartialEq)]
pub enum Follow {
    Remote(String),
    Local(PathBuf),
    Failed(String),
}
fn absolute_remote(path: &str) -> Option<String> {
    path.starts_with('/').then(|| path.to_owned())
}
impl SyncState {
    /// 入/切や対象タブの変更時に現在位置を基準にする。この時点では動かさない。
    pub fn rebase(&mut self, local: &Path, tab: Option<(&Stamp, &str)>) {
        self.local = Some(local.to_owned());
        self.remote_tab = tab.map(|(stamp, _)| stamp.clone());
        self.remote = tab.and_then(|(_, path)| absolute_remote(path));
        self.follow_local = None;
        self.follow_remote = None;
    }
    pub fn tracks(&self, tab: &Stamp) -> bool {
        self.remote_tab.as_ref() == Some(tab)
    }
    pub fn on_local(&mut self, revision: &Stamp, new: &Path) -> Option<Follow> {
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
    pub fn on_remote(&mut self, tab: &Stamp, navigation: &Stamp, new: &str) -> Option<Follow> {
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
    pub fn expect_local(&mut self, revision: Stamp) {
        self.follow_local = Some(revision);
    }
    pub fn expect_remote(&mut self, navigation: Stamp) {
        self.follow_remote = Some(navigation);
    }
    pub fn local_failed(&mut self, revision: &Stamp) -> Option<String> {
        (self.follow_local.as_ref() == Some(revision)).then(|| {
            self.follow_local = None;
            "同期ブラウズ: ローカル側へ移動できません".to_owned()
        })
    }
    pub fn remote_failed(&mut self, navigation: &Stamp, message: &str) -> Option<String> {
        (self.follow_remote.as_ref() == Some(navigation)).then(|| {
            self.follow_remote = None;
            format!("同期ブラウズ: リモート側へ移動できません ({message})")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PathStyle::{Posix, Windows};
    use super::*;

    fn parts(root: &str, list: &[&str]) -> (String, Vec<String>) {
        (root.into(), list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn components_match_python() {
        assert_eq!(
            components("/srv/app/logs", Posix),
            parts("/", &["srv", "app", "logs"])
        );
        assert_eq!(components("/", Posix), parts("/", &[]));
        assert_eq!(components("//x/y", Posix), parts("/", &["x", "y"]));
        assert_eq!(components("", Posix), parts("/", &[]));
        assert_eq!(components("a/b", Posix), parts("", &["a", "b"]));
        assert_eq!(components("/a/../..", Posix), parts("/", &[]));
        assert_eq!(
            components(r"C:\Users\me", Windows),
            parts("C:\\", &["Users", "me"])
        );
        assert_eq!(components("C:\\", Windows), parts("C:\\", &[]));
        assert_eq!(components("C:", Windows), parts("C:", &[]));
        assert_eq!(
            components(r"\\srv\share\a", Windows),
            parts(r"\\srv\share\", &["a"])
        );
        assert_eq!(components("C:/x/../y", Windows), parts("C:\\", &["y"]));
        assert_eq!(
            components(r"relative\a", Windows),
            parts("", &["relative", "a"])
        );
        assert_eq!(components(r"C:\..", Windows), parts("C:\\", &[]));
    }

    #[test]
    fn mirror_moves_follow_python_cases() {
        let m = |old, new, other| mirror_move(old, new, other, Posix, Posix);
        assert_eq!(
            m("/home/me/proj", "/home/me/proj/logs", "/srv/app").as_deref(),
            Some("/srv/app/logs")
        );
        assert_eq!(
            m("/home/me/proj", "/home/me", "/srv/app/logs").as_deref(),
            Some("/srv/app")
        );
        assert_eq!(
            m("/home/me/a", "/home/me/b", "/srv/app/a").as_deref(),
            Some("/srv/app/b")
        );
        assert_eq!(
            m("/home/me/proj", "/home/me/proj/x/y", "/srv/app").as_deref(),
            Some("/srv/app/x/y")
        );
        assert_eq!(m("/a/b/c", "/", "/srv"), None);
        assert_eq!(
            mirror_move(r"C:\Users\me", r"D:\data", r"C:\other", Windows, Windows),
            None
        );
        assert_eq!(
            mirror_move(r"C:\a", r"C:\a\b", "/srv", Windows, Posix).as_deref(),
            Some("/srv/b")
        );
        assert_eq!(
            mirror_move("/srv", "/srv/x", r"C:\a", Posix, Windows).as_deref(),
            Some(r"C:\a\x")
        );
        assert_eq!(mirror_move("/srv", "/", "C:\\", Posix, Windows), None);
        assert_eq!(
            mirror_move("/srv/x", "/srv", "C:\\a", Posix, Windows).as_deref(),
            Some("C:\\")
        );
        assert_eq!(
            mirror_move("/srv", "/srv/x", "C:", Posix, Windows).as_deref(),
            Some("C:x")
        );
    }
}

#[cfg(test)]
mod sync_state_tests {
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
