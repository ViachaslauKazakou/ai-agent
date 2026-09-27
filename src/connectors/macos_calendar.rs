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
// Compare numeric timestamps instead of JavaScript Date objects. Calendar's
// JXA bridge may return ObjC date wrappers, which do not compare reliably with
// native Date instances and can silently produce an empty result.
const from = Date.parse(env('AI_CAL_FROM'));
const to = Date.parse(env('AI_CAL_TO'));
const max = Number(env('AI_CAL_LIMIT'));
const fromDate = new Date(from);
const toDate = new Date(to);
function timestamp(value) {
  // JXA can expose Calendar dates as NSDate wrappers rather than native
  // JavaScript Date values. Prefer the native epoch method and fall back to
  // Date parsing for macOS versions that bridge them differently.
  if (value && typeof value.timeIntervalSince1970 === 'function') return Number(value.timeIntervalSince1970()) * 1000;
  const parsed = new Date(value).getTime();
  return Number.isFinite(parsed) ? parsed : NaN;
}
let out = [];
for (const cal of Calendar.calendars()) {
  // Ask Calendar for the date-bounded collection first. Reading every event
  // from every calendar is very slow for synced accounts and can make the
  // tool appear to return no data before the process times out.
  const events = cal.events.whose({ startDate: { $greaterThan: fromDate, $lessThan: toDate } })();
  for (const event of events) {
    const start = new Date(event.startDate());
    const end = new Date(event.endDate());
    const startValue = event.startDate();
    const endValue = event.endDate();
    const startMs = timestamp(startValue);
    const endMs = timestamp(endValue);
    // Include an event when it starts in the range or overlaps its beginning.
    // This also handles all-day and multi-day Calendar entries.
    if (Number.isFinite(startMs) && Number.isFinite(endMs) && endMs >= from && startMs < to && out.length < max) out.push({id: String(event.uid()), calendar: String(cal.name()), title: String(event.summary() || '(без названия)'), start: start.toISOString(), end: end.toISOString(), location: event.location() ? String(event.location()) : null, description: event.description() ? String(event.description()) : null, status: null, html_link: null, source: 'macos_calendar'});
  }
}
JSON.stringify(out);"#;
        let output = tokio::time::timeout(
            _timeout,
            tokio::process::Command::new("osascript")
                .args(["-l", "JavaScript", "-e", script])
                .env("AI_CAL_FROM", from)
                .env("AI_CAL_TO", to)
                .env("AI_CAL_LIMIT", limit.to_string())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| AppError::Tool("macOS Calendar timeout".into()))?
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
