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
    rejects("fn bell(): Circuit<2, 2, 3, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
}

#[test]
fn circuit_too_narrow_for_its_gate_indices_is_rejected() {
    // A width-1 register cannot host `CNOT @(0,1)`: qubit index 1 is out of bounds.
    let err =
        reject_err("fn bell(): Circuit<1, 1, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
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
    rejects("fn f(k: Nat, c: Circuit<2,2,3,Clifford>): Circuit<2,2,k*4,Clifford> = repeat(k, c)");
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
        "fn trotter(n: Nat): Circuit<n, n, n, Universal> = circuit { repeat(n, T @0) }\n\
         fn ising(n: Nat, n_steps: Int): Circuit<n, n, n_steps * n, Universal> =\n\
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
        // Annotating an inferred-Clifford gate as Universal is rejected.
        rejects(&single(g, "Universal"));
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

// ── Symbolic depth refinement (issue #13, SPEC §3.6) ─────────────────────────────

#[test]
fn symbolic_depth_annotation_matching_inferred_var_is_accepted() {
    // AC: `Circuit<1,1,n,_>` verifies the user's `n` against the inferred `DepthExpr::Var("n")`.
    // Here the body's depth is exactly the symbolic `n` it was given — the structural fast path.
    accepts("fn f(c: Circuit<1, 1, n, Clifford>): Circuit<1, 1, n, Clifford> = c");
}

#[test]
fn symbolic_depth_off_by_one_is_a_depth_mismatch() {
    // Body depth is `n`; the annotation claims `n + 1`. Z3 finds a counterexample (any n),
    // so this is a depth mismatch, reported specifically (not a generic unification failure).
    let err = reject_err("fn f(c: Circuit<1, 1, n, Clifford>): Circuit<1, 1, n + 1, Clifford> = c");
    assert!(
        matches!(err, TypeError::DepthMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn constant_depth_mismatch_is_a_depth_mismatch() {
    // AC: an incorrect concrete annotation (depth 3 where the circuit has depth 2) is rejected
    // with a depth-specific error — via the constant fast path, no solver needed.
    let err =
        reject_err("fn bell(): Circuit<2, 2, 3, Clifford> = circuit { H @0 |> CNOT @(0, 1) }");
    assert!(
        matches!(err, TypeError::DepthMismatch { .. }),
        "got {err:?}"
    );
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
