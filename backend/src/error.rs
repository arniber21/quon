// Error type for the backend crate — see issue #3.
//
// Follows the workspace `thiserror` convention (cf. `frontend::typecheck::TypeError`).
// Library code never `unwrap`s/`expect`s; every fallible path returns `BackendError`.

/// Errors raised while constructing or loading a [`crate::target::BackendTarget`].
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// A qubit index referenced in the descriptor is `>= num_qubits`.
    #[error("qubit index {got} out of range (num_qubits = {num_qubits})")]
    QubitOutOfRange { got: usize, num_qubits: usize },

    /// A connectivity edge names an endpoint outside `0..num_qubits`.
    #[error("edge ({a}, {b}) references a qubit >= num_qubits ({num_qubits})")]
    EdgeOutOfRange {
        a: usize,
        b: usize,
        num_qubits: usize,
    },

    /// A connectivity edge connects a qubit to itself.
    #[error("self-loop edge on qubit {0} is not allowed")]
    SelfLoop(usize),

    /// A native gate name has no registered decomposition.
    #[error("unknown native gate `{0}` (no decomposition registered)")]
    UnknownGate(String),

    /// A target descriptor names an architecture family this backend does not
    /// recognize.
    #[error("unknown target kind `{0}`")]
    UnknownTargetKind(String),

    /// A target descriptor is syntactically valid JSON but violates semantic
    /// invariants such as positive geometry, non-overlapping zones, or
    /// architecture-specific capacity limits.
    #[error("invalid target configuration: {0}")]
    InvalidTargetConfig(String),

    /// QEC error reporting or `--emit-qec-experiment` was requested, but the
    /// neutral-atom target has no `error_model`. Never invent defaults or
    /// derive rates from `fidelity` (ADR-0017).
    #[error(
        "neutral-atom target is missing error_model required for QEC error reporting or --emit-qec-experiment"
    )]
    MissingErrorModel,

    /// A two-qubit noise key was not of the form `\"u,v\"`.
    #[error("malformed two-qubit noise key `{0}` (expected \"u,v\")")]
    BadTwoQubitKey(String),

    /// A noise-map key was expected to be a qubit index but did not parse.
    #[error("malformed qubit-index key `{0}` in noise model")]
    BadQubitKey(String),

    /// The descriptor JSON was syntactically or structurally invalid. The
    /// underlying `serde_json` error names the offending field.
    #[error("invalid target descriptor JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The descriptor file could not be read.
    #[error("could not read target descriptor: {0}")]
    Io(#[from] std::io::Error),
}
