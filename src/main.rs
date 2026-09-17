//! Консольная точка входа учебного AI-агента.

use std::{
    io::{self, BufRead, Write},
    time::{Duration, Instant},
};

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

    if cli.graph_login {
        match ai_agent::connectors::auth::GraphAuth::from_env(Duration::from_secs(
            config.request_timeout_secs,
        )) {
            Ok(auth) => match auth.login().await {
                Ok(()) => println!("Microsoft Graph авторизация завершена."),
                Err(error) => eprintln!("Ошибка Microsoft Graph login: {error}"),
            },
            Err(error) => eprintln!("Ошибка Microsoft Graph login: {error}"),
        }
        return;
    }
    if cli.gmail_login {
        match ai_agent::connectors::gmail_auth::GmailAuth::from_env(Duration::from_secs(
            config.request_timeout_secs,
        )) {
            Ok(auth) => match auth.login().await {
                Ok(()) => println!("Gmail авторизация завершена."),
                Err(error) => eprintln!("Ошибка Gmail login: {error}"),
            },
            Err(error) => eprintln!("Ошибка Gmail login: {error}"),
        }
        return;
    }

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

    if cli.scheduler {
        let path = if cli.schedule_file.is_absolute() {
            cli.schedule_file.clone()
        } else {
            config.working_dir.join(&cli.schedule_file)
        };
        let schedule = match ai_agent::scheduler::ScheduleFile::load(&path) {
            Ok(schedule) => schedule,
            Err(error) => {
                eprintln!("Ошибка расписаний: {error}");
                return;
            }
        };
        run_scheduler(
            &mut session,
            provider,
            &config,
            &active_profile,
            &catalog,
            schedule,
        )
        .await;
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

async fn run_scheduler(
    session: &mut Session,
    provider: ConfiguredProvider,
    config: &Config,
    profile: &AgentProfile,
    catalog: &AgentCatalog,
    schedule: ai_agent::scheduler::ScheduleFile,
) {
    if schedule.jobs.iter().all(|job| !job.enabled) {
        eprintln!("В расписании нет включённых задач.");
        return;
    }
    loop {
        let job = match ai_agent::scheduler::wait_for_next(&schedule).await {
            Ok(Some(job)) => job,
            Ok(None) => return,
            Err(error) => {
                eprintln!("Ошибка scheduler: {error}");
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("Остановка scheduler по Ctrl-C.");
                return;
            }
            _ = request_completion(
                session,
                provider.clone(),
                config,
                profile,
                catalog,
                &job.prompt,
                false,
                true,
            ) => {}
        }
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
    request_completion(
        session, provider, config, profile, catalog, prompt, true, true,
    )
    .await;
}

async fn run_repl(
    session: &mut Session,
    mut provider: ConfiguredProvider,
    config: &Config,
    catalog: AgentCatalog,
    mut active_profile: AgentProfile,
) {
    let mut show_stats = true;
    let mut tools_enabled = true;
    print_banner(&config.provider, &active_profile.model, show_stats);
    if config.verbose {
        print_status(session);
    }

    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        print!("\x1b[1;36m❯\x1b[0m ");
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
            ReplCommand::Tools(argument) => match argument.as_deref() {
                None => println!(
                    "Tools: {}\nДоступные tools: {}",
                    if tools_enabled {
                        "включены"
                    } else {
                        "выключены"
                    },
                    config.enabled_tools.join(", ")
                ),
                Some("on") => {
                    tools_enabled = true;
                    println!("Tools включены для следующих запросов.");
                }
                Some("off") => {
                    tools_enabled = false;
                    println!("Tools выключены для следующих запросов.");
                }
                Some(value) => println!(
                    "Неизвестный режим tools: {value}. Используйте /tools on или /tools off."
                ),
            },
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
            ReplCommand::Config => println!("{}", config.to_pretty_json()),
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
            ReplCommand::Model(Some(model)) => match session.set_model(model) {
                Ok(()) => println!(
                    "\x1b[32m✓\x1b[0m Модель изменена: \x1b[1m{}\x1b[0m",
                    session.model()
                ),
                Err(error) => println!("Ошибка модели: {error}"),
            },
            ReplCommand::Model(None) => {
                println!("Текущая модель: \x1b[1m{}\x1b[0m", session.model())
            }
            ReplCommand::Stats(setting) => match setting.as_deref() {
                Some("on") => {
                    show_stats = true;
                    println!("\x1b[32m✓\x1b[0m Статистика включена.");
                }
                Some("off") => {
                    show_stats = false;
                    println!("\x1b[33m✓\x1b[0m Статистика выключена.");
                }
                Some(value) => println!(
                    "Неизвестная настройка: {value}. Используйте /stats on или /stats off."
                ),
                None => println!(
                    "Статистика: {}",
                    if show_stats {
                        "включена"
                    } else {
                        "выключена"
                    }
                ),
            },
            ReplCommand::Prompt(prompt) => {
                request_completion(
                    session,
                    provider.clone(),
                    config,
                    &active_profile,
                    &catalog,
                    &prompt,
                    show_stats,
                    tools_enabled,
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

#[allow(clippy::too_many_arguments)]
async fn request_completion(
    session: &mut Session,
    provider: ConfiguredProvider,
    config: &Config,
    profile: &AgentProfile,
    catalog: &AgentCatalog,
    prompt: &str,
    show_stats: bool,
    tools_enabled: bool,
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
    context.graph_base_url = std::env::var("MICROSOFT_GRAPH_BASE_URL").ok();
    context.graph_access_token = std::env::var("MICROSOFT_GRAPH_ACCESS_TOKEN").ok();
    context.gmail_client_id = std::env::var("GOOGLE_GMAIL_CLIENT_ID").ok();
    let system_prompt = match catalog.system_prompt(profile) {
        Ok(prompt) => prompt,
        Err(error) => {
            eprintln!("Ошибка skills: {error}");
            return;
        }
    };
    let mut agent = Agent::new(provider, registry, context, profile.max_tool_rounds)
        .with_system_prompt(system_prompt)
        .with_loop_limits(
            Some(Duration::from_secs(config.max_loop_seconds)),
            config.max_diff_bytes,
        )
        .with_tools_enabled(tools_enabled);
    let started = Instant::now();
    println!("\n\x1b[2m┌─ Вы запрашиваете\x1b[0m");
    println!("\x1b[2m│\x1b[0m {prompt}");
    println!("\x1b[2m└─ Ответ\x1b[0m\n");
    match agent.complete(session, prompt).await {
        Ok(response) => {
            println!("\x1b[1;32m◆ Assistant\x1b[0m\n{}", response.content);
            if show_stats {
                print_response_stats(response.usage.as_ref(), started.elapsed().as_secs_f64());
            }
            if let Err(error) = session.save_to(session_path(session)) {
                eprintln!("Предупреждение: не удалось сохранить сессию: {error}");
            }
        }
        Err(error) => eprintln!("Ошибка агента: {error}"),
    }
}

fn print_banner(provider: &str, model: &str, show_stats: bool) {
    println!("\n\x1b[1;35m╭────────────────────────────────────────╮\x1b[0m");
    println!(
        "\x1b[1;35m│\x1b[0m  \x1b[1mAI Agent v{}\x1b[0m  ·  \x1b[36m{provider}\x1b[0m",
        env!("CARGO_PKG_VERSION")
    );
    println!("\x1b[1;35m│\x1b[0m  Модель: \x1b[1m{model}\x1b[0m");
    println!(
        "\x1b[1;35m│\x1b[0m  Статистика: {}  ·  /help для команд",
        if show_stats { "on" } else { "off" }
    );
    println!("\x1b[1;35m╰────────────────────────────────────────╯\x1b[0m\n");
}

fn print_response_stats(usage: Option<&ai_agent::Usage>, elapsed_secs: f64) {
    let tokens = usage.and_then(|value| value.total_tokens);
    let prompt_tokens = usage.and_then(|value| value.prompt_tokens);
    let completion_tokens = usage.and_then(|value| value.completion_tokens);
    match (tokens, prompt_tokens, completion_tokens) {
        (Some(total), Some(prompt), Some(completion)) => println!(
            "\n\x1b[2m↳ {} токенов ({} prompt + {} ответ) · {:.2} с\x1b[0m\n",
            format_number(total),
            format_number(prompt),
            format_number(completion),
            elapsed_secs
        ),
        _ => println!("\n\x1b[2m↳ токены: н/д · {:.2} с\x1b[0m\n", elapsed_secs),
    }
}

fn format_number(value: u32) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(' ');
        }
        result.push(digit);
    }
    result
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
