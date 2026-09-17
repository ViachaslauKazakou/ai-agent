//! Lightweight project intelligence and LSP-adjacent tools.

use std::{path::Path, process::Stdio};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    AppError,
    tools::{Tool, ToolContext, ToolResult},
};

fn language(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("py") => "python",
        _ => "unknown",
    }
}

async fn command(context: &ToolContext, program: &str, args: &[&str]) -> Result<String, AppError> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(&context.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| AppError::Tool(format!("{program} недоступен: {error}")))?;
    let mut result = String::from_utf8_lossy(&output.stdout).into_owned();
    result.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(result)
}

#[derive(Debug, Default)]
pub struct SymbolIndex;

#[async_trait]
impl Tool for SymbolIndex {
    fn name(&self) -> &'static str {
        "project_symbols"
    }
    fn description(&self) -> &'static str {
        "List lightweight symbols, definitions and references in source files."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":200}},"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(200) as usize;
        let mut entries = Vec::new();
        let mut stack = vec![context.working_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir)
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
                if language(&path) == "unknown" {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (line_no, line) in text.lines().enumerate() {
                    let trimmed = line.trim_start();
                    let is_symbol = trimmed.starts_with("fn ")
                        || trimmed.starts_with("pub fn ")
                        || trimmed.starts_with("struct ")
                        || trimmed.starts_with("pub struct ")
                        || trimmed.starts_with("enum ")
                        || trimmed.starts_with("class ")
                        || trimmed.starts_with("def ")
                        || trimmed.starts_with("function ")
                        || trimmed.contains("interface ");
                    if is_symbol && (query.is_empty() || line.to_ascii_lowercase().contains(&query))
                    {
                        let relative = path
                            .strip_prefix(&context.working_dir)
                            .unwrap_or(&path)
                            .display();
                        entries.push(format!("{}:{}: {}", relative, line_no + 1, line.trim()));
                        if entries.len() >= limit {
                            break;
                        }
                    }
                }
                if entries.len() >= limit {
                    break;
                }
            }
        }
        entries.sort();
        Ok(ToolResult::success(entries.join("\n")))
    }
}

#[derive(Debug, Default)]
pub struct ProjectDiagnostics;

#[async_trait]
impl Tool for ProjectDiagnostics {
    fn name(&self) -> &'static str {
        "project_diagnostics"
    }
    fn description(&self) -> &'static str {
        "Run the project's read-only compiler/checker diagnostics."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"language":{"type":"string"}},"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let selected = args
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let language = if selected == "auto" { "rust" } else { selected };
        let result = match language {
            "rust" => command(context, "cargo", &["check", "--message-format=short"]).await?,
            "typescript" | "javascript" => {
                command(context, "npx", &["--no-install", "tsc", "--noEmit"]).await?
            }
            "python" => command(context, "python", &["-m", "compileall", "-q", "."]).await?,
            other => {
                return Err(AppError::Tool(format!(
                    "неподдерживаемый язык diagnostics: {other}"
                )));
            }
        };
        Ok(ToolResult::success(result))
    }
}

#[derive(Debug, Default)]
pub struct ProjectDefinitions;

#[async_trait]
impl Tool for ProjectDefinitions {
    fn name(&self) -> &'static str {
        "project_definition"
    }
    fn description(&self) -> &'static str {
        "Find definitions and references for a symbol."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"symbol":{"type":"string"}},"required":["symbol"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let symbol = args
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("project_definition требует symbol".to_owned()))?;
        let result = command(
            context,
            "git",
            &["grep", "-n", "-I", symbol, "--", ":(exclude).git"],
        )
        .await?;
        Ok(ToolResult::success(result))
    }
}

pub fn register(registry: &mut crate::tools::ToolRegistry) -> Result<(), AppError> {
    registry.register(SymbolIndex)?;
    registry.register(ProjectDiagnostics)?;
    registry.register(ProjectDefinitions)?;
    Ok(())
}

pub const TOOLS: &[&str] = &[
    "project_symbols",
    "project_diagnostics",
    "project_definition",
];
