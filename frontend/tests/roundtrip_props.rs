// Generative roundtrip property test — the centerpiece of the harness.
//
// This drives the *same* generator the cargo-fuzz targets use (frontend/fuzz/src/gen.rs),
// fed random bytes by proptest. Sharing one generator keeps in-tree (CI) coverage in lockstep
// with continuous fuzzing — no drift between the two. Any printer/parser asymmetry in
// precedence, associativity, or desugaring shows up here as a shrinkable counterexample.

mod support;

#[path = "../fuzz/src/gen.rs"]
mod generator;

use arbitrary::Unstructured;
use frontend::pretty::pretty;
use proptest::prelude::*;
use support::strip_decls;

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// pretty → parse_program → strip equals the generated AST.
    #[test]
    fn ast_roundtrips(bytes in prop::collection::vec(any::<u8>(), 64..8192)) {
        let mut u = Unstructured::new(&bytes);
        if let Ok(decls) = generator::arb_program(&mut u) {
            let printed = pretty(&decls);
            let mut reparsed = frontend::parse_program(&printed)
                .map_err(|e| TestCaseError::fail(format!("re-parse failed: {e:?}\n---\n{printed}")))?;
            strip_decls(&mut reparsed);
            prop_assert_eq!(reparsed, decls, "roundtrip mismatch\n--- printed ---\n{}", printed);
        }
    }

    /// Printing is idempotent: printing a reparsed tree yields byte-identical source.
    #[test]
    fn print_is_idempotent(bytes in prop::collection::vec(any::<u8>(), 64..8192)) {
        let mut u = Unstructured::new(&bytes);
        if let Ok(decls) = generator::arb_program(&mut u) {
            let printed = pretty(&decls);
            let reparsed = frontend::parse_program(&printed)
                .map_err(|e| TestCaseError::fail(format!("re-parse failed: {e:?}")))?;
            let printed2 = pretty(&reparsed);
            prop_assert_eq!(printed, printed2);
        }
    }
}
