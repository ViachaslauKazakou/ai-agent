use super::calendar::CalendarEvent;
use crate::AppError;
use std::time::Duration;

pub async fn list_events(
    from: &str,
    to: &str,
    limit: usize,
    _timeout: Duration,
) -> Result<Vec<CalendarEvent>, AppError> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (from, to, limit, _timeout);
        return Err(AppError::Tool(
            "macOS Calendar доступен только на macOS".into(),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let script = r#"ObjC.import('Foundation');
function env(name) {
  return ObjC.unwrap($.NSProcessInfo.processInfo.environment.objectForKey(name));
}
const Calendar = Application('Calendar');
const from = new Date(env('AI_CAL_FROM'));
const to = new Date(env('AI_CAL_TO'));
const max = Number(env('AI_CAL_LIMIT'));
let out = [];
for (const cal of Calendar.calendars()) {
  for (const event of cal.events()) {
    const start = event.startDate();
    if (start >= from && start <= to && out.length < max) out.push({id: String(event.uid()), calendar: String(cal.name()), title: String(event.summary() || '(без названия)'), start: start.toISOString(), end: event.endDate().toISOString(), location: event.location() ? String(event.location()) : null, description: event.description() ? String(event.description()) : null, status: null, html_link: null, source: 'macos_calendar'});
  }
}
JSON.stringify(out);"#;
        let output = tokio::process::Command::new("osascript")
            .args(["-l", "JavaScript", "-e", script])
            .env("AI_CAL_FROM", from)
            .env("AI_CAL_TO", to)
            .env("AI_CAL_LIMIT", limit.to_string())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AppError::Tool(format!("macOS Calendar недоступен: {e}")))?;
        if !output.status.success() {
            return Err(AppError::Tool(format!(
                "macOS Calendar: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|e| AppError::Tool(format!("macOS Calendar JSON error: {e}")))
    }
}
