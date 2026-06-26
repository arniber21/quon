#![no_main]
// Type-checking is total: generate a syntactically valid program and run the classical
// type checker over it. Most generated programs are ill-typed (or use the quantum fragment),
// so this is not a correctness oracle — it asserts *panic-freedom*: the checker must always
// terminate with `Ok`/`Err`, never panic, overflow, or recurse without bound, on any input
// the parser accepts. Mirrors the in-tree `check_never_panics` proptest.

use arbitrary::Unstructured;
use frontend::typecheck::TypeChecker;
use frontend_fuzz::gen::arb_program;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(decls) = arb_program(&mut u) else {
        return;
    };
    // Whole-program driver and the single-body synth hook must both be total.
    let _ = TypeChecker::new().check_decls(&decls);
    let _ = TypeChecker::new().synth_last_body(&decls);
});
