//! Read-only Git hosting connector.
//!
//! The provider contract deliberately contains only safe read operations.  A
//! future GitLab implementation can reuse the normalized models without
//! exposing GitHub-specific HTTP details to tools or the agent.

use crate::AppError;
use crate::tools::{Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{process::Stdio, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub default_branch: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: Option<String>,
    pub user: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: Option<String>,
    pub user: Option<String>,
    pub head: Option<String>,
    pub base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub body: String,
    pub user: Option<String>,
    pub html_url: Option<String>,
}

#[async_trait]
pub trait GitHostingProvider: Send + Sync {
    async fn repository(&self, repository: &str) -> Result<Repository, AppError>;
    async fn list_issues(
        &self,
        repository: &str,
        state: &str,
        limit: usize,
    ) -> Result<Vec<Issue>, AppError>;
    async fn issue(&self, repository: &str, number: u64) -> Result<Issue, AppError>;
    async fn issue_comments(
        &self,
        repository: &str,
        number: u64,
        limit: usize,
    ) -> Result<Vec<Comment>, AppError>;
    async fn list_pull_requests(
        &self,
        repository: &str,
        state: &str,
        limit: usize,
    ) -> Result<Vec<PullRequest>, AppError>;
}

#[derive(Clone)]
pub struct GitHubProvider {
    client: Client,
    token: String,
    base_url: String,
}

impl GitHubProvider {
    /// Uses an explicitly supplied token, or obtains one through `gh auth`.
    /// The token is kept only in this in-memory client and is never serialized.
    pub async fn from_environment(timeout: Duration) -> Result<Self, AppError> {
        let token = match std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            Some(token) => token,
            None => {
                let output = tokio::process::Command::new("gh")
                    .args(["auth", "token"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .map_err(|_| AppError::GithubAuth("GitHub authentication is unavailable; run `gh auth login` or set GITHUB_TOKEN".into()))?;
                if !output.status.success() {
                    return Err(AppError::GithubAuth("GitHub authentication is unavailable; run `gh auth login` or set GITHUB_TOKEN".into()));
                }
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            }
        };
        if token.is_empty() {
            return Err(AppError::GithubAuth(
                "GitHub token is empty; run `gh auth login`".into(),
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .user_agent("ai-agent")
            .build()
            .map_err(|_| AppError::Github("failed to create GitHub client".into()))?;
        Ok(Self {
            client,
            token,
            base_url: "https://api.github.com".into(),
        })
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, AppError> {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| AppError::Github(format!("network error: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(AppError::GithubHttp {
                status: status.as_u16(),
                message: redact(message),
            });
        }
        response
            .json()
            .await
            .map_err(|e| AppError::Github(format!("invalid response: {e}")))
    }
}

fn redact(mut value: String) -> String {
    for marker in ["ghp_", "github_pat_"] {
        if let Some(start) = value.find(marker) {
            value.replace_range(start.., "<redacted>");
        }
    }
    value.chars().take(500).collect()
}

#[derive(Deserialize)]
struct ApiRepository {
    name: String,
    full_name: String,
    default_branch: Option<String>,
    html_url: Option<String>,
}
#[derive(Deserialize)]
struct ApiUser {
    login: String,
}
#[derive(Deserialize)]
struct ApiLabel {
    name: String,
}
#[derive(Deserialize)]
struct ApiIssue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: Option<String>,
    user: Option<ApiUser>,
    #[serde(default)]
    labels: Vec<ApiLabel>,
}
#[derive(Deserialize)]
struct ApiPullRequest {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: Option<String>,
    user: Option<ApiUser>,
    head: Option<ApiBranch>,
    base: Option<ApiBranch>,
}
#[derive(Deserialize)]
struct ApiBranch {
    r#ref: String,
}
#[derive(Deserialize)]
struct ApiComment {
    id: u64,
    body: String,
    user: Option<ApiUser>,
    html_url: Option<String>,
}

impl From<ApiIssue> for Issue {
    fn from(v: ApiIssue) -> Self {
        Self {
            number: v.number,
            title: v.title,
            body: v.body,
            state: v.state,
            html_url: v.html_url,
            user: v.user.map(|u| u.login),
            labels: v.labels.into_iter().map(|l| l.name).collect(),
        }
    }
}
impl From<ApiPullRequest> for PullRequest {
    fn from(v: ApiPullRequest) -> Self {
        Self {
            number: v.number,
            title: v.title,
            body: v.body,
            state: v.state,
            html_url: v.html_url,
            user: v.user.map(|u| u.login),
            head: v.head.map(|b| b.r#ref),
            base: v.base.map(|b| b.r#ref),
        }
    }
}
impl From<ApiComment> for Comment {
    fn from(v: ApiComment) -> Self {
        Self {
            id: v.id,
            body: v.body,
            user: v.user.map(|u| u.login),
            html_url: v.html_url,
        }
    }
}

#[async_trait]
impl GitHostingProvider for GitHubProvider {
    async fn repository(&self, repository: &str) -> Result<Repository, AppError> {
        self.get::<ApiRepository>(&format!("/repos/{repository}"))
            .await
            .map(|v| Repository {
                name: v.name,
                full_name: v.full_name,
                default_branch: v.default_branch,
                html_url: v.html_url,
            })
    }
    async fn list_issues(
        &self,
        repository: &str,
        state: &str,
        limit: usize,
    ) -> Result<Vec<Issue>, AppError> {
        self.get::<Vec<ApiIssue>>(&format!(
            "/repos/{repository}/issues?state={}&per_page={}",
            urlencoding::encode(state),
            limit.clamp(1, 100)
        ))
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
    }
    async fn issue(&self, repository: &str, number: u64) -> Result<Issue, AppError> {
        self.get::<ApiIssue>(&format!("/repos/{repository}/issues/{number}"))
            .await
            .map(Into::into)
    }
    async fn issue_comments(
        &self,
        repository: &str,
        number: u64,
        limit: usize,
    ) -> Result<Vec<Comment>, AppError> {
        self.get::<Vec<ApiComment>>(&format!(
            "/repos/{repository}/issues/{number}/comments?per_page={}",
            limit.clamp(1, 100)
        ))
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
    }
    async fn list_pull_requests(
        &self,
        repository: &str,
        state: &str,
        limit: usize,
    ) -> Result<Vec<PullRequest>, AppError> {
        self.get::<Vec<ApiPullRequest>>(&format!(
            "/repos/{repository}/pulls?state={}&per_page={}",
            urlencoding::encode(state),
            limit.clamp(1, 100)
        ))
        .await
        .map(|v| v.into_iter().map(Into::into).collect())
    }
}

fn limit(args: &serde_json::Value) -> usize {
    args.get("limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize
}

async fn provider() -> Result<GitHubProvider, AppError> {
    GitHubProvider::from_environment(Duration::from_secs(30)).await
}

fn result<T: Serialize>(value: &T) -> ToolResult {
    let content = serde_json::to_string_pretty(value).unwrap_or_default();
    ToolResult {
        success: true,
        content,
        structured: serde_json::to_value(value).ok(),
        truncated: false,
        ephemeral: true,
    }
}

pub struct GitHubRepository;
#[async_trait]
impl Tool for GitHubRepository {
    fn name(&self) -> &'static str {
        "github_repository"
    }
    fn description(&self) -> &'static str {
        "Read GitHub repository metadata using GITHUB_TOKEN or gh auth."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"repository":{"type":"string"}},"required":["repository"],"additionalProperties":false})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _: &ToolContext,
    ) -> Result<ToolResult, AppError> {
        let repo = args
            .get("repository")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Tool("github_repository requires repository".into()))?;
        Ok(result(&provider().await?.repository(repo).await?))
    }
}

pub struct GitHubIssues;
#[async_trait]
impl Tool for GitHubIssues {
    fn name(&self) -> &'static str {
        "github_issues"
    }
    fn description(&self) -> &'static str {
        "List GitHub issues in read-only mode."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"repository":{"type":"string"},"state":{"type":"string","enum":["open","closed","all"]},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["repository"],"additionalProperties":false})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _: &ToolContext,
    ) -> Result<ToolResult, AppError> {
        let repo = args
            .get("repository")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Tool("github_issues requires repository".into()))?;
        let state = args
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("open");
        Ok(result(
            &provider()
                .await?
                .list_issues(repo, state, limit(&args))
                .await?,
        ))
    }
}

pub struct GitHubIssue;
#[async_trait]
impl Tool for GitHubIssue {
    fn name(&self) -> &'static str {
        "github_issue"
    }
    fn description(&self) -> &'static str {
        "Read one GitHub issue and its comments."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"repository":{"type":"string"},"number":{"type":"integer"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["repository","number"],"additionalProperties":false})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _: &ToolContext,
    ) -> Result<ToolResult, AppError> {
        let repo = args
            .get("repository")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Tool("github_issue requires repository".into()))?;
        let number = args
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AppError::Tool("github_issue requires number".into()))?;
        let p = provider().await?;
        Ok(result(
            &serde_json::json!({"issue": p.issue(repo, number).await?, "comments": p.issue_comments(repo, number, limit(&args)).await?}),
        ))
    }
}

pub struct GitHubPullRequests;
#[async_trait]
impl Tool for GitHubPullRequests {
    fn name(&self) -> &'static str {
        "github_pull_requests"
    }
    fn description(&self) -> &'static str {
        "List GitHub pull requests in read-only mode."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"repository":{"type":"string"},"state":{"type":"string","enum":["open","closed","all"]},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["repository"],"additionalProperties":false})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _: &ToolContext,
    ) -> Result<ToolResult, AppError> {
        let repo = args
            .get("repository")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AppError::Tool("github_pull_requests requires repository".into()))?;
        let state = args
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("open");
        Ok(result(
            &provider()
                .await?
                .list_pull_requests(repo, state, limit(&args))
                .await?,
        ))
    }
}
