//! Safe project memory and context compaction.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MEMORY_FILE: &str = ".aiagent/project-memory.json";
const AGENTS_FILE: &str = "AGENTS.md";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMemory {
    pub summary: String,
    pub preferences: Vec<String>,
    pub decisions: Vec<String>,
}

impl ProjectMemory {
    pub fn load(root: impl AsRef<Path>) -> Result<Self, String> {
        let path = root.as_ref().join(MEMORY_FILE);
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), String> {
        let root = root.as_ref();
        fs::create_dir_all(root.join(".aiagent")).map_err(|error| error.to_string())?;
        let path = root.join(MEMORY_FILE);
        let temporary = path.with_extension("tmp");
        let data = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        fs::write(&temporary, data).map_err(|error| error.to_string())?;
        fs::rename(temporary, path).map_err(|error| error.to_string())
    }

    pub fn instructions(root: impl AsRef<Path>) -> String {
        let root = root.as_ref();
        let mut paths = Vec::new();
        let mut current = Some(root);
        while let Some(path) = current {
            let candidate = path.join(AGENTS_FILE);
            if candidate.is_file() {
                paths.push(candidate);
            }
            current = path.parent();
        }
        paths.reverse();
        paths
            .into_iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn compact_text(messages: &[String], max_chars: usize) -> String {
        let mut result = messages.iter().rev().take(12).cloned().collect::<Vec<_>>();
        result.reverse();
        let mut text = result.join("\n");
        if text.len() > max_chars {
            text.truncate(max_chars);
        }
        text
    }
}

pub fn memory_path(root: impl Into<PathBuf>) -> PathBuf {
    root.into().join(MEMORY_FILE)
}
