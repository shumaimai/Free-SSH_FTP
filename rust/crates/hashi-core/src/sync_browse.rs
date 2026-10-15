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
