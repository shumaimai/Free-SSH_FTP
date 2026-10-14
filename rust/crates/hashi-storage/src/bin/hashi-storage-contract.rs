//! 両方向fixtureの試験用。秘密はコマンドライン/診断へ含めない。
use hashi_storage::{
    config::{Bundle, Profile},
    crypto,
};
use serde_json::{json, Value};
use std::io::{self, Read};

fn main() -> io::Result<()> {
    let mut input = String::new();
    io::stdin()
        .take(20 * 1024 * 1024)
        .read_to_string(&mut input)?;
    let req: Value =
        serde_json::from_str(&input).map_err(|_| io::Error::other("試験要求が不正です"))?;
    let text = |name: &str| {
        req[name]
            .as_str()
            .ok_or_else(|| io::Error::other("試験要求の項目がありません"))
    };
    let output = match text("operation")? {
        "encrypt_cloud" => {
            json!({"envelope": serde_json::from_slice::<Value>(&crypto::encrypt_cloud(text("plaintext")?.as_bytes(), text("passphrase")?)?).map_err(|_| io::Error::other("出力が不正です"))?})
        }
        "decrypt_cloud" => {
            json!({"plaintext": String::from_utf8(crypto::decrypt_cloud(&serde_json::to_vec(&req["envelope"])? ,text("passphrase")?)?).map_err(|_| io::Error::other("出力が不正です"))?})
        }
        "encrypt_fernet" => {
            json!({"token": crypto::encrypt_fernet(text("key")?, text("plaintext")?.as_bytes())?})
        }
        "encrypt_secrets" => {
            json!({"envelope": crypto::encrypt(&serde_json::to_vec(&req["secrets"])? ,text("passphrase")?)?})
        }
        "bundle_roundtrip" => {
            let bundle = Bundle::parse(&serde_json::to_vec(&req["bundle"])?)?;
            serde_json::to_value(bundle)?
        }
        "profile_roundtrip" => {
            let profile: Profile = serde_json::from_value(req["profile"].clone())?;
            profile.validate()?;
            serde_json::to_value(profile)?
        }
        _ => return Err(io::Error::other("未対応の試験操作です")),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
