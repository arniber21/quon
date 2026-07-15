//! First-order unification over [`Ty`] with mutable metavariables.
//!
//! The checker only ever needs *first-order* unification: prelude schemes introduce
//! flexible [`Ty::Meta`] variables (for the `A`, `B` in `map : (A -> B, List<A>) -> List<B>`),
//! and unification solves them against the concrete types that flow in from the call site.
//!
//! Solutions live in a single growable substitution ([`Table`]). [`Table::resolve`] walks
//! a metavariable to its current binding (one level), and [`Table::zonk`] applies the whole
//! substitution to a type, replacing every solved metavariable. Binding performs the
//! standard occurs-check so a metavariable can never be unified with a type that mentions it.

use super::error::TypeError;
use crate::lexer::SimpleSpan;
use crate::types::Ty;

/// The metavariable substitution: `subst[i]` is the solution of `?i`, or `None` if unsolved.
#[derive(Debug, Default)]
pub struct Table {
    subst: Vec<Option<Ty>>,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates a fresh, unsolved metavariable.
    pub fn fresh(&mut self) -> Ty {
        let id = self.subst.len() as u32;
        self.subst.push(None);
        Ty::Meta(id)
    }

    /// Follows a metavariable chain *one* type deep: if `ty` is a solved metavariable,
    /// returns its (recursively shallow-resolved) solution; otherwise returns `ty` as-is.
    /// The head constructor of the result is never a solved metavariable.
    pub fn resolve(&self, ty: &Ty) -> Ty {
        let mut cur = ty.clone();
        while let Ty::Meta(id) = cur {
            match self.subst.get(id as usize).and_then(|s| s.clone()) {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur
    }

    /// Fully applies the substitution to `ty`, recursively replacing every solved
    /// metavariable. Unsolved metavariables are left in place (the caller decides
    /// whether a residual `Meta` is an error or should be defaulted).
    pub fn zonk(&self, ty: &Ty) -> Ty {
        match self.resolve(ty) {
            Ty::List(t) => Ty::List(Box::new(self.zonk(&t))),
            Ty::Q(t) => Ty::Q(Box::new(self.zonk(&t))),
            Ty::Matrix(n, m, t) => Ty::Matrix(n, m, Box::new(self.zonk(&t))),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.zonk(t)).collect()),
            Ty::Fn(a, b) => Ty::Fn(Box::new(self.zonk(&a)), Box::new(self.zonk(&b))),
            Ty::Linear(a, b) => Ty::Linear(Box::new(self.zonk(&a)), Box::new(self.zonk(&b))),
            other => other,
        }
    }

    /// Unifies `a` and `b`, recording any metavariable solutions. `span` anchors the
    /// error if they are incompatible. The error carries the *zonked* types so the
    /// message reflects everything learned so far.
    pub fn unify(&mut self, a: &Ty, b: &Ty, span: SimpleSpan) -> Result<(), TypeError> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (&a, &b) {
            (Ty::Meta(i), Ty::Meta(j)) if i == j => Ok(()),
            (Ty::Meta(i), _) => self.bind(*i, &b, span),
            (_, Ty::Meta(j)) => self.bind(*j, &a, span),

            (Ty::Qubit, Ty::Qubit)
            | (Ty::Bit, Ty::Bit)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Int, Ty::Int)
            | (Ty::Float, Ty::Float)
            | (Ty::Unit, Ty::Unit) => Ok(()),
            (Ty::QReg(n), Ty::QReg(m)) if n.equiv(m) => Ok(()),
            (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),

            (Ty::List(x), Ty::List(y)) | (Ty::Q(x), Ty::Q(y)) => self.unify(x, y, span),
            (Ty::Tuple(xs), Ty::Tuple(ys)) if xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify(x, y, span)?;
                }
                Ok(())
            }
            (Ty::Fn(a1, b1), Ty::Fn(a2, b2)) | (Ty::Linear(a1, b1), Ty::Linear(a2, b2)) => {
                self.unify(a1, a2, span)?;
                self.unify(b1, b2, span)
            }
            (Ty::Matrix(n1, m1, t1), Ty::Matrix(n2, m2, t2)) if n1.equiv(n2) && m1.equiv(m2) => {
                self.unify(t1, t2, span)
            }
            (
                Ty::Circuit {
                    n: n1,
                    m: m1,
                    d: d1,
                    c: c1,
                },
                Ty::Circuit {
                    n: n2,
                    m: m2,
                    d: d2,
                    c: c2,
                },
            ) if n1.equiv(n2)
                && m1.equiv(m2)
                && (d1.is_hole() || d2.is_hole() || d1.equiv(d2))
                && c1 == c2 =>
            {
                Ok(())
            }

            (
                Ty::QecBlock {
                    family: f1,
                    distance: d1,
                },
                Ty::QecBlock {
                    family: f2,
                    distance: d2,
                },
            ) if f1 == f2 && d1.equiv(d2) => Ok(()),

            _ => Err(TypeError::Mismatch {
                expected: self.zonk(&a),
                found: self.zonk(&b),
                span,
            }),
        }
    }

    /// Solves `?id := ty`, failing the occurs-check if `ty` mentions `?id`.
    fn bind(&mut self, id: u32, ty: &Ty, span: SimpleSpan) -> Result<(), TypeError> {
        if self.occurs(id, ty) {
            return Err(TypeError::OccursCheck { span });
        }
        self.subst[id as usize] = Some(ty.clone());
        Ok(())
    }

    /// Whether metavariable `?id` appears anywhere in `ty` (after shallow resolution).
    fn occurs(&self, id: u32, ty: &Ty) -> bool {
        match self.resolve(ty) {
            Ty::Meta(j) => id == j,
            Ty::List(t) | Ty::Q(t) | Ty::Matrix(_, _, t) => self.occurs(id, &t),
            Ty::Tuple(ts) => ts.iter().any(|t| self.occurs(id, t)),
            Ty::Fn(a, b) | Ty::Linear(a, b) => self.occurs(id, &a) || self.occurs(id, &b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> SimpleSpan {
        (0..0).into()
    }

    #[test]
    fn binds_meta_to_concrete() {
        let mut t = Table::new();
        let m = t.fresh();
        t.unify(&m, &Ty::Int, sp()).unwrap();
        assert_eq!(t.zonk(&m), Ty::Int);
    }

    #[test]
    fn propagates_through_constructors() {
        let mut t = Table::new();
        let m = t.fresh();
        t.unify(&Ty::list(m.clone()), &Ty::list(Ty::Bool), sp())
            .unwrap();
        assert_eq!(t.zonk(&m), Ty::Bool);
    }

    #[test]
    fn transitive_meta_chain_resolves() {
        let mut t = Table::new();
        let a = t.fresh();
        let b = t.fresh();
        t.unify(&a, &b, sp()).unwrap();
        t.unify(&b, &Ty::Float, sp()).unwrap();
        assert_eq!(t.zonk(&a), Ty::Float);
    }

    #[test]
    fn rejects_constructor_clash() {
        let mut t = Table::new();
        assert!(matches!(
            t.unify(&Ty::Int, &Ty::Bool, sp()),
            Err(TypeError::Mismatch { .. })
        ));
    }

    #[test]
    fn occurs_check_rejects_infinite_type() {
        let mut t = Table::new();
        let m = t.fresh();
        assert!(matches!(
            t.unify(&m, &Ty::list(m.clone()), sp()),
            Err(TypeError::OccursCheck { .. })
        ));
    }

    #[test]
    fn fn_arity_and_components_unify() {
        let mut t = Table::new();
        let r = t.fresh();
        let lhs = Ty::func(Ty::Int, r.clone());
        let rhs = Ty::func(Ty::Int, Ty::Bool);
        t.unify(&lhs, &rhs, sp()).unwrap();
        assert_eq!(t.zonk(&r), Ty::Bool);
    }
}
