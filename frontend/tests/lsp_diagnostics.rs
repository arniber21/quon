//! Structured diagnostics and quick-fix tests (issue #44).

use frontend::diagnostics::{Diagnostic, DiagnosticCode, DiagnosticSeverity};
use frontend::{AnalysisResult, RichDiagnostic, analyze, apply_fixes};

fn diags(src: &str) -> Vec<RichDiagnostic> {
    analyze(src).diagnostics
}

fn assert_code(src: &str, code: &str) {
    let ds = diags(src);
    assert!(
        !ds.is_empty(),
        "expected at least one diagnostic for `{src}`"
    );
    assert!(
        ds.iter().any(|d| d.code.0 == code),
        "expected code `{code}` in {:?}",
        ds.iter().map(|d| d.code.0).collect::<Vec<_>>()
    );
}

fn assert_related_count(src: &str, n: usize) {
    let ds = diags(src);
    assert_eq!(
        ds.first().map(|d| d.related.len()).unwrap_or(0),
        n,
        "diagnostics: {ds:?}"
    );
}

fn first_with_code(src: &str, code: &str) -> RichDiagnostic {
    diags(src)
        .into_iter()
        .find(|d| d.code.0 == code)
        .unwrap_or_else(|| panic!("no diagnostic with code `{code}` for `{src}`"))
}

#[test]
fn rich_diagnostic_converts_to_legacy_diagnostic() {
    let rich = RichDiagnostic::new(
        DiagnosticCode::TYPE_MISMATCH,
        DiagnosticSeverity::Error,
        "type mismatch",
        (0..1).into(),
    );
    let legacy: Diagnostic = (&rich).into();
    assert_eq!(legacy.message, "type mismatch");
    assert_eq!(legacy.span, rich.span);
}

#[test]
fn lex_invalid_char_has_code() {
    let src = "a # b";
    assert_code(src, "quon.lex.invalid-char");
    let d = first_with_code(src, "quon.lex.invalid-char");
    let hash = src.find('#').unwrap();
    assert_eq!(d.span.start, hash);
}

#[test]
fn parse_error_has_code() {
    assert_code("fn f(): Int = )", "quon.parse.unexpected-token");
}

#[test]
fn linear_used_twice_related() {
    let src = "fn f(q: Qubit): QReg<2> = (q, q)";
    assert_code(src, "quon.linearity.used-twice");
    assert_related_count(src, 1);
}

#[test]
fn linear_unconsumed_borrow_fix() {
    let src = "fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}";
    let d = first_with_code(src, "quon.linearity.unconsumed");
    assert!(
        !d.fixes.is_empty(),
        "expected discard/reset quick fixes, got {:?}",
        d.fixes
    );
    let fixed = apply_fixes(src, &d.fixes);
    assert!(
        fixed.contains("discard(a)"),
        "fixed source should contain discard(a): {fixed}"
    );
}

#[test]
fn clifford_mismatch_fix() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }";
    let d = first_with_code(src, "quon.refinement.clifford-mismatch");
    assert!(
        d.fixes.iter().any(|f| f.title.contains("Universal")),
        "fixes: {:?}",
        d.fixes
    );
    let fixed = apply_fixes(src, &d.fixes);
    assert!(
        fixed.contains("Universal"),
        "fixed source should use Universal: {fixed}"
    );
    assert!(!fixed.contains("Clifford> = circuit"));
}

#[test]
fn depth_mismatch_constant_fix() {
    let src = "fn bell(): Circuit<2, 2, 1, Clifford> = circuit { H @0 |> CNOT @(0, 1) }";
    let d = first_with_code(src, "quon.refinement.depth-mismatch");
    assert!(
        !d.fixes.is_empty(),
        "expected depth quick fix, got {:?}",
        d.fixes
    );
    let fixed = apply_fixes(src, &d.fixes);
    assert!(
        fixed.contains("Circuit<2, 2, 2, Clifford>"),
        "fixed: {fixed}"
    );
}

#[test]
fn borrow_escape_no_fix() {
    let src = "fn f(): Q<Qubit> = run {\n  borrow a: Qubit in {\n    return a\n  }\n}";
    let d = first_with_code(src, "quon.borrow.escape");
    assert_eq!(d.fixes.len(), 0, "BorrowEscape must not offer auto-fixes");
    assert_eq!(d.related.len(), 1);
}

#[test]
fn partial_source_no_panic() {
    let snippets = [
        "",
        "fn f(): Cir",
        "fn f(): Int = (",
        "#",
        "fn f(): Int = ",
        "run { borrow a: Qubit in {",
        "ident_without_end",
    ];
    for src in snippets {
        let AnalysisResult { diagnostics } = analyze(src);
        let _ = diagnostics;
    }
}

#[test]
fn every_type_error_variant_has_unique_code() {
    use frontend::ast::CliffordClass;
    use frontend::typecheck::TypeError;
    use frontend::types::Ty;

    let span = (0..1).into();
    let errors = [
        TypeError::Mismatch {
            expected: Ty::Int,
            found: Ty::Bool,
            span,
        },
        TypeError::UnboundVariable {
            name: "x".into(),
            span,
        },
        TypeError::NotAFunction {
            found: Ty::Int,
            span,
        },
        TypeError::NotNumeric {
            found: Ty::Bool,
            span,
        },
        TypeError::ArityMismatch {
            expected: 2,
            found: 1,
            span,
        },
        TypeError::NonExhaustive {
            witness: "true".into(),
            span,
        },
        TypeError::UnreachableArm { span },
        TypeError::AmbiguousLambda { span },
        TypeError::OccursCheck { span },
        TypeError::AliasArity {
            name: "T".into(),
            expected: 1,
            found: 0,
            span,
        },
        TypeError::LinearUsedTwice {
            name: "q".into(),
            first: span,
            span,
        },
        TypeError::LinearUnconsumed {
            name: "q".into(),
            span,
        },
        TypeError::LinearBranchMismatch {
            name: "q".into(),
            span,
        },
        TypeError::LinearDiscard {
            name: "Qubit".into(),
            bound_name: None,
            binding_span: None,
            let_span: None,
            span,
        },
        TypeError::LinearCapture {
            name: "q".into(),
            span,
        },
        TypeError::NotACircuit {
            found: Ty::Int,
            span,
        },
        TypeError::QubitCountMismatch {
            expected: "1".into(),
            found: "2".into(),
            span,
        },
        TypeError::GateTargetArity {
            expected: 1,
            found: 2,
            span,
        },
        TypeError::IndexOutOfBounds {
            index: 1,
            width: 1,
            span,
        },
        TypeError::CliffordMismatch {
            expected: CliffordClass::Clifford,
            found: CliffordClass::Universal,
            span,
        },
        TypeError::DepthMismatch {
            expected: "1".into(),
            found: "2".into(),
            span,
        },
        TypeError::DepthIntractable {
            expr: "n".into(),
            span,
        },
        TypeError::ExpectedMonad {
            found: Ty::Int,
            span,
        },
        TypeError::BorrowEscape {
            name: "a".into(),
            span,
            borrow_span: span,
        },
        TypeError::NonDependentArg {
            func: "f".into(),
            param: "n".into(),
            span,
        },
        TypeError::IllFoundedRecursion {
            name: "f".into(),
            span,
        },
        TypeError::MutualRecursion {
            name: "f".into(),
            span,
        },
        TypeError::Unsupported {
            construct: "test",
            span,
        },
    ];
    let codes: Vec<_> = errors.iter().map(|e| e.code().0).collect();
    let unique: std::collections::BTreeSet<_> = codes.iter().copied().collect();
    assert_eq!(unique.len(), codes.len(), "duplicate codes among variants");
}

#[test]
fn linear_discard_fix_for_simple_let() {
    let src = "fn f(q: Qubit): Int = let _ = q in 0";
    let d = first_with_code(src, "quon.linearity.discard");
    assert!(
        d.fixes.iter().any(|f| f.title.contains("discard(q)")),
        "fixes: {:?}",
        d.fixes
    );
    let fixed = apply_fixes(src, &d.fixes);
    assert!(fixed.contains("discard(q)"), "{fixed}");
}
