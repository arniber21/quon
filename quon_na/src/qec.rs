//! QEC logical-op layer: code blocks and per-family overhead formulas.
//!
//! Sizing formulas and [`CodeFamily`] live in [`quon_qec`] (ADR-0015) and are
//! re-exported here for API stability. Backend-only IR concepts
//! ([`CodeBlock`], [`LogicalOp`]) with no Quon source-language representation
//! remain in this module. [`LogicalQubitId`] is owned by `quon_qec` and
//! re-exported from both this module and [`crate::graph`].
//!
//! Overhead formulas are normative in
//! [`docs/neutral_atom/architecture_model.md`](../../docs/neutral_atom/architecture_model.md)
//! §10 (issue #109).

use serde::{Deserialize, Serialize};

use crate::layout::AtomId;

pub use quon_qec::{
    CodeFamily, LogicalQubitId, NetRate, QecError, atoms_per_logical, ceil_div, repetition_n,
    surface_n,
};

/// Identifier for a code block grouping atoms under one [`CodeFamily`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBlockId(pub u32);

/// Logical-level operations scheduled against code blocks (no decoder).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalOp {
    LogicalX,
    LogicalZ,
    LogicalH,
    LogicalS,
    LogicalT,
    #[serde(rename = "logical_cx")]
    LogicalCX,
    #[serde(rename = "logical_ccz")]
    LogicalCCZ,
    SyndromeRound,
    MeasureLogical,
    ResetLogical,
    MagicStateInjection,
}

/// Group of atoms implementing one or more logical qubits under a [`CodeFamily`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBlock {
    pub id: CodeBlockId,
    pub family: CodeFamily,
    pub logical_qubits: Vec<LogicalQubitId>,
    pub atoms: Vec<AtomId>,
}

/// Expand `logical_qubits` under `family` into a [`CodeBlock`] with consecutive
/// [`AtomId`]s starting at `first_atom_id`.
///
/// Total atom count:
/// - [`CodeFamily::AbstractBlockCode`]: exactly `n` (one `[[n, k, d]]` block;
///   `logical_qubits.len()` must equal `k`).
/// - Other families: `atoms_per_logical(family) * logical_qubits.len()`.
pub fn expand_code_block(
    id: CodeBlockId,
    family: CodeFamily,
    logical_qubits: Vec<LogicalQubitId>,
    first_atom_id: u32,
) -> Result<CodeBlock, QecError> {
    if logical_qubits.is_empty() {
        return Err(QecError::EmptyLogicalQubits);
    }

    let total = match &family {
        CodeFamily::AbstractBlockCode { n, k, .. } => {
            // Validate n/k via the per-logical formula, then size the block to n.
            let _ = atoms_per_logical(&family)?;
            let actual = logical_qubits.len();
            if actual != usize::try_from(*k).map_err(|_| QecError::AtomCountOverflow)? {
                return Err(QecError::LogicalQubitCountMismatch {
                    expected: *k,
                    actual,
                });
            }
            *n
        }
        _ => {
            let per_logical = atoms_per_logical(&family)?;
            let logical_count =
                u32::try_from(logical_qubits.len()).map_err(|_| QecError::AtomCountOverflow)?;
            per_logical
                .checked_mul(logical_count)
                .ok_or(QecError::AtomCountOverflow)?
        }
    };
    let last_exclusive = first_atom_id
        .checked_add(total)
        .ok_or(QecError::AtomCountOverflow)?;

    let atoms = (first_atom_id..last_exclusive).map(AtomId).collect();

    Ok(CodeBlock {
        id,
        family,
        logical_qubits,
        atoms,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn surface_code_overhead_pins() {
        assert_eq!(surface_n(3), Ok(17));
        assert_eq!(surface_n(5), Ok(49));
        assert_eq!(
            atoms_per_logical(&CodeFamily::SurfaceCodeLike { distance: 3 }),
            Ok(17)
        );
        assert_eq!(
            atoms_per_logical(&CodeFamily::SurfaceCodeLike { distance: 5 }),
            Ok(49)
        );
    }

    #[test]
    fn surface_code_rejects_even_or_small_distance() {
        assert_eq!(
            surface_n(2),
            Err(QecError::InvalidSurfaceDistance { distance: 2 })
        );
        assert_eq!(
            surface_n(4),
            Err(QecError::InvalidSurfaceDistance { distance: 4 })
        );
        assert_eq!(
            surface_n(1),
            Err(QecError::InvalidSurfaceDistance { distance: 1 })
        );
    }

    #[test]
    fn repetition_code_overhead_pins() {
        assert_eq!(repetition_n(3), Ok(5));
        assert_eq!(repetition_n(5), Ok(9));
        assert_eq!(
            atoms_per_logical(&CodeFamily::RepetitionCodeToy { distance: 3 }),
            Ok(5)
        );
        assert_eq!(
            atoms_per_logical(&CodeFamily::RepetitionCodeToy { distance: 5 }),
            Ok(9)
        );
    }

    #[test]
    fn qldpc_net_rate_one_over_twenty_four() {
        let family = CodeFamily::HighRateQldpcLike {
            net_rate: NetRate {
                numerator: 1,
                denominator: 24,
            },
        };
        assert_eq!(atoms_per_logical(&family), Ok(24));
    }

    #[test]
    fn abstract_block_code_ceil_div() {
        let family = CodeFamily::AbstractBlockCode { n: 7, k: 3, d: 2 };
        // ceil(7/3) = 3
        assert_eq!(atoms_per_logical(&family), Ok(3));
        assert_eq!(ceil_div(7, 3), Ok(3));
        assert_eq!(ceil_div(6, 3), Ok(2));
        assert_eq!(ceil_div(1, 1), Ok(1));
    }

    #[test]
    fn expand_repetition_code_toy_block() {
        let block = expand_code_block(
            CodeBlockId(0),
            CodeFamily::RepetitionCodeToy { distance: 3 },
            vec![LogicalQubitId(0)],
            10,
        );
        let block = match block {
            Ok(block) => block,
            Err(error) => panic!("expand failed: {error}"),
        };
        assert_eq!(block.atoms.len(), 5);
        assert_eq!(
            block.atoms,
            vec![AtomId(10), AtomId(11), AtomId(12), AtomId(13), AtomId(14)]
        );
    }

    #[test]
    fn expand_abstract_block_requires_k_logicals() {
        let err = expand_code_block(
            CodeBlockId(1),
            CodeFamily::AbstractBlockCode { n: 6, k: 2, d: 2 },
            vec![LogicalQubitId(0)],
            0,
        );
        assert_eq!(
            err,
            Err(QecError::LogicalQubitCountMismatch {
                expected: 2,
                actual: 1
            })
        );

        let block = expand_code_block(
            CodeBlockId(1),
            CodeFamily::AbstractBlockCode { n: 6, k: 2, d: 2 },
            vec![LogicalQubitId(0), LogicalQubitId(1)],
            0,
        );
        let block = match block {
            Ok(block) => block,
            Err(error) => panic!("expand failed: {error}"),
        };
        // One [[6, 2, d]] block → exactly n = 6 atoms (not ceil(n/k)*k).
        assert_eq!(block.atoms.len(), 6);
    }

    #[test]
    fn expand_abstract_block_uses_n_when_n_not_divisible_by_k() {
        // ceil(7/3) = 3 per-logical estimate, but the block still has n = 7 atoms.
        assert_eq!(
            atoms_per_logical(&CodeFamily::AbstractBlockCode { n: 7, k: 3, d: 2 }),
            Ok(3)
        );
        let block = expand_code_block(
            CodeBlockId(2),
            CodeFamily::AbstractBlockCode { n: 7, k: 3, d: 2 },
            vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)],
            0,
        );
        let block = match block {
            Ok(block) => block,
            Err(error) => panic!("expand failed: {error}"),
        };
        assert_eq!(block.atoms.len(), 7);
    }

    #[test]
    fn code_family_json_uses_family_tag() {
        let family = CodeFamily::SurfaceCodeLike { distance: 3 };
        let value = match serde_json::to_value(&family) {
            Ok(value) => value,
            Err(error) => panic!("serialize: {error}"),
        };
        assert_eq!(
            value,
            json!({
                "family": "surface_code_like",
                "distance": 3,
            })
        );

        let decoded: CodeFamily = match serde_json::from_value(value) {
            Ok(decoded) => decoded,
            Err(error) => panic!("deserialize: {error}"),
        };
        assert_eq!(decoded, family);
    }

    #[test]
    fn code_family_rejects_unknown_json_fields() {
        let value = json!({
            "family": "repetition_code_toy",
            "distance": 3,
            "extra": true,
        });
        assert!(serde_json::from_value::<CodeFamily>(value).is_err());
    }

    #[test]
    fn logical_op_serializes_snake_case() {
        let value = match serde_json::to_value(LogicalOp::MagicStateInjection) {
            Ok(value) => value,
            Err(error) => panic!("serialize: {error}"),
        };
        assert_eq!(value, json!("magic_state_injection"));

        let cx = match serde_json::to_value(LogicalOp::LogicalCX) {
            Ok(value) => value,
            Err(error) => panic!("serialize: {error}"),
        };
        assert_eq!(cx, json!("logical_cx"));

        let ccz = match serde_json::to_value(LogicalOp::LogicalCCZ) {
            Ok(value) => value,
            Err(error) => panic!("serialize: {error}"),
        };
        assert_eq!(ccz, json!("logical_ccz"));
    }
}
