//! Project-local, bounded, read-only Coder inspection. No project code is executed.

use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[cfg(unix)]
use cap_std::fs::MetadataExt;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::AppError;

const MAX_ENTRIES: usize = 400;
const MAX_DEPTH: usize = 5;
const MAX_GIT_BYTES: u64 = 64 * 1024;
const MAX_DIFF_BYTES: u64 = 32 * 1024;
const MAX_EDIT_BYTES: usize = 32 * 1024;

/// One relative path in the bounded project tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderFileDto {
    pub path: String,
    pub directory: bool,
}

/// Project tree, with an explicit indication that the result is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderTreeDto {
    pub files: Vec<CoderFileDto>,
    pub truncated: bool,
}

/// A project-local Git worktree or index change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderChangeDto {
    pub path: String,
    pub status: String,
}

/// A bounded diff for one selected project file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderDiffDto {
    pub path: String,
    pub staged: bool,
    pub content: String,
}

/// UTF-8 content and its SHA-256 precondition; never read more than 32 KiB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderFileContentDto {
    pub path: String,
    pub content: String,
    pub digest: String,
}

/// Server-owned proposed replacement. Do not deserialize this from a client or
/// treat its random ID as authorization: the service must retain this value and
/// bind a separate approval to project, path and both content digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoderEditPreviewDto {
    id: Uuid,
    path: String,
    original_digest: String,
    new_digest: String,
    content: String,
    old_text: String,
    new_text: String,
}

impl CoderEditPreviewDto {
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn original_digest(&self) -> &str {
        &self.original_digest
    }
    pub fn new_digest(&self) -> &str {
        &self.new_digest
    }
    pub fn content(&self) -> &str {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderEditResultDto {
    pub path: String,
    pub digest: String,
    /// Relative to the selected project; this is not an automatic undo token.
    pub checkpoint: String,
}

/// Explicit, non-executing check result; arbitrary build/test commands are not exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderCheckDto {
    pub name: String,
    pub passed: bool,
    pub details: String,
}

/// Inspection-only Python environment status; creation and pip are not exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderVenvDto {
    pub present: bool,
    pub interpreter: Option<String>,
    pub note: String,
}

fn deny(message: &str) -> AppError {
    AppError::Tool(message.to_owned())
}

fn allowed_component(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        ".git" | ".aiagent" | ".venv" | "venv" | "node_modules" | "target" | "__pycache__" | ".env"
    ) && !lower.starts_with(".env.")
        && !lower.contains("secret")
        && !lower.contains("credential")
        && !lower.contains("token")
        && ![".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

fn safe_relative(raw: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(raw);
    if raw.is_empty() || raw.len() > 512 || raw.split(['/', '\\']).any(|part| part.is_empty() || part == "." || part == "..") || path.components().any(|component| {
        !matches!(component, Component::Normal(name) if name.to_str().is_some_and(allowed_component))
    }) { return Err(deny("Coder path must be a non-sensitive relative project file")); }
    Ok(path.to_path_buf())
}

fn io_error(error: std::io::Error) -> AppError {
    deny(&error.to_string())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn reject_secret_marker(text: &str) -> Result<(), AppError> {
    if text.contains("-----BEGIN PRIVATE KEY-----")
        || text.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
        || text.contains("-----BEGIN RSA PRIVATE KEY-----")
    {
        return Err(deny(
            "Coder refuses content containing a private key marker",
        ));
    }
    Ok(())
}

// Walk each component using directory handles, never resolving a link even if
// its destination would still be under the selected project.
fn parent_dir(root: &Dir, relative: &Path) -> Result<(Dir, String), AppError> {
    let mut dir = root.try_clone().map_err(io_error)?;
    let mut parts = relative.components().peekable();
    while let Some(component) = parts.next() {
        let Component::Normal(name) = component else {
            return Err(deny("invalid project path"));
        };
        let name = name
            .to_str()
            .ok_or_else(|| deny("invalid project path encoding"))?;
        if parts.peek().is_none() {
            return Ok((dir, name.to_owned()));
        }
        let meta = dir.symlink_metadata(name).map_err(io_error)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(deny("Coder requires ordinary, non-linked directories"));
        }
        dir = dir.open_dir(name).map_err(io_error)?;
    }
    Err(deny("empty project path"))
}

fn project_dir(root: &Path) -> Result<Dir, AppError> {
    let meta = fs::symlink_metadata(root).map_err(io_error)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(deny("Coder project must be a real directory"));
    }
    Dir::open_ambient_dir(root, ambient_authority()).map_err(io_error)
}

fn ordinary_file(meta: &cap_std::fs::Metadata) -> Result<(), AppError> {
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(deny(
            "Coder requires an ordinary file, not a link or special file",
        ));
    }
    #[cfg(unix)]
    {
        if meta.nlink() != 1 {
            return Err(deny("Coder refuses multiply linked files"));
        }
    }
    Ok(())
}

fn bounded_file(dir: &Dir, name: &str) -> Result<(Vec<u8>, cap_std::fs::Metadata), AppError> {
    let before = dir.symlink_metadata(name).map_err(io_error)?;
    ordinary_file(&before)?;
    if before.len() > MAX_EDIT_BYTES as u64 {
        return Err(deny("Coder file exceeds 32 KiB"));
    }
    let file = dir.open(name).map_err(io_error)?;
    let meta = file.metadata().map_err(io_error)?;
    ordinary_file(&meta)?;
    #[cfg(unix)]
    {
        if before.dev() != meta.dev() || before.ino() != meta.ino() {
            return Err(deny("Coder file changed while opening"));
        }
    }
    if meta.len() > MAX_EDIT_BYTES as u64 {
        return Err(deny("Coder file exceeds 32 KiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_EDIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_EDIT_BYTES {
        return Err(deny("Coder file exceeds 32 KiB"));
    }
    Ok((bytes, meta))
}

/// Read an existing ordinary UTF-8 project file, at most 32 KiB.
pub fn read_file(root: &Path, path: &str) -> Result<CoderFileContentDto, AppError> {
    let relative = safe_relative(path)?;
    let root = project_dir(root)?;
    let (dir, name) = parent_dir(&root, &relative)?;
    let (bytes, _) = bounded_file(&dir, &name)?;
    let content = String::from_utf8(bytes).map_err(|_| deny("Coder requires UTF-8 text"))?;
    reject_secret_marker(&content)?;
    Ok(CoderFileContentDto {
        path: path.to_owned(),
        digest: digest(content.as_bytes()),
        content,
    })
}

/// Preview a single, unique literal replacement; no file is written.
pub fn preview_edit(
    root: &Path,
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<CoderEditPreviewDto, AppError> {
    if old_text.is_empty()
        || new_text.is_empty()
        || old_text.len() > MAX_EDIT_BYTES
        || new_text.len() > MAX_EDIT_BYTES
    {
        return Err(deny(
            "Coder edit requires nonempty, bounded old and new text",
        ));
    }
    let original = read_file(root, path)?;
    let mut matches = original.content.match_indices(old_text);
    let (start, _) = matches
        .next()
        .ok_or_else(|| deny("Coder old text not found"))?;
    if matches.next().is_some() {
        return Err(deny("Coder old text must occur exactly once"));
    }
    let mut content = String::with_capacity(original.content.len().saturating_add(new_text.len()));
    content.push_str(&original.content[..start]);
    content.push_str(new_text);
    content.push_str(&original.content[start + old_text.len()..]);
    if content.len() > MAX_EDIT_BYTES || content.is_empty() || content == original.content {
        return Err(deny("Coder replacement must change the file within 32 KiB"));
    }
    reject_secret_marker(&content)?;
    Ok(CoderEditPreviewDto {
        id: Uuid::new_v4(),
        path: path.to_owned(),
        original_digest: original.digest,
        new_digest: digest(content.as_bytes()),
        content,
        old_text: old_text.to_owned(),
        new_text: new_text.to_owned(),
    })
}

fn checkpoint_dir(root: &Dir) -> Result<Dir, AppError> {
    let mut dir = root.try_clone().map_err(io_error)?;
    for name in [".aiagent", "checkpoints"] {
        match dir.symlink_metadata(name) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(deny(
                    "Coder checkpoint directory is not an ordinary directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                dir.create_dir(name).map_err(io_error)?
            }
            Err(error) => return Err(io_error(error)),
        }
        dir = dir.open_dir(name).map_err(io_error)?;
    }
    Ok(dir)
}

fn create_private(dir: &Dir, name: &str, content: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = dir.open_with(name, &options).map_err(io_error)?;
    let result = file.write_all(content).and_then(|_| file.sync_all());
    if result.is_err() {
        let _ = dir.remove_file(name);
    }
    result.map_err(io_error)
}

/// Apply only a server-retained preview AFTER a service has authorized this exact
/// project/path/original/new byte tuple. This function does not issue approvals.
/// Callers must serialize writes to the same project; digest checks alone are
/// not a filesystem compare-and-swap against independent concurrent writers.
pub fn apply_edit(
    root: &Path,
    preview: &CoderEditPreviewDto,
) -> Result<CoderEditResultDto, AppError> {
    #[cfg(windows)]
    return Err(deny(
        "Coder edits are disabled on Windows until safe link and atomic replacement checks are supported",
    ));

    #[cfg(not(windows))]
    {
        let relative = safe_relative(&preview.path)?;
        if preview.content.len() > MAX_EDIT_BYTES
            || preview.content.is_empty()
            || preview.old_text.is_empty()
            || preview.new_text.is_empty()
            || digest(preview.content.as_bytes()) != preview.new_digest
        {
            return Err(deny("invalid Coder preview"));
        }
        reject_secret_marker(&preview.content)?;
        let root = project_dir(root)?;
        let (dir, name) = parent_dir(&root, &relative)?;
        let (original, meta) = bounded_file(&dir, &name)?;
        if digest(&original) != preview.original_digest {
            return Err(deny("Coder file changed since preview"));
        }
        let original_text =
            std::str::from_utf8(&original).map_err(|_| deny("Coder requires UTF-8 text"))?;
        reject_secret_marker(original_text)?;
        if original_text.matches(&preview.old_text).count() != 1
            || original_text.replacen(&preview.old_text, &preview.new_text, 1) != preview.content
        {
            return Err(deny("Coder preview does not match proposed replacement"));
        }
        let checkpoints = checkpoint_dir(&root)?;
        let backup_name = format!("coder-{}.bak", Uuid::new_v4());
        create_private(&checkpoints, &backup_name, &original)?;
        let temporary = format!(".coder-{}.tmp", Uuid::new_v4());
        create_private(&dir, &temporary, preview.content.as_bytes())?;
        let result = (|| {
            let fresh = dir.symlink_metadata(&name).map_err(io_error)?;
            ordinary_file(&fresh)?;
            if meta.dev() != fresh.dev() || meta.ino() != fresh.ino() || meta.len() != fresh.len() {
                return Err(deny("Coder file changed before replacement"));
            }
            let (latest, _) = bounded_file(&dir, &name)?;
            if latest != original {
                return Err(deny("Coder file changed before replacement"));
            }
            // Preserve original permissions (including read-only bits); temp is
            // created private and never follows a path supplied by the client.
            let temp = dir.open(&temporary).map_err(io_error)?;
            // Never propagate setuid/setgid/sticky flags onto generated files.
            let mut permissions = meta.permissions();
            use cap_std::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() & 0o777);
            temp.set_permissions(permissions).map_err(io_error)?;
            temp.sync_all().map_err(io_error)?;
            dir.rename(&temporary, &dir, &name).map_err(io_error)
        })();
        if result.is_err() {
            let _ = dir.remove_file(&temporary);
        }
        result?;
        Ok(CoderEditResultDto {
            path: preview.path.clone(),
            digest: preview.new_digest.clone(),
            checkpoint: format!(".aiagent/checkpoints/{backup_name}"),
        })
    }
}

fn safe_entry(root: &Path, relative: &Path, may_be_missing: bool) -> Result<bool, AppError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(deny("invalid project path"));
        };
        let Some(name) = name.to_str() else {
            return Err(deny("invalid project path encoding"));
        };
        if !allowed_component(name) {
            return Err(deny("sensitive project path"));
        }
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(deny("symlink paths are not available in Coder"));
            }
            Ok(_) => {}
            Err(error) if may_be_missing && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(deny(&error.to_string())),
        }
    }
    Ok(true)
}

/// Lists ordinary entries without following links, including a depth and count limit.
pub fn tree(root: &Path) -> Result<CoderTreeDto, AppError> {
    let root = project_dir(root)?;
    let mut result = CoderTreeDto {
        files: Vec::new(),
        truncated: false,
    };
    let mut pending = vec![(root, PathBuf::new(), 0)];
    while let Some((dir, relative, depth)) = pending.pop() {
        let mut entries = dir
            .entries()
            .map_err(io_error)?
            .take(MAX_ENTRIES + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?;
        entries.sort_by_key(|entry| entry.file_name());
        if entries.len() > MAX_ENTRIES {
            result.truncated = true;
            entries.truncate(MAX_ENTRIES);
        }
        for entry in entries {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !allowed_component(name) {
                continue;
            }
            let kind = entry.file_type().map_err(io_error)?;
            if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                continue;
            }
            // A directory can be replaced by a link after enumeration. Only
            // descend through a handle after checking the opened directory.
            let child = if kind.is_dir() && depth < MAX_DEPTH {
                let before = dir.symlink_metadata(name).map_err(io_error)?;
                if !before.is_dir() || before.file_type().is_symlink() {
                    continue;
                }
                let opened = entry.open_dir().map_err(io_error)?;
                #[cfg(unix)]
                {
                    let after = opened.dir_metadata().map_err(io_error)?;
                    if before.dev() != after.dev() || before.ino() != after.ino() {
                        continue;
                    }
                }
                Some(opened)
            } else {
                None
            };
            if result.files.len() >= MAX_ENTRIES {
                result.truncated = true;
                return Ok(result);
            }
            let path = relative.join(name);
            result.files.push(CoderFileDto {
                path: path.to_string_lossy().into_owned(),
                directory: kind.is_dir(),
            });
            if kind.is_dir() {
                if let Some(child) = child {
                    pending.push((child, path, depth + 1));
                } else {
                    result.truncated = true;
                }
            }
        }
    }
    result.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn git_root(root: &Path) -> Result<(), AppError> {
    let meta = fs::symlink_metadata(root.join(".git"))
        .map_err(|_| deny("Git status requires the selected project to be the repository root"))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(deny("Git repository metadata must be a local directory"));
    }
    Ok(())
}

async fn git(root: &Path, args: &[&str], limit: u64) -> Result<Vec<u8>, AppError> {
    let (bytes, passed) = git_output(root, args, limit).await?;
    if !passed {
        return Err(deny("Git inspection failed"));
    }
    Ok(bytes)
}

async fn git_output(root: &Path, args: &[&str], limit: u64) -> Result<(Vec<u8>, bool), AppError> {
    git_root(root)?;
    // A fixed argv, empty inherited environment and disabled external diff prevent
    // project config from supplying helpers or reading user-scoped Git settings.
    let mut child = tokio::process::Command::new("git")
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "diff.external=",
            "-c",
            "color.ui=false",
        ])
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_SYSTEM",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| deny(&format!("Git unavailable: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| deny("Git output unavailable"))?;
    let operation = async {
        let mut bytes = Vec::new();
        stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| deny(&error.to_string()))?;
        if bytes.len() as u64 > limit {
            return Err(deny("Git output exceeds Coder limit"));
        }
        let passed = child
            .wait()
            .await
            .map_err(|error| deny(&error.to_string()))?
            .success();
        Ok((bytes, passed))
    };
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .map_err(|_| deny("Git inspection timed out"))?
}

/// Lists changed files only when the selected project is itself the Git root.
pub async fn changes(root: &Path) -> Result<Vec<CoderChangeDto>, AppError> {
    let bytes = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
            "--",
            ".",
        ],
        MAX_GIT_BYTES,
    )
    .await?;
    let mut result = Vec::new();
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(deny("unrecognized Git status"));
        }
        let path = std::str::from_utf8(&entry[3..]).map_err(|_| deny("non-UTF8 Git path"))?;
        let relative = match safe_relative(path) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if safe_entry(root, &relative, true).is_err() {
            continue;
        }
        result.push(CoderChangeDto {
            path: path.to_owned(),
            status: String::from_utf8_lossy(&entry[..2]).into_owned(),
        });
        if result.len() > MAX_ENTRIES {
            return Err(deny("Too many project changes"));
        }
    }
    Ok(result)
}

/// Returns a bounded tracked-file diff; untracked files have no Git diff.
pub async fn diff(root: &Path, raw: &str, staged: bool) -> Result<CoderDiffDto, AppError> {
    let relative = safe_relative(raw)?;
    let change = changes(root)
        .await?
        .into_iter()
        .find(|entry| entry.path == raw && entry.status != "??")
        .ok_or_else(|| deny("file has no visible tracked changes"))?;
    if safe_entry(root, &relative, true)? {
        let (dir, name) = parent_dir(&project_dir(root)?, &relative)?;
        let meta = dir.symlink_metadata(&name).map_err(io_error)?;
        ordinary_file(&meta)?;
    } else if !change.status.contains('D') {
        return Err(deny("file is not present in this project"));
    }
    let args = if staged {
        vec![
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--unified=3",
            "--",
            raw,
        ]
    } else {
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--unified=3",
            "--",
            raw,
        ]
    };
    let bytes = git(root, &args, MAX_DIFF_BYTES).await?;
    let content = String::from_utf8(bytes).map_err(|_| deny("non-UTF8 diff"))?;
    Ok(CoderDiffDto {
        path: raw.to_owned(),
        staged,
        content,
    })
}

/// Runs Git's whitespace check without invoking the project's test or build scripts.
pub async fn check_whitespace(root: &Path) -> Result<CoderCheckDto, AppError> {
    // Do not expose diagnostics (including offending lines) from excluded files.
    // Explicit literal pathspecs avoid inspecting hidden paths in the worktree.
    let paths = changes(root)
        .await?
        .into_iter()
        .filter(|change| change.status.as_bytes().get(1) != Some(&b'?'))
        .map(|change| change.path)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(CoderCheckDto {
            name: "git diff --check".into(),
            passed: true,
            details: "No visible unstaged tracked changes to check".into(),
        });
    }
    let mut args = vec!["diff", "--check", "--no-ext-diff", "--no-textconv", "--"];
    args.extend(paths.iter().map(String::as_str));
    let (bytes, passed) = git_output(root, &args, MAX_GIT_BYTES).await?;
    Ok(CoderCheckDto {
        name: "git diff --check".into(),
        passed,
        details: if passed {
            "No unstaged whitespace errors (untracked files are not checked)".into()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        },
    })
}

/// Inspects `.venv` metadata without executing Python or following a link.
pub fn venv(root: &Path) -> Result<CoderVenvDto, AppError> {
    let path = root.join(".venv");
    let present = match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(deny("linked virtual environments are not supported"));
        }
        Ok(meta) if meta.is_dir() => true,
        Ok(_) => return Err(deny(".venv is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(deny(&error.to_string())),
    };
    let candidate = if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    };
    let interpreter = if present
        && fs::symlink_metadata(path.join("pyvenv.cfg"))
            .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
        && safe_entry(root, Path::new(candidate), false).is_ok()
    {
        Some(candidate.to_owned())
    } else {
        None
    };
    Ok(CoderVenvDto { present, interpreter, note: "Inspection only: environment creation, installation and execution require a separate sandboxed approval contract".into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("coder-inspect-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn git(&self, args: &[&str]) {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&self.0)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn tree_hides_metadata_secrets_and_links_and_reports_limits() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join(".aiagent")).unwrap();
        fs::write(fixture.0.join(".aiagent/config.json"), "secret").unwrap();
        fs::write(fixture.0.join(".env"), "secret").unwrap();
        fs::write(fixture.0.join("code.rs"), "code").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/tmp", fixture.0.join("link")).unwrap();
        let listing = tree(&fixture.0).unwrap();
        assert_eq!(
            listing.files,
            vec![CoderFileDto {
                path: "code.rs".into(),
                directory: false
            }]
        );
        for index in 0..=MAX_ENTRIES {
            fs::write(fixture.0.join(format!("file-{index}")), "x").unwrap();
        }
        let listing = tree(&fixture.0).unwrap();
        assert!(listing.truncated);
        assert!(listing.files.len() <= MAX_ENTRIES);
    }

    #[tokio::test]
    async fn git_status_and_diff_are_bounded_and_reject_foreign_paths() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        fs::write(fixture.0.join("code.rs"), "first\n").unwrap();
        fixture.git(&["add", "code.rs"]);
        fs::write(fixture.0.join("code.rs"), "second\n").unwrap();
        fs::write(fixture.0.join(".env"), "private").unwrap();
        let status = changes(&fixture.0).await.unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].path, "code.rs");
        let patch = diff(&fixture.0, "code.rs", false).await.unwrap();
        assert!(patch.content.contains("+second"));
        assert!(
            diff(&fixture.0, "code.rs", true)
                .await
                .unwrap()
                .content
                .contains("+first")
        );
        for invalid in [
            "../outside",
            "/etc/passwd",
            ".env",
            ".git/config",
            "missing",
            "--output=/tmp/escape",
        ] {
            assert!(diff(&fixture.0, invalid, false).await.is_err(), "{invalid}");
        }
        fs::write(
            fixture.0.join("code.rs"),
            "x".repeat(MAX_DIFF_BYTES as usize + 200),
        )
        .unwrap();
        assert!(diff(&fixture.0, "code.rs", false).await.is_err());
    }

    #[tokio::test]
    async fn diff_does_not_read_unchanged_or_untracked_paths() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        fs::write(fixture.0.join("tracked.txt"), "visible\n").unwrap();
        fixture.git(&["add", "tracked.txt"]);
        fs::write(fixture.0.join("untracked.txt"), "private\n").unwrap();
        assert!(diff(&fixture.0, "tracked.txt", true).await.is_ok());
        assert!(diff(&fixture.0, "tracked.txt", false).await.is_ok());
        assert!(diff(&fixture.0, "untracked.txt", false).await.is_err());
        assert!(diff(&fixture.0, "untracked.txt", true).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("tracked.txt", fixture.0.join("linked.txt")).unwrap();
            fixture.git(&["add", "linked.txt"]);
            assert!(diff(&fixture.0, "linked.txt", true).await.is_err());
        }
    }

    #[tokio::test]
    async fn subdirectory_repo_is_refused_and_venv_is_inspection_only() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        fs::create_dir(fixture.0.join("child")).unwrap();
        assert!(changes(&fixture.0.join("child")).await.is_err());
        assert!(!venv(&fixture.0).unwrap().present);
        fs::create_dir(fixture.0.join(".venv")).unwrap();
        assert!(venv(&fixture.0).unwrap().present);
        #[cfg(unix)]
        {
            fs::remove_dir(fixture.0.join(".venv")).unwrap();
            std::os::unix::fs::symlink("/tmp", fixture.0.join(".venv")).unwrap();
            assert!(venv(&fixture.0).is_err());
        }
    }

    #[tokio::test]
    async fn whitespace_check_reports_failure_without_running_project_code() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        fs::write(fixture.0.join("code.rs"), "ok\n").unwrap();
        fixture.git(&["add", "code.rs"]);
        fs::write(fixture.0.join("code.rs"), "bad  \n").unwrap();
        let result = check_whitespace(&fixture.0).await.unwrap();
        assert!(!result.passed);
        assert!(result.details.contains("code.rs"));
    }

    #[tokio::test]
    async fn whitespace_check_does_not_disclose_excluded_files() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        fs::write(fixture.0.join(".env"), "old\n").unwrap();
        fixture.git(&["add", ".env"]);
        fs::write(fixture.0.join(".env"), "private-key  \n").unwrap();
        let result = check_whitespace(&fixture.0).await.unwrap();
        assert!(result.passed);
        assert!(!result.details.contains("private-key"));
    }

    #[test]
    fn preview_and_apply_create_backup_and_reject_stale_content() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("src")).unwrap();
        fs::write(fixture.0.join("src/main.rs"), "before\n").unwrap();
        let read = read_file(&fixture.0, "src/main.rs").unwrap();
        assert_eq!(read.content, "before\n");
        let preview = preview_edit(&fixture.0, "src/main.rs", "before", "after").unwrap();
        assert_ne!(preview.id(), Uuid::nil());
        assert_eq!(preview.original_digest(), read.digest);
        assert_eq!(
            fs::read_to_string(fixture.0.join("src/main.rs")).unwrap(),
            "before\n"
        );
        let result = apply_edit(&fixture.0, &preview).unwrap();
        assert_eq!(result.digest, preview.new_digest());
        assert_eq!(
            fs::read_to_string(fixture.0.join("src/main.rs")).unwrap(),
            "after\n"
        );
        assert_eq!(
            fs::read(fixture.0.join(&result.checkpoint)).unwrap(),
            b"before\n"
        );
        assert!(apply_edit(&fixture.0, &preview).is_err());
        let another = preview_edit(&fixture.0, "src/main.rs", "after", "again").unwrap();
        fs::write(fixture.0.join("src/main.rs"), "outside\n").unwrap();
        assert!(apply_edit(&fixture.0, &another).is_err());
        assert_eq!(
            fs::read_to_string(fixture.0.join("src/main.rs")).unwrap(),
            "outside\n"
        );
    }

    #[test]
    fn paths_links_and_special_files_are_refused() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("ok.txt"), "abc").unwrap();
        for path in [
            "",
            ".",
            "../ok.txt",
            "/etc/passwd",
            "a/../ok.txt",
            ".git/config",
            ".env.local",
            "a//b",
            "missing",
            "..\\ok.txt",
        ] {
            assert!(read_file(&fixture.0, path).is_err(), "{path}");
            assert!(preview_edit(&fixture.0, path, "a", "b").is_err(), "{path}");
        }
        assert!(read_file(&fixture.0, ".").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("ok.txt", fixture.0.join("link.txt")).unwrap();
            std::os::unix::fs::symlink("/tmp", fixture.0.join("external")).unwrap();
            assert!(read_file(&fixture.0, "link.txt").is_err());
            assert!(read_file(&fixture.0, "external/file").is_err());
            assert!(preview_edit(&fixture.0, "link.txt", "abc", "def").is_err());
            std::os::unix::fs::symlink("/tmp", fixture.0.join(".aiagent")).unwrap();
            let preview = preview_edit(&fixture.0, "ok.txt", "abc", "def").unwrap();
            assert!(apply_edit(&fixture.0, &preview).is_err());
            assert_eq!(fs::read(fixture.0.join("ok.txt")).unwrap(), b"abc");
        }
        assert!(read_file(&fixture.0, ".aiagent").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_and_fifo_are_refused() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("original"), "private").unwrap();
        fs::hard_link(outside.0.join("original"), fixture.0.join("alias")).unwrap();
        assert!(read_file(&fixture.0, "alias").is_err());
        assert!(preview_edit(&fixture.0, "alias", "private", "public").is_err());
        assert_eq!(fs::read(outside.0.join("original")).unwrap(), b"private");
        // A Unix socket is a special file and must never be opened as text.
        let socket = std::env::temp_dir().join(format!("c-{}", Uuid::new_v4()));
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::rename(&socket, fixture.0.join("socket")).unwrap();
        assert!(read_file(&fixture.0, "socket").is_err());
    }

    #[test]
    fn edit_limits_unique_match_and_secret_markers() {
        let fixture = Fixture::new();
        let path = fixture.0.join("text.txt");
        fs::write(&path, "aa").unwrap();
        assert!(preview_edit(&fixture.0, "text.txt", "a", "b").is_err());
        assert!(preview_edit(&fixture.0, "text.txt", "", "b").is_err());
        assert!(preview_edit(&fixture.0, "text.txt", "aa", "").is_err());
        assert!(preview_edit(&fixture.0, "text.txt", "aa", "aa").is_err());
        assert!(preview_edit(&fixture.0, "text.txt", "aa", "-----BEGIN PRIVATE KEY-----").is_err());
        fs::write(&path, "x".repeat(MAX_EDIT_BYTES)).unwrap();
        assert_eq!(
            read_file(&fixture.0, "text.txt").unwrap().content.len(),
            MAX_EDIT_BYTES
        );
        assert!(preview_edit(&fixture.0, "text.txt", "x", "y").is_err());
        assert!(
            preview_edit(
                &fixture.0,
                "text.txt",
                "x".repeat(MAX_EDIT_BYTES).as_str(),
                "y".repeat(MAX_EDIT_BYTES + 1).as_str()
            )
            .is_err()
        );
        fs::write(&path, "x".repeat(MAX_EDIT_BYTES + 1)).unwrap();
        assert!(read_file(&fixture.0, "text.txt").is_err());
        fs::write(&path, [0xff]).unwrap();
        assert!(read_file(&fixture.0, "text.txt").is_err());
    }
}
