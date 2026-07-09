//! Backend hardware descriptors for Quon — see issue #3, SPEC.md §8.
//!
//! A [`BackendTarget`] wraps architecture-specific target payloads behind
//! [`TargetKind`]. The current gate-model compiler uses [`FixedTarget`]; the
//! neutral-atom backend loads [`NeutralAtomTarget`] descriptors through the
//! same [`json::load`] entry point.

pub mod decompose;
pub mod descriptor;
pub mod error;
pub mod gates;
pub mod generic_openqasm;
pub mod json;
pub mod keys;
pub mod target;
pub mod unitary;

pub use descriptor::TargetDescriptor;
pub use error::BackendError;
pub use target::{
    AodMovement, AodMovementModel, AodSpeedModel, AodSpeedModelKind, BackendTarget,
    ConnectivityGraph, FixedTarget, GateOp, NativeGate, NeutralAtomCostModel, NeutralAtomFidelity,
    NeutralAtomGrid, NeutralAtomTarget, NeutralAtomTiming, NeutralAtomZone, NoiseModel,
    RydbergInteraction, TargetKind, UNREACHABLE, ZoneKind,
};
