//! AIへの質問に添付するファイル。画面が読んだファイルの中身(base64)だけを受け取り、パスは受け取らない。
//! 画像は中身の先頭バイトで種類を決めて会話へ添付し、文字のファイルは質問の本文へ埋め込む。
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};

pub const MAX_FILES: usize = 4;
const MAX_IMAGE: usize = 5 * 1024 * 1024;
const MAX_TEXT: usize = 200 * 1024;

#[derive(Deserialize)]
pub struct Attachment {
    pub name: String,
    /// ファイルの中身(base64)。
    pub data: String,
}

/// 中身の先頭で判定した画像の種類。拡張子や画面の申告は信じない。
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// 表示用の名前。制御文字・書式文字を除き、長さを抑える。
fn display_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .take(100)
        .collect();
    if cleaned.trim().is_empty() {
        "(名前なし)".into()
    } else {
        cleaned
    }
}

/// 添付を検査して、質問の末尾へ足す文字と、会話へ添付する画像に分ける。
pub fn prepare(attachments: Vec<Attachment>) -> Result<(String, Vec<Value>), String> {
    if attachments.len() > MAX_FILES {
        return Err(format!("添付できるのは{MAX_FILES}件までです"));
    }
    let mut text = String::new();
    let mut images = vec![];
    for attachment in attachments {
        let name = display_name(&attachment.name);
        // base64は元の約4/3。上限を大きく超えるものは復号する前に断る。
        if attachment.data.len() > MAX_IMAGE / 3 * 4 + 8 {
            return Err(format!(
                "「{name}」が大きすぎます(画像は5MB、文字のファイルは200KBまで)"
            ));
        }
        let bytes = STANDARD
            .decode(attachment.data.as_bytes())
            .map_err(|_| format!("「{name}」を読み取れませんでした"))?;
        if let Some(media_type) = image_type(&bytes) {
            if bytes.len() > MAX_IMAGE {
                return Err(format!("「{name}」が大きすぎます(画像は5MBまで)"));
            }
            images.push(json!({"media_type": media_type, "data": STANDARD.encode(&bytes)}));
            text.push_str(&format!("\n\n(添付画像「{name}」)"));
            continue;
        }
        if bytes.len() > MAX_TEXT {
            return Err(format!(
                "「{name}」が大きすぎます(文字のファイルは200KBまで)"
            ));
        }
        let body = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        let body = std::str::from_utf8(body)
            .ok()
            .filter(|body| !body.contains('\0'))
            .ok_or_else(|| {
                format!("「{name}」は対応していない形式です(画像はPNG/JPEG/GIF/WebP、文字はUTF-8のファイル)")
            })?;
        // 本文の中の```で囲みが閉じないよう、囲みは本文より長い```にする。
        let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        text.push_str(&format!(
            "\n\n添付ファイル「{name}」:\n{fence}\n{body}\n{fence}"
        ));
    }
    Ok((text, images))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, bytes: &[u8]) -> Attachment {
        Attachment {
            name: name.into(),
            data: STANDARD.encode(bytes),
        }
    }

    #[test]
    fn images_by_content_and_text_inlined() {
        let (text, images) = prepare(vec![
            file("a.png", b"\x89PNG\r\n\x1a\nrest"),
            file("memo.md", "# 見出し\n```sh\nls\n```".as_bytes()),
        ])
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0]["media_type"], "image/png");
        assert!(text.contains("添付画像「a.png」"));
        // 本文中の```より長い囲みを使う。
        assert!(text.contains("添付ファイル「memo.md」:\n````\n# 見出し"));
    }

    #[test]
    fn rejects_binary_too_many_and_bad_base64() {
        assert!(prepare(vec![file("x.exe", b"MZ\0\0\x90")]).is_err());
        assert!(prepare(vec![Attachment {
            name: "a".into(),
            data: "!!".into()
        }])
        .is_err());
        let many = (0..=MAX_FILES)
            .map(|i| file(&format!("{i}.txt"), b"a"))
            .collect();
        assert!(prepare(many).is_err());
        assert!(prepare(vec![file("big.txt", &vec![b'a'; MAX_TEXT + 1])]).is_err());
        // 名前の制御文字・向きを変える文字は表示から除く。
        let (text, _) = prepare(vec![file("a\u{202e}gpj.txt\n", b"x")]).unwrap();
        assert!(text.contains("「agpj.txt」"));
    }
}
