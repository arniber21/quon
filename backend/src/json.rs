// JSON loader for BackendTarget descriptors — see issue #3, SPEC.md §8.3.
//
// Parses the §8.3 wire format into a `TargetDescriptor`, then converts to the
// domain `BackendTarget`. All errors are typed `BackendError`; this is the
// untrusted-input boundary, so it never panics, `unwrap`s, or `expect`s.

use std::path::Path;

use crate::descriptor::{FixedTargetDescriptor, NeutralAtomTargetDescriptor, TargetDescriptor};
use crate::error::BackendError;
use crate::target::BackendTarget;

/// Load a target descriptor from a JSON file at `path`.
pub fn load(path: &Path) -> Result<BackendTarget, BackendError> {
    let src = std::fs::read_to_string(path)?;
    from_str(&src)
}

/// Parse a target descriptor from a JSON string. Primary entry point for tests
/// and fuzzing.
pub fn from_str(src: &str) -> Result<BackendTarget, BackendError> {
    let value: serde_json::Value = serde_json::from_str(src)?;
    let descriptor = match value.get("kind").and_then(serde_json::Value::as_str) {
        Some("neutral_atom_reconfigurable") => {
            TargetDescriptor::NeutralAtomReconfigurable(serde_json::from_value::<
                NeutralAtomTargetDescriptor,
            >(value)?)
        }
        Some("fixed") | None => {
            TargetDescriptor::Fixed(serde_json::from_value::<FixedTargetDescriptor>(value)?)
        }
        Some(kind) => return Err(BackendError::UnknownTargetKind(kind.to_owned())),
    };
    BackendTarget::try_from(descriptor)
}
