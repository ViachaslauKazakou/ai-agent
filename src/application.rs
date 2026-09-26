//! Application-facing API for desktop and browser clients.
//!
//! This module deliberately contains transport-neutral data transfer objects
//! (DTOs), rather than Tauri, HTTP, or frontend-specific types.  Keeping the
//! contract here gives every client the same vocabulary and prevents a UI from
//! reaching into `Agent`, `Session`, or `ToolContext` internals.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}

#[cfg(test)]
mod tests {
    use super::{APPLICATION_API_VERSION, ApplicationCommand, ApplicationService};
    use std::path::PathBuf;

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
}
