use std::path::Path;

use quonlint::{LintConfig, lint_source};

pub fn lint_snippet(src: &str) -> Vec<quonlint::LintDiagnostic> {
    lint_source(Path::new("test.qn"), src, &LintConfig::default())
}

#[allow(dead_code)]
pub fn lint_snippet_with_config(src: &str, config: &LintConfig) -> Vec<quonlint::LintDiagnostic> {
    lint_source(Path::new("test.qn"), src, config)
}

pub fn has_rule(diags: &[quonlint::LintDiagnostic], rule: &str) -> bool {
    diags.iter().any(|d| d.rule == rule)
}
