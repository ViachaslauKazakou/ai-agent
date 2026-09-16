//! Типы Chat Completions и OpenAI-compatible клиенты LiteLLM и Ollama.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppError, Config, Message, Role, ToolCallMessage};

/// Сообщение в формате OpenAI-compatible API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl LlmMessage {
    pub(crate) fn from_message(message: &Message) -> Self {
        Self {
            role: role_name(message.role()).to_owned(),
            content: Some(message.content().to_owned()),
            tool_calls: message.tool_calls().map(|calls| {
                calls
                    .iter()
                    .map(|call| ToolCall {
                        id: call.id.clone(),
                        kind: "function".to_owned(),
                        function: FunctionCall {
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        },
                    })
                    .collect()
            }),
            tool_call_id: message.tool_call_id().map(str::to_owned),
        }
    }

    pub(crate) fn from_tool_response(message: &LlmMessage) -> Result<Message, AppError> {
        if let Some(calls) = &message.tool_calls {
            return Message::assistant_tool_calls(
                message.content.clone(),
                calls.iter().map(ToolCallMessage::from).collect(),
            );
        }
        if message.role == "tool" {
            return Message::tool_result(
                message.tool_call_id.clone().ok_or_else(|| {
                    AppError::LlmResponse("tool message не содержит tool_call_id".to_owned())
                })?,
                message.content.clone().unwrap_or_default(),
            );
        }
        Message::new(Role::Assistant, message.content.clone().unwrap_or_default())
    }

    pub(crate) fn tool_result(tool_call_id: &str, content: String) -> Self {
        Self {
            role: "tool".to_owned(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_owned()),
        }
    }
}

impl From<&ToolCall> for ToolCallMessage {
    fn from(call: &ToolCall) -> Self {
        Self {
            id: call.id.clone(),
            name: call.function.name.clone(),
            arguments: call.function.arguments.clone(),
        }
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// Описание инструмента в Chat Completions request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn function(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            kind: "function".to_owned(),
            function: FunctionDefinition {
                name: name.to_owned(),
                description: description.to_owned(),
                parameters,
            },
        }
    }
}

/// Вызов инструмента, возвращаемый моделью.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

/// Имя функции и JSON-аргументы вызова.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Запрос генерации ответа.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl CompletionRequest {
    /// Создаёт запрос из доменной истории сообщений.
    pub fn from_messages(model: impl Into<String>, messages: &[Message]) -> Self {
        Self {
            model: model.into(),
            messages: messages.iter().map(LlmMessage::from_message).collect(),
            tools: None,
            temperature: None,
            max_tokens: None,
        }
    }

    pub fn from_llm_messages(
        model: impl Into<String>,
        messages: Vec<LlmMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: (!tools.is_empty()).then_some(tools),
            temperature: None,
            max_tokens: None,
        }
    }
}

/// Использование токенов, если оно возвращено провайдером.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

/// Внутренний ответ одного completion choice.
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ApiChoice {
    message: LlmMessage,
    finish_reason: Option<String>,
}

/// Ответ Chat Completions, нормализованный из API-обёртки `choices`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionResponse {
    pub message: LlmMessage,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// Модель, возвращённая OpenAI-compatible endpoint `/models`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
    usage: Option<Usage>,
}

impl TryFrom<ApiResponse> for CompletionResponse {
    type Error = AppError;

    fn try_from(response: ApiResponse) -> Result<Self, Self::Error> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AppError::LlmResponse("поле choices пустое".to_owned()))?;
        Ok(Self {
            message: choice.message,
            finish_reason: choice.finish_reason,
            usage: response.usage,
        })
    }
}

impl CompletionResponse {
    /// Возвращает текст ответа assistant, если модель его предоставила.
    pub fn content(&self) -> Option<&str> {
        self.message.content.as_deref()
    }
}

/// Общий контракт LLM-провайдера.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError>;
}

/// Клиент LiteLLM/OpenAI-compatible Chat Completions.
#[derive(Clone)]
pub struct LiteLlmProvider {
    client: reqwest::Client,
    endpoint: String,
    models_endpoint: String,
    api_key: Option<String>,
}

impl LiteLlmProvider {
    /// Создаёт HTTP-клиент с timeout из typed Config.
    pub fn new(config: &Config) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| AppError::LlmProvider(error.to_string()))?;
        let endpoint = format!(
            "{}/chat/completions",
            config.api_base_url.trim_end_matches('/')
        );
        let models_endpoint = format!("{}/models", config.api_base_url.trim_end_matches('/'));

        Ok(Self {
            client,
            endpoint,
            models_endpoint,
            api_key: config.api_key.clone(),
        })
    }

    #[cfg(test)]
    fn with_endpoint(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            models_endpoint: String::new(),
            api_key: None,
        }
    }

    #[cfg(test)]
    fn with_models_endpoint(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: String::new(),
            models_endpoint: endpoint,
            api_key: None,
        }
    }

    /// Возвращает модели, опубликованные LiteLLM/OpenAI-compatible API.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError> {
        let mut builder = self.client.get(&self.models_endpoint);
        if let Some(api_key) = &self.api_key {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {api_key}"));
        }

        let response = builder
            .send()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;

        if !status.is_success() {
            return Err(AppError::LlmHttp {
                status: status.as_u16(),
                message: truncate_for_error(&body),
            });
        }

        let api_response: ModelsResponse =
            serde_json::from_str(&body).map_err(|error| AppError::LlmJson(error.to_string()))?;
        Ok(api_response.data)
    }
}

#[async_trait]
impl LlmProvider for LiteLlmProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let mut builder = self.client.post(&self.endpoint).json(&request);
        if let Some(api_key) = &self.api_key {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {api_key}"));
        }

        let response = builder
            .send()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;

        if !status.is_success() {
            return Err(AppError::LlmHttp {
                status: status.as_u16(),
                message: truncate_for_error(&body),
            });
        }

        let api_response: ApiResponse =
            serde_json::from_str(&body).map_err(|error| AppError::LlmJson(error.to_string()))?;
        api_response.try_into()
    }
}

/// Клиент локального Ollama через его OpenAI-compatible API.
///
/// Ollama обычно слушает `http://localhost:11434`; endpoint `/v1` используется
/// намеренно, чтобы формат запросов был тем же, что и у LiteLLM.
#[derive(Clone)]
pub struct OllamaProvider {
    inner: LiteLlmProvider,
}

impl OllamaProvider {
    pub fn new(config: &Config) -> Result<Self, AppError> {
        Ok(Self {
            inner: LiteLlmProvider::new(config)?,
        })
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, AppError> {
        self.inner.list_models().await
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.inner.complete(request).await
    }
}

fn truncate_for_error(body: &str) -> String {
    const MAX_ERROR_LENGTH: usize = 500;
    body.chars().take(MAX_ERROR_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::{CompletionRequest, LiteLlmProvider, ModelInfo};
    use crate::{LlmProvider, Message, Role};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn request_uses_openai_role_names() {
        let message = Message::new(Role::User, "Привет").unwrap();
        let request = CompletionRequest::from_messages("local-model", &[message]);

        assert_eq!(
            serde_json::to_value(request).unwrap()["messages"][0]["role"],
            "user"
        );
    }

    #[test]
    fn response_deserializes_text_and_tool_calls() {
        let response: super::ApiResponse = serde_json::from_value(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Готово",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "read_file", "arguments": "{\"path\":\"README.md\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
        }))
        .unwrap();
        let response: super::CompletionResponse = response.try_into().unwrap();

        assert_eq!(response.content(), Some("Готово"));
        assert_eq!(
            response.message.tool_calls.as_ref().unwrap()[0].id,
            "call-1"
        );
    }

    #[tokio::test]
    async fn client_posts_json_and_parses_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let bytes = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
            assert!(request.contains("\"model\":\"local-model\""));

            let body = r#"{"choices":[{"message":{"role":"assistant","content":"pong"},"finish_reason":"stop"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let provider = LiteLlmProvider::with_endpoint(format!("http://{address}/chat/completions"));
        let request = CompletionRequest::from_messages(
            "local-model",
            &[Message::new(Role::User, "ping").unwrap()],
        );
        let response = provider.complete(request).await.unwrap();

        assert_eq!(response.content(), Some("pong"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn client_gets_available_models() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 1024];
            let bytes = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.starts_with("GET /models HTTP/1.1"));

            let body = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model","owned_by":"openai"},{"id":"local-model"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let provider = LiteLlmProvider::with_models_endpoint(format!("http://{address}/models"));
        let models = provider.list_models().await.unwrap();

        assert_eq!(
            models,
            vec![
                ModelInfo {
                    id: "gpt-4o".to_owned(),
                    object: Some("model".to_owned()),
                    owned_by: Some("openai".to_owned()),
                },
                ModelInfo {
                    id: "local-model".to_owned(),
                    object: None,
                    owned_by: None,
                },
            ]
        );
        server.await.unwrap();
    }
}
