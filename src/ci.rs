//! Provider-neutral CI/CD inspection with GitHub Actions and GitLab CI MVP support.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;

use crate::{
    AppError,
    tools::{Tool, ToolContext, ToolResult},
};

#[derive(Debug, Default)]
pub struct CiStatus;

#[async_trait]
impl Tool for CiStatus {
    fn name(&self) -> &'static str {
        "ci_status"
    }
    fn description(&self) -> &'static str {
        "Inspect local CI configuration and report provider-neutral status."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }
    async fn execute(&self, _args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let root = &context.working_dir;
        let mut found = Vec::new();
        if root.join(".github/workflows").is_dir() {
            found.push("github-actions: configured".to_owned());
        }
        if root.join(".gitlab-ci.yml").is_file() {
            found.push("gitlab-ci: configured".to_owned());
        }
        if root.join("Jenkinsfile").is_file() {
            found.push("jenkins: configured".to_owned());
        }
        if found.is_empty() {
            found.push("no supported CI configuration found".to_owned());
        }
        Ok(ToolResult::success(found.join("\n")))
    }
}

#[derive(Debug, Default)]
pub struct CiFailureAnalysis;

#[async_trait]
impl Tool for CiFailureAnalysis {
    fn name(&self) -> &'static str {
        "ci_failure_analysis"
    }
    fn description(&self) -> &'static str {
        "Read bounded CI logs or text and summarize likely failure causes."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let raw = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("ci_failure_analysis требует path".to_owned()))?;
        let path = context
            .working_dir
            .join(raw)
            .canonicalize()
            .map_err(|error| AppError::Tool(error.to_string()))?;
        if !path.starts_with(&context.working_dir) {
            return Err(AppError::PathOutsideWorkingDirectory(
                path.display().to_string(),
            ));
        }
        let text = fs::read_to_string(path).map_err(|error| AppError::Tool(error.to_string()))?;
        let mut lines = text
            .lines()
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                lower.contains("error")
                    || lower.contains("failed")
                    || lower.contains("panic")
                    || lower.contains("exception")
            })
            .take(50)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push("No obvious failure markers found.");
        }
        Ok(ToolResult::success(lines.join("\n")))
    }
}

pub fn register(registry: &mut crate::tools::ToolRegistry) -> Result<(), AppError> {
    registry.register(CiStatus)?;
    registry.register(CiFailureAnalysis)?;
    Ok(())
}
pub const TOOLS: &[&str] = &["ci_status", "ci_failure_analysis"];
