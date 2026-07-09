//! MLIR-free neutral-atom backend domain types.
//!
//! This crate is intentionally additive: it defines serializable Rust data
//! structures for neutral-atom layouts, schedules, validation helpers, QEC
//! code-block expansion, and resource reports without registering dialects or
//! requiring an MLIR context.

#[cfg(feature = "mlir")]
pub mod dialect;
pub mod layout;
pub mod qec;
pub mod report;
pub mod schedule;

pub use layout::{
    AodTrapRef, AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
pub use qec::{
    CodeBlock, CodeBlockId, CodeFamily, LogicalOp, LogicalQubitId, NetRate, QecError,
    atoms_per_logical, ceil_div, expand_code_block, repetition_n, surface_n,
};
pub use report::ResourceReport;
pub use schedule::{
    AtomMove, EntanglingAction, MeasurementBasis, MovementGroup, NeutralAtomAction, ScheduleError,
    ScheduleLayer, TransferDirection, TrapTransfer,
};
