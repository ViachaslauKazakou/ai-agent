# Desktop startup sequence

The Tauri adapter initializes persistent project metadata before the frontend requests its first command.

## Startup

1. `src-tauri/src/main.rs` resolves `~/.ai-agent/state.json` through `LaunchStateStore::in_user_home()`.
2. `ApplicationService::with_launch_state_store` loads the versioned document and records the current launch time.
3. The frontend can send `get_startup_state` through the existing `execute_command` IPC boundary.
4. The service returns `StartupStateDto` with recent project paths, stable opaque IDs, availability, timestamps, and the suggested project ID.
5. No project is initialized merely by listing startup state. Opening remains an explicit `open_project` command.

If the home directory cannot be resolved, the state file is malformed, or persistence fails, Tauri reports a diagnostic to stderr and starts an in-memory service. The original state document is not replaced. Project opening and chat therefore remain available for recovery.

## Opening a project

`open_project` validates and canonicalizes the selected directory, loads project configuration, then persists it as the most recent project. Registration is transactional at the application-service level: a configuration or state-write failure does not leave a partially registered project in runtime maps.

Opening an already registered canonical path returns the existing project instead of creating a duplicate runtime identifier. Persisted projects use the stable opaque identifier stored in launch state.

Missing recent project paths remain in startup metadata with `available: false`. This lets the launcher explain that a directory moved or was removed instead of silently losing history.

## Security boundary

The frontend does not receive filesystem permissions or direct access to the state file. `StartupStateDto` contains only:

- opaque project ID;
- canonical path;
- availability flag;
- last-opened timestamp;
- suggested project ID and launch timestamp.

Provider settings, credentials, messages, session history, tool inputs, and tool outputs never enter this DTO.

## Verification

```bash
cargo test -p ai-agent application::tests
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
