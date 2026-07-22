//! QEC report-layer sizing: code blocks derived from the production
//! expansion IR (ADR-0015 / ADR-0030).
//!
//! Family sizing formulas and [`CodeFamily`] live in [`quon_qec`] (ADR-0015)
//! and are re-exported here for API stability. The single expansion narrative
//! is [`quon_qec::expand_workload`] → [`quon_qec::ExpandedWorkload`]; this
//! module derives report-shaped [`CodeBlock`]s from that IR via
//! [`code_blocks_from_expanded`]. The legacy parallel era (`LogicalOp` /
//! `expand_code_block`) has been retired (issue #319) — there is no second
//! "code block expansion" path; report sizing flows from the production
//! expansion IR.
//!
//! [`LogicalQubitId`] is owned by `quon_qec` and re-exported from both this
//! module and [`crate::graph`].
//!
//! Overhead formulas are normative in
//! [`docs/neutral_atom/architecture_model.md`](../../docs/neutral_atom/architecture_model.md)
//! §10 (issue #109).
//!
//! # Legacy surface removed (issue #319)
//!
//! The toy expander `expand_code_block` and the `LogicalOp` enum were removed.
//! Sizing now flows solely from the production expansion IR:
//!
//! ```compile_fail
//! // Does not compile: `expand_code_block` and `LogicalOp` no longer exist.
//! use quon_na::qec::{expand_code_block, LogicalOp};
//! # fn main() {
//! let _ = expand_code_block;
//! let _ = LogicalOp::LogicalX;
//! # }
//! ```

use serde::{Deserialize, Serialize};

use crate::layout::AtomId;
use quon_qec::{ExpandedBlock, ExpandedWorkload};

pub use quon_qec::{
    CodeFamily, LogicalQubitId, NetRate, QecError, atoms_per_logical, ceil_div, repetition_n,
    surface_n,
};

/// Identifier for a code block grouping atoms under one [`CodeFamily`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBlockId(pub u32);

/// Group of atoms implementing one or more logical qubits under a [`CodeFamily`].
///
/// Produced by [`code_blocks_from_expanded`] from the production expansion IR
/// ([`quon_qec::ExpandedWorkload`]) and consumed by the report layer
/// ([`crate::report::ResourceReport::with_code_blocks`]). One [`CodeBlock`]
/// per expanded logical qubit. This is a report-sizing view of the expansion
/// IR, not an independent expansion path (ADR-0030).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBlock {
    pub id: CodeBlockId,
    pub family: CodeFamily,
    pub logical_qubits: Vec<LogicalQubitId>,
    pub atoms: Vec<AtomId>,
}

/// Derive report-shaped [`CodeBlock`]s from a production [`ExpandedWorkload`].
///
/// Each [`ExpandedBlock`] (one per logical qubit in the expansion IR) becomes
/// one [`CodeBlock`], mapping physical atom ids onto neutral-atom [`AtomId`]s.
/// This is the single expansion narrative (ADR-0015 / ADR-0030): report sizing
/// flows from [`quon_qec::expand_workload`], never from a parallel toy
/// expander. Used by the hybrid QEC schedule path
/// ([`crate::qec_schedule::run_from_qec_workload`]) and by report fixtures.
pub fn code_blocks_from_expanded(expanded: &ExpandedWorkload) -> Vec<CodeBlock> {
    expanded
        .blocks
        .iter()
        .map(|b: &ExpandedBlock| CodeBlock {
            id: CodeBlockId(b.logical_id.0),
            family: b.code_family.clone(),
            logical_qubits: vec![b.logical_id],
            atoms: b.atoms.iter().map(|a| AtomId(a.0)).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use quon_qec::{LogicalBasis, SourceFamily, WorkloadBuilder, expand_workload};

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

    /// Regression: `code_blocks_from_expanded` reproduces the atom layout the
    /// retired `expand_code_block` produced for a distance-3 repetition block
    /// (5 atoms = `2d − 1`, consecutive ids from 0), proving the migration is
    /// behavior-preserving for the report path.
    #[test]
    fn code_blocks_from_expanded_repetition_d3_matches_legacy_layout() {
        let mut builder = WorkloadBuilder::new();
        builder
            .construct(
                SourceFamily::Repetition,
                3,
                LogicalBasis::Z,
                LogicalQubitId(0),
            )
            .expect("construct");
        let expanded = expand_workload(&builder.finish()).expect("expand");
        let blocks = code_blocks_from_expanded(&expanded);

        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(block.id, CodeBlockId(0));
        assert_eq!(block.family, CodeFamily::RepetitionCodeToy { distance: 3 });
        assert_eq!(block.logical_qubits, vec![LogicalQubitId(0)]);
        assert_eq!(block.atoms.len(), 5);
        assert_eq!(
            block.atoms,
            vec![AtomId(0), AtomId(1), AtomId(2), AtomId(3), AtomId(4)]
        );
    }

    #[test]
    fn code_blocks_from_expanded_surface_d3() {
        let mut builder = WorkloadBuilder::new();
        builder
            .construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        let expanded = expand_workload(&builder.finish()).expect("expand");
        let blocks = code_blocks_from_expanded(&expanded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].family,
            CodeFamily::SurfaceCodeLike { distance: 3 }
        );
        assert_eq!(blocks[0].atoms.len(), 17);
    }

    /// Multiple logical qubits expand into one [`CodeBlock`] each, with atoms
    /// allocated consecutively across blocks (the production narrative).
    #[test]
    fn code_blocks_from_expanded_multiple_logicals_are_sequential() {
        let mut builder = WorkloadBuilder::new();
        builder
            .construct(
                SourceFamily::Repetition,
                3,
                LogicalBasis::Z,
                LogicalQubitId(0),
            )
            .expect("construct 0");
        builder
            .construct(
                SourceFamily::Repetition,
                5,
                LogicalBasis::Z,
                LogicalQubitId(1),
            )
            .expect("construct 1");
        let expanded = expand_workload(&builder.finish()).expect("expand");
        let blocks = code_blocks_from_expanded(&expanded);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].atoms.len(), 5);
        assert_eq!(blocks[1].atoms.len(), 9);
        // Sequential allocation: second block begins where the first ended.
        assert_eq!(blocks[1].atoms.first(), Some(&AtomId(5)));
        assert_eq!(blocks[1].atoms.last(), Some(&AtomId(13)));
    }

    /// Sizing-only families (no physical round expansion) still size a
    /// [`CodeBlock`] via the `quon_qec` formula — the report path for families
    /// without an expansion IR. This is the replacement for the retired
    /// `expand_code_block`'s sizing role for qLDPC / abstract blocks.
    #[test]
    fn qldpc_sizing_via_formula_matches_legacy_counts() {
        let family = CodeFamily::HighRateQldpcLike {
            net_rate: NetRate {
                numerator: 1,
                denominator: 24,
            },
        };
        let per = atoms_per_logical(&family).expect("atoms_per_logical");
        assert_eq!(per, 24);
        let logicals: Vec<_> = (0..12).map(LogicalQubitId).collect();
        // Same total the retired `expand_code_block` produced: 24 * 12 = 288.
        assert_eq!(usize::try_from(per).unwrap() * logicals.len(), 288);
    }
}
