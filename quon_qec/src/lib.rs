//! Shared MLIR-free QEC layer (ADR-0015).
//!
//! Owns code-family sizing formulas and the QEC workload IR collected from
//! typed builtins after `quantum.dynamic` lowering. Neutral-atom scheduling
//! (`quon_na`) and experiment emit consume these types; this crate has no
//! Melior / MLIR dependency.

pub mod expand;
pub mod experiment;
pub mod family;
pub mod lattice_surgery;
pub mod workload;

pub use expand::{
    ExpandError, ExpandedBlock, ExpandedWorkload, MergeBoundary, PauliFrameUpdate, PhysicalAtomId,
    PhysicalCnot, PhysicalRound, RoundKind, RoundLocalOp, RoundTerminal, StabilizerDef,
    expand_workload,
};
pub use experiment::{
    AtomRole, AtomSiteMapping, CheckGraph, ErrorModelSnapshot, ExperimentError, ExperimentRoundKind,
    FrameUpdateEntry, LogicalObservable, MeasurementScheduleEntry, NaScheduleRef,
    QEC_EXPERIMENT_KIND, QEC_EXPERIMENT_SCHEMA_VERSION, QecExperiment, StabilizerCheck,
    attach_barrier_cycles, build_experiment, dual_emit, emit_stim_structure, experiment_to_json,
    layer_nonoverlapping_cnots, na_refs_from_expanded, sibling_stim_path,
};
pub use family::{
    CodeFamily, NetRate, QecError, SourceFamily, atoms_per_logical, ceil_div, repetition_n,
    surface_n,
};
pub use workload::{
    LogicalBasis, LogicalQubitId, QecWorkload, WorkloadBlock, WorkloadBuilder, WorkloadError,
    WorkloadOp,
};
