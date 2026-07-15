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

/// Whole-program check through the desugaring pass (issue #8) first — the pipeline used for
/// `run { }` blocks, so monadic forms are checked on the desugared `Bind`/`Return` tree.
fn check_run(src: &str) -> Result<(), Vec<TypeError>> {
    let decls = crate::desugar::desugar_decls(parse_program(src).expect("parse failed"))
        .expect("desugar reported errors");
    TypeChecker::new().check_decls(&decls)
}

fn accepts_run(src: &str) {
    if let Err(e) = check_run(src) {
        panic!("expected `{src}` to type-check, got {e:?}");
    }
}

fn reject_run_err(src: &str) -> TypeError {
    let mut errs = check_run(src).expect_err("expected rejection");
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

#[test]
fn discarding_a_bare_qubit_with_underscore_name_is_rejected() {
    // A `_`-prefixed name discards a `split` *register* remainder (SPEC §3.4), but it must not
    // become a blanket escape hatch: a bare `Qubit` bound to `_q` is still a no-dropping error,
    // so a typo cannot silently leak a qubit.
    let err = reject_err("fn f(q: Qubit): Int = let _q = q in 0");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { .. }),
        "got {err:?}"
    );
}

#[test]
fn discarding_a_split_register_remainder_is_accepted() {
    // The sanctioned discard: `split` yields `(QReg<k>, QReg<n-k>)`, and the remainder may be
    // dropped with a `_`-prefixed name (as the QEC / Bernstein–Vazirani fixtures do).
    accepts_run(
        "fn f(q: QReg<2>): Q<QReg<1>> = run {\n  let (a, _rest) = split(1, q)\n  return a\n}",
    );
}

// ── Tensor introduction / elimination (QReg) ────────────────────────────────────

#[test]
fn a_pair_of_qubits_synthesizes_to_a_register() {
    assert_eq!(
        ty("fn f(a: Qubit, b: Qubit): QReg<2> = (a, b)"),
        Ty::QReg(DepthExpr::Nat(2))
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

// ── Circuit fragment (issue #11) ────────────────────────────────────────────────

#[test]
fn gate_primitives_have_their_circuit_types() {
    assert_eq!(
        ty("fn f(): Circuit<1, 1, 1, Clifford> = H"),
        Ty::Circuit {
            n: DepthExpr::Nat(1),
            m: DepthExpr::Nat(1),
            d: DepthExpr::Nat(1),
            c: crate::ast::CliffordClass::Clifford,
        }
    );
}

#[test]
fn bell_gate_synthesizes_its_annotated_type() {
    // Acceptance criterion: `H @0 |> CNOT @(0,1)` is `Circuit<2,2,2,Clifford>`.
    accepts("fn bell(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
}

#[test]
fn bell_gate_with_wrong_depth_is_rejected() {
    // Depth is an upper bound (SPEC §3.3): the rejection direction is an annotation *below* the
    // real depth. The bell circuit has depth 2; annotating depth 1 is rejected.
    rejects("fn bell(): Circuit<2, 2, 1, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
}

#[test]
fn width_changing_encode_circuit_is_accepted() {
    // Gate placement grows the ambient register footprint (encode : Circuit<1,3,…>).
    accepts("fn enc(): Circuit<1, 3, 2, Clifford> = circuit { CNOT @(0,1) |> CNOT @(0,2) }");
}

#[test]
fn gate_index_beyond_footprint_is_rejected() {
    let err = reject_err("fn f(): Circuit<1, 1, 1, Clifford> = circuit { H @5 }");
    assert!(
        matches!(err, TypeError::IndexOutOfBounds { .. }),
        "got {err:?}"
    );
}

#[test]
fn gate_index_out_of_bounds_is_rejected() {
    rejects("fn f(): Circuit<2, 2, 1, Clifford> = circuit { H @5 }");
}

#[test]
fn gate_with_wrong_target_arity_is_rejected() {
    // CNOT acts on two qubits; a single target is a placement error.
    rejects("fn f(): Circuit<2, 2, 1, Clifford> = circuit { CNOT @0 }");
}

// ── The five composition rules (§3.3) ───────────────────────────────────────────

#[test]
fn compose_chains_widths_and_adds_depths() {
    accepts(
        "fn f(a: Circuit<2,2,1,Clifford>, b: Circuit<2,2,2,Clifford>): Circuit<2,2,3,Clifford> = a |> b",
    );
}

#[test]
fn compose_with_mismatched_widths_is_rejected() {
    // Acceptance criterion: `f |> g` where `f.out ≠ g.in` is a type error.
    let err = reject_err("fn f(): Circuit<2,3,0,Clifford> = identity(2) |> identity(3)");
    assert!(
        matches!(err, TypeError::QubitCountMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn par_scales_width_and_keeps_depth() {
    accepts("fn f(c: Circuit<2,2,1,Clifford>): Circuit<6,6,1,Clifford> = par { c } * 3");
}

#[test]
fn adjoint_swaps_widths_and_keeps_depth() {
    accepts("fn f(c: Circuit<1,3,2,Clifford>): Circuit<3,1,2,Clifford> = adjoint(c)");
}

#[test]
fn controlled_increments_widths_and_depth() {
    accepts("fn f(c: Circuit<2,2,2,Clifford>): Circuit<3,3,3,Clifford> = controlled(c)");
}

#[test]
fn repeat_multiplies_depth_by_the_count() {
    // Acceptance criterion: `repeat(k, c)` with `k: Int`, `c: Circuit<n,n,d,_>` is `k*d`.
    accepts("fn f(k: Nat, c: Circuit<2,2,3,Clifford>): Circuit<2,2,k*3,Clifford> = repeat(k, c)");
}

#[test]
fn repeat_with_wrong_depth_is_rejected() {
    // `repeat(k, c)` over a depth-3 `c` synthesizes depth `k*3`; annotating `k*2` is below that
    // bound (false for any `k ≥ 1`), so it is rejected.
    rejects("fn f(k: Nat, c: Circuit<2,2,3,Clifford>): Circuit<2,2,k*2,Clifford> = repeat(k, c)");
}

// ── for-loops in circuit context (§5.8) ─────────────────────────────────────────

#[test]
fn parallel_for_over_qubits_keeps_body_depth() {
    accepts(
        "fn had_all(n: Nat): Circuit<n, n, 1, Clifford> = circuit { for q in qubits(n) { H(q) } }",
    );
}

#[test]
fn sequential_for_over_range_multiplies_depth() {
    // `range(k)` iterations are sequential: depth = count × body depth.
    accepts(
        "fn layer(k: Nat): Circuit<k, k, k, Universal> = circuit { for i in range(k) { T(i) } }",
    );
}

// ── Symbolic depth via fold over a circuit accumulator (§3.6) ────────────────────

#[test]
fn fold_over_circuit_synthesizes_symbolic_depth() {
    // Acceptance criterion: `ising_evolve` with `n_steps: Int` in the depth position
    // synthesizes `Circuit<n,n,n_steps*n,Universal>` with no explicit promotion syntax.
    accepts(
        "fn trotter(n: Nat): Circuit<n, n, 1, Universal> = circuit { for q in qubits(n) { T q } }\n\
         fn ising(n: Nat, n_steps: Int): Circuit<n, n, n_steps, Universal> =\n\
         fold(range(n_steps), identity(n), fn(acc, _) -> acc |> trotter(n))",
    );
}

#[test]
fn classical_fold_still_threads_its_accumulator() {
    // The circuit-fold special case must not disturb the ordinary classical fold.
    assert_eq!(
        ty("fn f(xs: List<Int>): Int = fold(xs, 0, fn(acc, x) -> acc + x)"),
        Ty::Int
    );
}

// ── Clifford classification inference (issue #12, §3.7) ──────────────────────────

/// A single-qubit gate placed in a 1-qubit circuit, checked against `class`.
fn single(gate: &str, class: &str) -> String {
    format!("fn f(): Circuit<1, 1, 1, {class}> = circuit {{ {gate} @0 }}")
}

#[test]
fn clifford_single_qubit_gates_are_inferred_clifford() {
    for g in ["I", "X", "Y", "Z", "H", "S", "S_dag", "SX", "SX_dag"] {
        accepts(&single(g, "Clifford"));
        // Annotating an inferred-Clifford gate as Universal is *accepted* by subsumption
        // (issue #58): `Clifford ⊑ Universal`, so a Clifford value satisfies a Universal
        // annotation. The reverse (Universal value, Clifford annotation) stays rejected
        // (see `universal_single_qubit_gates_are_inferred_universal`).
        accepts(&single(g, "Universal"));
    }
}

#[test]
fn universal_single_qubit_gates_are_inferred_universal() {
    for g in ["T", "T_dag"] {
        accepts(&single(g, "Universal"));
        let err = reject_err(&single(g, "Clifford"));
        assert!(
            matches!(err, TypeError::CliffordMismatch { .. }),
            "got {err:?}"
        );
    }
}

#[test]
fn clifford_two_qubit_gates_are_inferred_clifford() {
    for g in ["CNOT", "CX", "CY", "CZ", "SWAP", "iSWAP", "ECR"] {
        accepts(&format!(
            "fn f(): Circuit<2, 2, 1, Clifford> = circuit {{ {g} @(0, 1) }}"
        ));
    }
}

#[test]
fn universal_two_qubit_gates_are_inferred_universal() {
    for g in ["Rzz", "Rxx", "Ryy", "CRz", "CRx", "CP"] {
        accepts(&format!(
            "fn f(): Circuit<2, 2, 1, Universal> = circuit {{ {g}(0.5) @(0, 1) }}"
        ));
        rejects(&format!(
            "fn f(): Circuit<2, 2, 1, Clifford> = circuit {{ {g}(0.5) @(0, 1) }}"
        ));
    }
}

#[test]
fn composition_joins_classes_universal_absorbs_clifford() {
    // Acceptance criterion: `H @0 |> T @0` is Universal (join rule).
    accepts("fn f(): Circuit<1, 1, 2, Universal> = circuit { H @0 |> T @0 }");
    let err = reject_err("fn f(): Circuit<1, 1, 2, Clifford> = circuit { H @0 |> T @0 }");
    assert!(
        matches!(err, TypeError::CliffordMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn rotation_at_quarter_turn_is_clifford() {
    // Acceptance criterion: `Rz(PI/2)` is Clifford.
    accepts("fn f(): Circuit<1, 1, 1, Clifford> = circuit { Rz(PI / 2.0) @0 }");
    accepts("fn f(): Circuit<1, 1, 1, Clifford> = circuit { Rx(PI) @0 }");
    accepts("fn f(): Circuit<1, 1, 1, Clifford> = circuit { Ry(0.0) @0 }");
}

#[test]
fn rotation_at_generic_angle_is_universal() {
    // Acceptance criterion: `Rz(0.3)` is Universal.
    accepts("fn f(): Circuit<1, 1, 1, Universal> = circuit { Rz(0.3) @0 }");
    let err = reject_err("fn f(): Circuit<1, 1, 1, Clifford> = circuit { Rz(0.3) @0 }");
    assert!(
        matches!(err, TypeError::CliffordMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn rotation_at_runtime_angle_is_universal() {
    // A runtime angle cannot be proved a multiple of π/2, so it stays Universal.
    accepts("fn f(beta: Float): Circuit<1, 1, 1, Universal> = circuit { Rz(beta) @0 }");
    rejects("fn f(beta: Float): Circuit<1, 1, 1, Clifford> = circuit { Rz(beta) @0 }");
}

// ── Clifford subsumption (issue #58, SPEC §3.3/§3.7) ─────────────────────────────

#[test]
fn clifford_value_satisfies_universal_annotation() {
    // AC #58a: a Clifford value (`identity(1)`, depth 0) satisfies a Universal annotation,
    // since `Clifford ⊑ Universal`. This is what lets a Clifford base case inhabit a
    // Universal-annotated recursive definition (#60).
    accepts("fn f(): Circuit<1, 1, 0, Universal> = identity(1)");
}

#[test]
fn universal_value_violates_clifford_annotation() {
    // AC #58b: the reverse direction stays an error — a Universal value (`T @0`) never
    // satisfies a Clifford annotation.
    let err = reject_err("fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }");
    assert!(
        matches!(err, TypeError::CliffordMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn clifford_argument_satisfies_universal_parameter() {
    // AC #58c: a Clifford argument is accepted where a `Circuit<…, Universal>` parameter is
    // expected (subsumption flows through function-argument checking).
    accepts(
        "fn use_universal(c: Circuit<1, 1, 0, Universal>): Circuit<1, 1, 0, Universal> = c\n\
         fn f(): Circuit<1, 1, 0, Universal> = use_universal(identity(1))",
    );
}

// ── Symbolic depth refinement (issue #13, SPEC §3.6) ─────────────────────────────

#[test]
fn symbolic_depth_annotation_matching_inferred_var_is_accepted() {
    // AC: `Circuit<1,1,n,_>` verifies the user's `n` against the inferred `DepthExpr::Var("n")`.
    // Here the body's depth is exactly the symbolic `n` it was given — the structural fast path.
    accepts("fn f(c: Circuit<1, 1, n, Clifford>): Circuit<1, 1, n, Clifford> = c");
}

#[test]
fn symbolic_depth_exceeding_the_bound_is_a_depth_mismatch() {
    // Depth is an *upper bound* (SPEC §3.3): a synthesized depth larger than the annotation is
    // the rejection direction. Body depth is `n + 1`; the annotation claims only `n`. Z3 finds a
    // counterexample (any n), so this is a depth mismatch — reported specifically, not as a
    // generic unification failure. (The reverse — body `n`, annotation `n + 1` — is now *accepted*
    // as a valid looser bound; see `looser_depth_annotation_is_accepted_as_a_bound`.)
    let err = reject_err("fn f(c: Circuit<1, 1, n + 1, Clifford>): Circuit<1, 1, n, Clifford> = c");
    assert!(
        matches!(err, TypeError::DepthMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn constant_depth_below_the_synthesized_depth_is_a_depth_mismatch() {
    // AC: a concrete annotation *below* the real depth (annotated 1 where the circuit has depth 2)
    // is rejected with a depth-specific error — via the constant fast path, no solver needed.
    let err =
        reject_err("fn bell(): Circuit<2, 2, 1, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
    assert!(
        matches!(err, TypeError::DepthMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn looser_depth_annotation_is_accepted_as_a_bound() {
    // The dual of the two rejections above: depth is "bounded above by d", so an annotation
    // *looser* than the synthesized depth is a valid bound and accepted — constant (depth 2 bell
    // annotated 5) and symbolic (`n` annotated `n + 1`) alike.
    accepts("fn bell(): Circuit<2, 2, 5, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
    accepts("fn f(c: Circuit<1, 1, n, Clifford>): Circuit<1, 1, n + 1, Clifford> = c");
}

#[test]
fn z3_proves_a_nontrivial_symbolic_depth_equality() {
    // `repeat(2, c)` over a depth-`n+1` circuit has depth `2 * (n + 1)`; the annotation writes
    // the distributed form `2*n + 2`. These are not structurally equal but Z3 proves them so.
    accepts(
        "fn f(c: Circuit<1, 1, n + 1, Clifford>): Circuit<1, 1, 2 * n + 2, Clifford> = repeat(2, c)",
    );
}

#[test]
fn bilinear_depth_is_accepted() {
    // SPEC §3.6 ising-style depth: a product of two runtime variables (`n_steps * n`) is a
    // legitimate symbolic depth, not rejected as "non-linear".
    accepts(
        "fn f(n_steps: Int, c: Circuit<1, 1, n, Clifford>): Circuit<1, 1, n_steps * n, Clifford> \
         = repeat(n_steps, c)",
    );
}

#[test]
fn if_over_circuits_joins_depth_by_max() {
    // ADR-0005: a classically-selected circuit takes the worst-case depth of its arms.
    // `X` is depth 1, `identity(1)` is depth 0, so the conditional is depth `max(1,0) = 1`.
    assert_eq!(
        ty("fn f(b: Bool): Int = if b then X else identity(1)"),
        Ty::Circuit {
            n: DepthExpr::Nat(1),
            m: DepthExpr::Nat(1),
            d: DepthExpr::Nat(1),
            c: CliffordClass::Clifford,
        }
    );
}

#[test]
fn match_over_circuits_joins_depth_by_max() {
    assert_eq!(
        ty("fn f(k: Int): Int = match k { 0 => identity(1), _ => X }"),
        Ty::Circuit {
            n: DepthExpr::Nat(1),
            m: DepthExpr::Nat(1),
            d: DepthExpr::Nat(1),
            c: CliffordClass::Clifford,
        }
    );
}

// ── Value-dependent application: call-site substitution (issue #57, SPEC §3.9) ───

#[test]
fn dependent_call_specializes_a_nat_parameter_in_the_result_type() {
    // 57a: `dbl(n): Circuit<n, 2*n, 0, Clifford>` applied to `k + 1` yields
    // `Circuit<k+1, 2*(k+1), 0, Clifford>` — the `Nat` parameter `n` is replaced by the lowered
    // call-site argument *everywhere* it appears (both the `n` and the `2*n` position).
    let kp1 = || DepthExpr::Var("k".into()).seq(DepthExpr::Nat(1));
    assert_eq!(
        ty(
            "fn dbl(n: Nat): Circuit<n, 2 * n, 0, Clifford> = identity(n)\n\
            fn caller(k: Nat): Int = dbl(k + 1)"
        ),
        Ty::Circuit {
            n: kp1(),
            m: DepthExpr::repeat(DepthExpr::Nat(2), kp1()),
            d: DepthExpr::Nat(0),
            c: CliffordClass::Clifford,
        }
    );
}

#[test]
fn dependent_call_substitutes_into_a_register_width() {
    // 57b: `g(m): QReg<m>` applied to `n - 1` yields `QReg<n-1>` — the argument, not the formal.
    assert_eq!(
        ty("fn g(m: Nat): QReg<m> = qreg(m)\n\
            fn caller(n: Nat): Int = g(n - 1)"),
        Ty::QReg(DepthExpr::Var("n".into()).minus(DepthExpr::Nat(1)))
    );
}

#[test]
fn dependent_substitution_reaches_nested_type_positions() {
    // 57c: substitution recurses through `Q<…>` and `Matrix<…>` to the depth/width leaves.
    assert_eq!(
        ty("fn h(n: Nat): Q<QReg<n>> = qreg(n)\n\
            fn caller(k: Nat): Int = h(k + 2)"),
        Ty::Q(Box::new(Ty::QReg(
            DepthExpr::Var("k".into()).seq(DepthExpr::Nat(2))
        )))
    );
    assert_eq!(
        ty("fn mat(n: Nat): Matrix<n, n, Int> = identity(n)\n\
            fn caller(k: Nat): Int = mat(k + 2)"),
        Ty::Matrix(
            DepthExpr::Var("k".into()).seq(DepthExpr::Nat(2)),
            DepthExpr::Var("k".into()).seq(DepthExpr::Nat(2)),
            Box::new(Ty::Int),
        )
    );
}

#[test]
fn non_lowerable_nat_argument_is_a_non_dependent_arg_error() {
    // 57d: a `Nat` argument that is not a static depth expression (here a nested *application*)
    // cannot specialize the dependent parameter, and is reported specifically.
    let err = reject_err(
        "fn qft(n: Nat): Circuit<n, n, n, Universal> = identity(n)\n\
         fn foo(x: Int): Int = x\n\
         fn caller(x: Int): Int = qft(foo(x))",
    );
    assert!(
        matches!(err, TypeError::NonDependentArg { .. }),
        "got {err:?}"
    );
}

// ── Dependent match refinement: per-arm assumptions (issue #59, SPEC §3.6) ───────

#[test]
fn literal_match_arm_refines_the_scrutinee_for_width_obligations() {
    // 59a: in the `0 =>` arm of `match n`, the assumption `n = 0` lets `identity(0)` (width 0)
    // satisfy the function's `Circuit<n, n, …>` result — the width obligation `0 = n` is provable
    // *only* under that refinement. The `_` arm gets `n ≠ 0` and uses the full-width `identity(n)`.
    accepts(
        "fn f(n: Nat): Circuit<n, n, 0, Clifford> = \
         match n { 0 => identity(0), _ => identity(n) }",
    );
}

#[test]
fn match_refinement_does_not_leak_into_sibling_arms() {
    // 59b: the `n = 0` assumption from the first arm must not persist into the `_` arm. If it did,
    // `identity(0)` (width 0) would wrongly satisfy `Circuit<n, n, …>` there too. Under the correct
    // scoping the `_` arm only knows `n ≠ 0`, so the width obligation `0 = n` fails — a rejection.
    let err = reject_err(
        "fn f(n: Nat): Circuit<n, n, 0, Clifford> = \
         match n { 0 => identity(0), _ => identity(0) }",
    );
    assert!(
        matches!(err, TypeError::QubitCountMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn if_guard_introduces_no_nat_refinement() {
    // Documented boundary (SPEC §3.6): only `match` refines a `Nat` scrutinee; a `Bool` `if`-guard
    // does not. So `identity(0)` cannot satisfy `Circuit<n, n, …>` in an `if` branch — `0 = n` is
    // unprovable without the `n = 0` a `match { 0 => … }` arm would supply.
    let err = reject_err(
        "fn f(n: Nat, b: Bool): Circuit<n, n, 0, Clifford> = \
         if b then identity(0) else identity(n)",
    );
    assert!(
        matches!(err, TypeError::QubitCountMismatch { .. }),
        "got {err:?}"
    );
}

// ── Recursive circuit functions: well-founded measure (issue #60, SPEC §3.3) ─────

#[test]
fn well_founded_recursion_on_a_decreasing_nat_measure_is_accepted() {
    // 60c: `rec(n)` recurses on `n - 1` under the `_` arm's `n ≥ 1`, appending a depth-1 gate.
    // The synthesized depth `(n-1) + 1 = n` meets the `Circuit<1,1,n,…>` bound, and `n` strictly
    // decreases (staying ≥ 0) at the single recursive call — so the recursion is well-founded.
    accepts(
        "fn rec(n: Nat): Circuit<1, 1, n, Clifford> = \
         match n { 0 => identity(1), _ => rec(n - 1) |> X }",
    );
}

#[test]
fn non_decreasing_recursion_is_ill_founded() {
    // 60b: a self-call on the *same* (or a *larger*) argument has no decreasing measure.
    let same = reject_err("fn f(n: Nat): Circuit<1, 1, 0, Clifford> = f(n)");
    assert!(
        matches!(same, TypeError::IllFoundedRecursion { .. }),
        "got {same:?}"
    );
    let larger = reject_err("fn f(n: Nat): Circuit<1, 1, 0, Clifford> = f(n + 1)");
    assert!(
        matches!(larger, TypeError::IllFoundedRecursion { .. }),
        "got {larger:?}"
    );
}

#[test]
fn recursion_without_a_base_case_is_ill_founded() {
    // 60b′: `f(n) = f(n-1) |> X` decreases, but nothing establishes `n ≥ 1`, so `n - 1 ≥ 0` fails
    // at `n = 0` — the measure's non-negativity half forces a base case. (The depth bound `n` is
    // met, so the rejection is specifically the termination obligation, not a depth mismatch.)
    let err = reject_err("fn f(n: Nat): Circuit<1, 1, n, Clifford> = f(n - 1) |> X");
    assert!(
        matches!(err, TypeError::IllFoundedRecursion { .. }),
        "got {err:?}"
    );
}

#[test]
fn well_founded_recursion_with_a_wrong_depth_bound_is_a_depth_mismatch() {
    // 60d: the measure decreases, but the body's depth `(n-1) + 2 = n + 1` exceeds the annotated
    // `n` — termination holds, the *bound* does not. Reported as a depth mismatch, not a recursion
    // error, so the diagnostics stay precise about which obligation failed.
    let err = reject_err(
        "fn rec(n: Nat): Circuit<1, 1, n, Clifford> = \
         match n { 0 => identity(1), _ => rec(n - 1) |> X |> X }",
    );
    assert!(
        matches!(err, TypeError::DepthMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn recursive_qft_kernel_type_checks_end_to_end() {
    // 60a: a self-contained recursive QFT — the heart of `shor.qn`. The base case `identity(0)`
    // checks under `n = 0`, the self-call `qft(n-1)` terminates on the measure `n` under `n ≥ 1`,
    // and the synthesized step depth is proven `≤ 2*n*n` by Z3. (Full `shor.qn` is exercised by
    // `reference_algorithm_fixtures_type_check`; this is the minimal kernel in isolation.)
    accepts(
        "fn apply_hadamard(n: Nat): Circuit<n, n, 1, Clifford> = circuit { H @0 }\n\
         fn controlled_rotations(n: Nat): Circuit<n, n, 2 * (n - 1), Universal> = \
            circuit { for i in range(n - 1) { controlled(Rz(PI / 4.0)) @(0, i + 1) } }\n\
         fn qft(n: Nat): Circuit<n, n, 2 * n * n, Universal> = \
            match n { \
                0 => identity(0), \
                _ => apply_hadamard(n) |> controlled_rotations(n) \
                     |> (qft(n - 1) `on_high` n) |> swap_reverse(n) \
            }",
    );
}

#[test]
fn mutual_circuit_recursion_is_rejected() {
    // 60e: two circuit functions that call each other have no per-body decreasing measure, so the
    // cycle is rejected up front rather than accepted without a termination witness.
    let err = reject_err(
        "fn a(n: Nat): Circuit<1, 1, 0, Clifford> = b(n)\n\
         fn b(n: Nat): Circuit<1, 1, 0, Clifford> = a(n)",
    );
    assert!(
        matches!(err, TypeError::MutualRecursion { .. }),
        "got {err:?}"
    );
}

// ── Quantum monad: Q<τ>, run { } bind chains (issue #14, SPEC §3.5) ──────────────

#[test]
fn bind_measure_then_return_is_q_bit() {
    // AC: `run { x <- measure(q); return x }` type-checks with `q` consumed, result `Q<Bit>`.
    accepts_run("fn f(q: Qubit): Q<Bit> = run {\n  x <- measure(q)\n  return x\n}");
}

#[test]
fn using_a_qubit_after_measuring_it_is_rejected() {
    // AC: a second use of `q` after `measure(q)` is a no-cloning violation.
    let err = reject_run_err(
        "fn f(q: Qubit): Q<Bit> = run {\n  x <- measure(q)\n  y <- measure(q)\n  return x\n}",
    );
    assert!(
        matches!(err, TypeError::LinearUsedTwice { .. }),
        "got {err:?}"
    );
}

#[test]
fn binding_a_classical_value_with_arrow_is_rejected() {
    // AC: a `<-` whose right-hand side is not `Q<_>` (here a bare `Int`) is an error — the
    // monad is entered by measurement/allocation, not by classical values. Use `let`.
    let err = reject_run_err("fn f(): Q<Int> = run {\n  x <- 5\n  return x\n}");
    assert!(
        matches!(err, TypeError::ExpectedMonad { .. }),
        "got {err:?}"
    );
}

#[test]
fn return_lifts_a_value_into_the_monad() {
    accepts_run("fn f(): Q<Int> = run {\n  return 7\n}");
}

#[test]
fn measure_consumes_its_qubit_so_dropping_is_an_error() {
    // `measure` takes the qubit linearly; a function that measures nothing must still consume
    // its qubit parameter. Here `q` is never used, a no-dropping violation.
    let err = reject_run_err("fn f(q: Qubit): Q<Int> = run {\n  return 0\n}");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { .. }),
        "got {err:?}"
    );
}

#[test]
fn hello_bell_type_checks_end_to_end() {
    // AC: the `hello_bell` reference algorithm (SPEC §12) type-checks.
    accepts_run(
        "fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {\n\
         \x20   H @0 |> CNOT @(0, 1)\n\
         }\n\
         fn hello_bell(): Q<(Bit, Bit)> = run {\n\
         \x20   (q0, q1) <- bell_state() @ qreg(2)\n\
         \x20   b0       <- measure(q0)\n\
         \x20   b1       <- measure(q1)\n\
         \x20   return (b0, b1)\n\
         }",
    );
}

#[test]
fn teleport_type_checks_end_to_end() {
    // AC: the `teleport` reference algorithm (SPEC §12) type-checks end-to-end. Exercises
    // pure circuit application (`X @ b : Qubit`), monadic register binds, `Bit`-driven `if`,
    // and branch depth join (`if x_bit then X else identity(1)`).
    accepts_run(
        "fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {\n\
         \x20   H @0 |> CNOT @(0, 1)\n\
         }\n\
         fn teleport(msg: Qubit, alice: Qubit, bob: Qubit): Q<Qubit> = run {\n\
         \x20   (a, b)   <- bell_state() @ (alice, bob)\n\
         \x20   (m2, a2) <- adjoint(bell_state()) @ (msg, a)\n\
         \x20   x_bit    <- measure(m2)\n\
         \x20   z_bit    <- measure(a2)\n\
         \x20   let b2    = (if x_bit then X else identity(1)) @ b\n\
         \x20   let b3    = (if z_bit then Z else identity(1)) @ b2\n\
         \x20   return b3\n\
         }",
    );
}

// ── Register concatenation: `tensored` (issue #15, SPEC §5) ──────────────────────

#[test]
fn tensored_two_qubits_is_a_two_qubit_register() {
    // `q0 `tensored` q1 : QReg<2>` — tensor introduction over two qubit handles, each
    // consumed exactly once.
    assert_eq!(
        ty("fn f(q0: Qubit, q1: Qubit): QReg<2> = q0 `tensored` q1"),
        Ty::QReg(DepthExpr::Nat(2))
    );
}

#[test]
fn tensored_register_and_qubit_grows_the_register() {
    // `QReg<2> `tensored` Qubit : QReg<3>` — a qubit counts as a width-1 register.
    assert_eq!(
        ty("fn f(r: QReg<2>, q: Qubit): QReg<3> = r `tensored` q"),
        Ty::QReg(DepthExpr::Nat(3))
    );
}

#[test]
fn tensored_reusing_an_operand_is_no_cloning() {
    let err = reject_err("fn f(q: Qubit): QReg<2> = q `tensored` q");
    assert!(
        matches!(err, TypeError::LinearUsedTwice { .. }),
        "got {err:?}"
    );
}

// ── Borrow blocks: scoped ancilla allocation (issue #15, SPEC §3.4) ───────────────

#[test]
fn borrow_discard_terminator_is_accepted() {
    // AC: `discard(a)` is a valid terminal — measure-and-drop cleans up the ancilla.
    accepts_run("fn f(): Q<Unit> = run {\n  borrow a: Qubit in {\n    discard(a)\n  }\n}");
}

#[test]
fn borrow_reset_terminator_is_accepted() {
    // AC: `reset(a)` (measure and reprepare to |0⟩) is a valid terminal, result `Q<Qubit>`.
    accepts_run("fn f(): Q<Qubit> = run {\n  borrow a: Qubit in {\n    reset(a)\n  }\n}");
}

#[test]
fn borrow_gate_on_ancilla_then_cleanup_is_accepted() {
    // AC: an ancilla may be operated on (a gate placed on its handle) so long as the borrow
    // is ultimately cleaned up; here `H @ a` then `discard`.
    accepts_run(
        "fn f(): Q<Unit> = run {\n\
         \x20 borrow a: Qubit in {\n\
         \x20   a1 <- H @ a\n\
         \x20   discard(a1)\n\
         \x20 }\n\
         }",
    );
}

#[test]
fn borrow_returning_the_ancilla_is_an_escape() {
    // AC: `return a` lets the borrowed qubit escape the borrow scope — rejected.
    let err =
        reject_run_err("fn f(): Q<Qubit> = run {\n  borrow a: Qubit in {\n    return a\n  }\n}");
    assert!(matches!(err, TypeError::BorrowEscape { .. }), "got {err:?}");
}

#[test]
fn borrow_ancilla_in_a_returned_register_is_an_escape() {
    // The escape check sees through tensor introduction: `(q, a)` puts the ancilla into the
    // returned register, so it escapes even though it is no longer the bare name returned.
    let err = reject_run_err(
        "fn f(q: Qubit): Q<QReg<2>> = run {\n\
         \x20 borrow a: Qubit in {\n\
         \x20   return (q `tensored` a)\n\
         \x20 }\n\
         }",
    );
    assert!(matches!(err, TypeError::BorrowEscape { .. }), "got {err:?}");
}

#[test]
fn borrow_unconsumed_ancilla_is_rejected() {
    // AC: an ancilla that is never consumed (cleaned up) is a no-dropping violation.
    let err =
        reject_run_err("fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}");
    assert!(
        matches!(err, TypeError::LinearUnconsumed { .. }),
        "got {err:?}"
    );
}

#[test]
fn borrow_using_the_ancilla_twice_is_no_cloning() {
    let err = reject_run_err(
        "fn f(): Q<Bit> = run {\n\
         \x20 borrow a: Qubit in {\n\
         \x20   x <- measure(a)\n\
         \x20   y <- measure(a)\n\
         \x20   return x\n\
         \x20 }\n\
         }",
    );
    assert!(
        matches!(err, TypeError::LinearUsedTwice { .. }),
        "got {err:?}"
    );
}

#[test]
fn syndrome_measure_type_checks_end_to_end() {
    // AC: the `syndrome_measure` function from the 3-qubit bit-flip QEC reference algorithm
    // (SPEC §12) type-checks end-to-end. Exercises `destructure`, a two-ancilla `borrow`,
    // `tensored`, circuit application, mid-circuit `measure`, and the escape check (the data
    // qubits are returned, the ancillas are measured away and never escape).
    accepts_run(
        "fn parity(): Circuit<2, 2, 1, Clifford> = circuit { CNOT @(0, 1) }\n\
         fn syndrome_measure(q: QReg<3>): Q<(QReg<3>, Bit, Bit)> = run {\n\
         \x20 let (q0, q1, q2) = destructure(q)\n\
         \x20 borrow a1: Qubit, a2: Qubit in {\n\
         \x20   (q0a, a1b) <- parity() @ (q0 `tensored` a1)\n\
         \x20   (q1a, a2b) <- parity() @ (q1 `tensored` a2)\n\
         \x20   s1         <- measure(a1b)\n\
         \x20   s2         <- measure(a2b)\n\
         \x20   return (q0a `tensored` q1a `tensored` q2, s1, s2)\n\
         \x20 }\n\
         }",
    );
}

#[test]
fn reference_algorithm_fixtures_type_check() {
    let fixtures = [
        (
            "bell_state",
            include_str!("../../tests/fixtures/bell_state.qn"),
        ),
        ("grover", include_str!("../../tests/fixtures/grover.qn")),
        // `shor.qn` now type-checks end-to-end: the recursive `qft` kernel exercises the full
        // value-dependent machinery (issues #57–#60) — call-site substitution, dependent-match
        // base case, the well-founded `n` measure, and the `2*n*n` depth proven as an upper bound.
        ("shor", include_str!("../../tests/fixtures/shor.qn")),
        (
            "error_correction",
            include_str!("../../tests/fixtures/error_correction.qn"),
        ),
        ("qaoa", include_str!("../../tests/fixtures/qaoa.qn")),
        (
            "bernstein_vazirani",
            include_str!("../../tests/fixtures/bernstein_vazirani.qn"),
        ),
        ("ising", include_str!("../../tests/fixtures/ising.qn")),
        (
            "stdlib_forms",
            include_str!("../../tests/fixtures/stdlib_forms.qn"),
        ),
    ];
    for (name, src) in fixtures {
        let result = if src.contains("run {") {
            check_run(src)
        } else {
            check(src)
        };
        assert!(result.is_ok(), "{name}: {:?}", result.err());
    }
    let teleport = concat!(
        include_str!("../../tests/fixtures/bell_state.qn"),
        "\n",
        include_str!("../../tests/fixtures/teleport.qn"),
    );
    assert!(
        check_run(teleport).is_ok(),
        "teleport: {:?}",
        check_run(teleport).err()
    );
}

// ── QEC frontend (issue #247) ──────────────────────────────────────────────────

#[test]
fn qec_block_is_linear_resource() {
    accepts(
        "fn f(b: QecBlock<Repetition, 3>): QecBlock<Repetition, 3> = b",
    );
    rejects(
        "fn f(b: QecBlock<Repetition, 3>): (QecBlock<Repetition, 3>, QecBlock<Repetition, 3>) = (b, b)",
    );
}

#[test]
fn qec_constructors_typecheck() {
    accepts_run(
        "fn main(): Q<Bit> = run {
  b <- repetition_code<3>()
  measure_logical_z(b)
}",
    );
    accepts_run(
        "fn main(): Q<Bit> = run {
  b <- surface_code<3>()
  measure_logical_x(b)
}",
    );
    accepts_run(
        "fn main(): Q<Bit> = run {
  b <- surface_code_x<5>()
  measure_logical_z(b)
}",
    );
}

#[test]
fn qec_invalid_distance_rejected() {
    assert!(matches!(
        reject_run_err(
            "fn main(): Q<Bit> = run {
  b <- repetition_code<1>()
  measure_logical_z(b)
}"
        ),
        TypeError::InvalidQecDistance { family: "repetition", distance: 1, .. }
    ));
    assert!(matches!(
        reject_run_err(
            "fn main(): Q<Bit> = run {
  b <- surface_code<4>()
  measure_logical_z(b)
}"
        ),
        TypeError::InvalidQecDistance { family: "surface", distance: 4, .. }
    ));
    assert!(matches!(
        reject_run_err(
            "fn main(): Q<Bit> = run {
  b <- surface_code<2>()
  measure_logical_z(b)
}"
        ),
        TypeError::InvalidQecDistance { family: "surface", .. }
    ));
}

#[test]
fn qec_memory_round_and_generic_helper() {
    accepts_run(
        "fn rounds<F: CodeFamily, d: Nat>(b: QecBlock<F, d>): Q<QecBlock<F, d>> = run {
           b2 <- memory_round(b)
           return b2
         }
         fn main(): Q<Bit> = run {
           b <- repetition_code<3>()
           b2 <- rounds(b)
           measure_logical_z(b2)
         }",
    );
}

#[test]
fn qec_kinded_alias() {
    accepts_run(
        "type Encoded<F: CodeFamily, d: Nat> = QecBlock<F, d>\n\
         fn consume(b: Encoded<Repetition, 3>): Q<Bit> = run {\n\
           measure_logical_z(b)\n\
         }\n\
         fn main(): Q<Bit> = run {\n\
           b <- repetition_code<3>()\n\
           consume(b)\n\
         }",
    );
}

#[test]
fn qec_family_mismatch_on_logical_cx() {
    assert!(matches!(
        reject_run_err(
            "fn main(): Q<(QecBlock<Surface, 3>, QecBlock<Surface, 3>)> = run {
               a <- repetition_code<3>()
               b <- surface_code<3>()
               logical_cx(a, b)
             }"
        ),
        TypeError::Mismatch { .. }
    ));
}

#[test]
fn qec_logical_cx_surface_same_d() {
    accepts_run(
        "fn main(): Q<Bit> = run {
           a <- surface_code<3>()
           b <- surface_code<3>()
           (a2, b2) <- logical_cx(a, b)
           _ <- measure_logical_z(a2)
           measure_logical_z(b2)
         }",
    );
}

#[test]
fn qec_mixed_entrypoint_rejected() {
    assert!(matches!(
        reject_run_err(
            "fn main(): Q<Bit> = run {
               b <- repetition_code<3>()
               q <- qubit()
               _ <- measure(q)
               measure_logical_z(b)
             }"
        ),
        TypeError::MixedQecEntrypoint { .. }
    ));
}

#[test]
fn qec_oracle_nat_alias_still_works() {
    accepts(
        "type Oracle<n> = Circuit<n, n, _, Universal>
fn f(o: Oracle<2>): Oracle<2> = o",
    );
}

