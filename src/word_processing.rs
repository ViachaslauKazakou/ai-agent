//! Чистые функции обработки пользовательского текста.

use std::collections::HashSet;

use crate::error::AppError;

/// Преобразует текст в список принадлежащих программе строк.
pub fn parse_words(input: &str) -> Result<Vec<String>, AppError> {
    let words: Vec<String> = input.split_whitespace().map(str::to_owned).collect();

    if words.is_empty() {
        Err(AppError::EmptyInput)
    } else {
        Ok(words)
    }
}

/// Возвращает количество элементов в любом срезе.
pub fn count_items<T>(items: &[T]) -> usize {
    items.len()
}

/// Возвращает ссылку на первый элемент или `None` для пустого среза.
pub fn first_item<T>(items: &[T]) -> Option<&T> {
    items.first()
}

/// Считает количество различных слов без изменения исходного списка.
pub fn count_unique_words(words: &[String]) -> usize {
    let unique_words: HashSet<&str> = words.iter().map(String::as_str).collect();
    unique_words.len()
}

#[cfg(test)]
mod tests {
    use super::{count_unique_words, parse_words};
    use crate::AppError;

    #[test]
    fn parses_words_and_preserves_order() {
        let words = parse_words("Rust   делает  владение понятным").unwrap();

        assert_eq!(words, ["Rust", "делает", "владение", "понятным"]);
    }

    #[test]
    fn rejects_empty_input_with_domain_error() {
        assert_eq!(parse_words(" \n\t").unwrap_err(), AppError::EmptyInput);
    }

    #[test]
    fn counts_unique_words() {
        let words = vec!["rust".to_owned(), "код".to_owned(), "rust".to_owned()];

        assert_eq!(count_unique_words(&words), 2);
    }
}
