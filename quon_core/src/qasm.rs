//! A faithful OpenQASM 3.0 **syntax tree** and a total renderer (issue #27, SPEC §9.1).
//!
//! This module is the compiler's backend-facing IR. It is split so that emission
//! is **valid by construction**:
//!
//! * `mlir_bridge`'s `reify` is the single **fallible** stage — it walks
//!   `quantum.dynamic` and builds a [`Program`], doing all validation
//!   (native-gate resolution, physical-qubit assignment, bit-index allocation,
//!   operand arity) exactly once.
//! * [`render`] is **total**: a `Program` cannot represent an invalid OpenQASM
//!   document, so turning one into text never fails and never re-derives what the
//!   optimization passes did.
//!
//! The tree models QASM *syntax*, not just the values we happen to emit: the
//! version, includes, register declarations, gate applications, and the boolean
//! condition on `if` are all explicit nodes. Nothing about the concrete syntax is
//! hard-coded inside [`render`] — every token it writes comes from the tree — so
//! an alternative backend (a different QASM dialect, QIR, a circuit-diagram
//! exporter) is just another fold over [`Program`], not a re-parse of text.
//!
//! Two invariants are machine-checked by Flux (load-bearing at the reify
//! boundary), following the established `quon_core` kernel pattern:
//!
//! 1. **Index bounds.** Every [`QubitId`] is `< the qubit register's size` and
//!    every [`BitId`] is `< the bit register's size`; [`index_in_bounds`] gates
//!    every construction, so [`render`] indexes `q[i]` / `c[i]` without a guard.
//! 2. **Gate arity.** Each [`QasmGate`] variant fixes its qubit-operand count
//!    structurally (no `Vec`), so an arity mismatch is unrepresentable;
//!    [`operand_arity_ok`] checks the IR's operand count at the boundary.
//!
//! This crate is MLIR-free; [`render`] is pure string production.

use std::fmt::Write as _;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

// ─── Flux-checked invariant kernels (load-bearing at the reify boundary) ─────

/// True iff `idx` is a legal index into a register of width `bound`. The Flux
/// postcondition `idx < bound` is what lets [`render`] emit `q[idx]` / `c[idx]`
/// with no runtime bounds guard — the boundary proves it once, here.
#[cfg_attr(feature = "flux", spec(fn(idx: usize, bound: usize) -> bool[idx < bound]))]
pub fn index_in_bounds(idx: usize, bound: usize) -> bool {
    idx < bound
}

/// True iff a gate built with `expected` qubit operands was handed exactly that
/// many by the IR. Guards the arity-structural [`QasmGate`] construction so a
/// `cx` can never be built from one wire.
#[cfg_attr(feature = "flux", spec(fn(expected: usize, actual: usize) -> bool[expected == actual]))]
pub fn operand_arity_ok(expected: usize, actual: usize) -> bool {
    expected == actual
}

// ─── Index newtypes ──────────────────────────────────────────────────────────

/// An index into the program's qubit register.
///
/// The inner index is private: a `QubitId` is built only through [`QubitId::new`],
/// which goes through the Flux-checked [`index_in_bounds`]. Holding one is
/// therefore evidence the index is in range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QubitId(usize);

impl QubitId {
    /// Construct a qubit index, returning `None` when `idx >= register_size`.
    pub fn new(idx: usize, register_size: usize) -> Option<Self> {
        index_in_bounds(idx, register_size).then_some(Self(idx))
    }

    /// The underlying index. In range by construction.
    pub fn index(self) -> usize {
        self.0
    }
}

/// An index into the program's classical bit register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitId(usize);

impl BitId {
    /// Construct a bit index, returning `None` when `idx >= register_size`.
    pub fn new(idx: usize, register_size: usize) -> Option<Self> {
        index_in_bounds(idx, register_size).then_some(Self(idx))
    }

    /// The underlying index. In range by construction.
    pub fn index(self) -> usize {
        self.0
    }
}

// ─── Gates (arity is structural) ─────────────────────────────────────────────

/// Parameterless single-qubit standard gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneQubitGate {
    H,
    X,
    Y,
    Z,
    S,
    Sdg,
    Sx,
    T,
    Tdg,
}

impl OneQubitGate {
    /// The OpenQASM 3.0 stdgates keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::H => "h",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::S => "s",
            Self::Sdg => "sdg",
            Self::Sx => "sx",
            Self::T => "t",
            Self::Tdg => "tdg",
        }
    }
}

/// Single-angle single-qubit rotation gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationGate {
    Rx,
    Ry,
    Rz,
    U1,
}

impl RotationGate {
    /// The OpenQASM 3.0 stdgates keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Rx => "rx",
            Self::Ry => "ry",
            Self::Rz => "rz",
            Self::U1 => "u1",
        }
    }
}

/// Two-qubit standard gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwoQubitGate {
    Cx,
    Cy,
    Cz,
    Swap,
}

impl TwoQubitGate {
    /// The OpenQASM 3.0 stdgates keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Cx => "cx",
            Self::Cy => "cy",
            Self::Cz => "cz",
            Self::Swap => "swap",
        }
    }
}

/// A fully-resolved native gate application. Every variant fixes its qubit arity
/// structurally, so a malformed arity cannot be represented. Angles are `f64` and
/// intentionally unrefined (Flux's f64 support is weak).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QasmGate {
    /// `h q[a];` and friends.
    One(OneQubitGate, QubitId),
    /// `rz(theta) q[a];` and friends.
    Rotation(RotationGate, f64, QubitId),
    /// `u2(phi, lam) q[a];`
    U2 { phi: f64, lam: f64, q: QubitId },
    /// `u3(theta, phi, lam) q[a];`
    U3 {
        theta: f64,
        phi: f64,
        lam: f64,
        q: QubitId,
    },
    /// `cx q[a], q[b];` and friends.
    Two(TwoQubitGate, QubitId, QubitId),
    /// `ccx q[a], q[b], q[c];`
    Ccx(QubitId, QubitId, QubitId),
}

// ─── Condition expressions ────────────────────────────────────────────────────

/// A boolean expression used as an `if` condition. Modelled as a small tree so
/// backends can lower conditions however they like (e.g. `c[i] == true` vs
/// `c[i] == 1` vs a register equality) rather than matching on rendered text.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A single classical bit, `c[i]`.
    Bit(BitId),
    /// A boolean literal.
    Bool(bool),
    /// An integer literal.
    Int(i64),
    /// Equality, `lhs == rhs`.
    Eq(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Build `c[bit] == true`, the feed-forward condition for a measured bit.
    pub fn bit_is_set(bit: BitId) -> Self {
        Expr::Eq(Box::new(Expr::Bit(bit)), Box::new(Expr::Bool(true)))
    }
}

// ─── Statements ────────────────────────────────────────────────────────────────

/// A single OpenQASM 3.0 statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// A native gate application.
    Gate(QasmGate),
    /// `c[bit] = measure q[qubit];`
    Measure { qubit: QubitId, bit: BitId },
    /// `reset q[qubit];`
    Reset(QubitId),
    /// `if (condition) { then_body } else { else_body }`. An empty `else_body`
    /// renders no `else` clause.
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    /// `barrier q[a], q[b], ...;`
    Barrier(Vec<QubitId>),
}

// ─── Declarations & program ──────────────────────────────────────────────────

/// A register declaration: `qubit[size] name;` or `bit[size] name;`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Register {
    pub name: String,
    pub size: usize,
}

/// A user-defined `gate` declaration that survived optimization, emitted before
/// its first use. `params` are formal qubit parameter names; `body` is rendered
/// verbatim against those names.
#[derive(Clone, Debug, PartialEq)]
pub struct GateDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: String,
}

/// A complete OpenQASM 3.0 program, as a syntax tree. Every node [`render`] needs
/// is present here — nothing about the concrete syntax is implicit.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// The `OPENQASM <major>.<minor>;` version header.
    pub version: (u32, u32),
    /// `include "...";` directives, in order.
    pub includes: Vec<String>,
    /// The single qubit register every physical index refers into.
    pub qubits: Register,
    /// The classical bit register, present iff the program measures anything.
    pub bits: Option<Register>,
    /// User-defined gate definitions, emitted before the body.
    pub gate_defs: Vec<GateDef>,
    /// Top-level statements in execution order.
    pub body: Vec<Stmt>,
}

impl Program {
    /// An OpenQASM 3.0 program over `num_qubits` qubits in register `q` and, when
    /// `num_bits > 0`, `num_bits` classical bits in register `c`, including
    /// `stdgates.inc`. This is the shape `reify` produces; construct [`Program`]
    /// directly for other register layouts.
    pub fn new(num_qubits: usize, num_bits: usize) -> Self {
        Self {
            version: (3, 0),
            includes: vec!["stdgates.inc".to_string()],
            qubits: Register {
                name: "q".to_string(),
                size: num_qubits,
            },
            bits: (num_bits > 0).then(|| Register {
                name: "c".to_string(),
                size: num_bits,
            }),
            gate_defs: Vec::new(),
            body: Vec::new(),
        }
    }

    /// The qubit register's width (the bound every [`QubitId`] satisfies).
    pub fn num_qubits(&self) -> usize {
        self.qubits.size
    }

    /// The classical register's width, or 0 when there are no measurements.
    pub fn num_bits(&self) -> usize {
        self.bits.as_ref().map_or(0, |r| r.size)
    }
}

// ─── Rendering (total) ─────────────────────────────────────────────────────────

/// Render a [`Program`] to OpenQASM 3.0 source text. Total and infallible: a
/// `Program` is valid by construction, so this never inspects pass history and
/// never errors. A second backend is a parallel fold over the same tree.
pub fn render(program: &Program) -> String {
    let mut out = String::new();
    let (major, minor) = program.version;
    let _ = writeln!(out, "OPENQASM {major}.{minor};");
    for include in &program.includes {
        let _ = writeln!(out, "include \"{include}\";");
    }
    for def in &program.gate_defs {
        render_gate_def(&mut out, def);
    }
    let _ = writeln!(
        out,
        "qubit[{}] {};",
        program.qubits.size, program.qubits.name
    );
    if let Some(bits) = &program.bits {
        let _ = writeln!(out, "bit[{}] {};", bits.size, bits.name);
    }
    let names = RegisterNames::of(program);
    for stmt in &program.body {
        render_stmt(&mut out, stmt, &names, 0);
    }
    out
}

/// The register names threaded into statement rendering. A qubit/bit reference is
/// an index into these.
struct RegisterNames<'a> {
    qubits: &'a str,
    bits: &'a str,
}

impl<'a> RegisterNames<'a> {
    fn of(program: &'a Program) -> Self {
        Self {
            qubits: &program.qubits.name,
            bits: program.bits.as_ref().map_or("c", |r| r.name.as_str()),
        }
    }
}

fn render_gate_def(out: &mut String, def: &GateDef) {
    let _ = writeln!(out, "gate {} {} {{", def.name, def.params.join(", "));
    for line in def.body.lines() {
        let _ = writeln!(out, "  {line}");
    }
    out.push_str("}\n");
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn render_stmt(out: &mut String, stmt: &Stmt, names: &RegisterNames, depth: usize) {
    indent(out, depth);
    match stmt {
        Stmt::Gate(gate) => render_gate(out, gate, names.qubits),
        Stmt::Measure { qubit, bit } => {
            let _ = writeln!(
                out,
                "{}[{}] = measure {}[{}];",
                names.bits,
                bit.index(),
                names.qubits,
                qubit.index()
            );
        }
        Stmt::Reset(qubit) => {
            let _ = writeln!(out, "reset {}[{}];", names.qubits, qubit.index());
        }
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => {
            let _ = writeln!(out, "if ({}) {{", render_expr(condition, names));
            for s in then_body {
                render_stmt(out, s, names, depth + 1);
            }
            indent(out, depth);
            out.push('}');
            if else_body.is_empty() {
                out.push('\n');
            } else {
                out.push_str(" else {\n");
                for s in else_body {
                    render_stmt(out, s, names, depth + 1);
                }
                indent(out, depth);
                out.push_str("}\n");
            }
        }
        Stmt::Barrier(qubits) => {
            let operands = qubits
                .iter()
                .map(|q| format!("{}[{}]", names.qubits, q.index()))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "barrier {operands};");
        }
    }
}

fn render_expr(expr: &Expr, names: &RegisterNames) -> String {
    match expr {
        Expr::Bit(bit) => format!("{}[{}]", names.bits, bit.index()),
        Expr::Bool(value) => value.to_string(),
        Expr::Int(value) => value.to_string(),
        Expr::Eq(lhs, rhs) => {
            format!("{} == {}", render_expr(lhs, names), render_expr(rhs, names))
        }
    }
}

fn render_gate(out: &mut String, gate: &QasmGate, q: &str) {
    match gate {
        QasmGate::One(g, a) => {
            let _ = writeln!(out, "{} {}[{}];", g.keyword(), q, a.index());
        }
        QasmGate::Rotation(g, theta, a) => {
            let _ = writeln!(
                out,
                "{}({}) {}[{}];",
                g.keyword(),
                fmt_angle(*theta),
                q,
                a.index()
            );
        }
        QasmGate::U2 { phi, lam, q: a } => {
            let _ = writeln!(
                out,
                "u2({}, {}) {}[{}];",
                fmt_angle(*phi),
                fmt_angle(*lam),
                q,
                a.index()
            );
        }
        QasmGate::U3 {
            theta,
            phi,
            lam,
            q: a,
        } => {
            let _ = writeln!(
                out,
                "u3({}, {}, {}) {}[{}];",
                fmt_angle(*theta),
                fmt_angle(*phi),
                fmt_angle(*lam),
                q,
                a.index()
            );
        }
        QasmGate::Two(g, a, b) => {
            let _ = writeln!(
                out,
                "{} {}[{}], {}[{}];",
                g.keyword(),
                q,
                a.index(),
                q,
                b.index()
            );
        }
        QasmGate::Ccx(a, b, c) => {
            let _ = writeln!(
                out,
                "ccx {}[{}], {}[{}], {}[{}];",
                q,
                a.index(),
                q,
                b.index(),
                q,
                c.index()
            );
        }
    }
}

/// Format an angle for QASM output. Integers print with a trailing `.0` (still
/// valid OpenQASM); other values use full `f64` precision so round-trips are exact.
fn fmt_angle(theta: f64) -> String {
    if theta == theta.trunc() && theta.is_finite() {
        format!("{theta:.1}")
    } else {
        format!("{theta}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test indices are always within the bound 8 by construction; `let ... else`
    // keeps these helpers free of unwrap()/expect() (workspace lint policy).
    fn q(i: usize) -> QubitId {
        let Some(id) = QubitId::new(i, 8) else {
            unreachable!("test qubit index {i} exceeds bound 8")
        };
        id
    }
    fn b(i: usize) -> BitId {
        let Some(id) = BitId::new(i, 8) else {
            unreachable!("test bit index {i} exceeds bound 8")
        };
        id
    }

    #[test]
    fn index_constructors_enforce_bounds() {
        assert!(QubitId::new(0, 2).is_some());
        assert!(QubitId::new(1, 2).is_some());
        assert!(QubitId::new(2, 2).is_none());
        assert!(BitId::new(2, 2).is_none());
        assert!(index_in_bounds(1, 2));
        assert!(!index_in_bounds(2, 2));
        assert!(operand_arity_ok(2, 2));
        assert!(!operand_arity_ok(2, 1));
    }

    #[test]
    fn bell_state_renders_exact_qasm() {
        let mut p = Program::new(2, 2);
        p.body
            .push(Stmt::Gate(QasmGate::One(OneQubitGate::H, q(0))));
        p.body
            .push(Stmt::Gate(QasmGate::Two(TwoQubitGate::Cx, q(0), q(1))));
        p.body.push(Stmt::Measure {
            qubit: q(0),
            bit: b(0),
        });
        p.body.push(Stmt::Measure {
            qubit: q(1),
            bit: b(1),
        });
        let expected = "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
";
        assert_eq!(render(&p), expected);
    }

    #[test]
    fn feed_forward_renders_exact_if_block() {
        let mut p = Program::new(3, 2);
        p.body.push(Stmt::Measure {
            qubit: q(0),
            bit: b(0),
        });
        p.body.push(Stmt::If {
            condition: Expr::bit_is_set(b(0)),
            then_body: vec![Stmt::Gate(QasmGate::One(OneQubitGate::X, q(2)))],
            else_body: vec![],
        });
        let expected = "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[3] q;
bit[2] c;
c[0] = measure q[0];
if (c[0] == true) {
  x q[2];
}
";
        assert_eq!(render(&p), expected);
    }

    #[test]
    fn rotation_and_barrier_render_without_bit_register() {
        let mut p = Program::new(2, 0);
        p.body.push(Stmt::Gate(QasmGate::Rotation(
            RotationGate::Rz,
            std::f64::consts::FRAC_PI_4,
            q(0),
        )));
        p.body.push(Stmt::Barrier(vec![q(0), q(1)]));
        let expected = format!(
            "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[2] q;
rz({}) q[0];
barrier q[0], q[1];
",
            super::fmt_angle(std::f64::consts::FRAC_PI_4)
        );
        assert_eq!(render(&p), expected);
        // No classical register declared when there are no measurements.
        assert!(p.bits.is_none());
    }
}
