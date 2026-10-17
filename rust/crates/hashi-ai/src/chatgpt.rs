//! 公式「Sign in with ChatGPT」の公開クライアント認証と、その接続でのResponses(Python版`chatgpt_oauth.py`相当)。
//! 使う接続先はauth.openai.comと公開Responses APIだけ。state/nonce/PKCEを照合し、IDトークンの署名(RS256)・
//! issuer・aud・exp・subと、ChatGPTプランの使用許可(scope)を確かめる。
//! トークンはAI専用の暗号化ストアにだけ置き、画面・ログへ出さない。アカウントの切替は検証が済んでから行う。
use crate::{api::ApiProvider, http};
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use fs2::FileExt;
use hashi_storage::secrets::EncryptedSecrets;
use serde_json::{json, Map, Value};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

pub const ISSUER: &str = "https://auth.openai.com";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const USE_SCOPE: &str = "chatgpt.tokens.use.direct";
const DYNAMIC: &str = "dynamic_agent_client";
const ACCOUNTS: &str = "oauth:accounts";
const HOST: &str = "oauth:host";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
/// Responsesで関数をまとめるnamespaceの名前。
pub const NAMESPACE: &str = "hashi";

/// 認証の接続先。本番は公式だけ。試験ではloopbackの偽サーバーを指す。
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub issuer: String,
    pub resource: String,
}
impl Endpoints {
    pub fn official() -> Self {
        Self {
            issuer: ISSUER.into(),
            resource: RESOURCE.into(),
        }
    }
    fn authorize(&self) -> String {
        format!("{}/api/accounts/authorize", self.issuer)
    }
    fn token(&self) -> String {
        format!("{}/api/accounts/oauth/token", self.issuer)
    }
    /// メタデータが指す接続先(jwks・失効)は、発行者と同じホストだけを許す。
    fn same_host(&self, url: &str) -> Result<String> {
        let checked = http::validate_base_url(url)?;
        let (a, b) = (url::Url::parse(&checked)?, url::Url::parse(&self.issuer)?);
        if a.scheme() != b.scheme()
            || a.host_str() != b.host_str()
            || a.port_or_known_default() != b.port_or_known_default()
        {
            bail!("認証メタデータの接続先が公式ホストではありません");
        }
        Ok(checked)
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow!("乱数を作れませんでした"))?;
    Ok(bytes)
}

fn random_token() -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

/// このPCのHashiを表すID(`urn:uuid:`+UUIDv4)。初回に作って保存する。
fn new_host_id() -> Result<String> {
    let mut b = random_bytes::<16>()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn equal_time(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// 1回のサインインの試み。state/nonce/PKCEの値は試みごとに作り直す。
pub struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    host_id: String,
    registration: Option<Value>,
}

/// コールバックの拒否。`matched`が真なら正しいstateでの拒否なので、試みを終える。
#[derive(Debug)]
pub struct Rejected {
    pub matched: bool,
    pub error: anyhow::Error,
}

impl Attempt {
    pub fn new(host_id: &str, registration: Option<Value>) -> Result<Self> {
        Ok(Self {
            state: random_token()?,
            nonce: random_token()?,
            verifier: random_token()?,
            host_id: host_id.to_owned(),
            registration,
        })
    }

    fn registered_client(&self) -> Option<&str> {
        self.registration
            .as_ref()
            .and_then(|r| r["client_id"].as_str())
    }

    pub fn authorization_url(&self, endpoints: &Endpoints, redirect: &str) -> String {
        let challenge = URL_SAFE_NO_PAD.encode(ring::digest::digest(
            &ring::digest::SHA256,
            self.verifier.as_bytes(),
        ));
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("client_id", self.registered_client().unwrap_or(DYNAMIC))
            .append_pair("ext_agent_host_id", &self.host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect)
            .append_pair("scope", SCOPES)
            .append_pair("resource", &endpoints.resource)
            .append_pair("state", &self.state)
            .append_pair("nonce", &self.nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge);
        match &self.registration {
            Some(registration) => {
                if let Some(hint) = registration["id_token"].as_str() {
                    query.append_pair("id_token_hint", hint);
                }
                if let Some(email) = registration["email"].as_str().filter(|e| !e.is_empty()) {
                    query.append_pair("login_hint", email);
                }
            }
            None => {
                query.append_pair("agent_name_hint", "Hashi");
            }
        }
        format!("{}?{}", endpoints.authorize(), query.finish())
    }

    /// ブラウザから戻ったqueryを照合し、認可コードと発行されたクライアントIDを返す。
    pub fn callback(&self, query: &str) -> std::result::Result<(String, String), Rejected> {
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let count = |name: &str| pairs.iter().filter(|(k, _)| k == name).count();
        let value = |name: &str| -> Option<&str> {
            let mut found = pairs.iter().filter(|(k, _)| k == name);
            match (found.next(), found.next()) {
                (Some((_, v)), None) => Some(v.as_str()),
                _ => None,
            }
        };
        let reject = |matched: bool, message: &str| Rejected {
            matched,
            error: anyhow!("{message}"),
        };
        let Some(state) = value("state") else {
            return Err(reject(false, "認証コールバックが不正です"));
        };
        if !equal_time(state, &self.state) {
            return Err(reject(false, "認証状態が一致しません"));
        }
        if count("error") > 0 {
            return Err(reject(true, "ChatGPTの認証がキャンセルされました"));
        }
        let issued = if count("client_id") > 0 {
            value("client_id")
        } else {
            self.registered_client()
        };
        let Some(issued) = issued.filter(|id| !id.is_empty() && *id != DYNAMIC && id.len() <= 512)
        else {
            return Err(reject(true, "発行されたクライアントIDがありません"));
        };
        if self.registered_client().is_some_and(|id| id != issued) {
            return Err(reject(true, "選択した登録とクライアントIDが一致しません"));
        }
        let Some(code) = value("code").filter(|c| !c.is_empty()) else {
            return Err(reject(true, "認証コールバックが不正です"));
        };
        Ok((code.to_owned(), issued.to_owned()))
    }
}

/// 1件の接続を受けて応答する。試みが終わったら`Some`。雑音(別のstate・別のパス)は試みを消費しない。
fn serve_one(
    mut stream: TcpStream,
    port: u16,
    attempt: &Attempt,
) -> Option<Result<(String, String)>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut head = Vec::new();
    let mut buffer = [0; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(n) => head.extend_from_slice(&buffer[..n]),
        }
        if head.len() > 16_000 {
            reply(&mut stream, 404, "Not found");
            return None;
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split(' ');
    let (method, target) = (request.next().unwrap_or_default(), request.next());
    let Some(target) = target.filter(|_| method == "GET") else {
        reply(&mut stream, 404, "Not found");
        return None;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" {
        reply(&mut stream, 404, "Not found");
        return None;
    }
    let expected = format!("127.0.0.1:{port}");
    let host_ok = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("host") && value.trim() == expected
        })
    });
    if !host_ok {
        reply(&mut stream, 400, "Bad request");
        return None;
    }
    match attempt.callback(query) {
        Ok(result) => {
            reply(
                &mut stream,
                200,
                "Hashi: Sign-in received. You can close this window.",
            );
            Some(Ok(result))
        }
        Err(rejected) => {
            reply(&mut stream, 400, "Hashi: Sign-in was rejected.");
            rejected.matched.then_some(Err(rejected.error))
        }
    }
}

fn reply(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// ブラウザからの戻りを待つ。取消・締切で止まる。
fn wait_callback(
    listener: &TcpListener,
    attempt: &Attempt,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<(String, String)> {
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            bail!("サインインを停止しました");
        }
        if Instant::now() >= deadline {
            bail!("サインインがタイムアウトしました。もう一度お試しください");
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(done) = serve_one(stream, port, attempt) {
                    return done;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => bail!("サインインの待受けに失敗しました"),
        }
    }
}

fn b64_json(part: &str) -> Result<Value> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| anyhow!("IDトークンを検証できません"))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("IDトークンを検証できません"))
}

/// 画面に出す登録。トークンは含めない。
#[derive(Clone, Debug, PartialEq)]
pub struct AccountView {
    pub id: String,
    pub label: String,
    /// 更新トークンを持っている(サインイン済み)。
    pub connected: bool,
}

/// ChatGPTのモデル1件。
#[derive(Clone, Debug, PartialEq)]
pub struct ChatGptModel {
    pub slug: String,
    pub display_name: String,
}

/// 同じプロセス内の取引を直列にする(ファイルロックはプロセス間)。
static PROCESS: Mutex<()> = Mutex::new(());

/// サインイン済みの登録(クライアントIDごと)。保存はAI専用の暗号化ストアの`oauth:accounts`。
pub struct Accounts {
    store: EncryptedSecrets,
    endpoints: Endpoints,
}

impl Accounts {
    pub fn new(store: EncryptedSecrets, endpoints: Endpoints) -> Self {
        Self { store, endpoints }
    }

    /// 取得・更新・保存を1つの取引にする。別のHashiが処理中なら3秒まで待つ。
    fn transaction<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        let _local = PROCESS.lock().unwrap_or_else(|e| e.into_inner());
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.store.root().join("ai-oauth.guard"))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while FileExt::try_lock_exclusive(&file).is_err() {
            if Instant::now() >= deadline {
                bail!("別のHashiがChatGPTの認証を処理中です。少し待ってください");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let result = work();
        let _ = FileExt::unlock(&file);
        result
    }

    fn load(&self) -> Result<Map<String, Value>> {
        let raw = self
            .store
            .get(ACCOUNTS)
            .map_err(|_| anyhow!("ChatGPTの保存データを読めません"))?;
        match raw {
            None => Ok(Map::new()),
            Some(raw) => match serde_json::from_str(&raw) {
                Ok(Value::Object(map)) => Ok(map),
                _ => bail!("ChatGPTの保存データを読めません"),
            },
        }
    }

    fn save(&self, accounts: &Map<String, Value>) -> Result<()> {
        self.store
            .set(ACCOUNTS, Some(&Value::Object(accounts.clone()).to_string()))
            .map_err(|_| anyhow!("ChatGPTの登録を保存できませんでした"))
    }

    pub fn list(&self) -> Result<Vec<AccountView>> {
        self.transaction(|| {
            Ok(self
                .load()?
                .iter()
                .map(|(id, record)| {
                    let who = record["email"]
                        .as_str()
                        .filter(|e| !e.is_empty())
                        .or_else(|| record["subject"].as_str())
                        .unwrap_or("ChatGPT");
                    let short: String = id.chars().take(10).collect();
                    AccountView {
                        id: id.clone(),
                        label: format!("{who} · {short}"),
                        connected: record["refresh_token"]
                            .as_str()
                            .is_some_and(|t| !t.is_empty()),
                    }
                })
                .collect())
        })
    }

    fn metadata(&self) -> Result<Value> {
        let data = http::get_json(
            &format!("{}/.well-known/openid-configuration", self.endpoints.issuer),
            &[],
        )?;
        if data["issuer"].as_str() != Some(self.endpoints.issuer.as_str()) {
            bail!("認証発行者が一致しません");
        }
        Ok(data)
    }

    /// IDトークンを検証して中身(claims)を返す。
    fn identity(&self, token: &str, client_id: &str, nonce: Option<&str>) -> Result<Value> {
        let parts: Vec<&str> = token.split('.').collect();
        let [head, body, signature] = parts[..] else {
            bail!("IDトークンを検証できません");
        };
        let header = b64_json(head)?;
        if header["alg"].as_str() != Some("RS256") {
            bail!("IDトークンの署名方式が不正です");
        }
        let kid = header["kid"].as_str();
        let jwks_uri = self.metadata()?["jwks_uri"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("IDトークンの署名鍵が見つかりません"))?;
        let jwks = http::get_json(&self.endpoints.same_host(&jwks_uri)?, &[])?;
        let matching: Vec<&Value> = jwks["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|k| {
                k["kid"].as_str() == kid
                    && k["kty"].as_str() == Some("RSA")
                    && k["use"].as_str().unwrap_or("sig") == "sig"
            })
            .collect();
        let [key] = matching[..] else {
            bail!("IDトークンの署名鍵が見つかりません");
        };
        let decode = |name: &str| {
            key[name]
                .as_str()
                .and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
                .ok_or_else(|| anyhow!("IDトークンの署名鍵が不正です"))
        };
        let (n, e) = (decode("n")?, decode("e")?);
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| anyhow!("IDトークンを検証できません"))?;
        ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
            .verify(
                &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                format!("{head}.{body}").as_bytes(),
                &signature,
            )
            .map_err(|_| anyhow!("IDトークンを検証できません"))?;
        let claims = b64_json(body)?;
        let time = now();
        let exp = claims["exp"].as_f64();
        let iat = claims["iat"].as_f64();
        let audience_ok = match &claims["aud"] {
            Value::String(aud) => aud == client_id,
            Value::Array(list) => list.iter().any(|a| a.as_str() == Some(client_id)),
            _ => false,
        };
        if claims["iss"].as_str() != Some(self.endpoints.issuer.as_str())
            || !audience_ok
            || exp.is_none_or(|exp| exp <= time)
            || iat.is_none_or(|iat| iat > time + 60.0)
            || claims["nbf"].as_f64().is_some_and(|nbf| nbf > time + 60.0)
        {
            bail!("IDトークンを検証できません");
        }
        if let Some(nonce) = nonce {
            if claims["nonce"]
                .as_str()
                .is_none_or(|n| !equal_time(n, nonce))
            {
                bail!("認証nonceが一致しません");
            }
        }
        if claims["sub"].as_str().is_none_or(str::is_empty) {
            bail!("アカウントIDが不正です");
        }
        Ok(claims)
    }

    /// トークン応答を登録の形にする。プランの使用許可と更新トークンが無ければ受け付けない。
    fn record(
        tokens: &Value,
        client_id: &str,
        identity: &Value,
        previous: Option<&Value>,
    ) -> Result<Value> {
        let scopes: Vec<String> = match tokens["scope"].as_str() {
            Some(scope) => scope.split_whitespace().map(str::to_owned).collect(),
            None => previous
                .and_then(|p| p["scopes"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
        };
        if !scopes.iter().any(|s| s == USE_SCOPE) {
            bail!("ChatGPTプランの使用許可がありません。もう一度サインインしてください");
        }
        let access = tokens["access_token"].as_str().filter(|t| !t.is_empty());
        let bearer = tokens["token_type"]
            .as_str()
            .is_some_and(|t| t.eq_ignore_ascii_case("bearer"));
        let (Some(access), true) = (access, bearer) else {
            bail!("アクセストークンが不正です");
        };
        let expires_in = tokens["expires_in"]
            .as_f64()
            .or_else(|| tokens["expires_in"].as_str().and_then(|s| s.parse().ok()))
            .filter(|v| *v > 0.0)
            .ok_or_else(|| anyhow!("アクセストークンの期限が不正です"))?;
        let mut record = previous
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        record.insert("client_id".into(), json!(client_id));
        record.insert("subject".into(), identity["sub"].clone());
        record.insert(
            "email".into(),
            json!(identity["email"].as_str().unwrap_or_default()),
        );
        record.insert("scopes".into(), json!(scopes));
        record.insert("access_token".into(), json!(access));
        record.insert("expires_at".into(), json!(now() + expires_in));
        for name in ["id_token", "refresh_token"] {
            if let Some(value) = tokens[name].as_str().filter(|v| !v.is_empty()) {
                record.insert(name.into(), json!(value));
            }
        }
        if record
            .get("refresh_token")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            bail!("更新トークンがありません");
        }
        Ok(Value::Object(record))
    }

    /// サインインする。`registration`を渡すと、その登録で再認証する(別アカウントなら拒否)。
    /// `open`はブラウザで開くURLを受け取る。成功したら登録のID(クライアントID)を返す。
    pub fn sign_in(
        &self,
        cancel: &CancellationToken,
        registration: Option<&str>,
        open: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        self.sign_in_with_timeout(cancel, registration, open, SIGN_IN_TIMEOUT)
    }

    fn sign_in_with_timeout(
        &self,
        cancel: &CancellationToken,
        registration_id: Option<&str>,
        open: &mut dyn FnMut(&str) -> bool,
        timeout: Duration,
    ) -> Result<String> {
        let (registration, host) = self.transaction(|| {
            let accounts = self.load()?;
            let registration = match registration_id {
                Some(id) => Some(
                    accounts
                        .get(id)
                        .cloned()
                        .ok_or_else(|| anyhow!("選択した登録がありません"))?,
                ),
                None => None,
            };
            let host = match self.store.get(HOST).ok().flatten() {
                Some(host) if !host.is_empty() => host,
                _ => {
                    let host = new_host_id()?;
                    self.store
                        .set(HOST, Some(&host))
                        .map_err(|_| anyhow!("ChatGPTの登録を保存できませんでした"))?;
                    host
                }
            };
            Ok((registration, host))
        })?;
        let attempt = Attempt::new(&host, registration.clone())?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| anyhow!("サインインの待受けを開けませんでした"))?;
        let redirect = format!(
            "http://127.0.0.1:{}/auth/callback",
            listener.local_addr()?.port()
        );
        // 認証URLにはstate等が入るのでログへ出さない。
        if !open(&attempt.authorization_url(&self.endpoints, &redirect)) {
            bail!("ブラウザを開けませんでした");
        }
        let (code, client_id) = wait_callback(&listener, &attempt, cancel, timeout)?;
        drop(listener);
        let tokens = http::post_form(
            &self.endpoints.token(),
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &client_id),
                ("code", &code),
                ("code_verifier", &attempt.verifier),
                ("redirect_uri", &redirect),
                ("resource", &self.endpoints.resource),
            ],
        )
        .map_err(|e| anyhow!("ChatGPTのトークンを受け取れませんでした（{e}）"))?;
        let id_token = tokens["id_token"]
            .as_str()
            .ok_or_else(|| anyhow!("IDトークンがありません"))?;
        let identity = self.identity(id_token, &client_id, Some(&attempt.nonce))?;
        if let Some(registration) = &registration {
            if registration["subject"] != identity["sub"] {
                bail!("選択したChatGPTアカウントと一致しません");
            }
        }
        let record = Self::record(&tokens, &client_id, &identity, registration.as_ref())?;
        if cancel.is_cancelled() {
            bail!("サインインを停止しました");
        }
        self.transaction(|| {
            let mut accounts = self.load()?;
            accounts.insert(client_id.clone(), record);
            self.save(&accounts)
        })?;
        Ok(client_id)
    }

    /// 使えるアクセストークン。期限の60秒前を過ぎていたら更新してから返す。
    pub fn access_token(&self, id: &str) -> Result<String> {
        self.transaction(|| {
            let mut accounts = self.load()?;
            let record = accounts
                .get(id)
                .cloned()
                .filter(|r| r["refresh_token"].as_str().is_some_and(|t| !t.is_empty()))
                .ok_or_else(|| {
                    anyhow!("ChatGPTからサインアウトしています。AIモデルの管理でサインインしてください")
                })?;
            if !record["scopes"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v.as_str() == Some(USE_SCOPE)))
            {
                bail!("ChatGPTプランの使用許可がありません");
            }
            if record["expires_at"].as_f64().unwrap_or(0.0) > now() + 60.0 {
                return Ok(record["access_token"].as_str().unwrap_or_default().to_owned());
            }
            let client = record["client_id"].as_str().unwrap_or(id).to_owned();
            let refresh = record["refresh_token"].as_str().unwrap_or_default();
            let tokens = http::post_form(
                &self.endpoints.token(),
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &client),
                    ("refresh_token", refresh),
                    ("resource", &self.endpoints.resource),
                ],
            )
            .map_err(|e| anyhow!("ChatGPTのトークンを更新できませんでした（{e}）。もう一度サインインしてください"))?;
            let mut identity =
                json!({"sub": record["subject"], "email": record["email"].as_str().unwrap_or_default()});
            if let Some(id_token) = tokens["id_token"].as_str() {
                identity = self.identity(id_token, &client, None)?;
                if identity["sub"] != record["subject"] {
                    bail!("更新後のアカウントが一致しません");
                }
            }
            let updated = Self::record(&tokens, &client, &identity, Some(&record))?;
            let access = updated["access_token"].as_str().unwrap_or_default().to_owned();
            accounts.insert(id.to_owned(), updated);
            self.save(&accounts)?;
            Ok(access)
        })
    }

    /// このアカウントで使えるモデル(一覧に出してよいものだけ)。
    pub fn models(&self, id: &str) -> Result<Vec<ChatGptModel>> {
        let token = self.access_token(id)?;
        let data = http::get_json(
            &format!("{}/models", self.endpoints.resource),
            &[("Authorization".into(), format!("Bearer {token}"))],
        )?;
        Ok(data["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["visibility"].as_str() == Some("list"))
            .filter_map(|m| {
                let slug = m["slug"].as_str().filter(|s| !s.is_empty())?;
                Some(ChatGptModel {
                    slug: slug.to_owned(),
                    display_name: m["display_name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(slug)
                        .to_owned(),
                })
            })
            .collect())
    }

    /// サインアウト。遠隔の失効を試み、ローカルのトークンは必ず消す(登録の名前は残す)。
    /// 遠隔の失効を確認できたら`true`。
    pub fn sign_out(&self, id: &str) -> Result<bool> {
        self.transaction(|| {
            let mut accounts = self.load()?;
            let Some(record) = accounts.get(id).cloned() else {
                return Ok(true);
            };
            let mut revoked = true;
            if let Some(refresh) = record["refresh_token"].as_str().filter(|t| !t.is_empty()) {
                let client = record["client_id"].as_str().unwrap_or(id);
                revoked = self
                    .metadata()
                    .and_then(|m| {
                        let endpoint = m["revocation_endpoint"]
                            .as_str()
                            .ok_or_else(|| anyhow!("失効の接続先がありません"))?;
                        self.endpoints.same_host(endpoint)
                    })
                    .and_then(|endpoint| {
                        http::post_form_status(
                            &endpoint,
                            &[
                                ("token", refresh),
                                ("token_type_hint", "refresh_token"),
                                ("client_id", client),
                            ],
                        )
                    })
                    .is_ok();
            }
            let kept: Map<String, Value> = ["client_id", "subject", "email"]
                .iter()
                .filter_map(|k| record.get(*k).map(|v| ((*k).to_owned(), v.clone())))
                .collect();
            accounts.insert(id.to_owned(), Value::Object(kept));
            self.save(&accounts)?;
            Ok(revoked)
        })
    }

    /// 登録を丸ごと消す(サインアウト済みのものだけ)。
    pub fn forget(&self, id: &str) -> Result<()> {
        self.transaction(|| {
            let mut accounts = self.load()?;
            if accounts
                .get(id)
                .is_some_and(|r| r["refresh_token"].as_str().is_some_and(|t| !t.is_empty()))
            {
                bail!("先にサインアウトしてください");
            }
            accounts.remove(id);
            self.save(&accounts)
        })
    }
}

/// サインインした登録で会話する接続。要求のたびに最新のトークンを使い、関数はnamespaceでまとめる。
/// 通信はしない(トークンの取得は最初の要求のとき)。
pub fn provider(
    accounts: Arc<Accounts>,
    id: &str,
    model: &str,
    use_tools: bool,
) -> Result<ApiProvider> {
    let resource = accounts.endpoints.resource.clone();
    let id = id.to_owned();
    let mut provider =
        ApiProvider::new("openai", model, "", Some(&resource), use_tools, "responses")?
            .with_token_source(Arc::new(move || accounts.access_token(&id)));
    provider.tool_namespace = Some(NAMESPACE.into());
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{RsaKeyPair, RsaPublicKeyComponents, RSA_PKCS1_SHA256};
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const KEY: &[u8] = include_bytes!("../testdata/oauth-test-only-rsa.pk8.der");

    fn key_pair() -> RsaKeyPair {
        RsaKeyPair::from_pkcs8(KEY).unwrap()
    }

    fn jwk(kid: &str) -> Value {
        let parts: RsaPublicKeyComponents<Vec<u8>> = key_pair().public().into();
        json!({"kty": "RSA", "use": "sig", "kid": kid,
            "n": URL_SAFE_NO_PAD.encode(&parts.n), "e": URL_SAFE_NO_PAD.encode(&parts.e)})
    }

    fn sign(header: Value, claims: Value) -> String {
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let pair = key_pair();
        let mut signature = vec![0; pair.public().modulus_len()];
        pair.sign(
            &RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            input.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    /// 偽サーバーが受けた要求(パスとフォーム)。
    type Request = (String, HashMap<String, String>);

    /// 偽の認証サーバー。受けた要求を記録し、トークン交換ではPKCEを照合する。
    struct Fake {
        issuer: String,
        requests: Arc<Mutex<Vec<Request>>>,
        /// 発行するIDトークンのnonce(サインインの試みから受け取る)。
        nonce: Arc<Mutex<String>>,
        challenge: Arc<Mutex<String>>,
        subject: Arc<Mutex<String>>,
        expires_in: Arc<AtomicUsize>,
    }

    fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
        let mut data = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = stream.read(&mut buffer).ok()?;
            if n == 0 {
                return None;
            }
            data.extend_from_slice(&buffer[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end]).to_string();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while data.len() < end + 4 + length {
                    let n = stream.read(&mut buffer).ok()?;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..n]);
                }
                let body = String::from_utf8_lossy(&data[end + 4..]).to_string();
                return Some((head.lines().next()?.to_owned(), body));
            }
        }
    }

    impl Fake {
        fn start() -> Fake {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let issuer = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let fake = Fake {
                issuer: issuer.clone(),
                requests: Arc::default(),
                nonce: Arc::default(),
                challenge: Arc::default(),
                subject: Arc::new(Mutex::new("user-1".into())),
                expires_in: Arc::new(AtomicUsize::new(3600)),
            };
            let (requests, nonce, challenge, subject, expires) = (
                fake.requests.clone(),
                fake.nonce.clone(),
                fake.challenge.clone(),
                fake.subject.clone(),
                fake.expires_in.clone(),
            );
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let Some((line, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
                    let form: HashMap<String, String> =
                        url::form_urlencoded::parse(body.as_bytes())
                            .into_owned()
                            .collect();
                    requests.lock().unwrap().push((path.clone(), form.clone()));
                    let (status, payload) = match path.as_str() {
                        "/.well-known/openid-configuration" => (
                            200,
                            json!({"issuer": issuer, "jwks_uri": format!("{issuer}/jwks"),
                                "revocation_endpoint": format!("{issuer}/revoke")}),
                        ),
                        "/jwks" => (200, json!({"keys": [jwk("other-kid-x"), jwk("k1")]})),
                        "/api/accounts/oauth/token" => {
                            let client = form.get("client_id").cloned().unwrap_or_default();
                            let pkce_ok = form.get("grant_type").map(String::as_str)
                                == Some("refresh_token")
                                || form.get("code_verifier").is_some_and(|v| {
                                    URL_SAFE_NO_PAD.encode(ring::digest::digest(
                                        &ring::digest::SHA256,
                                        v.as_bytes(),
                                    )) == *challenge.lock().unwrap()
                                });
                            if !pkce_ok {
                                (400, json!({"error": "invalid_grant"}))
                            } else {
                                let now = now() as u64;
                                let id_token = sign(
                                    json!({"alg": "RS256", "kid": "k1"}),
                                    json!({"iss": issuer, "aud": client, "sub": *subject.lock().unwrap(),
                                        "email": "me@example.com", "iat": now, "exp": now + 600,
                                        "nonce": *nonce.lock().unwrap()}),
                                );
                                (
                                    200,
                                    json!({"access_token": format!("at-{}", requests.lock().unwrap().len()),
                                        "token_type": "Bearer", "expires_in": expires.load(Ordering::SeqCst),
                                        "refresh_token": "rt-1", "id_token": id_token,
                                        "scope": SCOPES}),
                                )
                            }
                        }
                        "/revoke" => (200, json!({})),
                        "/v1/models" => (
                            200,
                            json!({"models": [
                                {"slug": "gpt-visible", "display_name": "Visible", "visibility": "list"},
                                {"slug": "gpt-hidden", "visibility": "hide"}]}),
                        ),
                        _ => (404, json!({})),
                    };
                    let body = payload.to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            fake
        }

        fn endpoints(&self) -> Endpoints {
            Endpoints {
                issuer: self.issuer.clone(),
                resource: format!("{}/v1", self.issuer),
            }
        }

        fn paths(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|(p, _)| p.clone())
                .collect()
        }
    }

    fn accounts(fake: &Fake) -> (tempfile::TempDir, Accounts) {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedSecrets::with_root(
            dir.path().to_owned(),
            hashi_storage::secrets::Namespace::Ai,
        )
        .unwrap();
        (dir, Accounts::new(store, fake.endpoints()))
    }

    fn query_of(url: &str) -> HashMap<String, String> {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    /// ブラウザの代わりに、認証URLを読んでコールバックへ戻る。
    fn browser<'a>(
        fake: &'a Fake,
        client_id: Option<&'static str>,
    ) -> impl FnMut(&str) -> bool + 'a {
        move |url: &str| {
            let query = query_of(url);
            *fake.nonce.lock().unwrap() = query["nonce"].clone();
            *fake.challenge.lock().unwrap() = query["code_challenge"].clone();
            let redirect = url::Url::parse(&query["redirect_uri"]).unwrap();
            let port = redirect.port().unwrap();
            let mut back = format!("/auth/callback?state={}&code=code-1", query["state"]);
            if let Some(client) = client_id {
                back.push_str(&format!("&client_id={client}"));
            }
            std::thread::spawn(move || {
                // 別のstateの雑音は試みを終わらせない。
                for target in ["/auth/callback?state=wrong&code=x", "/other", back.as_str()] {
                    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    write!(
                        stream,
                        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
                    )
                    .unwrap();
                    let mut response = String::new();
                    let _ = stream.read_to_string(&mut response);
                }
            });
            true
        }
    }

    #[test]
    fn authorization_url_carries_pkce_state_and_the_dynamic_client() {
        let endpoints = Endpoints::official();
        let attempt = Attempt::new("urn:uuid:host", None).unwrap();
        let url = attempt.authorization_url(&endpoints, "http://127.0.0.1:5/auth/callback");
        assert!(url.starts_with("https://auth.openai.com/api/accounts/authorize?"));
        let q = query_of(&url);
        assert_eq!(q["client_id"], DYNAMIC);
        assert_eq!(q["agent_name_hint"], "Hashi");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["scope"], SCOPES);
        assert_eq!(q["resource"], RESOURCE);
        assert_eq!(q["state"], attempt.state);
        assert_ne!(q["code_challenge"], attempt.verifier);
        let again = Attempt::new("urn:uuid:host", None).unwrap();
        assert_ne!(again.state, attempt.state);
        let registered = Attempt::new(
            "urn:uuid:host",
            Some(json!({"client_id": "issued-1", "email": "me@example.com", "id_token": "t"})),
        )
        .unwrap();
        let q =
            query_of(&registered.authorization_url(&endpoints, "http://127.0.0.1:5/auth/callback"));
        assert_eq!(q["client_id"], "issued-1");
        assert_eq!(q["login_hint"], "me@example.com");
        assert!(!q.contains_key("agent_name_hint"));
    }

    #[test]
    fn callback_checks_state_client_and_duplicates() {
        let attempt = Attempt::new("h", None).unwrap();
        let state = attempt.state.clone();
        let ok = attempt.callback(&format!("state={state}&code=c&client_id=issued"));
        assert_eq!(ok.unwrap(), ("c".into(), "issued".into()));
        let noise = attempt
            .callback("state=other&code=c&client_id=issued")
            .unwrap_err();
        assert!(!noise.matched);
        let cancelled = attempt
            .callback(&format!("state={state}&error=access_denied"))
            .unwrap_err();
        assert!(cancelled.matched);
        for bad in [
            format!("state={state}&code=c"),
            format!("state={state}&code=c&client_id={DYNAMIC}"),
            format!("state={state}&state={state}&code=c&client_id=i"),
            format!("state={state}&client_id=i"),
        ] {
            assert!(attempt.callback(&bad).is_err(), "{bad}");
        }
        let registered = Attempt::new("h", Some(json!({"client_id": "mine"}))).unwrap();
        let s = registered.state.clone();
        assert_eq!(
            registered.callback(&format!("state={s}&code=c")).unwrap().1,
            "mine"
        );
        assert!(registered
            .callback(&format!("state={s}&code=c&client_id=other"))
            .is_err());
    }

    #[test]
    fn id_token_signature_audience_expiry_and_nonce_are_verified() {
        let fake = Fake::start();
        let (_dir, accounts) = accounts(&fake);
        let t = now() as u64;
        let good = json!({"iss": fake.issuer, "aud": "client", "sub": "u", "iat": t, "exp": t + 60, "nonce": "n"});
        let header = json!({"alg": "RS256", "kid": "k1"});
        let token = sign(header.clone(), good.clone());
        assert_eq!(
            accounts.identity(&token, "client", Some("n")).unwrap()["sub"],
            "u"
        );
        let with = |patch: Value| {
            let mut claims = good.clone();
            claims
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            sign(header.clone(), claims)
        };
        assert!(accounts.identity(&token, "other", Some("n")).is_err());
        assert!(accounts.identity(&token, "client", Some("x")).is_err());
        assert!(accounts
            .identity(&with(json!({"exp": t - 1})), "client", None)
            .is_err());
        assert!(accounts
            .identity(
                &with(json!({"iss": "https://evil.example"})),
                "client",
                None
            )
            .is_err());
        assert!(accounts
            .identity(&with(json!({"sub": ""})), "client", None)
            .is_err());
        assert!(accounts
            .identity(&with(json!({"aud": ["a", "client"]})), "client", None)
            .is_ok());
        assert!(accounts
            .identity(
                &sign(json!({"alg": "HS256", "kid": "k1"}), good.clone()),
                "client",
                None
            )
            .is_err());
        assert!(accounts
            .identity(
                &sign(json!({"alg": "RS256", "kid": "nope"}), good.clone()),
                "client",
                None
            )
            .is_err());
        // 署名後に中身を書き換えたものは通らない。
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        parts[1] = URL_SAFE_NO_PAD.encode(
            json!({"iss": fake.issuer, "aud": "client", "sub": "admin", "iat": t, "exp": t + 60})
                .to_string(),
        );
        assert!(accounts.identity(&parts.join("."), "client", None).is_err());
    }

    #[test]
    fn sign_in_refresh_models_and_sign_out_end_to_end() {
        let fake = Fake::start();
        let (_dir, accounts) = accounts(&fake);
        let cancel = CancellationToken::new();
        let id = accounts
            .sign_in(&cancel, None, &mut browser(&fake, Some("issued-1")))
            .unwrap();
        assert_eq!(id, "issued-1");
        let list = accounts.list().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].connected && list[0].label.starts_with("me@example.com"));
        // 保存データに認可コードやverifierは残さない。
        let saved = accounts.store.get(ACCOUNTS).unwrap().unwrap();
        assert!(!saved.contains("code-1"));
        // 期限内はそのまま、期限が近ければ更新する。
        let first = accounts.access_token(&id).unwrap();
        assert_eq!(accounts.access_token(&id).unwrap(), first);
        accounts
            .transaction(|| {
                let mut all = accounts.load()?;
                all.get_mut(&id).unwrap()["expires_at"] = json!(now() + 10.0);
                accounts.save(&all)
            })
            .unwrap();
        let refreshed = accounts.access_token(&id).unwrap();
        assert_ne!(refreshed, first);
        assert!(fake
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(p, f)| p.ends_with("/token")
                && f.get("grant_type").map(String::as_str) == Some("refresh_token")));
        let models = accounts.models(&id).unwrap();
        assert_eq!(
            models,
            vec![ChatGptModel {
                slug: "gpt-visible".into(),
                display_name: "Visible".into()
            }]
        );
        // 同じ登録で再認証できる。別アカウントは拒否する。
        assert_eq!(
            accounts
                .sign_in(&cancel, Some(&id), &mut browser(&fake, None))
                .unwrap(),
            id
        );
        *fake.subject.lock().unwrap() = "someone-else".into();
        let error = accounts
            .sign_in(&cancel, Some(&id), &mut browser(&fake, None))
            .unwrap_err();
        assert!(error.to_string().contains("一致しません"), "{error}");
        assert!(accounts.forget(&id).is_err());
        assert!(accounts.sign_out(&id).unwrap());
        assert!(fake.paths().iter().any(|p| p == "/revoke"));
        assert!(!accounts.list().unwrap()[0].connected);
        assert!(accounts.access_token(&id).is_err());
        let saved = accounts.store.get(ACCOUNTS).unwrap().unwrap();
        assert!(!saved.contains("rt-1") && !saved.contains("access_token"));
        accounts.forget(&id).unwrap();
        assert!(accounts.list().unwrap().is_empty());
    }

    #[test]
    fn sign_in_stops_on_cancel_timeout_and_a_closed_browser() {
        let fake = Fake::start();
        let (_dir, accounts) = accounts(&fake);
        let cancel = CancellationToken::new();
        let error = accounts
            .sign_in_with_timeout(&cancel, None, &mut |_| true, Duration::from_millis(300))
            .unwrap_err();
        assert!(error.to_string().contains("タイムアウト"));
        assert!(accounts.sign_in(&cancel, None, &mut |_| false).is_err());
        cancel.cancel();
        let error = accounts.sign_in(&cancel, None, &mut |_| true).unwrap_err();
        assert!(error.to_string().contains("停止"));
        assert!(accounts.list().unwrap().is_empty());
        // 存在しない登録での再認証は始めない。
        assert!(accounts
            .sign_in(&CancellationToken::new(), Some("nope"), &mut |_| true)
            .is_err());
    }

    #[test]
    fn plan_scope_and_refresh_token_are_required() {
        let identity = json!({"sub": "u", "email": "e"});
        let base = json!({"access_token": "a", "token_type": "bearer", "expires_in": 60,
            "refresh_token": "r", "scope": SCOPES});
        assert!(Accounts::record(&base, "c", &identity, None).is_ok());
        let mut no_scope = base.clone();
        no_scope["scope"] = json!("openid");
        assert!(Accounts::record(&no_scope, "c", &identity, None).is_err());
        let mut no_refresh = base.clone();
        no_refresh.as_object_mut().unwrap().remove("refresh_token");
        assert!(Accounts::record(&no_refresh, "c", &identity, None).is_err());
        // 更新応答に無い更新トークン・scopeは前の登録から引き継ぐ。
        let previous = Accounts::record(&base, "c", &identity, None).unwrap();
        let mut renewed = base.clone();
        renewed.as_object_mut().unwrap().remove("refresh_token");
        renewed.as_object_mut().unwrap().remove("scope");
        let record = Accounts::record(&renewed, "c", &identity, Some(&previous)).unwrap();
        assert_eq!(record["refresh_token"], "r");
        let mut mac = base;
        mac["token_type"] = json!("mac");
        assert!(Accounts::record(&mac, "c", &identity, None).is_err());
    }

    #[test]
    fn metadata_may_not_point_to_another_host() {
        let endpoints = Endpoints::official();
        assert!(endpoints.same_host("https://auth.openai.com/jwks").is_ok());
        assert!(endpoints.same_host("https://evil.example/jwks").is_err());
        assert!(endpoints.same_host("http://auth.openai.com/jwks").is_err());
        assert!(endpoints
            .same_host("https://auth.openai.com:8443/jwks")
            .is_err());
    }

    #[test]
    fn provider_wraps_tools_in_a_namespace_and_unwraps_call_names() {
        let fake = Fake::start();
        let (_dir, accounts) = accounts(&fake);
        let provider = provider(
            Arc::new(Accounts::new(accounts.store, Endpoints::official())),
            "id",
            "gpt-x",
            true,
        )
        .unwrap();
        assert_eq!(provider.key(), "");
        let tools = vec![json!({"name": "list_terminals", "description": "d", "parameters": {}})];
        let payload = provider.responses_payload(&[], "i", &tools);
        assert_eq!(payload["tools"][0]["type"], "namespace");
        assert_eq!(payload["tools"][0]["name"], NAMESPACE);
        assert_eq!(payload["tools"][0]["tools"][0]["type"], "function");
        assert_eq!(payload["store"], false);
        // サインアウト中は要求の前に止まる(トークンを送らない)。
        let error = provider
            .respond(&[], "i", &tools, &CancellationToken::new(), &mut |_| {})
            .unwrap_err();
        assert!(error.to_string().contains("サインイン"), "{error}");
    }
}
