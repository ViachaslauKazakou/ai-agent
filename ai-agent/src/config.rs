//! Сборка и валидация конфигурации приложения.

use std::{
    collections::HashMap,
    env, fmt, fs,
    path::{Path, PathBuf},
};

use crate::{AppError, cli::Cli};

const DEFAULT_BASE_URL: &str = "http://localhost:4000/v1";
const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
const DEFAULT_PROVIDER: &str = "litellm";
const DEFAULT_MODEL: &str = "demo-model";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 20;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 120;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_CONFIG_FILE: &str = ".agent.toml";

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    agent: Option<AgentFileConfig>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFileConfig {
    max_tool_rounds: Option<usize>,
    allow_write: Option<bool>,
    enabled_tools: Option<Vec<String>>,
    command_allowlist: Option<Vec<String>>,
    confirm_writes: Option<bool>,
}

/// Итоговая конфигурация после объединения defaults, окружения и CLI.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    /// Имя LLM-провайдера, например `litellm`.
    pub provider: String,
    /// Base URL будущего LLM-провайдера.
    pub api_base_url: String,
    /// Необязательный секрет API; не включается в отображение конфигурации.
    pub api_key: Option<String>,
    /// Имя модели.
    pub model: String,
    /// Абсолютная существующая рабочая директория.
    pub working_dir: PathBuf,
    /// Лимит будущих раундов инструментов.
    pub max_tool_rounds: usize,
    /// Тайм-аут будущих HTTP-запросов.
    pub request_timeout_secs: u64,
    /// Уровень логирования.
    pub log_level: String,
    /// Включён ли подробный вывод CLI.
    pub verbose: bool,
    /// Разрешена ли запись файлов инструментом `write_file`.
    pub allow_write: bool,
    /// Имена tools, разрешённых конфигурацией.
    pub enabled_tools: Vec<String>,
    pub command_allowlist: Vec<String>,
    pub confirm_writes: bool,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("provider", &self.provider)
            .field("api_base_url", &self.api_base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("working_dir", &self.working_dir)
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("log_level", &self.log_level)
            .field("verbose", &self.verbose)
            .field("allow_write", &self.allow_write)
            .field("enabled_tools", &self.enabled_tools)
            .field("command_allowlist", &self.command_allowlist)
            .field("confirm_writes", &self.confirm_writes)
            .finish()
    }
}

impl Config {
    /// Загружает `.env`, затем применяет переменные процесса и CLI.
    pub fn load(cli: &Cli) -> Result<Self, AppError> {
        match dotenvy::dotenv() {
            Ok(_) => {}
            Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::EnvironmentFile(error.to_string())),
        }

        let environment = env::vars().collect::<HashMap<_, _>>();
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE));
        let file = load_file_config(&config_path)?;
        Self::from_sources_with_file(cli, &environment, file)
    }

    /// Собирает конфигурацию из defaults, переданного окружения и CLI.
    pub fn from_sources(
        cli: &Cli,
        environment: &HashMap<String, String>,
    ) -> Result<Self, AppError> {
        Self::from_sources_with_file(cli, environment, FileConfig::default())
    }

    fn from_sources_with_file(
        cli: &Cli,
        environment: &HashMap<String, String>,
        file: FileConfig,
    ) -> Result<Self, AppError> {
        let file_agent = file.agent.unwrap_or_default();
        let provider = cli
            .provider
            .clone()
            .or_else(|| environment.get("LLM_PROVIDER").cloned())
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned());
        if provider.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "LLM_PROVIDER не может быть пустым".to_owned(),
            ));
        }
        if provider != "litellm" && provider != "ollama" {
            return Err(AppError::InvalidConfig(format!(
                "неподдерживаемый LLM_PROVIDER: {provider}"
            )));
        }
        let base_url = cli
            .base_url
            .clone()
            .or_else(|| {
                let variable = if provider == "ollama" {
                    "OLLAMA_BASE_URL"
                } else {
                    "LITELLM_BASE_URL"
                };
                environment.get(variable).cloned()
            })
            .unwrap_or_else(|| {
                if provider == "ollama" {
                    DEFAULT_OLLAMA_BASE_URL.to_owned()
                } else {
                    DEFAULT_BASE_URL.to_owned()
                }
            });
        let api_key = if provider == "ollama" {
            environment.get("OLLAMA_API_KEY").cloned()
        } else {
            environment.get("LITELLM_API_KEY").cloned()
        };
        let model = cli
            .model
            .clone()
            .or_else(|| environment.get("MODEL").cloned())
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let raw_working_dir = cli
            .working_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| environment.get("WORKING_DIR").cloned())
            .unwrap_or_else(|| ".".to_owned());
        let resolved_working_dir = absolute_existing_directory(&raw_working_dir)?;
        let max_tool_rounds = match cli.max_tool_rounds.or(file_agent.max_tool_rounds) {
            Some(value) => value,
            None => parse_env::<usize>(environment, "MAX_TOOL_ROUNDS")?
                .unwrap_or(DEFAULT_MAX_TOOL_ROUNDS),
        };
        let request_timeout_secs = match cli.request_timeout_secs {
            Some(value) => value,
            None => parse_env::<u64>(environment, "REQUEST_TIMEOUT_SECS")?
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
        };
        let log_level = environment
            .get("RUST_LOG")
            .cloned()
            .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());
        let verbose = match cli.verbose {
            Some(value) => value,
            None => parse_env_bool(environment, "AI_AGENT_VERBOSE")?.unwrap_or(false),
        };

        if model.trim().is_empty() {
            return Err(AppError::EmptyModel);
        }
        if base_url.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "base URL не может быть пустым".to_owned(),
            ));
        }
        if max_tool_rounds == 0 {
            return Err(AppError::InvalidConfig(
                "MAX_TOOL_ROUNDS должен быть больше нуля".to_owned(),
            ));
        }
        if request_timeout_secs == 0 {
            return Err(AppError::InvalidConfig(
                "REQUEST_TIMEOUT_SECS должен быть больше нуля".to_owned(),
            ));
        }

        let allow_write = cli.allow_write || file_agent.allow_write.unwrap_or(false);
        let enabled_tools = file_agent.enabled_tools.unwrap_or_else(|| {
            vec![
                "read_file".to_owned(),
                "list_directory".to_owned(),
                "write_file".to_owned(),
                "search_files".to_owned(),
                "read_lines".to_owned(),
                "project_search".to_owned(),
            ]
        });
        let command_allowlist = file_agent.command_allowlist.unwrap_or_default();
        let confirm_writes = file_agent.confirm_writes.unwrap_or(false);
        validate_tools(&enabled_tools, allow_write)?;

        Ok(Self {
            provider,
            api_base_url: base_url,
            api_key,
            model,
            working_dir: resolved_working_dir,
            max_tool_rounds,
            request_timeout_secs,
            log_level,
            verbose,
            allow_write,
            enabled_tools,
            command_allowlist,
            confirm_writes,
        })
    }
}

fn load_file_config(path: &Path) -> Result<FileConfig, AppError> {
    match fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(error) => Err(AppError::AgentConfig(format!(
            "{}: {error}",
            path.display()
        ))),
    }
}

fn validate_tools(tools: &[String], allow_write: bool) -> Result<(), AppError> {
    for tool in tools {
        if !matches!(
            tool.as_str(),
            "read_file"
                | "list_directory"
                | "write_file"
                | "search_files"
                | "read_lines"
                | "project_search"
                | "run_command"
        ) {
            return Err(AppError::UnknownTool(tool.clone()));
        }
    }
    if allow_write && !tools.iter().any(|tool| tool == "write_file") {
        return Err(AppError::InvalidConfig(
            "allow_write = true требует добавления write_file в enabled_tools".to_owned(),
        ));
    }
    Ok(())
}

fn parse_env<T>(environment: &HashMap<String, String>, name: &str) -> Result<Option<T>, AppError>
where
    T: std::str::FromStr,
{
    let Some(value) = environment.get(name) else {
        return Ok(None);
    };
    value
        .parse()
        .map(Some)
        .map_err(|_| AppError::InvalidEnvironmentValue {
            name: name.to_owned(),
            value: value.clone(),
        })
}

fn parse_env_bool(
    environment: &HashMap<String, String>,
    name: &str,
) -> Result<Option<bool>, AppError> {
    let Some(value) = environment.get(name) else {
        return Ok(None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(AppError::InvalidEnvironmentValue {
            name: name.to_owned(),
            value: value.clone(),
        }),
    }
}

fn absolute_existing_directory(value: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map_err(|error| AppError::InvalidWorkingDirectory(error.to_string()))?
            .join(path)
    };

    if !absolute.is_dir() {
        return Err(AppError::InvalidWorkingDirectory(
            absolute.display().to_string(),
        ));
    }

    absolute
        .canonicalize()
        .map_err(|error| AppError::InvalidWorkingDirectory(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{AgentFileConfig, Config, FileConfig};
    use crate::cli::Cli;
    use clap::Parser;
    use std::{collections::HashMap, path::Path};

    fn cli(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("ai-agent").chain(arguments.iter().copied())).unwrap()
    }

    #[test]
    fn applies_defaults_when_environment_and_cli_are_empty() {
        let config = Config::from_sources(&cli(&["--working-dir", "."]), &HashMap::new()).unwrap();

        assert_eq!(config.api_base_url, "http://localhost:4000/v1");
        assert_eq!(config.provider, "litellm");
        assert_eq!(config.model, "demo-model");
        assert_eq!(config.max_tool_rounds, 20);
        assert_eq!(config.request_timeout_secs, 120);
        assert!(!config.verbose);
        assert!(config.working_dir.is_absolute());
    }

    #[test]
    fn cli_overrides_environment() {
        let mut environment = HashMap::from([
            ("LLM_PROVIDER".to_owned(), "litellm".to_owned()),
            ("LITELLM_BASE_URL".to_owned(), "http://env/v1".to_owned()),
            ("MODEL".to_owned(), "env-model".to_owned()),
            ("MAX_TOOL_ROUNDS".to_owned(), "5".to_owned()),
            ("REQUEST_TIMEOUT_SECS".to_owned(), "60".to_owned()),
        ]);
        environment.insert("WORKING_DIR".to_owned(), "/tmp".to_owned());

        let config = Config::from_sources(
            &cli(&[
                "--base-url",
                "http://cli/v1",
                "--model",
                "cli-model",
                "--max-tool-rounds",
                "7",
                "--request-timeout-secs",
                "30",
                "--working-dir",
                ".",
            ]),
            &environment,
        )
        .unwrap();

        assert_eq!(config.api_base_url, "http://cli/v1");
        assert_eq!(config.model, "cli-model");
        assert_eq!(config.max_tool_rounds, 7);
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.working_dir, Path::new(".").canonicalize().unwrap());
    }

    #[test]
    fn invalid_working_directory_is_rejected() {
        let result = Config::from_sources(
            &cli(&["--working-dir", "/path/that/does/not/exist"]),
            &HashMap::new(),
        );

        assert!(matches!(
            result,
            Err(crate::AppError::InvalidWorkingDirectory(_))
        ));
    }

    #[test]
    fn rejects_empty_model_and_invalid_environment_value() {
        let empty_model = Config::from_sources(
            &cli(&["--model", "", "--working-dir", "."]),
            &HashMap::new(),
        );
        assert_eq!(empty_model.unwrap_err(), crate::AppError::EmptyModel);

        let invalid_rounds = Config::from_sources(
            &cli(&["--working-dir", "."]),
            &HashMap::from([(String::from("MAX_TOOL_ROUNDS"), String::from("many"))]),
        );
        assert!(matches!(
            invalid_rounds,
            Err(crate::AppError::InvalidEnvironmentValue { .. })
        ));
    }

    #[test]
    fn rejects_unknown_provider() {
        let result = Config::from_sources(
            &cli(&["--working-dir", "."]),
            &HashMap::from([(String::from("LLM_PROVIDER"), String::from("unknown"))]),
        );

        assert!(matches!(result, Err(crate::AppError::InvalidConfig(_))));
    }

    #[test]
    fn cli_value_masks_invalid_environment_value() {
        let config = Config::from_sources(
            &cli(&["--working-dir", ".", "--max-tool-rounds", "3"]),
            &HashMap::from([(String::from("MAX_TOOL_ROUNDS"), String::from("many"))]),
        )
        .unwrap();

        assert_eq!(config.max_tool_rounds, 3);
    }

    #[test]
    fn rejects_write_permission_without_write_tool() {
        let result = Config::from_sources_with_file(
            &cli(&["--working-dir", "."]),
            &HashMap::new(),
            FileConfig {
                agent: Some(AgentFileConfig {
                    max_tool_rounds: None,
                    allow_write: Some(true),
                    enabled_tools: Some(vec!["read_file".to_owned()]),
                    command_allowlist: None,
                    confirm_writes: None,
                }),
            },
        );

        assert!(matches!(result, Err(crate::AppError::InvalidConfig(_))));
    }

    #[test]
    fn rejects_unknown_config_tool() {
        let result = Config::from_sources_with_file(
            &cli(&["--working-dir", "."]),
            &HashMap::new(),
            FileConfig {
                agent: Some(AgentFileConfig {
                    max_tool_rounds: None,
                    allow_write: None,
                    enabled_tools: Some(vec!["shell".to_owned()]),
                    command_allowlist: None,
                    confirm_writes: None,
                }),
            },
        );

        assert_eq!(
            result.unwrap_err(),
            crate::AppError::UnknownTool("shell".to_owned())
        );
    }
}
