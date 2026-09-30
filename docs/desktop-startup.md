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

Creating a Desktop session records its UUID, provider key, and model alongside
the project's launch metadata. The next launch does not treat this metadata
as history: it offers a session only when a valid indexed or legacy file
exists. The frontend calls `get_restorable_session` and `restore_session` after
opening the project, and displays the number of stored messages without
exposing message bodies. Incompatible history is reported as a recoverable
launcher error. `list_sessions` returns the project's ten most recently updated
histories for explicit switching; creating an eleventh removes the oldest.
Legacy history is copied into project-local multi-session storage without
deleting the original CLI file.

At desktop startup `--working-dir /existing/project` is canonicalized and
passed to the frontend as a direct-open project path. An invalid argument
is reported on stderr; the launcher remains usable. An opening error keeps
the UI on the launcher. Configuration stays in that project's `.aiagent`
directory; an independently configured tool working directory is tracked
separately from the project directory.

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
cargo test -p via-agent --bin via-agent
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
