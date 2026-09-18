//! Парсинг аргументов командной строки и команд REPL.

use std::path::PathBuf;

use clap::{ArgAction, Parser};

/// Аргументы запуска приложения.
#[derive(Clone, Debug, Parser, PartialEq, Eq)]
#[command(
    name = "ai-agent",
    version,
    about = "Учебный консольный AI-агент без LLM"
)]
pub struct Cli {
    /// LLM provider: `litellm` (по умолчанию) или `ollama`.
    #[arg(long)]
    pub provider: Option<String>,

    /// Имя модели, которое будет сохранено в сессии.
    #[arg(long)]
    pub model: Option<String>,

    /// Совместимый OpenAI/LiteLLM base URL.
    #[arg(long, value_name = "URL")]
    pub base_url: Option<String>,

    /// Рабочая директория сессии.
    #[arg(long, value_name = "PATH")]
    pub working_dir: Option<PathBuf>,

    /// Каталог проекта с `.env`, `.agent.toml` и `.aiagent`.
    #[arg(long, value_name = "PATH")]
    pub project_dir: Option<PathBuf>,

    /// Создать project-local config.json и .aiagent, затем завершить работу.
    #[arg(long, action = ArgAction::SetTrue)]
    pub init: bool,

    /// Путь к TOML-конфигурации агента.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Максимальное число будущих раундов инструментов.
    #[arg(long, value_name = "N")]
    pub max_tool_rounds: Option<usize>,

    /// Тайм-аут будущих HTTP-запросов в секундах.
    #[arg(long, value_name = "SECONDS")]
    pub request_timeout_secs: Option<u64>,

    /// Разрешает инструменту write_file изменять файлы внутри working-dir.
    #[arg(long, action = ArgAction::SetTrue)]
    pub allow_write: bool,

    /// Включает диагностический вывод конфигурации.
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub verbose: Option<bool>,

    /// Одноразовый prompt. Если не указан, запускается REPL.
    pub prompt: Option<String>,

    /// Запустить встроенный кроссплатформенный runner расписаний.
    #[arg(long, action = ArgAction::SetTrue)]
    pub scheduler: bool,

    /// TOML-файл расписаний для `--scheduler`.
    #[arg(long, default_value = ".aiagent/schedules.toml", value_name = "PATH")]
    pub schedule_file: PathBuf,

    /// Выполнить Microsoft device-code login и сохранить refresh token в OS keychain.
    #[arg(long, action = ArgAction::SetTrue)]
    pub graph_login: bool,

    /// Выполнить Gmail OAuth login и сохранить refresh token в OS keychain.
    #[arg(long, action = ArgAction::SetTrue)]
    pub gmail_login: bool,

    /// Выполнить Google Calendar OAuth login и сохранить refresh token в OS keychain.
    #[arg(long, action = ArgAction::SetTrue)]
    pub google_calendar_login: bool,
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
    /// Показать, включить или выключить tools.
    Tools(Option<String>),
    Config,
    Permissions,
    Save,
    Load,
    Index(Option<String>),
    Search(String),
    /// Показать модели, доступные через LLM endpoint.
    Models,
    Agents,
    Agent(Option<String>),
    /// Показать или выбрать рабочую роль (alias для `/agent`).
    Role(Option<String>),
    Skills,
    Skill(String),
    /// Показать или изменить имя модели текущей сессии.
    Model(Option<String>),
    /// Включить, выключить или показать статистику ответа.
    Stats(Option<String>),
    Compact,
    CreateAgent(Option<String>),
    CreateSkill(Option<String>),
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
        "/exit" | "/quit" | "/q" | "/bye" => ReplCommand::Quit,
        "/clear" => ReplCommand::Clear,
        "/status" => ReplCommand::Status,
        "/tools" => ReplCommand::Tools((!argument.is_empty()).then(|| argument.to_owned())),
        "/config" => ReplCommand::Config,
        "/permissions" => ReplCommand::Permissions,
        "/save" => ReplCommand::Save,
        "/load" => ReplCommand::Load,
        "/index" => ReplCommand::Index((!argument.is_empty()).then(|| argument.to_owned())),
        "/search" if !argument.is_empty() => ReplCommand::Search(argument.to_owned()),
        "/search" => ReplCommand::Unknown(input.to_owned()),
        "/models" => ReplCommand::Models,
        "/agents" => ReplCommand::Agents,
        "/agent" => ReplCommand::Agent((!argument.is_empty()).then(|| argument.to_owned())),
        "/role" => ReplCommand::Role((!argument.is_empty()).then(|| argument.to_owned())),
        "/skills" => ReplCommand::Skills,
        "/skill" if !argument.is_empty() => ReplCommand::Skill(argument.to_owned()),
        "/skill" => ReplCommand::Unknown(input.to_owned()),
        "/model" => ReplCommand::Model((!argument.is_empty()).then(|| argument.to_owned())),
        "/stats" => ReplCommand::Stats((!argument.is_empty()).then(|| argument.to_owned())),
        "/compact" => ReplCommand::Compact,
        "/create-agent" => {
            ReplCommand::CreateAgent((!argument.is_empty()).then(|| argument.to_owned()))
        }
        "/create-skill" => {
            ReplCommand::CreateSkill((!argument.is_empty()).then(|| argument.to_owned()))
        }
        _ => ReplCommand::Unknown(command.to_owned()),
    }
}

/// Возвращает текст справки REPL.
pub fn help_text() -> &'static str {
    "Команды:\n  /help          показать эту справку\n  /status        показать состояние сессии\n  /tools         показать состояние и доступные tools\n  /tools on|off  включить/выключить tools для текущего запуска\n  /models        показать доступные модели\n  /agents        показать project-local агентов\n  /agent [NAME]  показать или выбрать агента\n  /role [NAME]   показать или выбрать рабочую роль\n  /create-agent  создать project-local агента через wizard\n  /skills        показать project-local skills\n  /skill NAME    выбрать skill агента\n  /create-skill  создать project-local skill через wizard\n  /config        показать конфигурацию\n  /permissions   показать permissions\n  /index         построить/обновить индекс проекта\n  /index status  показать состояние индекса\n  /search QUERY  поиск по индексированным фрагментам\n  /model         показать текущую модель\n  /model NAME    изменить имя модели\n  /stats         показать настройки статистики\n  /stats on|off  включить/выключить токены и время\n  /clear         очистить историю\n  /save, /load   сохранить/загрузить историю\n  /exit, /quit   выйти из REPL\n\nКонфигурация агентов и skills хранится в .aiagent/.\nЛюбой другой текст добавляется как сообщение пользователя."
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
                provider: None,
                model: Some("local-model".to_owned()),
                base_url: None,
                working_dir: Some(PathBuf::from("/tmp/project")),
                project_dir: None,
                init: false,
                config: None,
                max_tool_rounds: None,
                request_timeout_secs: None,
                allow_write: false,
                verbose: Some(true),
                prompt: Some("Изучи проект".to_owned()),
                scheduler: false,
                schedule_file: PathBuf::from(".aiagent/schedules.toml"),
                graph_login: false,
                gmail_login: false,
                google_calendar_login: false,
            }
        );
    }

    #[test]
    fn parses_repl_commands_and_plain_prompts() {
        assert_eq!(parse_repl_command("/help"), ReplCommand::Help);
        assert_eq!(parse_repl_command("/exit"), ReplCommand::Quit);
        assert_eq!(parse_repl_command("/quit"), ReplCommand::Quit);
        assert_eq!(parse_repl_command("/models"), ReplCommand::Models);
        assert_eq!(
            parse_repl_command("/role secretary"),
            ReplCommand::Role(Some("secretary".to_owned()))
        );
        assert_eq!(parse_repl_command("/role"), ReplCommand::Role(None));
        assert_eq!(
            parse_repl_command("/role create"),
            ReplCommand::Role(Some("create".to_owned()))
        );
        assert_eq!(
            parse_repl_command("/model local"),
            ReplCommand::Model(Some("local".to_owned()))
        );
        assert_eq!(
            parse_repl_command("прочитай README"),
            ReplCommand::Prompt("прочитай README".to_owned())
        );
    }
}
