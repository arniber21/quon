//! Fuzz the `DepthExpr` S-expression round-trip: for any expression we can
//! build, `parse(to_sexpr(expr)) == expr`. A counterexample is a real ser/de
//! bug. Run with `cargo +nightly fuzz run fuzz_depth_roundtrip`.
#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use mlir_bridge::dialect::depth::DepthExpr;

/// A lowercase identifier — never numeric, never an operator token, so it always
/// parses back as a `Var`.
fn ident(u: &mut Unstructured) -> arbitrary::Result<String> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let len = u.int_in_range::<usize>(1..=6)?;
    let mut name = String::with_capacity(len);
    for _ in 0..len {
        let index = u.int_in_range::<usize>(0..=ALPHABET.len() - 1)?;
        name.push(ALPHABET[index] as char);
    }
    Ok(name)
}

/// Builds a depth expression from the fuzzer's bytes, bounded by `depth`.
fn generate(u: &mut Unstructured, depth: u32) -> arbitrary::Result<DepthExpr> {
    if depth == 0 || u.is_empty() {
        return Ok(if bool::arbitrary(u)? {
            DepthExpr::Nat(u64::arbitrary(u)?)
        } else {
            DepthExpr::Var(ident(u)?)
        });
    }
    Ok(match u.int_in_range::<u8>(0..=4)? {
        0 => DepthExpr::Nat(u64::arbitrary(u)?),
        1 => DepthExpr::Var(ident(u)?),
        2 => generate(u, depth - 1)?.plus(generate(u, depth - 1)?),
        3 => DepthExpr::Mul(
            Box::new(generate(u, depth - 1)?),
            Box::new(generate(u, depth - 1)?),
        ),
        _ => generate(u, depth - 1)?.max_with(generate(u, depth - 1)?),
    })
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    if let Ok(expr) = generate(&mut u, 6) {
        assert_eq!(DepthExpr::parse(&expr.to_sexpr()), Ok(expr));
    }
});
