use crate::launcher::LauncherPaths;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{distributions::Alphanumeric, Rng};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

const TOKEN_ENDPOINT: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const LOOPBACK_REDIRECT_HOST: &str = "127.0.0.1";
const XBL_AUTH_ENDPOINT: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_ENDPOINT: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_ENDPOINT: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_PROFILE_ENDPOINT: &str = "https://api.minecraftservices.com/minecraft/profile";
const MINECRAFT_ENTITLEMENTS_ENDPOINT: &str =
    "https://api.minecraftservices.com/entitlements/mcstore";

type PendingLoginHandle = Arc<Mutex<PendingLogin>>;
type PendingLoginMap = HashMap<String, PendingLoginHandle>;

#[derive(Debug)]
enum PendingLogin {
    Waiting,
    Complete(MinecraftSession),
    Failed(String),
}

static PENDING_LOGINS: OnceLock<Mutex<PendingLoginMap>> = OnceLock::new();

fn pending_logins() -> &'static Mutex<PendingLoginMap> {
    PENDING_LOGINS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftSession {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub xuid: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginPollStatus {
    Pending,
    SlowDown,
    Complete(MinecraftSession),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Microsoft login client id is not configured. Set TOWNRISE_MS_CLIENT_ID at build/runtime or create microsoft-client-id.txt in the launcher data folder.")]
    MissingClientId,
    #[error("Microsoft login request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Microsoft login failed: {0}")]
    OAuth(String),
    #[error("Xbox account is not linked or XSTS authorization failed: {0}")]
    Xsts(String),
    #[error("This Microsoft account does not own Minecraft Java Edition")]
    MissingMinecraftOwnership,
    #[error(
        "Minecraft profile is missing. Create a Java profile name in the official launcher first."
    )]
    MissingMinecraftProfile,
    #[error("failed to save login session: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse login session: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxTokenResponse {
    token: String,
    display_claims: XboxDisplayClaims,
}

#[derive(Debug, Deserialize)]
struct XboxDisplayClaims {
    xui: Vec<XboxUserInfo>,
}

#[derive(Debug, Deserialize)]
struct XboxUserInfo {
    uhs: String,
    #[serde(default)]
    xid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MinecraftLoginResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfileResponse {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EntitlementsResponse {
    items: Vec<EntitlementItem>,
}

#[derive(Debug, Deserialize)]
struct EntitlementItem {
    name: String,
}

pub async fn begin_login(paths: &LauncherPaths) -> Result<LoginStart, AuthError> {
    let client_id = microsoft_client_id(paths)?;
    let listener = TcpListener::bind((LOOPBACK_REDIRECT_HOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://{LOOPBACK_REDIRECT_HOST}:{port}/callback");
    let login_id = random_token(32);
    let oauth_state = random_token(32);
    let code_verifier = random_token(96);
    let code_challenge = pkce_challenge(&code_verifier);
    let auth_url =
        microsoft_authorize_url(&client_id, &redirect_uri, &oauth_state, &code_challenge);
    let pending = Arc::new(Mutex::new(PendingLogin::Waiting));
    pending_logins()
        .lock()
        .expect("pending login lock poisoned")
        .insert(login_id.clone(), pending.clone());
    let paths_for_task = paths.clone();
    tokio::spawn(async move {
        let result = run_loopback_callback_server(
            listener,
            paths_for_task,
            client_id,
            redirect_uri,
            oauth_state,
            code_verifier,
        )
        .await;
        let mut guard = pending.lock().expect("pending login lock poisoned");
        *guard = match result {
            Ok(session) => PendingLogin::Complete(session),
            Err(error) => PendingLogin::Failed(error.to_string()),
        };
    });

    Ok(LoginStart {
        message: "브라우저에 Microsoft 로그인 화면을 열었습니다. 로그인 후 런처로 돌아오세요."
            .into(),
        device_code: login_id,
        user_code: String::new(),
        verification_uri: auth_url.clone(),
        verification_uri_complete: Some(auth_url),
        expires_in: 900,
        interval: 2,
    })
}

pub async fn poll_login(
    paths: &LauncherPaths,
    device_code: &str,
) -> Result<LoginPollStatus, AuthError> {
    if let Some(handle) = pending_logins()
        .lock()
        .expect("pending login lock poisoned")
        .get(device_code)
        .cloned()
    {
        let status = handle.lock().expect("pending login lock poisoned");
        return match &*status {
            PendingLogin::Waiting => Ok(LoginPollStatus::Pending),
            PendingLogin::Complete(session) => {
                let session = session.clone();
                drop(status);
                pending_logins()
                    .lock()
                    .expect("pending login lock poisoned")
                    .remove(device_code);
                Ok(LoginPollStatus::Complete(session))
            }
            PendingLogin::Failed(message) => Err(AuthError::OAuth(message.clone())),
        };
    }

    let client_id = microsoft_client_id(paths)?;
    let response = reqwest::Client::new()
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id.as_str()),
            ("device_code", device_code),
        ])
        .send()
        .await?;
    let status = response.status();
    let token: TokenResponse = response.json().await?;
    if status == StatusCode::BAD_REQUEST {
        match token.error.as_deref() {
            Some("authorization_pending") => return Ok(LoginPollStatus::Pending),
            Some("slow_down") => return Ok(LoginPollStatus::SlowDown),
            Some("authorization_declined") => {
                return Err(AuthError::OAuth("사용자가 로그인을 취소했습니다.".into()))
            }
            Some("expired_token") => {
                return Err(AuthError::OAuth(
                    "로그인 코드가 만료되었습니다. 다시 시도하세요.".into(),
                ))
            }
            _ => {}
        }
    }
    if !status.is_success() {
        return Err(AuthError::OAuth(
            token
                .error_description
                .unwrap_or_else(|| format!("HTTP {status}")),
        ));
    }
    let microsoft_access_token = token
        .access_token
        .ok_or_else(|| AuthError::OAuth("Microsoft access token is missing".into()))?;
    let session =
        complete_minecraft_login(&microsoft_access_token, token.expires_in.unwrap_or(3600)).await?;
    save_session(paths, &session).await?;
    Ok(LoginPollStatus::Complete(session))
}

pub async fn load_session(paths: &LauncherPaths) -> Result<Option<MinecraftSession>, AuthError> {
    let path = paths.auth_session_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(path).await?;
    let session: MinecraftSession = serde_json::from_str(&raw)?;
    if session.expires_at <= now_unix().saturating_add(60) {
        return Ok(None);
    }
    Ok(Some(session))
}

pub async fn logout(paths: &LauncherPaths) -> Result<(), AuthError> {
    let path = paths.auth_session_path();
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

async fn run_loopback_callback_server(
    listener: TcpListener,
    paths: LauncherPaths,
    client_id: String,
    redirect_uri: String,
    expected_state: String,
    code_verifier: String,
) -> Result<MinecraftSession, AuthError> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buffer = vec![0_u8; 8192];
        let read = stream.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..read]);
        let Some(first_line) = request.lines().next() else {
            write_loopback_response(&mut stream, 400, "요청을 읽지 못했습니다.").await?;
            continue;
        };
        let Some(target) = first_line.split_whitespace().nth(1) else {
            write_loopback_response(&mut stream, 400, "요청 경로를 읽지 못했습니다.").await?;
            continue;
        };
        if target == "/favicon.ico" {
            write_loopback_response(&mut stream, 204, "").await?;
            continue;
        }
        let url = format!("http://{LOOPBACK_REDIRECT_HOST}{target}");
        let parsed = Url::parse(&url).map_err(|error| AuthError::OAuth(error.to_string()))?;
        if parsed.path() != "/callback" {
            write_loopback_response(&mut stream, 404, "TownRise 로그인 콜백 주소가 아닙니다.")
                .await?;
            continue;
        }
        let mut code = None;
        let mut state = None;
        let mut oauth_error = None;
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                "error" => oauth_error = Some(value.into_owned()),
                _ => {}
            }
        }
        if let Some(error) = oauth_error {
            write_loopback_response(
                &mut stream,
                400,
                "Microsoft 로그인이 취소되었거나 실패했습니다.",
            )
            .await?;
            return Err(AuthError::OAuth(error));
        }
        if state.as_deref() != Some(expected_state.as_str()) {
            write_loopback_response(&mut stream, 400, "로그인 상태값이 일치하지 않습니다.").await?;
            return Err(AuthError::OAuth("OAuth state mismatch".into()));
        }
        let Some(code) = code else {
            write_loopback_response(&mut stream, 400, "Microsoft 인증 코드가 없습니다.").await?;
            return Err(AuthError::OAuth("authorization code is missing".into()));
        };
        let session =
            exchange_authorization_code(&client_id, &redirect_uri, &code_verifier, &code).await?;
        save_session(&paths, &session).await?;
        write_loopback_response(
            &mut stream,
            200,
            "Microsoft 로그인이 완료되었습니다. 이 창을 닫고 TownRise Launcher로 돌아가세요.",
        )
        .await?;
        return Ok(session);
    }
}

async fn exchange_authorization_code(
    client_id: &str,
    redirect_uri: &str,
    code_verifier: &str,
    code: &str,
) -> Result<MinecraftSession, AuthError> {
    let token: TokenResponse = reqwest::Client::new()
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let microsoft_access_token = token
        .access_token
        .ok_or_else(|| AuthError::OAuth("Microsoft access token is missing".into()))?;
    complete_minecraft_login(&microsoft_access_token, token.expires_in.unwrap_or(3600)).await
}

async fn write_loopback_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body_text: &str,
) -> Result<(), AuthError> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body = if status == 204 {
        String::new()
    } else {
        format!(
            r#"<!doctype html><meta charset="utf-8"><title>TownRise 로그인</title><body style="font-family:system-ui;background:#17140f;color:#fff4d6;padding:40px"><h1>TownRise Launcher</h1><p>{body_text}</p></body>"#
        )
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

fn microsoft_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let mut url = Url::parse("https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize")
        .expect("static Microsoft authorize URL is valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

async fn complete_minecraft_login(
    ms_access_token: &str,
    fallback_expires_in: u64,
) -> Result<MinecraftSession, AuthError> {
    let client = reqwest::Client::new();
    let xbl: XboxTokenResponse = client
        .post(XBL_AUTH_ENDPOINT)
        .json(&serde_json::json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_access_token}") },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let xsts_response = client
        .post(XSTS_AUTH_ENDPOINT)
        .json(&serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()
        .await?;
    if !xsts_response.status().is_success() {
        let body = xsts_response.text().await.unwrap_or_default();
        return Err(AuthError::Xsts(body));
    }
    let xsts: XboxTokenResponse = xsts_response.json().await?;
    let user = xsts
        .display_claims
        .xui
        .first()
        .ok_or_else(|| AuthError::Xsts("XSTS user hash is missing".into()))?;
    let identity_token = format!("XBL3.0 x={};{}", user.uhs, xsts.token);
    let mc: MinecraftLoginResponse = client
        .post(MINECRAFT_LOGIN_ENDPOINT)
        .json(&serde_json::json!({ "identityToken": identity_token }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let entitlements: EntitlementsResponse = client
        .get(MINECRAFT_ENTITLEMENTS_ENDPOINT)
        .bearer_auth(&mc.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !entitlements
        .items
        .iter()
        .any(|item| item.name == "game_minecraft" || item.name == "product_minecraft")
    {
        return Err(AuthError::MissingMinecraftOwnership);
    }

    let profile: MinecraftProfileResponse = client
        .get(MINECRAFT_PROFILE_ENDPOINT)
        .bearer_auth(&mc.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let username = profile.name.ok_or(AuthError::MissingMinecraftProfile)?;
    let uuid = profile.id.ok_or(AuthError::MissingMinecraftProfile)?;
    Ok(MinecraftSession {
        username,
        uuid,
        access_token: mc.access_token,
        xuid: user.xid.clone().unwrap_or_default(),
        expires_at: now_unix().saturating_add(mc.expires_in.min(fallback_expires_in)),
    })
}

async fn save_session(paths: &LauncherPaths, session: &MinecraftSession) -> Result<(), AuthError> {
    tokio::fs::create_dir_all(&paths.root_dir).await?;
    tokio::fs::write(
        paths.auth_session_path(),
        serde_json::to_string_pretty(session)?,
    )
    .await?;
    Ok(())
}

fn microsoft_client_id(paths: &LauncherPaths) -> Result<String, AuthError> {
    if let Ok(value) = std::env::var("TOWNRISE_MS_CLIENT_ID") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(value) = option_env!("TOWNRISE_MS_CLIENT_ID") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let path = paths.root_dir.join("microsoft-client-id.txt");
    if let Ok(value) = std::fs::read_to_string(path) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(AuthError::MissingClientId)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
