//! The classical prelude (SPEC §5.10–§5.11) as type schemes.
//!
//! These are the only source of parametric polymorphism in the classical fragment:
//! user functions are monomorphic (every parameter and return type is annotated), so
//! generalization happens *only* here. Each builtin is a [`Scheme`] — a type closed over
//! some rigid variables — and a reference to it is [`Scheme::instantiate`]d with fresh
//! metavariables so each call site solves independently.
//!
//! Multi-argument builtins are *curried*, matching the surface calling convention: the
//! parser lowers `map(f, xs)` to `App(App(map, f), xs)`, so `map`'s type is the curried
//! `(A -> B) -> List<A> -> List<B>` (the SPEC writes it tupled only for readability).

use super::unify::Table;
use crate::types::Ty;

/// A polymorphic type, closed over the rigid variables in `vars`.
#[derive(Debug, Clone)]
pub struct Scheme {
    /// Names of the rigid [`Ty::Var`]s bound by this scheme (e.g. `["A", "B"]`).
    pub vars: &'static [&'static str],
    /// The body, mentioning the bound variables as `Ty::Var`.
    pub body: Ty,
}

impl Scheme {
    /// A monomorphic scheme (no quantified variables) — used for constants like `PI`.
    fn mono(body: Ty) -> Scheme {
        Scheme { vars: &[], body }
    }

    /// Replaces every bound rigid variable with a fresh metavariable, yielding a type
    /// ready to unify against the arguments at one call site.
    pub fn instantiate(&self, table: &mut Table) -> Ty {
        let fresh: Vec<(&str, Ty)> = self.vars.iter().map(|v| (*v, table.fresh())).collect();
        subst_vars(&self.body, &fresh)
    }
}

/// Substitutes rigid `Ty::Var(name)` occurrences according to `mapping`.
fn subst_vars(ty: &Ty, mapping: &[(&str, Ty)]) -> Ty {
    match ty {
        Ty::Var(name) => mapping
            .iter()
            .find(|(v, _)| v == name)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| ty.clone()),
        Ty::List(t) => Ty::List(Box::new(subst_vars(t, mapping))),
        Ty::Q(t) => Ty::Q(Box::new(subst_vars(t, mapping))),
        Ty::Matrix(n, m, t) => Ty::Matrix(*n, *m, Box::new(subst_vars(t, mapping))),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_vars(t, mapping)).collect()),
        Ty::Fn(a, b) => Ty::Fn(
            Box::new(subst_vars(a, mapping)),
            Box::new(subst_vars(b, mapping)),
        ),
        Ty::Linear(a, b) => Ty::Linear(
            Box::new(subst_vars(a, mapping)),
            Box::new(subst_vars(b, mapping)),
        ),
        other => other.clone(),
    }
}

// Small builders to keep the table below readable.
fn v(name: &'static str) -> Ty {
    Ty::Var(name.to_string())
}
fn func(a: Ty, b: Ty) -> Ty {
    Ty::func(a, b)
}
fn list(t: Ty) -> Ty {
    Ty::list(t)
}
fn tuple(ts: Vec<Ty>) -> Ty {
    Ty::Tuple(ts)
}
/// Right-fold a curried function type: `curry([T1, T2], R) = T1 -> T2 -> R`.
fn curry(args: Vec<Ty>, ret: Ty) -> Ty {
    args.into_iter().rev().fold(ret, |acc, a| func(a, acc))
}

/// Looks up the scheme for a prelude function or constant, if `name` is one.
///
/// Covers the full classical prelude (SPEC §5.10) and physics constants (§5.11). The
/// quantum/linear prelude (allocation, measurement, gate combinators) lands with the
/// linear fragment in later issues.
pub fn lookup(name: &str) -> Option<Scheme> {
    let scheme = match name {
        // ── §5.10 Classical prelude (curried) ────────────────────────────────
        // range(n) : Int -> List<Int>
        "range" => Scheme::mono(func(Ty::Int, list(Ty::Int))),
        // map(f, xs) : (A -> B) -> List<A> -> List<B>
        "map" => Scheme {
            vars: &["A", "B"],
            body: curry(vec![func(v("A"), v("B")), list(v("A"))], list(v("B"))),
        },
        // fold(xs, z, f) : List<A> -> B -> (B -> A -> B) -> B
        "fold" => Scheme {
            vars: &["A", "B"],
            body: curry(
                vec![list(v("A")), v("B"), curry(vec![v("B"), v("A")], v("B"))],
                v("B"),
            ),
        },
        // take(n, xs) : Int -> List<A> -> List<A>
        "take" => Scheme {
            vars: &["A"],
            body: curry(vec![Ty::Int, list(v("A"))], list(v("A"))),
        },
        // zip(xs, ys) : List<A> -> List<B> -> List<(A, B)>
        "zip" => Scheme {
            vars: &["A", "B"],
            body: curry(
                vec![list(v("A")), list(v("B"))],
                list(tuple(vec![v("A"), v("B")])),
            ),
        },
        // float(n) : Int -> Float
        "float" => Scheme::mono(func(Ty::Int, Ty::Float)),
        // round(x) : Float -> Int
        "round" => Scheme::mono(func(Ty::Float, Ty::Int)),
        // sqrt(x) : Float -> Float
        "sqrt" => Scheme::mono(func(Ty::Float, Ty::Float)),
        // log2(x) : Float -> Float
        "log2" => Scheme::mono(func(Ty::Float, Ty::Float)),

        // ── §5.11 Physics constants ─────────────────────────────────────────
        "PI" | "TAU" | "E" => Scheme::mono(Ty::Float),

        _ => return None,
    };
    Some(scheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiation_uses_fresh_metas_each_time() {
        let mut t = Table::new();
        let map = lookup("map").unwrap();
        let a = map.instantiate(&mut t);
        let b = map.instantiate(&mut t);
        // Two instantiations must not share metavariables.
        assert_ne!(a, b);
    }

    #[test]
    fn monomorphic_constants_have_no_vars() {
        assert!(lookup("PI").unwrap().vars.is_empty());
        assert_eq!(lookup("range").unwrap().vars.len(), 0);
    }

    #[test]
    fn unknown_name_is_none() {
        assert!(lookup("definitely_not_a_builtin").is_none());
    }
}
