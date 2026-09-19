//! Публичное библиотечное ядро учебного AI-агента.
//!
//! Бинарная точка входа находится в [`main`](../bin/ai-agent), а доменная
//! логика вынесена сюда, чтобы её можно было проверять через unit- и
//! интеграционные тесты.

pub mod agent;
pub mod agents;
pub mod ci;
pub mod cli;
pub mod config;
pub mod connectors;
pub mod domain;
pub mod error;
pub mod index;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod project_intelligence;
pub mod scheduler;
pub mod security_review;
pub mod tools;
pub mod word_processing;

pub use config::Config;
pub use domain::{Message, Role, Session, ToolCallMessage};
pub use error::AppError;
pub use llm::{
    CompletionRequest, CompletionResponse, FunctionCall, FunctionDefinition, LiteLlmProvider,
    LlmMessage, LlmProvider, ModelInfo, OllamaProvider, ToolCall, ToolDefinition, Usage,
};
pub use word_processing::{count_items, count_unique_words, first_item, parse_words};
