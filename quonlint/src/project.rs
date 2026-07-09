use std::path::{Path, PathBuf};

use glob::glob;
use thiserror::Error;

use crate::config::{ConfigError, LintConfig};

#[derive(Debug, Error)]
pub enum LintError {
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("glob pattern `{pattern}` failed: {source}")]
    Glob {
        pattern: String,
        source: glob::PatternError,
    },
}

pub fn discover_qn_files(root: &Path, config: &LintConfig) -> Result<Vec<PathBuf>, LintError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut files = Vec::new();

    for pattern in &config.include {
        let full = root.join(pattern);
        let pattern_str = full.to_string_lossy().to_string();
        for entry in glob(&pattern_str).map_err(|e| LintError::Glob {
            pattern: pattern_str.clone(),
            source: e,
        })? {
            let path = entry.map_err(|e| LintError::Io {
                path: PathBuf::from(&pattern_str),
                source: e.into_error(),
            })?;
            if path.is_file() && !is_excluded(&path, &root, config) {
                files.push(path);
            }
        }
    }

    if files.is_empty() && root.is_file() && root.extension().is_some_and(|e| e == "qn") {
        files.push(root.clone());
    }

    files.sort();
    files.dedup();
    Ok(files)
}

fn is_excluded(path: &Path, root: &Path, config: &LintConfig) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    for pattern in &config.exclude {
        let full = root.join(pattern);
        let pattern_str = full.to_string_lossy().to_string();
        if glob_match(&pattern_str, path)
            || glob_match(pattern, path)
            || rel_str.contains(pattern.trim_end_matches("/**"))
        {
            return true;
        }
    }
    false
}

fn glob_match(pattern: &str, path: &Path) -> bool {
    glob::Pattern::new(pattern)
        .ok()
        .is_some_and(|p| p.matches_path(path))
}
