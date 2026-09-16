//! Парсинг аргументов командной строки и команд REPL.

use std::path::PathBuf;

use clap::Parser;

/// Аргументы запуска приложения.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "ai-agent",
    version,
    about = "Учебный консольный AI-агент без LLM"
)]
pub struct Cli {
    /// Имя модели, которое будет сохранено в сессии.
    #[arg(long, default_value = "demo-model")]
    pub model: String,

    /// Рабочая директория сессии.
    #[arg(long, value_name = "PATH", default_value = ".")]
    pub working_dir: PathBuf,

    /// Включает диагностический вывод конфигурации.
    #[arg(short, long)]
    pub verbose: bool,

    /// Одноразовый prompt. Если не указан, запускается REPL.
    pub prompt: Option<String>,
}

/// Команда, распознанная внутри REPL.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplCommand {
    /// Показать список команд.
    Help,
    /// Завершить REPL.
    Quit,
    /// Очистить историю текущей сессии.
    Clear,
    /// Показать состояние текущей сессии.
    Status,
    /// Изменить имя модели текущей сессии.
    Model(String),
    /// Добавить обычный пользовательский prompt.
    Prompt(String),
    /// Пустая строка.
    Empty,
    /// Неизвестная slash-команда.
    Unknown(String),
}

/// Преобразует строку REPL в структурированную команду.
pub fn parse_repl_command(input: &str) -> ReplCommand {
    let input = input.trim();
    if input.is_empty() {
        return ReplCommand::Empty;
    }

    if !input.starts_with('/') {
        return ReplCommand::Prompt(input.to_owned());
    }

    let mut parts = input.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default();
    let argument = parts.next().unwrap_or_default().trim();

    match command {
        "/help" => ReplCommand::Help,
        "/exit" | "/quit" => ReplCommand::Quit,
        "/clear" => ReplCommand::Clear,
        "/status" => ReplCommand::Status,
        "/model" if argument.is_empty() => ReplCommand::Unknown(input.to_owned()),
        "/model" => ReplCommand::Model(argument.to_owned()),
        _ => ReplCommand::Unknown(command.to_owned()),
    }
}

/// Возвращает текст справки REPL.
pub fn help_text() -> &'static str {
    "Команды:\n  /help          показать эту справку\n  /status        показать состояние сессии\n  /model NAME    изменить имя модели\n  /clear         очистить историю\n  /exit, /quit   выйти из REPL\n\nЛюбой другой текст добавляется как сообщение пользователя."
}

#[cfg(test)]
mod tests {
    use super::{Cli, ReplCommand, parse_repl_command};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_cli_options_and_prompt() {
        let cli = Cli::try_parse_from([
            "ai-agent",
            "--model",
            "local-model",
            "--working-dir",
            "/tmp/project",
            "--verbose",
            "Изучи проект",
        ])
        .unwrap();

        assert_eq!(
            cli,
            Cli {
                model: "local-model".to_owned(),
                working_dir: PathBuf::from("/tmp/project"),
                verbose: true,
                prompt: Some("Изучи проект".to_owned()),
            }
        );
    }

    #[test]
    fn parses_repl_commands_and_plain_prompts() {
        assert_eq!(parse_repl_command("/help"), ReplCommand::Help);
        assert_eq!(parse_repl_command("/exit"), ReplCommand::Quit);
        assert_eq!(parse_repl_command("/quit"), ReplCommand::Quit);
        assert_eq!(
            parse_repl_command("/model local"),
            ReplCommand::Model("local".to_owned())
        );
        assert_eq!(
            parse_repl_command("прочитай README"),
            ReplCommand::Prompt("прочитай README".to_owned())
        );
    }
}
