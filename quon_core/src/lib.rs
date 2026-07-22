//! `quon_core` — the MLIR-free shared kernel of the Quon workspace.
//!
//! Both `frontend` and `mlir_bridge` depend on this crate, so it must never
//! pull in `melior`/LLVM. The crate is organized as a small set of **named
//! domain modules**, each with a single, documented role — not a bag of
//! MLIR-free utilities. The center of gravity is [`DepthExpr`], the symbolic
//! gate-depth bound carried by `Circuit<n, m, d, C>` types in the frontend and
//! by `quantum.circ` op attributes downstream (ADR-0002); everything else is
//! here because it crosses the frontend↔IR seam and must stay MLIR-free.
//!
//! # What belongs here, and what does not
//!
//! A type belongs in `quon_core` iff **both** `frontend` and `mlir_bridge`
//! (or another non-MLIR crate) need to construct or inspect it *and* it has
//! no dependency on Melior/LLVM. A type that only one side touches, or that
//! drags in MLIR, does not belong here — it lives in its owning crate.
//!
//! # Module roster
//!
//! | Module           | Domain role                                                                  |
//! |------------------|------------------------------------------------------------------------------|
//! | [`depth`]        | Circuit index algebra — [`DepthExpr`], the symbolic gate-depth bound.        |
//! | [`optimization`] | Depth-algebra invariant kernels for peephole passes (companions to `depth`). |
//! | [`gates`]        | Single gate-metadata source — the canonical native-gate registry.          |
//! | [`qasm`]         | Emit-domain AST — the faithful OpenQASM 3.0 syntax tree + total renderer.   |
//! | [`linearity`]    | SSA use-count adapter for the no-cloning / linear-use judgment.              |
//! | [`metrics`]      | Snapshot/regression DTO — metrics wire types and comparison (no collector).  |
//!
//! # The linearity story (Δ ⇄ SSA)
//!
//! Linear qubit use is enforced at two distinct stages of the compiler, by two
//! *adapters* of one and the same judgment — the **no-cloning / linear-use
//! judgment** ("every qubit resource is consumed exactly once"):
//!
//! * **Frontend adapter — `Δ` (linear context).** `frontend`'s typing context
//!   `Δ : HashMap<Name, Type>` records named qubit resources and physically
//!   removes a name from `Δ` when the corresponding `Qubit`/`QReg` is consumed
//!   in the term. A second use is a *scope* error, caught statically at the
//!   source language.
//! * **IR adapter — SSA use-count kernels ([`linearity`]).** After lowering to
//!   `quantum.circ` / `quantum.dynamic`, names are gone; the judgment is
//!   re-expressed as "every `!qubit` SSA value has exactly one use" (SPEC
//!   §6.2–§6.3), checked by region verifier passes against [`LINEAR_USE_COUNT`].
//!
//! The two adapters share vocabulary but not a type: `Δ` and the SSA
//! use-count kernels stay as separate types in their owning crates, and neither
//! is subsumed by the other. See the [`linearity`] module docs and the
//! `Linear context` / `Linearity (SSA)` glossary entries in `CONTEXT.md`.
//!
//! # Why qasm and metrics stay here
//!
//! [`qasm`] (~1k LOC) is the compiler's backend-facing OpenQASM 3.0 syntax
//! tree and its total renderer. It is MLIR-free *by necessity* — emission is a
//! pure string fold over a tree that `mlir_bridge::reify` builds once, so
//! pulling it out into its own crate would only add a workspace edge without
//! removing a dependency. It stays in `quon_core` as the emit-domain AST,
//! consuming the [`gates`] registry via [`qasm::from_gate_info`].
//!
//! [`metrics`] carries the compile-metrics wire types (snapshot, comparison,
//! tolerances) and their pure snapshot/compare logic. It is MLIR-free so the
//! snapshot/compare path can be unit-tested without linking LLVM; the *metric
//! collector* (walking the IR to populate [`CircuitMetrics`]) stays in
//! `mlir_bridge`, not here — `quon_core` owns only the DTO and its tests.
//!
//! ```
//! // Issue #216: every domain module stays reachable from the crate root,
//! // and each one's canonical export is the documented center of its role.
//! use quon_core::{depth, optimization, gates, qasm, linearity, metrics};
//! use quon_core::{DepthExpr, REGISTRY, LINEAR_USE_COUNT, CircuitMetrics};
//!
//! // depth — Circuit index algebra (the crate's center of gravity).
//! assert_eq!(DepthExpr::Nat(2).seq(DepthExpr::Nat(3)).to_sexpr(), "(+ 2 3)");
//! // optimization — depth-algebra invariant kernels (companions to depth).
//! assert_eq!(quon_core::optimization::depth_after_removal(5, 2), 3);
//! // gates — single gate-metadata source (canonical native-gate registry).
//! let _ = REGISTRY;
//! // qasm — emit-domain AST (OpenQASM 3.0 syntax tree + total renderer).
//! let _ = quon_core::qasm::from_gate_info;
//! // linearity — SSA use-count adapter for the no-cloning judgment.
//! assert_eq!(LINEAR_USE_COUNT, 1);
//! // metrics — snapshot/regression DTO (collector lives in mlir_bridge).
//! let _ = CircuitMetrics::default();
//! ```

pub mod depth;
pub mod gates;
pub mod linearity;
pub mod metrics;
pub mod optimization;
pub mod qasm;

pub use depth::{DepthExpr, DepthParseError};
pub use gates::{
    GateClass, GateInfo, REGISTRY, canonical_id, inverse, inverse_or_self, is_inverse_pair,
    is_self_inverse, lookup, openqasm_name, std_gates, std_gates_slice, surface_gate,
};
pub use linearity::{
    LINEAR_USE_COUNT, UseCountViolation, barrier_identity_ok, classify_use_count,
    if_qubit_threading_ok, is_linear_use_count, is_reuse_after_measure, unitary_region_boundary_ok,
};
pub use metrics::{
    CircuitMetrics, ComparisonReport, CompileInfo, CompileStatus, MetricTolerance,
    MetricTolerances, MetricsError, MetricsSnapshot, ProgramInfo, RegressionConfig, SCHEMA_VERSION,
    TargetInfo, ToolchainInfo, Violation, compare, format_comparison_table, format_metrics_line,
    format_tolerance, format_watch_metrics_line, load_snapshot, save_snapshot,
};
pub use optimization::{
    arity_preserved, depth_after_removal, par_depth, seq_depth, single_qubit_pair,
};
pub use qasm::{
    BitId, Expr, GateDef, OneQubitGate, Program, QasmError, QasmGate, QasmGateBuildError, QubitId,
    Register, RotationGate, Stmt, TwoQubitGate, from_gate_info, index_in_bounds, operand_arity_ok,
    render,
};
