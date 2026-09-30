//! Project suggestions for the interactive console launcher, independent of terminal I/O.

use std::path::{Path, PathBuf};

use crate::{LaunchState, cli::Cli};

/// A selectable project and its label for the terminal adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectChoice {
    pub path: PathBuf,
    pub label: String,
}

/// Whether the console should offer recent projects rather than using CLI/env defaults.
pub fn should_offer_projects(cli: &Cli, environment_project: bool) -> bool {
    cli.prompt.is_none()
        && cli.project_dir.is_none()
        && cli.working_dir.is_none()
        && !environment_project
        && !cli.init
        && !cli.scheduler
        && !cli.graph_login
        && !cli.gmail_login
        && !cli.google_calendar_login
        && !cli.mcp_server
}

/// Builds a stable menu: most recent available project first, then other
/// available projects, with the current directory as an explicit alternative.
/// Missing projects remain in launch state but never become selectable.
pub fn project_choices(state: &LaunchState, current_dir: &Path) -> Vec<ProjectChoice> {
    let mut choices = Vec::new();
    for project in &state.projects {
        if project.path.is_dir()
            && !choices
                .iter()
                .any(|choice: &ProjectChoice| choice.path == project.path)
        {
            choices.push(ProjectChoice {
                label: format!("Недавний проект: {}", project.path.display()),
                path: project.path.clone(),
            });
        }
    }
    if !choices.iter().any(|choice| choice.path == current_dir) {
        choices.push(ProjectChoice {
            label: format!("Текущий каталог: {}", current_dir.display()),
            path: current_dir.to_path_buf(),
        });
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use clap::Parser;
    use uuid::Uuid;

    #[test]
    fn interactive_only_without_explicit_or_environment_project() {
        let plain = Cli::try_parse_from(["ai-agent"]).unwrap();
        assert!(should_offer_projects(&plain, false));
        assert!(!should_offer_projects(&plain, true));
        for args in [
            vec!["ai-agent", "--project-dir", "/tmp"],
            vec!["ai-agent", "--working-dir", "/tmp"],
            vec!["ai-agent", "hello"],
            vec!["ai-agent", "--init"],
            vec!["ai-agent", "--scheduler"],
            vec!["ai-agent", "--gmail-login"],
            vec!["ai-agent", "--graph-login"],
            vec!["ai-agent", "--google-calendar-login"],
            vec!["ai-agent", "--mcp-server"],
        ] {
            assert!(!should_offer_projects(
                &Cli::try_parse_from(args).unwrap(),
                false
            ));
        }
    }

    #[test]
    fn first_launch_and_unavailable_projects() {
        let root = std::env::temp_dir().join(format!("cli-choice-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut state = LaunchState::default();
        assert_eq!(project_choices(&state, &root).len(), 1);
        state.record_project(root.join("missing"), Utc::now());
        assert_eq!(project_choices(&state, &root).len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_projects_are_ordered_and_cwd_is_not_duplicated() {
        let root = std::env::temp_dir().join(format!("cli-choice-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let mut state = LaunchState::default();
        state.record_project(first.canonicalize().unwrap(), Utc::now());
        state.record_project(second.canonicalize().unwrap(), Utc::now());
        let choices = project_choices(&state, &first.canonicalize().unwrap());
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].path, second.canonicalize().unwrap());
        assert_eq!(choices[1].path, first.canonicalize().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
