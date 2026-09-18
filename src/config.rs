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
const JSON_CONFIG_FILE: &str = "config.json";
pub const PROJECT_CONFIG_PATH: &str = ".aiagent/config.json";

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct JsonConfig {
    provider: Option<String>,
    api_base_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    working_dir: Option<String>,
    max_tool_rounds: Option<usize>,
    request_timeout_secs: Option<u64>,
    log_level: Option<String>,
    verbose: Option<bool>,
    allow_write: Option<bool>,
    enabled_tools: Option<Vec<String>>,
    command_allowlist: Option<Vec<String>>,
    confirm_writes: Option<bool>,
    max_loop_seconds: Option<u64>,
    max_diff_bytes: Option<usize>,
    microsoft_graph_client_id: Option<String>,
    microsoft_graph_tenant: Option<String>,
    microsoft_graph_scope: Option<String>,
    google_gmail_client_id: Option<String>,
    google_gmail_client_secret: Option<String>,
}

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
    max_loop_seconds: Option<u64>,
    max_diff_bytes: Option<usize>,
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
    pub max_loop_seconds: u64,
    pub max_diff_bytes: usize,
    pub microsoft_graph_client_id: Option<String>,
    pub microsoft_graph_tenant: Option<String>,
    pub microsoft_graph_scope: Option<String>,
    pub google_gmail_client_id: Option<String>,
    pub google_gmail_client_secret: Option<String>,
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
    /// Возвращает безопасное читаемое JSON-представление конфигурации.
    /// Секрет API намеренно заменяется на `<redacted>`.
    pub fn to_pretty_json(&self) -> String {
        let value = serde_json::json!({
            "provider": self.provider,
            "api_base_url": self.api_base_url,
            "api_key": self.api_key.as_ref().map(|_| "<redacted>"),
            "model": self.model,
            "working_dir": self.working_dir,
            "max_tool_rounds": self.max_tool_rounds,
            "request_timeout_secs": self.request_timeout_secs,
            "log_level": self.log_level,
            "verbose": self.verbose,
            "allow_write": self.allow_write,
            "enabled_tools": self.enabled_tools,
            "command_allowlist": self.command_allowlist,
            "confirm_writes": self.confirm_writes,
            "max_loop_seconds": self.max_loop_seconds,
            "max_diff_bytes": self.max_diff_bytes,
            "microsoft_graph_client_id": self.microsoft_graph_client_id,
            "microsoft_graph_tenant": self.microsoft_graph_tenant,
            "microsoft_graph_scope": self.microsoft_graph_scope,
            "google_gmail_client_id": self.google_gmail_client_id,
            "google_gmail_client_secret": self.google_gmail_client_secret.as_ref().map(|_| "<redacted>"),
        });

        serde_json::to_string_pretty(&value).expect("configuration JSON should be serializable")
    }

    /// Загружает проектные `.env` и `.agent.toml`, затем применяет CLI.
    pub fn load(cli: &Cli) -> Result<Self, AppError> {
        let project_dir = resolve_project_dir(cli)?;
        migrate_legacy_project_config(&project_dir)?;
        initialize_project(&project_dir)?;
        let json_path = project_dir.join(PROJECT_CONFIG_PATH);
        let json = load_json_config(&json_path)?;
        let mut environment = env::vars().collect::<HashMap<_, _>>();
        if json.is_none() {
            match dotenvy::from_path(project_dir.join(".env")) {
                Ok(_) => environment = env::vars().collect(),
                Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::EnvironmentFile(error.to_string())),
            }
        }
        if let Some(config) = &json {
            set_if_some(&mut environment, "LLM_PROVIDER", config.provider.clone());
            set_if_some(&mut environment, "MODEL", config.model.clone());
            set_if_some(
                &mut environment,
                "LITELLM_BASE_URL",
                config.api_base_url.clone(),
            );
            set_if_some(&mut environment, "LITELLM_API_KEY", config.api_key.clone());
            if let Some(working_dir) = &config.working_dir {
                let path = PathBuf::from(working_dir);
                let resolved = if path.is_absolute() {
                    path
                } else {
                    project_dir.join(path)
                };
                environment.insert(
                    "WORKING_DIR".to_owned(),
                    resolved.to_string_lossy().into_owned(),
                );
            }
            set_if_some(&mut environment, "RUST_LOG", config.log_level.clone());
            set_if_some(
                &mut environment,
                "MICROSOFT_GRAPH_CLIENT_ID",
                config.microsoft_graph_client_id.clone(),
            );
            set_if_some(
                &mut environment,
                "MICROSOFT_GRAPH_TENANT",
                config.microsoft_graph_tenant.clone(),
            );
            set_if_some(
                &mut environment,
                "MICROSOFT_GRAPH_SCOPE",
                config.microsoft_graph_scope.clone(),
            );
            set_if_some(
                &mut environment,
                "GOOGLE_GMAIL_CLIENT_ID",
                config.google_gmail_client_id.clone(),
            );
            set_if_some(
                &mut environment,
                "GOOGLE_GMAIL_CLIENT_SECRET",
                config.google_gmail_client_secret.clone(),
            );
        }

        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| project_dir.join(PROJECT_CONFIG_PATH));
        let config_path = if config_path.is_absolute() {
            config_path
        } else {
            project_dir.join(config_path)
        };
        let file =
            if config_path.file_name().and_then(|name| name.to_str()) == Some(JSON_CONFIG_FILE) {
                json.clone().unwrap_or_default().into_file_config()
            } else {
                load_file_config(&config_path)?
            };
        let mut cli = cli.clone();
        if cli.working_dir.is_none() {
            cli.working_dir = Some(project_dir);
        }
        Self::from_sources_with_file(&cli, &environment, file)
    }

    pub fn save_model(&self, model: &str) -> Result<(), AppError> {
        persist_model(&self.working_dir, model)
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
                "apply_patch".to_owned(),
                "rollback_last_change".to_owned(),
                "search_files".to_owned(),
                "read_lines".to_owned(),
                "project_search".to_owned(),
            ]
        });
        let command_allowlist = file_agent.command_allowlist.unwrap_or_default();
        let confirm_writes = file_agent.confirm_writes.unwrap_or(false);
        let max_loop_seconds = file_agent.max_loop_seconds.unwrap_or(600);
        let max_diff_bytes = file_agent.max_diff_bytes.unwrap_or(100_000);
        let microsoft_graph_client_id = environment.get("MICROSOFT_GRAPH_CLIENT_ID").cloned();
        let microsoft_graph_tenant = environment.get("MICROSOFT_GRAPH_TENANT").cloned();
        let microsoft_graph_scope = environment.get("MICROSOFT_GRAPH_SCOPE").cloned();
        let google_gmail_client_id = environment.get("GOOGLE_GMAIL_CLIENT_ID").cloned();
        let google_gmail_client_secret = environment.get("GOOGLE_GMAIL_CLIENT_SECRET").cloned();
        let mut enabled_tools = enabled_tools;
        if (google_gmail_client_id.is_some() || microsoft_graph_client_id.is_some())
            && !enabled_tools
                .iter()
                .any(|tool| tool == "list_recent_emails")
        {
            enabled_tools.extend([
                "list_recent_emails".to_owned(),
                "get_email".to_owned(),
                "search_emails".to_owned(),
            ]);
        }
        if max_loop_seconds == 0 || max_diff_bytes == 0 {
            return Err(AppError::InvalidConfig(
                "лимиты coding loop должны быть больше нуля".to_owned(),
            ));
        }
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
            max_loop_seconds,
            max_diff_bytes,
            microsoft_graph_client_id,
            microsoft_graph_tenant,
            microsoft_graph_scope,
            google_gmail_client_id,
            google_gmail_client_secret,
        })
    }
}

impl JsonConfig {
    fn into_file_config(self) -> FileConfig {
        FileConfig {
            agent: Some(AgentFileConfig {
                max_tool_rounds: self.max_tool_rounds,
                allow_write: self.allow_write,
                enabled_tools: self.enabled_tools,
                command_allowlist: self.command_allowlist,
                confirm_writes: self.confirm_writes,
                max_loop_seconds: self.max_loop_seconds,
                max_diff_bytes: self.max_diff_bytes,
            }),
        }
    }
}

pub fn initialize_project(project_dir: &Path) -> Result<bool, AppError> {
    fs::create_dir_all(project_dir.join(".aiagent/agents"))
        .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    for directory in ["skills", "checkpoints"] {
        fs::create_dir_all(project_dir.join(".aiagent").join(directory))
            .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    }
    write_if_missing(
        project_dir.join(".aiagent/schedules.toml.example"),
        DEFAULT_SCHEDULE,
    )?;
    let agent_path = project_dir.join(".aiagent/agents/default.toml");
    let skill_path = project_dir.join(".aiagent/skills/testing/SKILL.md");
    fs::create_dir_all(agent_path.parent().expect("agent path has parent"))
        .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    fs::create_dir_all(skill_path.parent().expect("skill path has parent"))
        .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    write_if_missing(agent_path, DEFAULT_AGENT)?;
    write_if_missing(skill_path, DEFAULT_SKILL)?;
    let config_path = project_dir.join(PROJECT_CONFIG_PATH);
    if config_path.exists() {
        return Ok(false);
    }
    let values = load_env_file(project_dir.join(".env"))
        .or_else(|| load_env_file(project_dir.join(".env.example")))
        .unwrap_or_default();
    let mut config = JsonConfig {
        provider: values
            .get("LLM_PROVIDER")
            .cloned()
            .or_else(|| Some(DEFAULT_PROVIDER.to_owned())),
        api_base_url: values.get("LITELLM_BASE_URL").cloned(),
        api_key: values
            .get("LITELLM_API_KEY")
            .cloned()
            .filter(|value| !value.is_empty()),
        model: values
            .get("MODEL")
            .cloned()
            .or_else(|| Some(DEFAULT_MODEL.to_owned())),
        working_dir: Some(".".to_owned()),
        max_tool_rounds: values.get("MAX_TOOL_ROUNDS").and_then(|v| v.parse().ok()),
        request_timeout_secs: values
            .get("REQUEST_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok()),
        log_level: values.get("RUST_LOG").cloned(),
        ..Default::default()
    };
    if config.google_gmail_client_id.is_some() || config.microsoft_graph_client_id.is_some() {
        config.enabled_tools = Some(vec![
            "read_file".to_owned(),
            "list_directory".to_owned(),
            "write_file".to_owned(),
            "create_file".to_owned(),
            "apply_patch".to_owned(),
            "project_symbols".to_owned(),
            "project_diagnostics".to_owned(),
            "security_review".to_owned(),
            "list_recent_emails".to_owned(),
            "get_email".to_owned(),
            "search_emails".to_owned(),
        ]);
    }
    let data = serde_json::to_vec_pretty(&config)
        .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    write_if_missing(config_path, &data).map(|_| true)
}

fn migrate_legacy_project_config(project_dir: &Path) -> Result<(), AppError> {
    let new_path = project_dir.join(PROJECT_CONFIG_PATH);
    let old_path = project_dir.join(JSON_CONFIG_FILE);
    if !new_path.exists() && old_path.is_file() {
        fs::create_dir_all(new_path.parent().expect("project config has parent"))
            .map_err(|error| AppError::AgentConfig(error.to_string()))?;
        fs::copy(old_path, new_path).map_err(|error| AppError::AgentConfig(error.to_string()))?;
    }
    let old_agent = project_dir.join(".agent");
    let new_agent = project_dir.join(".aiagent");
    let old_checkpoints = old_agent.join("checkpoints");
    let new_checkpoints = new_agent.join("checkpoints");
    if old_checkpoints.is_dir() {
        fs::create_dir_all(&new_checkpoints)
            .map_err(|error| AppError::AgentConfig(error.to_string()))?;
        for entry in fs::read_dir(old_checkpoints)
            .map_err(|error| AppError::AgentConfig(error.to_string()))?
            .flatten()
        {
            let target = new_checkpoints.join(entry.file_name());
            if !target.exists() {
                fs::copy(entry.path(), target)
                    .map_err(|error| AppError::AgentConfig(error.to_string()))?;
            }
        }
    }
    Ok(())
}

pub fn persist_model(project_dir: &Path, model: &str) -> Result<(), AppError> {
    let path = project_dir.join(PROJECT_CONFIG_PATH);
    let mut config = load_json_config(&path)?.unwrap_or_default();
    config.model = Some(model.to_owned());
    let data = serde_json::to_vec_pretty(&config)
        .map_err(|error| AppError::AgentConfig(error.to_string()))?;
    fs::write(path, data).map_err(|error| AppError::AgentConfig(error.to_string()))
}

fn load_json_config(path: &Path) -> Result<Option<JsonConfig>, AppError> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::AgentConfig(format!(
            "{}: {error}",
            path.display()
        ))),
    }
}

fn load_env_file(path: PathBuf) -> Option<HashMap<String, String>> {
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (key, value) = line.split_once('=')?;
                Some((
                    key.trim().to_owned(),
                    value.trim().trim_matches('"').to_owned(),
                ))
            })
            .collect(),
    )
}

fn set_if_some(environment: &mut HashMap<String, String>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        environment.insert(key.to_owned(), value);
    }
}

fn write_if_missing(path: PathBuf, content: impl AsRef<[u8]>) -> Result<bool, AppError> {
    if path.exists() {
        return Ok(false);
    }
    fs::write(path, content).map_err(|error| AppError::AgentConfig(error.to_string()))?;
    Ok(true)
}

const DEFAULT_AGENT: &[u8] = br#"description = "Default project agent"
model = "demo-model"
system_prompt = "Work safely in this project. Explain a plan before changes and run relevant checks."
enabled_tools = ["read_file", "list_directory", "write_file", "create_file", "apply_patch", "project_symbols", "project_diagnostics", "security_review"]
allow_write = true
confirm_writes = true
command_allowlist = []
max_tool_rounds = 20
skills = ["testing"]
"#;
const DEFAULT_SKILL: &[u8] = b"description: Project testing guidance\n\nRun the relevant formatter, checker, and tests after changes.\n";
const DEFAULT_SCHEDULE: &[u8] = br#"# Copy to schedules.toml and add enabled jobs.
[[jobs]]
name = "tests"
cron = "0 0 * * * *"
prompt = "Run the project tests and report failures."
enabled = false
"#;

fn resolve_project_dir(cli: &Cli) -> Result<PathBuf, AppError> {
    let raw = cli
        .project_dir
        .clone()
        .or_else(|| cli.working_dir.clone())
        .or_else(|| env::var_os("AI_AGENT_PROJECT_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    absolute_existing_directory(&raw.to_string_lossy())
}

#[cfg(test)]
mod project_root_tests {
    use super::resolve_project_dir;
    use crate::cli::Cli;
    use clap::Parser;

    #[test]
    fn working_dir_is_used_as_project_root_without_project_dir() {
        let cli = Cli::try_parse_from(["ai-agent", "--working-dir", "/tmp"]).unwrap();
        assert_eq!(
            resolve_project_dir(&cli).unwrap(),
            std::path::PathBuf::from("/tmp").canonicalize().unwrap()
        );
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
                | "open_file"
                | "list_directory"
                | "write_file"
                | "create_file"
                | "delete_file"
                | "apply_patch"
                | "rollback_last_change"
                | "git_status"
                | "git_diff"
                | "git_log"
                | "git_create_branch"
                | "git_prepare_commit"
                | "git_commit"
                | "git_push"
                | "git_create_pr"
                | "project_symbols"
                | "project_diagnostics"
                | "project_definition"
                | "security_review"
                | "ci_status"
                | "ci_failure_analysis"
                | "search_files"
                | "read_lines"
                | "project_search"
                | "run_command"
                | "list_recent_emails"
                | "get_email"
                | "search_emails"
        ) {
            return Err(AppError::UnknownTool(tool.clone()));
        }
    }
    if allow_write
        && !tools
            .iter()
            .any(|tool| tool == "write_file" || tool == "create_file")
    {
        return Err(AppError::InvalidConfig(
            "allow_write = true требует добавления write_file или create_file в enabled_tools"
                .to_owned(),
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
    use super::{AgentFileConfig, Config, FileConfig, initialize_project};
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
                    max_loop_seconds: None,
                    max_diff_bytes: None,
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
                    max_loop_seconds: None,
                    max_diff_bytes: None,
                }),
            },
        );

        assert_eq!(
            result.unwrap_err(),
            crate::AppError::UnknownTool("shell".to_owned())
        );
    }

    #[test]
    fn initializes_empty_project_idempotently() {
        let root = std::env::temp_dir().join(format!("ai-bootstrap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(initialize_project(&root).unwrap());
        assert!(!initialize_project(&root).unwrap());
        assert!(root.join(".aiagent/config.json").is_file());
        assert!(root.join(".aiagent/agents/default.toml").is_file());
        assert!(root.join(".aiagent/skills/testing/SKILL.md").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_mail_tools_are_visible_in_effective_config() {
        let cli = Cli::try_parse_from(["ai-agent"]).unwrap();
        let mut environment = HashMap::new();
        environment.insert("GOOGLE_GMAIL_CLIENT_ID".to_owned(), "client".to_owned());
        let config = Config::from_sources(&cli, &environment).unwrap();
        assert!(
            config
                .enabled_tools
                .iter()
                .any(|tool| tool == "list_recent_emails")
        );
        assert!(config.enabled_tools.iter().any(|tool| tool == "get_email"));
        assert!(
            config
                .enabled_tools
                .iter()
                .any(|tool| tool == "search_emails")
        );
    }
}
