use std::path::PathBuf;

use quonlint::{LintConfig, Severity, has_failures, lint_project};

#[test]
fn project_lint_fixtures_no_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let config = LintConfig::discover_project(&root).with_fail_on(Severity::Error);
    let results = lint_project(&root, &config).expect("project lint");
    assert!(!results.is_empty(), "expected .qn files under project root");
    let all: Vec<_> = results.iter().flat_map(|(_, d)| d.iter()).collect();
    assert!(
        !all.iter().any(|d| d.severity == Severity::Error),
        "unexpected lint errors: {:?}",
        all.iter()
            .filter(|d| d.severity == Severity::Error)
            .collect::<Vec<_>>()
    );
}

#[test]
fn corpus_recursive_qft_no_unsound_depth() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/tests/fixtures/corpus/recursive_qft.qn");
    let src = std::fs::read_to_string(&path).unwrap();
    let diags = quonlint::lint_source(&path, &src, &LintConfig::default());
    assert!(
        !diags
            .iter()
            .any(|d| d.rule == "depth/unsound-depth-annotation")
    );
}

#[test]
fn fail_on_error_passes_on_clean_fixture() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/tests/fixtures/bell_state.qn");
    let src = std::fs::read_to_string(&path).unwrap();
    let diags = quonlint::lint_source(&path, &src, &LintConfig::default());
    assert!(!has_failures(&diags, Severity::Error));
}

#[test]
fn ci_corpus_warn_baseline() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest = root.join("test/tooling/ci-corpus.txt");
    let text = std::fs::read_to_string(&manifest).expect("read corpus");
    let config = LintConfig::discover_project(&root).with_fail_on(Severity::Warning);
    let mut warn_count = 0usize;
    for line in text.lines() {
        let path = line.split('#').next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let full = root.join(path);
        let src = std::fs::read_to_string(&full).expect("read fixture");
        let diags = quonlint::lint_source(&full, &src, &config);
        warn_count += diags
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
    }
    // Ratchet: CI corpus should not accumulate warn-level lint debt beyond baseline.
    assert!(
        warn_count <= 12,
        "warn count {warn_count} exceeds baseline (update baseline intentionally if expected)"
    );
}

#[test]
fn fail_on_warn_detects_info_when_elevated() {
    let src = r#"fn f(n: Nat): Circuit<n, n, _, Universal> = circuit {
        for i in range(n) { H @0 }
    }"#;
    let mut config = LintConfig::default();
    config
        .rule_severities
        .insert("depth/sequential-for-blowup".into(), Severity::Warning);
    let diags = quonlint::lint_source(PathBuf::from("t.qn").as_path(), src, &config);
    assert!(has_failures(&diags, Severity::Warning));
}
