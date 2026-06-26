// Property tests for the classical type checker (issue #9).
//
// Two complementary generators:
//
//   1. **Panic-freedom** — the checker is *total*: on any program the parser accepts
//      (including the full quantum language the in-tree AST generator emits), checking
//      must return `Ok`/`Err`, never panic, loop, or overflow. This reuses the very same
//      `frontend/fuzz/src/gen.rs` generator the cargo-fuzz targets drive, so in-tree CI
//      coverage and continuous fuzzing stay in lockstep (cf. `roundtrip_props.rs`).
//
//   2. **Well-typedness** — a *type-directed* generator builds a classical expression of a
//      chosen type, then asserts the checker accepts it AND synthesizes exactly that type.
//      This is the soundness direction: everything we deem well-typed by construction is
//      accepted, exercising application, let, if, lambdas, and the polymorphic prelude.

#[path = "../fuzz/src/gen.rs"]
mod generator;

use arbitrary::{Arbitrary, Result, Unstructured};
use frontend::typecheck::TypeChecker;
use frontend::types::Ty;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 600, ..ProptestConfig::default() })]

    /// Type-checking any parser-accepted program terminates without panicking.
    #[test]
    fn check_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..8192)) {
        let mut u = Unstructured::new(&bytes);
        if let Ok(decls) = generator::arb_program(&mut u) {
            // Both the whole-program driver and the single-body synth hook must be total.
            let _ = TypeChecker::new().check_decls(&decls);
            let _ = TypeChecker::new().synth_last_body(&decls);
        }
    }

    /// A type-directed expression of type `τ` is accepted and synthesizes back to `τ`.
    #[test]
    fn well_typed_expressions_synthesize_their_type(
        bytes in prop::collection::vec(any::<u8>(), 16..4096)
    ) {
        let mut u = Unstructured::new(&bytes);
        if let Ok((ty, expr)) = gen_typed_expr(&mut u, 4, &mut Vec::new()) {
            let src = format!("fn probe(): {ty} = {expr}");
            let decls = frontend::parse_program(&src)
                .map_err(|e| TestCaseError::fail(format!("generated source did not parse: {e:?}\n{src}")))?;

            // Accepted by the whole-program checker.
            if let Err(errs) = TypeChecker::new().check_decls(&decls) {
                return Err(TestCaseError::fail(format!("rejected well-typed source: {errs:?}\n{src}")));
            }
            // And its synthesized type is exactly the generated one.
            let synthed = TypeChecker::new().synth_last_body(&decls)
                .map_err(|e| TestCaseError::fail(format!("synth failed: {e}\n{src}")))?;
            prop_assert_eq!(synthed, ty, "source: {}", src);
        }
    }
}

// ── Type-directed generator ─────────────────────────────────────────────────────
//
// Drives a target type and an expression of that type from the same `Unstructured` byte
// stream. Tracks an environment of in-scope `(name, Ty)` bindings so it can emit variable
// references, `let`s, and lambdas passed to `map`/`fold`.

/// A small classical type whose values are easy to annotate and compare.
fn gen_simple_ty(u: &mut Unstructured, depth: u32) -> Result<Ty> {
    let leaf = depth == 0 || u.is_empty();
    Ok(match u.int_in_range(0..=if leaf { 3 } else { 5 })? {
        0 => Ty::Int,
        1 => Ty::Float,
        2 => Ty::Bool,
        3 => Ty::Unit,
        4 => Ty::list(gen_simple_ty(u, depth.saturating_sub(1))?),
        _ => {
            let n = u.int_in_range(2..=3)?;
            Ty::Tuple(
                (0..n)
                    .map(|_| gen_simple_ty(u, depth.saturating_sub(1)))
                    .collect::<Result<_>>()?,
            )
        }
    })
}

type Scope = Vec<(String, Ty)>;

/// Generate `(τ, source)` where `source` is a closed-over-`scope` expression of type `τ`.
fn gen_typed_expr(u: &mut Unstructured, depth: u32, scope: &mut Scope) -> Result<(Ty, String)> {
    let ty = gen_simple_ty(u, 2)?;
    let src = gen_of_type(u, &ty, depth, scope)?;
    Ok((ty, src))
}

/// Generate an expression of exactly `ty`.
fn gen_of_type(u: &mut Unstructured, ty: &Ty, depth: u32, scope: &mut Scope) -> Result<String> {
    // Sometimes reference an in-scope variable of the right type (exercises Γ lookup).
    if !u.is_empty() && bool::arbitrary(u)? {
        let candidates: Vec<String> = scope
            .iter()
            .filter(|(_, t)| t == ty)
            .map(|(n, _)| n.clone())
            .collect();
        if !candidates.is_empty() {
            return Ok(u.choose(&candidates)?.clone());
        }
    }

    if depth == 0 || u.is_empty() {
        return gen_leaf(u, ty);
    }

    // A `let` wrapper is available for any target type.
    if u.int_in_range(0..=4)? == 0 {
        return gen_let(u, ty, depth, scope);
    }

    match ty {
        Ty::Int => match u.int_in_range(0..=3)? {
            0 => Ok(format!(
                "({} + {})",
                gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?,
                gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?
            )),
            1 => Ok(format!(
                "({} * {})",
                gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?,
                gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?
            )),
            2 => Ok(format!(
                "round({})",
                gen_of_type(u, &Ty::Float, depth.saturating_sub(1), scope)?
            )),
            _ => gen_if(u, ty, depth, scope),
        },
        Ty::Float => match u.int_in_range(0..=3)? {
            0 => Ok(format!(
                "({} + {})",
                gen_of_type(u, &Ty::Float, depth.saturating_sub(1), scope)?,
                gen_of_type(u, &Ty::Float, depth.saturating_sub(1), scope)?
            )),
            1 => Ok(format!(
                "float({})",
                gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?
            )),
            2 => Ok(format!(
                "sqrt({})",
                gen_of_type(u, &Ty::Float, depth.saturating_sub(1), scope)?
            )),
            _ => gen_if(u, ty, depth, scope),
        },
        Ty::Bool => gen_if(u, ty, depth, scope),
        Ty::Unit => Ok("()".to_string()),
        Ty::Tuple(ts) => {
            let parts = ts
                .iter()
                .map(|t| gen_of_type(u, t, depth.saturating_sub(1), scope))
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("({})", parts.join(", ")))
        }
        Ty::List(elem) => gen_list(u, elem, depth, scope),
        _ => gen_leaf(u, ty),
    }
}

/// Base-case literal (or canonical value) for `ty`.
fn gen_leaf(u: &mut Unstructured, ty: &Ty) -> Result<String> {
    Ok(match ty {
        Ty::Int => format!("{}", u.int_in_range(0..=9)?),
        // Quarter steps always lex as floats and round-trip exactly.
        Ty::Float => format!("{:.2}", u8::arbitrary(u)? as f64 / 4.0),
        Ty::Bool => if bool::arbitrary(u)? { "true" } else { "false" }.to_string(),
        Ty::Unit => "()".to_string(),
        Ty::Tuple(ts) => {
            let parts = ts
                .iter()
                .map(|t| gen_leaf(u, t))
                .collect::<Result<Vec<_>>>()?;
            format!("({})", parts.join(", "))
        }
        Ty::List(elem) => format!("[{}]", gen_leaf(u, elem)?),
        _ => "()".to_string(),
    })
}

fn gen_if(u: &mut Unstructured, ty: &Ty, depth: u32, scope: &mut Scope) -> Result<String> {
    // Parenthesized: `if` is not an atom, so it needs grouping in argument positions.
    Ok(format!(
        "(if {} then {} else {})",
        gen_of_type(u, &Ty::Bool, depth.saturating_sub(1), scope)?,
        gen_of_type(u, ty, depth.saturating_sub(1), scope)?,
        gen_of_type(u, ty, depth.saturating_sub(1), scope)?
    ))
}

fn gen_let(u: &mut Unstructured, ty: &Ty, depth: u32, scope: &mut Scope) -> Result<String> {
    let bound_ty = gen_simple_ty(u, 1)?;
    let rhs = gen_of_type(u, &bound_ty, depth.saturating_sub(1), scope)?;
    let name = format!("v{}", scope.len());
    scope.push((name.clone(), bound_ty));
    let body = gen_of_type(u, ty, depth.saturating_sub(1), scope)?;
    scope.pop();
    Ok(format!("(let {name} = {rhs} in {body})"))
}

/// Generate a `List<elem>` expression, sometimes via the polymorphic prelude (`map`,
/// `take`, and `range` for `List<Int>`) with checker-inferred lambdas.
fn gen_list(u: &mut Unstructured, elem: &Ty, depth: u32, scope: &mut Scope) -> Result<String> {
    // Base case: a singleton literal list, terminating the recursion.
    if depth == 0 || u.is_empty() {
        return Ok(format!("[{}]", gen_leaf(u, elem)?));
    }
    let int_list = *elem == Ty::Int;
    match u.int_in_range(0..=if int_list { 3 } else { 2 })? {
        // Literal list of one or two elements.
        0 => {
            let a = gen_of_type(u, elem, depth.saturating_sub(1), scope)?;
            let b = gen_of_type(u, elem, depth.saturating_sub(1), scope)?;
            Ok(format!("[{a}, {b}]"))
        }
        // take(n, xs)
        1 => Ok(format!(
            "take({}, {})",
            gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?,
            gen_list(u, elem, depth.saturating_sub(1), scope)?
        )),
        // map(fn(a) -> body, src) where `a : src_elem` is in scope for `body`.
        2 => {
            let src_elem = gen_simple_ty(u, 1)?;
            let src_list = gen_list(u, &src_elem, depth.saturating_sub(1), scope)?;
            let name = format!("p{}", scope.len());
            scope.push((name.clone(), src_elem));
            let body = gen_of_type(u, elem, depth.saturating_sub(1), scope)?;
            scope.pop();
            Ok(format!("map(fn({name}) -> {body}, {src_list})"))
        }
        // range(n) : List<Int>
        _ => Ok(format!(
            "range({})",
            gen_of_type(u, &Ty::Int, depth.saturating_sub(1), scope)?
        )),
    }
}
