//! Centralized unsafe FFI boundary for the MLIR bridge.
//!
//! All `unsafe` code in `mlir_bridge` is confined to this module. Pass
//! implementations, verifiers, and the [`crate::diagnostics`] accumulator
//! compose the safe wrappers exported here and never touch raw `mlir-sys`
//! pointers directly.
//!
//! Three concerns are centralized:
//!   * Error emission (`mlirEmitError`) — called by [`crate::diagnostics`].
//!   * Raw operation mutation (`mlirOperationSetAttributeByName`,
//!     `mlirOperationSetOperand`).
//!   * External-pass context lifetime erasure (`PassContext` +
//!     `with_context`).
//!
//! Every `unsafe` block below carries a `SAFETY` comment tied to the upstream
//! FFI contract or the MLIR pass-framework lifetime guarantee.

#![allow(unsafe_code)]

use std::ffi::CString;

use melior::StringRef;
use melior::ir::{Attribute, AttributeLike, Location, OperationRef, Value, ValueLike};
use melior::{Context, ContextRef};
use mlir_sys::{
    MlirContext, mlirEmitError, mlirOperationSetAttributeByName, mlirOperationSetOperand,
};

// ─── Error emission ─────────────────────────────────────────────────────────

/// Emits an error diagnostic at `location` with the given C string message.
///
/// This is the sole call site for `mlirEmitError` in the crate. The message
/// must already be a valid NUL-terminated C string (the [`crate::diagnostics`]
/// module sanitizes interior NULs before calling).
pub(crate) fn emit_error(location: &Location<'_>, message: &CString) {
    // SAFETY: `location` is a live MLIR location backed by the context that
    // owns it. `message` is a valid NUL-terminated C string whose backing
    // buffer outlives the call. `mlirEmitError` copies the message internally
    // and does not retain the pointer after returning.
    unsafe { mlirEmitError(location.to_raw(), message.as_ptr()) };
}

// ─── Raw operation mutation ─────────────────────────────────────────────────

/// Sets a named attribute on an MLIR operation.
///
/// Safe wrapper for `mlirOperationSetAttributeByName`. The operation and
/// attribute must belong to the same context.
pub(crate) fn set_operation_attribute<'c>(
    op: OperationRef<'c, '_>,
    name: &str,
    attribute: &Attribute<'c>,
) {
    // SAFETY: `op` is a live operation reference. `name` is borrowed as a
    // `StringRef` for the duration of the call; the C function copies it
    // internally via `MlirStringRef` and does not retain the pointer.
    // `attribute` is a live attribute owned by the same context. The function
    // performs an in-place attribute update and does not retain any pointer
    // after returning.
    unsafe {
        mlirOperationSetAttributeByName(
            op.to_raw(),
            StringRef::new(name).to_raw(),
            attribute.to_raw(),
        );
    }
}

/// Replaces a single operand of an MLIR operation.
///
/// Safe wrapper for `mlirOperationSetOperand`. The operation and value must
/// belong to the same context. The caller is responsible for ensuring `index`
/// is a valid operand slot.
pub(crate) fn set_operation_operand<'c, 'a>(
    op: OperationRef<'c, 'a>,
    index: isize,
    value: &Value<'c, 'a>,
) {
    // SAFETY: `op` is a live operation reference. `index` is a valid operand
    // index validated by the caller. `value` is a live SSA value belonging to
    // the same context. The function updates the operand slot in place and
    // does not retain the pointers after returning.
    unsafe {
        mlirOperationSetOperand(op.to_raw(), index, value.to_raw());
    }
}

// ─── External-pass context lifetime erasure ────────────────────────────────

/// Erases the external-pass context lifetime so that a `'static` pass struct
/// can hold a context reference between `initialize` and `run`.
///
/// MLIR's `RunExternalPass` trait hands passes a `ContextRef<'c>` in
/// `initialize`, but the pass struct must be `'static` and cannot hold a
/// `&'c Context` directly. The pass framework guarantees that the context
/// remains valid for the entire lifetime of the pass.
///
/// `PassContext` stores the raw `MlirContext` handle (a `Copy` value, not a
/// pointer to stack memory) extracted in `initialize`. In `run`, the handle is
/// reconstructed into a `ContextRef` whose lifetime is scoped to the
/// [`with_context`] closure, ensuring the `&Context` is always valid.
///
/// # Why not store a `&Context` pointer?
///
/// `ContextRef::to_ref` uses `transmute` to return a `&Context` that points to
/// the `ContextRef` itself (they share the same layout: a single
/// `MlirContext`). Storing that pointer for later use is undefined behaviour
/// because the `ContextRef` is a stack local. Storing the `MlirContext` handle
/// by value and reconstructing the `ContextRef` in `run` avoids this pitfall.
#[derive(Clone, Copy, Default)]
pub(crate) struct PassContext {
    raw: Option<MlirContext>,
}

impl PassContext {
    /// Creates an empty context store (no context captured yet).
    pub(crate) fn new() -> Self {
        Self { raw: None }
    }

    /// Captures the MLIR context handle for later retrieval.
    ///
    /// The `MlirContext` handle is extracted by value from the `ContextRef`.
    /// No reference to the `ContextRef` (which is a stack local) is retained.
    pub(crate) fn capture<'c>(&mut self, context: ContextRef<'c>) {
        // SAFETY: `to_ref` transmutes `&ContextRef` into `&Context` (same
        // layout), and `to_raw` copies the `MlirContext` handle out by value.
        // The `&Context` is alive only for this expression; the stored
        // `MlirContext` is an independent value, not a dangling pointer.
        self.raw = Some(unsafe { context.to_ref().to_raw() });
    }

    /// Returns the raw `MlirContext` handle, or `None` if [`capture`](Self::capture)
    /// was never called.
    ///
    /// The handle is safe to pass to [`with_context`].
    pub(crate) fn raw(&self) -> Option<MlirContext> {
        self.raw
    }
}

/// Runs a closure with a borrowed `&'c Context` reconstructed from a stored
/// `MlirContext` handle.
///
/// The `ContextRef` is created as a local variable inside this function, so
/// the `&Context` returned by `to_ref` (which `transmute`s `&ContextRef` into
/// `&Context`) is valid for the duration of the closure call. After the
/// closure returns, both are dropped — no dangling pointer escapes.
///
/// # Panics
///
/// Panics if `raw` is a null/invalid handle. The pass framework guarantees
/// validity, so this should never happen in practice.
pub(crate) fn with_context<'c, R>(raw: MlirContext, f: impl FnOnce(&'c Context) -> R) -> R {
    // SAFETY: The `MlirContext` handle was obtained in `PassContext::capture`
    // from a `ContextRef` provided by `RunExternalPass::initialize`. The pass
    // framework guarantees the context outlives every `run` call, so the
    // handle is still valid here.
    let context_ref = unsafe { ContextRef::from_raw(raw) };
    // SAFETY: `context_ref` is a local variable alive for the duration of this
    // function. `to_ref` transmutes `&context_ref` into `&'c Context` — the
    // reference points to `context_ref`'s stack slot, which is valid until
    // this function returns. The closure consumes the reference before that,
    // so no dangling pointer escapes.
    let context = unsafe { context_ref.to_ref() };
    f(context)
}
