//! Internal unit tests for the classical type checker.
//!
//! These drive the real parser so the inputs are exactly what a programmer would write,
//! then assert on the *synthesized* type (via the private `synth_last_body` hook) and on
//! accept/reject behaviour. Living inside the module gives them the private surface;
//! span-level and end-to-end behaviour is covered additionally in `tests/typecheck.rs`.

use super::*;
use crate::parse_program;

/// Synthesize the body type of the last function in `src` (ignoring its return annotation).
fn synth(src: &str) -> Result<Ty, TypeError> {
    let decls = parse_program(src).expect("parse failed");
    TypeChecker::new().synth_last_body(&decls)
}

/// Synthesize, asserting success.
fn ty(src: &str) -> Ty {
    synth(src).unwrap_or_else(|e| panic!("synth failed for `{src}`: {e}"))
}

/// Whole-program check.
fn check(src: &str) -> Result<(), Vec<TypeError>> {
    let decls = parse_program(src).expect("parse failed");
    TypeChecker::new().check_decls(&decls)
}

fn accepts(src: &str) {
    if let Err(e) = check(src) {
        panic!("expected `{src}` to type-check, got {e:?}");
    }
}

fn rejects(src: &str) {
    assert!(check(src).is_err(), "expected `{src}` to be rejected");
}

/// Reject `src` and return its first type error, for asserting the *kind* of failure.
fn reject_err(src: &str) -> TypeError {
    let mut errs = check(src).expect_err("expected rejection");
    errs.swap_remove(0)
}

// ── Literal synthesis ──────────────────────────────────────────────────────────

#[test]
fn literals_synthesize_their_types() {
    assert_eq!(ty("fn f(): Int = 3"), Ty::Int);
    assert_eq!(ty("fn f(): Float = 3.5"), Ty::Float);
    assert_eq!(ty("fn f(): Bool = true"), Ty::Bool);
    assert_eq!(ty("fn f(): Bool = false"), Ty::Bool);
    assert_eq!(ty("fn f(): Unit = ()"), Ty::Unit);
}

#[test]
fn tuple_synthesizes_componentwise() {
    assert_eq!(
        ty("fn f(): Int = (1, true, 2.0)"),
        Ty::Tuple(vec![Ty::Int, Ty::Bool, Ty::Float])
    );
}

#[test]
fn nonempty_list_synthesizes_element_type() {
    assert_eq!(ty("fn f(): Int = [1, 2, 3]"), Ty::list(Ty::Int));
}

// ── Variables and scoping ──────────────────────────────────────────────────────

#[test]
fn parameter_is_in_scope_with_its_type() {
    assert_eq!(ty("fn f(x: Float): Float = x"), Ty::Float);
}

#[test]
fn let_binding_extends_scope() {
    assert_eq!(ty("fn f(): Int = let x = 7 in x"), Ty::Int);
}

#[test]
fn let_tuple_pattern_destructures() {
    assert_eq!(
        ty("fn f(p: (Int, Bool)): Bool = let (a, b) = p in b"),
        Ty::Bool
    );
}

#[test]
fn local_binding_shadows_prelude() {
    // `range` rebound to an Int shadows the prelude function.
    assert_eq!(ty("fn f(): Int = let range = 5 in range"), Ty::Int);
}

// ── Arithmetic ─────────────────────────────────────────────────────────────────

#[test]
fn integer_arithmetic_is_int() {
    assert_eq!(ty("fn f(): Int = 1 + 2 * 3"), Ty::Int);
}

#[test]
fn float_arithmetic_is_float() {
    assert_eq!(ty("fn f(): Float = 1.0 + 2.0"), Ty::Float);
}

#[test]
fn negation_preserves_numeric_type() {
    assert_eq!(ty("fn f(x: Float): Float = -x"), Ty::Float);
}

#[test]
fn mixed_numeric_arithmetic_is_rejected() {
    rejects("fn f(): Float = 1 + 2.0");
}

#[test]
fn arithmetic_on_bool_is_rejected() {
    rejects("fn f(): Int = true + 1");
}

// ── Application (→ elimination) ─────────────────────────────────────────────────

#[test]
fn application_synthesizes_codomain() {
    // `f : A -> B` applied to `e : A` synthesizes `B` (acceptance criterion).
    assert_eq!(
        ty("fn dbl(x: Int): Int = x + x\nfn g(): Int = dbl(21)"),
        Ty::Int
    );
}

#[test]
fn curried_multi_arg_application() {
    assert_eq!(
        ty("fn add(a: Int, b: Int): Int = a + b\nfn g(): Int = add(1, 2)"),
        Ty::Int
    );
}

#[test]
fn nullary_call_passes_unit() {
    assert_eq!(ty("fn k(): Bool = true\nfn g(): Bool = k()"), Ty::Bool);
}

#[test]
fn applying_a_non_function_is_rejected() {
    rejects("fn f(x: Int): Int = x(3)");
}

#[test]
fn argument_type_mismatch_is_rejected() {
    rejects("fn dbl(x: Int): Int = x + x\nfn g(): Int = dbl(true)");
}

// ── Lambdas (→ introduction), both modes ───────────────────────────────────────

#[test]
fn annotated_lambda_synthesizes_function_type() {
    assert_eq!(
        ty("fn f(): Int = fn(x: Int) -> x"),
        Ty::func(Ty::Int, Ty::Int)
    );
}

#[test]
fn unannotated_lambda_checks_against_expected() {
    accepts("fn apply(f: Int -> Int, x: Int): Int = f(x)\nfn g(): Int = apply(fn(y) -> y + 1, 2)");
}

#[test]
fn unannotated_lambda_without_context_is_rejected() {
    // Synthesis cannot infer the domain of an unannotated lambda.
    rejects("fn f(): Int = fn(x) -> x");
}

// ── if / then / else ───────────────────────────────────────────────────────────

#[test]
fn if_branches_must_agree() {
    assert_eq!(ty("fn f(b: Bool): Int = if b then 1 else 2"), Ty::Int);
}

#[test]
fn if_branch_mismatch_is_rejected() {
    rejects("fn f(b: Bool): Int = if b then 1 else true");
}

#[test]
fn if_condition_must_be_bool() {
    rejects("fn f(): Int = if 1 then 1 else 2");
}

// ── match: typing, exhaustiveness, reachability ─────────────────────────────────

#[test]
fn match_on_bool_is_exhaustive_with_both_arms() {
    accepts("fn f(b: Bool): Int = match b { true => 1, false => 0 }");
}

#[test]
fn match_on_bool_missing_arm_is_rejected() {
    rejects("fn f(b: Bool): Int = match b { true => 1 }");
}

#[test]
fn match_with_wildcard_is_exhaustive() {
    accepts("fn f(n: Int): Int = match n { 0 => 1, _ => 2 }");
}

#[test]
fn match_int_without_catchall_is_rejected() {
    rejects("fn f(n: Int): Int = match n { 0 => 1, 1 => 2 }");
}

#[test]
fn match_tuple_full_grid_is_exhaustive() {
    accepts(
        "fn f(p: (Bool, Bool)): Int = match p {\
         (true, true) => 1, (true, false) => 2,\
         (false, true) => 3, (false, false) => 4 }",
    );
}

#[test]
fn match_tuple_missing_corner_is_rejected() {
    rejects(
        "fn f(p: (Bool, Bool)): Int = match p {\
         (true, true) => 1, (true, false) => 2, (false, true) => 3 }",
    );
}

#[test]
fn match_unreachable_arm_is_rejected() {
    rejects("fn f(b: Bool): Int = match b { _ => 1, true => 2 }");
}

#[test]
fn match_arms_must_share_a_type() {
    rejects("fn f(b: Bool): Int = match b { true => 1, false => false }");
}

#[test]
fn match_binds_pattern_variables() {
    accepts("fn f(p: (Int, Int)): Int = match p { (a, b) => a + b }");
}

// ── Classical prelude against SPEC signatures ───────────────────────────────────

#[test]
fn range_is_int_to_list_int() {
    assert_eq!(ty("fn f(): List<Int> = range(10)"), Ty::list(Ty::Int));
}

#[test]
fn map_is_polymorphic() {
    assert_eq!(
        ty("fn f(xs: List<Int>): List<Int> = map(fn(x) -> x + x, xs)"),
        Ty::list(Ty::Int)
    );
}

#[test]
fn fold_threads_accumulator() {
    assert_eq!(
        ty("fn f(xs: List<Int>): Int = fold(xs, 0, fn(acc, x) -> acc + x)"),
        Ty::Int
    );
}

#[test]
fn zip_pairs_element_types() {
    assert_eq!(
        ty("fn f(xs: List<Int>, ys: List<Bool>): Int = zip(xs, ys)"),
        Ty::list(Ty::Tuple(vec![Ty::Int, Ty::Bool]))
    );
}

#[test]
fn float_round_sqrt_log2_chain() {
    accepts("fn f(n: Int): Float = sqrt(log2(float(n)))");
    assert_eq!(ty("fn f(x: Float): Int = round(x)"), Ty::Int);
}

#[test]
fn take_keeps_list_type() {
    assert_eq!(
        ty("fn f(xs: List<Bool>): List<Bool> = take(2, xs)"),
        Ty::list(Ty::Bool)
    );
}

#[test]
fn physics_constants_are_float() {
    assert_eq!(ty("fn f(): Float = PI"), Ty::Float);
    assert_eq!(ty("fn f(): Float = TAU + E"), Ty::Float);
}

#[test]
fn prelude_misuse_is_rejected() {
    rejects("fn f(): Float = sqrt(1)"); // sqrt wants Float, got Int
    rejects("fn f(): List<Int> = range(true)");
}

// ── Numeric inference: deferred constraints solved by context ────────────────────

#[test]
fn arithmetic_param_is_solved_by_the_argument_list() {
    // `p + p` does not eagerly default `p` to Int; the Float list pins it to Float.
    assert_eq!(
        ty("fn f(xs: List<Float>): List<Float> = map(fn(p) -> p + p, xs)"),
        Ty::list(Ty::Float)
    );
}

#[test]
fn deferred_numeric_var_defaults_to_int() {
    // Nothing constrains `p`, so it defaults to Int and the result is List<Int>.
    assert_eq!(
        ty("fn f(xs: List<Int>): List<Int> = map(fn(p) -> p + p, xs)"),
        Ty::list(Ty::Int)
    );
}

#[test]
fn arithmetic_on_a_context_bound_non_numeric_is_rejected() {
    // `p` is pinned to Bool by the list, but used in `+` — the deferred numeric check fires.
    rejects("fn f(xs: List<Bool>): List<Bool> = map(fn(p) -> p + p, xs)");
}

// ── Ascription ─────────────────────────────────────────────────────────────────

#[test]
fn ascription_checks_and_fixes_the_type() {
    assert_eq!(ty("fn f(): Int = (3 : Int)"), Ty::Int);
}

#[test]
fn ascription_mismatch_is_rejected() {
    rejects("fn f(): Int = (3 : Bool)");
}

// ── Totality: quantum forms are reported, never panicked on ─────────────────────

#[test]
fn quantum_forms_are_unsupported_not_panics() {
    rejects("fn f(): Int = H @ 0");
    rejects("fn f(): Int = measure(q)");
    rejects("fn f(): Int = circuit { H @ 0 }");
}

// ── Type aliases: resolution and cycle rejection ────────────────────────────────

#[test]
fn nullary_alias_resolves_to_its_body() {
    accepts("type Pair = (Int, Bool)\nfn f(p: Pair): Bool = let (_, b) = p in b");
}

#[test]
fn parameterized_alias_substitutes_arguments() {
    accepts("type Box<n> = QReg<n>\nfn f(): Int = 0");
}

#[test]
fn directly_recursive_alias_is_rejected() {
    // `type R = R` must be rejected, not expanded forever (regression: fuzzer OOM).
    rejects("type R = R\nfn f(x: R): Int = 0");
}

#[test]
fn mutually_recursive_aliases_are_rejected() {
    rejects("type A = B\ntype B = A\nfn f(x: A): Int = 0");
}

#[test]
fn parameterized_self_alias_with_growing_args_is_rejected() {
    // Each expansion grows the Nat argument, so a depth bound alone would OOM; the
    // name-cycle guard rejects it on first re-entry.
    rejects("type Reg<n> = Reg<n + 1>\nfn f(x: Reg<3>): Int = 0");
}

#[test]
fn mutual_recursion_resolves_signatures() {
    // `even` references `odd`, declared afterwards: signatures are collected before bodies.
    accepts(
        "fn even(n: Int): Bool = odd(n)\n\
         fn odd(n: Int): Bool = even(n)",
    );
}

// ── Linearity: no-cloning (contraction is absent) ───────────────────────────────

#[test]
fn using_a_qubit_once_is_accepted() {
    accepts("fn f(q: Qubit): Qubit = q");
}

#[test]
fn using_a_qubit_twice_is_rejected() {
    let err = reject_err("fn f(q: Qubit): QReg<2> = (q, q)");
    assert!(
        matches!(err, TypeError::LinearUsedTwice { ref name, .. } if name == "q"),
        "expected LinearUsedTwice on `q`, got {err:?}"
    );
}

#[test]
fn cloning_via_let_tuple_is_rejected() {
    // The SPEC's canonical no-cloning counterexample (§3.4).
    rejects("fn clone(q: Qubit): (Qubit, Qubit) = let p = (q, q) in p");
}

#[test]
fn a_qubit_threaded_through_a_let_is_used_once() {
    accepts("fn f(q: Qubit): Qubit = let r = q in r");
}

// ── Linearity: no-dropping (weakening is absent) ────────────────────────────────

#[test]
fn dropping_a_qubit_at_scope_exit_is_rejected() {
    let err = reject_err("fn f(q: Qubit): Int = 0");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { ref name, .. } if name == "q"),
        "expected LinearUnconsumed on `q`, got {err:?}"
    );
}

#[test]
fn dropping_a_let_bound_qubit_is_rejected() {
    let err = reject_err("fn f(q: QReg<2>): Int = let (a, b) = destructure(q) in 0");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { .. }),
        "got {err:?}"
    );
}

#[test]
fn discarding_a_qubit_with_wildcard_is_rejected() {
    let err = reject_err("fn f(q: Qubit): Int = let _ = q in 0");
    assert!(
        matches!(err, TypeError::LinearDiscard { .. }),
        "got {err:?}"
    );
}

#[test]
fn discarding_a_destructured_qubit_with_wildcard_is_rejected() {
    // `let (a, _) = destructure(q)` abandons the second qubit — rejected (SPEC §3.4).
    let err = reject_err("fn f(q: QReg<2>): Qubit = let (a, _) = destructure(q) in a");
    assert!(
        matches!(err, TypeError::LinearDiscard { .. }),
        "got {err:?}"
    );
}

// ── Tensor introduction / elimination (QReg) ────────────────────────────────────

#[test]
fn a_pair_of_qubits_synthesizes_to_a_register() {
    assert_eq!(
        ty("fn f(a: Qubit, b: Qubit): QReg<2> = (a, b)"),
        Ty::QReg(2)
    );
}

#[test]
fn destructure_consumes_a_register_and_rebinds_its_qubits() {
    accepts("fn f(q: QReg<2>): QReg<2> = let (a, b) = destructure(q) in (a, b)");
}

#[test]
fn destructure_arity_follows_the_register_size() {
    accepts("fn f(q: QReg<3>): QReg<3> = let (a, b, c) = destructure(q) in (a, b, c)");
    // A 3-register cannot be split into a 2-tuple pattern.
    rejects("fn f(q: QReg<3>): QReg<2> = let (a, b) = destructure(q) in (a, b)");
}

#[test]
fn destructure_of_a_non_register_is_rejected() {
    rejects("fn f(x: Int): Int = let (a, b) = destructure(x) in 0");
}

#[test]
fn reusing_a_destructured_qubit_is_rejected() {
    let err = reject_err("fn f(q: QReg<2>): QReg<2> = let (a, b) = destructure(q) in (a, a)");
    assert!(
        matches!(err, TypeError::LinearUsedTwice { ref name, .. } if name == "a"),
        "got {err:?}"
    );
}

// ── Branching: residuals must agree (if / match) ────────────────────────────────

#[test]
fn if_consuming_the_same_resource_in_both_branches_is_accepted() {
    accepts("fn f(c: Bool, q: Qubit): Qubit = if c then q else q");
}

#[test]
fn if_consuming_a_resource_in_one_branch_only_is_rejected() {
    // `then` spends `q`, `else` spends `q2`: their residuals disagree.
    let err = reject_err("fn f(c: Bool, q: Qubit, q2: Qubit): Qubit = if c then q else q2");
    assert!(
        matches!(err, TypeError::LinearBranchMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn match_consuming_the_same_resource_in_every_arm_is_accepted() {
    accepts("fn f(s: Bool, q: Qubit): Qubit = match s { true => q, false => q }");
}

#[test]
fn match_arm_with_a_different_residual_is_rejected() {
    let err = reject_err(
        "fn f(s: Bool, q: Qubit, q2: Qubit): Qubit = match s { true => q, false => q2 }",
    );
    assert!(
        matches!(err, TypeError::LinearBranchMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn match_arm_dropping_a_pattern_bound_resource_is_rejected() {
    // The arm binds the whole register as `p` but never consumes it.
    let err = reject_err("fn f(q: QReg<2>): Int = match q { p => 0 }");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { ref name, .. } if name == "p"),
        "got {err:?}"
    );
}

#[test]
fn a_resource_may_survive_a_branch_and_be_consumed_after() {
    // `q2` is consumed by neither arm (equal residuals), then consumed after the match.
    accepts(
        "fn f(s: Bool, q: Qubit, q2: Qubit): (Qubit, Qubit) = \
         let r = match s { true => q, false => q } in (r, q2)",
    );
}

// ── Closures may not capture linear resources ───────────────────────────────────

#[test]
fn capturing_a_qubit_in_a_closure_is_rejected() {
    let err = reject_err("fn f(q: Qubit): Int -> Qubit = fn(x: Int) -> q");
    assert!(
        matches!(err, TypeError::LinearCapture { ref name, .. } if name == "q"),
        "got {err:?}"
    );
}

#[test]
fn a_closure_consuming_its_own_linear_parameter_is_fine() {
    accepts("fn f(): Qubit -o Qubit = fn(q: Qubit) -> q");
}

#[test]
fn a_closure_dropping_its_own_linear_parameter_is_rejected() {
    rejects("fn f(): Qubit -o Int = fn(q: Qubit) -> 0");
}

// ── Classical programs are unaffected by the linear context ─────────────────────

#[test]
fn classical_functions_keep_an_empty_linear_context() {
    accepts("fn f(x: Int, y: Bool): Int = if y then x else x + 1");
    accepts("fn f(xs: List<Int>): List<Int> = map(fn(p) -> p + p, xs)");
}
