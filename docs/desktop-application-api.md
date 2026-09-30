# Desktop Application API

This document describes the transport-neutral application layer and the current
desktop client implementation for issue #6.

## Why this layer exists

The existing CLI owns terminal concerns such as `rustyline`, `dialoguer`, and
`stdin/stdout`.  A desktop client must not duplicate the agent loop or access
`ToolContext` directly.  `src/application.rs` therefore introduces a small
application boundary with serializable commands, events, and DTOs.

The boundary is deliberately independent of Tauri, HTTP, WebSocket, React, or
any other frontend technology.  The same contract can later be adapted to
Tauri IPC or a loopback browser transport.

## Contract rules

- Every envelope contains `api_version`, `request_id`, and `sequence`.
- Ordinary command/event DTOs never contain API keys, OAuth refresh tokens,
  raw tool arguments, or unbounded tool results. `SettingsDocuments` is an
  explicitly sensitive exception for the trusted local settings editor.
- Project and session identifiers are opaque to the transport.
- The backend remains responsible for path validation, configuration loading,
  tool permissions, confirmation, and secret redaction.
- New commands and events must be additive or require a new API version.

## Current commands

- `get_capabilities`
- `get_tutor_capabilities` (opened project only; Tutor unavailable)
- `get_startup_state`
- `open_project`
- `list_projects`
- `create_session`
- `list_sessions`
- `get_restorable_session`
- `restore_session`
- `delete_session`
- `cancel_request`
- `list_models`
- `refresh_models`
- `send_message`

The `ApplicationService::execute` dispatcher validates each command, assigns a
monotonic sequence number, and returns one `ApplicationEnvelope` containing the
same request ID.  Project paths must exist before registration; sessions can
only be created for registered projects.  This keeps basic validation in the
shared service instead of duplicating it in a desktop or browser adapter.

Provider metadata is intentionally public but limited to registry names, kinds,
and model identifiers. Unlike these metadata commands, the separate
`read_settings`/`write_settings` Tauri IPC commands exchange raw project-local
`.aiagent/config.json` and `.aiagent/providers.json` with the local WebView
editor. Both documents can contain API keys or connector secrets; editing them
therefore requires trusting the bundled frontend and the selected project.
OAuth refresh tokens stored in the OS credential store are not part of those
files. Settings are not returned through the generic application command/event
envelope, copied into global launch state or logged by the settings UI. Close
the dialog to clear its textareas; this does not remove the on-disk secrets.
Never use this editor with untrusted injected frontend content. The settings
save path validates and replaces each file independently, not as one transaction.
Known configuration validation and JSON parsing errors omit the original
invalid value (they report the field/line/column instead), since project-open
errors can also reach the generic UI or stderr. The editor displays only a
generic settings failure; inspect project files locally for details.

`open_project` now reuses the existing `Config::load` path. It canonicalizes the
directory, initializes missing `.aiagent` project files through the existing
configuration code, loads `providers.json`, and keeps the resulting `Config`
private in the service. The frontend receives only `ProviderDto` metadata. This
avoids a second desktop-specific configuration parser and keeps CLI and desktop
configuration behavior aligned.

`get_startup_state` returns secret-free metadata loaded from the user-scoped
launch-state document. Recent projects keep stable opaque IDs across desktop
processes and include an `available` flag so a launcher can render moved or
deleted paths without failing startup. Listing this state never initializes or
modifies a project; `open_project` remains the explicit activation boundary.

The application protocol is now **version 4** (`APPLICATION_API_VERSION`).
`get_restorable_session` returns `restorable_session` with an optional
`SessionDto` (UUID, project ID, provider, model, message count, optional timestamps
and bounded first-user-message title), never full message bodies or tool results.
Titles are project-local excerpts and may contain sensitive user text; do not
log them or copy them to global launch state. `restore_session` requires both project ID and a listed UUID;
it emits `session_restored` only after loading and validating the indexed history
or migrating `<project>/.aiagent/session.json`. The file's working directory must match
the configured canonical tool directory. Provider/model must exist in the
project registry; indexed histories use their stored provider, while legacy
histories use matching launch metadata or the current default. Incompatible/corrupt/stale
files return an error and are not overwritten by discovery or restoration.
The frontend currently reports the count of previous messages; rendering the
full transcript is deferred to a later stage.

`list_sessions` now returns up to ten persisted sessions for the opened project,
newest updated first. `restore_session` accepts the UUID of any listed session.
`create_session` creates an empty, separately persisted history even when a
session is active. Histories live in `<project>/.aiagent/sessions/<uuid>.json`;
`sessions/index.json` (schema version 1) records provider/model and creation/
update timestamps, optional canonical project identity and bounded title, plus
a migrated-legacy UUID tombstone. Existing version 1 indexes lacking the new
fields remain readable; the next checkpoint fills them. At the eleventh creation the least recently updated session
is removed; opening a session does not alter its update timestamp. The index is
published before deleting the old history, so interrupted cleanup may leave an
unindexed orphan rather than delete a referenced history. Failed writes do not
erase the existing indexed history. Corrupt/missing indexed histories cause an
explicit error; they are never silently overwritten. Agent responses checkpoint
their session independently, including partial histories on provider failure.
`delete_session` requires project ID and indexed UUID, checks the history and
configured provider/model, atomically removes the index entry before deleting
the history file and clears runtime/last-selected metadata. An interrupted
cleanup can leave an unindexed orphan. The original CLI legacy file is never
deleted; the migration tombstone prevents its reappearance in Desktop.
Damaged histories fail explicitly without deleting other sessions. The
frontend removes its decorative attachment control: file attachments are not
transmitted by the current prompt API.

Desktop prompts are not capped at the default 20 tool rounds: the model can
continue calling tools until it produces a final response. The configured
`max_loop_seconds` (600 seconds by default), tool permissions, and per-tool
limits still apply. The CLI retains its configured `max_tool_rounds` behavior.

For the Desktop Chat-bot only, the service grants file writes by default when
canonical `working_dir == project_dir` and `confirm_writes` is false. This is
independent of CLI `--allow-write` and does not alter CLI permissions. The
default tool list includes create/write/patch; explicitly configured
`enabled_tools` is still respected. Patch application uses a Desktop-only
non-interactive approval policy (no terminal stdin), with preview and Git
checkpoint preserved. If `confirm_writes` is true, or a separate tool directory
is configured, file writes fail closed until a UI confirmation or separately
scoped permission flow is implemented. Writes cannot escape the canonical
project through traversal or symlinks; protected `.git/`, `.aiagent/`, secret
files and secret content remain forbidden. Git operations that require explicit
interactive confirmation are unchanged; shell commands remain allowlisted.

Legacy `<project>/.aiagent/session.json` is copied into the index before a new
session is created or that history is restored; the legacy file remains for the
CLI. Incompatible or damaged legacy files block creation until repaired to
avoid losing them. A legacy session not yet indexed is also shown in the list.
The user-scoped launch state stores only the selected session UUID/provider/model
and never contains conversation contents. The UI shows only message counts on
restoration, not the full transcript.

When Tauri starts, it constructs `ApplicationService` with a
`LaunchStateStore`. A successful `open_project` loads project configuration
before atomically recording the canonical path. Configuration or persistence
errors therefore cannot leave a partially registered runtime project. See
`docs/desktop-startup.md` for the complete sequence and fallback behavior.

`refresh_models` must be used after opening a project to query the configured
provider endpoints. `list_models` returns only the registry allowlist; it does
not make network requests. Refreshing therefore requires a reachable endpoint
and a valid provider API key where applicable. Ollama must be running locally,
and an OpenAI-compatible provider must expose `GET /models`.

Workers use `RequestCancellation` as a cooperative flag.  A transport can
register a request, pass the non-serializable handle to the async provider
worker, and dispatch `cancel_request` later.  Providers and mutating tools must
check the flag only at safe boundaries; forcibly aborting a write in the middle
of a checkpoint transaction is not supported.

The desktop client now supports the existing non-streaming Agent loop through
`send_message`. The
provider request/response is awaited by the async Tauri command and the final
assistant DTO is returned to the UI. Streaming text, live tool lifecycle events,
and confirmation requests remain the next service-layer stage.

Assistant uses `assistant_capabilities`, `assistant_prompts`,
`save_assistant_prompts`, and `send_assistant_message` as thin Tauri commands
over `ApplicationService`. Only project/session identifiers, prompt text and an
optional selected document path cross IPC. The Rust service rechecks enabled
tools and uses an explicit read-only registry and `allow_write=false` context;
presets cannot widen permissions. Local documents are read by the registered
MCP tool, bounded to 30 KB of extracted output before appending to the prompt,
and never returned as raw tool results to the WebView. This prompt is saved in
the project's session history and sent to its configured LLM provider; neither
raw document contents nor prompt bodies appear in the execution log or global
launch state. Prompt presets use versioned schema 1 in
`<project>/.aiagent/assistant-prompts.json` and are replaced atomically; invalid
or future-version files are preserved. Disabled capabilities do not imply that
OAuth has been performed; connector failures are reported at execution time.

The preliminary Desktop Coder inspection exposes `coder_tree`, `coder_changes`,
`coder_diff`, `coder_check`, and `coder_venv` through transport-neutral Rust
service methods. They are read-only and bounded; the existing agent send endpoint
is deliberately not available in Coder. See [Coder limitations](desktop-coder.md).

`get_tutor_capabilities` uses the same version-4 command/event envelope and
requires an opened project with loaded configuration. It returns only disabled
Tutor-specific feature flags and an explanatory reason; it cannot open lessons
or call the provider. The DTOs in `src/tutor.rs` are a future contract, not
stored sessions. See [Tutor architecture and threat model](desktop-tutor.md).

## Frontend integration direction

The recommended desktop implementation is a Tauri 2 adapter over this API.
The contract can also be exposed by a loopback-only WebSocket/REST adapter for
a browser or PWA.  Such an adapter must add a per-launch authentication token,
strict origin checks, bounded payloads, and must never bind to `0.0.0.0`.

## Local desktop smoke test

The repository contains a Tauri 2 shell in `src-tauri/` and a dependency-light
frontend in `frontend/`. The client opens a project, loads its configuration,
refreshes provider models, creates a session, and sends prompts through the
existing Agent loop. Responses are currently request/response based; streaming
text and live tool lifecycle events remain a later stage.

The desktop shell also provides a native directory picker through the Tauri
dialog plugin. The selected path is still validated and canonicalized by the
Rust application service; the picker does not grant the frontend direct file
access or bypass project permissions.

Install the platform prerequisites from the Tauri documentation, including a
Rust toolchain, Node.js, and the operating-system WebView development package.
Then run:

```bash
cd frontend
npm install
npm run check
cd ../src-tauri
cargo tauri dev
```

If the Tauri CLI is not installed, use `cargo install tauri-cli --version '^2'`
or invoke it through the project tooling used by your environment. The
frontend is loaded from `frontend/`; no API keys are compiled into its assets
or Tauri configuration, but the settings editor receives raw keys on demand.
Choose a directory with the native `Choose...` button or
enter an absolute path, then select `Open project`, create a session, and use
the prompt field to send a request. The backend rejects missing paths before
registration.

`via-agent --working-dir /existing/project` preselects and opens that
directory through the same application command. Invalid or unsupported
arguments are reported on stderr and leave the launcher available.

For a production package, run `cargo tauri build` from `src-tauri/` only after
adding platform icons, signing identities and CI secrets through the target
platform's secure release configuration.
