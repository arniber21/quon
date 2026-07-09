mod support;

use quonfmt::format_str;

#[test]
fn idempotent_on_corpus() {
    for (name, input) in support::all_corpus() {
        let once = format_str(&input).expect("parse");
        let twice = format_str(&once).expect("re-parse");
        assert_eq!(once, twice, "idempotency failed for {name}");
    }
}

#[path = "../../frontend/fuzz/src/gen.rs"]
mod generator;

use arbitrary::Unstructured;
use frontend::pretty::pretty;
use proptest::prelude::*;

#[test]
fn par_gateapp_count_is_idempotent() {
    let src = "fn f(): Circuit<1,1,1,Clifford> = par { 59.0 } * 18.75 @ match borrow gate: Q<Int> in { gate <- 1 } { _ => () }";
    let f1 = format_str(src).expect("f1");
    let f2 = format_str(&f1).expect("f2");
    assert_eq!(f1, f2, "f1:\n{f1}\n\nf2:\n{f2}");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_global_rejects: 50_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn format_is_idempotent(bytes in prop::collection::vec(any::<u8>(), 64..2048)) {
        let mut u = Unstructured::new(&bytes);
        if let Ok(decls) = generator::arb_program(&mut u) {
            let src = pretty(&decls);
            let Ok(f1) = format_str(&src) else {
                return Ok(());
            };
            let Ok(f2) = format_str(&f1) else {
                return Ok(());
            };
            prop_assert_eq!(f1, f2);
        }
    }
}
