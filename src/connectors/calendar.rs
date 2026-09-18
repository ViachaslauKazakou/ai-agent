use super::gmail_auth::GmailAuth;
use crate::AppError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const API: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub calendar: String,
    pub title: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub html_link: Option<String>,
    pub source: String,
}

#[derive(Clone)]
pub struct GoogleCalendarClient {
    client: Client,
    auth: GmailAuth,
}

impl GoogleCalendarClient {
    pub fn new(auth: GmailAuth, timeout: Duration) -> Result<Self, AppError> {
        Ok(Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|e| AppError::Tool(e.to_string()))?,
            auth,
        })
    }

    pub async fn list_events(
        &self,
        from: &str,
        to: &str,
        limit: usize,
    ) -> Result<Vec<CalendarEvent>, AppError> {
        let calendars: CalendarListResponse = self.get("/users/me/calendarList").await?;
        let mut result = Vec::new();
        for calendar in calendars.items {
            if result.len() >= limit {
                break;
            }
            let url = format!(
                "{API}/calendars/{}/events",
                urlencoding::encode(&calendar.id)
            );
            let response = self
                .client
                .get(url)
                .bearer_auth(self.auth.access_token().await?)
                .query(&[
                    ("timeMin", from),
                    ("timeMax", to),
                    ("singleEvents", "true"),
                    ("orderBy", "startTime"),
                    (
                        "maxResults",
                        &limit.saturating_sub(result.len()).to_string(),
                    ),
                ])
                .send()
                .await
                .map_err(|e| AppError::Tool(format!("Google Calendar network error: {e}")))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| AppError::Tool(e.to_string()))?;
            if !status.is_success() {
                return Err(AppError::Tool(format!(
                    "Google Calendar HTTP {}: {}",
                    status.as_u16(),
                    body.chars().take(1000).collect::<String>()
                )));
            }
            let page: EventsResponse = serde_json::from_str(&body)
                .map_err(|e| AppError::Tool(format!("Google Calendar JSON error: {e}")))?;
            result.extend(
                page.items
                    .into_iter()
                    .map(|event| normalize(event, &calendar.summary)),
            );
        }
        Ok(result)
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, AppError> {
        let response = self
            .client
            .get(format!("{API}{path}"))
            .bearer_auth(self.auth.access_token().await?)
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("Google Calendar network error: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        if !status.is_success() {
            return Err(AppError::Tool(format!(
                "Google Calendar HTTP {}: {}",
                status.as_u16(),
                body.chars().take(1000).collect::<String>()
            )));
        }
        serde_json::from_str(&body)
            .map_err(|e| AppError::Tool(format!("Google Calendar JSON error: {e}")))
    }
}

#[derive(Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<CalendarRef>,
}
#[derive(Deserialize)]
struct CalendarRef {
    id: String,
    summary: String,
}
#[derive(Deserialize)]
struct EventsResponse {
    #[serde(default)]
    items: Vec<GoogleEvent>,
}
#[derive(Deserialize)]
struct GoogleEvent {
    id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    status: Option<String>,
    #[serde(rename = "htmlLink")]
    html_link: Option<String>,
    start: EventTime,
    end: EventTime,
}
#[derive(Deserialize)]
struct EventTime {
    date_time: Option<String>,
    date: Option<String>,
}

fn normalize(event: GoogleEvent, calendar: &str) -> CalendarEvent {
    CalendarEvent {
        id: event.id,
        calendar: calendar.to_owned(),
        title: event.summary.unwrap_or_else(|| "(без названия)".to_owned()),
        start: event.start.date_time.or(event.start.date),
        end: event.end.date_time.or(event.end.date),
        location: event.location,
        description: event.description,
        status: event.status,
        html_link: event.html_link,
        source: "google_calendar".to_owned(),
    }
}
