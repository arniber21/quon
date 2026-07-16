//! Fused QEC validation report (#280 / ADR-0020 amendment).
//!
//! A [`ValidationReport`] is a **new, separate** compiler artifact
//! (`*.validation.json` / `*.validation.md`) that places the analytic compiler
//! [`ResourceReport`](quon_na::ResourceReport) beside sampled Stim/Sinter
//! evidence, with clear provenance. It is **not** a mutation of the primary
//! `ResourceReport` DTO and **not** a threshold claim: the two evidence kinds
//! stay in clearly labeled `analytic` and `sampled` sections (ADR-0020).
//!
//! The sampled section is deserialized from the structured JSON emitted by
//! `python/quon_qec_sinter.py --json` (mirrored here as [`SampledEvidence`]).
//! Before fusing, [`fuse`] compares a provenance fingerprint (experiment SHA-256
//! plus family / distance / rounds) and refuses — or, with `allow_mismatch`,
//! warns — when the sampled data was produced against an incompatible artifact.

use quon_na::ResourceReport;
use quon_qec::{ErrorModelSnapshot, QecExperiment};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Wire schema version for `*.validation.json`.
pub const VALIDATION_SCHEMA_VERSION: u32 = 1;

/// Discriminator for the fused validation report envelope.
pub const VALIDATION_KIND: &str = "qec_validation_report";

/// Sampled-evidence schema version accepted from the Python harness.
pub const SAMPLED_SCHEMA_VERSION: u32 = 1;

/// `evidence_kind` value for the analytic section.
pub const ANALYTIC_EVIDENCE_KIND: &str = "analytic";

/// `evidence_kind` value for the sampled section.
pub const SAMPLED_EVIDENCE_KIND: &str = "sampled";

/// Top-level disclaimer embedded in the fused report.
pub const VALIDATION_DISCLAIMER: &str = "Fused QEC validation report: analytic compiler estimates and sampled Stim/Sinter evidence side by side, with provenance. Sampled results are validation evidence, not a threshold claim; the two evidence kinds remain in separate labeled sections (ADR-0020).";

/// Analytic-section disclaimer (compiler estimates, not sampled).
pub const ANALYTIC_DISCLAIMER: &str = "Analytic compiler estimates (schedule metrics + physical error budget = rate × counts). Not sampled logical failures and not a threshold claim (ADR-0017 / ADR-0020).";

/// Sampled-section disclaimer (Stim/Sinter evidence, not a claim).
pub const SAMPLED_DISCLAIMER: &str = "Sampled Stim/Sinter logical failures under the JSON error_model noise assumptions. Validation evidence only — not a threshold claim (ADR-0020 / ADR-0024).";

/// Failures fusing sampled evidence with an analytic artifact.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("sampled evidence contains no experiments to fuse")]
    NoSampledExperiments,
    #[error("unsupported sampled schema_version {got} (expected {expected})")]
    UnsupportedSampledSchema { got: u32, expected: u32 },
    #[error("sampled document evidence_kind must be \"sampled\" (got {got:?})")]
    NotSampled { got: String },
    #[error(
        "sampled data does not match the compiled QEC artifact — refusing to fuse \
         incompatible evidence (pass --allow-sampled-mismatch to downgrade to a \
         warning):\n{details}"
    )]
    ProvenanceMismatch { details: String },
}

/// Wilson-score confidence interval for a sampled logical failure rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfidenceInterval {
    pub low: f64,
    pub high: f64,
    pub level: f64,
    pub method: String,
}

/// One sampled point (fixed shots / error scale) for an experiment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampledResult {
    pub shots: u64,
    pub error_scale: f64,
    /// Noise assumptions actually sampled (post error-scale).
    pub noise_model: ErrorModelSnapshot,
    pub logical_failures: u64,
    pub logical_failure_rate: f64,
    pub confidence_interval: ConfidenceInterval,
}

/// Sampled evidence for a single `*.qec.json` experiment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampledExperiment {
    /// Path passed to the harness (informational).
    pub experiment: String,
    /// SHA-256 of the `*.qec.json` bytes the harness sampled.
    pub experiment_sha256: String,
    pub family: String,
    pub code_family: String,
    pub distance: u32,
    pub rounds: u32,
    /// Logical observable names (id / logical id / basis).
    pub logical_observables: Vec<String>,
    pub results: Vec<SampledResult>,
}

/// Structured sampled-evidence document from `quon_qec_sinter.py --json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampledEvidence {
    pub schema_version: u32,
    pub evidence_kind: String,
    pub disclaimer: String,
    pub decoder: String,
    pub seed: i64,
    pub tick_us: f64,
    pub confidence_level: f64,
    pub experiments: Vec<SampledExperiment>,
}

/// Provenance fingerprint tying the sampled evidence to the compiled artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Compiled `.qn` source path.
    pub source: String,
    /// Backend target id used for compilation.
    pub target_id: String,
    pub family: String,
    pub code_family: String,
    pub distance: u32,
    pub rounds: u32,
    pub logical_ids: Vec<u32>,
    /// SHA-256 of the emitted `*.qec.json` bytes.
    pub experiment_sha256: String,
    /// Sibling structure-level Stim basename.
    pub stim_file: String,
}

impl Provenance {
    /// Build provenance from the emitted experiment + compile context.
    pub fn from_experiment(
        experiment: &QecExperiment,
        source: String,
        target_id: String,
        experiment_sha256: String,
    ) -> Self {
        Self {
            source,
            target_id,
            family: experiment.family.clone(),
            code_family: experiment.code_family.clone(),
            distance: experiment.distance,
            rounds: experiment.rounds,
            logical_ids: experiment.logical_ids.clone(),
            experiment_sha256,
            stim_file: experiment.stim_file.clone(),
        }
    }
}

/// Analytic section: the primary compiler [`ResourceReport`], unmutated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyticSection {
    pub evidence_kind: String,
    pub disclaimer: String,
    pub resource_report: ResourceReport,
}

/// Sampled section: Stim/Sinter evidence with decoder + noise metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampledSection {
    pub evidence_kind: String,
    pub disclaimer: String,
    pub decoder: String,
    pub seed: i64,
    pub tick_us: f64,
    pub confidence_level: f64,
    pub experiments: Vec<SampledExperiment>,
}

/// Fused validation report (`*.validation.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationReport {
    pub schema_version: u32,
    pub kind: String,
    pub disclaimer: String,
    pub provenance: Provenance,
    pub analytic: AnalyticSection,
    pub sampled: SampledSection,
    /// Non-fatal provenance discrepancies (only when `--allow-sampled-mismatch`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mismatch_warnings: Vec<String>,
}

/// Field-level provenance discrepancies between the artifact and sampled data.
fn provenance_discrepancies(prov: &Provenance, exp: &SampledExperiment) -> Vec<String> {
    let mut out = Vec::new();
    if prov.experiment_sha256 != exp.experiment_sha256 {
        out.push(format!(
            "experiment_sha256: artifact {} != sampled {}",
            prov.experiment_sha256, exp.experiment_sha256
        ));
    }
    if prov.family != exp.family {
        out.push(format!(
            "family: artifact {:?} != sampled {:?}",
            prov.family, exp.family
        ));
    }
    if prov.code_family != exp.code_family {
        out.push(format!(
            "code_family: artifact {:?} != sampled {:?}",
            prov.code_family, exp.code_family
        ));
    }
    if prov.distance != exp.distance {
        out.push(format!(
            "distance: artifact {} != sampled {}",
            prov.distance, exp.distance
        ));
    }
    if prov.rounds != exp.rounds {
        out.push(format!(
            "rounds: artifact {} != sampled {}",
            prov.rounds, exp.rounds
        ));
    }
    out
}

/// Fuse analytic + sampled evidence into a labeled [`ValidationReport`].
///
/// Refuses on provenance mismatch unless `allow_mismatch` is set (then the
/// discrepancies are recorded in [`ValidationReport::mismatch_warnings`]).
pub fn fuse(
    provenance: Provenance,
    resource_report: ResourceReport,
    sampled: SampledEvidence,
    allow_mismatch: bool,
) -> Result<ValidationReport, ValidationError> {
    if sampled.schema_version != SAMPLED_SCHEMA_VERSION {
        return Err(ValidationError::UnsupportedSampledSchema {
            got: sampled.schema_version,
            expected: SAMPLED_SCHEMA_VERSION,
        });
    }
    if sampled.evidence_kind != SAMPLED_EVIDENCE_KIND {
        return Err(ValidationError::NotSampled {
            got: sampled.evidence_kind,
        });
    }
    if sampled.experiments.is_empty() {
        return Err(ValidationError::NoSampledExperiments);
    }

    // Prefer the experiment whose fingerprint matches; else fall back to the
    // first and report the discrepancy against it.
    let chosen = sampled
        .experiments
        .iter()
        .find(|e| e.experiment_sha256 == provenance.experiment_sha256)
        .unwrap_or(&sampled.experiments[0]);
    let discrepancies = provenance_discrepancies(&provenance, chosen);

    let mismatch_warnings = if discrepancies.is_empty() {
        Vec::new()
    } else if allow_mismatch {
        discrepancies
    } else {
        return Err(ValidationError::ProvenanceMismatch {
            details: discrepancies
                .iter()
                .map(|d| format!("  - {d}"))
                .collect::<Vec<_>>()
                .join("\n"),
        });
    };

    Ok(ValidationReport {
        schema_version: VALIDATION_SCHEMA_VERSION,
        kind: VALIDATION_KIND.to_string(),
        disclaimer: VALIDATION_DISCLAIMER.to_string(),
        provenance,
        analytic: AnalyticSection {
            evidence_kind: ANALYTIC_EVIDENCE_KIND.to_string(),
            disclaimer: ANALYTIC_DISCLAIMER.to_string(),
            resource_report,
        },
        sampled: SampledSection {
            evidence_kind: SAMPLED_EVIDENCE_KIND.to_string(),
            disclaimer: SAMPLED_DISCLAIMER.to_string(),
            decoder: sampled.decoder,
            seed: sampled.seed,
            tick_us: sampled.tick_us,
            confidence_level: sampled.confidence_level,
            experiments: sampled.experiments,
        },
        mismatch_warnings,
    })
}

/// Pretty JSON for a fused validation report (stable field order).
pub fn validation_report_to_json(report: &ValidationReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// Deterministic Markdown rendering of a fused validation report.
pub fn validation_report_to_markdown(report: &ValidationReport) -> String {
    let mut out = String::new();
    out.push_str("# QEC validation report\n\n");
    out.push_str("> ");
    out.push_str(&report.disclaimer);
    out.push_str("\n\n");

    if !report.mismatch_warnings.is_empty() {
        out.push_str("## ⚠ Provenance warnings\n\n");
        out.push_str(
            "Sampled evidence was attached despite a fingerprint mismatch \
             (`--allow-sampled-mismatch`):\n\n",
        );
        for w in &report.mismatch_warnings {
            out.push_str(&format!("- {w}\n"));
        }
        out.push('\n');
    }

    let p = &report.provenance;
    out.push_str("## Provenance\n\n");
    out.push_str("| Field | Value |\n| --- | --- |\n");
    out.push_str(&format!("| Source | `{}` |\n", p.source));
    out.push_str(&format!("| Target | `{}` |\n", p.target_id));
    out.push_str(&format!("| Family | {} |\n", p.family));
    out.push_str(&format!("| Code family | {} |\n", p.code_family));
    out.push_str(&format!("| Distance | {} |\n", p.distance));
    out.push_str(&format!("| Rounds | {} |\n", p.rounds));
    out.push_str(&format!(
        "| Logical ids | {} |\n",
        p.logical_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push_str(&format!(
        "| Experiment SHA-256 | `{}` |\n",
        p.experiment_sha256
    ));
    out.push_str(&format!("| Stim file | `{}` |\n\n", p.stim_file));

    let a = &report.analytic;
    let rr = &a.resource_report;
    out.push_str("## Analytic evidence\n\n");
    out.push_str(&format!("*evidence_kind: `{}`*\n\n", a.evidence_kind));
    out.push_str(&format!("{}\n\n", a.disclaimer));
    out.push_str("| Metric | Value |\n| --- | ---: |\n");
    out.push_str(&format!("| Logical qubits | {} |\n", rr.logical_qubits));
    out.push_str(&format!("| Physical atoms | {} |\n", rr.physical_atoms));
    out.push_str(&format!("| Estimated cycles | {} |\n", rr.estimated_cycles));
    out.push_str(&format!("| Bottleneck | {} |\n", rr.bottleneck.as_str()));
    out.push_str(&format!("| Rydberg stages | {} |\n", rr.rydberg_stages));
    out.push_str(&format!(
        "| Measurement rounds | {} |\n",
        rr.measurement_rounds
    ));
    if let Some(budget) = &rr.error_budget {
        out.push_str(&format!(
            "| Error budget (rydberg / measurement / reset) | {} / {} / {} |\n",
            budget.rydberg, budget.measurement, budget.reset
        ));
        out.push_str(&format!(
            "| Error budget (movement / transfer / idle) | {} / {} / {} |\n",
            budget.movement, budget.transfer, budget.idle
        ));
    }
    out.push('\n');

    let s = &report.sampled;
    out.push_str("## Sampled evidence\n\n");
    out.push_str(&format!("*evidence_kind: `{}`*\n\n", s.evidence_kind));
    out.push_str(&format!("{}\n\n", s.disclaimer));
    out.push_str(&format!("- Decoder: `{}`\n", s.decoder));
    out.push_str(&format!("- Seed: {}\n", s.seed));
    out.push_str(&format!("- Tick µs (idle proxy): {}\n", s.tick_us));
    out.push_str(&format!("- Confidence level: {}\n\n", s.confidence_level));

    for exp in &s.experiments {
        out.push_str(&format!(
            "### {} d={} rounds={}\n\n",
            exp.family, exp.distance, exp.rounds
        ));
        out.push_str(&format!(
            "- Logical observables: {}\n\n",
            if exp.logical_observables.is_empty() {
                "(none)".to_string()
            } else {
                exp.logical_observables.join(", ")
            }
        ));
        out.push_str(
            "| Shots | Error scale | Logical failures | Failure rate | 95% CI (Wilson) |\n",
        );
        out.push_str("| ---: | ---: | ---: | ---: | :--- |\n");
        for r in &exp.results {
            out.push_str(&format!(
                "| {} | {} | {} | {} | [{:.4}, {:.4}] |\n",
                r.shots,
                r.error_scale,
                r.logical_failures,
                r.logical_failure_rate,
                r.confidence_interval.low,
                r.confidence_interval.high,
            ));
        }
        out.push('\n');
    }

    out.push_str("## Notes\n\n");
    out.push_str(
        "- Analytic and sampled sections are different kinds of evidence and are kept \
         separate (ADR-0020). Neither is a threshold claim.\n",
    );
    out.push_str(
        "- Sampled logical failure rates come from Stim/Sinter under the JSON `error_model` \
         noise proxies (ADR-0024), not from schedule wall-clock time.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quon_na::ResourceReport;

    fn sample_result() -> SampledResult {
        SampledResult {
            shots: 64,
            error_scale: 1.0,
            noise_model: ErrorModelSnapshot {
                rydberg: 0.002,
                measurement: 0.003,
                reset: 0.004,
                movement: 0.0005,
                transfer: 0.0007,
                idle_per_us: 2e-9,
            },
            logical_failures: 1,
            logical_failure_rate: 1.0 / 64.0,
            confidence_interval: ConfidenceInterval {
                low: 0.0,
                high: 0.08,
                level: 0.95,
                method: "wilson".to_string(),
            },
        }
    }

    fn sampled_experiment(sha: &str) -> SampledExperiment {
        SampledExperiment {
            experiment: "rep_d3.qec.json".to_string(),
            experiment_sha256: sha.to_string(),
            family: "repetition".to_string(),
            code_family: "repetition_code_toy".to_string(),
            distance: 3,
            rounds: 2,
            logical_observables: vec!["obs0:L0:z".to_string()],
            results: vec![sample_result()],
        }
    }

    fn sampled_evidence(sha: &str) -> SampledEvidence {
        SampledEvidence {
            schema_version: SAMPLED_SCHEMA_VERSION,
            evidence_kind: SAMPLED_EVIDENCE_KIND.to_string(),
            disclaimer: "not a threshold claim".to_string(),
            decoder: "pymatching".to_string(),
            seed: 7,
            tick_us: 1.0,
            confidence_level: 0.95,
            experiments: vec![sampled_experiment(sha)],
        }
    }

    fn provenance(sha: &str) -> Provenance {
        Provenance {
            source: "rep.qn".to_string(),
            target_id: "generic_rna_v0".to_string(),
            family: "repetition".to_string(),
            code_family: "repetition_code_toy".to_string(),
            distance: 3,
            rounds: 2,
            logical_ids: vec![0],
            experiment_sha256: sha.to_string(),
            stim_file: "rep_d3.stim".to_string(),
        }
    }

    #[test]
    fn fuse_success_keeps_sections_separate() {
        let report = fuse(
            provenance("abc"),
            ResourceReport::default(),
            sampled_evidence("abc"),
            false,
        )
        .expect("fuse");
        assert_eq!(report.kind, VALIDATION_KIND);
        assert_eq!(report.analytic.evidence_kind, "analytic");
        assert_eq!(report.sampled.evidence_kind, "sampled");
        assert!(report.mismatch_warnings.is_empty());
        assert_eq!(report.sampled.experiments.len(), 1);
        assert_eq!(report.sampled.decoder, "pymatching");

        let json = validation_report_to_json(&report).expect("json");
        assert!(json.contains("\"analytic\""));
        assert!(json.contains("\"sampled\""));
        assert!(json.contains("not a threshold claim"));
        let back: ValidationReport = serde_json::from_str(&json).expect("round-trip");
        assert_eq!(back, report);
    }

    #[test]
    fn fuse_refuses_on_sha_mismatch() {
        let err = fuse(
            provenance("abc"),
            ResourceReport::default(),
            sampled_evidence("def"),
            false,
        )
        .expect_err("mismatch");
        match err {
            ValidationError::ProvenanceMismatch { details } => {
                assert!(details.contains("experiment_sha256"), "{details}");
            }
            other => panic!("expected mismatch, got {other:?}"),
        }
    }

    #[test]
    fn fuse_refuses_on_distance_mismatch() {
        let mut evidence = sampled_evidence("abc");
        evidence.experiments[0].distance = 5;
        let err = fuse(
            provenance("abc"),
            ResourceReport::default(),
            evidence,
            false,
        )
        .expect_err("mismatch");
        assert!(matches!(err, ValidationError::ProvenanceMismatch { .. }));
    }

    #[test]
    fn fuse_allows_mismatch_as_warning() {
        let report = fuse(
            provenance("abc"),
            ResourceReport::default(),
            sampled_evidence("def"),
            true,
        )
        .expect("fuse with allow");
        assert!(!report.mismatch_warnings.is_empty());
        assert!(
            report.mismatch_warnings[0].contains("experiment_sha256"),
            "{:?}",
            report.mismatch_warnings
        );
    }

    #[test]
    fn fuse_rejects_non_sampled_document() {
        let mut evidence = sampled_evidence("abc");
        evidence.evidence_kind = "analytic".to_string();
        let err = fuse(
            provenance("abc"),
            ResourceReport::default(),
            evidence,
            false,
        )
        .expect_err("kind");
        assert!(matches!(err, ValidationError::NotSampled { .. }));
    }

    #[test]
    fn fuse_rejects_unsupported_schema() {
        let mut evidence = sampled_evidence("abc");
        evidence.schema_version = 99;
        let err = fuse(
            provenance("abc"),
            ResourceReport::default(),
            evidence,
            false,
        )
        .expect_err("schema");
        assert!(matches!(
            err,
            ValidationError::UnsupportedSampledSchema {
                got: 99,
                expected: 1
            }
        ));
    }

    #[test]
    fn fuse_rejects_empty_experiments() {
        let mut evidence = sampled_evidence("abc");
        evidence.experiments.clear();
        let err = fuse(
            provenance("abc"),
            ResourceReport::default(),
            evidence,
            false,
        )
        .expect_err("empty");
        assert!(matches!(err, ValidationError::NoSampledExperiments));
    }

    #[test]
    fn sampled_evidence_rejects_unknown_fields() {
        let json = r#"{
            "schema_version": 1,
            "evidence_kind": "sampled",
            "disclaimer": "x",
            "decoder": "pymatching",
            "seed": 7,
            "tick_us": 1.0,
            "confidence_level": 0.95,
            "experiments": [],
            "bonus": true
        }"#;
        assert!(serde_json::from_str::<SampledEvidence>(json).is_err());
    }

    #[test]
    fn markdown_has_both_sections_and_disclaimer() {
        let report = fuse(
            provenance("abc"),
            ResourceReport::default(),
            sampled_evidence("abc"),
            false,
        )
        .expect("fuse");
        let md = validation_report_to_markdown(&report);
        assert!(md.contains("## Analytic evidence"));
        assert!(md.contains("## Sampled evidence"));
        assert!(md.contains("not a threshold claim"));
        assert!(md.contains("pymatching"));
        assert!(md.contains("Wilson"));
    }
}
