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

proptest! {
    #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

    #[test]
    fn format_is_idempotent(bytes in prop::collection::vec(any::<u8>(), 64..4096)) {
        let mut u = Unstructured::new(&bytes);
        if let Ok(decls) = generator::arb_program(&mut u) {
            let src = pretty(&decls);
            if let Ok(f1) = format_str(&src) {
                prop_assume!(format_str(&f1).is_ok());
                let f2 = format_str(&f1).unwrap();
                prop_assert_eq!(f1, f2);
            }
        }
    }
}
