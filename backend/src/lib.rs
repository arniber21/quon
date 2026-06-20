//! Backend hardware descriptors for Quon — see issue #3, SPEC.md §8.
//!
//! A [`BackendTarget`] combines a [`ConnectivityGraph`], a native gate set,
//! a [`NoiseModel`], and capability flags. Targets come from the built-in
//! [`generic_openqasm`] target or are loaded from a §8.3 JSON descriptor via
//! [`json::load`].

pub mod descriptor;
pub mod error;
pub mod gates;
pub mod generic_openqasm;
pub mod json;
pub mod keys;
pub mod target;

pub use descriptor::TargetDescriptor;
pub use error::BackendError;
pub use target::{BackendTarget, ConnectivityGraph, GateOp, NativeGate, NoiseModel, UNREACHABLE};
