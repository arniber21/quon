//! MLIR-free QEC workload IR (ADR-0015).
//!
//! Collected from `quantum.dynamic` QEC ops after monadic lowering. Hybrid
//! schedule expansion into `quantum.na` is issue #248 — this module stops at
//! the ordered op list plus per-block metadata.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::family::{CodeFamily, SourceFamily};

/// Backend/IR identifier for one encoded logical qubit after QEC lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalQubitId(pub u32);

/// Logical Pauli basis for preparation or measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalBasis {
    X,
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
    /// Surface-only logical CX (may remain a stub until #248 expands it).
    LogicalCx {
        control: LogicalQubitId,
        target: LogicalQubitId,
    },
}

/// Per-block metadata recovered from constructors (and validated against later ops).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadBlock {
    pub logical_id: LogicalQubitId,
    pub family: SourceFamily,
    pub distance: u32,
    pub init_basis: LogicalBasis,
    /// Backend sizing family (`RepetitionCodeToy` / `SurfaceCodeLike`).
    pub code_family: CodeFamily,
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
    #[error(
        "unsupported QEC combination: `{op}` is not valid for family `{family}` (distance {distance})"
    )]
    UnsupportedCombo {
        op: &'static str,
        family: &'static str,
        distance: u32,
    },
    #[error("logical_cx requires two distinct surface-code blocks at equal distance")]
    LogicalCxFamilyMismatch,
    #[error("invalid code-family distance: {0}")]
    InvalidFamily(#[from] crate::family::QecError),
}

/// Builder that records ops in order and validates family/op combinations.
#[derive(Debug, Default)]
pub struct WorkloadBuilder {
    blocks: Vec<WorkloadBlock>,
    ops: Vec<WorkloadOp>,
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
        self.ops.push(WorkloadOp::Construct {
            family,
            distance,
            basis,
            logical_id,
        });
        Ok(())
    }

    pub fn memory_round(&mut self, logical_id: LogicalQubitId) -> Result<(), WorkloadError> {
        self.require_block(logical_id)?;
        self.ops.push(WorkloadOp::MemoryRound { logical_id });
        Ok(())
    }

    pub fn measure_logical(
        &mut self,
        logical_id: LogicalQubitId,
        basis: LogicalBasis,
    ) -> Result<(), WorkloadError> {
        self.require_block(logical_id)?;
        self.ops.push(WorkloadOp::MeasureLogical { logical_id, basis });
        Ok(())
    }

    pub fn logical_cx(
        &mut self,
        control: LogicalQubitId,
        target: LogicalQubitId,
    ) -> Result<(), WorkloadError> {
        let a = self.require_block(control)?;
        let b = self.require_block(target)?;
        if a.family != SourceFamily::Surface
            || b.family != SourceFamily::Surface
            || a.distance != b.distance
            || control == target
        {
            return Err(WorkloadError::LogicalCxFamilyMismatch);
        }
        self.ops.push(WorkloadOp::LogicalCx { control, target });
        Ok(())
    }

    pub fn finish(self) -> QecWorkload {
        QecWorkload {
            blocks: self.blocks,
            ops: self.ops,
        }
    }

    fn require_block(&self, id: LogicalQubitId) -> Result<&WorkloadBlock, WorkloadError> {
        self.blocks
            .iter()
            .find(|b| b.logical_id == id)
            .ok_or(WorkloadError::UnknownLogicalId(id.0))
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
    fn surface_workload_with_logical_cx() {
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
        assert_eq!(
            w.block(LogicalQubitId(1)).map(|bl| bl.code_family.clone()),
            Some(CodeFamily::SurfaceCodeLike { distance: 3 })
        );
        assert!(matches!(
            w.ops[2],
            WorkloadOp::LogicalCx {
                control: LogicalQubitId(0),
                target: LogicalQubitId(1),
            }
        ));
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
            Err(WorkloadError::LogicalCxFamilyMismatch)
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
}
