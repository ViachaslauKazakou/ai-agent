//! Project-local agent profiles and skills.

use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

use crate::{AppError, Config};

pub const PROJECT_DIR: &str = ".aiagent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub enabled_tools: Vec<String>,
    pub allow_write: bool,
    pub confirm_writes: bool,
    pub command_allowlist: Vec<String>,
    pub max_tool_rounds: usize,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct AgentCatalog {
    profiles: BTreeMap<String, AgentProfile>,
    skills: BTreeMap<String, Skill>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFile {
    description: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    system_prompt: Option<String>,
    enabled_tools: Option<Vec<String>>,
    allow_write: Option<bool>,
    confirm_writes: Option<bool>,
    command_allowlist: Option<Vec<String>>,
    max_tool_rounds: Option<usize>,
    skills: Option<Vec<String>>,
}

impl AgentCatalog {
    pub fn load(working_dir: &Path, config: &Config) -> Result<Self, AppError> {
        let root = working_dir.join(PROJECT_DIR);
        let mut profiles = BTreeMap::new();
        profiles.insert("default".to_owned(), default_profile(config));

        let agents_dir = root.join("agents");
        if agents_dir.is_dir() {
            for entry in read_directory(&agents_dir)? {
                if entry.path().extension().and_then(|value| value.to_str()) != Some("toml") {
                    continue;
                }
                let name = entry
                    .path()
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| {
                        AppError::AgentConfig("некорректное имя файла агента".to_owned())
                    })?
                    .to_owned();
                if name == "default" {
                    profiles.remove("default");
                }
                let file: AgentFile = parse_file(&entry.path())?;
                let profile = merge_profile(name.clone(), file, config)?;
                validate_profile(&profile)?;
                profiles.insert(name, profile);
            }
        }

        let skills = load_skills(&root.join("skills"))?;
        for profile in profiles.values() {
            for skill in &profile.skills {
                if !skills.contains_key(skill) {
                    return Err(AppError::AgentConfig(format!(
                        "агент {} ссылается на отсутствующий skill: {skill}",
                        profile.name
                    )));
                }
            }
        }

        Ok(Self { profiles, skills })
    }

    pub fn profile(&self, name: &str) -> Option<&AgentProfile> {
        self.profiles.get(name)
    }

    pub fn profiles(&self) -> impl Iterator<Item = &AgentProfile> {
        self.profiles.values()
    }

    pub fn skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }

    pub fn system_prompt(&self, profile: &AgentProfile) -> Result<String, AppError> {
        let mut prompt = profile.system_prompt.clone();
        for name in &profile.skills {
            let skill = self
                .skills
                .get(name)
                .ok_or_else(|| AppError::AgentConfig(format!("неизвестный skill: {name}")))?;
            prompt.push_str("\n\n--- Skill: ");
            prompt.push_str(&skill.name);
            prompt.push_str(" ---\n");
            prompt.push_str(&skill.content);
        }
        Ok(prompt)
    }
}

fn default_profile(config: &Config) -> AgentProfile {
    AgentProfile {
        name: "default".to_owned(),
        description: "Безопасный агент по умолчанию".to_owned(),
        provider: config.provider.clone(),
        model: config.model.clone(),
        system_prompt: "Ты полезный AI-агент проекта. Соблюдай ограничения доступных tools и working directory.".to_owned(),
        enabled_tools: config.enabled_tools.clone(),
        allow_write: config.allow_write,
        confirm_writes: config.confirm_writes,
        command_allowlist: config.command_allowlist.clone(),
        max_tool_rounds: config.max_tool_rounds,
        skills: Vec::new(),
    }
}

fn merge_profile(name: String, file: AgentFile, config: &Config) -> Result<AgentProfile, AppError> {
    let profile = AgentProfile {
        name,
        description: file
            .description
            .unwrap_or_else(|| "Project-local agent".to_owned()),
        provider: file.provider.unwrap_or_else(|| config.provider.clone()),
        model: file.model.unwrap_or_else(|| config.model.clone()),
        system_prompt: file
            .system_prompt
            .unwrap_or_else(|| "Ты полезный AI-агент проекта.".to_owned()),
        enabled_tools: file
            .enabled_tools
            .unwrap_or_else(|| config.enabled_tools.clone()),
        allow_write: file.allow_write.unwrap_or(config.allow_write),
        confirm_writes: file.confirm_writes.unwrap_or(config.confirm_writes),
        command_allowlist: file
            .command_allowlist
            .unwrap_or_else(|| config.command_allowlist.clone()),
        max_tool_rounds: file.max_tool_rounds.unwrap_or(config.max_tool_rounds),
        skills: file.skills.unwrap_or_default(),
    };
    if profile.model.trim().is_empty() {
        return Err(AppError::EmptyModel);
    }
    if profile.provider != "litellm" && profile.provider != "ollama" {
        return Err(AppError::InvalidConfig(format!(
            "агент {} использует неподдерживаемый provider: {}",
            profile.name, profile.provider
        )));
    }
    Ok(profile)
}

fn validate_profile(profile: &AgentProfile) -> Result<(), AppError> {
    if profile.max_tool_rounds == 0 {
        return Err(AppError::InvalidConfig(format!(
            "агент {} имеет max_tool_rounds = 0",
            profile.name
        )));
    }
    for tool in &profile.enabled_tools {
        if !matches!(
            tool.as_str(),
            "read_file"
                | "list_directory"
                | "write_file"
                | "apply_patch"
                | "rollback_last_change"
                | "git_status"
                | "git_diff"
                | "git_log"
                | "git_create_branch"
                | "git_prepare_commit"
                | "git_commit"
                | "git_push"
                | "git_create_pr"
                | "search_files"
                | "read_lines"
                | "project_search"
                | "run_command"
                | "list_recent_emails"
                | "get_email"
                | "search_emails"
        ) {
            return Err(AppError::UnknownTool(tool.clone()));
        }
    }
    if profile.allow_write
        && !profile
            .enabled_tools
            .iter()
            .any(|tool| tool == "write_file")
    {
        return Err(AppError::InvalidConfig(format!(
            "агент {} разрешает запись без write_file",
            profile.name
        )));
    }
    Ok(())
}

fn load_skills(directory: &Path) -> Result<BTreeMap<String, Skill>, AppError> {
    let mut skills = BTreeMap::new();
    if !directory.is_dir() {
        return Ok(skills);
    }
    for entry in read_directory(directory)? {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path().join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&path)
            .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display())))?;
        if content.trim().is_empty() {
            return Err(AppError::AgentConfig(format!("skill {name} пустой")));
        }
        let description = content
            .lines()
            .find_map(|line| {
                line.strip_prefix("description:")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or("Project-local skill")
            .to_owned();
        skills.insert(
            name.clone(),
            Skill {
                name,
                description,
                content,
            },
        );
    }
    Ok(skills)
}

fn read_directory(path: &Path) -> Result<Vec<fs::DirEntry>, AppError> {
    fs::read_dir(path)
        .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display())))
}

fn parse_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, AppError> {
    let content = fs::read_to_string(path)
        .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display())))?;
    toml::from_str(&content)
        .map_err(|error| AppError::AgentConfig(format!("{}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;

    fn config(root: &Path) -> Config {
        Config::from_sources(
            &crate::cli::Cli::try_parse_from(["ai-agent", "--working-dir", root.to_str().unwrap()])
                .unwrap(),
            &std::collections::HashMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn loads_project_agent_and_skill() {
        let root = std::env::temp_dir().join(format!("ai-agent-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".aiagent/agents")).unwrap();
        fs::create_dir_all(root.join(".aiagent/skills/testing")).unwrap();
        fs::write(
            root.join(".aiagent/agents/reviewer.toml"),
            "model = 'review-model'\nskills = ['testing']\n",
        )
        .unwrap();
        fs::write(
            root.join(".aiagent/skills/testing/SKILL.md"),
            "description: Run tests carefully\n\nRun tests after changes.",
        )
        .unwrap();
        let catalog = AgentCatalog::load(&root, &config(&root)).unwrap();
        let profile = catalog.profile("reviewer").unwrap();
        assert_eq!(profile.model, "review-model");
        assert!(
            catalog
                .system_prompt(profile)
                .unwrap()
                .contains("Run tests after changes.")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
