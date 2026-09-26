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
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppError;

/// Version of the command/event contract exchanged with external clients.
///
/// The version is part of every envelope so a future desktop application can
/// reject an incompatible backend instead of silently misinterpreting a
/// command or event.
pub const APPLICATION_API_VERSION: u16 = 1;

/// Small, secret-free description of a project opened by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDto {
    /// Stable identifier assigned by the service for the current process.
    pub id: String,
    /// Canonical project path displayed by the client.
    pub path: PathBuf,
}

/// Secret-free representation of a session available to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDto {
    /// Session UUID used in subsequent commands.
    pub id: Uuid,
    /// Project to which the session belongs.
    pub project_id: String,
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
    /// Register a project path for later session commands.
    OpenProject { path: PathBuf },
    /// Create a new empty session for an opened project.
    CreateSession { project_id: String, model: String },
    /// Return sessions currently known to the service.
    ListSessions { project_id: String },
    /// Request cancellation of an in-flight operation.
    CancelRequest { request_id: Uuid },
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
    /// Project was accepted and is ready for session creation.
    ProjectOpened(ProjectDto),
    /// A new session became available.
    SessionCreated(SessionDto),
    /// Current sessions for a project.
    SessionsListed {
        project_id: String,
        sessions: Vec<SessionDto>,
    },
    /// Confirms that cancellation was requested for an operation.
    RequestCancelled { request_id: Uuid },
    /// Stable error event suitable for rendering in a client.
    Error { code: String, message: String },
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
#[derive(Debug, Default)]
pub struct ApplicationService {
    projects: BTreeMap<String, ProjectDto>,
    sessions: BTreeMap<Uuid, SessionDto>,
    next_sequence: u64,
    cancellations: BTreeMap<Uuid, RequestCancellation>,
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

    /// Returns the capabilities advertised by this service instance.
    pub fn capabilities(&self) -> ApplicationCapabilities {
        ApplicationCapabilities {
            api_version: APPLICATION_API_VERSION,
            streaming: false,
            cancellation: false,
            confirmations: true,
        }
    }

    /// Opens a project path and returns a process-local identifier.
    ///
    /// Canonicalization and full configuration loading remain in the existing
    /// config layer.  This method only establishes the application boundary;
    /// transport adapters must pass the validated path they received from that
    /// layer rather than duplicating path policy here.
    pub fn open_project(&mut self, path: PathBuf) -> ProjectDto {
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
        model: String,
    ) -> Result<SessionDto, String> {
        if !self.projects.contains_key(project_id) {
            return Err(format!("project not found: {project_id}"));
        }
        if model.trim().is_empty() {
            return Err("model must not be empty".to_owned());
        }
        let session = SessionDto {
            id: Uuid::new_v4(),
            project_id: project_id.to_owned(),
            model,
            message_count: 0,
        };
        self.sessions.insert(session.id, session.clone());
        Ok(session)
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
            ApplicationCommand::OpenProject { path } => {
                if !path.is_dir() {
                    return Err(AppError::InvalidWorkingDirectory(
                        path.display().to_string(),
                    ));
                }
                ApplicationEvent::ProjectOpened(self.open_project(path))
            }
            ApplicationCommand::CreateSession { project_id, model } => {
                let session = self
                    .create_session(&project_id, model)
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
        };

        Ok(ApplicationEnvelope {
            api_version: APPLICATION_API_VERSION,
            request_id,
            sequence: self.next_sequence(),
            payload: event,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        APPLICATION_API_VERSION, ApplicationCommand, ApplicationEvent, ApplicationService,
    };
    use std::path::PathBuf;
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
    fn service_scopes_sessions_to_their_project() {
        let mut service = ApplicationService::new();
        let first = service.open_project(PathBuf::from("/tmp/first"));
        let second = service.open_project(PathBuf::from("/tmp/second"));
        service
            .create_session(&first.id, "model-a".to_owned())
            .unwrap();
        service
            .create_session(&second.id, "model-b".to_owned())
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
}
