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
- DTOs never contain API keys, OAuth refresh tokens, raw tool arguments, or
  unbounded tool results.
- Project and session identifiers are opaque to the transport.
- The backend remains responsible for path validation, configuration loading,
  tool permissions, confirmation, and secret redaction.
- New commands and events must be additive or require a new API version.

## Current commands

- `get_capabilities`
- `get_startup_state`
- `open_project`
- `list_projects`
- `create_session`
- `list_sessions`
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
and model identifiers. API keys, endpoint secrets, and OAuth credentials never
cross the application boundary.

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
frontend is loaded from `frontend/`; no API keys are placed in the frontend or
Tauri configuration. Choose a directory with the native `Choose...` button or
enter an absolute path, then select `Open project`, create a session, and use
the prompt field to send a request. The backend rejects missing paths before
registration.

For a production package, run `cargo tauri build` from `src-tauri/` only after
adding platform icons, signing identities and CI secrets through the target
platform's secure release configuration.
