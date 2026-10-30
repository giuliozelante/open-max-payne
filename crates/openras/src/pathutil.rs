//! Path normalization and traversal guards for archive entries.

use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("archive path rejects '..' components: {0}")]
    ParentDir(String),
    #[error("archive path rejects absolute / leading slash: {0}")]
    Absolute(String),
    #[error("empty file basename")]
    EmptyName,
}

/// Normalize Windows `\` to `/` and validate a relative archive-relative path.
pub fn normalize_rel(input: &str) -> Result<String, PathError> {
    let s = input.replace('\\', "/");
    if s.starts_with('/') {
        return Err(PathError::Absolute(input.to_string()));
    }
    let path = Path::new(&s);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathError::ParentDir(input.to_string())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(PathError::Absolute(input.to_string()));
            }
        }
    }
    Ok(out.to_string_lossy().replace('\\', "/"))
}

/// Convert RAS Windows dir name (`\data\menu\`) to a relative PathBuf under dest.
pub fn win_dir_to_rel(name: &str) -> Result<PathBuf, PathError> {
    let trimmed = name.trim_matches('\\').trim_matches('/');
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }
    let norm = normalize_rel(&trimmed.replace('\\', "/"))?;
    Ok(PathBuf::from(norm))
}

pub fn rel_to_win_dir(rel: &Path) -> String {
    if rel.as_os_str().is_empty() || rel == Path::new(".") {
        return "\\".to_string();
    }
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    format!("\\{}\\", parts.join("\\"))
}

pub fn validate_basename(name: &str) -> Result<(), PathError> {
    if name.is_empty() || name.contains('\\') || name.contains('/') || name.contains("..") {
        return Err(PathError::EmptyName);
    }
    if name == ".." || name == "." {
        return Err(PathError::ParentDir(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dotdot() {
        assert!(normalize_rel("a/../b").is_err());
        assert!(normalize_rel("..\\x").is_err());
    }

    #[test]
    fn rejects_absolute() {
        assert!(normalize_rel("/etc/passwd").is_err());
    }

    #[test]
    fn normalizes_backslash() {
        assert_eq!(normalize_rel("data\\menu").unwrap(), "data/menu");
    }
}
