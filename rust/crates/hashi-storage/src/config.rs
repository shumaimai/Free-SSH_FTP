use crate::{crypto::SecretEnvelope, paths::Channel};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: String,
    pub initial_path: String,
    pub proxy_jump: String,
    pub agent_forwarding: bool,
    pub save_secrets: bool,
    pub sudo_same_as_password: bool,
    pub tags: Vec<String>,
    pub color: String,
    pub last_connected: f64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_method: "key".into(),
            key_path: String::new(),
            initial_path: String::new(),
            proxy_jump: String::new(),
            agent_forwarding: false,
            save_secrets: true,
            sudo_same_as_password: true,
            tags: vec![],
            color: String::new(),
            last_connected: 0.0,
        }
    }
}

impl Profile {
    pub fn id(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.port == 0 || !["key", "password", "agent"].contains(&self.auth_method.as_str()) {
            return Err(invalid("プロファイルのポート/認証方式が不正です"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostKey {
    pub key_type: String,
    pub fingerprint: String,
}

impl HostKey {
    /// SSHの署名アルゴリズム名ではなく公開鍵blobの先頭種別を使う。
    pub fn from_blob(blob: &[u8]) -> io::Result<Self> {
        let header: [u8; 4] = blob
            .get(..4)
            .ok_or_else(|| invalid("公開鍵blobが短すぎます"))?
            .try_into()
            .map_err(|_| invalid("公開鍵blobが不正です"))?;
        let length = u32::from_be_bytes(header) as usize;
        if length > 256 || blob.len() > 16384 {
            return Err(invalid("公開鍵blobのサイズが不正です"));
        }
        let key_type = std::str::from_utf8(
            blob.get(4..4 + length)
                .ok_or_else(|| invalid("公開鍵種別が欠損しています"))?,
        )
        .map_err(|_| invalid("公開鍵種別が不正です"))?
        .to_owned();
        if ![
            "ssh-rsa",
            "ssh-ed25519",
            "ecdsa-sha2-nistp256",
            "ecdsa-sha2-nistp384",
            "ecdsa-sha2-nistp521",
        ]
        .contains(&key_type.as_str())
        {
            return Err(invalid("未対応の公開鍵種別です"));
        }
        Ok(Self {
            key_type,
            fingerprint: format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob))),
        })
    }
}

pub type KnownHosts = BTreeMap<String, HostKey>;
#[derive(Debug, PartialEq, Eq)]
pub enum Trust {
    New,
    Match,
    Mismatch,
}
pub fn check_host(known: &KnownHosts, host: &str, port: u16, key: &HostKey) -> Trust {
    match known.get(&format!("{host}:{port}")) {
        None => Trust::New,
        Some(old) if old == key => Trust::Match,
        Some(_) => Trust::Mismatch,
    }
}
pub fn merge_hosts(known: &mut KnownHosts, incoming: &KnownHosts) -> Vec<String> {
    let mut conflicts = vec![];
    for (host, key) in incoming {
        match known.get(host) {
            Some(old) if old != key => conflicts.push(host.clone()),
            None => {
                known.insert(host.clone(), key.clone());
            }
            _ => {}
        }
    }
    conflicts
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub profiles: Vec<Profile>,
    pub settings: Map<String, Value>,
    pub known_hosts: KnownHosts,
    pub snippets: Value,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            profiles: vec![],
            settings: serde_json::from_str(include_str!("default-settings.json"))
                .expect("同梱設定は有効なJSON"),
            known_hosts: BTreeMap::new(),
            snippets: Value::Array(vec![]),
        }
    }
}
impl Snapshot {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("未対応の保存スキーマです"));
        }
        for profile in &self.profiles {
            profile.validate()?;
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub profiles: Vec<Profile>,
    pub known_hosts: KnownHosts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<SecretEnvelope>,
}
impl Bundle {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > MAX_JSON_BYTES {
            return Err(invalid("書き出しデータが大きすぎます"));
        }
        let bundle: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("書き出し形式を解析できません"))?;
        if bundle.format != "hashi-export" || bundle.version != 1 {
            return Err(invalid("未対応の書き出し形式/版です"));
        }
        for profile in &bundle.profiles {
            profile.validate()?;
        }
        Ok(bundle)
    }
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            format: "hashi-export".into(),
            version: 1,
            profiles: snapshot.profiles.clone(),
            known_hosts: snapshot.known_hosts.clone(),
            secrets: None,
        }
    }
}

pub(crate) fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > MAX_JSON_BYTES {
        return Err(invalid("保存データが大きすぎます"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(invalid("保存データが大きすぎます"));
    }
    Ok(Some(bytes))
}

fn read_legacy<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    read_optional(path)?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| invalid("旧保存データを解析できません。取り込みを中止します"))
        })
        .transpose()
}

/// 呼出し側でMigrationLeaseとマーカー未対応版の停止確認を保持する。
pub fn read_python_snapshot(
    root: &Path,
    _lease: &crate::migration::MigrationLease,
) -> io::Result<Snapshot> {
    let mut snapshot = Snapshot {
        profiles: read_legacy(&root.join("profiles.json"))?.unwrap_or_default(),
        known_hosts: read_legacy(&root.join("known_hosts.json"))?.unwrap_or_default(),
        ..Snapshot::default()
    };
    if let Some(settings) = read_legacy::<Map<String, Value>>(&root.join("settings.json"))? {
        snapshot.settings.extend(settings);
    }
    if let Some(snippets) = read_legacy(&root.join("snippets.json"))? {
        snapshot.snippets = snippets;
    }
    snapshot.validate()?;
    Ok(snapshot)
}

pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(channel: Channel) -> io::Result<Self> {
        if channel == Channel::Python {
            return Err(invalid("旧版の保存先へ書き込みできません"));
        }
        Self::at(crate::paths::config_root(channel)?)
    }
    pub fn at(root: PathBuf) -> io::Result<Self> {
        let legacy = crate::paths::config_root(Channel::Python)?;
        if root == legacy {
            return Err(invalid("旧版の保存先へ書き込みできません"));
        }
        fs::create_dir_all(&root)?;
        if let (Ok(root_path), Ok(legacy_path)) = (root.canonicalize(), legacy.canonicalize()) {
            if root_path == legacy_path {
                return Err(invalid("旧版の保存先へ書き込みできません"));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if fs::symlink_metadata(&root)?.file_type().is_symlink()
                || fs::metadata(&root)?.uid() != unsafe { libc::geteuid() }
            {
                return Err(invalid("保存先の所有者/種類が不正です"));
            }
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { root })
    }
    pub fn load(&self) -> io::Result<Snapshot> {
        match read_optional(&self.root.join("state.json"))? {
            None => Ok(Snapshot::default()),
            Some(bytes) => {
                let snapshot: Snapshot = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("保存データが破損しています"))?;
                snapshot.validate()?;
                Ok(snapshot)
            }
        }
    }
    pub fn transact(
        &self,
        update: impl FnOnce(&mut Snapshot) -> io::Result<()>,
    ) -> io::Result<Snapshot> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("storage.guard"))?;
        FileExt::try_lock_exclusive(&file)?;
        let mut snapshot = self.load()?;
        update(&mut snapshot)?;
        snapshot.validate()?;
        let bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|_| invalid("保存データを符号化できません"))?;
        if bytes.len() as u64 > MAX_JSON_BYTES {
            return Err(invalid("保存データが大きすぎます"));
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(self.root.join("state.json"))
            .map_err(|error| error.error)?;
        #[cfg(unix)]
        fs::File::open(&self.root)?.sync_all()?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/python-v1.json")).unwrap()
    }

    #[test]
    fn rsa_identity_uses_public_key_blob_not_rsa_sha2_signature_name() {
        use base64::engine::general_purpose::STANDARD;
        let f = fixture();
        let blob = STANDARD
            .decode(f["public_key_blob"].as_str().unwrap())
            .unwrap();
        let key = HostKey::from_blob(&blob).unwrap();
        assert_eq!(key.key_type, "ssh-rsa");
        assert_eq!(
            key.fingerprint,
            f["known_hosts"]["server.invalid:2222"]["fingerprint"]
        );
        let known: KnownHosts = serde_json::from_value(f["known_hosts"].clone()).unwrap();
        assert_eq!(
            check_host(&known, "server.invalid", 2222, &key),
            Trust::Match
        );
    }

    #[test]
    fn legacy_import_is_read_only_and_stops_on_corrupt_trust() {
        let f = fixture();
        let source = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        for name in ["profiles", "settings", "known_hosts"] {
            fs::write(
                source.path().join(format!("{name}.json")),
                serde_json::to_vec(&f[name]).unwrap(),
            )
            .unwrap();
        }
        let lease = crate::migration::acquire(runtime.path()).unwrap();
        let original = fs::read(source.path().join("known_hosts.json")).unwrap();
        let snapshot = read_python_snapshot(source.path(), &lease).unwrap();
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(
            fs::read(source.path().join("known_hosts.json")).unwrap(),
            original
        );
        fs::write(source.path().join("known_hosts.json"), b"{broken").unwrap();
        assert!(read_python_snapshot(source.path(), &lease).is_err());
        assert_eq!(
            fs::read(source.path().join("known_hosts.json")).unwrap(),
            b"{broken"
        );
    }

    #[test]
    fn concurrent_writer_does_not_replace_existing_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.path().join("storage.guard"))
            .unwrap();
        FileExt::try_lock_exclusive(&guard).unwrap();
        assert!(store
            .transact(|s| {
                s.profiles.push(Profile::default());
                Ok(())
            })
            .is_err());
        assert!(store.load().unwrap().profiles.is_empty());
        drop(guard);
        assert!(store
            .transact(|s| {
                s.profiles.push(Profile::default());
                Ok(())
            })
            .is_ok());
    }

    #[test]
    fn all_python_profile_fields_and_settings_roundtrip() {
        let f = fixture();
        let profiles: Vec<Profile> = serde_json::from_value(f["profiles"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&profiles).unwrap(), f["profiles"]);
        assert_eq!(
            serde_json::to_value(Snapshot::default().settings).unwrap(),
            f["settings"]
        );
        let bundle = Bundle::parse(&serde_json::to_vec(&f["bundle"]).unwrap()).unwrap();
        assert_eq!(bundle.profiles[0].id(), "demo@server.invalid:2222");
    }

    #[test]
    fn imports_preserve_conflicting_trust() {
        let f = fixture();
        let mut known: KnownHosts = serde_json::from_value(f["known_hosts"].clone()).unwrap();
        let original = known.clone();
        let mut incoming = known.clone();
        incoming.get_mut("server.invalid:2222").unwrap().fingerprint = "SHA256:changed".into();
        assert_eq!(
            merge_hosts(&mut known, &incoming),
            vec!["server.invalid:2222"]
        );
        assert_eq!(known, original);
        assert_eq!(merge_hosts(&mut known, &original), Vec::<String>::new());
    }

    #[test]
    fn atomic_updates_refuse_corrupt_existing_data_and_invalid_new_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().to_path_buf()).unwrap();
        store
            .transact(|s| {
                s.profiles.push(Profile::default());
                Ok(())
            })
            .unwrap();
        let original = fs::read(dir.path().join("state.json")).unwrap();
        assert!(store
            .transact(|s| {
                s.schema_version = 9;
                Ok(())
            })
            .is_err());
        assert_eq!(fs::read(dir.path().join("state.json")).unwrap(), original);
        fs::write(dir.path().join("state.json"), b"{").unwrap();
        assert!(store.transact(|_| Ok(())).is_err());
        assert_eq!(fs::read(dir.path().join("state.json")).unwrap(), b"{");
    }
}
