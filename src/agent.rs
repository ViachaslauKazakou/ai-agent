//! Agent loop: модель → tools → результаты → модель.

use serde_json::Value;

use crate::tools::{ToolContext, ToolRegistry};
use crate::{AppError, CompletionRequest, LlmMessage, LlmProvider, Message, Role, Session, Usage};

/// Результат обработки пользовательского prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResponse {
    pub content: String,
    pub tool_rounds: usize,
    pub usage: Option<Usage>,
}

/// Координатор LLM и зарегистрированных инструментов.
pub struct Agent<P> {
    provider: P,
    registry: ToolRegistry,
    context: ToolContext,
    max_tool_rounds: usize,
    messages: Vec<LlmMessage>,
    system_prompt: Option<String>,
    tools_enabled: bool,
}

impl<P: LlmProvider> Agent<P> {
    pub fn new(
        provider: P,
        registry: ToolRegistry,
        context: ToolContext,
        max_tool_rounds: usize,
    ) -> Self {
        Self {
            provider,
            registry,
            context,
            max_tool_rounds,
            messages: Vec::new(),
            system_prompt: None,
            tools_enabled: true,
        }
    }

    pub fn with_system_prompt(mut self, system_prompt: String) -> Self {
        self.system_prompt = Some(system_prompt);
        self
    }

    /// Включает или выключает передачу tool definitions в LLM.
    pub fn with_tools_enabled(mut self, tools_enabled: bool) -> Self {
        self.tools_enabled = tools_enabled;
        self
    }

    pub async fn complete(
        &mut self,
        session: &mut Session,
        prompt: &str,
    ) -> Result<AgentResponse, AppError> {
        if self.messages.is_empty() {
            self.messages = session
                .messages()
                .iter()
                .map(LlmMessage::from_message)
                .collect();
            if let Some(prompt) = &self.system_prompt {
                self.messages.insert(
                    0,
                    LlmMessage {
                        role: "system".to_owned(),
                        content: Some(prompt.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                );
            }
        }
        let user = Message::new(Role::User, prompt)?;
        session.add_message(user.clone());
        self.messages.push(LlmMessage::from_message(&user));

        let mut usage = Usage {
            prompt_tokens: Some(0),
            completion_tokens: Some(0),
            total_tokens: Some(0),
        };
        let mut has_usage = false;
        for round in 0..self.max_tool_rounds {
            let request = CompletionRequest::from_llm_messages(
                session.model(),
                self.messages.clone(),
                if self.tools_enabled {
                    self.registry.definitions()
                } else {
                    Vec::new()
                },
            );
            let response = self.provider.complete(request).await?;
            if let Some(response_usage) = &response.usage {
                usage.add_assign(response_usage);
                has_usage = true;
            }
            self.messages.push(response.message.clone());

            let Some(tool_calls) = response.message.tool_calls.clone() else {
                let content = response
                    .content()
                    .ok_or_else(|| {
                        AppError::LlmResponse("ответ не содержит content или tool_calls".to_owned())
                    })?
                    .to_owned();
                session.add_message(Message::new(Role::Assistant, &content)?);
                return Ok(AgentResponse {
                    content,
                    tool_rounds: round,
                    usage: has_usage.then_some(usage),
                });
            };

            if let Ok(message) = LlmMessage::from_tool_response(&response.message) {
                session.add_message(message);
            }

            for call in tool_calls {
                let args: Value = match serde_json::from_str(&call.function.arguments) {
                    Ok(args) => args,
                    Err(error) => {
                        let content = format!("Некорректные JSON-аргументы: {error}");
                        self.push_tool_result(session, &call.id, content.clone(), true);
                        continue;
                    }
                };
                let result = self
                    .registry
                    .execute(&call.function.name, args, &self.context)
                    .await;
                let (content, persist) = match result {
                    Ok(result) => (result.content, !result.ephemeral),
                    Err(error) => (error.to_string(), true),
                };
                self.push_tool_result(session, &call.id, content, persist);
            }
        }

        Err(AppError::ToolRoundLimit(self.max_tool_rounds))
    }

    fn push_tool_result(
        &mut self,
        session: &mut Session,
        id: &str,
        content: String,
        persist: bool,
    ) {
        self.messages
            .push(LlmMessage::tool_result(id, content.clone()));
        if persist && let Ok(message) = Message::tool_result(id, content) {
            session.add_message(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppError, CompletionResponse, FunctionCall, LlmMessage, ToolCall};
    use async_trait::async_trait;

    struct FakeProvider {
        step: std::sync::Mutex<usize>,
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            let mut step = self.step.lock().unwrap();
            *step += 1;
            if *step == 1 {
                assert!(request.tools.is_some());
                Ok(CompletionResponse {
                    message: LlmMessage {
                        role: "assistant".into(),
                        content: None,
                        tool_calls: Some(vec![ToolCall {
                            id: "call-1".into(),
                            kind: "function".into(),
                            function: FunctionCall {
                                name: "list_directory".into(),
                                arguments: "{\"path\":\".\"}".into(),
                            },
                        }]),
                        tool_call_id: None,
                    },
                    finish_reason: Some("tool_calls".into()),
                    usage: None,
                })
            } else {
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|message| message.tool_call_id.as_deref() == Some("call-1"))
                );
                Ok(CompletionResponse {
                    message: LlmMessage {
                        role: "assistant".into(),
                        content: Some("Готово".into()),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    finish_reason: Some("stop".into()),
                    usage: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn executes_tool_and_returns_final_answer() {
        let root = std::env::temp_dir().join(format!("ai-agent-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let registry = crate::tools::default_registry().unwrap();
        let mut agent = Agent::new(
            FakeProvider {
                step: std::sync::Mutex::new(0),
            },
            registry,
            ToolContext::new(&root, false),
            3,
        );
        let mut session = Session::new(&root, "local").unwrap();
        let result = agent.complete(&mut session, "Покажи файлы").await.unwrap();
        assert_eq!(result.content, "Готово");
        assert_eq!(session.messages().len(), 4);
        let assistant = session
            .messages()
            .iter()
            .find(|message| message.tool_calls().is_some())
            .unwrap();
        assert_eq!(assistant.role(), Role::Assistant);
        assert_eq!(assistant.tool_calls().unwrap()[0].id, "call-1");
        let tool = session
            .messages()
            .iter()
            .find(|message| message.tool_call_id().is_some())
            .unwrap();
        assert_eq!(tool.role(), Role::Tool);
        assert_eq!(tool.tool_call_id(), Some("call-1"));
        assert!(
            session
                .messages()
                .iter()
                .any(|message| message.content() == "Готово")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
