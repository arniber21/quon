// JSON loader for BackendTarget descriptors — see issue #3, SPEC.md §8.3

use crate::target::BackendTarget;
use anyhow::Result;

pub fn load(path: &std::path::Path) -> Result<BackendTarget> {
    let src = std::fs::read_to_string(path)?;
    let target: BackendTarget = serde_json::from_str(&src)?;
    Ok(target)
}
