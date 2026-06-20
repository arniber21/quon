//! Safe, Rust-native error handling over MLIR's C diagnostic API.
//!
//! Melior (0.27) does not wrap diagnostic *emission* — only the `mlir-sys`
//! `mlirEmitError` C entry point is available, and it is `unsafe`. Rather than
//! sprinkle raw FFI calls through the verifiers and passes, all error handling
//! flows through this module:
//!
//!   * Dialect verifiers and passes stay pure. They build up a [`Diagnostics`]
//!     accumulator (a Writer-style "diagnostic monad") and return it, or fold a
//!     typed [`Result`] into it with [`Diagnostics::report`]. None of that code
//!     touches the FFI boundary.
//!   * [`Diagnostics::emit`] is the *single* place in the crate that crosses
//!     into the unsafe MLIR C API. It is `safe` to call: the one `unsafe` block
//!     it contains is self-contained and sound (a valid location plus a
//!     NUL-free C string).
//!
//! This keeps the unsafe surface to one auditable function while the rest of
//! the bridge composes diagnostics with ordinary `Result`/iterator combinators.

use std::ffi::CString;
use std::fmt;

use melior::ir::Location;
use mlir_sys::mlirEmitError;

/// A single error diagnostic anchored at an IR [`Location`].
#[derive(Clone)]
pub struct Diagnostic<'c> {
    location: Location<'c>,
    message: String,
}

impl<'c> Diagnostic<'c> {
    /// Creates an error diagnostic.
    pub fn error(location: Location<'c>, message: impl Into<String>) -> Self {
        Self {
            location,
            message: message.into(),
        }
    }

    /// The human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The location the diagnostic is anchored at.
    pub fn location(&self) -> Location<'c> {
        self.location
    }

    /// Emits this diagnostic into MLIR's diagnostic engine.
    ///
    /// This is the only function in the crate that calls into the MLIR C
    /// diagnostic API. The message is sanitized of interior NUL bytes so the
    /// `CString` conversion is infallible.
    fn emit(&self) {
        let sanitized: String = self
            .message
            .chars()
            .map(|c| if c == '\0' { '?' } else { c })
            .collect();
        let message = CString::new(sanitized).unwrap_or_default();
        // SAFETY: `self.location` is a live MLIR location owned by the context,
        // and `message` is a valid NUL-terminated C string that outlives the
        // call. `mlirEmitError` copies the message and does not retain the
        // pointer.
        unsafe { mlirEmitError(self.location.to_raw(), message.as_ptr()) };
    }
}

impl fmt::Display for Diagnostic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl fmt::Debug for Diagnostic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostic")
            .field("message", &self.message)
            .finish()
    }
}

/// An accumulator of [`Diagnostic`]s — a Writer-style monad over the MLIR
/// diagnostic engine.
///
/// Verifiers and passes thread one of these through their logic, recording any
/// problems they find, and hand it back to the caller. The terminal operations
/// are [`Diagnostics::into_result`] (stay in pure Rust) and
/// [`Diagnostics::emit`] (flush to MLIR).
#[derive(Clone, Debug, Default)]
pub struct Diagnostics<'c> {
    items: Vec<Diagnostic<'c>>,
}

impl<'c> Diagnostics<'c> {
    /// Creates an empty accumulator.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Records an error diagnostic.
    pub fn error(&mut self, location: Location<'c>, message: impl Into<String>) {
        self.items.push(Diagnostic::error(location, message));
    }

    /// Pushes a pre-built diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic<'c>) {
        self.items.push(diagnostic);
    }

    /// Folds a typed verifier `Result` into the accumulator: on `Err`, records
    /// the error's `Display` text at `location`. This is the bind that connects
    /// pure, location-free verifier errors to the diagnostic writer.
    pub fn report<E: fmt::Display>(&mut self, location: Location<'c>, result: Result<(), E>) {
        if let Err(error) = result {
            self.error(location, error.to_string());
        }
    }

    /// Drains another accumulator into this one.
    pub fn absorb(&mut self, other: Diagnostics<'c>) {
        self.items.extend(other.items);
    }

    /// Whether any diagnostics were recorded.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The number of recorded diagnostics.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Iterates over the recorded diagnostics.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic<'c>> {
        self.items.iter()
    }

    /// Converts into a `Result`, the natural monadic exit into pure Rust: `Ok`
    /// when clean, `Err(self)` carrying every diagnostic otherwise.
    pub fn into_result(self) -> Result<(), Diagnostics<'c>> {
        if self.items.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }

    /// Emits every recorded diagnostic into MLIR and reports overall success.
    ///
    /// Returns `true` when no diagnostics were recorded (the MLIR
    /// `LogicalResult`-style success convention), `false` otherwise.
    pub fn emit(&self) -> bool {
        for diagnostic in &self.items {
            diagnostic.emit();
        }
        self.items.is_empty()
    }
}
