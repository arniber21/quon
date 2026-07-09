/// Surface keywords for completion and semantic highlighting.
pub fn keywords() -> &'static [&'static str] {
    &[
        "fn",
        "type",
        "let",
        "in",
        "return",
        "match",
        "circuit",
        "run",
        "borrow",
        "for",
        "if",
        "then",
        "else",
        "true",
        "false",
        "adjoint",
        "controlled",
        "par",
    ]
}

pub fn classical_builtins() -> &'static [&'static str] {
    &[
        "range",
        "map",
        "fold",
        "take",
        "zip",
        "float",
        "round",
        "sqrt",
        "log2",
        "measure",
        "measure_x",
        "measure_y",
        "reset",
        "discard",
        "qubit",
        "qreg",
        "PI",
        "E",
        "index",
    ]
}

pub fn gates() -> &'static [&'static str] {
    &[
        "I", "X", "Y", "Z", "H", "S", "S_dag", "SX", "SX_dag", "T", "T_dag", "CNOT", "CX", "CY",
        "CZ", "SWAP", "iSWAP", "ECR", "Rx", "Ry", "Rz", "Rzz", "Rxx", "Ryy", "CRz", "CRx", "CP",
    ]
}

/// Quantum/linear prelude names (allocation, combinators, etc.).
pub fn quantum_builtins() -> &'static [&'static str] {
    &[
        "qreg",
        "qubit",
        "destructure",
        "split",
        "tensored",
        "measure",
        "measure_x",
        "measure_y",
        "measure_all",
        "reset",
        "discard",
        "apply",
        "apply_dyn",
        "init_one",
        "init_plus",
        "map_q",
        "sequence_q",
        "return",
        "identity",
        "adjoint",
        "controlled",
        "repeat",
        "on_high",
        "on_low",
        "swap_reverse",
    ]
}

pub fn is_quantum_builtin(name: &str) -> bool {
    quantum_builtins().contains(&name)
}

pub fn gate_type(name: &str) -> Option<crate::types::Ty> {
    crate::typecheck::circuit::gate_type(name)
}
