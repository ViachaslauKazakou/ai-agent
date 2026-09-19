use super::gmail_auth::GmailAuth;
use crate::{
    AppError,
    connectors::{MessageSource, models::*},
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct GmailMailClient {
    client: Client,
    auth: GmailAuth,
}

impl GmailMailClient {
    pub fn new(auth: GmailAuth, timeout: Duration) -> Result<Self, AppError> {
        Ok(Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|e| AppError::Tool(e.to_string()))?,
            auth,
        })
    }
    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, AppError> {
        let response = self
            .client
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me{path}"
            ))
            .bearer_auth(self.auth.access_token().await?)
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Gmail network error: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        if !status.is_success() {
            return Err(AppError::Tool(format!(
                "Gmail HTTP {}: {}",
                status.as_u16(),
                body.chars().take(1000).collect::<String>()
            )));
        }
        serde_json::from_str(&body).map_err(|e| AppError::Tool(format!("Gmail JSON error: {e}")))
    }
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    messages: Vec<MessageRef>,
}
#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
}
#[derive(Debug, Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    snippet: Option<String>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
    payload: Option<Payload>,
}
#[derive(Debug, Deserialize)]
struct Payload {
    headers: Option<Vec<Header>>,
    body: Option<BodyPart>,
    parts: Option<Vec<BodyPart>>,
}
#[derive(Debug, Deserialize)]
struct Header {
    name: String,
    value: String,
}
#[derive(Debug, Deserialize)]
struct BodyPart {
    mime_type: Option<String>,
    body: Option<BodyData>,
    parts: Option<Vec<BodyPart>>,
}
#[derive(Debug, Deserialize)]
struct BodyData {
    data: Option<String>,
}

fn header(payload: &Payload, name: &str) -> Option<String> {
    payload
        .headers
        .as_ref()?
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
}
fn decode_body(data: &str) -> Option<String> {
    URL_SAFE_NO_PAD
        .decode(data)
        .or_else(|_| {
            let mut padded = data.to_owned();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            URL_SAFE_NO_PAD.decode(padded.trim_end_matches('='))
        })
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn body_text(part: &BodyPart, html: bool) -> Option<String> {
    if part.mime_type.as_deref() == Some(if html { "text/html" } else { "text/plain" })
        && let Some(data) = part.body.as_ref()?.data.as_ref()
    {
        let text = decode_body(data)?;
        return Some(if html { html_to_text(&text) } else { text });
    }
    part.parts
        .as_ref()?
        .iter()
        .find_map(|child| body_text(child, html))
}

fn html_to_text(html: &str) -> String {
    let mut text = html.to_owned();
    for tag in ["script", "style"] {
        while let (Some(start), Some(end)) = (
            text.to_ascii_lowercase().find(&format!("<{tag}")),
            text.to_ascii_lowercase().find(&format!("</{tag}>")),
        ) {
            if end > start {
                text.replace_range(start..end + tag.len() + 3, " ");
            } else {
                break;
            }
        }
    }
    text = text
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("</p>", "\n");
    let mut output = String::new();
    let mut inside = false;
    for ch in text.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => output.push(ch),
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::html_to_text;

    #[test]
    fn converts_html_body_to_readable_text() {
        assert_eq!(
            html_to_text("<p>Hello <b>world</b></p><style>x</style>"),
            "Hello world"
        );
    }
}
fn normalize(message: GmailMessage, include_body: bool) -> NormalizedMessage {
    let payload = message.payload.as_ref();
    NormalizedMessage {
        id: message.id,
        thread_id: message.thread_id,
        source: SourceKind::Gmail,
        sender: payload
            .and_then(|p| header(p, "From"))
            .unwrap_or_else(|| "unknown".into()),
        recipients: payload.and_then(|p| header(p, "To")).into_iter().collect(),
        subject: payload.and_then(|p| header(p, "Subject")),
        preview: message.snippet,
        body: include_body
            .then(|| {
                payload.and_then(|p| {
                    p.body
                        .as_ref()
                        .and_then(|part| body_text(part, false))
                        .or_else(|| {
                            p.parts
                                .as_ref()?
                                .iter()
                                .find_map(|part| body_text(part, false))
                        })
                        .or_else(|| p.body.as_ref().and_then(|part| body_text(part, true)))
                        .or_else(|| {
                            p.parts
                                .as_ref()?
                                .iter()
                                .find_map(|part| body_text(part, true))
                        })
                })
            })
            .flatten(),
        sent_at: payload
            .and_then(|p| header(p, "Date"))
            .or(message.internal_date),
        has_attachments: false,
    }
}

#[async_trait]
impl MessageSource for GmailMailClient {
    fn source(&self) -> SourceKind {
        SourceKind::Gmail
    }
    async fn list_messages(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<NormalizedMessage>, AppError> {
        let mut q = query.search.clone().unwrap_or_default();
        if query.unread_only {
            if !q.is_empty() {
                q.push(' ');
            }
            q.push_str("is:unread");
        }
        if query.lookback_hours > 0 {
            if !q.is_empty() {
                q.push(' ');
            }
            q.push_str(&format!("newer_than:{}h", query.lookback_hours));
        }
        let mut path = format!("/messages?maxResults={}", query.max_messages.clamp(1, 100));
        if !q.is_empty() {
            path.push_str("&q=");
            path.push_str(&urlencoding::encode(&q));
        }
        let page: ListResponse = self.get(&path).await?;
        let mut messages = Vec::with_capacity(page.messages.len());
        for reference in page.messages {
            let message: GmailMessage = self
                .get(&format!(
                    "/messages/{}?format=full",
                    urlencoding::encode(&reference.id)
                ))
                .await?;
            messages.push(normalize(message, query.include_body));
        }
        Ok(messages)
    }
    async fn get_message(&self, id: &str) -> Result<NormalizedMessage, AppError> {
        let message = normalize(
            self.get(&format!(
                "/messages/{}?format=full",
                urlencoding::encode(id)
            ))
            .await?,
            true,
        );
        if message.body.is_none() {
            return Err(AppError::Tool(format!(
                "Gmail message {id} has no readable inline text body; it may be HTML-only or use an unsupported attachment"
            )));
        }
        Ok(message)
    }
    async fn get_thread(&self, id: &str) -> Result<NormalizedThread, AppError> {
        let thread: ThreadResponse = self
            .get(&format!("/threads/{}?format=full", urlencoding::encode(id)))
            .await?;
        Ok(NormalizedThread {
            id: id.into(),
            source: SourceKind::Gmail,
            messages: thread
                .messages
                .into_iter()
                .map(|m| normalize(m, true))
                .collect(),
        })
    }
}
#[derive(Debug, Deserialize)]
struct ThreadResponse {
    #[serde(default)]
    messages: Vec<GmailMessage>,
}
