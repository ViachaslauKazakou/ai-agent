use super::auth::GraphAuth;
use super::gmail::GmailMailClient;
use super::gmail_auth::GmailAuth;
use super::{GraphMailClient, MessageQuery, MessageSource};
use crate::{
    AppError,
    tools::{Tool, ToolContext, ToolResult},
};
use async_trait::async_trait;
use serde_json::{Value, json};

fn client(context: &ToolContext) -> Result<GraphMailClient, AppError> {
    let client = GraphMailClient::new(
        context
            .graph_base_url
            .clone()
            .unwrap_or_else(|| "https://graph.microsoft.com/v1.0".into()),
        context.graph_access_token.clone().unwrap_or_default(),
        std::time::Duration::from_secs(30),
    )?;
    if context.graph_access_token.is_none() {
        Ok(client.with_auth(GraphAuth::from_env(std::time::Duration::from_secs(30))?))
    } else {
        Ok(client)
    }
}
fn source(context: &ToolContext) -> Result<Box<dyn MessageSource>, AppError> {
    if context
        .gmail_client_id
        .as_deref()
        .is_some_and(|client_id| !client_id.trim().is_empty())
    {
        let auth = GmailAuth::from_config(
            std::time::Duration::from_secs(30),
            context.gmail_client_id.clone().unwrap_or_default(),
            context.gmail_client_secret.clone(),
        )?;
        return Ok(Box::new(GmailMailClient::new(
            auth,
            std::time::Duration::from_secs(30),
        )?));
    }
    Ok(Box::new(client(context)?))
}
fn query(args: &Value, body: bool) -> MessageQuery {
    MessageQuery {
        lookback_hours: args
            .get("lookback_hours")
            .and_then(Value::as_u64)
            .unwrap_or(24)
            .min(24 * 30),
        max_messages: args
            .get("max_messages")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize,
        search: args.get("query").and_then(Value::as_str).map(str::to_owned),
        include_body: body,
    }
}

pub struct ListRecentEmails;
#[async_trait]
impl Tool for ListRecentEmails {
    fn name(&self) -> &'static str {
        "list_recent_emails"
    }
    fn description(&self) -> &'static str {
        "List recent Outlook messages; returns headers and previews only."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"lookback_hours":{"type":"integer","minimum":1,"maximum":720},"max_messages":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let messages = source(context)?.list_messages(&query(&args, false)).await?;
        Ok(ToolResult {
            success: true,
            content: serde_json::to_string_pretty(&messages).unwrap_or_default(),
            structured: Some(json!(messages)),
            truncated: false,
            ephemeral: true,
        })
    }
}

pub struct GetEmail;
#[async_trait]
impl Tool for GetEmail {
    fn name(&self) -> &'static str {
        "get_email"
    }
    fn description(&self) -> &'static str {
        "Read one Outlook email, including its body, on explicit request."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("get_email требует id".into()))?;
        let message = source(context)?.get_message(id).await?;
        Ok(ToolResult {
            success: true,
            content: serde_json::to_string_pretty(&message).unwrap_or_default(),
            structured: Some(json!(message)),
            truncated: false,
            ephemeral: true,
        })
    }
}

pub struct SearchEmails;
#[async_trait]
impl Tool for SearchEmails {
    fn name(&self) -> &'static str {
        "search_emails"
    }
    fn description(&self) -> &'static str {
        "Search Outlook messages by query, without loading bodies."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"},"max_messages":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let messages = source(context)?.list_messages(&query(&args, false)).await?;
        Ok(ToolResult {
            success: true,
            content: serde_json::to_string_pretty(&messages).unwrap_or_default(),
            structured: Some(json!(messages)),
            truncated: false,
            ephemeral: true,
        })
    }
}
