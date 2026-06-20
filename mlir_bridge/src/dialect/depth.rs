//! `DepthExpr` — the symbolic gate-depth bound carried by `quantum.circ` ops.
//!
//! A circuit's depth index `d` (the third parameter of `Circuit<n, m, d, C>`)
//! is a symbolic arithmetic expression over static `Nat` literals and runtime
//! `Int` variables. Per ADR-0002 it is stored as the `depth` op attribute,
//! serialized as an S-expression string, and reconstructed into this enum by
//! the optimization passes that combine or check depth bounds.
//!
//! Grammar (S-expressions):
//!
//! ```text
//!   depth   := nat | var | '(' op depth depth ')'
//!   op      := '+' | '*' | 'max'
//!   nat     := unsigned integer literal
//!   var     := identifier (first char alphabetic or '_')
//! ```
//!
//! Composition rules (SPEC §6.2): `compose` adds depths, `tensor` takes their
//! `max`, `controlled` adds one.

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

    /// Sequential composition: `self + other`, the depth rule for `compose`.
    pub fn plus(self, other: DepthExpr) -> Self {
        DepthExpr::Add(Box::new(self), Box::new(other))
    }

    /// Parallel composition: `max(self, other)`, the depth rule for `tensor`.
    pub fn max_with(self, other: DepthExpr) -> Self {
        DepthExpr::Max(Box::new(self), Box::new(other))
    }

    /// Adds a constant, the depth rule for `controlled` (`d + 1`).
    pub fn plus_const(self, n: u64) -> Self {
        self.plus(DepthExpr::Nat(n))
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
                "+" => Ok(lhs.plus(rhs)),
                "max" => Ok(lhs.max_with(rhs)),
                "*" => Ok(DepthExpr::Mul(Box::new(lhs), Box::new(rhs))),
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
            .plus(DepthExpr::Var("n".into()))
            .max_with(DepthExpr::Nat(1).plus_const(1));
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
        assert_eq!(DepthExpr::Nat(0).to_sexpr(), "0");
        assert_eq!(
            DepthExpr::Nat(1).plus(DepthExpr::Nat(2)).to_sexpr(),
            "(+ 1 2)"
        );
        assert_eq!(
            DepthExpr::Var("d".into()).plus_const(1).to_sexpr(),
            "(+ d 1)"
        );
    }
}
