use std::fs;

use ai_agent::{LAUNCH_STATE_SCHEMA_VERSION, LaunchState, LaunchStateStore};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

#[test]
fn public_launch_state_api_persists_recent_project_metadata() {
    let root = std::env::temp_dir().join(format!("ai-agent-public-state-{}", Uuid::new_v4()));
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let store = LaunchStateStore::new(root.join("user/state.json"));
    let opened_at = Utc
        .with_ymd_and_hms(2026, 9, 29, 12, 0, 0)
        .single()
        .unwrap();

    let saved = store.record_project_opened(&project, opened_at).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, saved);
    assert_eq!(loaded.schema_version, LAUNCH_STATE_SCHEMA_VERSION);
    assert_eq!(loaded.projects.len(), 1);
    assert_eq!(loaded.projects[0].path, project.canonicalize().unwrap());
    assert_eq!(loaded.last_project_id, Some(loaded.projects[0].id.clone()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_launch_state_api_does_not_touch_disk_on_construction() {
    let root = std::env::temp_dir().join(format!("ai-agent-lazy-state-{}", Uuid::new_v4()));
    let path = root.join("state.json");

    let store = LaunchStateStore::new(&path);

    assert_eq!(store.path(), path);
    assert!(!root.exists());
    assert_eq!(store.load().unwrap(), LaunchState::default());
}
