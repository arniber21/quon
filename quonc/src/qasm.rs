//! Minimal OpenQASM 2/3 ingestion for the neutral-atom pipeline (#304).
//!
//! Quon only *emits* OpenQASM (via `mlir_bridge`); this is the inverse entry
//! point for external benchmark circuits; issue #197's NA-scoped slice. A
//! hand-rolled parser covers the benchmark QASM2/3 subset — the `OPENQASM`
//! header, `include` directives, `qreg`/`qubit` declarations, gate
//! statements (`name [params] q[i],q[j];`), `creg`/`bit` classical
//! declarations (ignored for the NA path), `measure`/`reset` (ignored —
//! not interactions), `barrier` (segment flush), and `//` / `/* */`
//! comments. No external QASM crate is pulled in (issue constraint).
//!
//! The parsed program is lowered to the same contract
//! `quon_na::extract::extract_interaction_graph_and_local_gates` produces
//! for `.qn` source — an [`InteractionGraph`] plus captured 1-qubit
//! [`LocalGateExtract`]s — so the result enters the existing NA pipeline
//! through `quon_na::pipeline::run_from_graph_with_local_gates` unchanged:
//! each ≥2-qubit gate becomes one multi-qubit interaction edge; 1-qubit
//! gates are preserved per #298 and anchored to the most recent ≥2-qubit
//! interaction on their qubit. Qubit operands map to `LogicalQubitId`s
//! dense in declaration order across `qreg`/`qubit` registers.
//!
//! Scope is deliberately the benchmark subset: `gate`/`opaque` definitions
//! and classical control flow (`if`/`for`/`while`) are rejected with
//! actionable, line-tagged errors rather than silently dropped.

use std::collections::HashMap;

use thiserror::Error;

use quon_na::extract::LocalGateExtract;
use quon_na::graph::{
    DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, SegmentKind, schedule_dependency_segment,
};

/// One parsed qubit register declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QasmQreg {
    pub name: String,
    pub size: u32,
}

/// One parsed gate statement (post-parameter-evaluation).
#[derive(Clone, Debug, PartialEq)]
pub struct QasmGate {
    pub name: String,
    /// Resolved angle parameters (empty for fixed gates). Only the first is
    /// carried into [`LocalGateExtract::angle`] today, mirroring
    /// `quon_na::extract`'s single-`angle` capture; multi-parameter gates
    /// (`u2`/`u3`) lose their trailing params at this seam (a known
    /// limitation, documented in #304 — the benchmark subset uses at most
    /// one parameter per 1-qubit gate).
    pub params: Vec<f64>,
    /// `(register, index)` operands in source order.
    pub operands: Vec<(String, u32)>,
    pub line: u32,
}

/// A parsed OpenQASM program: qubit registers and gate statements in source
/// order. Classical declarations, `include`s, and the `OPENQASM` header are
/// consumed but not retained (they don't affect the NA interaction graph).
#[derive(Clone, Debug, PartialEq)]
pub struct QasmProgram {
    pub qregs: Vec<QasmQreg>,
    pub gates: Vec<QasmGate>,
    /// `barrier` statements split the program into dependency-DAG segments;
    /// each entry is the index of the gate that *starts* a new segment (the
    /// 0th segment is implicit). Equivalent to `barrier` flushing a
    /// [`SegmentKind::DependencyDag`] group in `quon_na::extract`.
    pub segment_starts: Vec<usize>,
}

/// Actionable errors while parsing or lowering an OpenQASM program (#304).
///
/// Every variant carries the 1-based source `line` so CLI/test failures
/// point straight at the offending construct; no silent drops.
#[derive(Debug, Error)]
pub enum QasmError {
    #[error("line {line}: {message}")]
    Lex { line: u32, message: String },
    #[error("line {line}: {message}")]
    Parse { line: u32, message: String },
    #[error(
        "line {line}: unsupported QASM construct `{construct}` — only the benchmark subset is supported (gate calls, qreg/qubit, creg/bit, measure, reset, barrier, include, OPENQASM header)"
    )]
    Unsupported { line: u32, construct: String },
    #[error("line {line}: qubit index {index} out of range for register `{reg}` of size {size}")]
    QubitOutOfRange {
        line: u32,
        reg: String,
        index: u32,
        size: u32,
    },
    #[error("line {line}: unknown qubit register `{reg}`")]
    UnknownRegister { line: u32, reg: String },
    #[error("line {line}: gate `{gate}` got {got} qubit operand(s) but needs at least 1")]
    EmptyOperands { line: u32, gate: String, got: usize },
    #[error(
        "line {line}: no qubit register declared before this gate — add a `qreg`/`qubit` declaration"
    )]
    NoQreg { line: u32 },
    #[error("interaction-graph construction failed: {0}")]
    Graph(#[from] quon_na::GraphError),
}

// ───────────────────────────── lexer ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// Raw numeric lexeme (integer or float); parsed to `f64`/`u32` by the
    /// consumer that knows the expected kind.
    Num(String),
    Str(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
    Equals,
    Colon,
    Arrow,
}
struct SpanTok {
    line: u32,
    kind: Tok,
}

fn lex(src: &str) -> Result<Vec<SpanTok>, QasmError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line: u32 = 1;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // line comment to end of line
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // block comment to matching `*/`
                i += 2;
                let start_line = line;
                let mut closed = false;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err(QasmError::Lex {
                        line: start_line,
                        message: "unterminated block comment".to_string(),
                    });
                }
            }
            b'(' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::LParen,
                });
                i += 1;
            }
            b')' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::RParen,
                });
                i += 1;
            }
            b'[' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::LBracket,
                });
                i += 1;
            }
            b']' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::RBracket,
                });
                i += 1;
            }
            b'{' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::LBrace,
                });
                i += 1;
            }
            b'}' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::RBrace,
                });
                i += 1;
            }
            b',' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Comma,
                });
                i += 1;
            }
            b';' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Semicolon,
                });
                i += 1;
            }
            b'+' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Plus,
                });
                i += 1;
            }
            b'-' => {
                // `->` arrow (measure target) vs minus
                if bytes.get(i + 1) == Some(&b'>') {
                    out.push(SpanTok {
                        line,
                        kind: Tok::Arrow,
                    });
                    i += 2;
                } else {
                    out.push(SpanTok {
                        line,
                        kind: Tok::Minus,
                    });
                    i += 1;
                }
            }
            b'*' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Star,
                });
                i += 1;
            }
            b'/' => {
                // not a comment (handled above) — division operator
                out.push(SpanTok {
                    line,
                    kind: Tok::Slash,
                });
                i += 1;
            }
            b'=' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Equals,
                });
                i += 1;
            }
            b':' => {
                out.push(SpanTok {
                    line,
                    kind: Tok::Colon,
                });
                i += 1;
            }
            b'"' => {
                let start_line = line;
                i += 1;
                let mut s = String::new();
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    s.push(bytes[i] as char);
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(QasmError::Lex {
                        line: start_line,
                        message: "unterminated string literal".to_string(),
                    });
                }
                i += 1; // closing quote
                out.push(SpanTok {
                    line,
                    kind: Tok::Str(s),
                });
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                // optional scientific suffix: e[+/-]digits
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let lexeme = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| QasmError::Lex {
                        line,
                        message: format!("non-utf8 numeric literal: {e}"),
                    })?
                    .to_string();
                out.push(SpanTok {
                    line,
                    kind: Tok::Num(lexeme),
                });
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let ident = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| QasmError::Lex {
                        line,
                        message: format!("non-utf8 identifier: {e}"),
                    })?
                    .to_string();
                out.push(SpanTok {
                    line,
                    kind: Tok::Ident(ident),
                });
            }
            other => {
                return Err(QasmError::Lex {
                    line,
                    message: format!("unexpected character `{other}` (byte 0x{other:02x})"),
                });
            }
        }
    }
    Ok(out)
}

// ───────────────────────────── parser ────────────────────────────

/// Parse an OpenQASM 2/3 program into a [`QasmProgram`].
pub fn parse(src: &str) -> Result<QasmProgram, QasmError> {
    let tokens = lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        qregs: Vec::new(),
        gates: Vec::new(),
        segment_starts: Vec::new(),
        reg_offsets: HashMap::new(),
        reg_sizes: HashMap::new(),
    };
    p.run()?;
    Ok(QasmProgram {
        qregs: p.qregs,
        gates: p.gates,
        segment_starts: p.segment_starts,
    })
}

struct Parser {
    tokens: Vec<SpanTok>,
    pos: usize,
    qregs: Vec<QasmQreg>,
    gates: Vec<QasmGate>,
    segment_starts: Vec<usize>,
    /// register name → (offset, size) into the global qubit id space.
    reg_offsets: HashMap<String, u32>,
    reg_sizes: HashMap<String, u32>,
}

impl Parser {
    fn run(&mut self) -> Result<(), QasmError> {
        while self.pos < self.tokens.len() {
            let tok = &self.tokens[self.pos];
            match &tok.kind {
                Tok::Ident(name) => {
                    let line = tok.line;
                    match name.as_str() {
                        "OPENQASM" => self.skip_header(line)?,
                        "include" => self.skip_include(line)?,
                        "qreg" => self.parse_qreg(line)?,
                        "qubit" => self.parse_qubit3(line)?,
                        "creg" | "bit" => self.skip_classical(line)?,
                        "gate" | "opaque" => {
                            return Err(QasmError::Unsupported {
                                line,
                                construct: format!("{name} … (gate definition)"),
                            });
                        }
                        "if" | "for" | "while" => {
                            return Err(QasmError::Unsupported {
                                line,
                                construct: format!("{name} … (classical control flow)"),
                            });
                        }
                        "barrier" => self.parse_barrier(line)?,
                        "measure" => self.skip_measure(line)?,
                        "reset" => self.skip_reset(line)?,
                        _ => self.parse_gate_call(line)?,
                    }
                }
                Tok::Semicolon => {
                    // stray semicolon (e.g. empty statement) — tolerate.
                    self.pos += 1;
                }
                other => {
                    return Err(QasmError::Parse {
                        line: tok.line,
                        message: format!("expected a statement, found `{other:?}`"),
                    });
                }
            }
        }
        Ok(())
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn expect(&mut self, want: &Tok, line: u32, what: &str) -> Result<(), QasmError> {
        match self.tokens.get(self.pos).map(|t| &t.kind) {
            Some(k) if k == want => {
                self.pos += 1;
                Ok(())
            }
            Some(k) => Err(QasmError::Parse {
                line,
                message: format!("expected {what}, found `{k:?}`"),
            }),
            None => Err(QasmError::Parse {
                line,
                message: format!("expected {what}, found end of input"),
            }),
        }
    }

    fn eat_ident(&mut self, line: u32, what: &str) -> Result<String, QasmError> {
        match self.tokens.get(self.pos).map(|t| t.kind.clone()) {
            Some(Tok::Ident(s)) => {
                self.pos += 1;
                Ok(s.clone())
            }
            Some(k) => Err(QasmError::Parse {
                line,
                message: format!("expected {what} identifier, found `{k:?}`"),
            }),
            None => Err(QasmError::Parse {
                line,
                message: format!("expected {what} identifier, found end of input"),
            }),
        }
    }

    /// `OPENQASM <version> ;`
    fn skip_header(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // OPENQASM
        // version: a number (2.0 / 3.0) or ident (e.g. `3`) — consume until `;`.
        while let Some(tok) = self.tokens.get(self.pos) {
            if matches!(tok.kind, Tok::Semicolon) {
                self.pos += 1;
                return Ok(());
            }
            self.pos += 1;
        }
        Err(QasmError::Parse {
            line,
            message: "OPENQASM header missing trailing `;`".to_string(),
        })
    }

    /// `include "<file>" ;`
    fn skip_include(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // include
        match self.tokens.get(self.pos).map(|t| t.kind.clone()) {
            Some(Tok::Str(_)) => {
                self.pos += 1;
            }
            Some(k) => {
                return Err(QasmError::Parse {
                    line,
                    message: format!("expected include string, found `{k:?}`"),
                });
            }
            None => {
                return Err(QasmError::Parse {
                    line,
                    message: "expected include string, found end of input".to_string(),
                });
            }
        }
        self.expect(&Tok::Semicolon, line, "`;` after include")
    }

    /// `qreg <name> [ <size> ] ;`
    fn parse_qreg(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // qreg
        let name = self.eat_ident(line, "register")?;
        self.expect(&Tok::LBracket, line, "`[`")?;
        let size = self.parse_u32(line, "register size")?;
        self.expect(&Tok::RBracket, line, "`]`")?;
        self.expect(&Tok::Semicolon, line, "`;`")?;
        self.declare_qreg(name, size, line)?;
        Ok(())
    }

    /// QASM3 `qubit [ <size> ] <name> ;` or `qubit <name> ;` (size 1).
    fn parse_qubit3(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // qubit
        let size = match self.peek() {
            Some(Tok::LBracket) => {
                self.pos += 1;
                let s = self.parse_u32(line, "qubit size")?;
                self.expect(&Tok::RBracket, line, "`]`")?;
                s
            }
            _ => 1,
        };
        let name = self.eat_ident(line, "register")?;
        self.expect(&Tok::Semicolon, line, "`;`")?;
        self.declare_qreg(name, size, line)?;
        Ok(())
    }

    /// `creg <name> [ <size> ] ;` / QASM3 `bit [ <size> ] <name> ;` / `bit <name> ;`
    fn skip_classical(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // creg | bit
        // optional `[ size ]`
        if matches!(self.peek(), Some(Tok::LBracket)) {
            self.pos += 1;
            // consume up to and including `]`
            while let Some(tok) = self.tokens.get(self.pos) {
                self.pos += 1;
                if matches!(tok.kind, Tok::RBracket) {
                    break;
                }
            }
        }
        // name
        if matches!(self.peek(), Some(Tok::Ident(_))) {
            self.pos += 1;
        }
        self.expect(&Tok::Semicolon, line, "`;`")?;
        Ok(())
    }

    fn declare_qreg(&mut self, name: String, size: u32, line: u32) -> Result<(), QasmError> {
        if size == 0 {
            return Err(QasmError::Parse {
                line,
                message: format!("register `{name}` declared with size 0"),
            });
        }
        if self.reg_offsets.contains_key(&name) {
            return Err(QasmError::Parse {
                line,
                message: format!("register `{name}` declared twice"),
            });
        }
        let offset = self.reg_sizes.values().copied().sum::<u32>();
        self.reg_offsets.insert(name.clone(), offset);
        self.reg_sizes.insert(name.clone(), size);
        self.qregs.push(QasmQreg { name, size });
        Ok(())
    }

    /// `barrier <operand> (, <operand>)* ;` — flush a new dependency-DAG
    /// segment. Operands are parsed (and validated against declared
    /// registers) but otherwise discarded: the NA scheduler only needs the
    /// segment boundary, not which qubits the barrier touched.
    fn parse_barrier(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // barrier
        if !self.qregs.is_empty() {
            self.parse_operands(line)?;
        }
        self.expect(&Tok::Semicolon, line, "`;`")?;
        // The next gate appended starts a new dependency-DAG segment. Record
        // its (0-based) gate index; the implicit segment 0 starts at index 0.
        // Skip a barrier before any gate or a duplicate (back-to-back
        // barriers) — those don't start a fresh non-empty segment.
        let start = self.gates.len();
        if start > *self.segment_starts.last().unwrap_or(&0) {
            self.segment_starts.push(start);
        }
        Ok(())
    }

    /// `measure <qoperand> (-> <coperand>)? ;` — ignored for the NA path
    /// (measurement is orchestrated separately by the `.qn` `measure_all`
    /// construct; QASM benchmarks without measurement stay measurement-free).
    fn skip_measure(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // measure
        // qubit operand(s) — accept `q[i]` or a bare register; tolerate `->` target.
        self.skip_operands_until_semicolon(line)?;
        Ok(())
    }

    /// `reset <qoperand> ;` — ignored (not an interaction).
    fn skip_reset(&mut self, line: u32) -> Result<(), QasmError> {
        self.pos += 1; // reset
        self.skip_operands_until_semicolon(line)?;
        Ok(())
    }

    /// Consume operands (and an optional `->` classical target) up to `;`.
    fn skip_operands_until_semicolon(&mut self, line: u32) -> Result<(), QasmError> {
        while let Some(tok) = self.tokens.get(self.pos) {
            match &tok.kind {
                Tok::Semicolon => {
                    self.pos += 1;
                    return Ok(());
                }
                Tok::Arrow => {
                    self.pos += 1;
                }
                Tok::Ident(_) | Tok::LBracket | Tok::RBracket | Tok::Num(_) | Tok::Comma => {
                    self.pos += 1;
                }
                k => {
                    return Err(QasmError::Parse {
                        line: tok.line,
                        message: format!("unexpected token `{k:?}` in measure/reset"),
                    });
                }
            }
        }
        Err(QasmError::Parse {
            line,
            message: "measure/reset missing trailing `;`".to_string(),
        })
    }

    /// `<name> [ ( <params> ) ] <operand> (, <operand>)* ;`
    fn parse_gate_call(&mut self, line: u32) -> Result<(), QasmError> {
        let name = self.eat_ident(line, "gate")?;
        if self.qregs.is_empty() {
            return Err(QasmError::NoQreg { line });
        }
        let params = if matches!(self.peek(), Some(Tok::LParen)) {
            self.parse_params(line)?
        } else {
            Vec::new()
        };
        let operands = self.parse_operands(line)?;
        self.expect(&Tok::Semicolon, line, "`;`")?;
        self.gates.push(QasmGate {
            name,
            params,
            operands,
            line,
        });
        Ok(())
    }

    /// `( <expr> (, <expr>)* )`
    fn parse_params(&mut self, line: u32) -> Result<Vec<f64>, QasmError> {
        self.pos += 1; // (
        let mut params = Vec::new();
        if matches!(self.peek(), Some(Tok::RParen)) {
            self.pos += 1;
            return Ok(params);
        }
        loop {
            let expr = self.collect_expr_until(line, &[Tok::Comma, Tok::RParen])?;
            params.push(eval_expr(&expr, line)?);
            match self.peek() {
                Some(Tok::Comma) => {
                    self.pos += 1;
                }
                Some(Tok::RParen) => {
                    self.pos += 1;
                    break;
                }
                Some(k) => {
                    return Err(QasmError::Parse {
                        line,
                        message: format!("expected `,` or `)` in gate parameters, found `{k:?}`"),
                    });
                }
                None => {
                    return Err(QasmError::Parse {
                        line,
                        message: "unterminated gate parameter list".to_string(),
                    });
                }
            }
        }
        Ok(params)
    }

    /// Collect the token slice of one parameter expression up to (not
    /// consuming) any token in `stops`. Used for parameter evaluation.
    fn collect_expr_until(&mut self, line: u32, stops: &[Tok]) -> Result<Vec<Tok>, QasmError> {
        let mut out = Vec::new();
        let mut depth = 0i32;
        while let Some(tok) = self.tokens.get(self.pos) {
            if depth == 0 && stops.iter().any(|s| s == &tok.kind) {
                break;
            }
            match &tok.kind {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    if depth == 0 {
                        return Err(QasmError::Parse {
                            line,
                            message: "unbalanced `)` in gate parameter".to_string(),
                        });
                    }
                    depth -= 1;
                }
                Tok::Semicolon | Tok::LBracket | Tok::RBracket | Tok::LBrace | Tok::RBrace => {
                    return Err(QasmError::Parse {
                        line: tok.line,
                        message: format!("unexpected `{}` in gate parameter", tok_name(&tok.kind)),
                    });
                }
                _ => {}
            }
            out.push(tok.kind.clone());
            self.pos += 1;
        }
        if depth != 0 {
            return Err(QasmError::Parse {
                line,
                message: "unbalanced `(` in gate parameter".to_string(),
            });
        }
        Ok(out)
    }

    /// `<reg> [ <idx> ] (, <reg> [ <idx> ])*`
    fn parse_operands(&mut self, line: u32) -> Result<Vec<(String, u32)>, QasmError> {
        let mut ops = Vec::new();
        loop {
            let reg = self.eat_ident(line, "qubit register")?;
            self.expect(&Tok::LBracket, line, "`[` for qubit index")?;
            let idx = self.parse_u32(line, "qubit index")?;
            self.expect(&Tok::RBracket, line, "`]`")?;
            // validate against declared registers now (line-tagged error).
            let size = match self.reg_sizes.get(&reg) {
                Some(&s) => s,
                None => {
                    return Err(QasmError::UnknownRegister { line, reg });
                }
            };
            if idx >= size {
                return Err(QasmError::QubitOutOfRange {
                    line,
                    reg,
                    index: idx,
                    size,
                });
            }
            ops.push((reg, idx));
            match self.peek() {
                Some(Tok::Comma) => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        Ok(ops)
    }

    /// Parse a non-negative integer literal token (qubit index / register size).
    fn parse_u32(&mut self, line: u32, what: &str) -> Result<u32, QasmError> {
        match self.tokens.get(self.pos).map(|t| t.kind.clone()) {
            Some(Tok::Num(s)) => {
                self.pos += 1;
                s.parse::<u32>().map_err(|e| QasmError::Parse {
                    line,
                    message: format!("invalid {what} `{s}`: {e}"),
                })
            }
            Some(k) => Err(QasmError::Parse {
                line,
                message: format!("expected {what}, found `{k:?}`"),
            }),
            None => Err(QasmError::Parse {
                line,
                message: format!("expected {what}, found end of input"),
            }),
        }
    }
}

fn tok_name(t: &Tok) -> &'static str {
    match t {
        Tok::Ident(_) => "identifier",
        Tok::Num(_) => "number",
        Tok::Str(_) => "string",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::Comma => ",",
        Tok::Semicolon => ";",
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::Equals => "=",
        Tok::Colon => ":",
        Tok::Arrow => "->",
    }
}
// ───────────────────── parameter expression evaluator ────────────
//
// Tiny recursive-descent evaluator for the QASM parameter subset:
// `pi`, numeric literals, `+ - * /`, unary minus, parentheses. Enough for
// `rz(pi/2)`, `rx(3*pi)`, `u3(0.1, -pi/2, 0)`; not a general CAS.

struct ExprCursor<'a> {
    toks: &'a [Tok],
    pos: usize,
}

fn eval_expr(toks: &[Tok], line: u32) -> Result<f64, QasmError> {
    let mut c = ExprCursor { toks, pos: 0 };
    let v = expr_add(&mut c, line)?;
    if c.pos != c.toks.len() {
        return Err(QasmError::Parse {
            line,
            message: format!(
                "trailing tokens in parameter expression: {:?}",
                &c.toks[c.pos..]
            ),
        });
    }
    Ok(v)
}

fn expr_add(c: &mut ExprCursor, line: u32) -> Result<f64, QasmError> {
    let mut v = expr_mul(c, line)?;
    while let Some(t) = c.toks.get(c.pos) {
        match t {
            Tok::Plus => {
                c.pos += 1;
                v += expr_mul(c, line)?;
            }
            Tok::Minus => {
                c.pos += 1;
                v -= expr_mul(c, line)?;
            }
            _ => break,
        }
    }
    Ok(v)
}

fn expr_mul(c: &mut ExprCursor, line: u32) -> Result<f64, QasmError> {
    let mut v = expr_unary(c, line)?;
    while let Some(t) = c.toks.get(c.pos) {
        match t {
            Tok::Star => {
                c.pos += 1;
                v *= expr_unary(c, line)?;
            }
            Tok::Slash => {
                c.pos += 1;
                let d = expr_unary(c, line)?;
                if d == 0.0 {
                    return Err(QasmError::Parse {
                        line,
                        message: "division by zero in parameter expression".to_string(),
                    });
                }
                v /= d;
            }
            _ => break,
        }
    }
    Ok(v)
}

fn expr_unary(c: &mut ExprCursor, line: u32) -> Result<f64, QasmError> {
    match c.toks.get(c.pos) {
        Some(Tok::Minus) => {
            c.pos += 1;
            Ok(-expr_unary(c, line)?)
        }
        Some(Tok::Plus) => {
            c.pos += 1;
            expr_unary(c, line)
        }
        _ => expr_atom(c, line),
    }
}

fn expr_atom(c: &mut ExprCursor, line: u32) -> Result<f64, QasmError> {
    match c.toks.get(c.pos) {
        Some(Tok::Num(s)) => {
            c.pos += 1;
            s.parse::<f64>().map_err(|e| QasmError::Parse {
                line,
                message: format!("invalid numeric parameter `{s}`: {e}"),
            })
        }
        Some(Tok::Ident(name)) if name.eq_ignore_ascii_case("pi") => {
            c.pos += 1;
            Ok(std::f64::consts::PI)
        }
        Some(Tok::LParen) => {
            c.pos += 1;
            let v = expr_add(c, line)?;
            match c.toks.get(c.pos) {
                Some(Tok::RParen) => {
                    c.pos += 1;
                    Ok(v)
                }
                Some(k) => Err(QasmError::Parse {
                    line,
                    message: format!("expected `)` in parameter, found `{k:?}`"),
                }),
                None => Err(QasmError::Parse {
                    line,
                    message: "unbalanced `(` in parameter".to_string(),
                }),
            }
        }
        Some(t) => Err(QasmError::Parse {
            line,
            message: format!("invalid parameter token `{}`", tok_name(t)),
        }),
        None => Err(QasmError::Parse {
            line,
            message: "unexpected end of parameter expression".to_string(),
        }),
    }
}

// ──────────── QasmProgram → InteractionGraph + local gates ─────────

/// Lower a parsed [`QasmProgram`] to the NA-pipeline entry contract: an
/// [`InteractionGraph`] (one interaction per ≥2-qubit gate, complete-subgraph
/// pairs for arity > 2) plus captured 1-qubit [`LocalGateExtract`]s anchored
/// to the most recent ≥2-qubit interaction on their qubit.
///
/// Mirrors `quon_na::extract::extract_interaction_graph_and_local_gates`:
/// ≥2-qubit gates form [`SegmentKind::DependencyDag`] segments whose ASAP
/// `dag_layer` is computed by [`schedule_dependency_segment`] (so a
/// barrier-free chain of disjoint matchings layers exactly like the `.qn`
/// path); 1-qubit gates are preserved end-to-end (#298) and spliced in by
/// `quon_na::pipeline::interleave_local_gates`.
pub fn build_interaction_graph(
    program: &QasmProgram,
) -> Result<(InteractionGraph, Vec<LocalGateExtract>), QasmError> {
    // Global qubit id space across all declared qregs, in declaration order.
    let mut offset: HashMap<String, u32> = HashMap::new();
    let mut total: u32 = 0;
    for reg in &program.qregs {
        offset.insert(reg.name.clone(), total);
        total = total.saturating_add(reg.size);
    }
    let vertices: Vec<LogicalQubitId> = (0..total).map(LogicalQubitId).collect();

    let mut interactions: Vec<Interaction> = Vec::new();
    let mut segments: Vec<InteractionSegment> = Vec::new();
    let mut local_gates: Vec<LocalGateExtract> = Vec::new();
    let mut next_id = 0u32;
    // Per-qubit most-recent ≥2-qubit interaction — the anchor a same-segment
    // 1-qubit gate attaches to (#298). Deliberately not reset across segments.
    let mut last_interaction_by_qubit: HashMap<LogicalQubitId, InteractionId> = HashMap::new();

    let mut seg_starts = program.segment_starts.iter();
    let mut next_barrier = seg_starts.next().copied();
    let mut current_segment: Vec<Interaction> = Vec::new();
    let mut current_ids: Vec<InteractionId> = Vec::new();

    let flush = |segment: &mut Vec<Interaction>,
                 ids: &mut Vec<InteractionId>,
                 interactions: &mut Vec<Interaction>,
                 segments: &mut Vec<InteractionSegment>| {
        if ids.is_empty() {
            segment.clear();
            return;
        }
        schedule_dependency_segment(segment);
        interactions.append(segment);
        segments.push(InteractionSegment {
            kind: SegmentKind::DependencyDag,
            interactions: std::mem::take(ids),
        });
    };

    for (gi, gate) in program.gates.iter().enumerate() {
        // A barrier starts a new segment at this gate index.
        if let Some(start) = next_barrier
            && gi == start
        {
            flush(
                &mut current_segment,
                &mut current_ids,
                &mut interactions,
                &mut segments,
            );
            next_barrier = seg_starts.next().copied();
        }

        let qubits: Vec<LogicalQubitId> = gate
            .operands
            .iter()
            .map(|(reg, idx)| {
                let off = *offset.get(reg).expect("declared register");
                LogicalQubitId(off + idx)
            })
            .collect();

        if qubits.len() == 1 {
            let qubit = qubits[0];
            local_gates.push(LocalGateExtract {
                qubit,
                gate_name: gate.name.clone(),
                angle: gate.params.first().copied(),
                after: last_interaction_by_qubit.get(&qubit).copied(),
            });
            continue;
        }

        if qubits.is_empty() {
            return Err(QasmError::EmptyOperands {
                line: gate.line,
                gate: gate.name.clone(),
                got: 0,
            });
        }

        // Canonicalize: sorted, deduped (matches quon_na::extract; the
        // interaction graph is undirected — operand order is irrelevant to
        // the NA entangling layer, which models every 2q gate as one
        // symmetric Entangle2).
        let mut sorted = qubits.clone();
        sorted.sort();
        sorted.dedup();
        let id = InteractionId(next_id);
        next_id += 1;
        current_ids.push(id);
        for &q in &sorted {
            last_interaction_by_qubit.insert(q, id);
        }
        current_segment.push(Interaction {
            id,
            qubits: sorted,
            gate_name: gate.name.clone(),
            dag_layer: 0,
            on_critical_path: false,
        });
    }
    flush(
        &mut current_segment,
        &mut current_ids,
        &mut interactions,
        &mut segments,
    );

    let graph =
        InteractionGraph::from_interactions(vertices, interactions, segments, DEFAULT_GAMMA)?;
    Ok((graph, local_gates))
}

/// Parse OpenQASM source and lower it to the NA entry contract in one call.
pub fn parse_to_graph(src: &str) -> Result<(InteractionGraph, Vec<LocalGateExtract>), QasmError> {
    let program = parse(src)?;
    build_interaction_graph(&program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> QasmProgram {
        parse(src).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
    }

    #[test]
    fn parses_qasm2_header_qreg_and_cx() {
        let p = parse_ok("OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[3];\ncx q[0],q[1];\n");
        assert_eq!(
            p.qregs,
            [QasmQreg {
                name: "q".into(),
                size: 3
            }]
        );
        assert_eq!(p.gates.len(), 1);
        assert_eq!(p.gates[0].name, "cx");
        assert_eq!(
            p.gates[0].operands,
            [("q".to_string(), 0), ("q".to_string(), 1)]
        );
        assert!(p.segment_starts.is_empty());
    }

    #[test]
    fn parses_qasm3_qubit_decl_and_params() {
        let p = parse_ok("OPENQASM 3.0;\nqubit[4] q;\nrz(pi/2) q[0];\ncz q[0],q[1];\n");
        assert_eq!(
            p.qregs,
            [QasmQreg {
                name: "q".into(),
                size: 4
            }]
        );
        assert_eq!(p.gates.len(), 2);
        assert!((p.gates[0].params[0] - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert_eq!(p.gates[1].name, "cz");
    }

    #[test]
    fn builds_chain_graph_with_four_layers() {
        // ising_n42 shape: even, odd, even, odd matchings of a 42-chain.
        let mut src = String::from("OPENQASM 2.0;\nqreg q[42];\n");
        for _ in 0..2 {
            for k in 0..21 {
                src.push_str(&format!("cz q[{}],q[{}];\n", 2 * k, 2 * k + 1));
            }
            for k in 0..20 {
                src.push_str(&format!("cz q[{}],q[{}];\n", 2 * k + 1, 2 * k + 2));
            }
        }
        let (graph, local) = parse_to_graph(&src).unwrap();
        assert!(local.is_empty());
        assert_eq!(graph.vertices.len(), 42);
        assert_eq!(graph.interactions.len(), 82);
        let layers: std::collections::BTreeSet<u32> =
            graph.interactions.iter().map(|i| i.dag_layer).collect();
        assert_eq!(
            layers.len(),
            4,
            "ASAP layering must yield 4 dag layers: {layers:?}"
        );
    }

    #[test]
    fn out_of_range_qubit_is_actionable() {
        let err = parse("qreg q[2];\ncx q[0],q[5];\n").unwrap_err();
        match err {
            QasmError::QubitOutOfRange { index, size, .. } => {
                assert_eq!(index, 5);
                assert_eq!(size, 2);
            }
            other => panic!("expected QubitOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn unknown_register_is_actionable() {
        let err = parse("qreg q[2];\ncx q[0],r[0];\n").unwrap_err();
        assert!(matches!(err, QasmError::UnknownRegister { ref reg, .. } if reg == "r"));
    }

    #[test]
    fn gate_definition_is_unsupported_and_actionable() {
        let err = parse("qreg q[2];\ngate mygate a { x a; }\n").unwrap_err();
        assert!(matches!(err, QasmError::Unsupported { .. }));
    }

    #[test]
    fn classical_control_flow_is_unsupported() {
        let err = parse("qreg q[2];\nfor i in [0:1] { h q[i]; }\n").unwrap_err();
        assert!(
            matches!(err, QasmError::Unsupported { ref construct, .. } if construct.starts_with("for"))
        );
    }

    #[test]
    fn no_qreg_before_gate_errors() {
        let err = parse("h q[0];\n").unwrap_err();
        assert!(matches!(err, QasmError::NoQreg { .. }));
    }

    #[test]
    fn one_qubit_gates_are_preserved_as_local_gates() {
        let (graph, local) =
            parse_to_graph("qreg q[2];\nh q[0];\ncz q[0],q[1];\nrz(0.5) q[1];\n").unwrap();
        assert_eq!(graph.interactions.len(), 1);
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].gate_name, "h");
        assert!(local[0].after.is_none());
        assert_eq!(local[1].gate_name, "rz");
        assert!((local[1].angle.unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(local[1].after, Some(InteractionId(0)));
    }

    #[test]
    fn barrier_splits_segments() {
        let p =
            parse_ok("qreg q[4];\ncz q[0],q[1];\nbarrier q[0],q[1],q[2],q[3];\ncz q[2],q[3];\n");
        assert_eq!(p.segment_starts, [1]);
        let (graph, _) = parse_to_graph(
            "qreg q[4];\ncz q[0],q[1];\nbarrier q[0],q[1],q[2],q[3];\ncz q[2],q[3];\n",
        )
        .unwrap();
        assert_eq!(graph.segments.len(), 2);
    }

    #[test]
    fn comments_are_stripped() {
        let p = parse_ok(
            "// line comment\nOPENQASM 2.0; /* block\ncomment */ qreg q[2];\ncx q[0],q[1]; // trailing\n",
        );
        assert_eq!(p.gates.len(), 1);
    }

    #[test]
    fn multiple_qregs_get_dense_ids() {
        let (graph, _) = parse_to_graph("qreg a[2];\nqreg b[2];\ncz a[0],b[1];\n").unwrap();
        assert_eq!(graph.vertices.len(), 4);
        assert_eq!(graph.interactions.len(), 1);
        assert_eq!(
            graph.interactions[0].qubits,
            [LogicalQubitId(0), LogicalQubitId(3)]
        );
    }
}
