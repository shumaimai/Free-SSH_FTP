//! Rust版の更新通知。旧Python版の`/releases/latest`は使わず、Release一覧から
//! このチャネルのタグ体系・prerelease区分・公式URLが揃ったものだけを選ぶ。
//! 解析に失敗したタグは除外し、解析失敗に頼ってチャネルを分けない。
use anyhow::Result;
use semver::Version;
use serde_json::Value;
use std::time::Duration;

pub const REPOSITORY: &str = "shumaimai/Free-SSH_FTP";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateChannel {
    /// `rust-preview-vX.Y.Z-alpha.N`。GitHub上はprerelease。
    Preview,
    /// `rust-vX.Y.Z`。GitHub上はprereleaseではない。
    Stable,
}
impl UpdateChannel {
    pub fn tag_prefix(self) -> &'static str {
        match self {
            Self::Preview => "rust-preview-v",
            Self::Stable => "rust-v",
        }
    }
    /// タグから版を取り出す。チャネルの接頭辞とSemVerに完全一致しなければNone。
    pub fn parse_tag(self, tag: &str) -> Option<Version> {
        let version = Version::parse(tag.strip_prefix(self.tag_prefix())?).ok()?;
        if !version.build.is_empty() {
            return None;
        }
        match self {
            Self::Preview => Some(version),
            Self::Stable => version.pre.is_empty().then_some(version),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub tag: String,
    pub version: Version,
    pub url: String,
}

/// 公式リポジトリのタグページだけを開く対象にする。
pub fn release_page(tag: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/tag/{tag}")
}

/// GitHub `GET /repos/{repo}/releases` の応答から、現在より新しい最新版を選ぶ。
pub fn select(channel: UpdateChannel, current: &Version, releases: &Value) -> Option<Update> {
    releases
        .as_array()?
        .iter()
        .filter_map(|release| {
            if release.get("draft").and_then(Value::as_bool) != Some(false) {
                return None;
            }
            let prerelease = release.get("prerelease").and_then(Value::as_bool)?;
            if prerelease != (channel == UpdateChannel::Preview) {
                return None;
            }
            let tag = release.get("tag_name").and_then(Value::as_str)?;
            let version = channel.parse_tag(tag)?;
            let url = release.get("html_url").and_then(Value::as_str)?;
            (url == release_page(tag)).then(|| Update {
                tag: tag.to_owned(),
                version,
                url: url.to_owned(),
            })
        })
        .filter(|update| &update.version > current)
        .max_by(|left, right| left.version.cmp(&right.version))
}

/// 一覧を取得して選ぶ。ネットワーク待ちがあるのでGUIスレッドから直接呼ばない。
pub fn check(channel: UpdateChannel, current: &str, url: Option<&str>) -> Result<Option<Update>> {
    let current = Version::parse(current)?;
    let default = format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=100");
    let user_agent = format!("Hashi-Rust/{current}");
    let releases = crate::http::get_json(
        url.unwrap_or(&default),
        &[
            ("Accept", "application/vnd.github+json"),
            ("User-Agent", &user_agent),
        ],
        &crate::http::Limits {
            timeout: Duration::from_secs(5),
            max_bytes: 4 * 1024 * 1024,
        },
    )?;
    Ok(select(channel, &current, &releases))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, prerelease: bool) -> Value {
        json!({"tag_name": tag, "prerelease": prerelease, "draft": false,
               "html_url": release_page(tag)})
    }

    #[test]
    fn tags_are_strictly_separated_by_channel() {
        use UpdateChannel::*;
        assert_eq!(
            Preview.parse_tag("rust-preview-v0.1.0-alpha.2"),
            Some(Version::parse("0.1.0-alpha.2").unwrap())
        );
        assert!(Stable.parse_tag("rust-preview-v0.1.0-alpha.2").is_none());
        assert!(Preview.parse_tag("rust-v1.0.0").is_none());
        assert!(Stable.parse_tag("rust-v1.0.0-rc.1").is_none());
        assert!(Stable.parse_tag("v1.1.2").is_none(), "Python版のタグを除外");
        assert!(Stable.parse_tag("rust-v1.0").is_none());
        assert!(Stable.parse_tag("rust-v1.0.0+build").is_none());
        assert!(Preview.parse_tag("rust-preview-v01.0.0").is_none());
    }

    #[test]
    fn prerelease_semver_order_and_python_releases_are_ignored() {
        let releases = json!([
            release("v9.9.9", false),
            release("rust-v2.0.0", false),
            release("rust-preview-v0.1.0-alpha.10", true),
            release("rust-preview-v0.1.0-alpha.9", true),
            release("rust-preview-v0.1.0-beta.1", false),
            release("rust-preview-v0.2.0-alpha.1", true)
                .as_object()
                .map(|r| {
                    let mut r = r.clone();
                    r.insert("draft".into(), json!(true));
                    Value::Object(r)
                })
                .unwrap(),
        ]);
        let current = Version::parse("0.1.0-alpha.2").unwrap();
        let update = select(UpdateChannel::Preview, &current, &releases).unwrap();
        assert_eq!(update.tag, "rust-preview-v0.1.0-alpha.10");
        let latest = Version::parse("0.1.0-alpha.10").unwrap();
        assert!(select(UpdateChannel::Preview, &latest, &releases).is_none());
        let stable = select(
            UpdateChannel::Stable,
            &Version::parse("1.0.0").unwrap(),
            &releases,
        )
        .unwrap();
        assert_eq!(stable.tag, "rust-v2.0.0");
    }

    #[test]
    fn foreign_urls_and_malformed_entries_are_rejected() {
        let mut forged = release("rust-v3.0.0", false);
        forged["html_url"] = json!("https://evil.example/rust-v3.0.0");
        let releases = json!([
            forged,
            {"tag_name": "rust-v4.0.0", "html_url": release_page("rust-v4.0.0")},
            {"tag_name": 5, "prerelease": false, "draft": false},
            "text",
        ]);
        let current = Version::parse("1.0.0").unwrap();
        assert!(select(UpdateChannel::Stable, &current, &releases).is_none());
        assert!(select(
            UpdateChannel::Stable,
            &current,
            &json!({"message": "rate limit"})
        )
        .is_none());
    }
}

/// 実GitHub APIでTLS/proxy/応答形式を確認する（ネットワーク前提のため手動実行）。
#[cfg(test)]
#[test]
#[ignore = "実ネットワークを使う手動確認"]
fn live_github_release_list_is_readable() {
    let result = check(UpdateChannel::Preview, "0.0.1-alpha.0", None);
    println!("{result:?}");
    result.unwrap();
}
