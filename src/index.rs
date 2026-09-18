//! Локальный инкрементальный индекс текстовых файлов проекта.

use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

const FORMAT_VERSION: u32 = 1;
const DEFAULT_MAX_FILE_BYTES: u64 = 1_000_000;
const DEFAULT_CHUNK_LINES: usize = 80;
const MAX_RESULTS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChunk {
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: String,
    pub size: u64,
    pub modified_secs: u64,
    pub hash: String,
    pub language: String,
    pub chunks: Vec<FileChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub root: PathBuf,
    pub generated_secs: u64,
    pub files: HashMap<String, IndexedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: usize,
    pub text: String,
}

impl ProjectIndex {
    pub fn index_path(root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().join(".aiagent").join("index.json")
    }

    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        let mut index = Self {
            version: FORMAT_VERSION,
            root: root.clone(),
            generated_secs: now_secs(),
            files: HashMap::new(),
        };
        index.update()?;
        Ok(index)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let data = fs::read(path)?;
        let index: Self = serde_json::from_slice(&data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if index.version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "неподдерживаемая версия индекса",
            ));
        }
        Ok(index)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let data =
            serde_json::to_vec_pretty(self).map_err(|error| io::Error::other(error.to_string()))?;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(temporary, path)
    }

    pub fn update(&mut self) -> io::Result<()> {
        let mut seen = std::collections::HashSet::new();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if !is_excluded_directory(&name) {
                        pending.push(path);
                    }
                    continue;
                }
                if !path.is_file() || is_excluded_file(&path) {
                    continue;
                }
                let relative = path.strip_prefix(&self.root).unwrap_or(&path);
                let key = relative.to_string_lossy().replace('\\', "/");
                seen.insert(key.clone());
                let metadata = fs::metadata(&path)?;
                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_secs());
                let unchanged = self.files.get(&key).is_some_and(|file| {
                    file.size == metadata.len() && file.modified_secs == modified_secs
                });
                if !unchanged {
                    if let Some(file) = index_file(&path, &key, &metadata)? {
                        self.files.insert(key, file);
                    } else {
                        self.files.remove(&key);
                    }
                }
            }
        }
        self.files.retain(|path, _| seen.contains(path));
        self.generated_secs = now_secs();
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits = self
            .files
            .values()
            .flat_map(|file| {
                file.chunks.iter().filter_map(|chunk| {
                    let haystack = format!("{} {}", file.path, chunk.text).to_lowercase();
                    let score = terms.iter().filter(|term| haystack.contains(*term)).count();
                    (score > 0).then(|| SearchHit {
                        path: file.path.clone(),
                        start_line: chunk.start_line,
                        end_line: chunk.end_line,
                        score,
                        text: chunk.text.clone(),
                    })
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.start_line.cmp(&right.start_line))
        });
        hits.truncate(limit.min(MAX_RESULTS));
        hits
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

fn index_file(path: &Path, key: &str, metadata: &fs::Metadata) -> io::Result<Option<IndexedFile>> {
    if metadata.len() > DEFAULT_MAX_FILE_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let Ok(text) = String::from_utf8(bytes.clone()) else {
        return Ok(None);
    };
    let lines = text.lines().collect::<Vec<_>>();
    let chunks = lines
        .chunks(DEFAULT_CHUNK_LINES)
        .enumerate()
        .map(|(chunk_number, lines)| FileChunk {
            start_line: chunk_number * DEFAULT_CHUNK_LINES + 1,
            end_line: chunk_number * DEFAULT_CHUNK_LINES + lines.len(),
            text: lines.join("\n"),
        })
        .collect();
    Ok(Some(IndexedFile {
        path: key.to_owned(),
        size: metadata.len(),
        modified_secs: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs()),
        hash: format!("{:016x}", fnv1a(&bytes)),
        language: language_for(path),
        chunks,
    }))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn language_for(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(|| "text".to_owned(), str::to_lowercase)
}

fn is_excluded_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".agent" | ".aiagent"
    )
}

fn is_excluded_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".agent-session.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::ProjectIndex;

    #[test]
    fn indexes_text_searches_and_excludes_directories() {
        let root = std::env::temp_dir().join(format!("ai-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn memory() {}\n").unwrap();
        std::fs::write(root.join("target/ignored.rs"), "memory").unwrap();

        let index = ProjectIndex::build(&root).unwrap();
        assert_eq!(index.file_count(), 1);
        assert_eq!(index.search("memory", 10)[0].path, "src/lib.rs");
        assert_eq!(index.files["src/lib.rs"].language, "rs");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saves_and_loads_json() {
        let root = std::env::temp_dir().join(format!("ai-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("README.md"), "hello").unwrap();
        let index = ProjectIndex::build(&root).unwrap();
        let path = ProjectIndex::index_path(&root);
        index.save(&path).unwrap();
        let loaded = ProjectIndex::load(path).unwrap();
        assert_eq!(loaded.file_count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
