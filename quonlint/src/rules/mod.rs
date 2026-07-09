mod ancilla;
mod depth;
mod gates;
mod monad;

use crate::context::LintContext;
use crate::diagnostic::{LintDiagnostic, RuleId, Severity};

pub trait LintRule: Send + Sync {
    fn id(&self) -> RuleId;
    fn default_severity(&self) -> Severity;
    fn description(&self) -> &'static str;
    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic));
}

pub fn register_rules() -> Vec<Box<dyn LintRule>> {
    vec![
        Box::new(depth::SequentialForBlowup),
        Box::new(depth::UnsoundDepthAnnotation),
        Box::new(depth::RepeatNonLiteralCount),
        Box::new(depth::ControlledChain),
        Box::new(gates::UniversalInCliffordBlock),
        Box::new(gates::NonNativeDensity),
        Box::new(gates::ConsecutiveRotations),
        Box::new(gates::SwapInSource),
        Box::new(ancilla::DiscardInBorrow),
        Box::new(ancilla::NestedBorrow),
        Box::new(ancilla::UnmeasuredAncillaOutput),
        Box::new(monad::CircuitBindWithoutApply),
        Box::new(monad::NestedRunBlock),
        Box::new(monad::UnusedMeasurement),
    ]
}

pub fn all_rule_ids() -> Vec<RuleId> {
    register_rules().into_iter().map(|r| r.id()).collect()
}

pub(crate) fn emit_if_allowed(
    ctx: &LintContext<'_>,
    rule: &str,
    default_severity: Severity,
    diag: LintDiagnostic,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    if !ctx.config.rule_enabled(&rule.to_string()) {
        return;
    }
    let severity = ctx
        .config
        .effective_severity(&rule.to_string(), default_severity);
    if severity == Severity::Allow {
        return;
    }
    if !severity.is_emitted(ctx.config.min_severity) {
        return;
    }
    if ctx
        .suppressions
        .is_suppressed_with_source(rule, diag.span.start, ctx.source)
    {
        return;
    }
    emit(LintDiagnostic { severity, ..diag });
}
