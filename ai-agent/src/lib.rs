//! Публичное библиотечное ядро учебного AI-агента.
//!
//! Бинарная точка входа находится в [`main`](../bin/ai-agent), а доменная
//! логика вынесена сюда, чтобы её можно было проверять через unit- и
//! интеграционные тесты.

pub mod cli;
pub mod domain;
pub mod error;
pub mod word_processing;

pub use domain::{Message, Role, Session};
pub use error::AppError;
pub use word_processing::{count_items, count_unique_words, first_item, parse_words};
