use std::path::{Path, PathBuf};

use frontend::TypedProgram;
use frontend::analyze_program;

use crate::config::LintConfig;
use crate::context::LintContext;
use crate::diagnostic::{LintDiagnostic, Severity};
use crate::project::{LintError, discover_qn_files};
use crate::rules::register_rules;
use crate::suppressions::SuppressionState;

/// Lint a single source string. Returns empty on parse/type errors.
pub fn lint_source(path: &Path, src: &str, config: &LintConfig) -> Vec<LintDiagnostic> {
    let analysis = analyze_program(src);
    let Some(typed) = TypedProgram::from_analysis(&analysis) else {
        return Vec::new();
    };
    run_rules(path, src, &typed, config)
}

/// Lint multiple paths (files or directories).
pub fn lint_paths(
    paths: &[PathBuf],
    config: &LintConfig,
) -> Result<Vec<(PathBuf, Vec<LintDiagnostic>)>, LintError> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            files.extend(discover_qn_files(path, config)?);
        } else if path.extension().is_some_and(|e| e == "qn") {
            files.push(path.clone());
        }
    }
    files.sort();
    files.dedup();

    let mut results = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(&file).map_err(|e| LintError::Io {
            path: file.clone(),
            source: e,
        })?;
        let diags = lint_source(&file, &src, config);
        results.push((file, diags));
    }
    Ok(results)
}

/// Lint a project root using discovered config and `.qn` files.
pub fn lint_project(
    root: &Path,
    config: &LintConfig,
) -> Result<Vec<(PathBuf, Vec<LintDiagnostic>)>, LintError> {
    let files = discover_qn_files(root, config)?;
    let mut results = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(&file).map_err(|e| LintError::Io {
            path: file.clone(),
            source: e,
        })?;
        let file_config = if config.config_path.is_some() {
            config.clone()
        } else {
            LintConfig::discover_for_file(&file)
        };
        let diags = lint_source(&file, &src, &file_config);
        results.push((file, diags));
    }
    Ok(results)
}

fn run_rules(
    path: &Path,
    src: &str,
    typed: &TypedProgram,
    config: &LintConfig,
) -> Vec<LintDiagnostic> {
    let suppressions = SuppressionState::parse(src);
    let ctx = LintContext::new(src, path, typed, config, &suppressions);
    let rules = register_rules();
    let mut diags = Vec::new();
    for rule in rules {
        let mut emit = |d: LintDiagnostic| diags.push(d);
        rule.run(&ctx, &mut emit);
    }
    diags.sort_by_key(|d| (d.span.start, d.rule.clone()));
    diags.dedup_by(|a, b| a.rule == b.rule && a.span == b.span);
    diags
}

/// Whether any diagnostic meets the fail-on threshold.
pub fn has_failures(diags: &[LintDiagnostic], fail_on: Severity) -> bool {
    diags.iter().any(|d| d.severity.fails_build(fail_on))
}
