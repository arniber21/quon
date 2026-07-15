//! Hybrid QEC expansion: workload → per-round physical gate graphs (ADR-0016).
//!
//! Produces MLIR-free physical rounds that `quon_na` schedules with existing
//! planners *inside* each round. Round boundaries are first-class so NA
//! compaction can attach Barrier edges and must not reorder across memory
//! rounds.
//!
//! Atom layout for [`SourceFamily::Repetition`] / [`CodeFamily::RepetitionCodeToy`]:
//! alternating data/check chain of length `N = 2d − 1` (architecture_model §10.2 /
//! [Kelly15]): `D C D C … D`.
//!
//! Syndrome gadget (Kelly-style bit-flip): each check ancilla starts in `|0⟩`
//! and extracts ZZ parity of its two neighboring data qubits via CNOTs
//! (control = data, target = check), then Z-measure + reset the check.

use thiserror::Error;

use crate::family::{CodeFamily, QecError, SourceFamily, repetition_n};
use crate::workload::{
    LogicalBasis, LogicalQubitId, QecWorkload, WorkloadBlock, WorkloadOp,
};

/// Dense physical atom id within an expanded layout (0-based globally).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAtomId(pub u32);

/// CNOT for syndrome extraction: control (data) → target (check ancilla).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalCnot {
    pub control: PhysicalAtomId,
    pub target: PhysicalAtomId,
}

/// Post-entangling measure or reset on one atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundTerminal {
    Measure {
        atom: PhysicalAtomId,
        basis: LogicalBasis,
    },
    Reset {
        atom: PhysicalAtomId,
    },
}

/// Why this physical round exists in the expanded schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundKind {
    /// Block allocation / preparation (no entangling in v0).
    Construct,
    /// One syndrome-extraction memory round.
    MemoryRound,
    /// Final logical Pauli measurement (consumes the block).
    MeasureLogical,
}

/// One schedulable unit: entangling CNOTs, then terminal measure/reset.
///
/// `quon_na` runs place/entangle/move on `entangling` alone, then appends
/// `terminal` layers and a durable round barrier, then compacts with cuts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalRound {
    pub kind: RoundKind,
    pub logical_id: LogicalQubitId,
    pub entangling: Vec<PhysicalCnot>,
    pub terminal: Vec<RoundTerminal>,
}

/// Per-block atom layout after expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedBlock {
    pub logical_id: LogicalQubitId,
    pub family: SourceFamily,
    pub code_family: CodeFamily,
    pub distance: u32,
    pub init_basis: LogicalBasis,
    /// Full atom chain in layout order.
    pub atoms: Vec<PhysicalAtomId>,
    pub data_atoms: Vec<PhysicalAtomId>,
    pub check_atoms: Vec<PhysicalAtomId>,
}

/// Fully expanded QEC workload ready for hybrid NA scheduling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedWorkload {
    pub blocks: Vec<ExpandedBlock>,
    pub rounds: Vec<PhysicalRound>,
}

impl ExpandedWorkload {
    /// Total physical atoms across all blocks.
    pub fn physical_atom_count(&self) -> usize {
        self.blocks.iter().map(|b| b.atoms.len()).sum()
    }

    /// Number of [`RoundKind::MemoryRound`] rounds.
    pub fn memory_round_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MemoryRound)
            .count()
    }
}

/// Failures expanding a [`QecWorkload`] into physical rounds.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ExpandError {
    #[error(transparent)]
    Qec(#[from] QecError),
    #[error("unknown logical qubit id {0} in workload op")]
    UnknownLogicalId(u32),
    #[error(
        "surface-code / lattice-surgery expansion is out of scope for #248 \
         (see #249 / #250); got family `{family}` distance {distance}"
    )]
    UnsupportedFamily {
        family: &'static str,
        distance: u32,
    },
    #[error("logical_cx expansion is out of scope for #248 (see #250)")]
    LogicalCxUnsupported,
    #[error("expanded atom id overflow")]
    AtomIdOverflow,
    #[error(
        "repetition-code construct requires Z init basis (ADR-0014); got `{basis}` \
         for logical {logical_id}"
    )]
    NonZInitBasis {
        logical_id: u32,
        basis: &'static str,
    },
}

/// Expand a validated [`QecWorkload`] into per-round physical CNOT / measure / reset.
pub fn expand_workload(workload: &QecWorkload) -> Result<ExpandedWorkload, ExpandError> {
    let mut next_atom = 0u32;
    let mut blocks = Vec::with_capacity(workload.blocks.len());
    let mut layouts: Vec<ExpandedBlock> = Vec::with_capacity(workload.blocks.len());

    for meta in &workload.blocks {
        let layout = expand_block_layout(meta, &mut next_atom)?;
        blocks.push(layout.clone());
        layouts.push(layout);
    }

    let mut rounds = Vec::new();
    for op in &workload.ops {
        match op {
            WorkloadOp::Construct {
                logical_id,
                family,
                distance,
                basis,
            } => {
                let layout = find_layout(&layouts, *logical_id)?;
                if *family != SourceFamily::Repetition {
                    return Err(ExpandError::UnsupportedFamily {
                        family: family.as_str(),
                        distance: *distance,
                    });
                }
                if *basis != LogicalBasis::Z || layout.init_basis != LogicalBasis::Z {
                    return Err(ExpandError::NonZInitBasis {
                        logical_id: logical_id.0,
                        basis: basis.as_str(),
                    });
                }
                // Construct allocates; v0 emits no physical gates (state prep is |0…0⟩).
                rounds.push(PhysicalRound {
                    kind: RoundKind::Construct,
                    logical_id: *logical_id,
                    entangling: Vec::new(),
                    terminal: Vec::new(),
                });
            }
            WorkloadOp::MemoryRound { logical_id } => {
                let layout = find_layout(&layouts, *logical_id)?;
                if layout.family != SourceFamily::Repetition {
                    return Err(ExpandError::UnsupportedFamily {
                        family: layout.family.as_str(),
                        distance: layout.distance,
                    });
                }
                rounds.push(repetition_memory_round(layout)?);
            }
            WorkloadOp::MeasureLogical { logical_id, basis } => {
                let layout = find_layout(&layouts, *logical_id)?;
                if layout.family != SourceFamily::Repetition {
                    return Err(ExpandError::UnsupportedFamily {
                        family: layout.family.as_str(),
                        distance: layout.distance,
                    });
                }
                rounds.push(repetition_measure_logical(layout, *basis));
            }
            WorkloadOp::LogicalCx { .. } => {
                return Err(ExpandError::LogicalCxUnsupported);
            }
        }
    }

    Ok(ExpandedWorkload { blocks, rounds })
}

fn find_layout(
    layouts: &[ExpandedBlock],
    id: LogicalQubitId,
) -> Result<&ExpandedBlock, ExpandError> {
    layouts
        .iter()
        .find(|b| b.logical_id == id)
        .ok_or(ExpandError::UnknownLogicalId(id.0))
}

fn expand_block_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    match meta.family {
        SourceFamily::Repetition => expand_repetition_layout(meta, next_atom),
        SourceFamily::Surface => Err(ExpandError::UnsupportedFamily {
            family: "surface",
            distance: meta.distance,
        }),
    }
}

/// Alternating `D C D C … D` chain; `N = 2d − 1`.
fn expand_repetition_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    let n = repetition_n(meta.distance)?;
    let first = *next_atom;
    let last = next_atom
        .checked_add(n)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;

    let atoms: Vec<PhysicalAtomId> = (first..last).map(PhysicalAtomId).collect();
    let mut data_atoms = Vec::with_capacity(meta.distance as usize);
    let mut check_atoms = Vec::with_capacity(meta.distance.saturating_sub(1) as usize);
    for (offset, &atom) in atoms.iter().enumerate() {
        if offset % 2 == 0 {
            data_atoms.push(atom);
        } else {
            check_atoms.push(atom);
        }
    }

    Ok(ExpandedBlock {
        logical_id: meta.logical_id,
        family: meta.family,
        code_family: meta.code_family.clone(),
        distance: meta.distance,
        init_basis: meta.init_basis,
        atoms,
        data_atoms,
        check_atoms,
    })
}

/// Kelly-style bit-flip syndrome: each check `C_i` is the CNOT target of
/// neighboring data `D_i` and `D_{i+1}` (ZZ parity ladder on the alternating chain).
fn repetition_cnot_pattern(layout: &ExpandedBlock) -> Result<Vec<PhysicalCnot>, ExpandError> {
    let d = layout.distance;
    if layout.data_atoms.len() != d as usize
        || layout.check_atoms.len() != d.saturating_sub(1) as usize
    {
        return Err(ExpandError::Qec(QecError::InvalidRepetitionDistance {
            distance: d,
        }));
    }
    let mut cnots = Vec::with_capacity(2 * layout.check_atoms.len());
    for i in 0..layout.check_atoms.len() {
        let check = layout.check_atoms[i];
        let left = layout.data_atoms[i];
        let right = layout.data_atoms[i + 1];
        cnots.push(PhysicalCnot {
            control: left,
            target: check,
        });
        cnots.push(PhysicalCnot {
            control: right,
            target: check,
        });
    }
    Ok(cnots)
}

fn repetition_memory_round(layout: &ExpandedBlock) -> Result<PhysicalRound, ExpandError> {
    let entangling = repetition_cnot_pattern(layout)?;
    let mut terminal = Vec::with_capacity(2 * layout.check_atoms.len());
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        });
    }
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Reset { atom });
    }
    Ok(PhysicalRound {
        kind: RoundKind::MemoryRound,
        logical_id: layout.logical_id,
        entangling,
        terminal,
    })
}

fn repetition_measure_logical(layout: &ExpandedBlock, basis: LogicalBasis) -> PhysicalRound {
    let terminal = layout
        .data_atoms
        .iter()
        .map(|&atom| RoundTerminal::Measure { atom, basis })
        .collect();
    PhysicalRound {
        kind: RoundKind::MeasureLogical,
        logical_id: layout.logical_id,
        entangling: Vec::new(),
        terminal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::WorkloadBuilder;

    fn repetition_d3_two_rounds() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.memory_round(LogicalQubitId(0)).expect("r2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        b.finish()
    }

    #[test]
    fn repetition_d3_layout_is_alternating_five_atoms() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.blocks.len(), 1);
        let block = &expanded.blocks[0];
        assert_eq!(block.distance, 3);
        assert_eq!(
            block.code_family,
            CodeFamily::RepetitionCodeToy { distance: 3 }
        );
        assert_eq!(
            block.atoms,
            vec![
                PhysicalAtomId(0),
                PhysicalAtomId(1),
                PhysicalAtomId(2),
                PhysicalAtomId(3),
                PhysicalAtomId(4),
            ]
        );
        assert_eq!(
            block.data_atoms,
            vec![PhysicalAtomId(0), PhysicalAtomId(2), PhysicalAtomId(4)]
        );
        assert_eq!(
            block.check_atoms,
            vec![PhysicalAtomId(1), PhysicalAtomId(3)]
        );
        assert_eq!(expanded.physical_atom_count(), 5);
        assert_eq!(block.init_basis, LogicalBasis::Z);
    }

    #[test]
    fn repetition_d3_memory_round_cnot_measure_reset_pattern() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.memory_round_count(), 2);

        let memory: Vec<_> = expanded
            .rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MemoryRound)
            .collect();
        assert_eq!(memory.len(), 2);

        // Kelly bit-flip: CNOT(D→C) for each neighboring data.
        let expected_cnot = vec![
            PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(3),
            },
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(3),
            },
        ];
        for round in &memory {
            assert_eq!(round.entangling, expected_cnot);
            assert_eq!(
                round.terminal,
                vec![
                    RoundTerminal::Measure {
                        atom: PhysicalAtomId(1),
                        basis: LogicalBasis::Z,
                    },
                    RoundTerminal::Measure {
                        atom: PhysicalAtomId(3),
                        basis: LogicalBasis::Z,
                    },
                    RoundTerminal::Reset {
                        atom: PhysicalAtomId(1),
                    },
                    RoundTerminal::Reset {
                        atom: PhysicalAtomId(3),
                    },
                ]
            );
        }
    }

    #[test]
    fn repetition_d3_op_order_construct_rounds_measure() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        let kinds: Vec<_> = expanded.rounds.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RoundKind::Construct,
                RoundKind::MemoryRound,
                RoundKind::MemoryRound,
                RoundKind::MeasureLogical,
            ]
        );
        let mz = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MeasureLogical)
            .expect("mz");
        assert!(mz.entangling.is_empty());
        assert_eq!(
            mz.terminal,
            vec![
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(0),
                    basis: LogicalBasis::Z,
                },
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(2),
                    basis: LogicalBasis::Z,
                },
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(4),
                    basis: LogicalBasis::Z,
                },
            ]
        );
    }

    #[test]
    fn surface_construct_is_rejected_in_248() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        let err = expand_workload(&b.finish()).expect_err("surface");
        assert!(matches!(
            err,
            ExpandError::UnsupportedFamily {
                family: "surface",
                distance: 3
            }
        ));
    }

    #[test]
    fn logical_cx_is_rejected_in_248() {
        // Hand-build Repetition blocks + LogicalCx so the CX arm is exercised
        // (WorkloadBuilder refuses CX on non-Surface).
        let workload = QecWorkload {
            blocks: vec![
                WorkloadBlock {
                    logical_id: LogicalQubitId(0),
                    family: SourceFamily::Repetition,
                    distance: 3,
                    init_basis: LogicalBasis::Z,
                    code_family: CodeFamily::RepetitionCodeToy { distance: 3 },
                },
                WorkloadBlock {
                    logical_id: LogicalQubitId(1),
                    family: SourceFamily::Repetition,
                    distance: 3,
                    init_basis: LogicalBasis::Z,
                    code_family: CodeFamily::RepetitionCodeToy { distance: 3 },
                },
            ],
            ops: vec![
                WorkloadOp::Construct {
                    family: SourceFamily::Repetition,
                    distance: 3,
                    basis: LogicalBasis::Z,
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::Construct {
                    family: SourceFamily::Repetition,
                    distance: 3,
                    basis: LogicalBasis::Z,
                    logical_id: LogicalQubitId(1),
                },
                WorkloadOp::LogicalCx {
                    control: LogicalQubitId(0),
                    target: LogicalQubitId(1),
                },
            ],
        };
        let err = expand_workload(&workload).expect_err("logical_cx");
        assert!(matches!(err, ExpandError::LogicalCxUnsupported));
    }

    #[test]
    fn repetition_construct_rejects_non_z_init_basis() {
        let workload = QecWorkload {
            blocks: vec![WorkloadBlock {
                logical_id: LogicalQubitId(0),
                family: SourceFamily::Repetition,
                distance: 3,
                init_basis: LogicalBasis::X,
                code_family: CodeFamily::RepetitionCodeToy { distance: 3 },
            }],
            ops: vec![WorkloadOp::Construct {
                family: SourceFamily::Repetition,
                distance: 3,
                basis: LogicalBasis::X,
                logical_id: LogicalQubitId(0),
            }],
        };
        let err = expand_workload(&workload).expect_err("non-z");
        assert!(matches!(
            err,
            ExpandError::NonZInitBasis {
                logical_id: 0,
                basis: "x"
            }
        ));
    }
}
