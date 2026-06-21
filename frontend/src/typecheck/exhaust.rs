//! Pattern-matrix analysis: exhaustiveness and reachability.
//!
//! This is Maranget's *usefulness* algorithm ("Warnings for pattern matching", JFP 2007),
//! restricted to the pattern grammar Quon's classical fragment can produce — wildcards,
//! variables (which match like wildcards), `Bool`/`Int` literals, and tuples.
//!
//! Two queries are built on the one `useful` core:
//!
//! * **Exhaustiveness** — is the all-wildcard row useful against the arms? If so the
//!   `match` misses a case, and we reconstruct a concrete [`Witness`] for the diagnostic.
//! * **Reachability** — is arm `i`'s pattern useful against arms `0..i`? If not, the arm
//!   is dead code (an earlier arm subsumes it).
//!
//! The scrutinee's [`Ty`] supplies each column's constructor *signature*, which is what
//! lets us decide whether a finite type (`Bool`, a tuple) is fully covered versus an
//! open one (`Int`, `Float`) that always needs a catch-all.

use crate::ast::{LitPat, Pat};
use crate::types::Ty;
use std::fmt;

/// A head constructor in the restricted pattern space.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ctor {
    Bool(bool),
    Int(i64),
    /// A tuple of the given arity (its sole constructor).
    Tuple(usize),
}

impl Ctor {
    /// Number of sub-patterns this constructor exposes.
    fn arity(&self) -> usize {
        match self {
            Ctor::Bool(_) | Ctor::Int(_) => 0,
            Ctor::Tuple(n) => *n,
        }
    }

    /// The field types this constructor exposes, given the column type it lives in.
    fn field_types(&self, col_ty: &Ty) -> Vec<Ty> {
        match self {
            Ctor::Tuple(_) => match col_ty {
                Ty::Tuple(ts) => ts.clone(),
                // Off-type tuple pattern (rejected earlier by `check_pat`); be total anyway.
                _ => vec![Ty::Var("_".into()); self.arity()],
            },
            _ => vec![],
        }
    }
}

/// A reconstructed counter-example showing a value the `match` fails to cover.
#[derive(Debug, Clone)]
pub enum Witness {
    Wild,
    Bool(bool),
    Int(i64),
    Tuple(Vec<Witness>),
}

impl fmt::Display for Witness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Witness::Wild => f.write_str("_"),
            Witness::Bool(b) => write!(f, "{b}"),
            Witness::Int(n) => write!(f, "{n}"),
            Witness::Tuple(ws) => {
                f.write_str("(")?;
                for (i, w) in ws.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{w}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// The head constructor of `pat`, or `None` for wildcards/variables.
fn head_ctor(pat: &Pat) -> Option<Ctor> {
    match pat {
        Pat::Wildcard | Pat::Var(_) => None,
        Pat::Tuple(ps) => Some(Ctor::Tuple(ps.len())),
        Pat::Lit(LitPat::Bool(b)) => Some(Ctor::Bool(*b)),
        Pat::Lit(LitPat::Int(n)) => Some(Ctor::Int(*n)),
    }
}

/// Whether the constructors present in a column form a *complete* signature for `ty`:
/// every value of the type is headed by one of them. Open types (`Int`, `Float`, …)
/// are never complete via enumeration — they need a wildcard to be exhaustive.
fn is_complete(ty: &Ty, present: &[Ctor]) -> bool {
    match ty {
        Ty::Bool => present.contains(&Ctor::Bool(true)) && present.contains(&Ctor::Bool(false)),
        Ty::Tuple(ts) => present.contains(&Ctor::Tuple(ts.len())),
        _ => false,
    }
}

/// Specialize the matrix `rows` by constructor `ctor`: keep only rows whose first
/// pattern matches `ctor`, expanding that pattern's sub-patterns (or wildcards for a
/// wildcard/variable head) into the leading columns.
fn specialize<'a>(rows: &[Vec<&'a Pat>], ctor: &Ctor) -> Vec<Vec<&'a Pat>> {
    let arity = ctor.arity();
    let mut out = Vec::new();
    for row in rows {
        let (first, rest) = row.split_first().expect("specialize on width-0 row");
        match first {
            Pat::Wildcard | Pat::Var(_) => {
                let mut new_row = vec![&WILDCARD; arity];
                new_row.extend_from_slice(rest);
                out.push(new_row);
            }
            Pat::Tuple(ps) => {
                if *ctor == Ctor::Tuple(ps.len()) {
                    let mut new_row: Vec<&Pat> = ps.iter().map(|sp| &sp.0).collect();
                    new_row.extend_from_slice(rest);
                    out.push(new_row);
                }
            }
            Pat::Lit(LitPat::Bool(b)) => {
                if *ctor == Ctor::Bool(*b) {
                    out.push(rest.to_vec());
                }
            }
            Pat::Lit(LitPat::Int(n)) => {
                if *ctor == Ctor::Int(*n) {
                    out.push(rest.to_vec());
                }
            }
        }
    }
    out
}

/// The default matrix: rows whose first pattern is a wildcard/variable, with that
/// column dropped. Used when the column's constructors are an incomplete signature.
fn default_matrix<'a>(rows: &[Vec<&'a Pat>]) -> Vec<Vec<&'a Pat>> {
    rows.iter()
        .filter_map(|row| {
            let (first, rest) = row.split_first().expect("default on width-0 row");
            matches!(first, Pat::Wildcard | Pat::Var(_)).then(|| rest.to_vec())
        })
        .collect()
}

/// A shared wildcard pattern to splice into specialized rows.
static WILDCARD: Pat = Pat::Wildcard;

/// Constructors appearing as a column-0 head, de-duplicated, order-preserving.
fn present_ctors(rows: &[Vec<&Pat>]) -> Vec<Ctor> {
    let mut ctors = Vec::new();
    for row in rows {
        if let Some(c) = head_ctor(row[0])
            && !ctors.contains(&c)
        {
            ctors.push(c);
        }
    }
    ctors
}

/// Picks a concrete witness value for an *incomplete* column: a constructor the column
/// does not cover. Falls back to `_` when nothing constrains the choice.
fn missing_witness(ty: &Ty, present: &[Ctor]) -> Witness {
    // A column no pattern constrains is best shown as `_` (any value is uncovered).
    if present.is_empty() {
        return Witness::Wild;
    }
    match ty {
        Ty::Bool => {
            if !present.contains(&Ctor::Bool(false)) {
                Witness::Bool(false)
            } else {
                Witness::Bool(true)
            }
        }
        Ty::Int => {
            // Smallest non-negative literal not already matched.
            let mut n = 0;
            while present.contains(&Ctor::Int(n)) {
                n += 1;
            }
            Witness::Int(n)
        }
        _ => Witness::Wild,
    }
}

/// Core: is the all-wildcard row useful against `rows` (width = `types.len()`)?
/// Returns a witness row when it is — i.e. a concrete value no row matches.
fn missing_row(rows: &[Vec<&Pat>], types: &[Ty]) -> Option<Vec<Witness>> {
    // Base case: zero columns. The empty value is covered iff some row exists.
    let Some((col_ty, rest_ty)) = types.split_first() else {
        return rows.is_empty().then(Vec::new);
    };

    let present = present_ctors(rows);
    if is_complete(col_ty, &present) {
        // Every head is covered; the gap (if any) must be deeper. Try each constructor.
        for ctor in &present {
            let spec = specialize(rows, ctor);
            let mut field_ty = ctor.field_types(col_ty);
            field_ty.extend_from_slice(rest_ty);
            if let Some(mut witness) = missing_row(&spec, &field_ty) {
                let fields = witness.drain(..ctor.arity()).collect::<Vec<_>>();
                let head = match ctor {
                    Ctor::Bool(b) => Witness::Bool(*b),
                    Ctor::Int(n) => Witness::Int(*n),
                    Ctor::Tuple(_) => Witness::Tuple(fields),
                };
                let mut out = vec![head];
                out.extend(witness);
                return Some(out);
            }
        }
        None
    } else {
        // Incomplete: a missing head constructor witnesses the gap, if the tail is open.
        let default = default_matrix(rows);
        missing_row(&default, rest_ty).map(|rest| {
            let mut out = vec![missing_witness(col_ty, &present)];
            out.extend(rest);
            out
        })
    }
}

/// Is pattern row `q` useful against the matrix `rows` (i.e. does `q` match some value
/// no earlier row does)? Drives reachability/redundancy checks.
fn useful(rows: &[Vec<&Pat>], q: &[&Pat], types: &[Ty]) -> bool {
    let Some((col_ty, rest_ty)) = types.split_first() else {
        return rows.is_empty();
    };
    let (q_head, q_rest) = q.split_first().expect("useful: query narrower than types");

    match head_ctor(q_head) {
        Some(ctor) => {
            let spec_rows = specialize(rows, &ctor);
            let mut spec_q: Vec<&Pat> = match q_head {
                Pat::Tuple(ps) => ps.iter().map(|sp| &sp.0).collect(),
                _ => vec![],
            };
            spec_q.extend_from_slice(q_rest);
            let mut field_ty = ctor.field_types(col_ty);
            field_ty.extend_from_slice(rest_ty);
            useful(&spec_rows, &spec_q, &field_ty)
        }
        None => {
            let present = present_ctors(rows);
            if is_complete(col_ty, &present) {
                present.iter().any(|ctor| {
                    let spec_rows = specialize(rows, ctor);
                    let mut spec_q = vec![&WILDCARD; ctor.arity()];
                    spec_q.extend_from_slice(q_rest);
                    let mut field_ty = ctor.field_types(col_ty);
                    field_ty.extend_from_slice(rest_ty);
                    useful(&spec_rows, &spec_q, &field_ty)
                })
            } else {
                useful(&default_matrix(rows), q_rest, rest_ty)
            }
        }
    }
}

/// Result of analysing a `match`'s arms against the scrutinee type.
pub struct MatchAnalysis {
    /// A witness pattern the arms fail to cover, or `None` if the match is exhaustive.
    pub missing: Option<Witness>,
    /// Indices of arms that can never be reached (subsumed by earlier arms).
    pub unreachable: Vec<usize>,
}

/// Analyse `arms` (top-level patterns, in source order) against scrutinee type `scrut_ty`.
pub fn analyze(arms: &[&Pat], scrut_ty: &Ty) -> MatchAnalysis {
    let types = [scrut_ty.clone()];

    // Reachability: arm i is dead if it isn't useful against arms 0..i.
    let mut unreachable = Vec::new();
    let mut seen: Vec<Vec<&Pat>> = Vec::new();
    for (i, pat) in arms.iter().enumerate() {
        let q = [*pat];
        if !useful(&seen, &q, &types) {
            unreachable.push(i);
        }
        seen.push(vec![*pat]);
    }

    // Exhaustiveness: is the wildcard row still useful against every arm?
    let all: Vec<Vec<&Pat>> = arms.iter().map(|p| vec![*p]).collect();
    let missing = missing_row(&all, &types).map(|mut row| row.remove(0));

    MatchAnalysis {
        missing,
        unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{SimpleSpan, Sp};

    fn sp<T>(t: T) -> Sp<T> {
        (t, SimpleSpan::from(0..0))
    }

    fn pbool(b: bool) -> Pat {
        Pat::Lit(LitPat::Bool(b))
    }
    fn pint(n: i64) -> Pat {
        Pat::Lit(LitPat::Int(n))
    }

    #[test]
    fn bool_needs_both_branches() {
        let arms = [pbool(true)];
        let refs: Vec<&Pat> = arms.iter().collect();
        let a = analyze(&refs, &Ty::Bool);
        assert!(a.missing.is_some());
        assert_eq!(a.missing.unwrap().to_string(), "false");
    }

    #[test]
    fn bool_both_branches_is_exhaustive() {
        let arms = [pbool(true), pbool(false)];
        let refs: Vec<&Pat> = arms.iter().collect();
        assert!(analyze(&refs, &Ty::Bool).missing.is_none());
    }

    #[test]
    fn wildcard_is_exhaustive() {
        let arms = [Pat::Wildcard];
        let refs: Vec<&Pat> = arms.iter().collect();
        assert!(analyze(&refs, &Ty::Int).missing.is_none());
    }

    #[test]
    fn int_literals_without_wildcard_are_non_exhaustive() {
        let arms = [pint(0), pint(1)];
        let refs: Vec<&Pat> = arms.iter().collect();
        let a = analyze(&refs, &Ty::Int);
        assert_eq!(a.missing.unwrap().to_string(), "2");
    }

    #[test]
    fn tuple_of_bools_full_grid_is_exhaustive() {
        let arms = [
            Pat::Tuple(vec![sp(pbool(true)), sp(pbool(true))]),
            Pat::Tuple(vec![sp(pbool(true)), sp(pbool(false))]),
            Pat::Tuple(vec![sp(pbool(false)), sp(pbool(true))]),
            Pat::Tuple(vec![sp(pbool(false)), sp(pbool(false))]),
        ];
        let refs: Vec<&Pat> = arms.iter().collect();
        let ty = Ty::Tuple(vec![Ty::Bool, Ty::Bool]);
        assert!(analyze(&refs, &ty).missing.is_none());
    }

    #[test]
    fn tuple_of_bools_missing_one_corner() {
        let arms = [
            Pat::Tuple(vec![sp(pbool(true)), sp(pbool(true))]),
            Pat::Tuple(vec![sp(pbool(true)), sp(pbool(false))]),
            Pat::Tuple(vec![sp(pbool(false)), sp(pbool(true))]),
        ];
        let refs: Vec<&Pat> = arms.iter().collect();
        let ty = Ty::Tuple(vec![Ty::Bool, Ty::Bool]);
        let a = analyze(&refs, &ty);
        assert_eq!(a.missing.unwrap().to_string(), "(false, false)");
    }

    #[test]
    fn tuple_with_wildcard_column_is_exhaustive() {
        // (true, _) | (false, _) covers all (Bool, Bool).
        let arms = [
            Pat::Tuple(vec![sp(pbool(true)), sp(Pat::Wildcard)]),
            Pat::Tuple(vec![sp(pbool(false)), sp(Pat::Wildcard)]),
        ];
        let refs: Vec<&Pat> = arms.iter().collect();
        let ty = Ty::Tuple(vec![Ty::Bool, Ty::Bool]);
        assert!(analyze(&refs, &ty).missing.is_none());
    }

    #[test]
    fn redundant_arm_after_wildcard_is_unreachable() {
        let arms = [Pat::Wildcard, pbool(true)];
        let refs: Vec<&Pat> = arms.iter().collect();
        let a = analyze(&refs, &Ty::Bool);
        assert_eq!(a.unreachable, vec![1]);
    }

    #[test]
    fn duplicate_literal_arm_is_unreachable() {
        let arms = [pbool(true), pbool(false), pbool(true)];
        let refs: Vec<&Pat> = arms.iter().collect();
        let a = analyze(&refs, &Ty::Bool);
        assert_eq!(a.unreachable, vec![2]);
    }
}
