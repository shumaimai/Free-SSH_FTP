//! WinVaultのTargetName・UserName・UTF-16LEを扱う。旧ストアは読取だけにする。
use std::io;
use windows_sys::Win32::{
    Foundation::ERROR_NOT_FOUND,
    Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE,
        CRED_PERSIST_ENTERPRISE, CRED_TYPE_GENERIC,
    },
};

fn wide(text: &str) -> io::Result<Vec<u16>> {
    if text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NULを含む資格情報名です",
        ));
    }
    Ok(text.encode_utf16().chain(Some(0)).collect())
}

unsafe fn read_wide(ptr: *const u16) -> io::Result<String> {
    if ptr.is_null() {
        return Err(io::Error::other("資格情報の利用者名がありません"));
    }
    let mut length = 0;
    while length < 4096 && *ptr.add(length) != 0 {
        length += 1;
    }
    if length == 4096 {
        return Err(io::Error::other("資格情報名が長すぎます"));
    }
    String::from_utf16(std::slice::from_raw_parts(ptr, length))
        .map_err(|_| io::Error::other("資格情報名の文字コードが不正です"))
}

fn read_target(target: &str, username: &str) -> io::Result<Option<String>> {
    let target = wide(target)?;
    unsafe {
        let mut ptr = std::ptr::null_mut();
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        struct Owned(*mut CREDENTIALW);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe { CredFree(self.0.cast()) };
            }
        }
        let owned = Owned(ptr);
        let record = &*owned.0;
        if read_wide(record.UserName)? != username {
            return Ok(None);
        }
        if record.CredentialBlobSize > CRED_MAX_CREDENTIAL_BLOB_SIZE {
            return Err(io::Error::other("資格情報が大きすぎます"));
        }
        if record.CredentialBlobSize == 0 {
            return Ok(Some(String::new()));
        }
        if record.CredentialBlob.is_null() {
            return Err(io::Error::other("資格情報の値がありません"));
        }
        let bytes =
            std::slice::from_raw_parts(record.CredentialBlob, record.CredentialBlobSize as usize);
        let utf16 = if bytes.len().is_multiple_of(2) {
            let big_endian = bytes.starts_with(&[0xfe, 0xff]);
            let data = if big_endian || bytes.starts_with(&[0xff, 0xfe]) {
                &bytes[2..]
            } else {
                bytes
            };
            let words: Vec<_> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| {
                    if big_endian {
                        u16::from_be_bytes([b[0], b[1]])
                    } else {
                        u16::from_le_bytes([b[0], b[1]])
                    }
                })
                .collect();
            String::from_utf16(&words).ok()
        } else {
            None
        };
        Ok(Some(
            utf16
                .or_else(|| std::str::from_utf8(bytes).ok().map(str::to_owned))
                .ok_or_else(|| io::Error::other("資格情報の文字コードが不正です"))?,
        ))
    }
}

pub fn get(service: &str, username: &str) -> io::Result<Option<String>> {
    if let Some(value) = read_target(service, username)? {
        return Ok(Some(value));
    }
    read_target(&format!("{username}@{service}"), username)
}

/// 新Rustサービスだけへ保存する。衝突を避け、利用者別TargetNameを常に使う。
pub fn set(
    channel: crate::paths::Channel,
    ai: bool,
    username: &str,
    value: Option<&str>,
) -> io::Result<()> {
    if channel == crate::paths::Channel::Python {
        return Err(io::Error::other("旧資格情報へ書き込みできません"));
    }
    let service = if ai {
        channel.ai_credential_service()
    } else {
        channel.credential_service()
    };
    write(&format!("{username}@{service}"), username, value)
}

fn write(target: &str, username: &str, value: Option<&str>) -> io::Result<()> {
    if value.is_none() && read_target(target, username)?.is_none() {
        return Ok(());
    }
    let mut target = wide(target)?;
    let mut username = wide(username)?;
    unsafe {
        if let Some(value) = value {
            let mut bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
            if bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
                return Err(io::Error::other("資格情報がWindows保存上限を超えています"));
            }
            let record = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: target.as_mut_ptr(),
                UserName: username.as_mut_ptr(),
                CredentialBlobSize: bytes.len() as u32,
                CredentialBlob: bytes.as_mut_ptr(),
                Persist: CRED_PERSIST_ENTERPRISE,
                ..Default::default()
            };
            if CredWriteW(&record, 0) == 0 {
                return Err(io::Error::last_os_error());
            }
        } else if CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                return Err(error);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_and_compound_targets_verify_username_and_japanese_encoding() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let service = format!("Hashi.Rust.Test.{random:x?}");
        let first = format!("alice@{service}");
        write(&service, "alice", Some("日本語🔑")).unwrap();
        // Python keyring 25.7.0と同じ永続化scopeを試験専用TargetNameで確認する。
        unsafe {
            let target = wide(&service).unwrap();
            let mut ptr = std::ptr::null_mut();
            assert_ne!(
                CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr),
                0
            );
            let persist = (*ptr).Persist;
            CredFree(ptr.cast());
            assert_eq!(persist, CRED_PERSIST_ENTERPRISE);
        }
        assert_eq!(get(&service, "alice").unwrap().as_deref(), Some("日本語🔑"));
        write(&first, "alice", Some("退避済み秘密")).unwrap();
        write(&service, "bob", Some("別利用者")).unwrap();
        assert_eq!(
            get(&service, "alice").unwrap().as_deref(),
            Some("退避済み秘密")
        );
        assert_eq!(get(&service, "bob").unwrap().as_deref(), Some("別利用者"));
        assert!(get(&service, "unknown").unwrap().is_none());
        write(&first, "alice", None).unwrap();
        write(&service, "bob", None).unwrap();
    }
}
