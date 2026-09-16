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
}

impl Message {
    /// Создаёт сообщение, отклоняя пустой или состоящий только из пробелов текст.
    pub fn new(role: Role, content: impl Into<String>) -> Result<Self, AppError> {
        let content = content.into();
        if content.trim().is_empty() {
            return Err(AppError::EmptyMessage);
        }

        Ok(Self { role, content })
    }

    /// Возвращает роль сообщения без передачи владения.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Возвращает текст сообщения как заимствованную строку.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Состояние одного диалога агента.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    id: Uuid,
    working_dir: PathBuf,
    model: String,
    messages: Vec<Message>,
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
    fn empty_model_is_rejected() {
        assert_eq!(Session::new(".", "\t").unwrap_err(), AppError::EmptyModel);
    }
}
