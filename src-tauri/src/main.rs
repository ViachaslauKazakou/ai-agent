//! Tauri desktop adapter for the transport-neutral application service.
//!
//! The adapter intentionally contains no agent-loop or permission logic.  It
//! only translates IPC requests into `ai_agent::application` commands and
//! serializes the resulting safe DTO envelope for the frontend.

use std::sync::Mutex;

use ai_agent::application::{
    ApplicationCommand, ApplicationEnvelope, ApplicationEvent, ApplicationService,
};
use tauri::State;
use uuid::Uuid;

/// Shared service state owned by one desktop process.
struct DesktopState(Mutex<ApplicationService>);

/// Executes one versioned application command through the shared service.
///
/// Tauri exposes only this narrow boundary.  The frontend cannot access the
/// filesystem, credentials, tools, or the internal session maps directly.
#[tauri::command]
fn execute_command(
    state: State<'_, DesktopState>,
    request_id: String,
    command: ApplicationCommand,
) -> Result<ApplicationEnvelope<ApplicationEvent>, String> {
    let request_id =
        Uuid::parse_str(&request_id).map_err(|error| format!("invalid request id: {error}"))?;
    let mut service = state
        .0
        .lock()
        .map_err(|_| "desktop service lock is poisoned".to_owned())?;
    service
        .execute(request_id, command)
        .map_err(|error| error.to_string())
}

/// Returns the Tauri application and registers the stateful command adapter.
fn main() {
    tauri::Builder::default()
        .manage(DesktopState(Mutex::new(ApplicationService::new())))
        .invoke_handler(tauri::generate_handler![execute_command])
        .run(tauri::generate_context!())
        .expect("error while running ai-agent desktop application");
}
