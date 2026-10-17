//! 編集内容のモデル。未編集のバイト列を保持し、HEXはサイズを変更しない。
use anyhow::{bail, Context, Result};
use std::{
    io::{Read, Write},
    path::Path,
};
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Latin1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Newline {
    Lf,
    CrLf,
    Cr,
}
impl Newline {
    pub fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub enum Body {
    Text(String),
    Hex(Vec<u8>),
}
#[derive(Clone)]
pub struct Document {
    pub body: Body,
    pub encoding: Encoding,
    pub newline: Newline,
    original: Vec<u8>,
    saved: Body,
}

pub fn looks_binary(raw: &[u8]) -> bool {
    let raw = &raw[..raw.len().min(8192)];
    if raw.is_empty()
        || raw.starts_with(&[239, 187, 191])
        || raw.starts_with(&[255, 254])
        || raw.starts_with(&[254, 255])
    {
        return false;
    }
    if raw.contains(&0) {
        return true;
    }
    if std::str::from_utf8(raw).is_err()
        && !(1..=3).any(|cut| std::str::from_utf8(&raw[..raw.len().saturating_sub(cut)]).is_ok())
    {
        return true;
    }
    let controls = raw
        .iter()
        .filter(|b| **b < 32 && ![9, 10, 13, 12, 27].contains(b))
        .count();
    controls * 100 > raw.len() * 5
}
fn decode(raw: &[u8]) -> (String, Encoding) {
    let decoded = if let Some(raw) = raw.strip_prefix(&[239, 187, 191]) {
        std::str::from_utf8(raw)
            .ok()
            .map(|text| (text.to_owned(), Encoding::Utf8Bom))
    } else if raw.starts_with(&[255, 254]) || raw.starts_with(&[254, 255]) {
        let little = raw[0] == 255;
        if raw.len() % 2 == 1 {
            None
        } else {
            let words: Vec<u16> = raw[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| {
                    if little {
                        u16::from_le_bytes([p[0], p[1]])
                    } else {
                        u16::from_be_bytes([p[0], p[1]])
                    }
                })
                .collect();
            String::from_utf16(&words).ok().map(|text| {
                (
                    text,
                    if little {
                        Encoding::Utf16Le
                    } else {
                        Encoding::Utf16Be
                    },
                )
            })
        }
    } else {
        std::str::from_utf8(raw)
            .ok()
            .map(|text| (text.to_owned(), Encoding::Utf8))
    };
    decoded.unwrap_or_else(|| {
        (
            raw.iter().map(|b| char::from(*b)).collect(),
            Encoding::Latin1,
        )
    })
}
impl Document {
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self> {
        if raw.len() > MAX_BYTES {
            bail!("内蔵エディタは8MiBまでです");
        }
        let (body, encoding, newline) = if looks_binary(&raw) {
            (Body::Hex(raw.clone()), Encoding::Utf8, Newline::Lf)
        } else {
            let (text, encoding) = decode(&raw);
            let newline = if text.contains("\r\n") {
                Newline::CrLf
            } else if text.contains('\r') {
                Newline::Cr
            } else {
                Newline::Lf
            };
            (
                Body::Text(text.replace("\r\n", "\n").replace('\r', "\n")),
                encoding,
                newline,
            )
        };
        Ok(Self {
            saved: body.clone(),
            body,
            encoding,
            newline,
            original: raw,
        })
    }
    pub fn dirty(&self) -> bool {
        self.body != self.saved
    }
    pub fn original(&self) -> &[u8] {
        &self.original
    }
    pub fn payload(&self) -> Result<Vec<u8>> {
        if !self.dirty() {
            return Ok(self.original.clone());
        }
        let bytes = match &self.body {
            Body::Hex(bytes) => {
                if bytes.len() != self.original.len() {
                    bail!("16進編集ではサイズを変更できません");
                }
                bytes.clone()
            }
            Body::Text(text) => {
                if text.len() > MAX_BYTES * 4 {
                    bail!("編集内容が大きすぎます");
                }
                let text = text
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', self.newline.text());
                match self.encoding {
                    Encoding::Utf8 => text.into_bytes(),
                    Encoding::Utf8Bom => {
                        let mut bytes = vec![239, 187, 191];
                        bytes.extend_from_slice(text.as_bytes());
                        bytes
                    }
                    Encoding::Utf16Le | Encoding::Utf16Be => {
                        let little = self.encoding == Encoding::Utf16Le;
                        let mut bytes = if little {
                            vec![255, 254]
                        } else {
                            vec![254, 255]
                        };
                        for c in text.encode_utf16() {
                            bytes.extend_from_slice(&if little {
                                c.to_le_bytes()
                            } else {
                                c.to_be_bytes()
                            });
                        }
                        bytes
                    }
                    Encoding::Latin1 => text
                        .chars()
                        .map(|c| {
                            u8::try_from(c as u32)
                                .context("元のLatin-1へ変換できない文字があります")
                        })
                        .collect::<Result<Vec<u8>>>()?,
                }
            }
        };
        if bytes.len() > MAX_BYTES {
            bail!("保存内容が8MiBを超えます");
        }
        Ok(bytes)
    }
    pub fn overwrite_byte(&mut self, offset: usize, value: u8) -> Result<()> {
        let Body::Hex(bytes) = &mut self.body else {
            bail!("16進モードではありません");
        };
        *bytes.get_mut(offset).context("バイト位置が範囲外です")? = value;
        Ok(())
    }
    pub fn mark_saved(&mut self, payload: &[u8]) -> Result<()> {
        if self.payload()? != payload {
            bail!("保存中に編集内容が変わりました");
        }
        self.original = payload.to_vec();
        self.saved = self.body.clone();
        Ok(())
    }
    pub fn find(&self, query: &str, start: usize, forward: bool) -> Option<std::ops::Range<usize>> {
        let Body::Text(text) = &self.body else {
            return None;
        };
        if query.is_empty() {
            return None;
        }
        let start = text.floor_char_boundary(start.min(text.len()));
        let found = if forward {
            text[start..]
                .find(query)
                .map(|n| n + start)
                .or_else(|| text[..start].find(query))
        } else {
            text[..start]
                .rfind(query)
                .or_else(|| text[start..].rfind(query).map(|n| n + start))
        }?;
        Some(found..found + query.len())
    }
    pub fn replace_all(&mut self, query: &str, replacement: &str) -> Result<usize> {
        let Body::Text(text) = &mut self.body else {
            bail!("テキストモードではありません");
        };
        if query.is_empty() {
            return Ok(0);
        }
        let count = text.matches(query).count();
        let length = text
            .len()
            .saturating_sub(count.saturating_mul(query.len()))
            .saturating_add(count.saturating_mul(replacement.len()));
        if length > MAX_BYTES * 4 {
            bail!("置換結果が大きすぎます");
        }
        *text = text.replace(query, replacement);
        Ok(count)
    }
}

pub fn read_local(path: &Path) -> Result<Document> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        bail!("通常ファイルを選択してください");
    }
    let mut bytes = vec![];
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    Document::from_bytes(bytes)
}
pub fn save_local(path: &Path, original: &[u8], payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_BYTES {
        bail!("保存内容が大きすぎます");
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("通常ファイル以外へ保存できません");
    }
    let before = read_local(path)?;
    if before.original() != original {
        bail!("ファイルが外部で変更されました。再読込が必要です");
    }
    let parent = path.parent().context("保存先フォルダが不明です")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(payload)?;
    temp.as_file().set_permissions(metadata.permissions())?;
    temp.as_file().sync_all()?;
    if read_local(path)?.original() != original {
        bail!("保存中にファイルが外部で変更されました");
    }
    temp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub async fn read_remote(
    session: &russh_sftp::client::SftpSession,
    path: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Document> {
    use tokio::io::AsyncReadExt;
    let size = session.metadata(path).await?.len();
    if size > MAX_BYTES as u64 {
        bail!("内蔵エディタは8MiBまでです");
    }
    let mut file = session.open(path).await?;
    let mut raw = vec![];
    let mut bytes = vec![0; 32768];
    loop {
        let count = tokio::select! {_=cancel.cancelled()=>bail!("読込を取り消しました"),count=file.read(&mut bytes)=>count?};
        if count == 0 {
            break;
        }
        if raw.len() + count > MAX_BYTES {
            bail!("ファイルが8MiBを超えました");
        }
        raw.extend_from_slice(&bytes[..count]);
    }
    file.close().await?;
    Document::from_bytes(raw)
}

pub async fn save_remote(
    ssh: &crate::ssh::SshSession,
    session: &russh_sftp::client::SftpSession,
    path: &str,
    original: &[u8],
    payload: &[u8],
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    use russh_sftp::{
        client::{fs::Metadata, RawSftpSession},
        protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
    };
    use tokio::io::AsyncWriteExt;
    if payload.len() > MAX_BYTES
        || !path.starts_with('/')
        || path.contains('\0')
        || path.len() > 16384
    {
        bail!("保存内容/宛先が不正です");
    }
    let original_meta = session.symlink_metadata(path).await?;
    if !original_meta.is_regular() {
        bail!("通常ファイル以外へ保存できません。リンクは実体を開いてください");
    }
    if read_remote(session, path, cancel).await?.original() != original {
        bail!("リモートファイルが外部で変更されました。再読込が必要です");
    }
    // POSIX rename拡張の有無を変更前に確認する。未対応なら元ファイルを保全して拒否。
    let channel = ssh.connection.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await?;
    if !version
        .extensions
        .get("posix-rename@openssh.com")
        .is_some_and(|v| v == "1")
    {
        bail!("atomic保存に必要なposix-rename拡張がありません");
    }
    let parent = path.rsplit_once('/').context("保存先フォルダが不明です")?.0;
    let temporary = format!("{parent}/.hashi-edit-{}.tmp", uuid::Uuid::new_v4());
    let mut file = session
        .open_with_flags_and_attributes(
            &temporary,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
            FileAttributes {
                permissions: Some(0o600),
                ..Default::default()
            },
        )
        .await?;
    let result=async {
        for chunk in payload.chunks(32768){tokio::select!{_=cancel.cancelled()=>bail!("保存を取り消しました"),result=file.write_all(chunk)=>result?};}
        file.flush().await?;file.sync_all().await?;
        file.set_metadata(Metadata {permissions:original_meta.permissions,uid:original_meta.uid,gid:original_meta.gid,..Default::default()}).await?;
        file.sync_all().await?;
        file.close().await?;
        if read_remote(session,path,cancel).await?.original()!=original{bail!("保存中にリモートファイルが外部で変更されました");}
        if cancel.is_cancelled(){bail!("保存を取り消しました");}
        let mut encoded=vec![];
        for string in [&temporary,path]{encoded.extend_from_slice(&(string.len() as u32).to_be_bytes());encoded.extend_from_slice(string.as_bytes());}
        // commitの応答待ちでは取消でfutureを捨てない。結果不明なら未保存状態を保つ。
        match raw.extended("posix-rename@openssh.com",encoded).await?{
            Packet::Status(status) if status.status_code==StatusCode::Ok=>Ok(()),
            _=>bail!("atomic保存の完了を確認できません"),
        }
    }.await;
    if result.is_err() {
        let _ = session.remove_file(&temporary).await;
    }
    let _ = raw.close_session();
    result
}

pub fn save_as_local(path: &Path, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_BYTES {
        bail!("保存内容が大きすぎます");
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            let document = read_local(path)?;
            save_local(path, document.original(), payload)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("保存先が不明です")?;
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            temp.write_all(payload)?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(path).map_err(|error| error.error)?;
            #[cfg(unix)]
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_as_is_atomic_and_unrepresentable_latin1_stays_dirty() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.dat");
        save_as_local(&path, &[0, 13, 10, 255]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0, 13, 10, 255]);
        save_as_local(&path, &[0, 14, 10, 255]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0, 14, 10, 255]);
        let mut document = Document::from_bytes(vec![b'a', 255]).unwrap();
        assert_eq!(document.encoding, Encoding::Latin1);
        let Body::Text(text) = &mut document.body else {
            panic!()
        };
        text.push('😀');
        assert!(document.payload().is_err());
        assert!(document.dirty());
        assert_eq!(document.original(), [b'a', 255]);
    }
    #[test]
    fn unchanged_bytes_preserve_bom_endianness_and_mixed_newlines() {
        let utf16 = |little: bool| {
            let mut bytes = if little {
                vec![255, 254]
            } else {
                vec![254, 255]
            };
            for c in "日本語\r\n😀\r\n".encode_utf16() {
                bytes.extend_from_slice(&if little {
                    c.to_le_bytes()
                } else {
                    c.to_be_bytes()
                });
            }
            bytes
        };
        for bytes in [
            b"utf8\r\nmixed\nend\r".to_vec(),
            b"\xef\xbb\xbfhello\r\n".to_vec(),
            utf16(true),
            utf16(false),
        ] {
            let mut doc = Document::from_bytes(bytes.clone()).unwrap();
            assert_eq!(doc.payload().unwrap(), bytes);
            let Body::Text(text) = &mut doc.body else {
                panic!()
            };
            text.push('!');
            let payload = doc.payload().unwrap();
            assert!(doc.dirty());
            let next = Document::from_bytes(payload.clone()).unwrap();
            assert_eq!(next.encoding, doc.encoding);
            assert_eq!(next.newline, doc.newline);
            doc.mark_saved(&payload).unwrap();
            assert!(!doc.dirty());
            assert_eq!(doc.payload().unwrap(), payload);
        }
    }
    #[test]
    fn hex_is_fixed_size_and_failed_save_keeps_dirty_state() {
        let original = vec![0, 13, 10, 255];
        let mut doc = Document::from_bytes(original.clone()).unwrap();
        doc.overwrite_byte(1, 99).unwrap();
        assert!(doc.dirty());
        assert_eq!(doc.payload().unwrap(), [0, 99, 10, 255]);
        assert!(doc.overwrite_byte(4, 1).is_err());
        assert!(doc.mark_saved(&original).is_err());
        assert!(doc.dirty());
        let Body::Hex(bytes) = &mut doc.body else {
            panic!()
        };
        bytes.push(1);
        assert!(doc.payload().is_err());
    }
    #[test]
    fn local_conflict_preserves_external_data_and_unicode_search_wraps() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file.txt");
        std::fs::write(&path, b"original").unwrap();
        let doc = read_local(&path).unwrap();
        std::fs::write(&path, b"external").unwrap();
        assert!(save_local(&path, doc.original(), b"edited").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external");
        save_local(&path, b"external", b"edited").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"edited");
        let mut doc = Document::from_bytes("日本 日本".as_bytes().to_vec()).unwrap();
        assert_eq!(doc.find("日本", 6, true), Some(7..13));
        assert_eq!(doc.find("日本", 13, true), Some(0..6));
        assert_eq!(doc.replace_all("日本", "語").unwrap(), 2);
    }
}
