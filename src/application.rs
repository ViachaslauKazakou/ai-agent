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
    ProviderRegistry, RecentSession, Session,
    agents::AgentCatalog,
    cli::Cli,
    session_store::ProjectSessionStore,
    tools::{ToolContext, registry_from_names},
};

/// Version of the command/event contract exchanged with external clients.
///
/// The version is part of every envelope so a future desktop application can
/// reject an incompatible backend instead of silently misinterpreting a
/// command or event.
pub const APPLICATION_API_VERSION: u16 = 4;

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
    /// Latest session metadata, when the project has a restorable session.
    pub last_session: Option<SessionDto>,
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
    /// Project-local creation timestamp (legacy histories have no original timestamp).
    pub created_at: Option<chrono::DateTime<Utc>>,
    /// Most recent persisted checkpoint timestamp.
    pub updated_at: Option<chrono::DateTime<Utc>>,
    /// Bounded excerpt of the first user message, never stored in global state.
    pub title: Option<String>,
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
    /// Discover the latest project-local session, without reading its history into the UI.
    GetRestorableSession { project_id: String },
    /// Load a project session after validating its identity.
    RestoreSession {
        project_id: String,
        session_id: Uuid,
    },
    /// Permanently remove a selected project's indexed history.
    DeleteSession {
        project_id: String,
        session_id: Uuid,
    },
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
    /// A project-local legacy session is available (or no file exists).
    RestorableSession {
        project_id: String,
        session: Option<SessionDto>,
    },
    /// History was loaded into the backend; the UI only receives metadata.
    SessionRestored(SessionDto),
    /// An indexed session was removed from the project.
    SessionDeleted {
        project_id: String,
        session_id: Uuid,
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
    /// Validated histories awaiting their first agent request.
    restored_histories: BTreeMap<Uuid, Session>,
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

/// Desktop grants project-local file edits without the CLI's --allow-write flag.
/// A separate tool directory or a requested interactive confirmation cannot
/// safely inherit that grant from a non-interactive client.
fn desktop_tool_context(config: &Config, activity: Arc<Mutex<Option<String>>>) -> ToolContext {
    let allow_write = config.working_dir == config.project_dir && !config.confirm_writes;
    let mut context = ToolContext::new(&config.working_dir, allow_write);
    context.confirm_writes = config.confirm_writes;
    context.auto_approve_patch = allow_write;
    context.status = activity;
    context
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
                    last_session: project
                        .last_session
                        .as_ref()
                        .filter(|session| {
                            let store = ProjectSessionStore::new(&project.path);
                            let Ok(index) = store.load() else {
                                return false;
                            };
                            let indexed = index.sessions.iter().any(|entry| {
                                entry.id == session.id
                                    && entry.model == session.model
                                    && entry.provider == session.provider
                            });
                            if !indexed && index.migrated_legacy_id == Some(session.id) {
                                return false;
                            }
                            let path = if indexed {
                                store.history_path(session.id)
                            } else {
                                project.path.join(".aiagent/session.json")
                            };
                            Session::load_from(path).is_ok_and(|history| {
                                history.id() == session.id && history.model() == session.model
                            })
                        })
                        .map(|session| SessionDto {
                            id: session.id,
                            project_id: project.id.clone(),
                            provider: session.provider.clone(),
                            model: session.model.clone(),
                            message_count: 0,
                            created_at: None,
                            updated_at: None,
                            title: None,
                        }),
                })
                .collect(),
        }
    }

    /// Discovers the latest indexed or legacy session without exposing message bodies.
    pub fn restorable_session(&self, project_id: &str) -> Result<Option<SessionDto>, AppError> {
        let config = self
            .configs
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?;
        let store = ProjectSessionStore::new(&config.project_dir);
        let index = store.load()?;
        let last_id = self
            .launch_state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .and_then(|project| project.last_session_id);
        let legacy = legacy_session_path(config);
        if legacy.exists() {
            let history = Session::load_from(&legacy)?;
            if index.migrated_legacy_id != Some(history.id())
                && last_id == Some(history.id())
                && !index.sessions.iter().any(|entry| entry.id == history.id())
            {
                if index.sessions.len() == crate::session_store::MAX_PROJECT_SESSIONS {
                    return Err(AppError::SessionPersistence(
                        "legacy session cannot be migrated: project already has ten sessions"
                            .into(),
                    ));
                }
                let saved = self
                    .launch_state
                    .projects
                    .iter()
                    .find(|project| project.id == project_id)
                    .and_then(|project| project.last_session.as_ref())
                    .filter(|entry| entry.id == history.id());
                let provider =
                    saved.map_or(config.provider.as_str(), |entry| entry.provider.as_str());
                return self
                    .validate_history(project_id, config, &history, provider)
                    .map(Some);
            }
        }
        if let Some(entry) = last_id
            .and_then(|id| index.sessions.iter().find(|entry| entry.id == id))
            .or_else(|| index.sessions.iter().max_by_key(|entry| entry.updated_at))
        {
            return self
                .indexed_session(project_id, config, &store, entry)
                .map(Some);
        }
        if !legacy.exists() {
            return Ok(None);
        }
        let history = Session::load_from(legacy)?;
        if index.migrated_legacy_id == Some(history.id()) {
            return Ok(None);
        }
        if index.sessions.len() == crate::session_store::MAX_PROJECT_SESSIONS {
            return Err(AppError::SessionPersistence(
                "legacy session cannot be migrated: project already has ten sessions".into(),
            ));
        }
        let saved = self
            .launch_state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .and_then(|project| project.last_session.as_ref())
            .filter(|saved| saved.id == history.id());
        if saved.is_some_and(|saved| saved.model != history.model()) {
            return Err(AppError::SessionPersistence(
                "session model differs from recorded metadata".to_owned(),
            ));
        }
        let provider = saved.map_or(config.provider.as_str(), |saved| saved.provider.as_str());
        self.validate_history(project_id, config, &history, provider)
            .map(Some)
    }

    fn indexed_session(
        &self,
        project_id: &str,
        config: &Config,
        store: &ProjectSessionStore,
        entry: &crate::session_store::StoredSession,
    ) -> Result<SessionDto, AppError> {
        let history = Session::load_from(store.history_path(entry.id))?;
        if history.id() != entry.id || history.model() != entry.model {
            return Err(AppError::SessionPersistence(
                "session index differs from history".into(),
            ));
        }
        let mut session = self.validate_history(project_id, config, &history, &entry.provider)?;
        session.created_at = Some(entry.created_at);
        session.updated_at = Some(entry.updated_at);
        session.title = entry.title.clone();
        Ok(session)
    }

    /// Lists persisted sessions newest first; corrupt histories are reported instead of hidden.
    pub fn available_sessions(&self, project_id: &str) -> Result<Vec<SessionDto>, AppError> {
        let config = self
            .configs
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?;
        let store = ProjectSessionStore::new(&config.project_dir);
        let mut index = store.load()?;
        index
            .sessions
            .sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
        let mut sessions = index
            .sessions
            .iter()
            .map(|entry| self.indexed_session(project_id, config, &store, entry))
            .collect::<Result<Vec<_>, _>>()?;
        let legacy = legacy_session_path(config);
        if legacy.exists() {
            let history = Session::load_from(legacy)?;
            if index.migrated_legacy_id != Some(history.id())
                && !sessions.iter().any(|entry| entry.id == history.id())
            {
                if sessions.len() == crate::session_store::MAX_PROJECT_SESSIONS {
                    return Err(AppError::SessionPersistence(
                        "legacy session cannot be migrated: project already has ten sessions"
                            .into(),
                    ));
                }
                let saved = self
                    .launch_state
                    .projects
                    .iter()
                    .find(|project| project.id == project_id)
                    .and_then(|project| project.last_session.as_ref())
                    .filter(|entry| entry.id == history.id());
                let provider =
                    saved.map_or(config.provider.as_str(), |entry| entry.provider.as_str());
                sessions.push(self.validate_history(project_id, config, &history, provider)?);
            }
        }
        Ok(sessions)
    }

    /// Ensures legacy history is indexed before allowing a new session to evict old entries.
    fn prepare_legacy(&self, project_id: &str, config: &Config) -> Result<(), AppError> {
        let legacy = legacy_session_path(config);
        if !legacy.exists() {
            return Ok(());
        }
        let history = Session::load_from(&legacy)?;
        if ProjectSessionStore::new(&config.project_dir)
            .load()?
            .migrated_legacy_id
            == Some(history.id())
        {
            return Ok(());
        }
        let saved = self
            .launch_state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .and_then(|project| project.last_session.as_ref())
            .filter(|entry| entry.id == history.id());
        let provider = saved.map_or(config.provider.as_str(), |entry| entry.provider.as_str());
        self.validate_history(project_id, config, &history, provider)?;
        ProjectSessionStore::new(&config.project_dir).migrate_legacy(&legacy, provider)
    }

    fn validate_history(
        &self,
        project_id: &str,
        config: &Config,
        history: &Session,
        provider: &str,
    ) -> Result<SessionDto, AppError> {
        let canonical = history.working_dir().canonicalize().map_err(|_| {
            AppError::SessionPersistence("session working directory is unavailable".to_owned())
        })?;
        if canonical != config.working_dir {
            return Err(AppError::SessionPersistence(
                "session belongs to another working directory".to_owned(),
            ));
        }
        validate_selection(config, provider, history.model())?;
        if self
            .sessions
            .get(&history.id())
            .is_some_and(|existing| existing.project_id != project_id)
        {
            return Err(AppError::SessionPersistence(
                "session ID belongs to another project".to_owned(),
            ));
        }
        Ok(SessionDto {
            id: history.id(),
            project_id: project_id.to_owned(),
            provider: provider.to_owned(),
            model: history.model().to_owned(),
            message_count: history.messages().len(),
            created_at: None,
            updated_at: None,
            title: None,
        })
    }

    /// Activates an on-disk legacy history only for its opened project and ID.
    pub fn restore_session(
        &mut self,
        project_id: &str,
        session_id: Uuid,
    ) -> Result<SessionDto, AppError> {
        let config = self
            .configs
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?;
        let store = ProjectSessionStore::new(&config.project_dir);
        let index = store.load()?;
        if let Some(entry) = index.sessions.iter().find(|entry| entry.id == session_id) {
            let session = self.indexed_session(project_id, config, &store, entry)?;
            let history = Session::load_from(store.history_path(session_id))?;
            self.persist_active_session(&session)?;
            self.agents.remove(&session_id);
            self.restored_histories.insert(session_id, history);
            self.sessions.insert(session_id, session.clone());
            return Ok(session);
        }
        let history = Session::load_from(legacy_session_path(config))?;
        if index.migrated_legacy_id == Some(history.id()) {
            return Err(AppError::SessionPersistence(
                "session has been removed by retention policy".into(),
            ));
        }
        if history.id() != session_id {
            return Err(AppError::SessionPersistence(
                "session ID changed or is not in this project".into(),
            ));
        }
        let saved = self
            .launch_state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .and_then(|project| project.last_session.as_ref())
            .filter(|saved| saved.id == session_id);
        if saved.is_some_and(|saved| saved.model != history.model()) {
            return Err(AppError::SessionPersistence(
                "session model differs from recorded metadata".to_owned(),
            ));
        }
        let provider = saved.map_or(config.provider.as_str(), |saved| saved.provider.as_str());
        let session = self.validate_history(project_id, config, &history, provider)?;
        self.prepare_legacy(project_id, config)?;
        self.persist_active_session(&session)?;
        self.agents.remove(&session_id);
        self.restored_histories.insert(session_id, history);
        self.sessions.insert(session_id, session.clone());
        Ok(session)
    }

    /// Deletes only a validated, indexed history in the requested project.
    pub fn delete_session(&mut self, project_id: &str, session_id: Uuid) -> Result<(), AppError> {
        let config = self
            .configs
            .get(project_id)
            .ok_or_else(|| AppError::InvalidConfig(format!("project not found: {project_id}")))?;
        let store = ProjectSessionStore::new(&config.project_dir);
        let entry = store
            .load()?
            .sessions
            .into_iter()
            .find(|entry| entry.id == session_id)
            .ok_or_else(|| {
                AppError::SessionPersistence("session is not indexed in this project".into())
            })?;
        self.indexed_session(project_id, config, &store, &entry)?;
        store.delete(session_id)?;
        self.sessions.remove(&session_id);
        self.agents.remove(&session_id);
        self.restored_histories.remove(&session_id);
        let mut next = self.launch_state.clone();
        if let Some(project) = next
            .projects
            .iter_mut()
            .find(|project| project.id == project_id)
            .filter(|project| project.last_session_id == Some(session_id))
        {
            project.last_session_id = None;
            project.last_session = None;
            if next.last_project_id.as_deref() == Some(project_id) {
                next.last_session_id = None;
            }
        }
        if let Some(state) = &self.launch_state_store {
            state.save(&next)?;
        }
        self.launch_state = next;
        Ok(())
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
        self.load_project_config_with_working_dir(project, None)
    }

    /// Loads a project's configuration, optionally overriding its tool working directory.
    pub fn load_project_config_with_working_dir(
        &mut self,
        project: &ProjectDto,
        working_dir: Option<&std::path::Path>,
    ) -> Result<(), AppError> {
        let path = project.path.to_string_lossy().into_owned();
        let mut cli = Cli::try_parse_from(["ai-agent", "--project-dir", path.as_str()])
            .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
        cli.working_dir = working_dir.map(std::path::Path::to_path_buf);
        let config = Config::load(&cli)?;
        self.providers = public_providers(&config.providers);
        self.configs.insert(project.id.clone(), config);
        Ok(())
    }

    /// Opens a canonical project with an optional tool working directory override.
    pub fn open_project_with_working_dir(
        &mut self,
        request_id: Uuid,
        path: PathBuf,
        working_dir: Option<PathBuf>,
    ) -> Result<ApplicationEnvelope<ApplicationEvent>, AppError> {
        if !path.is_dir() {
            return Err(AppError::InvalidWorkingDirectory(
                path.display().to_string(),
            ));
        }
        let canonical = path.canonicalize().map_err(|error| {
            AppError::InvalidWorkingDirectory(format!("{}: {error}", path.display()))
        })?;
        let override_path = working_dir.as_deref();
        if let Some(project) = self
            .projects
            .values()
            .find(|project| project.path == canonical)
            .cloned()
        {
            let config = project_config(&project.path, override_path)?;
            self.providers = public_providers(&config.providers);
            self.configs.insert(project.id.clone(), config);
            return Ok(ApplicationEnvelope {
                api_version: APPLICATION_API_VERSION,
                request_id,
                sequence: self.next_sequence(),
                payload: ApplicationEvent::ProjectOpened(project),
            });
        }
        let mut next = self.launch_state.clone();
        let recent = next.record_project(canonical.clone(), Utc::now());
        let project = ProjectDto {
            id: recent.id.clone(),
            path: canonical,
        };
        let config = project_config(&project.path, working_dir.as_deref())?;
        if let Some(store) = &self.launch_state_store {
            store.save(&next)?;
        }
        self.providers = public_providers(&config.providers);
        self.configs.insert(project.id.clone(), config);
        self.launch_state = next;
        self.projects.insert(project.id.clone(), project.clone());
        Ok(ApplicationEnvelope {
            api_version: APPLICATION_API_VERSION,
            request_id,
            sequence: self.next_sequence(),
            payload: ApplicationEvent::ProjectOpened(project),
        })
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
        if let Some(config) = self.configs.get(project_id) {
            validate_selection(config, &provider, &model).map_err(|error| error.to_string())?;
            self.prepare_legacy(project_id, config)
                .map_err(|error| error.to_string())?;
        }
        let session = SessionDto {
            id: Uuid::new_v4(),
            project_id: project_id.to_owned(),
            provider,
            model,
            message_count: 0,
            created_at: None,
            updated_at: None,
            title: None,
        };
        if let Some(config) = self.configs.get(project_id) {
            let history = Session::new_with_id(session.id, &config.working_dir, &session.model)
                .map_err(|error| error.to_string())?;
            let store = ProjectSessionStore::new(&config.project_dir);
            store
                .create(&history, &session.provider)
                .map_err(|error| error.to_string())?;
            let retained = store.load().map_err(|error| error.to_string())?;
            let entry = retained
                .sessions
                .iter()
                .find(|item| item.id == session.id)
                .unwrap();
            let mut session = session;
            session.created_at = Some(entry.created_at);
            session.updated_at = Some(entry.updated_at);
            self.sessions.retain(|id, entry| {
                entry.project_id != project_id
                    || retained.sessions.iter().any(|item| item.id == *id)
            });
            self.agents.retain(|id, _| {
                retained.sessions.iter().any(|item| item.id == *id)
                    || self.sessions.contains_key(id)
            });
            self.restored_histories.retain(|id, _| {
                retained.sessions.iter().any(|item| item.id == *id)
                    || self.sessions.contains_key(id)
            });
            self.sessions.insert(session.id, session.clone());
            return Ok(session);
        }
        self.sessions.insert(session.id, session.clone());
        Ok(session)
    }

    /// Persists the selected session as the project's startup default.
    fn persist_active_session(&mut self, session: &SessionDto) -> Result<(), AppError> {
        let mut next_launch_state = self.launch_state.clone();
        next_launch_state.record_session(
            &session.project_id,
            RecentSession {
                id: session.id,
                provider: session.provider.clone(),
                model: session.model.clone(),
            },
        )?;
        if let Some(store) = &self.launch_state_store {
            store.save(&next_launch_state)?;
        }
        self.launch_state = next_launch_state;
        Ok(())
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
        let mut context = desktop_tool_context(&config, activity);
        context.command_allowlist = profile.command_allowlist.clone();
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
        let provider = desktop_provider(&config, &session.provider)?;
        let agent = crate::agent::Agent::new(provider, registry, context, profile.max_tool_rounds)
            .without_tool_round_limit()
            .with_system_prompt(catalog.system_prompt(profile)?)
            .with_loop_limits(
                Some(std::time::Duration::from_secs(config.max_loop_seconds)),
                config.max_diff_bytes,
            )
            .with_reasoning_effort(config.reasoning_effort.clone());
        // Session model is selected by the desktop model list. The existing
        // Agent API derives the request model from Session, so no provider
        // credential or model selection is duplicated in the UI adapter.
        let mut runtime_session = if let Some(history) = self.restored_histories.remove(&session.id)
        {
            history
        } else {
            Session::new_with_id(session.id, &config.working_dir, &session.model)?
        };
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
        let config = self.configs.get(&session_dto.project_id).ok_or_else(|| {
            AppError::InvalidConfig("project configuration is not loaded".to_owned())
        })?;
        let store = ProjectSessionStore::new(&config.project_dir);
        let path = store.history_path(session_id);
        let persisted = Session::load_from(&path)?;
        if persisted.id() != session_id
            || persisted.model() != session_dto.model
            || persisted.working_dir().canonicalize().ok().as_deref()
                != Some(config.working_dir.as_path())
        {
            return Err(AppError::SessionPersistence(
                "session history changed".into(),
            ));
        }
        if !store.load()?.sessions.iter().any(|entry| {
            entry.id == session_id
                && entry.provider == session_dto.provider
                && entry.model == session_dto.model
        }) {
            return Err(AppError::SessionPersistence(
                "session is no longer indexed".into(),
            ));
        }
        let runtime = self
            .agents
            .get_mut(&session_id)
            .ok_or_else(|| AppError::InvalidConfig("agent runtime not found".to_owned()))?;
        let response = runtime.agent.complete(&mut runtime.session, prompt).await;
        let message_count = runtime.session.messages().len();
        store.checkpoint(&runtime.session)?;
        if let Some(dto) = self.sessions.get_mut(&session_id) {
            dto.message_count = message_count;
            if let Some(entry) = store
                .load()?
                .sessions
                .into_iter()
                .find(|entry| entry.id == session_id)
            {
                dto.title = entry.title;
                dto.updated_at = Some(entry.updated_at);
            }
        }
        let response = response?;
        self.persist_active_session(&session_dto)?;
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
                return self.open_project_with_working_dir(request_id, path, None);
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
                self.persist_active_session(&session)?;
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
                    sessions: self.available_sessions(&project_id)?,
                }
            }
            ApplicationCommand::GetRestorableSession { project_id } => {
                let session = self.restorable_session(&project_id)?;
                ApplicationEvent::RestorableSession {
                    project_id,
                    session,
                }
            }
            ApplicationCommand::RestoreSession {
                project_id,
                session_id,
            } => ApplicationEvent::SessionRestored(self.restore_session(&project_id, session_id)?),
            ApplicationCommand::DeleteSession {
                project_id,
                session_id,
            } => {
                self.delete_session(&project_id, session_id)?;
                ApplicationEvent::SessionDeleted {
                    project_id,
                    session_id,
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

fn legacy_session_path(config: &Config) -> PathBuf {
    config.project_dir.join(".aiagent/session.json")
}

fn project_config(
    path: &std::path::Path,
    working_dir: Option<&std::path::Path>,
) -> Result<Config, AppError> {
    let path = path.to_string_lossy().into_owned();
    let mut cli = Cli::try_parse_from(["ai-agent", "--project-dir", path.as_str()])
        .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
    cli.working_dir = working_dir.map(std::path::Path::to_path_buf);
    Config::load(&cli)
}

fn validate_selection(config: &Config, provider: &str, model: &str) -> Result<(), AppError> {
    let entry = config.providers.provider(provider).ok_or_else(|| {
        AppError::InvalidConfig(format!("session provider is unavailable: {provider}"))
    })?;
    if model.trim().is_empty()
        || (!entry.models.is_empty() && !entry.models.iter().any(|name| name == model))
    {
        return Err(AppError::InvalidConfig(format!(
            "session model is unavailable: {model}"
        )));
    }
    Ok(())
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
        desktop_tool_context,
    };
    use crate::{LaunchStateStore, Message, Role, Session, tools::registry_from_names};
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    use uuid::Uuid;

    #[tokio::test]
    async fn desktop_default_can_edit_only_the_selected_project() {
        let root = std::env::temp_dir().join(format!("desktop-write-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(first.join(".git")).unwrap();
        fs::create_dir_all(second.join(".git")).unwrap();
        let mut service = ApplicationService::new();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: first.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!("expected project"),
        };
        let config = &service.configs[&project.id];
        assert!(!config.allow_write);
        let context = desktop_tool_context(config, Arc::new(Mutex::new(None)));
        assert!(context.allow_write);
        let catalog = crate::agents::AgentCatalog::load(&config.working_dir, config).unwrap();
        let registry =
            registry_from_names(&catalog.profile("default").unwrap().enabled_tools).unwrap();
        assert!(registry.names().contains(&"create_file".to_owned()));
        registry
            .execute(
                "create_file",
                json!({"path":"nested/main.rs","content":"old"}),
                &context,
            )
            .await
            .unwrap();
        registry
            .execute(
                "apply_patch",
                json!({"path":"nested/main.rs","old_text":"old","new_text":"new","apply":true}),
                &context,
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(first.join("nested/main.rs")).unwrap(),
            "new"
        );
        assert!(
            registry
                .execute(
                    "write_file",
                    json!({"path": second.join("file.rs").display().to_string(),"content":"bad"}),
                    &context
                )
                .await
                .is_err()
        );
        assert!(!second.join("file.rs").exists());

        service
            .open_project_with_working_dir(Uuid::new_v4(), first.clone(), Some(second.clone()))
            .unwrap();
        let external_config = &service.configs[&project.id];
        let external_context = desktop_tool_context(external_config, Arc::new(Mutex::new(None)));
        assert!(!external_context.allow_write);
        assert!(
            registry
                .execute(
                    "create_file",
                    json!({"path":"blocked.rs","content":"bad"}),
                    &external_context
                )
                .await
                .is_err()
        );
        assert!(!second.join("blocked.rs").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_confirmation_setting_disables_unconfirmed_writes() {
        let root = std::env::temp_dir().join(format!("desktop-confirm-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let mut service = ApplicationService::new();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject { path: root.clone() },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!("expected project"),
        };
        let mut config = service.configs[&project.id].clone();
        config.confirm_writes = true;
        assert!(!desktop_tool_context(&config, Arc::new(Mutex::new(None))).allow_write);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_serialization_is_stable_and_versioned() {
        let command = ApplicationCommand::OpenProject {
            path: PathBuf::from("/tmp/project"),
        };
        let json = serde_json::to_value(command).unwrap();

        assert_eq!(json["type"], "open_project");
        assert_eq!(json["payload"]["path"], "/tmp/project");
        assert_eq!(APPLICATION_API_VERSION, 4);
    }

    #[test]
    fn startup_command_serialization_is_additive() {
        let json = serde_json::to_value(ApplicationCommand::GetStartupState).unwrap();

        assert_eq!(json["type"], "get_startup_state");
        assert_eq!(APPLICATION_API_VERSION, 4);
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

    #[test]
    fn latest_session_and_model_are_restored_when_project_reopens() {
        let root = std::env::temp_dir().join(format!("ai-agent-session-state-{}", Uuid::new_v4()));
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
        let ApplicationEvent::ProjectOpened(project) = opened.payload else {
            panic!("expected project_opened event");
        };
        let created = service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::CreateSession {
                    project_id: project.id.clone(),
                    provider: "litellm".to_owned(),
                    model: "restored-model".to_owned(),
                },
            )
            .unwrap();
        let ApplicationEvent::SessionCreated(created) = created.payload else {
            panic!("expected session_created event");
        };
        let mut history = Session::new_with_id(created.id, &project_path, &created.model).unwrap();
        history.add_message(Message::new(Role::User, "private prompt").unwrap());
        history.add_message(Message::new(Role::Assistant, "private response").unwrap());
        history
            .save_to(project_path.join(format!(".aiagent/sessions/{}.json", created.id)))
            .unwrap();

        let mut reconstructed = ApplicationService::with_launch_state_store(store).unwrap();
        reconstructed
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: project_path.clone(),
                },
            )
            .unwrap();
        assert!(reconstructed.list_sessions(&project.id).is_empty());
        let discovery_id = Uuid::new_v4();
        let discovered = reconstructed
            .execute(
                discovery_id,
                ApplicationCommand::GetRestorableSession {
                    project_id: project.id.clone(),
                },
            )
            .unwrap();
        assert_eq!(discovered.request_id, discovery_id);
        let ApplicationEvent::RestorableSession {
            session: Some(candidate),
            ..
        } = discovered.payload
        else {
            panic!("expected candidate")
        };
        assert_eq!(candidate.id, created.id);
        assert_eq!(candidate.provider, "litellm");
        assert_eq!(candidate.message_count, 2);
        assert!(
            !serde_json::to_string(&candidate)
                .unwrap()
                .contains("private prompt")
        );
        let restore_id = Uuid::new_v4();
        let restored = reconstructed
            .execute(
                restore_id,
                ApplicationCommand::RestoreSession {
                    project_id: project.id.clone(),
                    session_id: candidate.id,
                },
            )
            .unwrap();
        assert_eq!(restored.request_id, restore_id);
        assert!(restored.sequence > discovered.sequence);
        assert_eq!(
            restored.payload,
            ApplicationEvent::SessionRestored(candidate.clone())
        );
        assert_eq!(reconstructed.list_sessions(&project.id), vec![candidate]);
        assert_eq!(
            reconstructed.restored_histories.get(&created.id).unwrap(),
            &history
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_discovery_handles_missing_corrupt_foreign_and_stale_files() {
        let root = std::env::temp_dir().join(format!("ai-agent-session-errors-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let mut service = ApplicationService::new();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: first.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!("expected project"),
        };
        assert_eq!(service.restorable_session(&project.id).unwrap(), None);
        let path = first.join(".aiagent/session.json");
        fs::write(&path, "{broken").unwrap();
        assert!(service.restorable_session(&project.id).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
        let foreign = Session::new(&second, "demo-model").unwrap();
        foreign.save_to(&path).unwrap();
        assert!(
            service
                .restorable_session(&project.id)
                .unwrap_err()
                .to_string()
                .contains("another working directory")
        );
        let local = Session::new(&first, "demo-model").unwrap();
        local.save_to(&path).unwrap();
        assert!(
            service
                .restore_session(&project.id, Uuid::new_v4())
                .is_err()
        );
        assert!(service.list_sessions(&project.id).is_empty());
        assert!(
            service
                .create_session(&project.id, "litellm".into(), "demo-model".into())
                .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_and_working_directory_are_validated_for_restoration() {
        let root = std::env::temp_dir().join(format!("ai-agent-session-config-{}", Uuid::new_v4()));
        let project_path = root.join("project");
        let tools_path = root.join("tools");
        fs::create_dir_all(&project_path).unwrap();
        fs::create_dir_all(&tools_path).unwrap();
        let mut service = ApplicationService::new();
        let opened = service
            .open_project_with_working_dir(
                Uuid::new_v4(),
                project_path.clone(),
                Some(tools_path.clone()),
            )
            .unwrap();
        let ApplicationEvent::ProjectOpened(project) = opened.payload else {
            panic!("expected project")
        };
        assert_eq!(
            service.configs[&project.id].project_dir,
            project_path.canonicalize().unwrap()
        );
        assert_eq!(
            service.configs[&project.id].working_dir,
            tools_path.canonicalize().unwrap()
        );
        let history = Session::new(&tools_path, "demo-model").unwrap();
        history
            .save_to(project_path.join(".aiagent/session.json"))
            .unwrap();
        assert_eq!(
            service.restorable_session(&project.id).unwrap().unwrap().id,
            history.id()
        );
        let providers_path = project_path.join(".aiagent/providers.json");
        let mut providers: crate::ProviderRegistry =
            serde_json::from_slice(&fs::read(&providers_path).unwrap()).unwrap();
        providers.providers.get_mut("litellm").unwrap().models = vec!["different-model".into()];
        fs::write(&providers_path, serde_json::to_vec(&providers).unwrap()).unwrap();
        service
            .open_project_with_working_dir(
                Uuid::new_v4(),
                project_path.clone(),
                Some(tools_path.clone()),
            )
            .unwrap();
        assert!(
            service
                .restorable_session(&project.id)
                .unwrap_err()
                .to_string()
                .contains("model is unavailable")
        );
        providers.providers.remove("litellm");
        fs::write(&providers_path, serde_json::to_vec(&providers).unwrap()).unwrap();
        // A project with an unavailable default provider cannot load configuration.
        assert!(
            service
                .open_project_with_working_dir(Uuid::new_v4(), project_path, Some(tools_path))
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restoration_commands_round_trip_and_reject_cross_project_ids() {
        let root = std::env::temp_dir().join(format!("ai-agent-isolation-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let mut service = ApplicationService::new();
        let first_project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: first.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let second_project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: second.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let history = Session::new(&first, "demo-model").unwrap();
        history
            .save_to(first.join(".aiagent/session.json"))
            .unwrap();
        let command = ApplicationCommand::RestoreSession {
            project_id: second_project.id.clone(),
            session_id: history.id(),
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["type"], "restore_session");
        assert_eq!(
            serde_json::from_value::<ApplicationCommand>(json).unwrap(),
            command
        );
        assert!(service.execute(Uuid::new_v4(), command).is_err());
        assert!(service.list_sessions(&second_project.id).is_empty());
        let discovered = service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::GetRestorableSession {
                    project_id: first_project.id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&discovered).unwrap()["payload"]["type"],
            "restorable_session"
        );
        assert_eq!(discovered.api_version, APPLICATION_API_VERSION);
        let restored = service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::RestoreSession {
                    project_id: first_project.id.clone(),
                    session_id: history.id(),
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap()["payload"]["type"],
            "session_restored"
        );
        assert_eq!(restored.sequence, discovered.sequence + 1);
        assert!(service.list_sessions(&second_project.id).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_recorded_provider_cannot_restore_history() {
        let root =
            std::env::temp_dir().join(format!("ai-agent-provider-history-{}", Uuid::new_v4()));
        let project_path = root.join("project");
        fs::create_dir_all(&project_path).unwrap();
        let store = LaunchStateStore::new(root.join("user/state.json"));
        let mut service = ApplicationService::with_launch_state_store(store.clone()).unwrap();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: project_path.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let created = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::CreateSession {
                    project_id: project.id.clone(),
                    provider: "ollama".into(),
                    model: "demo-model".into(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::SessionCreated(session) => session,
            _ => panic!(),
        };
        Session::new_with_id(created.id, &project_path, &created.model)
            .unwrap()
            .save_to(project_path.join(".aiagent/session.json"))
            .unwrap();
        let providers_path = project_path.join(".aiagent/providers.json");
        let mut providers: crate::ProviderRegistry =
            serde_json::from_slice(&fs::read(&providers_path).unwrap()).unwrap();
        providers.providers.remove("ollama");
        fs::write(&providers_path, serde_json::to_vec(&providers).unwrap()).unwrap();
        let mut reopened = ApplicationService::with_launch_state_store(store).unwrap();
        reopened
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject { path: project_path },
            )
            .unwrap();
        assert!(
            reopened
                .restorable_session(&project.id)
                .unwrap_err()
                .to_string()
                .contains("provider is unavailable")
        );
        assert!(reopened.restore_session(&project.id, created.id).is_err());
        assert!(reopened.list_sessions(&project.id).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multiple_sessions_persist_and_can_switch_after_restart() {
        let root = std::env::temp_dir().join(format!("desktop-switch-{}", Uuid::new_v4()));
        let project_path = root.join("project");
        fs::create_dir_all(&project_path).unwrap();
        let state = LaunchStateStore::new(root.join("user/state.json"));
        let mut service = ApplicationService::with_launch_state_store(state.clone()).unwrap();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: project_path.clone(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let first = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::CreateSession {
                    project_id: project.id.clone(),
                    provider: "litellm".into(),
                    model: "first".into(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::SessionCreated(session) => session,
            _ => panic!(),
        };
        let second = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::CreateSession {
                    project_id: project.id.clone(),
                    provider: "ollama".into(),
                    model: "second".into(),
                },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::SessionCreated(session) => session,
            _ => panic!(),
        };
        let mut history =
            Session::load_from(project_path.join(format!(".aiagent/sessions/{}.json", first.id)))
                .unwrap();
        history.add_message(Message::new(Role::User, "private content").unwrap());
        crate::session_store::ProjectSessionStore::new(&project_path)
            .checkpoint(&history)
            .unwrap();
        let mut restarted = ApplicationService::with_launch_state_store(state).unwrap();
        restarted
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject {
                    path: project_path.clone(),
                },
            )
            .unwrap();
        let listed = restarted
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::ListSessions {
                    project_id: project.id.clone(),
                },
            )
            .unwrap();
        let ApplicationEvent::SessionsListed { sessions, .. } = listed.payload else {
            panic!()
        };
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, first.id);
        assert_eq!(sessions[0].message_count, 1);
        assert_eq!(sessions[0].title.as_deref(), Some("private content"));
        assert!(
            !serde_json::to_string(&restarted.startup_state())
                .unwrap()
                .contains("private content")
        );
        assert_eq!(
            restarted
                .restore_session(&project.id, second.id)
                .unwrap()
                .provider,
            "ollama"
        );
        assert_eq!(
            restarted
                .restore_session(&project.id, first.id)
                .unwrap()
                .message_count,
            1
        );
        assert_eq!(
            restarted
                .restorable_session(&project.id)
                .unwrap()
                .unwrap()
                .id,
            first.id
        );
        assert!(
            restarted
                .restore_session(&project.id, Uuid::new_v4())
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evicted_active_session_cannot_send_or_restore_even_with_legacy_copy() {
        let root = std::env::temp_dir().join(format!("desktop-eviction-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let mut service = ApplicationService::new();
        let project = match service
            .execute(
                Uuid::new_v4(),
                ApplicationCommand::OpenProject { path: root.clone() },
            )
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let legacy = Session::new(&root, "demo-model").unwrap();
        legacy.save_to(root.join(".aiagent/session.json")).unwrap();
        service.restore_session(&project.id, legacy.id()).unwrap();
        for number in 0..crate::session_store::MAX_PROJECT_SESSIONS {
            service
                .create_session(&project.id, "litellm".into(), format!("model-{number}"))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!service.sessions.contains_key(&legacy.id()));
        assert!(!service.agents.contains_key(&legacy.id()));
        assert!(!service.restored_histories.contains_key(&legacy.id()));
        assert!(service.restore_session(&project.id, legacy.id()).is_err());
        assert!(
            service
                .available_sessions(&project.id)
                .unwrap()
                .iter()
                .all(|s| s.id != legacy.id())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_session_clears_active_selection_and_preserves_legacy_and_other_project() {
        let root = std::env::temp_dir().join(format!("desktop-delete-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let state = LaunchStateStore::new(root.join("user/state.json"));
        let mut service = ApplicationService::with_launch_state_store(state.clone()).unwrap();
        let open = |service: &mut ApplicationService, path| match service
            .execute(Uuid::new_v4(), ApplicationCommand::OpenProject { path })
            .unwrap()
            .payload
        {
            ApplicationEvent::ProjectOpened(project) => project,
            _ => panic!(),
        };
        let project = open(&mut service, first.clone());
        let other = open(&mut service, second.clone());
        let legacy = Session::new(&first, "demo-model").unwrap();
        legacy.save_to(first.join(".aiagent/session.json")).unwrap();
        let selected = service.restore_session(&project.id, legacy.id()).unwrap();
        assert_eq!(selected.id, legacy.id());
        let foreign = service
            .create_session(&other.id, "litellm".into(), "demo-model".into())
            .unwrap();
        assert!(service.delete_session(&other.id, legacy.id()).is_err());
        let command = ApplicationCommand::DeleteSession {
            project_id: project.id.clone(),
            session_id: legacy.id(),
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["type"], "delete_session");
        assert_eq!(
            serde_json::from_value::<ApplicationCommand>(json).unwrap(),
            command
        );
        let event = service.execute(Uuid::new_v4(), command).unwrap();
        assert!(
            matches!(event.payload, ApplicationEvent::SessionDeleted { session_id, .. } if session_id == legacy.id())
        );
        assert!(service.list_sessions(&project.id).is_empty());
        assert!(service.available_sessions(&project.id).unwrap().is_empty());
        assert!(service.restorable_session(&project.id).unwrap().is_none());
        assert!(service.restore_session(&project.id, legacy.id()).is_err());
        assert!(service.delete_session(&project.id, legacy.id()).is_err());
        assert_eq!(
            service.available_sessions(&other.id).unwrap()[0].id,
            foreign.id
        );
        assert!(first.join(".aiagent/session.json").exists());
        let reopened = ApplicationService::with_launch_state_store(state).unwrap();
        assert!(
            reopened
                .startup_state()
                .recent_projects
                .iter()
                .find(|item| item.id == project.id)
                .unwrap()
                .last_session
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
