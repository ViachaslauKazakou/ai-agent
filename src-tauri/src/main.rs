//! Tauri desktop adapter for the transport-neutral application service.
//!
//! The adapter intentionally contains no agent-loop or permission logic.  It
//! only translates IPC requests into `ai_agent::application` commands and
//! serializes the resulting safe DTO envelope for the frontend.

use ai_agent::application::{
    ApplicationCommand, ApplicationEnvelope, ApplicationEvent, ApplicationService,
    SettingsDocuments,
};
use ai_agent::LaunchStateStore;
use std::{path::PathBuf, sync::Arc};
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Shared service state owned by one desktop process.
struct DesktopState(Mutex<ApplicationService>);

/// Redacted current tool name shared with the desktop timeline.
struct DesktopActivity(Arc<std::sync::Mutex<Option<String>>>);

/// A validated launch path shown once on the launcher before activation.
struct DirectOpen(Option<PathBuf>);

/// Parses the optional desktop path; invalid flags never silently open a project.
fn direct_open_path(args: impl IntoIterator<Item = String>) -> Result<Option<PathBuf>, String> {
    let mut args = args.into_iter();
    let _binary = args.next();
    let Some(flag) = args.next() else {
        return Ok(None);
    };
    if flag != "--working-dir" {
        return Err(format!("unsupported desktop argument: {flag}"));
    }
    let path = args.next().ok_or("--working-dir requires a directory")?;
    if args.next().is_some() {
        return Err("unexpected additional desktop arguments".to_owned());
    }
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err(format!(
            "direct-open directory unavailable: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Supplies a prevalidated launch path without granting WebView filesystem access.
#[tauri::command]
fn initial_project_path(path: State<'_, DirectOpen>) -> Option<PathBuf> {
    path.0.clone()
}

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
    eprintln!(
        "[desktop] execute_command completed success={}",
        result.is_ok()
    );
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
    let request_id =
        Uuid::parse_str(&request_id).map_err(|error| format!("invalid request id: {error}"))?;
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
    activity: State<'_, DesktopActivity>,
    request_id: String,
    session_id: String,
    prompt: String,
) -> Result<ApplicationEnvelope<ApplicationEvent>, String> {
    let request_id =
        Uuid::parse_str(&request_id).map_err(|error| format!("invalid request id: {error}"))?;
    let session_id =
        Uuid::parse_str(&session_id).map_err(|error| format!("invalid session id: {error}"))?;
    eprintln!("[desktop] send_message session_id={session_id} request_id={request_id}");
    let mut service = state.0.lock().await;
    let result = service
        .send_message_with_activity(request_id, session_id, &prompt, activity.0.clone())
        .await
        .map_err(|error| error.to_string());
    eprintln!(
        "[desktop] send_message completed success={}",
        result.is_ok()
    );
    result
}

/// Loads the two project-local JSON documents for the settings dialog.
#[tauri::command]
async fn read_settings(
    state: State<'_, DesktopState>,
    project_id: String,
) -> Result<SettingsDocuments, String> {
    let service = state.0.lock().await;
    service
        .read_settings(&project_id)
        .map_err(|error| error.to_string())
}

/// Validates and atomically saves edited project settings.
///
/// Validation happens in the shared configuration module before any runtime
/// state is reloaded, so malformed JSON cannot replace the active files.
#[tauri::command]
async fn write_settings(
    state: State<'_, DesktopState>,
    project_id: String,
    config_json: String,
    providers_json: String,
) -> Result<(), String> {
    let mut service = state.0.lock().await;
    service
        .write_settings(&project_id, &config_json, &providers_json)
        .map_err(|error| error.to_string())
}

/// Returns only the current tool name; arguments and results stay private.
#[tauri::command]
fn tool_activity(activity: State<'_, DesktopActivity>) -> Option<String> {
    activity.0.lock().ok().and_then(|value| value.clone())
}

/// Returns the Tauri application and registers the stateful command adapter.
fn main() {
    let launch_path = match direct_open_path(std::env::args()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[desktop] direct-open ignored: {error}");
            None
        }
    };
    let service = desktop_service();
    tauri::Builder::default()
        .manage(DirectOpen(launch_path))
        .manage(DesktopState(Mutex::new(service)))
        .manage(DesktopActivity(Arc::new(std::sync::Mutex::new(None))))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            execute_command,
            initial_project_path,
            refresh_models,
            send_message,
            tool_activity,
            read_settings,
            write_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running ai-agent desktop application");
}

/// Builds the desktop service with user-scoped startup persistence.
///
/// A damaged or unavailable state file must not make the desktop binary
/// unusable. The original file is preserved by `LaunchStateStore`; this launch
/// falls back to memory-only state and reports the diagnostic to stderr.
fn desktop_service() -> ApplicationService {
    let store = match LaunchStateStore::in_user_home() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("[desktop] launch state disabled: {error}");
            return ApplicationService::new();
        }
    };
    match ApplicationService::with_launch_state_store(store) {
        Ok(service) => service,
        Err(error) => {
            eprintln!("[desktop] launch state disabled: {error}");
            ApplicationService::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::direct_open_path;

    #[test]
    fn direct_open_accepts_only_an_existing_directory() {
        let path = std::env::temp_dir();
        let parsed = direct_open_path([
            "via-agent".to_owned(),
            "--working-dir".to_owned(),
            path.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert_eq!(parsed, Some(path.canonicalize().unwrap()));
        assert!(direct_open_path([
            "via-agent".to_owned(),
            "--working-dir".to_owned(),
            path.join("missing").to_string_lossy().into_owned()
        ])
        .is_err());
        assert!(direct_open_path(["via-agent".to_owned(), "--working-dir".to_owned()]).is_err());
        assert!(direct_open_path(["via-agent".to_owned(), "--other".to_owned()]).is_err());
        assert_eq!(direct_open_path(["via-agent".to_owned()]).unwrap(), None);
    }
}
