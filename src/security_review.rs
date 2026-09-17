//! Deterministic security and quality review findings.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;

use crate::{
    AppError,
    tools::{Tool, ToolContext, ToolResult},
};

#[derive(Debug, Default)]
pub struct SecurityReview;

#[async_trait]
impl Tool for SecurityReview {
    fn name(&self) -> &'static str {
        "security_review"
    }
    fn description(&self) -> &'static str {
        "Scan project source for common security and quality risks."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"scope":{"type":"string"}},"additionalProperties":false})
    }
    async fn execute(&self, _args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let mut findings = Vec::new();
        let mut stack = vec![context.working_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir)
                .map_err(|error| AppError::Tool(error.to_string()))?
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    if !matches!(
                        entry.file_name().to_string_lossy().as_ref(),
                        ".git" | "target" | "node_modules" | ".agent"
                    ) {
                        stack.push(path);
                    }
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                for (line_no, line) in text.lines().enumerate() {
                    let lower = line.to_ascii_lowercase();
                    let patterns = [
                        (
                            "high",
                            "secret marker",
                            lower.contains("ghp_")
                                || lower.contains("private_key")
                                || lower.contains("client_secret"),
                        ),
                        (
                            "high",
                            "shell command construction",
                            lower.contains("command::new")
                                && (lower.contains("user") || lower.contains("input")),
                        ),
                        (
                            "medium",
                            "possible SQL string concatenation",
                            lower.contains("select ") && lower.contains("+ "),
                        ),
                        (
                            "medium",
                            "unsafe deserialization",
                            lower.contains("pickle") || lower.contains("unsafe"),
                        ),
                        (
                            "low",
                            "TODO/FIXME quality gap",
                            lower.contains("todo") || lower.contains("fixme"),
                        ),
                    ];
                    for (severity, rule, matched) in patterns {
                        if matched {
                            findings.push(format!(
                                "{severity}: {rule}: {}:{}",
                                path.strip_prefix(&context.working_dir)
                                    .unwrap_or(&path)
                                    .display(),
                                line_no + 1
                            ));
                        }
                    }
                }
            }
        }
        if findings.is_empty() {
            findings.push("OK: known security and quality patterns not found".to_owned());
        }
        Ok(ToolResult::success(findings.join("\n")))
    }
}

pub fn register(registry: &mut crate::tools::ToolRegistry) -> Result<(), AppError> {
    registry.register(SecurityReview)?;
    Ok(())
}
pub const TOOLS: &[&str] = &["security_review"];
