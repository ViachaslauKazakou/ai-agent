//! Google OAuth 2.0 Authorization Code + PKCE for installed applications.

use crate::AppError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::Entry;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

const SERVICE: &str = "ai-agent.gmail";
const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const REDIRECT_URI: &str = "http://127.0.0.1:8765/oauth2callback";
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

#[derive(Clone)]
pub struct GmailAuth {
    client: Client,
    client_id: String,
    state: Arc<Mutex<TokenState>>,
}

#[derive(Default)]
struct TokenState {
    access_token: Option<String>,
    expires_at: Option<Instant>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

impl GmailAuth {
    pub fn from_env(timeout: Duration) -> Result<Self, AppError> {
        let client_id = std::env::var("GOOGLE_GMAIL_CLIENT_ID")
            .map_err(|_| AppError::InvalidConfig("GOOGLE_GMAIL_CLIENT_ID не задан".into()))?;
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| AppError::Tool(e.to_string()))?;
        Ok(Self {
            client,
            client_id,
            state: Arc::new(Mutex::new(TokenState::default())),
        })
    }

    fn entry(&self) -> Result<Entry, AppError> {
        Entry::new(SERVICE, &self.client_id)
            .map_err(|e| AppError::Tool(format!("OS credential store: {e}")))
    }

    pub async fn login(&self) -> Result<(), AppError> {
        let listener = TcpListener::bind("127.0.0.1:8765")
            .await
            .map_err(|e| AppError::Tool(format!("Gmail callback port 8765: {e}")))?;
        let verifier = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
            urlencoding::encode(&self.client_id),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPE),
            challenge,
            urlencoding::encode(&state)
        );
        println!("Откройте в браузере для Gmail авторизации:\n{url}");
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| AppError::Tool(format!("Gmail callback: {e}")))?;
        let mut buffer = [0_u8; 8192];
        let length = socket
            .read(&mut buffer)
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        let request = String::from_utf8_lossy(&buffer[..length]);
        let target = request
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| AppError::Tool("Gmail callback: некорректный HTTP request".into()))?;
        let params = parse_query(target.split('?').nth(1).unwrap_or_default());
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nGmail authorization completed. You can close this tab.";
        let _ = socket.write_all(response).await;
        if params.get("state") != Some(&state) {
            return Err(AppError::Tool("Gmail OAuth state mismatch".into()));
        }
        let code = params.get("code").cloned().ok_or_else(|| {
            AppError::Tool(format!(
                "Gmail OAuth: {}",
                params
                    .get("error_description")
                    .or_else(|| params.get("error"))
                    .cloned()
                    .unwrap_or_else(|| "authorization code отсутствует".into())
            ))
        })?;
        let token = self
            .client
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("code", code.as_str()),
                ("client_id", self.client_id.as_str()),
                ("redirect_uri", REDIRECT_URI),
                ("grant_type", "authorization_code"),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Gmail token network error: {e}")))?;
        let status = token.status();
        let body = token
            .text()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::Tool(format!(
                "Gmail token HTTP {}: {e}; body: {}",
                status.as_u16(),
                truncate(&body)
            ))
        })?;
        let access = token.access_token.ok_or_else(|| {
            AppError::Tool(format!(
                "Gmail token HTTP {}: {}",
                status.as_u16(),
                token
                    .error_description
                    .or(token.error)
                    .unwrap_or_else(|| "unknown error".to_owned())
            ))
        })?;
        if let Some(refresh) = token.refresh_token {
            self.entry()?.set_password(&refresh).map_err(|e| {
                AppError::Tool(format!("не удалось сохранить Gmail refresh token: {e}"))
            })?;
        }
        self.set_access(access, token.expires_in.unwrap_or(3600))
            .await;
        Ok(())
    }

    async fn set_access(&self, token: String, expires_in: u64) {
        let mut state = self.state.lock().await;
        state.access_token = Some(token);
        state.expires_at =
            Some(Instant::now() + Duration::from_secs(expires_in.saturating_sub(60)));
    }

    pub async fn access_token(&self) -> Result<String, AppError> {
        {
            let state = self.state.lock().await;
            if state.expires_at.is_some_and(|until| until > Instant::now()) {
                return Ok(state.access_token.clone().unwrap_or_default());
            }
        }
        let refresh = self.entry()?.get_password().map_err(|e| {
            AppError::Tool(format!(
                "нет Gmail refresh token; запустите --gmail-login: {e}"
            ))
        })?;
        let response = self
            .client
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("refresh_token", refresh.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Gmail refresh network error: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::Tool(format!(
                "Gmail refresh HTTP {}: {e}; body: {}",
                status.as_u16(),
                truncate(&body)
            ))
        })?;
        let access = token.access_token.ok_or_else(|| {
            AppError::Tool(format!(
                "Gmail refresh HTTP {}: {}",
                status.as_u16(),
                token
                    .error_description
                    .or(token.error)
                    .unwrap_or_else(|| "unknown error".to_owned())
            ))
        })?;
        self.set_access(access.clone(), token.expires_in.unwrap_or(3600))
            .await;
        Ok(access)
    }
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                key.to_owned(),
                urlencoding::decode(value).ok()?.into_owned(),
            ))
        })
        .collect()
}
fn truncate(body: &str) -> String {
    body.chars().take(1000).collect()
}
