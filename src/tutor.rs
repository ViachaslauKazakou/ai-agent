//! Transport-neutral AI Tutor descriptors; lesson execution is not enabled.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Maximum size in bytes of one serialized lesson descriptor.
pub const MAX_TUTOR_LESSON_BYTES: usize = 65_536;

/// An ordered, bounded teaching question; its content is untrusted display data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutorQuestionDto {
    /// Lesson-local stable identifier, not a filesystem path.
    pub id: String,
    /// Short display title.
    pub title: String,
    /// Plain-text question shown to the learner.
    pub text: String,
}

/// Immutable lesson description; it is not an instruction or a tool grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutorLessonDto {
    /// Opaque lesson identifier.
    pub id: Uuid,
    /// Human-readable plain-text title.
    pub title: String,
    /// Ordered question list.
    pub questions: Vec<TutorQuestionDto>,
}

impl TutorLessonDto {
    /// Validates byte budgets and uniqueness before a future store or runner can use a lesson.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.title.trim().is_empty() || self.title.len() > 120 {
            return Err("Tutor lesson title must contain 1–120 bytes");
        }
        if !(1..=20).contains(&self.questions.len()) {
            return Err("Tutor lesson requires 1–20 questions");
        }
        let mut ids = HashSet::new();
        for question in &self.questions {
            if question.id.is_empty()
                || question.id.len() > 64
                || !question
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
                || !ids.insert(question.id.as_str())
                || question.title.trim().is_empty()
                || question.title.len() > 120
                || question.text.trim().is_empty()
                || question.text.len() > 4_096
            {
                return Err("Tutor question has an invalid ID, title or text");
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|_| "Tutor lesson cannot be serialized")?;
        if bytes.len() > MAX_TUTOR_LESSON_BYTES {
            return Err("Tutor lesson exceeds the byte budget");
        }
        Ok(())
    }
}

/// Proposed backend-owned lifecycle state (not currently persisted or advanced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TutorSessionStatus {
    /// Created but not started.
    Draft,
    /// Active lesson.
    InProgress,
    /// Completed lesson.
    Completed,
    /// Explicitly left before completion.
    Abandoned,
}

/// Proposed backend-owned lesson phase (not a model-selected action).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TutorPhase {
    /// Introduction.
    Intro,
    /// Explanation.
    Theory,
    /// Guided discussion.
    Dialog,
    /// Assessment.
    Quiz,
    /// Lesson wrap-up.
    Summary,
}

/// Proposed session metadata, separate from Chat-bot's session/history type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutorSessionDto {
    /// Opaque Tutor-only session identifier.
    pub id: Uuid,
    /// Opened project's opaque identifier.
    pub project_id: String,
    /// Bound lesson identifier.
    pub lesson_id: Uuid,
    /// Backend-controlled status.
    pub status: TutorSessionStatus,
    /// Backend-controlled phase.
    pub phase: TutorPhase,
    /// Zero-based position in the immutable question list.
    pub question_index: usize,
}

/// Per-project Tutor feature flags; unrelated to general chat capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TutorCapabilitiesDto {
    /// Whether a lesson can actually be started.
    pub available: bool,
    /// Whether bounded text turns can run.
    pub text: bool,
    /// Whether ordered incremental output can run.
    pub streaming: bool,
    /// Whether an active Tutor turn can be cancelled.
    pub cancellation: bool,
    /// Whether voice capture and playback are supported.
    pub voice: bool,
    /// Whether user-initiated lesson edits are supported.
    pub editing: bool,
    /// Whether Tutor progress can be persisted and restored.
    pub progress: bool,
    /// Human-readable explanation when Tutor is unavailable.
    pub reason: Option<String>,
}

impl TutorCapabilitiesDto {
    /// Reports the fail-closed capabilities of the contract-only Tutor slice.
    pub fn unavailable() -> Self {
        Self {
            available: false,
            text: false,
            streaming: false,
            cancellation: false,
            voice: false,
            editing: false,
            progress: false,
            reason: Some(
                "AI Tutor is not available yet; lesson execution is not implemented".into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson() -> TutorLessonDto {
        TutorLessonDto {
            id: Uuid::new_v4(),
            title: "Basics".into(),
            questions: vec![TutorQuestionDto {
                id: "q_1".into(),
                title: "First".into(),
                text: "What is a variable?".into(),
            }],
        }
    }

    #[test]
    fn lesson_round_trips_and_rejects_excess_fields() {
        let lesson = lesson();
        lesson.validate().unwrap();
        let encoded = serde_json::to_value(&lesson).unwrap();
        assert_eq!(
            serde_json::from_value::<TutorLessonDto>(encoded.clone()).unwrap(),
            lesson
        );
        let mut extra = encoded;
        extra["metadata"] = serde_json::json!({"execute": "shell"});
        assert!(serde_json::from_value::<TutorLessonDto>(extra).is_err());
    }

    #[test]
    fn lesson_rejects_empty_duplicate_and_oversized_inputs() {
        let mut value = lesson();
        value.title = "  ".into();
        assert!(value.validate().is_err());
        value = lesson();
        value.questions.clear();
        assert!(value.validate().is_err());
        value = lesson();
        value.questions.extend(vec![value.questions[0].clone(); 20]);
        assert!(value.validate().is_err());
        value = lesson();
        value.questions.push(value.questions[0].clone());
        assert!(value.validate().is_err());
        value = lesson();
        value.questions[0].id = "../escape".into();
        assert!(value.validate().is_err());
        value = lesson();
        value.questions[0].text = "🦀".repeat(1_025);
        assert!(value.validate().is_err());
        value = lesson();
        value.questions[0].text = " ".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn session_contract_is_distinct_from_chat_session() {
        let session = TutorSessionDto {
            id: Uuid::new_v4(),
            project_id: "project-1".into(),
            lesson_id: Uuid::new_v4(),
            status: TutorSessionStatus::InProgress,
            phase: TutorPhase::Theory,
            question_index: 0,
        };
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["status"], "in_progress");
        assert_eq!(json["phase"], "theory");
        assert_eq!(
            serde_json::from_value::<TutorSessionDto>(json).unwrap(),
            session
        );
    }
}
