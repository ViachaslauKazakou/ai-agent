use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Outlook,
    Gmail,
    Telegram,
    Teams,
    Whatsapp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageQuery {
    pub lookback_hours: u64,
    pub max_messages: usize,
    pub search: Option<String>,
    pub include_body: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub source: SourceKind,
    pub sender: String,
    pub recipients: Vec<String>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    pub body: Option<String>,
    pub sent_at: Option<String>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedThread {
    pub id: String,
    pub source: SourceKind,
    pub messages: Vec<NormalizedMessage>,
}
