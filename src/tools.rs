//! Безопасный MVP-набор инструментов агента.

use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::index::ProjectIndex;
use crate::{AppError, ToolDefinition};

/// Контекст вызова инструмента.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub allow_write: bool,
    pub confirm_writes: bool,
    pub command_allowlist: Vec<String>,
    pub interactive: bool,
    pub max_file_bytes: usize,
    pub max_result_bytes: usize,
    /// Read-only Microsoft Graph connection settings; token is never serialized.
    pub graph_base_url: Option<String>,
    pub graph_access_token: Option<String>,
    pub gmail_client_id: Option<String>,
}

impl ToolContext {
    pub fn new(working_dir: impl Into<PathBuf>, allow_write: bool) -> Self {
        Self {
            working_dir: working_dir.into(),
            allow_write,
            confirm_writes: false,
            command_allowlist: Vec::new(),
            interactive: false,
            max_file_bytes: 1_000_000,
            max_result_bytes: 50_000,
            graph_base_url: None,
            graph_access_token: None,
            gmail_client_id: None,
        }
    }

    fn resolve_existing(&self, raw: &str) -> Result<PathBuf, AppError> {
        let path = self.working_dir.join(raw);
        let canonical = path
            .canonicalize()
            .map_err(|error| AppError::Tool(format!("{}: {error}", path.display())))?;
        ensure_inside(&self.working_dir, &canonical)?;
        Ok(canonical)
    }

    fn resolve_new(&self, raw: &str) -> Result<PathBuf, AppError> {
        let path = self.working_dir.join(raw);
        let parent = path
            .parent()
            .ok_or_else(|| AppError::Tool("нет родительского каталога".to_owned()))?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|error| AppError::Tool(error.to_string()))?;
        ensure_inside(&self.working_dir, &canonical_parent)?;
        Ok(canonical_parent.join(
            path.file_name()
                .ok_or_else(|| AppError::Tool("некорректное имя файла".to_owned()))?,
        ))
    }
}

fn ensure_inside(root: &Path, path: &Path) -> Result<(), AppError> {
    let root = root
        .canonicalize()
        .map_err(|error| AppError::Tool(error.to_string()))?;
    if !path.starts_with(&root) {
        return Err(AppError::PathOutsideWorkingDirectory(
            path.display().to_string(),
        ));
    }
    Ok(())
}

/// Результат выполнения инструмента.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
    pub structured: Option<Value>,
    pub truncated: bool,
    /// Результат нужен модели, но не должен попадать в persistent session history.
    pub ephemeral: bool,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
            structured: None,
            truncated: false,
            ephemeral: false,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            success: false,
            content: error.into(),
            structured: None,
            truncated: false,
            ephemeral: false,
        }
    }
}

/// Общий контракт инструмента.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError>;
}

/// Реестр доступных инструментов.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> Result<(), AppError> {
        let name = tool.name();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(AppError::Tool(format!(
                "имя инструмента должно быть snake_case: {name}"
            )));
        }
        if self.tools.insert(name.to_owned(), Arc::new(tool)).is_some() {
            return Err(AppError::Tool(format!(
                "инструмент уже зарегистрирован: {name}"
            )));
        }
        Ok(())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| {
                ToolDefinition::function(tool.name(), tool.description(), tool.parameters_schema())
            })
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        context: &ToolContext,
    ) -> Result<ToolResult, AppError> {
        self.tools
            .get(name)
            .ok_or_else(|| AppError::UnknownTool(name.to_owned()))?
            .execute(args, context)
            .await
    }
}

#[derive(Debug, Default)]
pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a UTF-8 text file inside the working directory."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("read_file требует строковый path".to_owned()))?;
        let path = context.resolve_existing(path)?;
        let metadata = fs::metadata(&path).map_err(|e| AppError::Tool(e.to_string()))?;
        if !metadata.is_file() {
            return Err(AppError::Tool("путь не является обычным файлом".to_owned()));
        }
        if metadata.len() as usize > context.max_file_bytes {
            return Err(AppError::Tool("файл превышает лимит размера".to_owned()));
        }
        let content = fs::read_to_string(&path).map_err(|e| AppError::Tool(e.to_string()))?;
        let start = args.get("start_line").and_then(Value::as_u64).unwrap_or(1) as usize;
        let end = args
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let mut result = String::new();
        for (index, line) in content.lines().enumerate().skip(start.saturating_sub(1)) {
            let line_number = index + 1;
            if end.is_some_and(|end| line_number > end) {
                break;
            }
            result.push_str(&format!("{line_number}: {line}\n"));
        }
        let truncated = result.len() > context.max_result_bytes;
        if truncated {
            result.truncate(context.max_result_bytes);
        }
        Ok(ToolResult {
            success: true,
            content: result,
            structured: None,
            truncated,
            ephemeral: false,
        })
    }
}

#[derive(Debug, Default)]
pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &'static str {
        "list_directory"
    }
    fn description(&self) -> &'static str {
        "List entries in a directory inside the working directory."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":[],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = context.resolve_existing(raw)?;
        if !path.is_dir() {
            return Err(AppError::Tool("путь не является каталогом".to_owned()));
        }
        let mut entries = fs::read_dir(path)
            .map_err(|e| AppError::Tool(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        entries.sort();
        let mut result = entries.join("\n");
        result.push('\n');
        let truncated = result.len() > context.max_result_bytes;
        if truncated {
            result.truncate(context.max_result_bytes);
        }
        Ok(ToolResult {
            success: true,
            content: result,
            structured: None,
            truncated,
            ephemeral: false,
        })
    }
}

#[derive(Debug, Default)]
pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Write UTF-8 text to a file inside the working directory; requires --allow-write."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        if !context.allow_write {
            return Err(AppError::WriteConfirmationRequired);
        }
        if context.confirm_writes && context.interactive {
            print!("Разрешить запись файла? [y/N] ");
            io::stdout()
                .flush()
                .map_err(|error| AppError::Tool(error.to_string()))?;
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .map_err(|error| AppError::Tool(error.to_string()))?;
            if !matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "д" | "да"
            ) {
                return Err(AppError::Tool("запись отклонена пользователем".to_owned()));
            }
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("write_file требует path".to_owned()))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("write_file требует content".to_owned()))?;
        if content.len() > context.max_file_bytes {
            return Err(AppError::Tool(
                "записываемый текст превышает лимит".to_owned(),
            ));
        }
        let path = context.resolve_new(path)?;
        fs::write(&path, content).map_err(|e| AppError::Tool(e.to_string()))?;
        Ok(ToolResult::success(format!(
            "Записано {} байт в {}",
            content.len(),
            path.display()
        )))
    }
}

#[derive(Debug, Default)]
pub struct SearchFiles;

#[async_trait]
impl Tool for SearchFiles {
    fn name(&self) -> &'static str {
        "search_files"
    }
    fn description(&self) -> &'static str {
        "Recursively search text files inside the working directory."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string"}},"required":["query"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("search_files требует query".to_owned()))?;
        if query.is_empty() {
            return Err(AppError::Tool("query не может быть пустым".to_owned()));
        }
        let root =
            context.resolve_existing(args.get("path").and_then(Value::as_str).unwrap_or("."))?;
        let mut result = String::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).map_err(|e| AppError::Tool(e.to_string()))? {
                let entry = entry.map_err(|e| AppError::Tool(e.to_string()))?;
                let path = entry.path();
                let metadata =
                    fs::symlink_metadata(&path).map_err(|e| AppError::Tool(e.to_string()))?;
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !metadata.is_file() || metadata.len() as usize > context.max_file_bytes {
                    continue;
                }
                let Ok(content) = fs::read_to_string(&path) else {
                    continue;
                };
                for (line_no, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        result.push_str(&format!("{}:{}: {}\n", path.display(), line_no + 1, line));
                        if result.len() >= context.max_result_bytes {
                            result.truncate(context.max_result_bytes);
                            return Ok(ToolResult {
                                success: true,
                                content: result,
                                structured: None,
                                truncated: true,
                                ephemeral: false,
                            });
                        }
                    }
                }
            }
        }
        Ok(ToolResult {
            success: true,
            content: result,
            structured: None,
            truncated: false,
            ephemeral: false,
        })
    }
}

#[derive(Debug, Default)]
pub struct ProjectSearch;

#[async_trait]
impl Tool for ProjectSearch {
    fn name(&self) -> &'static str {
        "project_search"
    }

    fn description(&self) -> &'static str {
        "Search indexed project chunks by keywords and return relevant file fragments."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20}},"required":["query"],"additionalProperties":false})
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("project_search требует query".to_owned()))?;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let index_path = ProjectIndex::index_path(&context.working_dir);
        let mut index = match ProjectIndex::load(&index_path) {
            Ok(index) if index.root == context.working_dir => index,
            _ => ProjectIndex::build(&context.working_dir)
                .map_err(|error| AppError::Tool(error.to_string()))?,
        };
        index
            .update()
            .map_err(|error| AppError::Tool(error.to_string()))?;
        index
            .save(&index_path)
            .map_err(|error| AppError::Tool(error.to_string()))?;
        let content = index
            .search(query, limit)
            .into_iter()
            .map(|hit| {
                format!(
                    "{}:{}-{} (score={}):\n{}",
                    hit.path, hit.start_line, hit.end_line, hit.score, hit.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(ToolResult::success(content))
    }
}

#[derive(Debug, Default)]
pub struct ReadLines;

#[async_trait]
impl Tool for ReadLines {
    fn name(&self) -> &'static str {
        "read_lines"
    }
    fn description(&self) -> &'static str {
        "Read a selected line range from a UTF-8 file."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}},"required":["path","start_line","end_line"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let start = args
            .get("start_line")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::Tool("read_lines требует start_line".to_owned()))?;
        let end = args
            .get("end_line")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::Tool("read_lines требует end_line".to_owned()))?;
        if start == 0 || end < start {
            return Err(AppError::Tool("некорректный диапазон строк".to_owned()));
        }
        ReadFile
            .execute(
                json!({"path": args.get("path"), "start_line": start, "end_line": end}),
                context,
            )
            .await
    }
}

#[derive(Debug, Default)]
pub struct RunCommand;

#[async_trait]
impl Tool for RunCommand {
    fn name(&self) -> &'static str {
        "run_command"
    }
    fn description(&self) -> &'static str {
        "Run an explicitly allowlisted command in the working directory."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"},"args":{"type":"array","items":{"type":"string"}}},"required":["command"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("run_command требует command".to_owned()))?;
        if !context
            .command_allowlist
            .iter()
            .any(|allowed| allowed == command)
        {
            return Err(AppError::Tool("команда отсутствует в allowlist".to_owned()));
        }
        let args = args
            .get("args")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut child = tokio::process::Command::new(command);
        child
            .args(args.iter().filter_map(Value::as_str))
            .current_dir(&context.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = child
            .output()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
        content.push_str(&String::from_utf8_lossy(&output.stderr));
        let truncated = content.len() > context.max_result_bytes;
        if truncated {
            content.truncate(context.max_result_bytes);
        }
        Ok(ToolResult {
            success: output.status.success(),
            content,
            structured: None,
            truncated,
            ephemeral: false,
        })
    }
}

pub fn default_registry() -> Result<ToolRegistry, AppError> {
    let mut registry = ToolRegistry::new();
    registry.register(ReadFile)?;
    registry.register(ListDirectory)?;
    registry.register(WriteFile)?;
    registry.register(SearchFiles)?;
    registry.register(ReadLines)?;
    registry.register(ProjectSearch)?;
    registry.register(crate::connectors::tools::ListRecentEmails)?;
    registry.register(crate::connectors::tools::GetEmail)?;
    registry.register(crate::connectors::tools::SearchEmails)?;
    Ok(registry)
}

/// Создаёт registry из списка tools, заданного пользователем.
pub fn registry_from_names(names: &[String]) -> Result<ToolRegistry, AppError> {
    let mut registry = ToolRegistry::new();
    for name in names {
        match name.as_str() {
            "read_file" => registry.register(ReadFile)?,
            "list_directory" => registry.register(ListDirectory)?,
            "write_file" => registry.register(WriteFile)?,
            "search_files" => registry.register(SearchFiles)?,
            "read_lines" => registry.register(ReadLines)?,
            "project_search" => registry.register(ProjectSearch)?,
            "list_recent_emails" => {
                registry.register(crate::connectors::tools::ListRecentEmails)?
            }
            "get_email" => registry.register(crate::connectors::tools::GetEmail)?,
            "search_emails" => registry.register(crate::connectors::tools::SearchEmails)?,
            "run_command" => registry.register(RunCommand)?,
            unknown => return Err(AppError::UnknownTool(unknown.to_owned())),
        }
    }
    Ok(registry)
}
