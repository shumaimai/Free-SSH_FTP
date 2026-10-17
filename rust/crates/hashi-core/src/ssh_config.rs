//! OpenSSH設定のHostエイリアス。外部コマンドを起動する指定は拒否する。
use anyhow::{bail, Context, Result};
use hashi_storage::config::Profile;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

pub fn default_path() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| PathBuf::from(home).join(".ssh/config"))
}

fn words(line: &str) -> Result<Vec<String>> {
    let mut words = vec![];
    let mut word = String::new();
    let mut quote = None;
    let mut chars = line.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        if quote == Some(c) {
            quote = None;
            continue;
        }
        if quote.is_none() && (c == '\'' || c == '"') {
            quote = Some(c);
            started = true;
            continue;
        }
        if c == '\\'
            && quote != Some('\'')
            && chars
                .peek()
                .is_some_and(|next| next.is_whitespace() || matches!(next, '\\' | '"' | '\'' | '#'))
        {
            word.push(chars.next().unwrap());
            started = true;
            continue;
        }
        if quote.is_none() {
            if c == '#' {
                break;
            }
            if c.is_whitespace() {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
                continue;
            }
        }
        word.push(c);
        started = true;
    }
    if quote.is_some() {
        bail!("SSH設定の引用符が閉じていません");
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

fn wildcard(pattern: &str, host: &str) -> bool {
    let pattern: Vec<_> = pattern.chars().collect();
    let host: Vec<_> = host.chars().collect();
    let mut previous = vec![false; host.len() + 1];
    previous[0] = true;
    for c in pattern {
        let mut next = vec![false; host.len() + 1];
        if c == '*' {
            next[0] = previous[0];
        }
        for j in 1..=host.len() {
            next[j] = match c {
                '*' => previous[j] || next[j - 1],
                '?' => previous[j - 1],
                c => previous[j - 1] && c.eq_ignore_ascii_case(&host[j - 1]),
            };
        }
        previous = next;
    }
    previous[host.len()]
}
fn matches(patterns: &[String], host: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negative) = pattern.strip_prefix('!') {
            if wildcard(negative, host) {
                return false;
            }
        } else if wildcard(pattern, host) {
            matched = true;
        }
    }
    matched
}

pub fn resolve_text(profile: &Profile, text: &str, home: &Path) -> Result<Profile> {
    if text.len() > 1024 * 1024 || profile.host.len() > 1024 {
        bail!("SSH設定が大きすぎます");
    }
    let mut active = true;
    let mut values = BTreeMap::<String, String>::new();
    for (index, line) in text.lines().enumerate() {
        if line.len() > 16384 {
            bail!("SSH設定の行が長すぎます");
        }
        let mut parts = words(line).with_context(|| format!("SSH設定 {}行目", index + 1))?;
        if parts.is_empty() {
            continue;
        }
        let keyword = parts.remove(0);
        let (keyword, assigned) = keyword
            .split_once('=')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((&keyword, None));
        if let Some(value) = assigned {
            if !value.is_empty() {
                parts.insert(0, value.to_owned());
            }
        }
        if parts.first().is_some_and(|p| p == "=") {
            parts.remove(0);
        }
        let keyword = keyword.to_ascii_lowercase();
        if keyword == "host" {
            active = matches(&parts, &profile.host);
            continue;
        }
        // 現行の対応範囲外のルーティングを黙って無視して直結しない。
        if keyword == "match" || keyword == "include" {
            bail!("SSH設定のMatch/Includeは未対応です。HostとProxyJumpへ平坦化してください");
        }
        if !active {
            continue;
        }
        if [
            "hostname",
            "user",
            "port",
            "identityfile",
            "proxyjump",
            "forwardagent",
            "proxycommand",
        ]
        .contains(&keyword.as_str())
        {
            if parts.is_empty() {
                bail!("SSH設定の値がありません: {keyword}");
            }
            values.entry(keyword).or_insert_with(|| parts.join(" "));
        }
    }
    if values
        .get("proxycommand")
        .is_some_and(|p| !p.eq_ignore_ascii_case("none"))
    {
        bail!("ProxyCommandは未対応です。ProxyJumpを指定してください（直接接続はしません）");
    }
    let mut resolved = profile.clone();
    if let Some(host) = values.get("hostname") {
        resolved.host = host.replace("%h", &profile.host);
    }
    if resolved.username.is_empty() {
        if let Some(user) = values.get("user") {
            resolved.username = user.clone();
        }
    }
    if resolved.port == 22 {
        if let Some(port) = values.get("port") {
            resolved.port = port.parse().context("SSH設定のポートが不正です")?;
        }
    }
    if resolved.auth_method == "key" && resolved.key_path.is_empty() {
        if let Some(path) = values.get("identityfile") {
            let mut expanded = String::new();
            let mut chars = path.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    expanded.push(c);
                    continue;
                }
                match chars.next().context("IdentityFileのトークンが不正です")? {
                    '%' => expanded.push('%'),
                    'd' => expanded.push_str(&home.to_string_lossy()),
                    'h' => expanded.push_str(&resolved.host),
                    'n' => expanded.push_str(&profile.host),
                    'r' => expanded.push_str(&resolved.username),
                    'p' => expanded.push_str(&resolved.port.to_string()),
                    _ => bail!("未対応のIdentityFileトークンです"),
                }
            }
            resolved.key_path = if let Some(rest) = expanded
                .strip_prefix("~/")
                .or_else(|| expanded.strip_prefix("~\\"))
            {
                home.join(rest).to_string_lossy().into_owned()
            } else {
                expanded
            };
        }
    }
    if resolved.proxy_jump.is_empty() {
        if let Some(jump) = values.get("proxyjump") {
            if !jump.eq_ignore_ascii_case("none") {
                resolved.proxy_jump = jump.clone();
            }
        }
    }
    if !resolved.agent_forwarding
        && values
            .get("forwardagent")
            .is_some_and(|p| p.eq_ignore_ascii_case("yes"))
    {
        resolved.agent_forwarding = true;
    }
    resolved.validate()?;
    if resolved.host.contains('\0') || resolved.username.contains('\0') {
        bail!("SSH接続先が不正です");
    }
    Ok(resolved)
}

pub fn resolve_at(profile: &Profile, path: &Path) -> Result<Profile> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(profile.clone()),
        Err(e) => return Err(e).context("SSH設定を読み込めません"),
    };
    let mut bytes = vec![];
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("SSH設定が大きすぎます");
    }
    let text = std::str::from_utf8(&bytes).context("SSH設定はUTF-8で保存してください")?;
    let home = path
        .parent()
        .and_then(Path::parent)
        .context("SSH設定のホームが不明です")?;
    resolve_text(profile, text, home)
}
pub fn resolve(profile: &Profile) -> Result<Profile> {
    match default_path() {
        Some(path) => resolve_at(profile, &path),
        None => Ok(profile.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alias_priority_negation_quotes_and_expansion() {
        let config = "Host test !excluded\nHostName 203.0.113.10\nUser deploy\nPort=2222\nIdentityFile \"~/.ssh/key %n\"\nForwardAgent yes\nHost *\nUser fallback\n";
        let p = Profile {
            host: "test".into(),
            ..Default::default()
        };
        let resolved = resolve_text(&p, config, Path::new("/home/test")).unwrap();
        assert_eq!(resolved.host, "203.0.113.10");
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.username, "deploy");
        assert_eq!(
            PathBuf::from(&resolved.key_path),
            Path::new("/home/test").join(".ssh/key test")
        );
        assert!(resolved.agent_forwarding);
        assert!(p.username.is_empty());
        let p = Profile {
            username: "explicit".into(),
            port: 2200,
            key_path: "/other".into(),
            proxy_jump: "none".into(),
            ..p
        };
        let resolved = resolve_text(&p, config, Path::new("/home/test")).unwrap();
        assert_eq!(resolved.username, "explicit");
        assert_eq!(resolved.port, 2200);
        assert_eq!(resolved.key_path, "/other");
        assert_eq!(resolved.proxy_jump, "none");
        assert!(!matches(&["*".into(), "!ex*".into()], "excluded"));
    }
    #[test]
    fn routing_is_never_silently_downgraded() {
        let p = Profile {
            host: "test".into(),
            ..Default::default()
        };
        for line in [
            "ProxyCommand ssh -W %h:%p jump",
            "Include config.d/*",
            "Match exec command",
            "Port 0",
        ] {
            assert!(
                resolve_text(&p, line, Path::new("/home/test")).is_err(),
                "{line}"
            );
        }
        let r = resolve_text(
            &p,
            "Host test\nProxyJump none\nProxyCommand none\nHost other\nProxyCommand command",
            Path::new("/home/test"),
        )
        .unwrap();
        assert!(r.proxy_jump.is_empty());
        assert!(resolve_text(&p, "IdentityFile \"unterminated", Path::new("/home/test")).is_err());
    }
}
