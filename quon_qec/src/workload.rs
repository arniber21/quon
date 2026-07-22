//! MLIR-free QEC workload IR (ADR-0015).
//!
//! Collected from `quantum.dynamic` QEC ops after lowering. Hybrid
//! schedule expansion into `quantum.na` lives in [`crate::expand`] (#248).

use std::collections::HashSet;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::family::{CodeFamily, SourceFamily};

/// Backend/IR identifier for one encoded logical qubit after QEC lowering.
///
/// Single source of truth for CONTEXT's "Logical qubit"; re-exported from
/// `quon_na` (graph / qec layers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalQubitId(pub u32);

/// Logical Pauli basis for preparation or measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum LogicalBasis {
    X,
    #[default]
    Z,
}

impl LogicalBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            LogicalBasis::X => "x",
            LogicalBasis::Z => "z",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "x" | "X" => Some(LogicalBasis::X),
            "z" | "Z" => Some(LogicalBasis::Z),
            _ => None,
        }
    }
}

/// One QEC builtin in program order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum WorkloadOp {
    /// Allocate / prepare a logical block (`repetition_code` / `surface_code` / `surface_code_x`).
    Construct {
        family: SourceFamily,
        distance: u32,
        basis: LogicalBasis,
        logical_id: LogicalQubitId,
    },
    /// One syndrome-extraction (`memory_round`) cycle.
    MemoryRound { logical_id: LogicalQubitId },
    /// Consume a block with a logical Pauli measurement.
    MeasureLogical {
        logical_id: LogicalQubitId,
        basis: LogicalBasis,
    },
    /// Surface-only logical CX (lattice-surgery expand in [`crate::lattice_surgery`]).
    LogicalCx {
        control: LogicalQubitId,
        target: LogicalQubitId,
    },
    /// Magic-state-consuming logical T gate (issue #283).
    ///
    /// Consumes one magic-state resource and applies T to the block.
    /// Surface-code only. This is a compiler model of magic-state
    /// consumption, not a validated distillation factory.
    LogicalT { logical_id: LogicalQubitId },
    /// Magic-state-consuming logical T† gate (issue #283).
    LogicalTdag { logical_id: LogicalQubitId },
    /// Magic-state-consuming logical CCZ gate (issue #283).
    ///
    /// Consumes one magic-state resource and applies CCZ to three blocks.
    /// Surface-code only.
    LogicalCcz {
        a: LogicalQubitId,
        b: LogicalQubitId,
        c: LogicalQubitId,
    },
}

/// Per-block metadata recovered from constructors (and validated against later ops).
///
/// `code_family` is always derived from `family` + `distance` at construct /
/// deserialize time (never an independent wire SoT).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkloadBlock {
    pub logical_id: LogicalQubitId,
    pub family: SourceFamily,
    pub distance: u32,
    pub init_basis: LogicalBasis,
    /// Backend sizing family (`RepetitionCodeToy` / `SurfaceCodeLike`).
    pub code_family: CodeFamily,
}

impl<'de> Deserialize<'de> for WorkloadBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            logical_id: LogicalQubitId,
            family: SourceFamily,
            distance: u32,
            init_basis: LogicalBasis,
            #[serde(default)]
            code_family: Option<CodeFamily>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let derived = raw
            .family
            .to_code_family(raw.distance)
            .map_err(de::Error::custom)?;
        if let Some(ref claimed) = raw.code_family
            && claimed != &derived
        {
            return Err(de::Error::custom(
                "code_family inconsistent with family/distance",
            ));
        }
        if raw.family == SourceFamily::Repetition && raw.init_basis == LogicalBasis::X {
            return Err(de::Error::custom(
                "repetition code does not support X-basis init",
            ));
        }
        Ok(WorkloadBlock {
            logical_id: raw.logical_id,
            family: raw.family,
            distance: raw.distance,
            init_basis: raw.init_basis,
            code_family: derived,
        })
    }
}

/// Ordered QEC workload collected from a `run { }` / `quantum.dynamic` program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QecWorkload {
    pub blocks: Vec<WorkloadBlock>,
    pub ops: Vec<WorkloadOp>,
}

impl QecWorkload {
    /// Empty workload (no QEC builtins).
    pub fn empty() -> Self {
        Self {
            blocks: Vec::new(),
            ops: Vec::new(),
        }
    }

    /// Number of `memory_round` ops in program order.
    pub fn memory_round_count(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| matches!(op, WorkloadOp::MemoryRound { .. }))
            .count()
    }

    /// Look up block metadata by logical id.
    pub fn block(&self, id: LogicalQubitId) -> Option<&WorkloadBlock> {
        self.blocks.iter().find(|b| b.logical_id == id)
    }
}

/// Failures while assembling or validating a [`QecWorkload`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WorkloadError {
    #[error("unknown logical qubit id {0}")]
    UnknownLogicalId(u32),
    #[error("duplicate construct for logical qubit {0}")]
    DuplicateConstruct(u32),
    #[error("logical qubit {0} was already measured")]
    AlreadyMeasured(u32),
    #[error("use of logical qubit {0} after it was measured")]
    UseAfterMeasure(u32),
    #[error(
        "unsupported QEC combination: `{op}` is not valid for family `{family}` (distance {distance})"
    )]
    UnsupportedCombo {
        op: &'static str,
        family: &'static str,
        distance: u32,
    },
    #[error("logical_cx requires surface-code blocks; logical {id} is `{family}`")]
    LogicalCxNotSurface { id: u32, family: &'static str },
    #[error(
        "logical_cx requires equal distances; control distance {control_distance}, target distance {target_distance}"
    )]
    LogicalCxDistanceMismatch {
        control_distance: u32,
        target_distance: u32,
    },
    #[error("logical_cx control and target must be distinct; both are {0}")]
    LogicalCxSameLogical(u32),
    #[error("`{op}` requires surface-code blocks; logical {id} is `{family}`")]
    NonCliffordNotSurface {
        op: &'static str,
        id: u32,
        family: &'static str,
    },
    #[error("logical_ccz requires three distinct logical ids")]
    LogicalCczNotDistinct,
    #[error("logical_ccz requires equal distances on all three blocks")]
    LogicalCczDistanceMismatch,
    #[error("invalid code-family distance: {0}")]
    InvalidFamily(#[from] crate::family::QecError),
}

/// Builder that records ops in order and validates family/op combinations.
#[derive(Debug, Default)]
pub struct WorkloadBuilder {
    blocks: Vec<WorkloadBlock>,
    ops: Vec<WorkloadOp>,
    /// Logical ids that have been constructed and not yet measured.
    live: HashSet<LogicalQubitId>,
}

impl WorkloadBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn construct(
        &mut self,
        family: SourceFamily,
        distance: u32,
        basis: LogicalBasis,
        logical_id: LogicalQubitId,
    ) -> Result<(), WorkloadError> {
        if self.blocks.iter().any(|b| b.logical_id == logical_id) {
            return Err(WorkloadError::DuplicateConstruct(logical_id.0));
        }
        // Repetition has no X-basis constructor in source (ADR-0014).
        if family == SourceFamily::Repetition && basis == LogicalBasis::X {
            return Err(WorkloadError::UnsupportedCombo {
                op: "construct_x",
                family: family.as_str(),
                distance,
            });
        }
        let code_family = family.to_code_family(distance)?;
        self.blocks.push(WorkloadBlock {
            logical_id,
            family,
            distance,
            init_basis: basis,
            code_family,
        });
        self.live.insert(logical_id);
        self.ops.push(WorkloadOp::Construct {
            family,
            distance,
            basis,
            logical_id,
        });
        Ok(())
    }

    pub fn memory_round(&mut self, logical_id: LogicalQubitId) -> Result<(), WorkloadError> {
        self.require_live(logical_id)?;
        self.ops.push(WorkloadOp::MemoryRound { logical_id });
        Ok(())
    }

    pub fn measure_logical(
        &mut self,
        logical_id: LogicalQubitId,
        basis: LogicalBasis,
    ) -> Result<(), WorkloadError> {
        if self.blocks.iter().all(|b| b.logical_id != logical_id) {
            return Err(WorkloadError::UnknownLogicalId(logical_id.0));
        }
        if !self.live.contains(&logical_id) {
            return Err(WorkloadError::AlreadyMeasured(logical_id.0));
        }
        self.live.remove(&logical_id);
        self.ops
            .push(WorkloadOp::MeasureLogical { logical_id, basis });
        Ok(())
    }

    pub fn logical_cx(
        &mut self,
        control: LogicalQubitId,
        target: LogicalQubitId,
    ) -> Result<(), WorkloadError> {
        if control == target {
            return Err(WorkloadError::LogicalCxSameLogical(control.0));
        }
        let a = self.require_live(control)?.clone();
        let b = self.require_live(target)?.clone();
        if a.family != SourceFamily::Surface {
            return Err(WorkloadError::LogicalCxNotSurface {
                id: control.0,
                family: a.family.as_str(),
            });
        }
        if b.family != SourceFamily::Surface {
            return Err(WorkloadError::LogicalCxNotSurface {
                id: target.0,
                family: b.family.as_str(),
            });
        }
        if a.distance != b.distance {
            return Err(WorkloadError::LogicalCxDistanceMismatch {
                control_distance: a.distance,
                target_distance: b.distance,
            });
        }
        self.ops.push(WorkloadOp::LogicalCx { control, target });
        Ok(())
    }

    /// Record a magic-state-consuming logical T gate (issue #283).
    ///
    /// Surface-code only. The block remains live (T does not consume the block).
    pub fn logical_t(&mut self, logical_id: LogicalQubitId) -> Result<(), WorkloadError> {
        let block = self.require_live(logical_id)?.clone();
        if block.family != SourceFamily::Surface {
            return Err(WorkloadError::NonCliffordNotSurface {
                op: "logical_t",
                id: logical_id.0,
                family: block.family.as_str(),
            });
        }
        self.ops.push(WorkloadOp::LogicalT { logical_id });
        Ok(())
    }

    /// Record a magic-state-consuming logical T† gate (issue #283).
    pub fn logical_tdag(&mut self, logical_id: LogicalQubitId) -> Result<(), WorkloadError> {
        let block = self.require_live(logical_id)?.clone();
        if block.family != SourceFamily::Surface {
            return Err(WorkloadError::NonCliffordNotSurface {
                op: "logical_tdag",
                id: logical_id.0,
                family: block.family.as_str(),
            });
        }
        self.ops.push(WorkloadOp::LogicalTdag { logical_id });
        Ok(())
    }

    /// Record a magic-state-consuming logical CCZ gate (issue #283).
    ///
    /// Surface-code only. All three blocks must be surface-code at the same
    /// distance. Blocks remain live.
    pub fn logical_ccz(
        &mut self,
        a: LogicalQubitId,
        b: LogicalQubitId,
        c: LogicalQubitId,
    ) -> Result<(), WorkloadError> {
        if a == b || b == c || a == c {
            return Err(WorkloadError::LogicalCczNotDistinct);
        }
        let ba = self.require_live(a)?.clone();
        let bb = self.require_live(b)?.clone();
        let bc = self.require_live(c)?.clone();
        for (id, block) in [(a, &ba), (b, &bb), (c, &bc)] {
            if block.family != SourceFamily::Surface {
                return Err(WorkloadError::NonCliffordNotSurface {
                    op: "logical_ccz",
                    id: id.0,
                    family: block.family.as_str(),
                });
            }
        }
        if ba.distance != bb.distance || bb.distance != bc.distance {
            return Err(WorkloadError::LogicalCczDistanceMismatch);
        }
        self.ops.push(WorkloadOp::LogicalCcz { a, b, c });
        Ok(())
    }

    pub fn finish(self) -> QecWorkload {
        QecWorkload {
            blocks: self.blocks,
            ops: self.ops,
        }
    }

    fn require_live(&self, id: LogicalQubitId) -> Result<&WorkloadBlock, WorkloadError> {
        let block = self
            .blocks
            .iter()
            .find(|b| b.logical_id == id)
            .ok_or(WorkloadError::UnknownLogicalId(id.0))?;
        if !self.live.contains(&id) {
            return Err(WorkloadError::UseAfterMeasure(id.0));
        }
        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::CodeFamily;

    #[test]
    fn repetition_memory_ctor_rounds_measure_order_and_metadata() {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("round 1");
        b.memory_round(LogicalQubitId(0)).expect("round 2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("measure");
        let w = b.finish();

        assert_eq!(w.blocks.len(), 1);
        let block = &w.blocks[0];
        assert_eq!(block.family, SourceFamily::Repetition);
        assert_eq!(block.distance, 3);
        assert_eq!(block.init_basis, LogicalBasis::Z);
        assert_eq!(
            block.code_family,
            CodeFamily::RepetitionCodeToy { distance: 3 }
        );
        assert_eq!(w.memory_round_count(), 2);
        assert_eq!(
            w.ops,
            vec![
                WorkloadOp::Construct {
                    family: SourceFamily::Repetition,
                    distance: 3,
                    basis: LogicalBasis::Z,
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::MemoryRound {
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::MemoryRound {
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::MeasureLogical {
                    logical_id: LogicalQubitId(0),
                    basis: LogicalBasis::Z,
                },
            ]
        );
    }

    #[test]
    fn surface_workload_with_logical_cx_full_order_and_metadata() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("a");
        b.construct(SourceFamily::Surface, 3, LogicalBasis::X, LogicalQubitId(1))
            .expect("b");
        b.logical_cx(LogicalQubitId(0), LogicalQubitId(1))
            .expect("cx");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz a");
        b.measure_logical(LogicalQubitId(1), LogicalBasis::X)
            .expect("mx b");
        let w = b.finish();

        assert_eq!(w.blocks.len(), 2);
        let a = w.block(LogicalQubitId(0)).expect("block 0");
        assert_eq!(a.family, SourceFamily::Surface);
        assert_eq!(a.distance, 3);
        assert_eq!(a.init_basis, LogicalBasis::Z);
        assert_eq!(a.code_family, CodeFamily::SurfaceCodeLike { distance: 3 });
        let b_block = w.block(LogicalQubitId(1)).expect("block 1");
        assert_eq!(b_block.family, SourceFamily::Surface);
        assert_eq!(b_block.distance, 3);
        assert_eq!(b_block.init_basis, LogicalBasis::X);
        assert_eq!(
            b_block.code_family,
            CodeFamily::SurfaceCodeLike { distance: 3 }
        );
        assert_eq!(
            w.ops,
            vec![
                WorkloadOp::Construct {
                    family: SourceFamily::Surface,
                    distance: 3,
                    basis: LogicalBasis::Z,
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::Construct {
                    family: SourceFamily::Surface,
                    distance: 3,
                    basis: LogicalBasis::X,
                    logical_id: LogicalQubitId(1),
                },
                WorkloadOp::LogicalCx {
                    control: LogicalQubitId(0),
                    target: LogicalQubitId(1),
                },
                WorkloadOp::MeasureLogical {
                    logical_id: LogicalQubitId(0),
                    basis: LogicalBasis::Z,
                },
                WorkloadOp::MeasureLogical {
                    logical_id: LogicalQubitId(1),
                    basis: LogicalBasis::X,
                },
            ]
        );
    }

    #[test]
    fn unsupported_repetition_x_construct() {
        let mut b = WorkloadBuilder::new();
        let err = b
            .construct(
                SourceFamily::Repetition,
                3,
                LogicalBasis::X,
                LogicalQubitId(0),
            )
            .expect_err("x init");
        assert!(matches!(
            err,
            WorkloadError::UnsupportedCombo {
                op: "construct_x",
                family: "repetition",
                ..
            }
        ));
    }

    #[test]
    fn logical_cx_rejects_repetition() {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        assert_eq!(
            b.logical_cx(LogicalQubitId(0), LogicalQubitId(1)),
            Err(WorkloadError::LogicalCxNotSurface {
                id: 0,
                family: "repetition",
            })
        );
    }

    #[test]
    fn logical_cx_rejects_distance_mismatch_and_same_logical() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 5, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        assert_eq!(
            b.logical_cx(LogicalQubitId(0), LogicalQubitId(1)),
            Err(WorkloadError::LogicalCxDistanceMismatch {
                control_distance: 3,
                target_distance: 5,
            })
        );

        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        assert_eq!(
            b.logical_cx(LogicalQubitId(0), LogicalQubitId(0)),
            Err(WorkloadError::LogicalCxSameLogical(0))
        );
    }

    #[test]
    fn rejects_use_after_measure_and_double_measure() {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        assert_eq!(
            b.memory_round(LogicalQubitId(0)),
            Err(WorkloadError::UseAfterMeasure(0))
        );
        assert_eq!(
            b.measure_logical(LogicalQubitId(0), LogicalBasis::Z),
            Err(WorkloadError::AlreadyMeasured(0))
        );
    }

    #[test]
    fn workload_serde_denies_unknown_fields() {
        let value = serde_json::json!({
            "blocks": [],
            "ops": [],
            "extra": true,
        });
        assert!(serde_json::from_value::<QecWorkload>(value).is_err());
    }

    #[test]
    fn workload_block_serde_rejects_inconsistent_code_family() {
        let value = serde_json::json!({
            "logical_id": 0,
            "family": "repetition",
            "distance": 3,
            "init_basis": "z",
            "code_family": { "family": "surface_code_like", "distance": 5 },
        });
        assert!(serde_json::from_value::<WorkloadBlock>(value).is_err());
    }

    #[test]
    fn workload_block_serde_derives_code_family() {
        let value = serde_json::json!({
            "logical_id": 0,
            "family": "surface",
            "distance": 3,
            "init_basis": "x",
        });
        let block: WorkloadBlock = serde_json::from_value(value).expect("derive");
        assert_eq!(
            block.code_family,
            CodeFamily::SurfaceCodeLike { distance: 3 }
        );
    }

    #[test]
    fn logical_t_records_magic_state_consumption() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.logical_t(LogicalQubitId(0)).unwrap();
        let w = b.finish();
        assert_eq!(w.ops.len(), 2);
        assert!(matches!(
            w.ops[1],
            WorkloadOp::LogicalT {
                logical_id: LogicalQubitId(0)
            }
        ));
    }

    #[test]
    fn logical_tdag_records_magic_state_consumption() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.logical_tdag(LogicalQubitId(0)).unwrap();
        let w = b.finish();
        assert!(matches!(w.ops[1], WorkloadOp::LogicalTdag { .. }));
    }

    #[test]
    fn logical_t_rejects_repetition() {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .unwrap();
        assert_eq!(
            b.logical_t(LogicalQubitId(0)),
            Err(WorkloadError::NonCliffordNotSurface {
                op: "logical_t",
                id: 0,
                family: "repetition"
            })
        );
    }

    #[test]
    fn logical_ccz_requires_three_distinct_surface_blocks() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(2))
            .unwrap();
        b.logical_ccz(LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2))
            .unwrap();
        let w = b.finish();
        assert!(matches!(w.ops.last(), Some(WorkloadOp::LogicalCcz { .. })));
    }

    #[test]
    fn logical_ccz_rejects_non_distinct() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        assert_eq!(
            b.logical_ccz(LogicalQubitId(0), LogicalQubitId(0), LogicalQubitId(1)),
            Err(WorkloadError::LogicalCczNotDistinct)
        );
    }

    #[test]
    fn logical_ccz_rejects_repetition() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(1),
        )
        .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(2))
            .unwrap();
        assert_eq!(
            b.logical_ccz(LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)),
            Err(WorkloadError::NonCliffordNotSurface {
                op: "logical_ccz",
                id: 1,
                family: "repetition"
            })
        );
    }
}
