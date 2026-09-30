# Launch state

`LaunchStateStore` provides shared startup metadata for CLI and Desktop adapters. It is intentionally independent from project configuration and session contents.

## Location

Production composition roots resolve the document as:

```text
~/.ai-agent/state.json
```

Library code and tests can inject any file through `LaunchStateStore::new(path)`. Tests must not use `LaunchStateStore::in_user_home()`.

The file and its parent directory are created on the first successful save, not during library construction.

## Schema version 1

The document stores:

- `schema_version` — exact format version;
- `last_started_at` — UTC timestamp recorded by an adapter;
- `last_project_id` — stable opaque identifier of the latest project;
- `last_session_id` — reserved link for the session-index phase;
- `projects` — at most 20 canonical project paths, newest first.

Opening the same canonical path promotes its existing record and preserves its ID. A new project receives a random UUID-based identifier. Runtime IDs such as `project-1` are not persisted.

## Safety guarantees

Launch state contains metadata only. It must never contain:

- provider API keys or provider configuration documents;
- OAuth credentials or connector tokens;
- prompts, assistant messages, or session history;
- tool arguments, results, or file contents.

Writes use a uniquely named temporary sibling followed by a filesystem rename. A failed rename removes the temporary file. Readers either observe the previous complete document or the newly serialized document on filesystems that support atomic replacement.

A missing file produces the default versioned state. Malformed JSON and unknown schema versions return `AppError::LaunchState`; the original file remains unchanged so an adapter can report, back up, or migrate it instead of silently losing data.

## Adapter integration

Stage 1 exposes the storage primitive but does not read the user's home automatically during ordinary library or service construction. Future CLI and Tauri stages will:

1. resolve the production store in their composition root;
2. handle a load error without preventing an explicitly configured project from opening;
3. record startup and successfully opened projects;
4. expose only secret-free DTOs to the frontend.

## Verification

```bash
cargo test -p ai-agent launch_state
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
