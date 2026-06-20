//! Property-based ("fuzzed") tests for `DepthExpr` S-expression ser/de.
//!
//! These run under `cargo test`. For continuous fuzzing, see `fuzz/` (nightly
//! `cargo fuzz`), which exercises the same invariants on raw bytes.

use mlir_bridge::dialect::depth::DepthExpr;
use proptest::prelude::*;

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
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.plus(b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| DepthExpr::Mul(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.max_with(b)),
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
}
