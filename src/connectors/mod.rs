//! Платформенно-независимый слой источников сообщений.
//!
//! Конкретные интеграции (Graph, Telegram, Teams, WhatsApp) должны реализовывать
//! один read-only контракт и возвращать нормализованные сообщения.

pub mod auth;
pub mod calendar;
pub mod github;
pub mod gmail;
pub mod gmail_auth;
pub mod graph;
pub mod macos_calendar;
pub mod models;
pub mod tools;

pub use calendar::{CalendarEvent, GoogleCalendarClient};
pub use github::{GitHostingProvider, GitHubProvider};
pub use gmail::GmailMailClient;
pub use graph::GraphMailClient;
pub use models::{MessageQuery, NormalizedMessage, NormalizedThread, SourceKind};

use crate::AppError;
use async_trait::async_trait;

#[async_trait]
pub trait MessageSource: Send + Sync {
    fn source(&self) -> SourceKind;
    async fn list_messages(&self, query: &MessageQuery)
    -> Result<Vec<NormalizedMessage>, AppError>;
    async fn get_message(&self, id: &str) -> Result<NormalizedMessage, AppError>;
    async fn get_thread(&self, id: &str) -> Result<NormalizedThread, AppError>;
}
