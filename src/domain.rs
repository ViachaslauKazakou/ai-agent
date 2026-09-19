//! Предметные типы сессии агента.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;

/// Автор или назначение сообщения в истории диалога.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Инструкция, задающая поведение агента.
    System,
    /// Сообщение пользователя.
    User,
    /// Ответ языковой модели или агента.
    Assistant,
    /// Результат выполнения инструмента.
    Tool,
}

/// Одно сообщение сессии.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    role: Role,
    content: String,
    tool_calls: Option<Vec<ToolCallMessage>>,
    tool_call_id: Option<String>,
}

/// Минимальные данные вызова инструмента, сохраняемые в истории сессии.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallMessage {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Краткая запись выполнения tool без аргументов и содержимого результата.
/// Она помогает восстановить ход сессии и не сохраняет потенциально чувствительные данные.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub kind: String,
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

impl Message {
    /// Создаёт сообщение, отклоняя пустой или состоящий только из пробелов текст.
    pub fn new(role: Role, content: impl Into<String>) -> Result<Self, AppError> {
        let content = content.into();
        if content.trim().is_empty() {
            return Err(AppError::EmptyMessage);
        }

        Ok(Self {
            role,
            content,
            tool_calls: None,
            tool_call_id: None,
        })
    }

    /// Создаёт assistant-сообщение, содержащее вызовы инструментов.
    pub fn assistant_tool_calls(
        content: Option<String>,
        tool_calls: Vec<ToolCallMessage>,
    ) -> Result<Self, AppError> {
        if tool_calls.is_empty() {
            return Err(AppError::LlmResponse(
                "assistant tool_calls пусты".to_owned(),
            ));
        }
        Ok(Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        })
    }

    /// Создаёт tool-сообщение с идентификатором вызова.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, AppError> {
        let content = content.into();
        Ok(Self {
            role: Role::Tool,
            content,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        })
    }

    /// Возвращает роль сообщения без передачи владения.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Возвращает текст сообщения как заимствованную строку.
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn tool_calls(&self) -> Option<&[ToolCallMessage]> {
        self.tool_calls.as_deref()
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool_call_id.as_deref()
    }
}

/// Состояние одного диалога агента.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    id: Uuid,
    working_dir: PathBuf,
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    events: Vec<SessionEvent>,
}

impl Session {
    /// Создаёт новую сессию с UUID и пустой историей сообщений.
    pub fn new(
        working_dir: impl Into<PathBuf>,
        model: impl Into<String>,
    ) -> Result<Self, AppError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(AppError::EmptyModel);
        }

        Ok(Self {
            id: Uuid::new_v4(),
            working_dir: working_dir.into(),
            model,
            messages: Vec::new(),
            events: Vec::new(),
        })
    }

    /// Возвращает стабильный идентификатор этой сессии.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Возвращает рабочую директорию без передачи владения.
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// Возвращает имя модели без передачи владения.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Изменяет имя модели после проверки, что оно не пустое.
    pub fn set_model(&mut self, model: impl Into<String>) -> Result<(), AppError> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(AppError::EmptyModel);
        }

        self.model = model;
        Ok(())
    }

    /// Возвращает историю сообщений только для чтения.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Возвращает краткий audit trail вызовов tools без их payload-ов.
    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    /// Сохраняет безопасную метаинформацию о выполнении tool.
    pub fn record_tool_event(&mut self, name: impl Into<String>, success: bool, duration_ms: u64) {
        self.events.push(SessionEvent {
            kind: "tool".to_owned(),
            name: name.into(),
            success,
            duration_ms,
        });
    }

    /// Добавляет сообщение в конец истории и возвращает новую длину истории.
    pub fn add_message(&mut self, message: Message) -> usize {
        self.messages.push(message);
        self.messages.len()
    }

    /// Удаляет всю историю сообщений и возвращает количество удалённых сообщений.
    pub fn clear_messages(&mut self) -> usize {
        let count = self.messages.len();
        self.messages.clear();
        count
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), AppError> {
        let data = serde_json::to_vec_pretty(self)
            .map_err(|error| AppError::SessionPersistence(error.to_string()))?;
        std::fs::write(path, data).map_err(|error| AppError::SessionPersistence(error.to_string()))
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let data =
            std::fs::read(path).map_err(|error| AppError::SessionPersistence(error.to_string()))?;
        serde_json::from_slice(&data)
            .map_err(|error| AppError::SessionPersistence(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Message, Role, Session};
    use crate::AppError;

    #[test]
    fn message_exposes_role_and_content_without_transferring_ownership() {
        let message = Message::new(Role::User, "Привет").unwrap();

        assert_eq!(message.role(), Role::User);
        assert_eq!(message.content(), "Привет");
    }

    #[test]
    fn empty_message_is_rejected() {
        assert_eq!(
            Message::new(Role::User, "  ").unwrap_err(),
            AppError::EmptyMessage
        );
    }

    #[test]
    fn session_starts_empty_and_stores_messages_in_order() {
        let mut session = Session::new(".", "demo-model").unwrap();
        let first = Message::new(Role::System, "Ты помощник").unwrap();
        let second = Message::new(Role::User, "Привет").unwrap();

        assert_eq!(session.add_message(first), 1);
        assert_eq!(session.add_message(second), 2);
        assert_eq!(session.messages()[0].role(), Role::System);
        assert_eq!(session.messages()[1].content(), "Привет");
    }

    #[test]
    fn preserves_tool_call_metadata_in_session_messages() {
        let assistant = Message::assistant_tool_calls(
            None,
            vec![super::ToolCallMessage {
                id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{\"path\":\"README.md\"}".to_owned(),
            }],
        )
        .unwrap();
        let tool = Message::tool_result("call-1", "content").unwrap();

        assert_eq!(assistant.role(), Role::Assistant);
        assert_eq!(assistant.tool_calls().unwrap()[0].id, "call-1");
        assert_eq!(tool.role(), Role::Tool);
        assert_eq!(tool.tool_call_id(), Some("call-1"));
    }

    #[test]
    fn empty_model_is_rejected() {
        assert_eq!(Session::new(".", "\t").unwrap_err(), AppError::EmptyModel);
    }
}
