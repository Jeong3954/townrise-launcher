use crate::launcher::LauncherPaths;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const DEVICE_CODE_ENDPOINT: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_ENDPOINT: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const XBL_AUTH_ENDPOINT: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_ENDPOINT: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_ENDPOINT: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_PROFILE_ENDPOINT: &str = "https://api.minecraftservices.com/minecraft/profile";
const MINECRAFT_ENTITLEMENTS_ENDPOINT: &str =
    "https://api.minecraftservices.com/entitlements/mcstore";

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
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
    message: Option<String>,
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
    let response: DeviceCodeResponse = reqwest::Client::new()
        .post(DEVICE_CODE_ENDPOINT)
        .form(&[("client_id", client_id.as_str()), ("scope", SCOPE)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(LoginStart {
        message: response.message.unwrap_or_else(|| {
            format!(
                "{} 에 접속한 뒤 코드 {} 를 입력하세요.",
                response.verification_uri, response.user_code
            )
        }),
        device_code: response.device_code,
        user_code: response.user_code,
        verification_uri: response.verification_uri,
        verification_uri_complete: response.verification_uri_complete,
        expires_in: response.expires_in,
        interval: response.interval.unwrap_or(5),
    })
}

pub async fn poll_login(
    paths: &LauncherPaths,
    device_code: &str,
) -> Result<LoginPollStatus, AuthError> {
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
