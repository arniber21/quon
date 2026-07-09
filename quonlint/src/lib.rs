//! Quon source linter — experiment quality and ergonomics rules.

mod config;
mod context;
mod diagnostic;
mod engine;
mod project;
mod reporter;
mod rules;
mod suppressions;

pub use config::LintConfig;
pub use diagnostic::{LintDiagnostic, RuleId, Severity};
pub use engine::{has_failures, lint_paths, lint_project, lint_source};
pub use project::LintError;
pub use reporter::span;
pub use reporter::{JsonOutput, diagnostics_to_lsp, report_github, report_human, write_json};
pub use rules::{all_rule_ids, register_rules};
