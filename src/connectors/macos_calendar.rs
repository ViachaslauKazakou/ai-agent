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
function recurrenceValue(rule, key) {
  const match = rule.match(new RegExp('(?:^|;)'+key+'=([^;]+)'));
  return match ? match[1] : null;
}
function dayNumber(code) {
  return {SU: 0, MO: 1, TU: 2, WE: 3, TH: 4, FR: 5, SA: 6}[code];
}
function occurrences(event, fromMs, toMs) {
  const rule = String(event.recurrence() || '');
  const masterStart = new Date(event.startDate());
  const masterEnd = new Date(event.endDate());
  const duration = masterEnd.getTime() - masterStart.getTime();
  if (!rule || rule === 'missing value') return [{start: masterStart, end: masterEnd}];
  const frequency = recurrenceValue(rule, 'FREQ');
  const interval = Number(recurrenceValue(rule, 'INTERVAL') || 1);
  const count = Number(recurrenceValue(rule, 'COUNT') || 0);
  const untilValue = recurrenceValue(rule, 'UNTIL');
  const until = untilValue
    ? Date.parse(untilValue.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/, '$1-$2-$3T$4:$5:$6Z'))
    : Infinity;
  const days = (recurrenceValue(rule, 'BYDAY') || '').split(',').map(dayNumber).filter(Number.isFinite);
  if (frequency !== 'WEEKLY' || !days.length) return [{start: masterStart, end: masterEnd}];
  const result = [];
  const firstDay = new Date(masterStart);
  firstDay.setHours(0, 0, 0, 0);
  const baseWeekStart = firstDay.getTime() - firstDay.getDay() * 86400000;
  let generated = 0;
  for (let dayOffset = 0; dayOffset < 3660; dayOffset += 1) {
    const day = new Date(firstDay.getTime() + dayOffset * 86400000);
    const week = Math.floor((day.getTime() - baseWeekStart) / (7 * 86400000));
    if (week % interval !== 0 || !days.includes(day.getDay())) continue;
    const occurrenceStart = new Date(day.getTime() + (masterStart.getTime() - firstDay.getTime()));
    const occurrenceMs = occurrenceStart.getTime();
    generated += 1;
    if ((count && generated > count) || occurrenceMs > until) break;
    if (occurrenceMs < toMs && occurrenceMs + duration >= fromMs) {
      result.push({start: occurrenceStart, end: new Date(occurrenceMs + duration)});
    }
    if (occurrenceMs > toMs && generated > count) break;
  }
  return result;
}
let out = [];
for (const cal of Calendar.calendars()) {
  // Ask Calendar for the date-bounded collection first. Reading every event
  // from every calendar is very slow for synced accounts and can make the
  // tool appear to return no data before the process times out.
  const events = cal.events();
  for (const event of events) {
    for (const occurrence of occurrences(event, from, to)) {
      const start = occurrence.start;
      const end = occurrence.end;
      const startMs = timestamp(start);
      const endMs = timestamp(end);
      // Expand recurring Calendar events because JXA exposes only the master
      // event; the Calendar UI still displays its future occurrences.
      if (Number.isFinite(startMs) && Number.isFinite(endMs) && endMs >= from && startMs < to && out.length < max) out.push({id: String(event.uid()) + ':' + start.toISOString(), calendar: String(cal.name()), title: String(event.summary() || '(без названия)'), start: start.toISOString(), end: end.toISOString(), location: event.location() ? String(event.location()) : null, description: event.description() ? String(event.description()) : null, status: null, html_link: null, source: 'macos_calendar'});
      if (out.length >= max) break;
    }
    if (out.length >= max) break;
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
