//! Python v1のscrypt/Fernet形式。秘密をエラー文へ含めない。
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE},
    Engine,
};
use fernet::Fernet;
use serde::{Deserialize, Serialize};
use std::io;

const MAX_ENCRYPTED_BYTES: usize = 16 * 1024 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SecretEnvelope {
    pub kdf: String,
    pub salt: String,
    pub token: String,
}

#[derive(Deserialize, Serialize)]
pub struct CloudEnvelope {
    pub format: String,
    pub version: u32,
    #[serde(flatten)]
    pub encrypted: SecretEnvelope,
    pub updated_at: f64,
}

pub fn derive_key(passphrase: &str, salt: &[u8]) -> io::Result<String> {
    if salt.len() != 16 {
        return Err(invalid("scrypt saltの長さが違います"));
    }
    let params = scrypt::Params::new(14, 8, 1).map_err(|_| invalid("scrypt設定が不正です"))?;
    let mut key = [0; 32];
    // Unicodeを正規化せずPythonのUTF-8と一致させる。
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, &mut key)
        .map_err(|_| invalid("scrypt鍵導出に失敗しました"))?;
    Ok(URL_SAFE.encode(key))
}

pub fn decrypt_fernet(key: &str, token: &str) -> io::Result<Vec<u8>> {
    if token.len() > MAX_ENCRYPTED_BYTES {
        return Err(invalid("暗号化データが大きすぎます"));
    }
    Fernet::new(key)
        .ok_or_else(|| invalid("暗号化鍵の形式が違います"))?
        .decrypt(token)
        .map_err(|_| invalid("認証情報を復号できません"))
}

pub fn encrypt_fernet(key: &str, bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_ENCRYPTED_BYTES / 2 {
        return Err(invalid("秘密データが大きすぎます"));
    }
    Ok(Fernet::new(key)
        .ok_or_else(|| invalid("暗号化鍵の形式が違います"))?
        .encrypt(bytes))
}

pub fn decrypt(envelope: &SecretEnvelope, passphrase: &str) -> io::Result<Vec<u8>> {
    if envelope.kdf != "scrypt" || envelope.salt.len() > 64 {
        return Err(invalid("暗号化形式が違います"));
    }
    let salt = STANDARD
        .decode(&envelope.salt)
        .map_err(|_| invalid("saltを解析できません"))?;
    decrypt_fernet(&derive_key(passphrase, &salt)?, &envelope.token)
}

pub fn encrypt(bytes: &[u8], passphrase: &str) -> io::Result<SecretEnvelope> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|_| invalid("乱数を取得できません"))?;
    Ok(SecretEnvelope {
        kdf: "scrypt".into(),
        salt: STANDARD.encode(salt),
        token: encrypt_fernet(&derive_key(passphrase, &salt)?, bytes)?,
    })
}

pub fn decrypt_cloud(bytes: &[u8], passphrase: &str) -> io::Result<Vec<u8>> {
    if bytes.len() > MAX_ENCRYPTED_BYTES {
        return Err(invalid("同期データが大きすぎます"));
    }
    let env: CloudEnvelope =
        serde_json::from_slice(bytes).map_err(|_| invalid("同期形式が不正です"))?;
    if env.format != "hashi-cloudsync" || env.version != 1 {
        return Err(invalid("未対応の同期形式/版です"));
    }
    decrypt(&env.encrypted, passphrase)
}

pub fn encrypt_cloud(bytes: &[u8], passphrase: &str) -> io::Result<Vec<u8>> {
    let env = CloudEnvelope {
        format: "hashi-cloudsync".into(),
        version: 1,
        encrypted: encrypt(bytes, passphrase)?,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("時刻を取得できません"))?
            .as_secs_f64(),
    };
    serde_json::to_vec(&env).map_err(|_| invalid("同期データを符号化できません"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/python-v1.json")).unwrap()
    }

    #[test]
    fn python_encrypted_files_and_japanese_scrypt_are_readable() {
        let f = fixture();
        let plaintext = decrypt_fernet(
            f["fernet_key"].as_str().unwrap(),
            f["fernet_token"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&plaintext).unwrap(),
            f["fernet_plaintext"]
        );
        let envelope: SecretEnvelope =
            serde_json::from_value(f["encrypted_secrets"].clone()).unwrap();
        let plaintext = decrypt(&envelope, f["passphrase"].as_str().unwrap()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&plaintext).unwrap(),
            f["secret_values"]
        );
        assert!(decrypt(&envelope, "間違い").is_err());
        let cloud = serde_json::to_vec(&f["cloud_envelope"]).unwrap();
        assert_eq!(
            decrypt_cloud(&cloud, f["passphrase"].as_str().unwrap()).unwrap(),
            "試験データ\n".as_bytes()
        );
    }

    #[test]
    fn tampered_and_future_ciphertext_is_rejected() {
        let f = fixture();
        let mut cloud = f["cloud_envelope"].clone();
        cloud["version"] = Value::from(2);
        assert!(decrypt_cloud(
            &serde_json::to_vec(&cloud).unwrap(),
            f["passphrase"].as_str().unwrap()
        )
        .is_err());
        assert!(decrypt_fernet(f["fernet_key"].as_str().unwrap(), "invalid").is_err());
    }
}
