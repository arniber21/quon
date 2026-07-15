//! Hybrid QEC expansion: workload → per-round physical gate graphs (ADR-0016).
//!
//! Produces MLIR-free physical rounds that `quon_na` schedules with existing
//! planners *inside* each round. Round boundaries are first-class so NA
//! compaction can attach Barrier edges and must not reorder across memory
//! rounds.
//!
//! # Repetition ([`SourceFamily::Repetition`])
//!
//! Alternating `D C D C … D` chain of length `N = 2d − 1` (architecture_model
//! §10.2 / [Kelly15]). Each check extracts ZZ parity of its two neighboring
//! data qubits via CNOT(data→check), then Z-measure + reset.
//!
//! # Surface ([`SourceFamily::Surface`])
//!
//! Rotated surface code with `N = 2d² − 1` (architecture_model §10.1 /
//! [Bravyi24] §1; [BMD07]). Data on a `d×d` grid; X/Z check ancillas on
//! plaquettes (smooth top/bottom X boundaries, rough left/right Z). Memory
//! rounds sandwich X-check Hadamards around CX(check→data), use CX(data→check)
//! for Z-checks, then measure+reset all checks. Lattice surgery / `logical_cx`
//! remains out of scope (#250).

use thiserror::Error;

use crate::family::{CodeFamily, QecError, SourceFamily, repetition_n, surface_n};
use crate::workload::{
    LogicalBasis, LogicalQubitId, QecWorkload, WorkloadBlock, WorkloadOp,
};

/// Dense physical atom id within an expanded layout (0-based globally).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAtomId(pub u32);

/// CNOT for syndrome extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalCnot {
    pub control: PhysicalAtomId,
    pub target: PhysicalAtomId,
}

/// Single-qubit op scheduled around entangling (Stim + expand IR; NA planners
/// skip these — microwave/Raman locals need no AOD place/move).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundLocalOp {
    H { atom: PhysicalAtomId },
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

/// One schedulable unit: optional locals, entangling CNOTs, terminals.
///
/// `quon_na` runs place/entangle/move on `entangling` alone (locals are
/// Stim/IR-only), then appends `terminal` layers and a durable round barrier.
///
/// Surface rounds use a Z-then-X phase split: the first [`Self::z_cnot_count`]
/// CNOTs are Z-plaquette extraction; [`Self::local_mid`] Hadamards then the
/// remaining X-plaquette CNOTs; [`Self::local_after`] closes the X sandwich.
/// Repetition leaves `z_cnot_count == entangling.len()` and mid empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalRound {
    pub kind: RoundKind,
    pub logical_id: LogicalQubitId,
    pub local_before: Vec<RoundLocalOp>,
    pub entangling: Vec<PhysicalCnot>,
    /// Prefix of [`Self::entangling`] belonging to Z-phase extraction.
    pub z_cnot_count: usize,
    pub local_mid: Vec<RoundLocalOp>,
    pub local_after: Vec<RoundLocalOp>,
    pub terminal: Vec<RoundTerminal>,
}

/// One stabilizer generator in an expanded block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StabilizerDef {
    pub check: PhysicalAtomId,
    /// Pauli type of the stabilizer (`X` or `Z` plaquette).
    pub basis: LogicalBasis,
    pub data: Vec<PhysicalAtomId>,
}

/// Per-block atom layout after expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedBlock {
    pub logical_id: LogicalQubitId,
    pub family: SourceFamily,
    pub code_family: CodeFamily,
    pub distance: u32,
    pub init_basis: LogicalBasis,
    /// Full atom list in layout order.
    pub atoms: Vec<PhysicalAtomId>,
    pub data_atoms: Vec<PhysicalAtomId>,
    pub check_atoms: Vec<PhysicalAtomId>,
    /// Lattice coordinates parallel to [`Self::atoms`] (Stim `QUBIT_COORDS`).
    pub coords: Vec<(i32, i32)>,
    pub stabilizers: Vec<StabilizerDef>,
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
    #[error("logical_cx expansion is out of scope for #249 (see #250)")]
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
    #[error(
        "surface-code stabilizer layout invalid for distance {distance}: \
         expected {expected} checks, got {got}"
    )]
    InvalidSurfaceLayout {
        distance: u32,
        expected: u32,
        got: usize,
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
                basis,
                ..
            } => {
                let layout = find_layout(&layouts, *logical_id)?;
                if *family == SourceFamily::Repetition
                    && (*basis != LogicalBasis::Z || layout.init_basis != LogicalBasis::Z)
                {
                    return Err(ExpandError::NonZInitBasis {
                        logical_id: logical_id.0,
                        basis: basis.as_str(),
                    });
                }
                // Construct allocates; v0 emits no physical gates (|0…0⟩ prep).
                rounds.push(PhysicalRound {
                    kind: RoundKind::Construct,
                    logical_id: *logical_id,
                    local_before: Vec::new(),
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal: Vec::new(),
                });
            }
            WorkloadOp::MemoryRound { logical_id } => {
                let layout = find_layout(&layouts, *logical_id)?;
                rounds.push(match layout.family {
                    SourceFamily::Repetition => repetition_memory_round(layout)?,
                    SourceFamily::Surface => surface_memory_round(layout),
                });
            }
            WorkloadOp::MeasureLogical { logical_id, basis } => {
                let layout = find_layout(&layouts, *logical_id)?;
                rounds.push(measure_logical_round(layout, *basis));
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
        SourceFamily::Surface => expand_surface_layout(meta, next_atom),
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
    let coords: Vec<(i32, i32)> = (0..n as i32).map(|i| (i, 0)).collect();
    let mut data_atoms = Vec::with_capacity(meta.distance as usize);
    let mut check_atoms = Vec::with_capacity(meta.distance.saturating_sub(1) as usize);
    for (offset, &atom) in atoms.iter().enumerate() {
        if offset % 2 == 0 {
            data_atoms.push(atom);
        } else {
            check_atoms.push(atom);
        }
    }
    let mut stabilizers = Vec::with_capacity(check_atoms.len());
    for i in 0..check_atoms.len() {
        stabilizers.push(StabilizerDef {
            check: check_atoms[i],
            basis: LogicalBasis::Z,
            data: vec![data_atoms[i], data_atoms[i + 1]],
        });
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
        coords,
        stabilizers,
    })
}

/// Rotated surface layout: `d²` data + `d² − 1` checks (Stim-faithful geometry).
fn expand_surface_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    let n = surface_n(meta.distance)?;
    let d = meta.distance as i32;
    let expected_checks = n.saturating_sub(meta.distance.saturating_mul(meta.distance));

    // Data at odd×odd lattice sites, row-major atom ids.
    let mut data_coord: Vec<((i32, i32), u32)> = Vec::with_capacity((d * d) as usize);
    for r in 0..d {
        for c in 0..d {
            data_coord.push(((2 * c + 1, 2 * r + 1), (r * d + c) as u32));
        }
    }
    let data_at: std::collections::HashMap<(i32, i32), u32> =
        data_coord.iter().copied().collect();

    // Plaquette centers on even×even sites; smooth (top/bottom) X, rough (L/R) Z.
    let mut plaquettes: Vec<((i32, i32), LogicalBasis, Vec<u32>)> = Vec::new();
    for y in (0..=2 * d).step_by(2) {
        for x in (0..=2 * d).step_by(2) {
            let mut neighbors = Vec::new();
            for (dx, dy) in [(-1, -1), (-1, 1), (1, -1), (1, 1)] {
                if let Some(&idx) = data_at.get(&(x + dx, y + dy)) {
                    neighbors.push(idx);
                }
            }
            if neighbors.len() < 2 {
                continue;
            }
            neighbors.sort_unstable();
            let basis = if (x + y) % 4 == 2 {
                LogicalBasis::X
            } else {
                LogicalBasis::Z
            };
            let on_top = y == 0;
            let on_bot = y == 2 * d;
            let on_left = x == 0;
            let on_right = x == 2 * d;
            if (on_top || on_bot) && basis != LogicalBasis::X {
                continue;
            }
            if (on_left || on_right) && basis != LogicalBasis::Z {
                continue;
            }
            if (on_top || on_bot) && (on_left || on_right) {
                continue;
            }
            plaquettes.push(((x, y), basis, neighbors));
        }
    }
    plaquettes.sort_by_key(|&((x, y), _, _)| (y, x));

    if plaquettes.len() != expected_checks as usize {
        return Err(ExpandError::InvalidSurfaceLayout {
            distance: meta.distance,
            expected: expected_checks,
            got: plaquettes.len(),
        });
    }

    let first = *next_atom;
    let last = next_atom
        .checked_add(n)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;

    let data_atoms: Vec<PhysicalAtomId> = (0..(d * d) as u32)
        .map(|i| PhysicalAtomId(first + i))
        .collect();
    let check_atoms: Vec<PhysicalAtomId> = (0..plaquettes.len() as u32)
        .map(|i| PhysicalAtomId(first + (d * d) as u32 + i))
        .collect();

    let mut atoms = Vec::with_capacity(n as usize);
    let mut coords = Vec::with_capacity(n as usize);
    for r in 0..d {
        for c in 0..d {
            atoms.push(PhysicalAtomId(first + (r * d + c) as u32));
            coords.push((2 * c + 1, 2 * r + 1));
        }
    }
    let mut stabilizers = Vec::with_capacity(plaquettes.len());
    for (i, &((x, y), basis, ref neighbors)) in plaquettes.iter().enumerate() {
        let check = check_atoms[i];
        atoms.push(check);
        coords.push((x, y));
        stabilizers.push(StabilizerDef {
            check,
            basis,
            data: neighbors
                .iter()
                .map(|&local| PhysicalAtomId(first + local))
                .collect(),
        });
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
        coords,
        stabilizers,
    })
}

/// Kelly-style bit-flip syndrome: each check `C_i` is the CNOT target of
/// neighboring data `D_i` and `D_{i+1}`.
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
    let z_cnot_count = entangling.len();
    Ok(PhysicalRound {
        kind: RoundKind::MemoryRound,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling,
        z_cnot_count,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal,
    })
}

fn surface_memory_round(layout: &ExpandedBlock) -> PhysicalRound {
    // Z-then-X phases: overlapping X/Z CX in one block does not implement
    // simultaneous stabilizer extraction (Stim needs its 4-layer schedule or
    // a serial Z-then-X split). We take the serial split for all odd d.
    let mut z_cnots = Vec::new();
    let mut x_cnots = Vec::new();
    let mut x_h = Vec::new();

    for stab in &layout.stabilizers {
        match stab.basis {
            LogicalBasis::X => {
                x_h.push(RoundLocalOp::H { atom: stab.check });
                for &data in &stab.data {
                    x_cnots.push(PhysicalCnot {
                        control: stab.check,
                        target: data,
                    });
                }
            }
            LogicalBasis::Z => {
                for &data in &stab.data {
                    z_cnots.push(PhysicalCnot {
                        control: data,
                        target: stab.check,
                    });
                }
            }
        }
    }

    let z_cnot_count = z_cnots.len();
    let mut entangling = z_cnots;
    entangling.extend(x_cnots);

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

    PhysicalRound {
        kind: RoundKind::MemoryRound,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling,
        z_cnot_count,
        local_mid: x_h.clone(),
        local_after: x_h,
        terminal,
    }
}

fn measure_logical_round(layout: &ExpandedBlock, basis: LogicalBasis) -> PhysicalRound {
    let terminal = layout
        .data_atoms
        .iter()
        .map(|&atom| RoundTerminal::Measure { atom, basis })
        .collect();
    PhysicalRound {
        kind: RoundKind::MeasureLogical,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
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

    fn surface_d3_two_rounds() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
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
        assert_eq!(block.stabilizers.len(), 2);
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
            assert!(round.local_before.is_empty());
            assert!(round.local_after.is_empty());
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
    fn surface_d3_layout_is_seventeen_atoms_with_eight_checks() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.physical_atom_count(), 17);
        let block = &expanded.blocks[0];
        assert_eq!(block.family, SourceFamily::Surface);
        assert_eq!(
            block.code_family,
            CodeFamily::SurfaceCodeLike { distance: 3 }
        );
        assert_eq!(block.data_atoms.len(), 9);
        assert_eq!(block.check_atoms.len(), 8);
        assert_eq!(block.stabilizers.len(), 8);
        assert_eq!(
            block.data_atoms,
            (0..9).map(PhysicalAtomId).collect::<Vec<_>>()
        );
        assert_eq!(
            block.check_atoms,
            (9..17).map(PhysicalAtomId).collect::<Vec<_>>()
        );
        let x_checks: Vec<_> = block
            .stabilizers
            .iter()
            .filter(|s| s.basis == LogicalBasis::X)
            .map(|s| s.check.0)
            .collect();
        let z_checks: Vec<_> = block
            .stabilizers
            .iter()
            .filter(|s| s.basis == LogicalBasis::Z)
            .map(|s| s.check.0)
            .collect();
        assert_eq!(x_checks, vec![9, 11, 14, 16]);
        assert_eq!(z_checks, vec![10, 12, 13, 15]);
        assert_eq!(block.coords[0], (1, 1));
        assert_eq!(block.coords[8], (5, 5));
        assert_eq!(block.coords[9], (2, 0)); // top X
    }

    #[test]
    fn surface_d3_memory_round_cnot_h_measure_reset_pattern() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.memory_round_count(), 2);
        let memory = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MemoryRound)
            .expect("memory");

        let h_atoms: Vec<_> = memory
            .local_mid
            .iter()
            .map(|op| match op {
                RoundLocalOp::H { atom } => atom.0,
            })
            .collect();
        assert_eq!(h_atoms, vec![9, 11, 14, 16]);
        assert!(memory.local_before.is_empty());
        assert_eq!(memory.local_mid, memory.local_after);
        assert_eq!(memory.entangling.len(), 24);
        assert_eq!(memory.z_cnot_count, 12);

        let expected_cnot = vec![
            // Z10: [0,1,3,4]
            PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(1),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(3),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(10),
            },
            // Z12: [2,5]
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(12),
            },
            PhysicalCnot {
                control: PhysicalAtomId(5),
                target: PhysicalAtomId(12),
            },
            // Z13: [3,6]
            PhysicalCnot {
                control: PhysicalAtomId(3),
                target: PhysicalAtomId(13),
            },
            PhysicalCnot {
                control: PhysicalAtomId(6),
                target: PhysicalAtomId(13),
            },
            // Z15: [4,5,7,8]
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(5),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(7),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(8),
                target: PhysicalAtomId(15),
            },
            // X9: [0,1]
            PhysicalCnot {
                control: PhysicalAtomId(9),
                target: PhysicalAtomId(0),
            },
            PhysicalCnot {
                control: PhysicalAtomId(9),
                target: PhysicalAtomId(1),
            },
            // X11: [1,2,4,5]
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(2),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(4),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(5),
            },
            // X14: [3,4,6,7]
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(3),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(4),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(6),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(7),
            },
            // X16: [7,8]
            PhysicalCnot {
                control: PhysicalAtomId(16),
                target: PhysicalAtomId(7),
            },
            PhysicalCnot {
                control: PhysicalAtomId(16),
                target: PhysicalAtomId(8),
            },
        ];
        assert_eq!(memory.entangling, expected_cnot);

        assert_eq!(memory.terminal.len(), 16); // 8 MZ + 8 Reset
        for (i, atom) in (9..17).enumerate() {
            assert_eq!(
                memory.terminal[i],
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(atom),
                    basis: LogicalBasis::Z,
                }
            );
            assert_eq!(
                memory.terminal[8 + i],
                RoundTerminal::Reset {
                    atom: PhysicalAtomId(atom),
                }
            );
        }
    }

    #[test]
    fn surface_d3_op_order_construct_rounds_measure() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
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
        assert_eq!(mz.terminal.len(), 9);
        assert!(
            mz.terminal.iter().all(|t| matches!(
                t,
                RoundTerminal::Measure {
                    basis: LogicalBasis::Z,
                    ..
                }
            ))
        );
    }

    #[test]
    fn surface_code_x_construct_allowed() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::X, LogicalQubitId(0))
            .expect("construct_x");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
            .expect("mx");
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.blocks[0].init_basis, LogicalBasis::X);
        assert_eq!(expanded.physical_atom_count(), 17);
    }

    #[test]
    fn surface_d5_layout_counts() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 5, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.physical_atom_count(), 49);
        assert_eq!(expanded.blocks[0].data_atoms.len(), 25);
        assert_eq!(expanded.blocks[0].check_atoms.len(), 24);
        assert_eq!(expanded.blocks[0].stabilizers.len(), 24);
    }

    #[test]
    fn logical_cx_is_rejected() {
        let workload = QecWorkload {
            blocks: vec![
                WorkloadBlock {
                    logical_id: LogicalQubitId(0),
                    family: SourceFamily::Surface,
                    distance: 3,
                    init_basis: LogicalBasis::Z,
                    code_family: CodeFamily::SurfaceCodeLike { distance: 3 },
                },
                WorkloadBlock {
                    logical_id: LogicalQubitId(1),
                    family: SourceFamily::Surface,
                    distance: 3,
                    init_basis: LogicalBasis::Z,
                    code_family: CodeFamily::SurfaceCodeLike { distance: 3 },
                },
            ],
            ops: vec![
                WorkloadOp::Construct {
                    family: SourceFamily::Surface,
                    distance: 3,
                    basis: LogicalBasis::Z,
                    logical_id: LogicalQubitId(0),
                },
                WorkloadOp::Construct {
                    family: SourceFamily::Surface,
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
