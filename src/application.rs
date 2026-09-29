//! Application-facing API for desktop and browser clients.
//!
//! This module deliberately contains transport-neutral data transfer objects
//! (DTOs), rather than Tauri, HTTP, or frontend-specific types.  Keeping the
//! contract here gives every client the same vocabulary and prevents a UI from
//! reaching into `Agent`, `Session`, or `ToolContext` internals.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::Utc;
use clap::Parser;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppError, Config, LaunchState, LaunchStateStore, LiteLlmProvider, OllamaProvider,
    ProviderRegistry, Session,
    agents::AgentCatalog,
    cli::Cli,
    tools::{ToolContext, registry_from_names},
};

/// Version of the command/event contract exchanged with external clients.
///
/// The version is part of every envelope so a future desktop application can
/// reject an incompatible backend instead of silently misinterpreting a
/// command or event.
pub const APPLICATION_API_VERSION: u16 = 1;

/// Small, secret-free description of a project opened by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDto {
    /// Opaque identifier stable across launches when persistence is configured.
    pub id: String,
    /// Canonical project path displayed by the client.
    pub path: PathBuf,
}

/// Persisted project metadata rendered by a desktop startup screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupProjectDto {
    /// Stable opaque identifier from the launch-state document.
    pub id: String,
    /// Last canonical path recorded for the project.
    pub path: PathBuf,
    /// Whether the path currently resolves to an accessible directory.
    pub available: bool,
    /// UTC timestamp of the most recent successful open operation.
    pub last_opened_at: chrono::DateTime<Utc>,
}

/// Secret-free state required to render the desktop startup screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupStateDto {
    /// UTC timestamp of the previous or current recorded application launch.
    pub last_started_at: Option<chrono::DateTime<Utc>>,
    /// Stable identifier of the project suggested to the user.
    pub suggested_project_id: Option<String>,
    /// Persisted projects ordered from most recently opened to least recent.
    pub recent_projects: Vec<StartupProjectDto>,
}

/// Secret-free representation of a session available to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDto {
    /// Session UUID used in subsequent commands.
    pub id: Uuid,
    /// Project to which the session belongs.
    pub project_id: String,
    /// Provider paired with the selected model. Keeping this pair prevents a
    /// LiteLLM model from being sent to Ollama (or the reverse).
    pub provider: String,
    /// Model name; credentials and provider configuration are intentionally absent.
    pub model: String,
    /// Number of messages currently retained by the session.
    pub message_count: usize,
}

/// Capabilities that a frontend may use to conditionally render controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationCapabilities {
    /// API contract version understood by this backend.
    pub api_version: u16,
    /// Whether the backend can emit incremental assistant text events.
    pub streaming: bool,
    /// Whether an in-flight request can be cancelled.
    pub cancellation: bool,
    /// Whether the backend has a confirmation boundary for mutating tools.
    pub confirmations: bool,
}

/// Commands accepted by a desktop or browser adapter.
///
/// Commands contain identifiers and user intent only.  They do not contain
/// tokens, filesystem handles, or raw tool internals; those remain owned by
/// the backend service and its permission checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ApplicationCommand {
    /// Ask the backend for its supported protocol features.
    GetCapabilities,
    /// Return persisted, secret-free metadata for the startup screen.
    GetStartupState,
    /// Register a project path for later session commands.
    OpenProject { path: PathBuf },
    /// Return projects already registered in this service instance.
    ListProjects,
    /// Create a new empty session for an opened project.
    CreateSession {
        project_id: String,
        provider: String,
        model: String,
    },
    /// Return sessions currently known to the service.
    ListSessions { project_id: String },
    /// Request cancellation of an in-flight operation.
    CancelRequest { request_id: Uuid },
    /// Return configured provider names and their public model metadata.
    ListModels,
    /// Query configured provider endpoints for their current model list.
    RefreshModels { project_id: String },
    /// Start one prompt for an existing session.
    SendMessage { session_id: Uuid, prompt: String },
}

/// Events emitted by the application service.
///
/// The first stage exposes state and lifecycle events.  Streaming text,
/// tool events, confirmations, and cancellation are represented explicitly so
/// later implementations can extend the service without coupling it to a UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ApplicationEvent {
    /// Response containing the backend feature set.
    Capabilities(ApplicationCapabilities),
    /// Persisted metadata required by the desktop startup screen.
    StartupState(StartupStateDto),
    /// Project was accepted and is ready for session creation.
    ProjectOpened(ProjectDto),
    /// Projects currently registered in the service.
    ProjectsListed { projects: Vec<ProjectDto> },
    /// A new session became available.
    SessionCreated(SessionDto),
    /// Current sessions for a project.
    SessionsListed {
        project_id: String,
        sessions: Vec<SessionDto>,
    },
    /// Confirms that cancellation was requested for an operation.
    RequestCancelled { request_id: Uuid },
    /// Models available from the configured provider registry.
    ModelsListed { providers: Vec<ProviderDto> },
    /// Indicates that agent execution has started for a request.
    RequestStarted { session_id: Uuid },
    /// Final non-streaming assistant response.
    AssistantMessage {
        session_id: Uuid,
        content: String,
        tool_rounds: usize,
    },
    /// Tool status suitable for a UI timeline.
    ToolStatus { session_id: Uuid, status: String },
    /// Stable error event suitable for rendering in a client.
    Error { code: String, message: String },
}

/// Public provider metadata safe to show in a frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDto {
    /// Registry key used by project profiles.
    pub name: String,
    /// Provider implementation kind, never its endpoint credentials.
    pub kind: String,
    /// Models declared in the project registry.
    pub models: Vec<String>,
    /// Whether the last live model request reached this provider successfully.
    pub reachable: bool,
}

/// Configuration documents exposed to the desktop settings editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsDocuments {
    /// Project behavior, access, and connector settings.
    pub config_json: String,
    /// Provider endpoints, model allowlists, and provider credentials.
    pub providers_json: String,
}

/// Envelope used to correlate a client command with emitted events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationEnvelope<T> {
    /// Protocol version used for this message.
    pub api_version: u16,
    /// Client-generated request identifier.
    pub request_id: Uuid,
    /// Monotonically increasing event number within one service instance.
    pub sequence: u64,
    /// Command or event payload.
    pub payload: T,
}

/// Minimal stateful service used by transport adapters.
///
/// It owns only application metadata in this stage.  LLM execution will be
/// added behind the same service boundary in the streaming/cancellation stage;
/// keeping this state separate from a transport makes the design reusable for
/// Tauri IPC, a loopback browser transport, and the existing CLI adapter.
#[derive(Default)]
pub struct ApplicationService {
    projects: BTreeMap<String, ProjectDto>,
    sessions: BTreeMap<Uuid, SessionDto>,
    providers: Vec<ProviderDto>,
    /// Project configuration remains private because it can contain secrets.
    configs: BTreeMap<String, Config>,
    /// Runtime agents are kept separate from DTO state and keyed by session.
    agents: BTreeMap<Uuid, DesktopAgent>,
    next_sequence: u64,
    cancellations: BTreeMap<Uuid, RequestCancellation>,
    /// User-scoped metadata is kept separate from project configuration.
    launch_state: LaunchState,
    /// The store is optional so embedded clients and tests remain filesystem-free.
    launch_state_store: Option<LaunchStateStore>,
}

/// Selects the configured provider implementation while keeping credentials
/// inside the Rust client, exactly as the CLI does.
fn desktop_provider(config: &Config, name: &str) -> Result<DesktopProvider, AppError> {
    let provider = config
        .providers
        .provider(name)
        .ok_or_else(|| AppError::InvalidConfig(format!("provider not found: {name}")))?;
    let mut selected = config.clone();
    selected.provider = name.to_owned();
    selected.api_base_url = provider.base_url.clone();
    selected.api_key = provider.api_key.clone();
    match provider.kind.as_str() {
        "ollama" => Ok(DesktopProvider::Ollama {
            provider: OllamaProvider::new(&selected)?,
            supports_reasoning_effort: provider.supports_reasoning_effort,
            supports_reasoning_with_tools: provider.supports_reasoning_with_tools,
            reasoning_effort_models: provider.reasoning_effort_models.clone(),
            reasoning_with_tools_models: provider.reasoning_with_tools_models.clone(),
        }),
        _ => Ok(DesktopProvider::LiteLlm {
            provider: LiteLlmProvider::new(&selected)?,
            supports_reasoning_effort: provider.supports_reasoning_effort,
            supports_reasoning_with_tools: provider.supports_reasoning_with_tools,
            reasoning_effort_models: provider.reasoning_effort_models.clone(),
            reasoning_with_tools_models: provider.reasoning_with_tools_models.clone(),
        }),
    }
}

/// Runtime state required to execute a session without exposing internals to
/// the transport or frontend layers.
struct DesktopAgent {
    agent: crate::agent::Agent<DesktopProvider>,
    session: Session,
}

/// Provider wrapper that applies project configuration and keeps provider
/// selection reusable between CLI and desktop code.
#[derive(Clone)]
enum DesktopProvider {
    LiteLlm {
        provider: crate::LiteLlmProvider,
        supports_reasoning_effort: bool,
        supports_reasoning_with_tools: bool,
        reasoning_effort_models: Vec<String>,
        reasoning_with_tools_models: Vec<String>,
    },
    Ollama {
        provider: crate::OllamaProvider,
        supports_reasoning_effort: bool,
        supports_reasoning_with_tools: bool,
        reasoning_effort_models: Vec<String>,
        reasoning_with_tools_models: Vec<String>,
    },
}

#[async_trait::async_trait]
impl crate::LlmProvider for DesktopProvider {
    async fn complete(
        &self,
        request: crate::CompletionRequest,
    ) -> Result<crate::CompletionResponse, AppError> {
        let mut request = request;
        let (
            supports_reasoning_effort,
            supports_reasoning_with_tools,
            reasoning_effort_models,
            reasoning_with_tools_models,
        ) = match self {
            Self::LiteLlm {
                supports_reasoning_effort,
                supports_reasoning_with_tools,
                reasoning_effort_models,
                reasoning_with_tools_models,
                ..
            }
            | Self::Ollama {
                supports_reasoning_effort,
                supports_reasoning_with_tools,
                reasoning_effort_models,
                reasoning_with_tools_models,
                ..
            } => (
                *supports_reasoning_effort,
                *supports_reasoning_with_tools,
                reasoning_effort_models,
                reasoning_with_tools_models,
            ),
        };
        request.apply_reasoning_capabilities(
            supports_reasoning_effort,
            supports_reasoning_with_tools,
            reasoning_effort_models,
            reasoning_with_tools_models,
        );
        match self {
            Self::LiteLlm { provider, .. } => provider.complete(request).await,
            Self::Ollama { provider, .. } => provider.complete(request).await,
        }
    }
}

/// Cooperative cancellation handle shared by a service and its async worker.
///
/// Cancellation is cooperative rather than forceful: an HTTP provider or a
/// mutating tool must observe the flag at a safe boundary and stop there.  The
/// design avoids aborting a write halfway through a checkpoint transaction.
#[derive(Debug, Clone, Default)]
pub struct RequestCancellation {
    cancelled: Arc<AtomicBool>,
}

impl RequestCancellation {
    /// Creates a handle in the active state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the operation as cancelled.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether the worker should stop at its next safe boundary.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl ApplicationService {
    /// Creates an empty service with no open projects or sessions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a service backed by persistent user-scoped launch metadata.
    ///
    /// Loading and recording the launch happen here so every desktop transport
    /// observes the same startup state. An invalid document is returned to the
    /// composition root instead of being overwritten or hidden.
    pub fn with_launch_state_store(store: LaunchStateStore) -> Result<Self, AppError> {
        let mut launch_state = store.load()?;
        launch_state.record_launch(Utc::now());
        store.save(&launch_state)?;
        Ok(Self {
            launch_state,
            launch_state_store: Some(store),
            ..Self::default()
        })
    }

    /// Returns current startup metadata without opening or initializing projects.
    pub fn startup_state(&self) -> StartupStateDto {
        StartupStateDto {
            last_started_at: self.launch_state.last_started_at,
            suggested_project_id: self.launch_state.last_project_id.clone(),
            recent_projects: self
                .launch_state
                .projects
                .iter()
                .map(|project| StartupProjectDto {
                    id: project.id.clone(),
                    path: project.path.clone(),
                    available: project.path.is_dir(),
                    last_opened_at: project.last_opened_at,
                })
                .collect(),
        }
    }

    /// Returns the capabilities advertised by this service instance.
    pub fn capabilities(&self) -> ApplicationCapabilities {
        ApplicationCapabilities {
            api_version: APPLICATION_API_VERSION,
            streaming: false,
            cancellation: true,
            confirmations: true,
        }
    }

    /// Replaces public provider metadata loaded by the host configuration.
    ///
    /// Credentials are intentionally not accepted here; the host keeps them
    /// inside provider implementations and exposes only names, kinds, and
    /// model identifiers to a frontend.
    pub fn set_providers(&mut self, providers: Vec<ProviderDto>) {
        self.providers = providers;
    }

    /// Loads project-local configuration through the existing Config loader.
    ///
    /// The service deliberately reuses the CLI configuration path instead of
    /// implementing a second parser. Only safe provider metadata is copied to
    /// the frontend-facing state; credentials stay inside `Config`.
    pub fn load_project_config(&mut self, project: &ProjectDto) -> Result<(), AppError> {
        let path = project.path.to_string_lossy().into_owned();
        let cli = Cli::try_parse_from(["ai-agent", "--working-dir", path.as_str()])
            .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
        let config = Config::load(&cli)?;
        self.providers = public_providers(&config.providers);
        self.configs.insert(project.id.clone(), config);
        Ok(())
    }

    /// Queries each configured provider's `/models` endpoint.
    ///
    /// The registry remains the source of provider credentials and endpoints.
    /// Only returned model identifiers are copied into the public DTO state.
    pub async fn refresh_models(&mut self, project_id: &str) -> Result<Vec<ProviderDto>, AppError> {
        let config = self
            .configs
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?
            .clone();
        let mut refreshed = Vec::new();

        for (name, provider_config) in &config.providers.providers {
            let mut provider_configured = config.clone();
            provider_configured.provider = name.clone();
            provider_configured.api_base_url = provider_config.base_url.clone();
            provider_configured.api_key = provider_config.api_key.clone();
            let declared_models = provider_config.models.clone();
            let live_models = match provider_config.kind.as_str() {
                "ollama" => match OllamaProvider::new(&provider_configured) {
                    Ok(provider) => provider.list_models().await,
                    Err(error) => Err(error),
                },
                _ => match LiteLlmProvider::new(&provider_configured) {
                    Ok(provider) => provider.list_models().await,
                    Err(error) => Err(error),
                },
            };
            let (models, reachable) = match live_models {
                Ok(models) => (models.into_iter().map(|model| model.id).collect(), true),
                Err(_) => (declared_models, false),
            };
            refreshed.push(ProviderDto {
                name: name.clone(),
                kind: provider_config.kind.clone(),
                models,
                reachable,
            });
        }
        self.providers = refreshed.clone();
        Ok(refreshed)
    }

    /// Opens a project path and returns a process-local identifier.
    ///
    /// Canonicalization and full configuration loading remain in the existing
    /// config layer.  This method only establishes the application boundary;
    /// transport adapters must pass the validated path they received from that
    /// layer rather than duplicating path policy here.
    pub fn open_project(&mut self, path: PathBuf) -> ProjectDto {
        if let Some(project) = self.projects.values().find(|project| project.path == path) {
            return project.clone();
        }
        let id = format!("project-{}", self.projects.len() + 1);
        let project = ProjectDto {
            id: id.clone(),
            path,
        };
        self.projects.insert(id, project.clone());
        project
    }

    /// Creates a session metadata record for an opened project.
    pub fn create_session(
        &mut self,
        project_id: &str,
        provider: String,
        model: String,
    ) -> Result<SessionDto, String> {
        if !self.projects.contains_key(project_id) {
            return Err(format!("project not found: {project_id}"));
        }
        if model.trim().is_empty() {
            return Err("model must not be empty".to_owned());
        }
        let model = model.trim().to_owned();
        let session = SessionDto {
            id: Uuid::new_v4(),
            project_id: project_id.to_owned(),
            provider,
            model,
            message_count: 0,
        };
        self.sessions.insert(session.id, session.clone());
        Ok(session)
    }

    /// Creates the runtime agent for a session using the same project-local
    /// profile, tools, provider, and safety settings as the CLI.
    fn initialize_agent(
        &mut self,
        session: &SessionDto,
        activity: Arc<Mutex<Option<String>>>,
    ) -> Result<(), AppError> {
        let config = self
            .configs
            .get(&session.project_id)
            .ok_or_else(|| {
                AppError::InvalidConfig("project configuration is not loaded".to_owned())
            })?
            .clone();
        let catalog = AgentCatalog::load(&config.working_dir, &config)?;
        let profile = catalog
            .profile("default")
            .ok_or_else(|| AppError::AgentConfig("default agent profile is missing".to_owned()))?;
        let registry = registry_from_names(&profile.enabled_tools)?;
        let mut context = ToolContext::new(&config.working_dir, config.allow_write);
        context.confirm_writes = config.confirm_writes;
        context.command_allowlist = profile.command_allowlist.clone();
        context.interactive = false;
        context.graph_client_id = config.microsoft_graph_client_id.clone();
        context.graph_tenant = config.microsoft_graph_tenant.clone();
        context.graph_scope = config.microsoft_graph_scope.clone();
        context.gmail_client_id = config.google_gmail_client_id.clone();
        context.gmail_client_secret = config.google_gmail_client_secret.clone();
        context.google_calendar_client_id = config.google_calendar_client_id.clone();
        context.google_calendar_client_secret = config.google_calendar_client_secret.clone();
        context.web_search_provider = config.web_search_provider.clone();
        context.web_search_endpoint = config.web_search_endpoint.clone();
        context.web_search_api_key = config.web_search_api_key.clone();
        context.status = activity;
        let provider = desktop_provider(&config, &session.provider)?;
        let agent = crate::agent::Agent::new(provider, registry, context, profile.max_tool_rounds)
            .with_system_prompt(catalog.system_prompt(profile)?)
            .with_loop_limits(
                Some(std::time::Duration::from_secs(config.max_loop_seconds)),
                config.max_diff_bytes,
            )
            .with_reasoning_effort(config.reasoning_effort.clone());
        // Session model is selected by the desktop model list. The existing
        // Agent API derives the request model from Session, so no provider
        // credential or model selection is duplicated in the UI adapter.
        let mut runtime_session = Session::new(&config.working_dir, &session.model)?;
        runtime_session.set_model(&session.model)?;
        self.agents.insert(
            session.id,
            DesktopAgent {
                agent,
                session: runtime_session,
            },
        );
        Ok(())
    }

    /// Runs the existing non-streaming Agent loop for a desktop prompt.
    pub async fn send_message(
        &mut self,
        request_id: Uuid,
        session_id: Uuid,
        prompt: &str,
    ) -> Result<ApplicationEnvelope<ApplicationEvent>, AppError> {
        self.send_message_with_activity(request_id, session_id, prompt, Arc::new(Mutex::new(None)))
            .await
    }

    /// Runs a prompt while sharing the tool activity slot with a transport.
    /// The Tauri adapter polls this slot and emits timeline events without
    /// holding the service lock, so the UI remains responsive during tools.
    pub async fn send_message_with_activity(
        &mut self,
        request_id: Uuid,
        session_id: Uuid,
        prompt: &str,
        activity: Arc<Mutex<Option<String>>>,
    ) -> Result<ApplicationEnvelope<ApplicationEvent>, AppError> {
        let session_dto = self
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| AppError::InvalidConfig("session not found".to_owned()))?;
        if prompt.trim().is_empty() {
            return Err(AppError::EmptyMessage);
        }
        if !self.agents.contains_key(&session_id) {
            self.initialize_agent(&session_dto, activity)?;
        }
        let runtime = self
            .agents
            .get_mut(&session_id)
            .ok_or_else(|| AppError::InvalidConfig("agent runtime not found".to_owned()))?;
        let response = runtime.agent.complete(&mut runtime.session, prompt).await?;
        let message_count = runtime.session.messages().len();
        if let Some(dto) = self.sessions.get_mut(&session_id) {
            dto.message_count = message_count;
        }
        Ok(ApplicationEnvelope {
            api_version: APPLICATION_API_VERSION,
            request_id,
            sequence: self.next_sequence(),
            payload: ApplicationEvent::AssistantMessage {
                session_id,
                content: response.content,
                tool_rounds: response.tool_rounds,
            },
        })
    }

    /// Reads both project-local settings documents for the settings dialog.
    pub fn read_settings(&self, project_id: &str) -> Result<SettingsDocuments, AppError> {
        let project = self
            .projects
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?;
        Ok(SettingsDocuments {
            config_json: Config::read_project_config_json(&project.path)?,
            providers_json: Config::read_provider_config_json(&project.path)?,
        })
    }

    /// Validates and persists settings, then reloads the service configuration.
    pub fn write_settings(
        &mut self,
        project_id: &str,
        config_json: &str,
        providers_json: &str,
    ) -> Result<(), AppError> {
        let project =
            self.projects.get(project_id).cloned().ok_or_else(|| {
                AppError::InvalidConfig(format!("project not found: {project_id}"))
            })?;
        Config::write_project_config_json(&project.path, config_json)?;
        Config::write_provider_config_json(&project.path, providers_json)?;
        self.load_project_config(&project)
    }

    /// Lists only sessions belonging to the requested project.
    pub fn list_sessions(&self, project_id: &str) -> Vec<SessionDto> {
        self.sessions
            .values()
            .filter(|session| session.project_id == project_id)
            .cloned()
            .collect()
    }

    /// Allocates the next event sequence number for a transport adapter.
    pub fn next_sequence(&mut self) -> u64 {
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_sequence
    }

    /// Registers a request and returns the handle that should be passed to its
    /// provider/tool worker.  The handle is intentionally not serializable.
    pub fn register_request(&mut self, request_id: Uuid) -> RequestCancellation {
        let cancellation = RequestCancellation::new();
        self.cancellations.insert(request_id, cancellation.clone());
        cancellation
    }

    /// Marks a registered request as cancelled and reports whether it existed.
    pub fn cancel_request(&self, request_id: Uuid) -> bool {
        let Some(cancellation) = self.cancellations.get(&request_id) else {
            return false;
        };
        cancellation.cancel();
        true
    }

    /// Removes a completed request so cancellation state cannot accumulate.
    pub fn finish_request(&mut self, request_id: Uuid) {
        self.cancellations.remove(&request_id);
    }

    /// Executes one metadata command and returns exactly one correlated event.
    ///
    /// Keeping command dispatch in the service, instead of in a Tauri command
    /// or HTTP handler, gives every frontend the same validation and event
    /// semantics.  Later commands that start asynchronous agent work can use
    /// the same request ID while emitting additional lifecycle events.
    pub fn execute(
        &mut self,
        request_id: Uuid,
        command: ApplicationCommand,
    ) -> Result<ApplicationEnvelope<ApplicationEvent>, AppError> {
        let event = match command {
            ApplicationCommand::GetCapabilities => {
                ApplicationEvent::Capabilities(self.capabilities())
            }
            ApplicationCommand::GetStartupState => {
                ApplicationEvent::StartupState(self.startup_state())
            }
            ApplicationCommand::OpenProject { path } => {
                if !path.is_dir() {
                    return Err(AppError::InvalidWorkingDirectory(
                        path.display().to_string(),
                    ));
                }
                let canonical = path.canonicalize().map_err(|error| {
                    AppError::InvalidWorkingDirectory(format!("{}: {error}", path.display()))
                })?;
                if let Some(project) = self
                    .projects
                    .values()
                    .find(|project| project.path == canonical)
                    .cloned()
                {
                    return Ok(ApplicationEnvelope {
                        api_version: APPLICATION_API_VERSION,
                        request_id,
                        sequence: self.next_sequence(),
                        payload: ApplicationEvent::ProjectOpened(project),
                    });
                }

                // Mutate a snapshot first. Failed configuration or persistence
                // must not expose a partially registered project to clients.
                let mut next_launch_state = self.launch_state.clone();
                let recent = next_launch_state.record_project(canonical.clone(), Utc::now());
                let project = ProjectDto {
                    id: recent.id.clone(),
                    path: canonical,
                };
                self.load_project_config(&project)?;
                if let Some(store) = &self.launch_state_store
                    && let Err(error) = store.save(&next_launch_state)
                {
                    self.configs.remove(&project.id);
                    return Err(error);
                }
                self.launch_state = next_launch_state;
                self.projects.insert(project.id.clone(), project.clone());
                ApplicationEvent::ProjectOpened(project)
            }
            ApplicationCommand::ListProjects => ApplicationEvent::ProjectsListed {
                projects: self.projects.values().cloned().collect(),
            },
            ApplicationCommand::CreateSession {
                project_id,
                provider,
                model,
            } => {
                let session = self
                    .create_session(&project_id, provider, model)
                    .map_err(AppError::InvalidConfig)?;
                ApplicationEvent::SessionCreated(session)
            }
            ApplicationCommand::ListSessions { project_id } => {
                if !self.projects.contains_key(&project_id) {
                    return Err(AppError::InvalidConfig(format!(
                        "project not found: {project_id}"
                    )));
                }
                ApplicationEvent::SessionsListed {
                    project_id: project_id.clone(),
                    sessions: self.list_sessions(&project_id),
                }
            }
            ApplicationCommand::CancelRequest { request_id } => {
                if !self.cancel_request(request_id) {
                    return Err(AppError::InvalidConfig(format!(
                        "request not found: {request_id}"
                    )));
                }
                ApplicationEvent::RequestCancelled { request_id }
            }
            ApplicationCommand::ListModels => ApplicationEvent::ModelsListed {
                providers: self.providers.clone(),
            },
            ApplicationCommand::RefreshModels { .. } => {
                return Err(AppError::InvalidConfig(
                    "refresh_models must use the asynchronous service method".to_owned(),
                ));
            }
            ApplicationCommand::SendMessage { .. } => {
                return Err(AppError::InvalidConfig(
                    "send_message must use the asynchronous service method".to_owned(),
                ));
            }
        };

        Ok(ApplicationEnvelope {
            api_version: APPLICATION_API_VERSION,
            request_id,
            sequence: self.next_sequence(),
            payload: event,
        })
    }
}

/// Converts private provider configuration into the credential-free DTO used
/// by the desktop client.
fn public_providers(registry: &ProviderRegistry) -> Vec<ProviderDto> {
    registry
        .providers
        .iter()
        .map(|(name, provider)| ProviderDto {
            name: name.clone(),
            kind: provider.kind.clone(),
            models: provider.models.clone(),
            reachable: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        APPLICATION_API_VERSION, ApplicationCommand, ApplicationEvent, ApplicationService,
    };
    use crate::LaunchStateStore;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    #[test]
    fn command_serialization_is_stable_and_versioned() {
        let command = ApplicationCommand::OpenProject {
            path: PathBuf::from("/tmp/project"),
        };
        let json = serde_json::to_value(command).unwrap();

        assert_eq!(json["type"], "open_project");
        assert_eq!(json["payload"]["path"], "/tmp/project");
        assert_eq!(APPLICATION_API_VERSION, 1);
    }

    #[test]
    fn startup_command_serialization_is_additive() {
        let json = serde_json::to_value(ApplicationCommand::GetStartupState).unwrap();

        assert_eq!(json["type"], "get_startup_state");
        assert_eq!(APPLICATION_API_VERSION, 1);
    }

    #[test]
    fn service_scopes_sessions_to_their_project() {
        let mut service = ApplicationService::new();
        let first = service.open_project(PathBuf::from("/tmp/first"));
        let second = service.open_project(PathBuf::from("/tmp/second"));
        service
            .create_session(&first.id, "litellm".to_owned(), "model-a".to_owned())
            .unwrap();
        service
            .create_session(&second.id, "ollama".to_owned(), "model-b".to_owned())
            .unwrap();

        let sessions = service.list_sessions(&first.id);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].model, "model-a");
    }

    #[test]
    fn command_execution_returns_correlated_ordered_event() {
        let mut service = ApplicationService::new();
        let request_id = Uuid::new_v4();
        let event = service
            .execute(request_id, ApplicationCommand::GetCapabilities)
            .unwrap();

        assert_eq!(event.api_version, APPLICATION_API_VERSION);
        assert_eq!(event.request_id, request_id);
        assert_eq!(event.sequence, 1);
        assert!(matches!(event.payload, ApplicationEvent::Capabilities(_)));
    }

    #[test]
    fn opening_missing_project_is_rejected_before_registration() {
        let mut service = ApplicationService::new();
        let result = service.execute(
            Uuid::new_v4(),
            ApplicationCommand::OpenProject {
                path: PathBuf::from("/path/that/does/not/exist"),
            },
        );

        assert!(matches!(
            result,
            Err(crate::AppError::InvalidWorkingDirectory(_))
        ));
    }

    #[test]
    fn cancellation_is_cooperative_and_removed_when_request_finishes() {
        let mut service = ApplicationService::new();
        let request_id = Uuid::new_v4();
        let cancellation = service.register_request(request_id);

        assert!(!cancellation.is_cancelled());
        service.cancel_request(request_id);
        assert!(cancellation.is_cancelled());
        service.finish_request(request_id);
        assert!(!service.cancel_request(request_id));
    }

    #[test]
    fn persisted_project_is_available_after_service_reconstruction() {
        let root = std::env::temp_dir().join(format!("ai-agent-app-state-{}", Uuid::new_v4()));
        let project_path = root.join("project");
        fs::create_dir_all(&project_path).unwrap();
        let store = LaunchStateStore::new(root.join("user/state.json"));
        let mut service = ApplicationService::with_launch_state_store(store.clone()).unwrap();

        let opened = service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: project_path.clone(),
                },
            )
            .unwrap();
        let ApplicationEvent::ProjectOpened(opened_project) = opened.payload else {
            panic!("expected project_opened event");
        };

        let mut reconstructed = ApplicationService::with_launch_state_store(store).unwrap();
        let startup = reconstructed
            .execute(Uuid::new_v4(), ApplicationCommand::GetStartupState)
            .unwrap();
        let ApplicationEvent::StartupState(startup) = startup.payload else {
            panic!("expected startup_state event");
        };

        assert_eq!(startup.recent_projects.len(), 1);
        assert_eq!(startup.recent_projects[0].id, opened_project.id);
        assert_eq!(
            startup.recent_projects[0].path,
            project_path.canonicalize().unwrap()
        );
        assert!(startup.recent_projects[0].available);
        assert_eq!(startup.suggested_project_id, Some(opened_project.id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_state_marks_missing_project_unavailable() {
        let root = std::env::temp_dir().join(format!("ai-agent-stale-state-{}", Uuid::new_v4()));
        let project_path = root.join("project");
        fs::create_dir_all(&project_path).unwrap();
        let store = LaunchStateStore::new(root.join("user/state.json"));
        store
            .record_project_opened(&project_path, chrono::Utc::now())
            .unwrap();
        fs::remove_dir_all(&project_path).unwrap();

        let service = ApplicationService::with_launch_state_store(store).unwrap();
        let startup = service.startup_state();

        assert_eq!(startup.recent_projects.len(), 1);
        assert!(!startup.recent_projects[0].available);
        fs::remove_dir_all(root).unwrap();
    }
}
