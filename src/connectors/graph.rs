use super::auth::GraphAuth;
use crate::{
    AppError,
    connectors::{MessageSource, models::*},
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct GraphMailClient {
    client: Client,
    base_url: String,
    access_token: String,
    auth: Option<GraphAuth>,
}

impl GraphMailClient {
    pub fn new(
        base_url: impl Into<String>,
        access_token: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, AppError> {
        let access_token = access_token.into();
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| AppError::Tool(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            access_token,
            auth: None,
        })
    }

    pub fn with_auth(mut self, auth: GraphAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, AppError> {
        let token = match &self.auth {
            Some(auth) => auth.access_token().await?,
            None if !self.access_token.is_empty() => self.access_token.clone(),
            None => return Err(AppError::Tool("Graph token не задан".into())),
        };
        let mut request = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .header("Accept", "application/json");
        if path.contains("$search=") {
            request = request.header("ConsistencyLevel", "eventual");
        }
        let response = request
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Graph network error: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "неизвестная ошибка".into());
            return Err(AppError::Tool(format!(
                "Microsoft Graph HTTP {}: {}",
                status.as_u16(),
                message
            )));
        }
        response
            .json()
            .await
            .map_err(|e| AppError::Tool(format!("Graph JSON error: {e}")))
    }
}

#[derive(Debug, Deserialize)]
struct GraphPage {
    value: Vec<GraphMessage>,
    #[serde(rename = "@odata.nextLink")]
    _next: Option<String>,
}
#[derive(Debug, Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
    subject: Option<String>,
    body: Option<GraphBody>,
    #[serde(rename = "bodyPreview")]
    body_preview: Option<String>,
    sender: Option<GraphRecipient>,
    #[serde(rename = "toRecipients", default)]
    to_recipients: Vec<GraphRecipient>,
    #[serde(rename = "receivedDateTime")]
    received: Option<String>,
    #[serde(rename = "hasAttachments", default)]
    has_attachments: bool,
}
#[derive(Debug, Deserialize)]
struct GraphBody {
    content: String,
}
#[derive(Debug, Deserialize)]
struct GraphRecipient {
    #[serde(rename = "emailAddress")]
    email_address: Option<GraphAddress>,
    name: Option<String>,
}
#[derive(Debug, Deserialize)]
struct GraphAddress {
    address: Option<String>,
}

fn recipient(value: Option<&GraphRecipient>) -> String {
    value
        .and_then(|r| {
            r.email_address
                .as_ref()
                .and_then(|a| a.address.clone())
                .or_else(|| r.name.clone())
        })
        .unwrap_or_else(|| "unknown".into())
}
fn normalize(m: GraphMessage, include_body: bool) -> NormalizedMessage {
    NormalizedMessage {
        id: m.id,
        thread_id: m.conversation_id,
        source: SourceKind::Outlook,
        sender: recipient(m.sender.as_ref()),
        recipients: m.to_recipients.iter().map(|r| recipient(Some(r))).collect(),
        subject: m.subject,
        preview: m.body_preview,
        body: include_body.then(|| m.body.map(|b| b.content)).flatten(),
        sent_at: m.received,
        has_attachments: m.has_attachments,
    }
}

#[async_trait]
impl MessageSource for GraphMailClient {
    fn source(&self) -> SourceKind {
        SourceKind::Outlook
    }
    async fn list_messages(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<NormalizedMessage>, AppError> {
        let limit = query.max_messages.clamp(1, 100);
        let mut path = format!(
            "/me/mailFolders/inbox/messages?$top={limit}&$orderby=receivedDateTime%20desc&$select=id,conversationId,subject,bodyPreview,sender,toRecipients,receivedDateTime,hasAttachments,isRead"
        );
        if query.lookback_hours > 0 {
            path.push_str(&format!(
                "&$filter=receivedDateTime%20ge%20{}",
                chrono::Utc::now()
                    .checked_sub_signed(chrono::Duration::hours(query.lookback_hours as i64))
                    .unwrap_or_else(chrono::Utc::now)
                    .to_rfc3339()
            ));
        }
        if query.unread_only {
            path.push_str("&$filter=isRead%20eq%20false");
        }
        if query.include_body {
            path.push_str(",body");
        }
        if let Some(search) = &query.search {
            path.push_str(&format!(
                "&$search={}",
                urlencoding::encode(&format!("\"{}\"", search))
            ));
        }
        let page: GraphPage = self.get(&path).await?;
        Ok(page
            .value
            .into_iter()
            .map(|m| normalize(m, query.include_body))
            .collect())
    }
    async fn get_message(&self, id: &str) -> Result<NormalizedMessage, AppError> {
        let encoded = urlencoding::encode(id);
        let m: GraphMessage = self.get(&format!("/me/messages/{encoded}?$select=id,conversationId,subject,body,bodyPreview,sender,toRecipients,receivedDateTime,hasAttachments")).await?;
        Ok(normalize(m, true))
    }
    async fn get_thread(&self, id: &str) -> Result<NormalizedThread, AppError> {
        let encoded = urlencoding::encode(id);
        let page: GraphPage = self.get(&format!("/me/mailFolders/inbox/messages?$top=100&$filter=conversationId%20eq%20'{}'&$orderby=receivedDateTime%20asc&$select=id,conversationId,subject,body,bodyPreview,sender,toRecipients,receivedDateTime,hasAttachments", encoded)).await?;
        let messages = page.value.into_iter().map(|m| normalize(m, true)).collect();
        Ok(NormalizedThread {
            id: id.into(),
            source: SourceKind::Outlook,
            messages,
        })
    }
}
