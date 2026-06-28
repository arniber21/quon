// Type-checker integration tests (issue #9, classical fragment).
//
// These exercise the public `frontend::check_program` facade — parse + type-check — and
// assert on the unified `Diagnostic` stream, with a focus on the acceptance criterion that
// errors are *span-accurate*: the caret lands on the offending token, not the enclosing form.

use frontend::check_program;
use frontend::diagnostics::Diagnostic;

fn errors(src: &str) -> Vec<Diagnostic> {
    check_program(src).expect_err("expected the program to be rejected")
}

/// The single diagnostic for a program expected to have exactly one error.
fn only_error(src: &str) -> Diagnostic {
    let mut es = errors(src);
    assert_eq!(es.len(), 1, "expected exactly one diagnostic, got {es:?}");
    es.pop().unwrap()
}

/// Assert the diagnostic's span covers exactly the (first) occurrence of `needle` in `src`.
fn assert_span_on(src: &str, needle: &str, diag: &Diagnostic) {
    let start = src
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in source"));
    let end = start + needle.len();
    let span = diag.span;
    assert_eq!(
        (span.start, span.end),
        (start, end),
        "diagnostic `{}` span {:?} did not land on `{needle}` ({start}..{end})",
        diag.message,
        span,
    );
}

#[test]
fn well_typed_programs_are_accepted() {
    let src = "\
        fn double(x: Int): Int = x + x\n\
        fn pipeline(xs: List<Int>): Int = fold(map(double, xs), 0, fn(acc, x) -> acc + x)\n\
        fn classify(b: Bool): Int = match b { true => 1, false => 0 }\n";
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn if_branch_mismatch_points_at_offending_branch() {
    let src = "fn f(b: Bool): Int = if b then 1 else true";
    let diag = only_error(src);
    // The `else` branch `true` is what violates the branch agreement.
    assert_span_on(src, "true", &diag);
}

#[test]
fn unbound_variable_points_at_the_name() {
    let src = "fn f(): Int = ghost";
    let diag = only_error(src);
    assert!(diag.message.contains("unbound variable"));
    assert_span_on(src, "ghost", &diag);
}

#[test]
fn argument_mismatch_points_at_the_argument() {
    let src = "fn dbl(x: Int): Int = x + x\nfn g(): Int = dbl(true)";
    let diag = only_error(src);
    assert_span_on(src, "true", &diag);
}

#[test]
fn return_type_mismatch_points_at_body() {
    let src = "fn f(): Int = false";
    let diag = only_error(src);
    assert!(diag.message.contains("expected `Int`"));
    assert_span_on(src, "false", &diag);
}

#[test]
fn non_exhaustive_match_reports_a_witness() {
    let src = "fn f(b: Bool): Int = match b { true => 1 }";
    let diag = only_error(src);
    assert!(
        diag.message.contains("non-exhaustive") && diag.message.contains("false"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn non_exhaustive_tuple_match_names_the_missing_corner() {
    let src = "fn f(p: (Bool, Bool)): Int = match p { (true, _) => 1 }";
    let diag = only_error(src);
    assert!(
        diag.message.contains("(false, _)"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn unreachable_arm_points_at_the_dead_pattern() {
    let src = "fn f(n: Int): Int = match n { _ => 0, 5 => 1 }";
    let diag = only_error(src);
    assert!(diag.message.contains("unreachable"));
    assert_span_on(src, "5", &diag);
}

#[test]
fn each_broken_function_yields_its_own_diagnostic() {
    // Per-declaration error collection: two bad bodies → two diagnostics.
    let src = "fn a(): Int = true\nfn b(): Bool = 1";
    assert_eq!(errors(src).len(), 2);
}

#[test]
fn measurement_synthesizes_the_quantum_monad() {
    // Issue #14: `measure(q) : Q<Bit>`. Used where the return type is `Q<Bit>` it checks; the
    // old `: Bit` form is now a `Q<Bit>` vs `Bit` mismatch, not an "unsupported" report.
    assert!(check_program("fn f(q: Qubit): Q<Bit> = measure(q)").is_ok());
    let diag = only_error("fn f(q: Qubit): Bit = measure(q)");
    assert!(
        diag.message.contains("Q<Bit>"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn a_circuit_used_where_a_scalar_is_expected_is_a_circuit_error() {
    // A `circuit { }` block no longer reports "unsupported"; against a non-circuit type it is
    // a structured type error.
    let diag = only_error("fn f(): Int = circuit { H @0 }");
    assert!(
        diag.message.contains("expected a circuit"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn bell_state_fixture_type_checks() {
    // The real reference fixture (not an inline copy) checks end-to-end through the full
    // parse → desugar → type-check pipeline (issue #14). `bell_state.qn` defines both
    // `bell_state` and `hello_bell`, so it is self-contained; `teleport.qn` references
    // `bell_state` from this file, so its end-to-end check is the inline test in the unit
    // suite (with `bell_state` in scope).
    let src = include_str!("fixtures/bell_state.qn");
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn type_alias_to_classical_type_resolves() {
    let src = "type Pair = (Int, Bool)\nfn f(p: Pair): Int = let (a, _) = p in a";
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn reference_algorithm_fixtures_type_check_via_facade() {
    for (file, src) in [
        ("bell_state", include_str!("fixtures/bell_state.qn")),
        ("grover", include_str!("fixtures/grover.qn")),
        // `shor.qn` excluded: its recursive `qft` kernel is a known gap (see SPEC §12
        // "Known gap"). It still lexes/parses in the `reference_algorithms` snapshot suite.
        (
            "error_correction",
            include_str!("fixtures/error_correction.qn"),
        ),
        ("qaoa", include_str!("fixtures/qaoa.qn")),
        (
            "bernstein_vazirani",
            include_str!("fixtures/bernstein_vazirani.qn"),
        ),
        ("ising", include_str!("fixtures/ising.qn")),
        ("stdlib_forms", include_str!("fixtures/stdlib_forms.qn")),
    ] {
        assert!(
            check_program(src).is_ok(),
            "{file}: {:?}",
            check_program(src).err()
        );
    }
    let teleport = concat!(
        include_str!("fixtures/bell_state.qn"),
        "\n",
        include_str!("fixtures/teleport.qn"),
    );
    assert!(
        check_program(teleport).is_ok(),
        "teleport: {:?}",
        check_program(teleport).err()
    );
}
