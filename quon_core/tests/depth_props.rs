//! Property-based ("fuzzed") tests for `DepthExpr` S-expression ser/de.
//!
//! This is the test surface for the depth codec; it lives with the type in
//! `quon_core` rather than in any crate that consumes it.

use proptest::prelude::*;
use quon_core::DepthExpr;

/// A recursive generator for arbitrary depth expressions. Var names are
/// lowercase identifiers, which never collide with the `+`/`*`/`max` operators
/// (those only appear in operator position) so every generated tree round-trips.
fn depth_strategy() -> impl Strategy<Value = DepthExpr> {
    let leaf = prop_oneof![
        any::<u64>().prop_map(DepthExpr::Nat),
        "[a-z][a-z0-9_]*".prop_map(DepthExpr::Var),
    ];
    leaf.prop_recursive(6, 64, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.seq(b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| DepthExpr::repeat(a, b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.par(b)),
        ]
    })
}

/// Like [`depth_strategy`] but with literals in a realistic range for qubit counts and gate
/// depths. Normalisation folds literals with *saturating* arithmetic, so idempotence is only
/// meaningful below the saturation ceiling — circuit indices never approach `u64::MAX`.
fn small_depth_strategy() -> impl Strategy<Value = DepthExpr> {
    let leaf = prop_oneof![
        (0u64..256).prop_map(DepthExpr::Nat),
        "[a-z][a-z0-9_]*".prop_map(DepthExpr::Var),
    ];
    leaf.prop_recursive(6, 64, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.seq(b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| DepthExpr::repeat(a, b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.par(b)),
        ]
    })
}

proptest! {
    /// `parse ∘ to_sexpr` is the identity on every depth expression.
    #[test]
    fn sexpr_round_trips(expr in depth_strategy()) {
        prop_assert_eq!(DepthExpr::parse(&expr.to_sexpr()), Ok(expr));
    }

    /// `parse` never panics on arbitrary text.
    #[test]
    fn parse_never_panics(source in ".*") {
        let _ = DepthExpr::parse(&source);
    }

    /// `parse` never panics on strings drawn from the token alphabet (more
    /// likely to reach deep into the recursive descent).
    #[test]
    fn parse_token_soup_never_panics(source in "[-()+*maxn0-9 \t]{0,64}") {
        let _ = DepthExpr::parse(&source);
    }

    /// Whenever a raw string parses, reprinting and reparsing is stable.
    #[test]
    fn parse_is_idempotent(source in "[()+*a-z0-9 ]{0,48}") {
        if let Ok(expr) = DepthExpr::parse(&source) {
            prop_assert_eq!(DepthExpr::parse(&expr.to_sexpr()), Ok(expr));
        }
    }

    /// `normalize` is idempotent: a canonical form is a fixed point.
    #[test]
    fn normalize_is_idempotent(expr in small_depth_strategy()) {
        let once = expr.normalize();
        prop_assert_eq!(once.normalize(), once);
    }

    /// Normalisation respects the algebra: re-associating an operator does not change the
    /// canonical form (so `equiv` is reflexive across associativity).
    #[test]
    fn normalize_is_associativity_stable(
        a in small_depth_strategy(),
        b in small_depth_strategy(),
        c in small_depth_strategy(),
    ) {
        prop_assert!(a.clone().seq(b.clone()).seq(c.clone()).equiv(&a.clone().seq(b.clone().seq(c.clone()))));
        prop_assert!(a.clone().par(b.clone()).par(c.clone()).equiv(&a.clone().par(b.clone().par(c.clone()))));
        prop_assert!(DepthExpr::repeat(DepthExpr::repeat(a.clone(), b.clone()), c.clone())
            .equiv(&DepthExpr::repeat(a, DepthExpr::repeat(b, c))));
    }

    /// Normalisation respects commutativity of every AC operator.
    #[test]
    fn normalize_is_commutative(a in small_depth_strategy(), b in small_depth_strategy()) {
        prop_assert!(a.clone().seq(b.clone()).equiv(&b.clone().seq(a.clone())));
        prop_assert!(a.clone().par(b.clone()).equiv(&b.clone().par(a.clone())));
        prop_assert!(DepthExpr::repeat(a.clone(), b.clone()).equiv(&DepthExpr::repeat(b, a)));
    }

    /// A normalised expression still round-trips through the S-expression codec.
    #[test]
    fn normalized_form_round_trips(expr in small_depth_strategy()) {
        let norm = expr.normalize();
        prop_assert_eq!(DepthExpr::parse(&norm.to_sexpr()), Ok(norm));
    }
}
