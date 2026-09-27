//! Tauri desktop adapter for the transport-neutral application service.
//!
//! The adapter intentionally contains no agent-loop or permission logic.  It
//! only translates IPC requests into `ai_agent::application` commands and
//! serializes the resulting safe DTO envelope for the frontend.

use ai_agent::application::{
    ApplicationCommand, ApplicationEnvelope, ApplicationEvent, ApplicationService,
};
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Shared service state owned by one desktop process.
struct DesktopState(Mutex<ApplicationService>);

/// Executes one versioned application command through the shared service.
///
/// Tauri exposes only this narrow boundary.  The frontend cannot access the
/// filesystem, credentials, tools, or the internal session maps directly.
#[tauri::command]
async fn execute_command(
    state: State<'_, DesktopState>,
    request_id: String,
    command: ApplicationCommand,
) -> Result<ApplicationEnvelope<ApplicationEvent>, String> {
    eprintln!("[desktop] execute_command request_id={request_id}");
    let request_id =
        Uuid::parse_str(&request_id).map_err(|error| format!("invalid request id: {error}"))?;
    let mut service = state.0.lock().await;
    let result = service
        .execute(request_id, command)
        .map_err(|error| error.to_string());
    eprintln!("[desktop] execute_command completed success={}", result.is_ok());
    result
}

/// Refreshes model identifiers from the configured provider APIs.
///
/// Network access is kept in the Rust service; the frontend receives only the
/// bounded public provider DTOs returned by the application layer.
#[tauri::command]
async fn refresh_models(
    state: State<'_, DesktopState>,
    request_id: String,
    project_id: String,
) -> Result<ApplicationEnvelope<ApplicationEvent>, String> {
    eprintln!("[desktop] refresh_models project_id={project_id} request_id={request_id}");
    let request_id = Uuid::parse_str(&request_id)
        .map_err(|error| format!("invalid request id: {error}"))?;
    let mut service = state.0.lock().await;
    let providers = service
        .refresh_models(&project_id)
        .await
        .map_err(|error| error.to_string())?;
    let result = Ok(ApplicationEnvelope {
        api_version: ai_agent::application::APPLICATION_API_VERSION,
        request_id,
        sequence: service.next_sequence(),
        payload: ApplicationEvent::ModelsListed { providers },
    });
    eprintln!("[desktop] refresh_models completed success=true");
    result
}

/// Runs one complete prompt through the existing Agent loop.
///
/// The current provider API is request/response based, so this command keeps
/// the window responsive through Tauri's async command boundary while the
/// next stage adds streamed text and tool lifecycle events.
#[tauri::command]
async fn send_message(
    state: State<'_, DesktopState>,
    request_id: String,
    session_id: String,
    prompt: String,
) -> Result<ApplicationEnvelope<ApplicationEvent>, String> {
    let request_id = Uuid::parse_str(&request_id)
        .map_err(|error| format!("invalid request id: {error}"))?;
    let session_id = Uuid::parse_str(&session_id)
        .map_err(|error| format!("invalid session id: {error}"))?;
    eprintln!("[desktop] send_message session_id={session_id} request_id={request_id}");
    let mut service = state.0.lock().await;
    let result = service
        .send_message(request_id, session_id, &prompt)
        .await
        .map_err(|error| error.to_string());
    eprintln!("[desktop] send_message completed success={}", result.is_ok());
    result
}

/// Returns the Tauri application and registers the stateful command adapter.
fn main() {
    tauri::Builder::default()
        .manage(DesktopState(Mutex::new(ApplicationService::new())))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![execute_command, refresh_models, send_message])
        .run(tauri::generate_context!())
        .expect("error while running ai-agent desktop application");
}
