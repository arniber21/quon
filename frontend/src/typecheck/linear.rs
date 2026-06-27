//! The linear context `Δ` — split-context resource tracking (issue #10, SPEC §3.4).
//!
//! Where `Γ` ([`super::Env`]) holds *unrestricted* classical values that may be used any
//! number of times, `Δ` holds **linear resources** — qubits, registers, and circuit values
//! that must be consumed *exactly once*. The two structural rules a linear discipline drops
//! are realized here as physical bookkeeping:
//!
//! * **No contraction (no-cloning).** A name is *physically removed* from the available set
//!   the moment it is used. A second use finds it gone and is rejected
//!   ([`TypeError::LinearUsedTwice`]).
//! * **No weakening (no-dropping).** A resource still available when its binding scope ends
//!   is rejected ([`TypeError::LinearUnconsumed`]); the caller drives this check via
//!   [`Delta::is_available`] over the names a scope introduced.
//!
//! A spent resource is not forgotten: it moves to a `consumed` map keyed by the span of the
//! use that took it, so the double-use diagnostic can point at *both* offending sites. That
//! is also why [`Delta::try_consume`] distinguishes "not a linear name" (caller should look
//! in `Γ`/the prelude) from "linear name, already spent".

use crate::lexer::SimpleSpan;
use crate::types::Ty;
use std::collections::{BTreeSet, HashMap};

use super::error::TypeError;

/// One available linear resource: its type and the span of the binding that introduced it.
#[derive(Debug, Clone)]
struct LinEntry {
    ty: Ty,
    intro: SimpleSpan,
}

/// The linear context. `available` is the live resources; `consumed` records, per spent
/// name, the span of the use that consumed it (for the no-cloning diagnostic).
#[derive(Debug, Clone, Default)]
pub struct Delta {
    available: HashMap<String, LinEntry>,
    consumed: HashMap<String, SimpleSpan>,
}

impl Delta {
    pub fn new() -> Self {
        Self::default()
    }

    /// Introduce a fresh linear resource `name : ty`, bound at `intro`. A binding shadowing
    /// a still-available linear name would silently drop the old one, so that is surfaced as
    /// a no-dropping error rather than a quiet overwrite.
    pub fn introduce(&mut self, name: String, ty: Ty, intro: SimpleSpan) -> Result<(), TypeError> {
        if let Some(old) = self.available.get(&name) {
            return Err(TypeError::LinearUnconsumed {
                name,
                span: old.intro,
            });
        }
        self.consumed.remove(&name);
        self.available.insert(name, LinEntry { ty, intro });
        Ok(())
    }

    /// Attempt to consume `name`.
    ///
    /// * `None` — `name` is not a linear resource at all; the caller should resolve it in
    ///   `Γ`/the prelude as usual.
    /// * `Some(Ok(ty))` — consumed; `name` is now spent and removed from the live set.
    /// * `Some(Err(_))` — `name` is a linear resource that was *already* spent (no-cloning).
    pub fn try_consume(
        &mut self,
        name: &str,
        use_span: SimpleSpan,
    ) -> Option<Result<Ty, TypeError>> {
        if let Some(entry) = self.available.remove(name) {
            self.consumed.insert(name.to_string(), use_span);
            return Some(Ok(entry.ty));
        }
        if let Some(&first) = self.consumed.get(name) {
            return Some(Err(TypeError::LinearUsedTwice {
                name: name.to_string(),
                first,
                span: use_span,
            }));
        }
        None
    }

    /// Whether `name` is a linear resource still live in this context.
    pub fn is_available(&self, name: &str) -> bool {
        self.available.contains_key(name)
    }

    /// The set of currently-live resource names — the *residual* of a scope. Branch joins
    /// compare these sets across arms; equality means every arm spent the same resources.
    pub fn residual(&self) -> BTreeSet<String> {
        self.available.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(lo: usize, hi: usize) -> SimpleSpan {
        (lo..hi).into()
    }

    #[test]
    fn introduce_then_consume_yields_the_type() {
        let mut d = Delta::new();
        d.introduce("q".into(), Ty::Qubit, sp(0, 1)).unwrap();
        assert!(d.is_available("q"));
        let got = d.try_consume("q", sp(5, 6)).expect("known").unwrap();
        assert_eq!(got, Ty::Qubit);
        assert!(!d.is_available("q"));
    }

    #[test]
    fn second_consume_is_used_twice_with_both_spans() {
        let mut d = Delta::new();
        d.introduce("q".into(), Ty::Qubit, sp(0, 1)).unwrap();
        d.try_consume("q", sp(5, 6)).unwrap().unwrap();
        let err = d
            .try_consume("q", sp(9, 10))
            .expect("still known")
            .unwrap_err();
        match err {
            TypeError::LinearUsedTwice { name, first, span } => {
                assert_eq!(name, "q");
                assert_eq!((first.start, first.end), (5, 6));
                assert_eq!((span.start, span.end), (9, 10));
            }
            other => panic!("expected LinearUsedTwice, got {other:?}"),
        }
    }

    #[test]
    fn unknown_name_is_none() {
        let mut d = Delta::new();
        assert!(d.try_consume("nope", sp(0, 1)).is_none());
    }

    #[test]
    fn residual_tracks_live_resources_only() {
        let mut d = Delta::new();
        d.introduce("a".into(), Ty::Qubit, sp(0, 1)).unwrap();
        d.introduce("b".into(), Ty::Qubit, sp(2, 3)).unwrap();
        d.try_consume("a", sp(5, 6)).unwrap().unwrap();
        assert_eq!(d.residual(), BTreeSet::from(["b".to_string()]));
    }

    #[test]
    fn shadowing_a_live_resource_is_a_drop() {
        let mut d = Delta::new();
        d.introduce("q".into(), Ty::Qubit, sp(0, 1)).unwrap();
        let err = d.introduce("q".into(), Ty::Qubit, sp(4, 5)).unwrap_err();
        assert!(matches!(err, TypeError::LinearUnconsumed { .. }));
    }

    #[test]
    fn reintroducing_a_consumed_name_is_allowed() {
        let mut d = Delta::new();
        d.introduce("q".into(), Ty::Qubit, sp(0, 1)).unwrap();
        d.try_consume("q", sp(5, 6)).unwrap().unwrap();
        // After consumption the name is free to bind again (e.g. a later let in sequence).
        d.introduce("q".into(), Ty::Qubit, sp(8, 9)).unwrap();
        assert!(d.is_available("q"));
    }
}
