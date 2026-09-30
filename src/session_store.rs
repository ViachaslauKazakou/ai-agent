//! Project-local, bounded desktop session histories; launch state contains metadata only.

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppError, Role, Session};

pub const MAX_PROJECT_SESSIONS: usize = 10;
const SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredSession {
    pub id: Uuid,
    pub provider: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Canonical owner; absent only in indexes created before stage 6.
    #[serde(default)]
    pub project_path: Option<PathBuf>,
    /// Short excerpt of the first user prompt, stored only inside the project.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionIndex {
    schema_version: u16,
    pub sessions: Vec<StoredSession>,
    /// Legacy UUID already copied once; prevents resurrecting it after eviction.
    #[serde(default)]
    pub migrated_legacy_id: Option<Uuid>,
}

impl Default for SessionIndex {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sessions: Vec::new(),
            migrated_legacy_id: None,
        }
    }
}

/// Manages histories for one canonical project root.
pub struct ProjectSessionStore {
    root: PathBuf,
    project_dir: PathBuf,
}

impl ProjectSessionStore {
    pub fn new(project_dir: &Path) -> Self {
        Self {
            root: project_dir.join(".aiagent/sessions"),
            project_dir: project_dir
                .canonicalize()
                .unwrap_or_else(|_| project_dir.to_path_buf()),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    pub fn history_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    pub fn load(&self) -> Result<SessionIndex, AppError> {
        let data = match fs::read(self.index_path()) {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(SessionIndex::default()),
            Err(error) => return Err(failure(error)),
        };
        let index: SessionIndex = serde_json::from_slice(&data).map_err(failure)?;
        if index.schema_version != SCHEMA_VERSION || index.sessions.len() > MAX_PROJECT_SESSIONS {
            return Err(AppError::SessionPersistence(
                "unsupported or oversized session index".into(),
            ));
        }
        let mut ids = HashSet::new();
        if index.sessions.iter().any(|entry| !ids.insert(entry.id)) {
            return Err(AppError::SessionPersistence(
                "duplicate session ID in index".into(),
            ));
        }
        if index.sessions.iter().any(|entry| {
            entry
                .project_path
                .as_ref()
                .is_some_and(|path| path != &self.project_dir)
                || entry.created_at > entry.updated_at
                || entry.title.as_ref().is_some_and(|title| title.len() > 160)
        }) {
            return Err(failure(
                "invalid session index metadata or project identity",
            ));
        }
        Ok(index)
    }

    fn save(&self, index: &SessionIndex) -> Result<(), AppError> {
        fs::create_dir_all(&self.root).map_err(failure)?;
        let path = self.index_path();
        let temporary = self.root.join(format!(".index-{}.tmp", Uuid::new_v4()));
        let data = serde_json::to_vec_pretty(index).map_err(failure)?;
        fs::write(&temporary, data).map_err(failure)?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(temporary);
            return Err(failure(error));
        }
        Ok(())
    }

    /// Copies a valid legacy history before publishing its index entry. The original
    /// remains untouched for the CLI; a failed migration is safe to retry.
    pub fn migrate_legacy(&self, legacy: &Path, provider: &str) -> Result<(), AppError> {
        if !legacy.exists() {
            return Ok(());
        }
        let history = Session::load_from(legacy)?;
        let mut index = self.load()?;
        if index.migrated_legacy_id == Some(history.id()) {
            return Ok(());
        }
        if let Some(entry) = index.sessions.iter().find(|entry| entry.id == history.id()) {
            let indexed = Session::load_from(self.history_path(history.id()))?;
            if indexed.id() != history.id() || indexed.model() != entry.model {
                return Err(failure("migrated history ID differs"));
            }
            index.migrated_legacy_id = Some(history.id());
            return self.save(&index);
        }
        if index.sessions.len() >= MAX_PROJECT_SESSIONS {
            return Err(AppError::SessionPersistence(
                "legacy session cannot be migrated: project already has ten sessions".into(),
            ));
        }
        let path = self.history_path(history.id());
        if path.exists() {
            let existing = Session::load_from(&path)?;
            if existing != history {
                return Err(AppError::SessionPersistence(
                    "unindexed session file blocks migration".into(),
                ));
            }
        } else {
            fs::create_dir_all(&self.root).map_err(failure)?;
            history.save_to(&path)?;
        }
        let now = Utc::now();
        index.sessions.push(StoredSession {
            id: history.id(),
            provider: provider.into(),
            model: history.model().into(),
            created_at: now,
            updated_at: now,
            project_path: Some(self.project_dir.clone()),
            title: session_title(&history),
        });
        index.migrated_legacy_id = Some(history.id());
        self.save(&index)
    }

    /// Adds a new history; after publishing the new index, prunes the oldest.
    pub fn create(&self, history: &Session, provider: &str) -> Result<(), AppError> {
        let mut index = self.load()?;
        let path = self.history_path(history.id());
        if path.exists() || index.sessions.iter().any(|entry| entry.id == history.id()) {
            return Err(AppError::SessionPersistence(
                "session ID already exists".into(),
            ));
        }
        fs::create_dir_all(&self.root).map_err(failure)?;
        let displaced = if index.sessions.len() == MAX_PROJECT_SESSIONS {
            let oldest = index
                .sessions
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| (entry.updated_at, entry.created_at))
                .map(|(position, _)| position)
                .unwrap();
            let old = index.sessions.remove(oldest);
            let original = self.history_path(old.id);
            Session::load_from(&original)?;
            Some(original)
        } else {
            None
        };
        let now = Utc::now();
        index.sessions.push(StoredSession {
            id: history.id(),
            provider: provider.into(),
            model: history.model().into(),
            created_at: now,
            updated_at: now,
            project_path: Some(self.project_dir.clone()),
            title: session_title(history),
        });
        let result = history.save_to(&path).and_then(|_| self.save(&index));
        if result.is_err() {
            let _ = fs::remove_file(&path);
        } else if let Some(original) = displaced {
            // The new index is already committed; cleanup cannot roll it back.
            let _ = fs::remove_file(original);
        }
        result
    }

    /// Records a successful history checkpoint without changing its identity.
    pub fn checkpoint(&self, history: &Session) -> Result<(), AppError> {
        let mut index = self.load()?;
        let entry = index
            .sessions
            .iter_mut()
            .find(|entry| entry.id == history.id())
            .ok_or_else(|| AppError::SessionPersistence("session is not indexed".into()))?;
        let existing = Session::load_from(self.history_path(history.id()))?;
        if existing.id() != history.id() || existing.model() != entry.model {
            return Err(AppError::SessionPersistence(
                "session file ID differs from index".into(),
            ));
        }
        if existing.working_dir() != history.working_dir() {
            return Err(failure("session working directory changed"));
        }
        entry.updated_at = Utc::now();
        entry.project_path = Some(self.project_dir.clone());
        entry.title = session_title(history);
        history.save_to(self.history_path(history.id()))?;
        self.save(&index)
    }

    /// Removes an indexed history from discovery before deleting its file.
    /// An interrupted file cleanup leaves only an unindexed orphan.
    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let mut index = self.load()?;
        let position = index
            .sessions
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| failure("session is not indexed or has already been deleted"))?;
        let entry = &index.sessions[position];
        let path = self.history_path(id);
        let history = Session::load_from(&path)?;
        if history.id() != id || history.model() != entry.model {
            return Err(failure("session index differs from history"));
        }
        index.sessions.remove(position);
        self.save(&index)?;
        // The index is authoritative; do not roll it back if cleanup fails.
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(failure(format!(
                "session removed from index; orphan cleanup failed: {error}"
            ))),
        }
    }
}

fn session_title(history: &Session) -> Option<String> {
    history
        .messages()
        .iter()
        .find(|message| message.role() == Role::User)
        .map(|message| {
            message
                .content()
                .split_whitespace()
                .flat_map(|part| part.chars().chain(std::iter::once(' ')))
                .take(80)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .filter(|text| !text.is_empty())
}

fn failure(error: impl std::fmt::Display) -> AppError {
    AppError::SessionPersistence(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> PathBuf {
        let root = std::env::temp_dir().join(format!("desktop-sessions-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn creation_retains_only_ten_and_preserves_other_projects() {
        let root = sandbox();
        let other = sandbox();
        let store = ProjectSessionStore::new(&root);
        let other_store = ProjectSessionStore::new(&other);
        let foreign = Session::new(&other, "foreign").unwrap();
        other_store.create(&foreign, "litellm").unwrap();
        let mut ids = Vec::new();
        for number in 0..11 {
            let session = Session::new(&root, format!("model-{number}")).unwrap();
            ids.push(session.id());
            store.create(&session, "litellm").unwrap();
            // ensure timestamps differ, including on coarse file systems
            if number < 10 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let index = store.load().unwrap();
        assert_eq!(index.sessions.len(), MAX_PROJECT_SESSIONS);
        assert!(!index.sessions.iter().any(|entry| entry.id == ids[0]));
        assert!(!store.history_path(ids[0]).exists());
        for id in &ids[1..] {
            assert_eq!(
                Session::load_from(store.history_path(*id)).unwrap().id(),
                *id
            );
        }
        assert_eq!(other_store.load().unwrap().sessions.len(), 1);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn migrates_legacy_idempotently_without_deleting_original() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let legacy = root.join("session.json");
        let session = Session::new(&root, "legacy").unwrap();
        session.save_to(&legacy).unwrap();
        store.migrate_legacy(&legacy, "ollama").unwrap();
        store.migrate_legacy(&legacy, "ollama").unwrap();
        assert_eq!(store.load().unwrap().sessions.len(), 1);
        assert_eq!(store.load().unwrap().sessions[0].provider, "ollama");
        assert_eq!(Session::load_from(&legacy).unwrap().id(), session.id());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn damaged_index_and_missing_history_never_trigger_eviction() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let first = Session::new(&root, "first").unwrap();
        store.create(&first, "litellm").unwrap();
        fs::remove_file(store.history_path(first.id())).unwrap();
        assert!(store.checkpoint(&first).is_err());
        let index_path = store.index_path();
        fs::write(&index_path, "{broken").unwrap();
        assert!(
            store
                .create(&Session::new(&root, "second").unwrap(), "ollama")
                .is_err()
        );
        assert_eq!(fs::read_to_string(index_path).unwrap(), "{broken");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evicted_legacy_is_not_imported_again() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let legacy = root.join("session.json");
        let first = Session::new(&root, "original").unwrap();
        first.save_to(&legacy).unwrap();
        store.migrate_legacy(&legacy, "litellm").unwrap();
        for number in 0..MAX_PROJECT_SESSIONS {
            let session = Session::new(&root, format!("new-{number}")).unwrap();
            store.create(&session, "ollama").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(store.load().unwrap().sessions.len(), MAX_PROJECT_SESSIONS);
        assert!(!store.history_path(first.id()).exists());
        store.migrate_legacy(&legacy, "litellm").unwrap();
        assert_eq!(store.load().unwrap().migrated_legacy_id, Some(first.id()));
        assert_eq!(store.load().unwrap().sessions.len(), MAX_PROJECT_SESSIONS);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_migration_resumes_only_when_copy_matches_legacy() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let legacy = root.join("session.json");
        let first = Session::new(&root, "original").unwrap();
        first.save_to(&legacy).unwrap();
        fs::create_dir_all(&store.root).unwrap();
        first.save_to(store.history_path(first.id())).unwrap();
        store.migrate_legacy(&legacy, "litellm").unwrap();
        assert_eq!(store.load().unwrap().migrated_legacy_id, Some(first.id()));
        assert_eq!(store.load().unwrap().sessions.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn divergent_unindexed_copy_is_never_overwritten() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let legacy = root.join("session.json");
        let first = Session::new(&root, "original").unwrap();
        first.save_to(&legacy).unwrap();
        fs::create_dir_all(&store.root).unwrap();
        let mut changed = first.clone();
        changed.add_message(crate::Message::new(crate::Role::User, "different").unwrap());
        changed.save_to(store.history_path(first.id())).unwrap();
        assert!(store.migrate_legacy(&legacy, "litellm").is_err());
        assert_eq!(
            Session::load_from(store.history_path(first.id())).unwrap(),
            changed
        );
        assert!(store.load().unwrap().sessions.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_index_loads_and_checkpoint_adds_bounded_title_and_owner() {
        let root = sandbox();
        let store = ProjectSessionStore::new(&root);
        let mut history = Session::new(&root, "model").unwrap();
        store.create(&history, "litellm").unwrap();
        let path = store.index_path();
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        json["sessions"][0]
            .as_object_mut()
            .unwrap()
            .remove("project_path");
        json["sessions"][0].as_object_mut().unwrap().remove("title");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(store.load().unwrap().sessions[0].project_path.is_none());
        history.add_message(crate::Message::new(crate::Role::Tool, "secret tool result").unwrap());
        history
            .add_message(crate::Message::new(crate::Role::User, "  first   question  ").unwrap());
        store.checkpoint(&history).unwrap();
        let entry = &store.load().unwrap().sessions[0];
        assert_eq!(entry.title.as_deref(), Some("first question"));
        assert_eq!(
            entry.project_path.as_deref(),
            Some(root.canonicalize().unwrap().as_path())
        );
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("secret tool result")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_is_scoped_and_corrupt_history_cannot_be_deleted() {
        let root = sandbox();
        let other = sandbox();
        let store = ProjectSessionStore::new(&root);
        let foreign = ProjectSessionStore::new(&other);
        let first = Session::new(&root, "model").unwrap();
        let second = Session::new(&root, "model").unwrap();
        store.create(&first, "litellm").unwrap();
        store.create(&second, "litellm").unwrap();
        assert!(foreign.delete(first.id()).is_err());
        fs::write(store.history_path(first.id()), "{broken").unwrap();
        assert!(store.delete(first.id()).is_err());
        assert_eq!(store.load().unwrap().sessions.len(), 2);
        store.delete(second.id()).unwrap();
        assert!(!store.history_path(second.id()).exists());
        assert!(store.delete(second.id()).is_err());
        assert_eq!(
            fs::read_to_string(store.history_path(first.id())).unwrap(),
            "{broken"
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(other).unwrap();
    }
}
