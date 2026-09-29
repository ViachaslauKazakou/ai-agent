//! Minimal MCP-over-stdio client and the bundled local MCP server.

use crate::AppError;
use crate::tools::{Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Allowlist for local document access. The extension is checked only after
/// canonicalization and workspace-boundary validation.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "toml", "yml", "yaml", "txt", "json", "md", "doc", "docx", "pdf",
];

#[derive(Clone)]
pub struct McpTool {
    name: &'static str,
    description: &'static str,
    schema: Value,
}

impl McpTool {
    /// Build the adapter for bounded local document reading.
    pub fn file_reader() -> Self {
        Self {
            name: "mcp_read_local_file",
            description: "Read an allowed local document through the bundled MCP server.",
            schema: json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}),
        }
    }

    /// Build the adapter for web search, fetching, and optional browser tabs.
    pub fn web_search() -> Self {
        Self {
            name: "mcp_web_search",
            description: "Search the web, optionally open result URLs in browser tabs, and fetch their text.",
            schema: json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":10},"open_tabs":{"type":"boolean"},"urls":{"type":"array","items":{"type":"string"}}},"required":["query"],"additionalProperties":false}),
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, AppError> {
        call_server(self.name, args, context).await
    }
}

async fn call_server(
    name: &str,
    args: Value,
    context: &ToolContext,
) -> Result<ToolResult, AppError> {
    // The desktop executable is a Tauri process, not the CLI binary. Spawning
    // `current_exe --mcp-server` from it starts a second GUI process and can
    // make the desktop client disappear. Keep the MCP implementation shared,
    // but execute the local server handler in-process for both clients.
    let content =
        execute_server_tool_with_context(json!({"name": name, "arguments": args}), context).await?;
    Ok(ToolResult::success(content))
}

pub async fn run_server() -> Result<(), AppError> {
    // stdout is reserved for newline-delimited JSON-RPC; diagnostics belong on
    // stderr so they cannot corrupt the MCP protocol.
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?
    {
        let request: Value = serde_json::from_str(&line)
            .map_err(|e| AppError::Tool(format!("MCP request JSON: {e}")))?;
        let Some(id) = request.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "initialize" => Ok(
                json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"ai-agent-local","version":env!("CARGO_PKG_VERSION")}}),
            ),
            "tools/list" => Ok(
                json!({"tools":[{"name":"mcp_read_local_file","description":"Read an allowed local document.","inputSchema":McpTool::file_reader().schema},{"name":"mcp_web_search","description":"Search the web and open/fetch result pages.","inputSchema":McpTool::web_search().schema}]}),
            ),
            "tools/call" => execute_server_tool(request.get("params").cloned().unwrap_or_default())
                .await
                .map(|text| json!({"content":[{"type":"text","text":text}]})),
            _ => Err(AppError::Tool(format!("MCP method not found: {method}"))),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":error.to_string()}]}})
            }
        };
        stdout
            .write_all(response.to_string().as_bytes())
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        stdout
            .write_all(b"\n")
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        stdout
            .flush()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
    }
    Ok(())
}

async fn execute_server_tool(params: Value) -> Result<String, AppError> {
    let root = std::env::var_os("AI_AGENT_MCP_ROOT")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| AppError::Tool("MCP root не задан".into()))?;
    let mut context = ToolContext::new(root, false);
    context.web_search_provider = std::env::var("AI_AGENT_WEB_SEARCH_PROVIDER").ok();
    context.web_search_endpoint = std::env::var("AI_AGENT_WEB_SEARCH_ENDPOINT").ok();
    context.web_search_api_key = std::env::var("AI_AGENT_WEB_SEARCH_API_KEY").ok();
    execute_server_tool_with_context(params, &context).await
}

async fn execute_server_tool_with_context(
    params: Value,
    context: &ToolContext,
) -> Result<String, AppError> {
    match params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "mcp_read_local_file" => {
            read_local_file(
                params.get("arguments").cloned().unwrap_or_default(),
                context,
            )
            .await
        }
        "mcp_web_search" => {
            web_search(
                params.get("arguments").cloned().unwrap_or_default(),
                context,
            )
            .await
        }
        name => Err(AppError::Tool(format!("MCP tool not found: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::{ALLOWED_EXTENSIONS, McpTool};
    use crate::tools::{Tool, ToolContext};
    use serde_json::json;

    #[test]
    fn allows_only_document_extensions() {
        assert!(ALLOWED_EXTENSIONS.contains(&"pdf"));
        assert!(ALLOWED_EXTENSIONS.contains(&"docx"));
        assert!(!ALLOWED_EXTENSIONS.contains(&"exe"));
        assert!(!ALLOWED_EXTENSIONS.contains(&"png"));
    }

    #[tokio::test]
    async fn web_search_returns_validation_error_without_spawning_a_client_process() {
        let context = ToolContext::new(std::env::temp_dir(), false);
        let error = McpTool::web_search()
            .execute(json!({"query": "  "}), &context)
            .await
            .expect_err("blank query must be rejected");

        assert!(error.to_string().contains("query обязателен"));
    }
}

async fn read_local_file(args: Value, context: &ToolContext) -> Result<String, AppError> {
    // Canonicalization prevents `..` and symlink traversal outside the root.
    let root = &context.working_dir;
    let path = root.join(
        args.get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("path обязателен".into()))?,
    );
    let canonical = path
        .canonicalize()
        .map_err(|e| AppError::Tool(e.to_string()))?;
    if !canonical.starts_with(
        root.canonicalize()
            .map_err(|e| AppError::Tool(e.to_string()))?,
    ) {
        return Err(AppError::Tool("путь вне working_dir".into()));
    }
    let extension = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::Tool(format!(
            "расширение .{extension} не разрешено"
        )));
    }
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?;
    if metadata.len() > 5_000_000 {
        return Err(AppError::Tool("файл превышает лимит 5 MB".into()));
    }
    let text = match extension.as_str() {
        "docx" => extract_docx(&canonical).await?,
        "pdf" => command_text("pdftotext", &[canonical.to_string_lossy().as_ref(), "-"]).await?,
        "doc" => {
            command_text(
                "textutil",
                &[
                    "-convert",
                    "txt",
                    "-stdout",
                    canonical.to_string_lossy().as_ref(),
                ],
            )
            .await?
        }
        _ => tokio::fs::read_to_string(&canonical).await.map_err(|e| {
            AppError::Tool(format!(
                "только UTF-8 текст поддерживается для .{extension}: {e}"
            ))
        })?,
    };
    let start = args.get("start_line").and_then(Value::as_u64).unwrap_or(1) as usize;
    let end = args
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    Ok(text
        .lines()
        .enumerate()
        .skip(start.saturating_sub(1))
        .take_while(|(i, _)| end.is_none_or(|n| *i < n))
        .map(|(i, line)| format!("{}: {line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn extract_docx(path: &Path) -> Result<String, AppError> {
    let xml = command_text(
        "unzip",
        &["-p", path.to_string_lossy().as_ref(), "word/document.xml"],
    )
    .await?;
    Ok(xml
        .replace("</w:p>", "\n")
        .replace("<w:tab/>", "\t")
        .split('<')
        .map(|part| part.split('>').nth(1).unwrap_or(part))
        .collect::<String>())
}

async fn command_text(program: &str, args: &[&str]) -> Result<String, AppError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| AppError::Tool(format!("{program} недоступен: {e}")))?;
    if !output.status.success() {
        return Err(AppError::Tool(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn web_search(args: Value, context: &ToolContext) -> Result<String, AppError> {
    // Web pages are untrusted data. Opening browser tabs is opt-in only.
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .filter(|q| !q.trim().is_empty())
        .ok_or_else(|| AppError::Tool("query обязателен".into()))?;
    let max = args
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 10);
    let provider = context
        .web_search_provider
        .as_deref()
        .unwrap_or("duckduckgo");
    let links = match provider {
        "duckduckgo" => duckduckgo_search(query, max).await?,
        "tavily" => tavily_search(query, max, context).await?,
        other => {
            return Err(AppError::Tool(format!(
                "неподдерживаемый WEB_SEARCH_PROVIDER: {other}"
            )));
        }
    };
    if args
        .get("open_tabs")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        for url in &links {
            open_url(url).await?;
        }
    }
    let requested = args
        .get("urls")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(3)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut output = format!(
        "Search results for: {query}\n{}",
        links
            .iter()
            .enumerate()
            .map(|(i, url)| format!("{}. {url}", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    );
    for url in requested {
        let text = reqwest::get(url)
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?
            .text()
            .await
            .map_err(|e| AppError::Tool(e.to_string()))?;
        output.push_str(&format!(
            "\n\nURL: {url}\n{}",
            text.chars().take(5000).collect::<String>()
        ));
    }
    Ok(output)
}

async fn duckduckgo_search(query: &str, max: u64) -> Result<Vec<String>, AppError> {
    let html = reqwest::Client::new()
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?
        .text()
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?;
    Ok(html
        .split("result__a")
        .skip(1)
        .take(max as usize)
        .filter_map(|fragment| {
            fragment
                .split("href=\"")
                .nth(1)
                .and_then(|v| v.split('"').next())
                .map(str::to_owned)
        })
        .collect())
}

async fn tavily_search(
    query: &str,
    max: u64,
    context: &ToolContext,
) -> Result<Vec<String>, AppError> {
    let key = context
        .web_search_api_key
        .clone()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Tool("Tavily требует WEB_SEARCH_API_KEY".into()))?;
    let endpoint = context
        .web_search_endpoint
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://api.tavily.com/search".into());
    let response: Value = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({"api_key":key,"query":query,"max_results":max}))
        .send()
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?
        .json()
        .await
        .map_err(|e| AppError::Tool(e.to_string()))?;
    Ok(response
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|item| item.get("url").and_then(Value::as_str).map(str::to_owned))
        .collect())
}

async fn open_url(url: &str) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    command
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AppError::Tool(format!("браузер недоступен: {e}")))?;
    Ok(())
}
