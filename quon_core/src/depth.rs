//! `DepthExpr` — the symbolic gate-depth bound of a circuit.
//!
//! A circuit's depth index `d` (the third parameter of `Circuit<n, m, d, C>`)
//! is a symbolic arithmetic expression over static `Nat` literals and runtime
//! `Int` variables. The frontend carries it on `Circuit` types; downstream it is
//! serialized as an S-expression string into the `depth` op attribute of
//! `quantum.circ` ops (ADR-0002) and reconstructed by the passes that combine or
//! check depth bounds.
//!
//! Grammar (S-expressions):
//!
//! ```text
//!   depth   := nat | var | '(' op depth depth ')'
//!   op      := '+' | '*' | 'max' | '-' | '/' | '^'
//!   nat     := unsigned integer literal
//!   var     := identifier (first char alphabetic or '_')
//! ```
//!
//! Composition rules: sequential composition (`|>`) adds depths, parallel
//! composition (`par`) takes their `max`, and `controlled` adds one.

use std::fmt;

use thiserror::Error;

/// A symbolic depth-bound expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepthExpr {
    /// A static natural-number literal.
    Nat(u64),
    /// A runtime integer variable, referenced by name.
    Var(String),
    /// Sequential composition: `lhs + rhs`.
    Add(Box<DepthExpr>, Box<DepthExpr>),
    /// Repetition / scaling: `lhs * rhs`.
    Mul(Box<DepthExpr>, Box<DepthExpr>),
    /// Parallel composition: `max(lhs, rhs)`.
    Max(Box<DepthExpr>, Box<DepthExpr>),
    /// Natural subtraction: `lhs - rhs` (saturating at 0 when constant).
    Sub(Box<DepthExpr>, Box<DepthExpr>),
    /// Natural division: `lhs / rhs` (only when `rhs` is a non-zero constant).
    Div(Box<DepthExpr>, Box<DepthExpr>),
    /// Exponentiation: `lhs ^ rhs` (only when `rhs` is a small constant).
    Exp(Box<DepthExpr>, Box<DepthExpr>),
    /// A depth hole (`_`) — accepts any depth during annotation checking.
    Hole,
}

/// An error parsing a [`DepthExpr`] S-expression.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DepthParseError {
    /// The input was empty or ended early.
    #[error("unexpected end of depth expression")]
    UnexpectedEnd,
    /// A token did not fit the grammar.
    #[error("unexpected token {0:?} in depth expression")]
    UnexpectedToken(String),
    /// An operator was not one of `+`, `*`, `max`.
    #[error("unknown depth operator {0:?}")]
    UnknownOperator(String),
    /// Extra tokens trailed a complete expression.
    #[error("trailing tokens after depth expression")]
    TrailingTokens,
}

impl DepthExpr {
    /// The zero-depth expression (identity for `+` and `max`).
    pub fn zero() -> Self {
        DepthExpr::Nat(0)
    }

    /// Natural subtraction: `self - other`.
    pub fn minus(self, other: DepthExpr) -> Self {
        DepthExpr::Sub(Box::new(self), Box::new(other))
    }

    /// Natural division: `self / other`.
    pub fn quot(self, other: DepthExpr) -> Self {
        DepthExpr::Div(Box::new(self), Box::new(other))
    }

    /// Exponentiation: `self ^ other`.
    pub fn power(self, other: DepthExpr) -> Self {
        DepthExpr::Exp(Box::new(self), Box::new(other))
    }

    /// Sequential composition (`|>`): `self + other`. Depths add.
    pub fn seq(self, other: DepthExpr) -> Self {
        DepthExpr::Add(Box::new(self), Box::new(other))
    }

    /// Parallel composition (`par`): `max(self, other)`. Depth is the max.
    pub fn par(self, other: DepthExpr) -> Self {
        DepthExpr::Max(Box::new(self), Box::new(other))
    }

    /// `k`-fold repetition: `k * d`.
    pub fn repeat(k: DepthExpr, d: DepthExpr) -> Self {
        DepthExpr::Mul(Box::new(k), Box::new(d))
    }

    /// Control on a qubit: `self + 1`.
    pub fn controlled(self) -> Self {
        self.seq(DepthExpr::Nat(1))
    }

    /// Serializes to the canonical S-expression form.
    pub fn to_sexpr(&self) -> String {
        let mut out = String::new();
        self.write_sexpr(&mut out);
        out
    }

    fn write_sexpr(&self, out: &mut String) {
        match self {
            DepthExpr::Nat(n) => out.push_str(&n.to_string()),
            DepthExpr::Var(name) => out.push_str(name),
            DepthExpr::Add(l, r) => Self::write_binary(out, "+", l, r),
            DepthExpr::Mul(l, r) => Self::write_binary(out, "*", l, r),
            DepthExpr::Max(l, r) => Self::write_binary(out, "max", l, r),
            DepthExpr::Sub(l, r) => Self::write_binary(out, "-", l, r),
            DepthExpr::Div(l, r) => Self::write_binary(out, "/", l, r),
            DepthExpr::Exp(l, r) => Self::write_binary(out, "^", l, r),
            DepthExpr::Hole => out.push('_'),
        }
    }

    fn write_binary(out: &mut String, op: &str, l: &DepthExpr, r: &DepthExpr) {
        out.push('(');
        out.push_str(op);
        out.push(' ');
        l.write_sexpr(out);
        out.push(' ');
        r.write_sexpr(out);
        out.push(')');
    }

    /// A canonical form for structural comparison of depth bounds.
    ///
    /// Folds literals and canonicalises the three associative–commutative operators
    /// (`+`, `*`, `max`) by flattening nested applications, combining constants, dropping
    /// identities (`x+0`, `x*1`, `max(x,0)`), the absorbing `x*0 = 0`, and deduplicating
    /// `max` operands (`max(x,x) = x`), then sorting the operands into a deterministic order.
    ///
    /// Two depth expressions equal under those laws (for every assignment of the variables
    /// over the naturals) normalise to the *same* tree, so `a.normalize() == b.normalize()`
    /// is a **sound but incomplete** equality test: it decides the cases the type checker
    /// meets at composition boundaries (constant depths, `k*d`, `max(d₁,d₂)`, `d+1`) without
    /// an SMT solver. Distributivity and the harder residual obligations are left to the Z3
    /// bridge (issue #13).
    pub fn normalize(&self) -> DepthExpr {
        match self {
            DepthExpr::Nat(_) | DepthExpr::Var(_) | DepthExpr::Hole => self.clone(),
            DepthExpr::Add(..) => norm_ac(self, AcOp::Add),
            DepthExpr::Mul(..) => norm_ac(self, AcOp::Mul),
            DepthExpr::Max(..) => norm_ac(self, AcOp::Max),
            DepthExpr::Sub(l, r) => {
                DepthExpr::Sub(Box::new(l.normalize()), Box::new(r.normalize()))
            }
            DepthExpr::Div(l, r) => {
                DepthExpr::Div(Box::new(l.normalize()), Box::new(r.normalize()))
            }
            DepthExpr::Exp(l, r) => {
                DepthExpr::Exp(Box::new(l.normalize()), Box::new(r.normalize()))
            }
        }
    }

    /// Whether two depth bounds are provably equal by [`DepthExpr::normalize`]'s algebra.
    pub fn equiv(&self, other: &DepthExpr) -> bool {
        self.normalize() == other.normalize()
    }

    /// Evaluate to a concrete `u64` when the expression is fully constant (no variables),
    /// e.g. to recover a register's literal size. Returns `None` if any `Var` remains.
    pub fn as_const(&self) -> Option<u64> {
        match self {
            DepthExpr::Nat(n) => Some(*n),
            DepthExpr::Var(_) => None,
            DepthExpr::Add(l, r) => Some(l.as_const()?.saturating_add(r.as_const()?)),
            DepthExpr::Mul(l, r) => Some(l.as_const()?.saturating_mul(r.as_const()?)),
            DepthExpr::Max(l, r) => Some(l.as_const()?.max(r.as_const()?)),
            DepthExpr::Sub(l, r) => Some(l.as_const()?.saturating_sub(r.as_const()?)),
            DepthExpr::Div(l, r) => {
                let d = r.as_const()?;
                if d == 0 {
                    return None;
                }
                Some(l.as_const()? / d)
            }
            DepthExpr::Exp(l, r) => {
                let exp = u32::try_from(r.as_const()?).ok()?;
                l.as_const()?.checked_pow(exp)
            }
            DepthExpr::Hole => None,
        }
    }

    /// Whether this depth is a hole wildcard.
    pub fn is_hole(&self) -> bool {
        matches!(self, DepthExpr::Hole)
    }

    /// Parses a [`DepthExpr`] from its S-expression form.
    pub fn parse(source: &str) -> Result<Self, DepthParseError> {
        let tokens = tokenize(source);
        let mut cursor = 0;
        let expr = parse_expr(&tokens, &mut cursor)?;
        if cursor != tokens.len() {
            return Err(DepthParseError::TrailingTokens);
        }
        Ok(expr)
    }
}

impl fmt::Display for DepthExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_sexpr())
    }
}

/// One of the three associative–commutative depth operators, with the algebraic data
/// [`norm_ac`] needs: how to fold two literals and what the operator's identity is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AcOp {
    Add,
    Mul,
    Max,
}

impl AcOp {
    /// The identity literal: `0` for `+`/`max`, `1` for `*`.
    fn identity(self) -> u64 {
        match self {
            AcOp::Add | AcOp::Max => 0,
            AcOp::Mul => 1,
        }
    }

    /// Fold two literal operands.
    fn fold(self, a: u64, b: u64) -> u64 {
        match self {
            AcOp::Add => a.saturating_add(b),
            AcOp::Mul => a.saturating_mul(b),
            AcOp::Max => a.max(b),
        }
    }

    /// Whether `e` is an application of *this* operator (so flattening recurses into it).
    fn matches(self, e: &DepthExpr) -> bool {
        matches!(
            (self, e),
            (AcOp::Add, DepthExpr::Add(..))
                | (AcOp::Mul, DepthExpr::Mul(..))
                | (AcOp::Max, DepthExpr::Max(..))
        )
    }

    /// Rebuild a binary application of this operator.
    fn build(self, l: DepthExpr, r: DepthExpr) -> DepthExpr {
        match self {
            AcOp::Add => DepthExpr::Add(Box::new(l), Box::new(r)),
            AcOp::Mul => DepthExpr::Mul(Box::new(l), Box::new(r)),
            AcOp::Max => DepthExpr::Max(Box::new(l), Box::new(r)),
        }
    }
}

/// Normalise an application of one AC operator: flatten same-operator nesting, fold the
/// literal operands into a single constant, normalise the remaining sub-terms, then apply
/// the operator-specific identity/absorbing/dedup laws and rebuild in sorted order.
fn norm_ac(e: &DepthExpr, op: AcOp) -> DepthExpr {
    let mut terms: Vec<DepthExpr> = Vec::new();
    let mut konst = op.identity();
    collect(e, op, &mut terms, &mut konst);

    // `x * 0 = 0` short-circuits everything.
    if op == AcOp::Mul && konst == 0 {
        return DepthExpr::Nat(0);
    }

    // `max` is idempotent: dedupe its non-constant operands.
    if op == AcOp::Max {
        terms.sort_by_key(DepthExpr::to_sexpr);
        terms.dedup();
    }

    // Re-introduce the folded constant only when it is not the operator's identity, or when
    // it is the sole operand (so an all-literal expression still yields its value).
    let mut operands = terms;
    if konst != op.identity() || operands.is_empty() {
        operands.push(DepthExpr::Nat(konst));
    }
    operands.sort_by_key(DepthExpr::to_sexpr);

    // Rebuild left-associated in sorted order. An empty operand list cannot arise here (the
    // constant is pushed when nothing else remains), but the operator's identity is the
    // correct value for the empty application regardless, so no panic is needed.
    operands
        .into_iter()
        .reduce(|acc, t| op.build(acc, t))
        .unwrap_or(DepthExpr::Nat(op.identity()))
}

/// Walk `e`, flattening nested applications of `op`: literal leaves fold into `konst`,
/// every other sub-term is normalised and pushed onto `terms`.
fn collect(e: &DepthExpr, op: AcOp, terms: &mut Vec<DepthExpr>, konst: &mut u64) {
    if op.matches(e) {
        let (l, r) = match e {
            DepthExpr::Add(l, r) | DepthExpr::Mul(l, r) | DepthExpr::Max(l, r) => (l, r),
            _ => unreachable!("matches() guaranteed an application of op"),
        };
        collect(l, op, terms, konst);
        collect(r, op, terms, konst);
    } else if let DepthExpr::Nat(n) = e {
        *konst = op.fold(*konst, *n);
    } else {
        // A non-matching sub-term may itself *normalise* to a literal (e.g. `x * 0 = 0`);
        // fold those into the constant too, so a hidden absorbing zero or identity does not
        // survive as a stray operand and break idempotence.
        match e.normalize() {
            DepthExpr::Nat(n) => *konst = op.fold(*konst, n),
            other => terms.push(other),
        }
    }
}

fn tokenize(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut atom = String::new();
    for ch in source.chars() {
        match ch {
            '(' | ')' => {
                if !atom.is_empty() {
                    tokens.push(std::mem::take(&mut atom));
                }
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() => {
                if !atom.is_empty() {
                    tokens.push(std::mem::take(&mut atom));
                }
            }
            c => atom.push(c),
        }
    }
    if !atom.is_empty() {
        tokens.push(atom);
    }
    tokens
}

fn parse_expr(tokens: &[String], cursor: &mut usize) -> Result<DepthExpr, DepthParseError> {
    let token = tokens.get(*cursor).ok_or(DepthParseError::UnexpectedEnd)?;
    match token.as_str() {
        "(" => {
            *cursor += 1;
            let op = tokens
                .get(*cursor)
                .ok_or(DepthParseError::UnexpectedEnd)?
                .clone();
            *cursor += 1;
            let lhs = parse_expr(tokens, cursor)?;
            let rhs = parse_expr(tokens, cursor)?;
            let close = tokens.get(*cursor).ok_or(DepthParseError::UnexpectedEnd)?;
            if close != ")" {
                return Err(DepthParseError::UnexpectedToken(close.clone()));
            }
            *cursor += 1;
            match op.as_str() {
                "+" => Ok(lhs.seq(rhs)),
                "max" => Ok(lhs.par(rhs)),
                "*" => Ok(DepthExpr::repeat(lhs, rhs)),
                "-" => Ok(lhs.minus(rhs)),
                "/" => Ok(lhs.quot(rhs)),
                "^" => Ok(lhs.power(rhs)),
                other => Err(DepthParseError::UnknownOperator(other.to_string())),
            }
        }
        ")" => Err(DepthParseError::UnexpectedToken(token.clone())),
        atom => {
            *cursor += 1;
            Ok(parse_atom(atom))
        }
    }
}

fn parse_atom(atom: &str) -> DepthExpr {
    if atom == "_" {
        return DepthExpr::Hole;
    }
    match atom.parse::<u64>() {
        Ok(n) => DepthExpr::Nat(n),
        Err(_) => DepthExpr::Var(atom.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_sexpr() {
        let expr = DepthExpr::Nat(2)
            .seq(DepthExpr::Var("n".into()))
            .par(DepthExpr::Nat(1).controlled());
        let text = expr.to_sexpr();
        assert_eq!(DepthExpr::parse(&text), Ok(expr));
    }

    #[test]
    fn parses_literal_and_var() {
        assert_eq!(DepthExpr::parse("3"), Ok(DepthExpr::Nat(3)));
        assert_eq!(DepthExpr::parse("n"), Ok(DepthExpr::Var("n".into())));
    }

    #[test]
    fn rejects_unknown_operator() {
        assert_eq!(
            DepthExpr::parse("(min 1 2)"),
            Err(DepthParseError::UnknownOperator("min".into()))
        );
    }

    #[test]
    fn rejects_trailing_tokens() {
        assert_eq!(
            DepthExpr::parse("1 2"),
            Err(DepthParseError::TrailingTokens)
        );
    }

    #[test]
    fn canonical_forms() {
        assert_eq!(DepthExpr::zero().to_sexpr(), "0");
        assert_eq!(
            DepthExpr::Nat(1).seq(DepthExpr::Nat(2)).to_sexpr(),
            "(+ 1 2)"
        );
        assert_eq!(
            DepthExpr::Var("d".into()).controlled().to_sexpr(),
            "(+ d 1)"
        );
    }

    #[test]
    fn mul_round_trips() {
        let expr = DepthExpr::repeat(DepthExpr::Var("n".into()), DepthExpr::Nat(4));
        assert_eq!(expr.to_sexpr(), "(* n 4)");
        assert_eq!(DepthExpr::parse("(* n 4)"), Ok(expr));
    }

    #[test]
    fn parsing_is_whitespace_insensitive() {
        let expected = DepthExpr::Nat(1).seq(DepthExpr::Nat(2));
        assert_eq!(DepthExpr::parse("(+  1\t2)"), Ok(expected.clone()));
        assert_eq!(DepthExpr::parse("  (+ 1 2)  "), Ok(expected));
    }

    #[test]
    fn parses_deeply_nested() {
        let expr = DepthExpr::Nat(1)
            .seq(DepthExpr::Nat(2))
            .par(DepthExpr::Var("k".into()).controlled());
        assert_eq!(DepthExpr::parse(&expr.to_sexpr()), Ok(expr));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(DepthExpr::parse(""), Err(DepthParseError::UnexpectedEnd));
        assert_eq!(DepthExpr::parse("("), Err(DepthParseError::UnexpectedEnd));
        assert_eq!(
            DepthExpr::parse(")"),
            Err(DepthParseError::UnexpectedToken(")".into()))
        );
        assert_eq!(
            DepthExpr::parse("(+ 1)"),
            Err(DepthParseError::UnexpectedToken(")".into()))
        );
        assert!(DepthExpr::parse("(+ 1 2").is_err());
    }

    // ── Normalisation ──────────────────────────────────────────────────────────

    fn nat(n: u64) -> DepthExpr {
        DepthExpr::Nat(n)
    }
    fn var(s: &str) -> DepthExpr {
        DepthExpr::Var(s.into())
    }

    #[test]
    fn normalises_constant_folding() {
        assert_eq!(nat(1).seq(nat(1)).normalize(), nat(2));
        assert_eq!(DepthExpr::repeat(nat(2), nat(3)).normalize(), nat(6));
        assert_eq!(nat(3).par(nat(7)).normalize(), nat(7));
    }

    #[test]
    fn normalises_identities_and_absorbing() {
        assert_eq!(var("d").seq(nat(0)).normalize(), var("d")); // d + 0 = d
        assert_eq!(DepthExpr::repeat(nat(1), var("d")).normalize(), var("d")); // 1 * d = d
        assert_eq!(DepthExpr::repeat(nat(0), var("d")).normalize(), nat(0)); // 0 * d = 0
        assert_eq!(var("d").par(nat(0)).normalize(), var("d")); // max(d, 0) = d
        assert_eq!(var("x").par(var("x")).normalize(), var("x")); // max(x, x) = x
    }

    #[test]
    fn normalisation_is_commutative_and_associative() {
        // n_steps * n  vs  n * n_steps
        assert!(
            DepthExpr::repeat(var("n_steps"), var("n"))
                .equiv(&DepthExpr::repeat(var("n"), var("n_steps")))
        );
        // (a + b) + c  vs  a + (b + c)
        let left = var("a").seq(var("b")).seq(var("c"));
        let right = var("a").seq(var("b").seq(var("c")));
        assert!(left.equiv(&right));
    }

    #[test]
    fn controlled_depth_is_canonical() {
        // controlled adds one: `d + 1`, regardless of how the `1` is grouped.
        assert!(var("d").controlled().equiv(&nat(1).seq(var("d"))));
    }

    #[test]
    fn normalisation_is_idempotent() {
        let exprs = [
            nat(1).seq(nat(1)).seq(var("n")),
            DepthExpr::repeat(var("k"), var("d").controlled()),
            var("a").par(var("b")).par(var("a")),
        ];
        for e in exprs {
            let once = e.normalize();
            assert_eq!(once.normalize(), once, "not idempotent: {e}");
        }
    }

    #[test]
    fn distinct_bounds_stay_distinct() {
        assert!(!var("a").seq(var("b")).equiv(&var("a"))); // a + b ≠ a
        assert!(!var("x").seq(var("x")).equiv(&var("x"))); // x + x ≠ x (Add not idempotent)
        assert!(!nat(2).equiv(&nat(3)));
    }
}
