//! Microsoft identity platform device-code authentication.
//!
//! Access tokens live only in memory. Refresh tokens are kept in the native
//! OS credential store and are never part of application/session config.

use crate::AppError;
use keyring::Entry;
use reqwest::Client;
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const SERVICE: &str = "ai-agent.microsoft-graph";
const DEFAULT_SCOPE: &str = "offline_access Mail.Read User.Read";

#[derive(Clone)]
pub struct GraphAuth {
    client: Client,
    tenant: String,
    client_id: String,
    scope: String,
    state: Arc<Mutex<TokenState>>,
}

#[derive(Default)]
struct TokenState {
    access_token: Option<String>,
    expires_at: Option<Instant>,
}

#[derive(Debug, Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
    message: Option<String>,
}
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

impl GraphAuth {
    pub fn from_env(timeout: Duration) -> Result<Self, AppError> {
        let client_id = std::env::var("MICROSOFT_GRAPH_CLIENT_ID")
            .map_err(|_| AppError::InvalidConfig("MICROSOFT_GRAPH_CLIENT_ID не задан".into()))?;
        let tenant = std::env::var("MICROSOFT_GRAPH_TENANT").unwrap_or_else(|_| "common".into());
        let scope = std::env::var("MICROSOFT_GRAPH_SCOPE").unwrap_or_else(|_| DEFAULT_SCOPE.into());
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| AppError::Tool(e.to_string()))?;
        Ok(Self {
            client,
            tenant,
            client_id,
            scope,
            state: Arc::new(Mutex::new(TokenState::default())),
        })
    }

    fn entry(&self) -> Result<Entry, AppError> {
        Entry::new(SERVICE, &format!("{}:{}", self.tenant, self.client_id))
            .map_err(|e| AppError::Tool(format!("OS credential store: {e}")))
    }

    pub async fn login(&self) -> Result<(), AppError> {
        let endpoint = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0",
            self.tenant
        );
        let response = self
            .client
            .post(format!("{endpoint}/devicecode"))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scope", self.scope.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Microsoft login network error: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::Tool(format!("Microsoft login response read error: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Tool(format_oauth_error(
                "Microsoft device-code",
                status.as_u16(),
                &body,
            )));
        }
        let code: DeviceCode = serde_json::from_str(&body).map_err(|e| {
            AppError::Tool(format!(
                "Microsoft device-code response (HTTP {}): {e}; body: {}",
                status.as_u16(),
                truncate_response(&body)
            ))
        })?;
        println!(
            "{}",
            code.message.unwrap_or_else(|| format!(
                "Откройте {} и введите {}",
                code.verification_uri, code.user_code
            ))
        );
        let interval = Duration::from_secs(code.interval.unwrap_or(5));
        let deadline = Instant::now() + Duration::from_secs(code.expires_in);
        loop {
            if Instant::now() >= deadline {
                return Err(AppError::Tool("device code истёк".into()));
            }
            let response = self
                .client
                .post(format!("{endpoint}/token"))
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("client_id", self.client_id.as_str()),
                    ("device_code", code.device_code.as_str()),
                ])
                .send()
                .await
                .map_err(|e| AppError::Tool(format!("Microsoft token network error: {e}")))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| AppError::Tool(format!("Microsoft token response read error: {e}")))?;
            let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
                AppError::Tool(format!(
                    "Microsoft token response (HTTP {}): {e}; body: {}",
                    status.as_u16(),
                    truncate_response(&body)
                ))
            })?;
            if let Some(access) = token.access_token {
                if let Some(refresh) = token.refresh_token {
                    self.entry()?.set_password(&refresh).map_err(|e| {
                        AppError::Tool(format!("не удалось сохранить refresh token: {e}"))
                    })?;
                }
                self.set_access(access, token.expires_in.unwrap_or(3600))
                    .await;
                return Ok(());
            }
            if token.error.as_deref() != Some("authorization_pending")
                && token.error.as_deref() != Some("slow_down")
            {
                return Err(AppError::Tool(
                    token
                        .error_description
                        .unwrap_or_else(|| "Microsoft authentication failed".into()),
                ));
            }
            tokio::time::sleep(interval).await;
        }
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
                "нет Graph refresh token; запустите --graph-login: {e}"
            ))
        })?;
        let endpoint = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant
        );
        let response = self
            .client
            .post(endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", self.client_id.as_str()),
                ("refresh_token", refresh.as_str()),
                ("scope", self.scope.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Microsoft refresh network error: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::Tool(format!("Microsoft refresh response read error: {e}")))?;
        let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::Tool(format!(
                "Microsoft refresh response (HTTP {}): {e}; body: {}",
                status.as_u16(),
                truncate_response(&body)
            ))
        })?;
        if !status.is_success() && token.access_token.is_none() {
            return Err(AppError::Tool(format!(
                "Microsoft refresh HTTP {}: {}",
                status.as_u16(),
                token
                    .error_description
                    .as_deref()
                    .unwrap_or("unknown error")
            )));
        }
        let access = token.access_token.ok_or_else(|| {
            AppError::Tool(
                token
                    .error_description
                    .unwrap_or_else(|| "Microsoft refresh failed".into()),
            )
        })?;
        if let Some(refresh) = token.refresh_token {
            self.entry()?
                .set_password(&refresh)
                .map_err(|e| AppError::Tool(e.to_string()))?;
        }
        self.set_access(access.clone(), token.expires_in.unwrap_or(3600))
            .await;
        Ok(access)
    }
}

fn truncate_response(body: &str) -> String {
    const MAX_LEN: usize = 1000;
    let compact = body.replace(['\n', '\r'], " ");
    if compact.len() <= MAX_LEN {
        compact
    } else {
        let prefix: String = compact.chars().take(MAX_LEN).collect();
        format!("{prefix}…")
    }
}

fn format_oauth_error(operation: &str, status: u16, body: &str) -> String {
    match serde_json::from_str::<OAuthErrorResponse>(body) {
        Ok(error) => format!(
            "{operation} HTTP {status}: {}{}",
            error.error.as_deref().unwrap_or("unknown error"),
            error
                .error_description
                .as_deref()
                .map(|description| format!(" — {description}"))
                .unwrap_or_default()
        ),
        Err(_) => format!("{operation} HTTP {status}: {}", truncate_response(body)),
    }
}
