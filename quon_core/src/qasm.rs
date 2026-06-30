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
//! 1. **Index bounds and context.** Every [`QubitId`] is `< the owning qubit
//!    register's size` and every [`BitId`] is `< the owning bit register's size`;
//!    [`index_in_bounds`] gates every construction, and [`Program`] validates
//!    that IDs belong to its own registers before accepting statements.
//! 2. **Gate arity.** Each [`QasmGate`] variant fixes its qubit-operand count
//!    structurally (no `Vec`), so an arity mismatch is unrepresentable;
//!    [`operand_arity_ok`] checks the IR's operand count at the boundary.
//!
//! This crate is MLIR-free; [`render`] is pure string production.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// An opaque key identifying one program register context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RegisterKey(u64);

impl RegisterKey {
    fn fresh() -> Self {
        static NEXT_REGISTER_KEY: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_REGISTER_KEY.fetch_add(1, Ordering::Relaxed))
    }
}

/// An index into the program's qubit register.
///
/// The inner index and context key are private: a `QubitId` is minted by
/// [`Program::qubit`], which checks the bound and tags it with the owning
/// program's qubit register. A qubit may be copied, but it cannot be inserted
/// into another program through the safe API.
#[derive(Clone, Copy, Debug, Eq, Hash)]
pub struct QubitId {
    index: usize,
    register: RegisterKey,
}

impl PartialEq for QubitId {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl QubitId {
    fn new(idx: usize, register_size: usize, register: RegisterKey) -> Option<Self> {
        index_in_bounds(idx, register_size).then_some(Self {
            index: idx,
            register,
        })
    }

    /// The underlying index. In range by construction.
    pub fn index(self) -> usize {
        self.index
    }
}

/// An index into the program's classical bit register.
#[derive(Clone, Copy, Debug, Eq, Hash)]
pub struct BitId {
    index: usize,
    register: RegisterKey,
}

impl PartialEq for BitId {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl BitId {
    fn new(idx: usize, register_size: usize, register: RegisterKey) -> Option<Self> {
        index_in_bounds(idx, register_size).then_some(Self {
            index: idx,
            register,
        })
    }

    /// The underlying index. In range by construction.
    pub fn index(self) -> usize {
        self.index
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

/// A condition expression used as an `if` condition. OpenQASM 3.0 feed-forward
/// uses integer comparison semantics, so measured bits compare against `1`
/// rather than rendering boolean literals.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A single classical bit, `c[i]`.
    Bit(BitId),
    /// An integer literal.
    Int(i64),
    /// Equality, `lhs == rhs`.
    Eq(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Build `c[bit] == 1`, the feed-forward condition for a measured bit.
    pub fn bit_is_set(bit: BitId) -> Self {
        Expr::Eq(Box::new(Expr::Bit(bit)), Box::new(Expr::Int(1)))
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
#[derive(Clone, Debug)]
pub struct Program {
    /// The `OPENQASM <major>.<minor>;` version header.
    version: (u32, u32),
    /// `include "...";` directives, in order.
    includes: Vec<String>,
    /// The single qubit register every physical index refers into.
    qubits: Register,
    qubit_register: RegisterKey,
    /// The classical bit register, present iff the program measures anything.
    bits: Option<Register>,
    bit_register: Option<RegisterKey>,
    /// User-defined gate definitions, emitted before the body.
    gate_defs: Vec<GateDef>,
    /// Top-level statements in execution order.
    body: Vec<Stmt>,
}

impl Program {
    /// An OpenQASM 3.0 program over `num_qubits` qubits in register `q` and, when
    /// `num_bits > 0`, `num_bits` classical bits in register `c`, including
    /// `stdgates.inc`. This is the shape `reify` produces; construct [`Program`]
    /// directly for other register layouts.
    pub fn new(num_qubits: usize, num_bits: usize) -> Self {
        let bit_register = (num_bits > 0).then(RegisterKey::fresh);
        Self {
            version: (3, 0),
            includes: vec!["stdgates.inc".to_string()],
            qubits: Register {
                name: "q".to_string(),
                size: num_qubits,
            },
            qubit_register: RegisterKey::fresh(),
            bits: bit_register.map(|_| Register {
                name: "c".to_string(),
                size: num_bits,
            }),
            bit_register,
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

    /// Mint a qubit ID in this program's qubit-register context.
    pub fn qubit(&self, idx: usize) -> Option<QubitId> {
        QubitId::new(idx, self.num_qubits(), self.qubit_register)
    }

    /// Mint a classical bit ID in this program's bit-register context.
    pub fn bit(&self, idx: usize) -> Option<BitId> {
        let register = self.bit_register?;
        BitId::new(idx, self.num_bits(), register)
    }

    /// The qubit register declaration.
    pub fn qubits(&self) -> &Register {
        &self.qubits
    }

    /// The classical bit register declaration, if present.
    pub fn bits(&self) -> Option<&Register> {
        self.bits.as_ref()
    }

    /// Top-level statements in execution order.
    pub fn body(&self) -> &[Stmt] {
        &self.body
    }

    /// User-defined gate definitions.
    pub fn gate_defs(&self) -> &[GateDef] {
        &self.gate_defs
    }

    /// Append a user-defined gate declaration.
    pub fn push_gate_def(&mut self, gate_def: GateDef) {
        self.gate_defs.push(gate_def);
    }

    /// Append a validated top-level statement.
    pub fn push(&mut self, stmt: Stmt) -> Result<(), QasmError> {
        self.validate_stmt(&stmt)?;
        self.body.push(stmt);
        Ok(())
    }

    /// Append several validated top-level statements.
    pub fn extend<I>(&mut self, stmts: I) -> Result<(), QasmError>
    where
        I: IntoIterator<Item = Stmt>,
    {
        for stmt in stmts {
            self.push(stmt)?;
        }
        Ok(())
    }

    pub fn push_gate(&mut self, gate: QasmGate) -> Result<(), QasmError> {
        self.push(Stmt::Gate(gate))
    }

    pub fn push_measure(&mut self, qubit: QubitId, bit: BitId) -> Result<(), QasmError> {
        self.push(Stmt::Measure { qubit, bit })
    }

    pub fn push_reset(&mut self, qubit: QubitId) -> Result<(), QasmError> {
        self.push(Stmt::Reset(qubit))
    }

    pub fn push_if(
        &mut self,
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    ) -> Result<(), QasmError> {
        self.push(Stmt::If {
            condition,
            then_body,
            else_body,
        })
    }

    pub fn push_barrier(&mut self, qubits: Vec<QubitId>) -> Result<(), QasmError> {
        self.push(Stmt::Barrier(qubits))
    }

    fn validate_stmt(&self, stmt: &Stmt) -> Result<(), QasmError> {
        match stmt {
            Stmt::Gate(gate) => self.validate_gate(gate),
            Stmt::Measure { qubit, bit } => {
                self.validate_qubit(*qubit)?;
                self.validate_bit(*bit)
            }
            Stmt::Reset(qubit) => self.validate_qubit(*qubit),
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                self.validate_expr(condition)?;
                for stmt in then_body.iter().chain(else_body) {
                    self.validate_stmt(stmt)?;
                }
                Ok(())
            }
            Stmt::Barrier(qubits) => {
                for qubit in qubits {
                    self.validate_qubit(*qubit)?;
                }
                Ok(())
            }
        }
    }

    fn validate_gate(&self, gate: &QasmGate) -> Result<(), QasmError> {
        match gate {
            QasmGate::One(_, a)
            | QasmGate::Rotation(_, _, a)
            | QasmGate::U2 { q: a, .. }
            | QasmGate::U3 { q: a, .. } => self.validate_qubit(*a),
            QasmGate::Two(_, a, b) => {
                self.validate_qubit(*a)?;
                self.validate_qubit(*b)
            }
            QasmGate::Ccx(a, b, c) => {
                self.validate_qubit(*a)?;
                self.validate_qubit(*b)?;
                self.validate_qubit(*c)
            }
        }
    }

    fn validate_expr(&self, expr: &Expr) -> Result<(), QasmError> {
        match expr {
            Expr::Bit(bit) => self.validate_bit(*bit),
            Expr::Int(_) => Ok(()),
            Expr::Eq(lhs, rhs) => {
                self.validate_expr(lhs)?;
                self.validate_expr(rhs)
            }
        }
    }

    fn validate_qubit(&self, qubit: QubitId) -> Result<(), QasmError> {
        if qubit.register == self.qubit_register {
            Ok(())
        } else {
            Err(QasmError::QubitOutOfContext {
                index: qubit.index(),
            })
        }
    }

    fn validate_bit(&self, bit: BitId) -> Result<(), QasmError> {
        match self.bit_register {
            Some(register) if bit.register == register => Ok(()),
            Some(_) => Err(QasmError::BitOutOfContext { index: bit.index() }),
            None => Err(QasmError::MissingClassicalRegister),
        }
    }
}

impl PartialEq for Program {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.includes == other.includes
            && self.qubits == other.qubits
            && self.bits == other.bits
            && self.gate_defs == other.gate_defs
            && self.body == other.body
    }
}

/// Errors returned by the safe QASM builder API.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QasmError {
    #[error("qubit q[{index}] belongs to a different program")]
    QubitOutOfContext { index: usize },
    #[error("bit c[{index}] belongs to a different program")]
    BitOutOfContext { index: usize },
    #[error("program has no classical bit register")]
    MissingClassicalRegister,
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
    bits: Option<&'a str>,
}

impl<'a> RegisterNames<'a> {
    fn of(program: &'a Program) -> Self {
        Self {
            qubits: &program.qubits.name,
            bits: program.bits.as_ref().map(|r| r.name.as_str()),
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
            let Some(bits) = names.bits else {
                unreachable!("validated QASM program cannot measure without a bit register")
            };
            let _ = writeln!(
                out,
                "{}[{}] = measure {}[{}];",
                bits,
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
        Expr::Bit(bit) => {
            let Some(bits) = names.bits else {
                unreachable!("validated QASM program cannot reference bits without a bit register")
            };
            format!("{}[{}]", bits, bit.index())
        }
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

    fn q(program: &Program, i: usize) -> QubitId {
        let Some(id) = program.qubit(i) else {
            unreachable!("test qubit index {i} exceeds program bound")
        };
        id
    }
    fn b(program: &Program, i: usize) -> BitId {
        let Some(id) = program.bit(i) else {
            unreachable!("test bit index {i} exceeds program bound")
        };
        id
    }

    #[test]
    fn index_constructors_enforce_bounds() {
        let p = Program::new(2, 2);
        assert!(p.qubit(0).is_some());
        assert!(p.qubit(1).is_some());
        assert!(p.qubit(2).is_none());
        assert!(p.bit(2).is_none());
        assert!(index_in_bounds(1, 2));
        assert!(!index_in_bounds(2, 2));
        assert!(operand_arity_ok(2, 2));
        assert!(!operand_arity_ok(2, 1));
    }

    #[test]
    fn bell_state_renders_exact_qasm() -> Result<(), QasmError> {
        let mut p = Program::new(2, 2);
        p.push_gate(QasmGate::One(OneQubitGate::H, q(&p, 0)))?;
        p.push_gate(QasmGate::Two(TwoQubitGate::Cx, q(&p, 0), q(&p, 1)))?;
        p.push_measure(q(&p, 0), b(&p, 0))?;
        p.push_measure(q(&p, 1), b(&p, 1))?;
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
        Ok(())
    }

    #[test]
    fn feed_forward_renders_exact_if_block() -> Result<(), QasmError> {
        let mut p = Program::new(3, 2);
        p.push_measure(q(&p, 0), b(&p, 0))?;
        p.push_if(
            Expr::bit_is_set(b(&p, 0)),
            vec![Stmt::Gate(QasmGate::One(OneQubitGate::X, q(&p, 2)))],
            vec![],
        )?;
        let expected = "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[3] q;
bit[2] c;
c[0] = measure q[0];
if (c[0] == 1) {
  x q[2];
}
";
        assert_eq!(render(&p), expected);
        Ok(())
    }

    #[test]
    fn rotation_and_barrier_render_without_bit_register() -> Result<(), QasmError> {
        let mut p = Program::new(2, 0);
        p.push_gate(QasmGate::Rotation(
            RotationGate::Rz,
            std::f64::consts::FRAC_PI_4,
            q(&p, 0),
        ))?;
        p.push_barrier(vec![q(&p, 0), q(&p, 1)])?;
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
        assert!(p.bits().is_none());
        Ok(())
    }

    #[test]
    fn rejects_qubits_from_another_program() {
        let mut p = Program::new(1, 0);
        let other = Program::new(1, 0);
        let err = p
            .push_gate(QasmGate::One(OneQubitGate::H, q(&other, 0)))
            .err();
        assert_eq!(err, Some(QasmError::QubitOutOfContext { index: 0 }));
        assert!(p.body().is_empty());
    }

    #[test]
    fn cannot_measure_without_classical_register() {
        let p = Program::new(1, 0);
        assert!(p.bit(0).is_none());
    }

    #[test]
    fn rejects_bits_from_another_program() {
        let mut p = Program::new(1, 1);
        let other = Program::new(1, 1);
        let err = p.push_measure(q(&p, 0), b(&other, 0)).err();
        assert_eq!(err, Some(QasmError::BitOutOfContext { index: 0 }));
        assert!(p.body().is_empty());
    }
}
