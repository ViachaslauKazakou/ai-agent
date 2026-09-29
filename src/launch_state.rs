//! Persistent, user-scoped metadata used to bootstrap CLI and desktop clients.
//!
//! The launch-state document deliberately stores only project paths and opaque
//! identifiers. Provider credentials, connector tokens, prompts, tool payloads,
//! and session messages belong to their dedicated stores and must never be
//! added to this format.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppError;

/// Current on-disk schema version of [`LaunchState`].
pub const LAUNCH_STATE_SCHEMA_VERSION: u16 = 1;

/// Maximum number of recent projects retained in the user-scoped document.
const MAX_RECENT_PROJECTS: usize = 20;

/// Name of the agent directory created below the user's home directory.
const USER_STATE_DIRECTORY: &str = ".ai-agent";

/// Name of the launch-state document inside [`USER_STATE_DIRECTORY`].
const USER_STATE_FILE: &str = "state.json";

/// Versioned metadata required to suggest a project on the next launch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchState {
    /// Version used to validate or migrate this serialized document.
    pub schema_version: u16,
    /// Most recent time at which an adapter recorded application startup.
    pub last_started_at: Option<DateTime<Utc>>,
    /// Stable identifier of the most recently opened project.
    pub last_project_id: Option<String>,
    /// Reserved link to a persisted session once session indexing is added.
    pub last_session_id: Option<Uuid>,
    /// Projects ordered from most recently to least recently opened.
    pub projects: Vec<RecentProject>,
}

impl Default for LaunchState {
    fn default() -> Self {
        Self {
            schema_version: LAUNCH_STATE_SCHEMA_VERSION,
            last_started_at: None,
            last_project_id: None,
            last_session_id: None,
            projects: Vec::new(),
        }
    }
}

impl LaunchState {
    /// Records an adapter startup without changing project selection.
    pub fn record_launch(&mut self, started_at: DateTime<Utc>) {
        self.last_started_at = Some(started_at);
    }

    /// Inserts or promotes a canonical project path in the recent-project list.
    ///
    /// Existing entries keep their stable identifier. A newly discovered path
    /// receives a random opaque identifier that remains stable after reload.
    pub fn record_project(
        &mut self,
        canonical_path: PathBuf,
        opened_at: DateTime<Utc>,
    ) -> &RecentProject {
        let existing = self
            .projects
            .iter()
            .position(|project| project.path == canonical_path);

        let mut project = existing
            .map(|index| self.projects.remove(index))
            .unwrap_or_else(|| RecentProject {
                id: Uuid::new_v4().to_string(),
                path: canonical_path,
                last_opened_at: opened_at,
                last_session_id: None,
            });

        project.last_opened_at = opened_at;
        self.last_project_id = Some(project.id.clone());
        self.projects.insert(0, project);
        self.projects.truncate(MAX_RECENT_PROJECTS);
        &self.projects[0]
    }
}

/// Secret-free metadata describing a project known to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    /// Opaque identifier persisted independently from runtime map ordering.
    pub id: String,
    /// Absolute canonical path used to deduplicate project records.
    pub path: PathBuf,
    /// Most recent successful time at which the project was opened.
    pub last_opened_at: DateTime<Utc>,
    /// Reserved link to the project's most recently selected session.
    pub last_session_id: Option<Uuid>,
}

/// File-backed repository for user-scoped launch metadata.
///
/// Callers should inject [`LaunchStateStore::new`] in tests. Only CLI and
/// desktop composition roots should use [`LaunchStateStore::in_user_home`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchStateStore {
    path: PathBuf,
}

impl LaunchStateStore {
    /// Creates a repository for an explicit state file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Resolves the documented `~/.ai-agent/state.json` location.
    pub fn in_user_home() -> Result<Self, AppError> {
        let home = dirs::home_dir().ok_or_else(|| {
            AppError::LaunchState("не удалось определить домашнюю директорию".to_owned())
        })?;
        Ok(Self::new(
            home.join(USER_STATE_DIRECTORY).join(USER_STATE_FILE),
        ))
    }

    /// Returns the backing path for diagnostics and adapter documentation.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads and validates state, returning defaults when it does not exist.
    ///
    /// Malformed documents and unknown versions are returned as errors and are
    /// not overwritten. The adapter can then start with an in-memory default
    /// while preserving the original file for diagnostics or migration.
    pub fn load(&self) -> Result<LaunchState, AppError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(LaunchState::default()),
            Err(error) => return Err(self.error("не удалось прочитать", error)),
        };

        let state: LaunchState = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::LaunchState(format!(
                "не удалось разобрать {}: {error}",
                self.path.display()
            ))
        })?;
        if state.schema_version != LAUNCH_STATE_SCHEMA_VERSION {
            return Err(AppError::LaunchState(format!(
                "неподдерживаемая версия {} в {}; ожидается {}",
                state.schema_version,
                self.path.display(),
                LAUNCH_STATE_SCHEMA_VERSION
            )));
        }
        Ok(state)
    }

    /// Serializes state through a temporary sibling and atomically replaces it.
    pub fn save(&self, state: &LaunchState) -> Result<(), AppError> {
        if state.schema_version != LAUNCH_STATE_SCHEMA_VERSION {
            return Err(AppError::LaunchState(format!(
                "нельзя сохранить версию {}; ожидается {}",
                state.schema_version, LAUNCH_STATE_SCHEMA_VERSION
            )));
        }

        let parent = self.path.parent().ok_or_else(|| {
            AppError::LaunchState(format!(
                "у {} нет родительского каталога",
                self.path.display()
            ))
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| self.error("не удалось создать каталог", error))?;

        let bytes = serde_json::to_vec_pretty(state).map_err(|error| {
            AppError::LaunchState(format!("не удалось сериализовать состояние: {error}"))
        })?;
        let temporary = parent.join(format!(".state-{}.tmp", Uuid::new_v4()));

        fs::write(&temporary, bytes)
            .map_err(|error| self.error_for(&temporary, "не удалось записать", error))?;
        if let Err(error) = fs::rename(&temporary, &self.path) {
            // A failed replacement must not leave unbounded temporary files.
            let _ = fs::remove_file(&temporary);
            return Err(self.error("не удалось заменить", error));
        }
        Ok(())
    }

    /// Loads state, records startup, persists it, and returns the new snapshot.
    pub fn record_launch(&self, started_at: DateTime<Utc>) -> Result<LaunchState, AppError> {
        let mut state = self.load()?;
        state.record_launch(started_at);
        self.save(&state)?;
        Ok(state)
    }

    /// Records an existing project after canonicalizing its directory path.
    pub fn record_project_opened(
        &self,
        project_path: &Path,
        opened_at: DateTime<Utc>,
    ) -> Result<LaunchState, AppError> {
        let canonical_path = project_path.canonicalize().map_err(|error| {
            self.error_for(
                project_path,
                "проект недоступен или не является каталогом",
                error,
            )
        })?;
        if !canonical_path.is_dir() {
            return Err(AppError::LaunchState(format!(
                "проект не является каталогом: {}",
                canonical_path.display()
            )));
        }

        let mut state = self.load()?;
        state.record_project(canonical_path, opened_at);
        self.save(&state)?;
        Ok(state)
    }

    fn error(&self, action: &str, error: std::io::Error) -> AppError {
        self.error_for(&self.path, action, error)
    }

    fn error_for(&self, path: &Path, action: &str, error: std::io::Error) -> AppError {
        AppError::LaunchState(format!("{action} {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{TimeZone, Utc};

    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ai-agent-launch-state-{name}-{}", Uuid::new_v4()))
    }

    fn instant(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 29, 10, 0, second)
            .single()
            .unwrap()
    }

    #[test]
    fn missing_document_returns_versioned_default() {
        let store = LaunchStateStore::new(sandbox("missing").join("state.json"));

        let state = store.load().unwrap();

        assert_eq!(state, LaunchState::default());
    }

    #[test]
    fn save_creates_parent_and_round_trips_state() {
        let root = sandbox("round-trip");
        let store = LaunchStateStore::new(root.join("nested/state.json"));
        let mut state = LaunchState::default();
        state.record_launch(instant(1));

        store.save(&state).unwrap();

        assert_eq!(store.load().unwrap(), state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recording_same_project_preserves_id_and_promotes_it() {
        let first = sandbox("first-project");
        let second = sandbox("second-project");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();
        let mut state = LaunchState::default();

        let first_id = state.record_project(first.clone(), instant(1)).id.clone();
        state.record_project(second.clone(), instant(2));
        state.record_project(first.clone(), instant(3));

        assert_eq!(state.projects.len(), 2);
        assert_eq!(state.projects[0].path, first);
        assert_eq!(state.projects[0].id, first_id);
        assert_eq!(state.projects[0].last_opened_at, instant(3));
        assert_eq!(state.projects[1].path, second);
        assert_eq!(state.last_project_id.as_deref(), Some(first_id.as_str()));
        fs::remove_dir_all(&first).unwrap();
        fs::remove_dir_all(&second).unwrap();
    }

    #[test]
    fn malformed_document_is_preserved_and_reported() {
        let root = sandbox("malformed");
        let path = root.join("state.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, b"{broken").unwrap();
        let store = LaunchStateStore::new(&path);

        let error = store.load().unwrap_err();

        assert!(matches!(error, AppError::LaunchState(_)));
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let root = sandbox("future");
        let path = root.join("state.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &path,
            br#"{"schema_version":2,"last_started_at":null,"last_project_id":null,"last_session_id":null,"projects":[]}"#,
        )
        .unwrap();

        let error = LaunchStateStore::new(&path).load().unwrap_err();

        assert!(error.to_string().contains("неподдерживаемая версия 2"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_project_list_is_bounded() {
        let mut state = LaunchState::default();
        for index in 0..=MAX_RECENT_PROJECTS {
            state.record_project(PathBuf::from(format!("/project-{index}")), instant(1));
        }

        assert_eq!(state.projects.len(), MAX_RECENT_PROJECTS);
        assert_eq!(state.projects[0].path, PathBuf::from("/project-20"));
        assert_eq!(state.projects[19].path, PathBuf::from("/project-1"));
    }

    #[test]
    fn serialized_state_contains_metadata_only() {
        let mut state = LaunchState::default();
        state.record_project(PathBuf::from("/safe/project"), instant(1));

        let json = serde_json::to_string(&state).unwrap();

        assert!(!json.contains("api_key"));
        assert!(!json.contains("messages"));
        assert!(!json.contains("tool"));
        assert!(!json.contains("prompt"));
    }
}
