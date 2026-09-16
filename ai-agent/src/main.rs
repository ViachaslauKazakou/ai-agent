//! Консольная точка входа учебного AI-агента.

use std::io::{self, BufRead, Write};

use ai_agent::cli::{Cli, ReplCommand, help_text, parse_repl_command};
use ai_agent::{
    Config, LiteLlmProvider, LlmProvider, ModelInfo, OllamaProvider, Session,
    agent::Agent,
    agents::{AgentCatalog, AgentProfile},
    index::ProjectIndex,
    tools::{ToolContext, registry_from_names},
};
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = match Config::load(&cli) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Ошибка конфигурации: {error}");
            return;
        }
    };

    let mut session = match Session::new(&config.working_dir, &config.model) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("Ошибка создания сессии: {error}");
            return;
        }
    };

    let catalog = match AgentCatalog::load(&config.working_dir, &config) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("Ошибка загрузки .aiagent/: {error}");
            return;
        }
    };
    let active_profile = catalog
        .profile("default")
        .expect("default profile exists")
        .clone();
    let provider = match ConfiguredProvider::new(&config, &active_profile) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("Ошибка инициализации LLM-провайдера: {error}");
            return;
        }
    };
    if let Err(error) = session.set_model(&active_profile.model) {
        eprintln!("Ошибка модели агента: {error}");
        return;
    }

    if let Some(prompt) = cli.prompt {
        run_once(
            &mut session,
            provider,
            &config,
            &active_profile,
            &catalog,
            &prompt,
        )
        .await;
    } else {
        run_repl(&mut session, provider, &config, catalog, active_profile).await;
    }
}

async fn run_once(
    session: &mut Session,
    provider: ConfiguredProvider,
    config: &Config,
    profile: &AgentProfile,
    catalog: &AgentCatalog,
    prompt: &str,
) {
    println!("Prompt: {prompt}");
    if config.verbose {
        print_status(session);
    }
    request_completion(session, provider, config, profile, catalog, prompt).await;
}

async fn run_repl(
    session: &mut Session,
    mut provider: ConfiguredProvider,
    config: &Config,
    catalog: AgentCatalog,
    mut active_profile: AgentProfile,
) {
    println!(
        "Учебный AI-агент ({}). Введите /help для справки.",
        config.provider
    );
    if config.verbose {
        print_status(session);
    }

    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        print!("agent> ");
        if let Err(error) = io::stdout().flush() {
            eprintln!("Ошибка вывода приглашения: {error}");
            return;
        }

        input.clear();
        match stdin.lock().read_line(&mut input) {
            Ok(0) => {
                println!();
                return;
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Ошибка чтения ввода: {error}");
                return;
            }
        }

        match parse_repl_command(&input) {
            ReplCommand::Help => println!("{}", help_text()),
            ReplCommand::Quit => {
                println!("Сессия завершена.");
                return;
            }
            ReplCommand::Clear => {
                println!("История очищена: {} сообщений.", session.clear_messages());
            }
            ReplCommand::Status => print_status(session),
            ReplCommand::Tools => println!("Доступные tools: {}", config.enabled_tools.join(", ")),
            ReplCommand::Models => match provider.list_models().await {
                Ok(models) if models.is_empty() => println!("Доступные модели не найдены."),
                Ok(models) => {
                    println!("Доступные модели:");
                    for model in models {
                        println!("- {}", model.id);
                    }
                }
                Err(error) => println!("Ошибка получения моделей: {error}"),
            },
            ReplCommand::Agents => {
                for profile in catalog.profiles() {
                    println!("- {}: {}", profile.name, profile.description);
                }
            }
            ReplCommand::Agent(None) => println!("Активный агент: {}", active_profile.name),
            ReplCommand::Agent(Some(name)) => match catalog.profile(&name) {
                Some(profile) => {
                    active_profile = profile.clone();
                    if let Err(error) = session.set_model(&active_profile.model) {
                        println!("Ошибка модели агента: {error}");
                    } else {
                        match ConfiguredProvider::new(config, &active_profile) {
                            Ok(new_provider) => {
                                provider = new_provider;
                                println!("Активный агент: {}", active_profile.name);
                            }
                            Err(error) => println!("Ошибка provider агента: {error}"),
                        }
                    }
                }
                None => println!("Неизвестный агент: {name}"),
            },
            ReplCommand::Skills => {
                for skill in catalog.skills() {
                    println!("- {}: {}", skill.name, skill.description);
                }
            }
            ReplCommand::Skill(name) => {
                if catalog.skills().any(|skill| skill.name == name) {
                    if !active_profile.skills.contains(&name) {
                        active_profile.skills.push(name.clone());
                    }
                    println!(
                        "Skill активирован для агента {}: {name}",
                        active_profile.name
                    );
                } else {
                    println!("Неизвестный skill: {name}");
                }
            }
            ReplCommand::Config => println!("Конфигурация: {config:?}"),
            ReplCommand::Permissions => println!(
                "allow_write={}, confirm_writes={}, command_allowlist={:?}",
                config.allow_write, config.confirm_writes, config.command_allowlist
            ),
            ReplCommand::Save => match session.save_to(session_path(session)) {
                Ok(()) => println!("Сессия сохранена."),
                Err(error) => println!("Ошибка сохранения: {error}"),
            },
            ReplCommand::Load => match Session::load_from(session_path(session)) {
                Ok(loaded) => {
                    *session = loaded;
                    println!("Сессия загружена.");
                }
                Err(error) => println!("Ошибка загрузки: {error}"),
            },
            ReplCommand::Index(argument) => {
                let path = ProjectIndex::index_path(session.working_dir());
                if argument.as_deref() == Some("status") {
                    match ProjectIndex::load(&path) {
                        Ok(index) => println!(
                            "Индекс: {} файлов, обновлён {}",
                            index.file_count(),
                            index.generated_secs
                        ),
                        Err(error) => println!("Индекс отсутствует или повреждён: {error}"),
                    }
                } else {
                    match ProjectIndex::build(session.working_dir())
                        .and_then(|index| index.save(&path).map(|()| index))
                    {
                        Ok(index) => println!("Индекс обновлён: {} файлов.", index.file_count()),
                        Err(error) => println!("Ошибка индекса: {error}"),
                    }
                }
            }
            ReplCommand::Search(query) => {
                let path = ProjectIndex::index_path(session.working_dir());
                match ProjectIndex::load(&path) {
                    Ok(index) => {
                        for hit in index.search(&query, 10) {
                            println!(
                                "{}:{}-{} (score={}):\n{}\n",
                                hit.path, hit.start_line, hit.end_line, hit.score, hit.text
                            );
                        }
                    }
                    Err(error) => println!("Сначала выполните /index: {error}"),
                }
            }
            ReplCommand::Model(model) => match session.set_model(model) {
                Ok(()) => println!("Модель изменена: {}", session.model()),
                Err(error) => println!("Ошибка модели: {error}"),
            },
            ReplCommand::Prompt(prompt) => {
                request_completion(
                    session,
                    provider.clone(),
                    config,
                    &active_profile,
                    &catalog,
                    &prompt,
                )
                .await;
            }
            ReplCommand::Empty => {}
            ReplCommand::Unknown(command) => {
                println!("Неизвестная команда: {command}. Введите /help.");
            }
        }
    }
}

async fn request_completion(
    session: &mut Session,
    provider: ConfiguredProvider,
    config: &Config,
    profile: &AgentProfile,
    catalog: &AgentCatalog,
    prompt: &str,
) {
    let registry = match registry_from_names(&profile.enabled_tools) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("Ошибка инструментов: {error}");
            return;
        }
    };
    let mut context = ToolContext::new(&config.working_dir, profile.allow_write);
    context.confirm_writes = profile.confirm_writes;
    context.command_allowlist = profile.command_allowlist.clone();
    context.interactive = true;
    let system_prompt = match catalog.system_prompt(profile) {
        Ok(prompt) => prompt,
        Err(error) => {
            eprintln!("Ошибка skills: {error}");
            return;
        }
    };
    let mut agent = Agent::new(provider, registry, context, profile.max_tool_rounds)
        .with_system_prompt(system_prompt);
    match agent.complete(session, prompt).await {
        Ok(response) => {
            println!("Assistant: {}", response.content);
            if let Err(error) = session.save_to(session_path(session)) {
                eprintln!("Предупреждение: не удалось сохранить сессию: {error}");
            }
        }
        Err(error) => eprintln!("Ошибка агента: {error}"),
    }
}

#[derive(Clone)]
enum ConfiguredProvider {
    LiteLlm(LiteLlmProvider),
    Ollama(OllamaProvider),
}

impl ConfiguredProvider {
    fn new(config: &Config, profile: &AgentProfile) -> Result<Self, ai_agent::AppError> {
        let mut profile_config = config.clone();
        profile_config.provider = profile.provider.clone();
        if profile.provider == "ollama" && config.provider != "ollama" {
            profile_config.api_base_url = "http://localhost:11434/v1".to_owned();
            profile_config.api_key = None;
        } else if profile.provider == "litellm" && config.provider != "litellm" {
            profile_config.api_base_url = "http://localhost:4000/v1".to_owned();
            profile_config.api_key = None;
        }
        match profile.provider.as_str() {
            "litellm" => Ok(Self::LiteLlm(LiteLlmProvider::new(&profile_config)?)),
            "ollama" => Ok(Self::Ollama(OllamaProvider::new(&profile_config)?)),
            other => Err(ai_agent::AppError::InvalidConfig(format!(
                "неподдерживаемый LLM_PROVIDER: {other}"
            ))),
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ai_agent::AppError> {
        match self {
            Self::LiteLlm(provider) => provider.list_models().await,
            Self::Ollama(provider) => provider.list_models().await,
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfiguredProvider {
    async fn complete(
        &self,
        request: ai_agent::CompletionRequest,
    ) -> Result<ai_agent::CompletionResponse, ai_agent::AppError> {
        match self {
            Self::LiteLlm(provider) => provider.complete(request).await,
            Self::Ollama(provider) => provider.complete(request).await,
        }
    }
}

fn session_path(session: &Session) -> std::path::PathBuf {
    session.working_dir().join(".agent-session.json")
}

fn print_status(session: &Session) {
    println!("Сессия: {}", session.id());
    println!("Модель: {}", session.model());
    println!("Рабочая директория: {}", session.working_dir().display());
    println!("Сообщений: {}", session.messages().len());
}
