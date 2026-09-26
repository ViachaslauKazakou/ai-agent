# Desktop Application API

This document describes the first transport-neutral layer for the desktop
client planned in issue #6.

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
- `open_project`
- `create_session`
- `list_sessions`
- `cancel_request`

The `ApplicationService::execute` dispatcher validates each command, assigns a
monotonic sequence number, and returns one `ApplicationEnvelope` containing the
same request ID.  Project paths must exist before registration; sessions can
only be created for registered projects.  This keeps basic validation in the
shared service instead of duplicating it in a desktop or browser adapter.

Workers use `RequestCancellation` as a cooperative flag.  A transport can
register a request, pass the non-serializable handle to the async provider
worker, and dispatch `cancel_request` later.  Providers and mutating tools must
check the flag only at safe boundaries; forcibly aborting a write in the middle
of a checkpoint transaction is not supported.

The current stage manages project and session metadata only.  It intentionally
does not claim to stream LLM output yet.  Streaming, cancellation, tool
lifecycle events, and confirmation requests are the next service-layer stage.

## Frontend integration direction

The recommended desktop implementation is a Tauri 2 adapter over this API.
The contract can also be exposed by a loopback-only WebSocket/REST adapter for
a browser or PWA.  Such an adapter must add a per-launch authentication token,
strict origin checks, bounded payloads, and must never bind to `0.0.0.0`.
