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

    pub fn normalize_tool_arguments(arguments: &str) -> Result<String, AppError> {
        if serde_json::from_str::<Value>(arguments).is_ok() {
            return Ok(arguments.to_owned());
        }
        let start = arguments.find('{');
        let end = arguments.rfind('}');
        if let (Some(start), Some(end)) = (start, end)
            && start < end
        {
            let candidate = &arguments[start..=end];
            if serde_json::from_str::<Value>(candidate).is_ok() {
                return Ok(candidate.to_owned());
            }
        }
        Err(AppError::LlmResponse(format!(
            "tool arguments не являются JSON: {}",
            arguments.chars().take(160).collect::<String>()
        )))
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
    /// Disables provider-side reasoning mode for tool calls. Some LiteLLM
    /// model groups reject reasoning_effort together with Chat Completions
    /// function tools unless it is explicitly set to `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl CompletionRequest {
    /// LiteLLM/Bedrock reasoning deployments reject output limits below 16, but
    /// a tool-enabled turn also needs enough room for a complete function call
    /// and its JSON arguments. A 256-token floor prevents the model from
    /// returning an empty assistant message after spending the whole budget on
    /// tool-call reasoning.
    pub const MIN_PROVIDER_MAX_TOKENS: u32 = 256;

    /// Создаёт запрос из доменной истории сообщений.
    pub fn from_messages(model: impl Into<String>, messages: &[Message]) -> Self {
        Self {
            model: model.into(),
            messages: messages.iter().map(LlmMessage::from_message).collect(),
            tools: None,
            temperature: None,
            max_tokens: Some(Self::MIN_PROVIDER_MAX_TOKENS),
            reasoning_effort: None,
        }
    }

    pub fn from_llm_messages(
        model: impl Into<String>,
        messages: Vec<LlmMessage>,
        tools: Vec<ToolDefinition>,
        reasoning_effort: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: (!tools.is_empty()).then_some(tools),
            temperature: None,
            max_tokens: Some(Self::MIN_PROVIDER_MAX_TOKENS),
            // Provider adapters remove this field when the endpoint does not
            // support reasoning together with function tools.
            reasoning_effort: Some(reasoning_effort.into()),
        }
    }

    /// Applies provider capabilities before serializing an OpenAI-compatible
    /// request. Chat Completions routes that cannot combine reasoning and
    /// function tools need an explicit `none`: omitting the field can make a
    /// gateway reapply its model-level default effort.
    pub fn apply_reasoning_capabilities(
        &mut self,
        supports_reasoning_effort: bool,
        supports_reasoning_with_tools: bool,
        reasoning_effort_models: &[String],
        reasoning_with_tools_models: &[String],
    ) {
        let model_supports_effort = supports_reasoning_effort
            || reasoning_effort_models
                .iter()
                .any(|model| model == &self.model);
        if !model_supports_effort {
            self.reasoning_effort = None;
            return;
        }

        let model_supports_tools = supports_reasoning_with_tools
            || reasoning_with_tools_models
                .iter()
                .any(|model| model == &self.model);
        if self.tools.is_some() && !model_supports_tools && self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("none".to_owned());
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

impl Usage {
    pub fn add_assign(&mut self, other: &Usage) {
        self.prompt_tokens = add_optional(self.prompt_tokens, other.prompt_tokens);
        self.completion_tokens = add_optional(self.completion_tokens, other.completion_tokens);
        self.total_tokens = add_optional(self.total_tokens, other.total_tokens);
    }
}

fn add_optional(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
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
    forward_reasoning_allowlist: bool,
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
            forward_reasoning_allowlist: true,
        })
    }

    #[cfg(test)]
    fn with_endpoint(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            models_endpoint: String::new(),
            api_key: None,
            forward_reasoning_allowlist: true,
        }
    }

    #[cfg(test)]
    fn with_models_endpoint(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: String::new(),
            models_endpoint: endpoint,
            api_key: None,
            forward_reasoning_allowlist: true,
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
        let mut payload =
            serde_json::to_value(&request).map_err(|error| AppError::LlmJson(error.to_string()))?;
        if self.forward_reasoning_allowlist && request.reasoning_effort.is_some() {
            payload
                .as_object_mut()
                .expect("CompletionRequest serializes to an object")
                .insert(
                    "allowed_openai_params".to_owned(),
                    serde_json::json!(["reasoning_effort"]),
                );
        }
        let mut last_response = None;

        // OpenAI-compatible gateways differ in both token-limit spelling and
        // reasoning/tool support. Retry only explicit capability errors, so a
        // real provider failure is returned immediately and never duplicated.
        for _ in 0..3 {
            let response = self.send_completion_payload(payload.clone()).await?;
            if (200..300).contains(&response.status) {
                let api_response: ApiResponse = serde_json::from_str(&response.body)
                    .map_err(|error| AppError::LlmJson(error.to_string()))?;
                return api_response.try_into();
            }

            let Some(object) = payload.as_object_mut() else {
                break;
            };
            if supports_reasoning_tool_alias(&response.body) {
                let needs_retry = object
                    .get("reasoning_effort")
                    .and_then(Value::as_str)
                    .is_some_and(|effort| effort != "none");
                if needs_retry {
                    object.insert(
                        "reasoning_effort".to_owned(),
                        Value::String("none".to_owned()),
                    );
                    last_response = Some(response);
                    continue;
                }
            }
            if rejects_reasoning_effort(&response.body)
                && object.remove("reasoning_effort").is_some()
            {
                object.remove("allowed_openai_params");
                last_response = Some(response);
                continue;
            }
            if supports_completion_token_alias(&response.body)
                && let Some(max_tokens) = object.remove("max_tokens")
            {
                object.insert("max_completion_tokens".to_owned(), max_tokens);
                last_response = Some(response);
                continue;
            }
            last_response = Some(response);
            break;
        }

        let response = last_response.expect("completion request always has a response");
        Err(AppError::LlmHttp {
            status: response.status,
            message: truncate_for_error(&response.body),
        })
    }
}

struct CompletionHttpResponse {
    status: u16,
    body: String,
}

impl LiteLlmProvider {
    async fn send_completion_payload(
        &self,
        payload: Value,
    ) -> Result<CompletionHttpResponse, AppError> {
        let mut builder = self.client.post(&self.endpoint).json(&payload);
        if let Some(api_key) = &self.api_key {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {api_key}"));
        }
        let response = builder
            .send()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| AppError::LlmNetwork(error.to_string()))?;
        Ok(CompletionHttpResponse { status, body })
    }
}

fn supports_completion_token_alias(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("max_tokens") && lower.contains("max_completion_tokens")
}

fn supports_reasoning_tool_alias(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("function tools")
        && lower.contains("reasoning_effort")
        && (lower.contains("not supported") || lower.contains("set reasoning_effort"))
}

fn rejects_reasoning_effort(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("reasoning_effort")
        && (lower.contains("unsupportedparamserror")
            || lower.contains("unsupported parameter")
            || lower.contains("does not support parameters"))
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
        let mut inner = LiteLlmProvider::new(config)?;
        inner.forward_reasoning_allowlist = false;
        Ok(Self { inner })
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

    #[test]
    fn normalizes_reasoning_prefix_before_tool_json() {
        assert_eq!(
            super::LlmMessage::normalize_tool_arguments("Wait, check tests. {\"limit\":20}")
                .unwrap(),
            "{\"limit\":20}"
        );
    }
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
    fn requests_use_provider_safe_minimum_output_tokens() {
        let request = CompletionRequest::from_messages("model", &[]);
        assert_eq!(
            request.max_tokens,
            Some(CompletionRequest::MIN_PROVIDER_MAX_TOKENS)
        );
    }

    #[test]
    fn detects_explicit_provider_token_parameter_error() {
        assert!(super::supports_completion_token_alias(
            "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead"
        ));
        assert!(!super::supports_completion_token_alias(
            "Unsupported parameter: 'temperature'"
        ));
    }

    #[test]
    fn detects_reasoning_tools_capability_error() {
        assert!(super::supports_reasoning_tool_alias(
            "Function tools with reasoning_effort are not supported for this model"
        ));
        assert!(!super::supports_reasoning_tool_alias(
            "Unsupported parameter: 'reasoning_effort'"
        ));
    }

    #[test]
    fn detects_provider_rejection_of_reasoning_effort() {
        assert!(super::rejects_reasoning_effort(
            "litellm.UnsupportedParamsError: openai does not support parameters: ['reasoning_effort']"
        ));
        assert!(super::rejects_reasoning_effort(
            "Unsupported parameter: 'reasoning_effort'"
        ));
        assert!(!super::rejects_reasoning_effort(
            "Function tools with reasoning_effort are not supported"
        ));
    }

    #[test]
    fn incompatible_tool_requests_explicitly_disable_reasoning() {
        let mut request = CompletionRequest::from_llm_messages(
            "gpt-6-luna",
            Vec::new(),
            vec![super::ToolDefinition::function(
                "test_tool",
                "test",
                json!({"type": "object"}),
            )],
            "high",
        );

        request.apply_reasoning_capabilities(true, false, &[], &[]);

        assert_eq!(request.reasoning_effort.as_deref(), Some("none"));
    }

    #[test]
    fn compatible_tool_requests_preserve_configured_reasoning_effort() {
        let mut request = CompletionRequest::from_llm_messages(
            "reasoning-model",
            Vec::new(),
            vec![super::ToolDefinition::function(
                "test_tool",
                "test",
                json!({"type": "object"}),
            )],
            "high",
        );
        request.apply_reasoning_capabilities(true, true, &[], &[]);

        assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn unsupported_reasoning_is_omitted_entirely() {
        let mut request =
            CompletionRequest::from_llm_messages("plain-model", Vec::new(), Vec::new(), "medium");

        request.apply_reasoning_capabilities(false, false, &[], &[]);

        assert!(request.reasoning_effort.is_none());
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
    async fn client_sends_explicit_none_with_function_tools() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw_request = vec![0; 8192];
            let bytes = stream.read(&mut raw_request).await.unwrap();
            let raw_request = String::from_utf8_lossy(&raw_request[..bytes]);
            let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
            let payload: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(payload["reasoning_effort"], "none");
            assert_eq!(payload["allowed_openai_params"][0], "reasoning_effort");
            assert!(payload["tools"].is_array());

            let body = r#"{"choices":[{"message":{"role":"assistant","content":"pong"},"finish_reason":"stop"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let provider = LiteLlmProvider::with_endpoint(format!("http://{address}/chat/completions"));
        let mut request = CompletionRequest::from_llm_messages(
            "gpt-6-luna",
            Vec::new(),
            vec![super::ToolDefinition::function(
                "test_tool",
                "test",
                json!({"type": "object"}),
            )],
            "high",
        );
        request.apply_reasoning_capabilities(true, false, &[], &[]);
        provider.complete(request).await.unwrap();

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
