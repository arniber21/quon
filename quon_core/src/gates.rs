//! Canonical native-gate registry (issue #209).
//!
//! Single source of truth for gate metadata shared by the typechecker,
//! backend native set, adjoint/inverse helpers, gate cancellation, and
//! OpenQASM emission. Adapters map registry ids to Melior attribute strings,
//! QASM keywords, or ZX nodes — they do not re-declare arity, class, or inverse.
//!
//! # Adding a gate
//!
//! 1. Append a [`GateInfo`] entry to [`REGISTRY`] (canonical id, aliases, arity,
//!    class, inverse id, OpenQASM spelling, parametric flag).
//! 2. If the gate is OpenQASM-emitable, ensure `openqasm` is `Some(...)` so
//!    [`std_gates`] and the backend `generic_openqasm` target pick it up.
//! 3. Typecheck and emit then consume the new entry automatically via
//!    [`lookup`] / [`surface_gate`] / [`std_gates`]. Specialized ZX / unitary
//!    tables may still need a one-line adapter until those islands migrate.

use std::sync::OnceLock;

/// Clifford vs Universal classification (SPEC §3.7, §5.4–§5.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateClass {
    Clifford,
    Universal,
}

/// Metadata for one native gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateInfo {
    /// Canonical Quon surface id (PascalCase / SPEC spelling), e.g. `"CNOT"`, `"S_dag"`.
    pub id: &'static str,
    /// Alternate surface spellings that resolve to this gate (e.g. `"CX"` → CNOT).
    pub aliases: &'static [&'static str],
    /// Qubit arity.
    pub arity: usize,
    /// Intrinsic Clifford class (rotations may still be specialised by angle elsewhere).
    pub class: GateClass,
    /// Canonical id of the inverse gate. Equal to [`GateInfo::id`] when self-inverse.
    pub inverse: &'static str,
    /// OpenQASM 3.0 stdgates keyword, if this gate is part of the backend native set.
    /// `None` for Quon-only gates that lower via decomposition (e.g. `iSWAP`, `Rzz`).
    pub openqasm: Option<&'static str>,
    /// True for parametric rotations (`Float -> Circuit<…>`).
    pub parametric: bool,
}

/// The full gate table. Order is stable for [`std_gates`] iteration.
pub static REGISTRY: &[GateInfo] = &[
    // ── §5.4 single-qubit, Clifford ─────────────────────────────────────────
    GateInfo {
        id: "I",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "I",
        openqasm: None, // identity is elided on emit
        parametric: false,
    },
    GateInfo {
        id: "X",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "X",
        openqasm: Some("x"),
        parametric: false,
    },
    GateInfo {
        id: "Y",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "Y",
        openqasm: Some("y"),
        parametric: false,
    },
    GateInfo {
        id: "Z",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "Z",
        openqasm: Some("z"),
        parametric: false,
    },
    GateInfo {
        id: "H",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "H",
        openqasm: Some("h"),
        parametric: false,
    },
    GateInfo {
        id: "S",
        aliases: &[],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "S_dag",
        openqasm: Some("s"),
        parametric: false,
    },
    GateInfo {
        id: "S_dag",
        aliases: &["Sdag", "S†", "sdg"],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "S",
        openqasm: Some("sdg"),
        parametric: false,
    },
    GateInfo {
        id: "SX",
        aliases: &["sx"],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "SX_dag",
        openqasm: Some("sx"),
        parametric: false,
    },
    GateInfo {
        id: "SX_dag",
        aliases: &["SXdag", "sxdg"],
        arity: 1,
        class: GateClass::Clifford,
        inverse: "SX",
        openqasm: None, // not in OpenQASM stdgates.inc; decompose if needed
        parametric: false,
    },
    // ── §5.4 single-qubit, Universal ────────────────────────────────────────
    GateInfo {
        id: "T",
        aliases: &[],
        arity: 1,
        class: GateClass::Universal,
        inverse: "T_dag",
        openqasm: Some("t"),
        parametric: false,
    },
    GateInfo {
        id: "T_dag",
        aliases: &["Tdag", "T†", "tdg"],
        arity: 1,
        class: GateClass::Universal,
        inverse: "T",
        openqasm: Some("tdg"),
        parametric: false,
    },
    // ── §5.5 two-qubit, Clifford ────────────────────────────────────────────
    GateInfo {
        id: "CNOT",
        aliases: &["CX", "cx"],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "CNOT",
        openqasm: Some("cx"),
        parametric: false,
    },
    GateInfo {
        id: "CY",
        aliases: &["cy"],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "CY",
        openqasm: Some("cy"),
        parametric: false,
    },
    GateInfo {
        id: "CZ",
        aliases: &["cz"],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "CZ",
        openqasm: Some("cz"),
        parametric: false,
    },
    GateInfo {
        id: "SWAP",
        aliases: &["swap"],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "SWAP",
        openqasm: Some("swap"),
        parametric: false,
    },
    GateInfo {
        id: "iSWAP",
        aliases: &[],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "iSWAP", // iSWAP† = iSWAP up to global phase in Quon's model
        openqasm: None,
        parametric: false,
    },
    GateInfo {
        id: "ECR",
        aliases: &[],
        arity: 2,
        class: GateClass::Clifford,
        inverse: "ECR",
        openqasm: None,
        parametric: false,
    },
    // ── §5.4/§5.5 parametric rotations ──────────────────────────────────────
    GateInfo {
        id: "Rx",
        aliases: &["rx"],
        arity: 1,
        class: GateClass::Universal,
        inverse: "Rx", // adjoint = Rx(-θ); name unchanged
        openqasm: Some("rx"),
        parametric: true,
    },
    GateInfo {
        id: "Ry",
        aliases: &["ry"],
        arity: 1,
        class: GateClass::Universal,
        inverse: "Ry",
        openqasm: Some("ry"),
        parametric: true,
    },
    GateInfo {
        id: "Rz",
        aliases: &["rz"],
        arity: 1,
        class: GateClass::Universal,
        inverse: "Rz",
        openqasm: Some("rz"),
        parametric: true,
    },
    GateInfo {
        id: "Rzz",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "Rzz",
        openqasm: None,
        parametric: true,
    },
    GateInfo {
        id: "Rxx",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "Rxx",
        openqasm: None,
        parametric: true,
    },
    GateInfo {
        id: "Ryy",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "Ryy",
        openqasm: None,
        parametric: true,
    },
    GateInfo {
        id: "CRz",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "CRz",
        openqasm: None,
        parametric: true,
    },
    GateInfo {
        id: "CRx",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "CRx",
        openqasm: None,
        parametric: true,
    },
    GateInfo {
        id: "CP",
        aliases: &[],
        arity: 2,
        class: GateClass::Universal,
        inverse: "CP",
        openqasm: None,
        parametric: true,
    },
    // ── Backend-only OpenQASM stdgates (not Quon surface primitives) ────────
    GateInfo {
        id: "u1",
        aliases: &[],
        arity: 1,
        class: GateClass::Universal,
        inverse: "u1",
        openqasm: Some("u1"),
        parametric: true,
    },
    GateInfo {
        id: "u2",
        aliases: &[],
        arity: 1,
        class: GateClass::Universal,
        inverse: "u2",
        openqasm: Some("u2"),
        parametric: true,
    },
    GateInfo {
        id: "u3",
        aliases: &[],
        arity: 1,
        class: GateClass::Universal,
        inverse: "u3",
        openqasm: Some("u3"),
        parametric: true,
    },
    GateInfo {
        id: "ccx",
        aliases: &["CCX", "Toffoli"],
        arity: 3,
        class: GateClass::Universal,
        inverse: "ccx",
        openqasm: Some("ccx"),
        parametric: false,
    },
];

/// Case-folded key for alias resolution. OpenQASM keywords are lowercase;
/// Quon surface names are mixed; IR may carry `S†` / `Sdag` / `S_dag`.
fn fold_key(name: &str) -> String {
    // Preserve dagger character; otherwise ASCII-lowercase for case-insensitive match.
    name.chars()
        .map(|c| {
            if c == '†' {
                c
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

fn alias_index() -> &'static std::collections::HashMap<String, &'static GateInfo> {
    static INDEX: OnceLock<std::collections::HashMap<String, &'static GateInfo>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        for gate in REGISTRY {
            map.insert(fold_key(gate.id), gate);
            for alias in gate.aliases {
                map.insert(fold_key(alias), gate);
            }
            if let Some(qasm) = gate.openqasm {
                map.insert(fold_key(qasm), gate);
            }
        }
        map
    })
}

/// Look up a gate by canonical id, alias, or OpenQASM spelling (case-insensitive).
pub fn lookup(name: &str) -> Option<&'static GateInfo> {
    alias_index().get(&fold_key(name)).copied()
}

/// Canonical id for `name`, or `None` if unknown.
pub fn canonical_id(name: &str) -> Option<&'static str> {
    lookup(name).map(|g| g.id)
}

/// OpenQASM keyword for `name`, if the gate is in the stdgates set.
pub fn openqasm_name(name: &str) -> Option<&'static str> {
    lookup(name).and_then(|g| g.openqasm)
}

/// Inverse gate's canonical id. Unknown names return `None`.
///
/// Self-inverse gates return their own id. Parametric rotations return the same
/// id (adjoint negates the angle at the call site, not the name).
pub fn inverse(name: &str) -> Option<&'static str> {
    lookup(name).map(|g| g.inverse)
}

/// Inverse for surface/IR names used by elaborate, lower, and uncomputation.
///
/// Falls back to returning `name` unchanged when the gate is unknown, matching
/// prior `inverse_gate_name` behaviour for self-inverse / unrecognized names.
pub fn inverse_or_self(name: &str) -> String {
    match inverse(name) {
        Some(inv) => inv.to_string(),
        None => name.to_string(),
    }
}

/// True if `a` and `b` form an inverse pair (including self-inverse).
///
/// Used by gate cancellation. Names are resolved through aliases first
/// (`CX` ≡ `CNOT`, `S†` ≡ `S_dag`).
pub fn is_inverse_pair(a: &str, b: &str) -> bool {
    let Some(ga) = lookup(a) else {
        return false;
    };
    let Some(gb) = lookup(b) else {
        return false;
    };
    // Parametric gates cancel only by angle merge, not by name pair.
    if ga.parametric || gb.parametric {
        return false;
    }
    ga.inverse == gb.id && gb.inverse == ga.id
}

/// True if the gate is self-inverse (inverse id equals canonical id).
pub fn is_self_inverse(name: &str) -> bool {
    lookup(name).is_some_and(|g| g.inverse == g.id && !g.parametric)
}

/// Quon surface gate? (SPEC §5.4–§5.5 primitives, excluding backend-only u1/u2/u3/ccx).
pub fn is_surface_gate(info: &GateInfo) -> bool {
    !matches!(info.id, "u1" | "u2" | "u3" | "ccx")
}

/// Look up a Quon surface gate (typecheck / elaborate). Backend-only QASM
/// keywords like `u3` return `None` so they are not treated as Quon primitives.
pub fn surface_gate(name: &str) -> Option<&'static GateInfo> {
    lookup(name).filter(|g| is_surface_gate(g))
}

/// OpenQASM 3.0 standard-gate table: `(keyword, arity)` pairs for the backend
/// native set. Derived from [`REGISTRY`] so it cannot drift from typecheck.
pub fn std_gates() -> Vec<(&'static str, usize)> {
    REGISTRY
        .iter()
        .filter_map(|g| g.openqasm.map(|kw| (kw, g.arity)))
        .collect()
}

/// Static slice of OpenQASM `(keyword, arity)` for callers that need `&[(&str, usize)]`.
///
/// Built once from [`REGISTRY`]. Prefer [`std_gates`] when a owned `Vec` is fine.
pub fn std_gates_slice() -> &'static [(&'static str, usize)] {
    static SLICE: OnceLock<Vec<(&'static str, usize)>> = OnceLock::new();
    SLICE.get_or_init(std_gates).as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cnot_aliases_resolve() {
        assert_eq!(canonical_id("CNOT"), Some("CNOT"));
        assert_eq!(canonical_id("CX"), Some("CNOT"));
        assert_eq!(canonical_id("cx"), Some("CNOT"));
    }

    #[test]
    fn dagger_aliases_resolve() {
        assert_eq!(canonical_id("S_dag"), Some("S_dag"));
        assert_eq!(canonical_id("Sdag"), Some("S_dag"));
        assert_eq!(canonical_id("S†"), Some("S_dag"));
        assert_eq!(canonical_id("sdg"), Some("S_dag"));
        assert_eq!(canonical_id("T†"), Some("T_dag"));
        assert_eq!(canonical_id("tdg"), Some("T_dag"));
    }

    #[test]
    fn inverses_round_trip() {
        assert_eq!(inverse("S"), Some("S_dag"));
        assert_eq!(inverse("S_dag"), Some("S"));
        assert_eq!(inverse("S†"), Some("S"));
        assert_eq!(inverse("T"), Some("T_dag"));
        assert_eq!(inverse("H"), Some("H"));
        assert_eq!(inverse("CNOT"), Some("CNOT"));
        assert_eq!(inverse("CX"), Some("CNOT"));
    }

    #[test]
    fn inverse_pairs() {
        assert!(is_inverse_pair("H", "H"));
        assert!(is_inverse_pair("CNOT", "CX"));
        assert!(is_inverse_pair("S", "S†"));
        assert!(is_inverse_pair("S_dag", "S"));
        assert!(is_inverse_pair("T", "Tdag"));
        assert!(!is_inverse_pair("H", "X"));
        assert!(!is_inverse_pair("Rz", "Rz")); // parametric — angle merge, not cancel
    }

    #[test]
    fn surface_gates_exclude_backend_only() {
        assert!(surface_gate("H").is_some());
        assert!(surface_gate("CNOT").is_some());
        assert!(surface_gate("Rzz").is_some());
        assert!(surface_gate("u3").is_none());
        assert!(surface_gate("ccx").is_none());
        // But lookup still finds them for emit/backend.
        assert!(lookup("u3").is_some());
        assert!(lookup("ccx").is_some());
    }

    #[test]
    fn std_gates_covers_openqasm_set() {
        let gates = std_gates();
        let names: Vec<&str> = gates.iter().map(|(n, _)| *n).collect();
        for expected in [
            "h", "x", "y", "z", "s", "sdg", "sx", "t", "tdg", "rx", "ry", "rz", "u1", "u2", "u3",
            "cx", "cy", "cz", "swap", "ccx",
        ] {
            assert!(
                names.contains(&expected),
                "missing OpenQASM gate `{expected}` in std_gates"
            );
        }
        assert_eq!(
            gates.iter().find(|(n, _)| *n == "cx").map(|(_, a)| *a),
            Some(2)
        );
        assert_eq!(
            gates.iter().find(|(n, _)| *n == "ccx").map(|(_, a)| *a),
            Some(3)
        );
    }

    #[test]
    fn clifford_class_matches_spec() {
        for name in [
            "I", "X", "Y", "Z", "H", "S", "S_dag", "SX", "SX_dag", "CNOT", "CZ",
        ] {
            assert_eq!(
                surface_gate(name).map(|g| g.class),
                Some(GateClass::Clifford),
                "{name}"
            );
        }
        for name in ["T", "T_dag", "Rx", "Rzz"] {
            assert_eq!(
                surface_gate(name).map(|g| g.class),
                Some(GateClass::Universal),
                "{name}"
            );
        }
    }
}
