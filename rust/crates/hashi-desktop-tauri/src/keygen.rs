//! SSH鍵の生成。秘密鍵の保存・既存ファイルの扱い・公開鍵の登録はhashi-coreの規則を使う。
//! パスフレーズは生成の間だけ持ち、ログや画面イベントへ出さない。
use crate::{app::AppState, session};
use hashi_core::keygen::{self, KeyKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeygenDefaults {
    ed25519: String,
    ecdsa: String,
    rsa: String,
    ecdsa_bits: Vec<u16>,
    rsa_bits: Vec<usize>,
}

fn default_path(name: &str) -> String {
    std::env::home_dir()
        .map(|home| home.join(".ssh").join(name).to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
pub fn keygen_defaults() -> KeygenDefaults {
    KeygenDefaults {
        ed25519: default_path("id_ed25519"),
        ecdsa: default_path("id_ecdsa"),
        rsa: default_path("id_rsa"),
        ecdsa_bits: keygen::ECDSA_BITS.to_vec(),
        rsa_bits: keygen::RSA_BITS.to_vec(),
    }
}

#[tauri::command]
pub fn keygen_path_exists(path: String) -> bool {
    !path.trim().is_empty() && std::path::Path::new(path.trim()).exists()
}

/// 秘密鍵の保存先を選ぶ（OSのダイアログ）。
#[tauri::command]
pub async fn keygen_pick_path() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .save_file()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "保存先の選択に失敗しました".to_owned())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeygenArgs {
    /// "ed25519" / "ecdsa" / "rsa"
    kind: String,
    /// ECDSA/RSAのビット数（Ed25519は無視）。候補にない値は受け付けない。
    bits: u32,
    passphrase: String,
    comment: String,
    path: String,
    overwrite: bool,
    /// 公開鍵を登録する接続のセッションid。
    register_session: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    path: String,
    public_line: String,
    registered: Option<bool>,
    warning: Option<&'static str>,
}

#[tauri::command]
pub async fn keygen_generate(
    state: State<'_, AppState>,
    args: KeygenArgs,
) -> Result<Generated, String> {
    let kind = match args.kind.as_str() {
        "ed25519" => KeyKind::Ed25519,
        "ecdsa" => KeyKind::Ecdsa(
            *keygen::ECDSA_BITS
                .iter()
                .find(|bits| u32::from(**bits) == args.bits)
                .ok_or("ECDSAのビット数が不正です")?,
        ),
        "rsa" => KeyKind::Rsa(
            *keygen::RSA_BITS
                .iter()
                .find(|bits| **bits as u32 == args.bits)
                .ok_or("RSAのビット数が不正です")?,
        ),
        _ => return Err("鍵の種別が不正です".into()),
    };
    let path = PathBuf::from(args.path.trim());
    if args.path.trim().is_empty() {
        return Err("保存先を入力してください".into());
    }
    // 登録先は接続中のセッションだけ。使えない接続を黙って無視しない。
    let target = match &args.register_session {
        Some(id) => Some(
            session::find(&state, id)?
                .files()
                .ok_or("この接続ではファイルを使えません")?,
        ),
        None => None,
    };
    let _work = match &target {
        Some(files) => Some(files.work.start().map_err(|e| e.to_string())?),
        None => None,
    };
    let (comment, passphrase, overwrite) = (args.comment, args.passphrase, args.overwrite);
    let path_for_write = path.clone();
    let public_line = tokio::task::spawn_blocking(move || {
        let key = keygen::generate(kind, &comment)?;
        let content = keygen::private_openssh(&key, &passphrase)?;
        keygen::write_private_key(&path_for_write, &content, overwrite)?;
        keygen::public_line(&key)
    })
    .await
    .map_err(|_| "生成処理が中断されました".to_owned())?
    .map_err(|error| format!("{error:#}"))?;
    let registered =
        match &target {
            Some(files) => Some(files.register_public_key(&public_line).await.map_err(
                |error| format!("秘密鍵は保存しましたが、公開鍵を登録できません: {error:#}"),
            )?),
            None => None,
        };
    let home = std::env::home_dir().unwrap_or_default();
    Ok(Generated {
        warning: keygen::location_warning(&path, cfg!(windows), &home),
        path: path.to_string_lossy().into_owned(),
        public_line,
        registered,
    })
}
