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
use uuid::Uuid;

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

    fn validate_edit(&self, path: &Path, content: &str) -> Result<(), AppError> {
        if is_protected_path(path) {
            return Err(AppError::UnsafeEdit(format!(
                "запрещённый секретный или credential-файл: {}",
                path.display()
            )));
        }
        if let Some(secret) = find_secret(content) {
            return Err(AppError::UnsafeEdit(secret));
        }
        if find_git_root(&self.working_dir).is_none() {
            return Err(AppError::UnsafeEdit(
                "working_dir не находится внутри Git-репозитория".to_owned(),
            ));
        }
        Ok(())
    }

    fn ensure_mutation_allowed(&self, path: &Path, content: &str) -> Result<(), AppError> {
        self.validate_edit(path, content)?;
        if !self.allow_write {
            return Err(AppError::WriteConfirmationRequired);
        }
        Ok(())
    }
}

fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = path.canonicalize().ok()?;
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn is_protected_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.contains("token")
}

fn find_secret(content: &str) -> Option<String> {
    const MARKERS: &[&str] = &[
        "-----BEGIN ",
        "ghp_",
        "github_pat_",
        "sk-",
        "AKIA",
        "xoxb-",
        "access_token=",
        "client_secret=",
    ];
    MARKERS
        .iter()
        .find(|marker| content.contains(**marker))
        .map(|marker| format!("содержимое похоже на секрет (маркер `{marker}`)"))
}

fn checkpoint(path: &Path, content: &[u8]) -> Result<PathBuf, AppError> {
    let root = path
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .ok_or_else(|| AppError::UnsafeEdit("Git-репозиторий не найден".to_owned()))?;
    let dir = root.join(".agent").join("checkpoints");
    fs::create_dir_all(&dir).map_err(|error| AppError::Tool(error.to_string()))?;
    let backup = dir.join(format!(
        "{}-{}.bak",
        Uuid::new_v4(),
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&backup, content).map_err(|error| AppError::Tool(error.to_string()))?;
    fs::write(
        dir.join("latest.json"),
        json!({"path": path, "backup": backup}).to_string(),
    )
    .map_err(|error| AppError::Tool(error.to_string()))?;
    Ok(backup)
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

fn git_root(context: &ToolContext) -> Result<PathBuf, AppError> {
    find_git_root(&context.working_dir)
        .ok_or_else(|| AppError::UnsafeEdit("working_dir не является Git-репозиторием".to_owned()))
}

fn confirm_action(context: &ToolContext, prompt: &str) -> Result<(), AppError> {
    if !context.interactive || !context.confirm_writes {
        return Err(AppError::WriteConfirmationRequired);
    }
    print!("{prompt} [y/N] ");
    io::stdout()
        .flush()
        .map_err(|error| AppError::Tool(error.to_string()))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| AppError::Tool(error.to_string()))?;
    if matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "д" | "да"
    ) {
        Ok(())
    } else {
        Err(AppError::Tool(
            "операция отклонена пользователем".to_owned(),
        ))
    }
}

async fn run_git(context: &ToolContext, args: &[&str]) -> Result<String, AppError> {
    let root = git_root(context)?;
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| AppError::Tool(format!("git недоступен: {error}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(AppError::Tool(format!(
            "git {}: {}",
            args.join(" "),
            text.trim()
        )));
    }
    if let Some(secret) = find_secret(&text) {
        return Err(AppError::UnsafeEdit(format!(
            "вывод git содержит секрет: {secret}"
        )));
    }
    Ok(text)
}

fn truncate_result(mut text: String, context: &ToolContext) -> ToolResult {
    let truncated = text.len() > context.max_result_bytes;
    if truncated {
        text.truncate(context.max_result_bytes);
    }
    ToolResult {
        success: true,
        content: text,
        structured: None,
        truncated,
        ephemeral: false,
    }
}

#[derive(Debug, Clone, Copy)]
enum GitAction {
    Status,
    Diff,
    Log,
    Branch,
    PrepareCommit,
    Commit,
    Push,
    Pr,
}

macro_rules! git_tool {
    ($type:ident, $name:literal, $description:literal, $action:ident, $schema:expr) => {
        #[derive(Debug, Default)]
        pub struct $type;
        #[async_trait]
        impl Tool for $type {
            fn name(&self) -> &'static str {
                $name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn parameters_schema(&self) -> Value {
                $schema
            }
            async fn execute(
                &self,
                args: Value,
                context: &ToolContext,
            ) -> Result<ToolResult, AppError> {
                execute_git(GitAction::$action, args, context).await
            }
        }
    };
}

git_tool!(
    GitStatus,
    "git_status",
    "Show repository status in porcelain format.",
    Status,
    json!({"type":"object","properties":{},"additionalProperties":false})
);
git_tool!(
    GitDiff,
    "git_diff",
    "Show the repository diff without changing files.",
    Diff,
    json!({"type":"object","properties":{"staged":{"type":"boolean"}},"additionalProperties":false})
);
git_tool!(
    GitLog,
    "git_log",
    "Show recent repository commits.",
    Log,
    json!({"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false})
);
git_tool!(
    GitCreateBranch,
    "git_create_branch",
    "Create and switch to a new Git branch after confirmation.",
    Branch,
    json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false})
);
git_tool!(
    GitPrepareCommit,
    "git_prepare_commit",
    "Prepare a commit summary and detect secrets in staged changes.",
    PrepareCommit,
    json!({"type":"object","properties":{},"additionalProperties":false})
);
git_tool!(
    GitCommit,
    "git_commit",
    "Create a commit only after explicit confirmation.",
    Commit,
    json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false})
);
git_tool!(
    GitPush,
    "git_push",
    "Push the current branch only after explicit confirmation.",
    Push,
    json!({"type":"object","properties":{"remote":{"type":"string"},"branch":{"type":"string"}},"additionalProperties":false})
);
git_tool!(
    GitCreatePr,
    "git_create_pr",
    "Create a GitHub Pull Request through gh after explicit confirmation.",
    Pr,
    json!({"type":"object","properties":{"title":{"type":"string"},"body":{"type":"string"},"base":{"type":"string"}},"required":["title","body"],"additionalProperties":false})
);

async fn execute_git(
    action: GitAction,
    args: Value,
    context: &ToolContext,
) -> Result<ToolResult, AppError> {
    let result = match action {
        GitAction::Status => run_git(context, &["status", "--short", "--branch"]).await?,
        GitAction::Diff => {
            if args.get("staged").and_then(Value::as_bool).unwrap_or(false) {
                run_git(context, &["diff", "--cached"]).await?
            } else {
                run_git(context, &["diff"]).await?
            }
        }
        GitAction::Log => {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 50)
                .to_string();
            run_git(context, &["log", "-n", &limit, "--oneline", "--decorate"]).await?
        }
        GitAction::Branch => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Tool("git_create_branch требует name".to_owned()))?;
            validate_branch_name(name)?;
            confirm_action(
                context,
                &format!("Создать и переключиться на ветку `{name}`?"),
            )?;
            run_git(context, &["switch", "-c", name]).await?
        }
        GitAction::PrepareCommit => {
            let diff = run_git(context, &["diff", "--cached"]).await?;
            if diff.trim().is_empty() {
                return Err(AppError::Tool("нет staged изменений для commit".to_owned()));
            }
            format!("Staged diff готов к commit:\n{diff}")
        }
        GitAction::Commit => {
            let message = args
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Tool("git_commit требует message".to_owned()))?;
            if message.trim().is_empty() || message.contains('\n') {
                return Err(AppError::Tool(
                    "commit message должен быть непустым однострочным текстом".to_owned(),
                ));
            }
            let staged = run_git(context, &["diff", "--cached"]).await?;
            if staged.trim().is_empty() {
                return Err(AppError::Tool("нет staged изменений для commit".to_owned()));
            }
            if let Some(secret) = find_secret(&staged) {
                return Err(AppError::UnsafeEdit(format!(
                    "commit заблокирован: {secret}"
                )));
            }
            confirm_action(context, &format!("Создать commit `{message}`?"))?;
            run_git(context, &["commit", "-m", message]).await?
        }
        GitAction::Push => {
            let remote = args
                .get("remote")
                .and_then(Value::as_str)
                .unwrap_or("origin");
            let branch = args.get("branch").and_then(Value::as_str).unwrap_or("HEAD");
            confirm_action(
                context,
                &format!("Отправить изменения в `{remote}/{branch}`?"),
            )?;
            run_git(context, &["push", remote, branch]).await?
        }
        GitAction::Pr => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Tool("git_create_pr требует title".to_owned()))?;
            let body = args
                .get("body")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Tool("git_create_pr требует body".to_owned()))?;
            let base = args.get("base").and_then(Value::as_str).unwrap_or("main");
            if let Some(secret) = find_secret(&format!("{title}\n{body}")) {
                return Err(AppError::UnsafeEdit(format!("PR заблокирован: {secret}")));
            }
            confirm_action(
                context,
                &format!("Создать Pull Request `{title}` в `{base}`?"),
            )?;
            let root = git_root(context)?;
            let output = tokio::process::Command::new("gh")
                .args([
                    "pr", "create", "--base", base, "--title", title, "--body", body,
                ])
                .current_dir(root)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|error| AppError::Tool(format!("gh недоступен: {error}")))?;
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if !output.status.success() {
                return Err(AppError::Tool(format!("gh pr create: {}", text.trim())));
            }
            text
        }
    };
    Ok(truncate_result(result, context))
}

fn validate_branch_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name.starts_with('-')
        || name.contains("..")
        || name.contains(' ')
        || name.contains('~')
        || name.contains('^')
        || name.contains(':')
        || name.contains('?')
        || name.contains('*')
        || name.contains('[')
        || name.ends_with('/')
    {
        return Err(AppError::Tool("некорректное имя Git-ветки".to_owned()));
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
        context.ensure_mutation_allowed(&path, content)?;
        let previous = fs::read(&path).unwrap_or_default();
        let backup = checkpoint(&path, &previous)?;
        fs::write(&path, content).map_err(|e| AppError::Tool(e.to_string()))?;
        Ok(ToolResult::success(format!(
            "Изменение применено после checkpoint {}: записано {} байт в {}",
            backup.display(),
            content.len(),
            path.display()
        )))
    }
}

#[derive(Debug, Default)]
pub struct ApplyPatch;

#[async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Preview and safely apply a single text replacement; apply=false only shows the diff."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"apply":{"type":"boolean"}},"required":["path","old_text","new_text"],"additionalProperties":false})
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        let raw = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("apply_patch требует path".to_owned()))?;
        let old = args
            .get("old_text")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("apply_patch требует old_text".to_owned()))?;
        let new = args
            .get("new_text")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("apply_patch требует new_text".to_owned()))?;
        if old.is_empty() {
            return Err(AppError::Tool("old_text не может быть пустым".to_owned()));
        }
        let path = context.resolve_existing(raw)?;
        let current =
            fs::read_to_string(&path).map_err(|error| AppError::Tool(error.to_string()))?;
        let occurrences = current.matches(old).count();
        if occurrences != 1 {
            return Err(AppError::UnsafeEdit(format!(
                "ожидалось ровно одно совпадение, найдено {occurrences}"
            )));
        }
        let updated = current.replacen(old, new, 1);
        context.validate_edit(&path, &updated)?;
        let diff = unified_diff(&path, &current, &updated);
        if !args.get("apply").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(ToolResult::success(format!(
                "Предпросмотр diff (изменение НЕ применено):\n{diff}"
            )));
        }
        if !context.interactive || !context.confirm_writes {
            return Err(AppError::WriteConfirmationRequired);
        }
        print!("Применить этот diff? [y/N] ");
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
            return Err(AppError::Tool("patch отклонён пользователем".to_owned()));
        }
        let backup = checkpoint(&path, current.as_bytes())?;
        fs::write(&path, updated).map_err(|error| AppError::Tool(error.to_string()))?;
        Ok(ToolResult::success(format!(
            "Diff применён. Checkpoint: {}\n{diff}",
            backup.display()
        )))
    }
}

#[derive(Debug, Default)]
pub struct RollbackLastChange;

#[async_trait]
impl Tool for RollbackLastChange {
    fn name(&self) -> &'static str {
        "rollback_last_change"
    }
    fn description(&self) -> &'static str {
        "Restore the last safe-edit checkpoint."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }
    async fn execute(&self, _args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        if !context.allow_write {
            return Err(AppError::WriteConfirmationRequired);
        }
        if context.confirm_writes && context.interactive {
            print!("Откатить последнее изменение? [y/N] ");
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
                return Err(AppError::Tool("откат отклонён пользователем".to_owned()));
            }
        }
        let root = find_git_root(&context.working_dir)
            .ok_or_else(|| AppError::UnsafeEdit("Git-репозиторий не найден".to_owned()))?;
        let value: Value = serde_json::from_str(
            &fs::read_to_string(root.join(".agent/checkpoints/latest.json"))
                .map_err(|error| AppError::Tool(error.to_string()))?,
        )
        .map_err(|error| AppError::Tool(error.to_string()))?;
        let path = context.resolve_existing(
            value["path"]
                .as_str()
                .ok_or_else(|| AppError::Tool("checkpoint повреждён".to_owned()))?,
        )?;
        let backup = PathBuf::from(
            value["backup"]
                .as_str()
                .ok_or_else(|| AppError::Tool("checkpoint повреждён".to_owned()))?,
        );
        let content = fs::read(&backup).map_err(|error| AppError::Tool(error.to_string()))?;
        fs::write(&path, content).map_err(|error| AppError::Tool(error.to_string()))?;
        Ok(ToolResult::success(format!(
            "Последнее изменение отменено: {}",
            path.display()
        )))
    }
}

fn unified_diff(path: &Path, before: &str, after: &str) -> String {
    let mut diff = format!("--- a/{}\n+++ b/{}\n", path.display(), path.display());
    for line in before.lines() {
        diff.push_str(&format!("-{}\n", line));
    }
    for line in after.lines() {
        diff.push_str(&format!("+{}\n", line));
    }
    diff
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn detects_secret_markers_and_protected_names() {
        assert!(find_secret("token=ghp_example").is_some());
        assert!(is_protected_path(Path::new(".env")));
        assert!(is_protected_path(Path::new("client_credentials.json")));
        assert!(!is_protected_path(Path::new("src/main.rs")));
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
    crate::project_intelligence::register(&mut registry)?;
    crate::security_review::register(&mut registry)?;
    crate::ci::register(&mut registry)?;
    registry.register(ApplyPatch)?;
    registry.register(RollbackLastChange)?;
    registry.register(GitStatus)?;
    registry.register(GitDiff)?;
    registry.register(GitLog)?;
    registry.register(GitCreateBranch)?;
    registry.register(GitPrepareCommit)?;
    registry.register(GitCommit)?;
    registry.register(GitPush)?;
    registry.register(GitCreatePr)?;
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
            "project_symbols" => registry.register(crate::project_intelligence::SymbolIndex)?,
            "project_diagnostics" => {
                registry.register(crate::project_intelligence::ProjectDiagnostics)?
            }
            "project_definition" => {
                registry.register(crate::project_intelligence::ProjectDefinitions)?
            }
            "security_review" => registry.register(crate::security_review::SecurityReview)?,
            "ci_status" => registry.register(crate::ci::CiStatus)?,
            "ci_failure_analysis" => registry.register(crate::ci::CiFailureAnalysis)?,
            "apply_patch" => registry.register(ApplyPatch)?,
            "rollback_last_change" => registry.register(RollbackLastChange)?,
            "git_status" => registry.register(GitStatus)?,
            "git_diff" => registry.register(GitDiff)?,
            "git_log" => registry.register(GitLog)?,
            "git_create_branch" => registry.register(GitCreateBranch)?,
            "git_prepare_commit" => registry.register(GitPrepareCommit)?,
            "git_commit" => registry.register(GitCommit)?,
            "git_push" => registry.register(GitPush)?,
            "git_create_pr" => registry.register(GitCreatePr)?,
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
