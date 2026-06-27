// Linear type-checker integration tests (issue #10): split context Γ;Δ, no-cloning,
// no-dropping, `destructure`, and branch-residual agreement. These drive the public
// `frontend::check_program` facade and focus on the acceptance criterion that linearity
// errors are *span-accurate* — the caret lands on the offending token.

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

/// Assert the diagnostic's span covers the *first* occurrence of `needle` in `src`.
fn assert_span_on_first(src: &str, needle: &str, diag: &Diagnostic) {
    let start = src
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in source"));
    assert_span(src, start, needle, diag);
}

/// Assert the diagnostic's span covers the *last* occurrence of `needle` in `src` — used
/// when the offending site is a repeated name (e.g. the second use of a consumed qubit).
fn assert_span_on_last(src: &str, needle: &str, diag: &Diagnostic) {
    let start = src
        .rfind(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in source"));
    assert_span(src, start, needle, diag);
}

fn assert_span(src: &str, start: usize, needle: &str, diag: &Diagnostic) {
    let end = start + needle.len();
    let span = diag.span;
    assert_eq!(
        (span.start, span.end),
        (start, end),
        "diagnostic `{}` span {:?} did not land on `{needle}` ({start}..{end}) in `{src}`",
        diag.message,
        span,
    );
}

#[test]
fn well_typed_linear_programs_are_accepted() {
    // A register destructured, both qubits threaded back out — every resource consumed once.
    let src = "fn f(q: QReg<2>): QReg<2> = let (a, b) = destructure(q) in (a, b)";
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn using_a_qubit_twice_points_at_the_second_use() {
    let src = "fn f(q: Qubit): QReg<2> = (q, q)";
    let diag = only_error(src);
    assert!(
        diag.message.contains("used more than once"),
        "message was: {}",
        diag.message
    );
    // The caret lands on the *second* `q` — the one that violates linearity.
    assert_span_on_last(src, "q", &diag);
}

#[test]
fn dropping_a_qubit_points_at_its_binding() {
    let src = "fn f(q: Qubit): Int = 0";
    let diag = only_error(src);
    assert!(
        diag.message.contains("never consumed"),
        "message was: {}",
        diag.message
    );
    // The binding is the parameter's type annotation.
    assert_span_on_first(src, "Qubit", &diag);
}

#[test]
fn discarding_a_qubit_with_wildcard_is_reported() {
    let src = "fn f(q: Qubit): Int = let _ = q in 0";
    let diag = only_error(src);
    assert!(
        diag.message.contains("discard"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn branch_residual_mismatch_points_at_the_offending_branch() {
    let src = "fn f(c: Bool, q: Qubit, q2: Qubit): Qubit = if c then q else q2";
    let diag = only_error(src);
    assert!(
        diag.message.contains("not all"),
        "message was: {}",
        diag.message
    );
    // `then` spends `q`; the `else` branch leaves it unmatched — caret on `else q2`.
    assert_span_on_last(src, "q2", &diag);
}

#[test]
fn capturing_a_linear_resource_in_a_closure_is_reported() {
    let src = "fn f(q: Qubit): Int -> Qubit = fn(x: Int) -> q";
    let diag = only_error(src);
    assert!(
        diag.message.contains("capture"),
        "message was: {}",
        diag.message
    );
    assert_span_on_last(src, "q", &diag);
}
