//! QEC experiment dual-emit and validation-report fusion.
//!
//! [`emit_qec_experiment_artifacts`] writes the `*.qec.json` + sibling `.stim`
//! pair with the same atomic order as the CLI (Stim first, JSON second, Stim
//! removed if JSON fails). [`emit_qec_validation`] reuses that pair, attaches
//! the analytic resource report, fuses provenance-checked sampled evidence, and
//! writes a separate `*.validation.json` + `.md`.
//!
//! Stim/Sinter subprocess launch stays in the binary. Pass a [`SinterSampler`]
//! that runs only when no pre-sampled JSON is attached.

use std::path::{Path, PathBuf};

use anstyle::Style;
use anyhow::{Context as _, Result, anyhow, bail};
use backend::TargetKind;
use quon_na::{
    NeutralAtomAction, ScheduleLayer, attach_qec_error_budget, require_target_error_model,
    resource_report_to_json, round_barrier_cuts,
};
use quon_qec::{
    attach_barrier_cycles, dual_emit, expand_workload, experiment_to_json, na_refs_from_expanded,
    sibling_stim_path,
};
use sha2::{Digest, Sha256};

use crate::compile::{CompileReport, CompileRequest};
use crate::emit::write_atomic;
use crate::validation::{
    Provenance, SampledEvidence, fuse, validation_report_to_json, validation_report_to_markdown,
};

/// Flags for `--emit-qec-validation` that are not the Stim/Sinter subprocess.
#[derive(Clone, Copy, Debug)]
pub struct QecValidationOptions<'a> {
    /// Pre-sampled evidence JSON. When set, [`SinterSampler`] is not called.
    pub attach_sampled: Option<&'a Path>,
    /// Warn instead of refusing when sampled provenance does not match.
    pub allow_sampled_mismatch: bool,
    /// Suppress the success note. Mismatch warnings still print.
    pub quiet: bool,
}

/// Python Stim/Sinter sampling. The quonc binary owns interpreter resolution
/// and the subprocess; this library only decides when sampling runs.
pub trait SinterSampler {
    fn sample(&self, qec_json: &Path, sampled_json: &Path) -> Result<()>;
}

/// Dual-emit `*.qec.json` + sibling `.stim` from the same expanded QEC IR (ADR-0018).
pub fn emit_qec_experiment_artifacts(
    request: &CompileRequest,
    report: &CompileReport,
    json_path: &Path,
) -> Result<()> {
    build_and_write_qec_experiment(request, report, json_path)?;
    Ok(())
}

/// Dual-emit the QEC experiment pair and return the semantic experiment DTO.
///
/// Shared by `--emit-qec-experiment` and `--emit-qec-validation` (#280) so the
/// dual-emit contract (ADR-0018) has a single source of truth.
pub fn build_and_write_qec_experiment(
    request: &CompileRequest,
    report: &CompileReport,
    json_path: &Path,
) -> Result<quon_qec::QecExperiment> {
    let workload = report.qec_workload.as_ref().ok_or_else(|| {
        anyhow!(
            "--emit-qec-experiment requires a QEC-backed program (e.g. repetition_code / \
             memory_round); bare-qubit NA programs have no experiment IR"
        )
    })?;
    let na = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na,
        _ => bail!(
            "--emit-qec-experiment requires a neutral_atom_reconfigurable target \
             (see targets/neutral_atom/)"
        ),
    };
    // ADR-0017: hard-fail when error_model is missing (never invent rates).
    // Snapshot type is unified with quon_qec::ErrorModelSnapshot (backend alias).
    let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
    let error_model = model.error_model_snapshot();

    // Re-expand from the same in-memory workload IR (never re-parse quantum.na).
    let expanded =
        expand_workload(workload).map_err(|e| anyhow!("QEC expand for experiment: {e}"))?;
    let stim_path = sibling_stim_path(json_path);
    let stim_basename = stim_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("experiment.stim")
        .to_string();

    let mut na_refs = na_refs_from_expanded(&expanded);
    if let Some(layers) = &report.na_schedule {
        let barriers = memory_round_barrier_cycles(layers, expanded.barrier_round_count())?;
        attach_barrier_cycles(&mut na_refs, &barriers).map_err(|e| anyhow!("{e}"))?;
    }

    let (experiment, stim) =
        dual_emit(&expanded, error_model, &stim_basename, na_refs).map_err(|e| anyhow!("{e}"))?;
    let json = experiment_to_json(&experiment).map_err(|e| anyhow!("{e}"))?;

    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = stim_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut json_body = json;
    if !json_body.ends_with('\n') {
        json_body.push('\n');
    }
    let mut stim_body = stim;
    if !stim_body.ends_with('\n') {
        stim_body.push('\n');
    }

    // Atomic dual write: Stim first, then JSON; clean up Stim if JSON fails.
    write_atomic(&stim_path, &stim_body)
        .with_context(|| format!("write QEC Stim circuit {}", stim_path.display()))?;
    if let Err(e) = write_atomic(json_path, &json_body)
        .with_context(|| format!("write QEC experiment JSON {}", json_path.display()))
    {
        let _ = std::fs::remove_file(&stim_path);
        return Err(e);
    }

    Ok(experiment)
}

/// Compiler-driven QEC validation report (#280 / ADR-0020 amendment).
///
/// One user-facing entry point runs: compile (already done) → QEC experiment
/// dual-emit → analytic resource report → Stim/Sinter sampling (Python) →
/// provenance-checked fusion into a **separate** `*.validation.json` + `.md`.
/// Primary artifacts (QEC pair, resource report, sampled JSON) are kept beside
/// the report so evidence kinds stay separate (ADR-0020).
///
/// `warning` and `ok` are the CLI's stderr styles so color stays owned by the
/// binary's `--color` choice.
pub fn emit_qec_validation(
    request: &CompileRequest,
    report: &CompileReport,
    validation_path: &Path,
    options: &QecValidationOptions<'_>,
    warning: Style,
    ok: Style,
    sampler: &impl SinterSampler,
) -> Result<()> {
    let base = validation_base(validation_path);
    let qec_json_path = with_suffix(&base, ".qec.json");
    let resource_report_path = with_suffix(&base, ".resource_report.json");
    let sampled_path = with_suffix(&base, ".sampled.json");
    let markdown_path = with_suffix(&base, ".validation.md");

    // 1. QEC experiment dual-emit (analytic structure + sibling .stim).
    let experiment = build_and_write_qec_experiment(request, report, &qec_json_path)?;
    let qec_bytes = std::fs::read(&qec_json_path)
        .with_context(|| format!("read QEC experiment {}", qec_json_path.display()))?;
    let experiment_sha256 = sha256_hex_bytes(&qec_bytes);

    // 2. Analytic resource report (attach physical error budget, ADR-0017).
    let na = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na,
        _ => bail!(
            "--emit-qec-validation requires a neutral_atom_reconfigurable target \
             (see targets/neutral_atom/)"
        ),
    };
    let resource_report = report.resource_report.as_ref().ok_or_else(|| {
        anyhow!("no resource report available (compile with a neutral-atom target)")
    })?;
    let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
    let resource_report = attach_qec_error_budget(resource_report.clone(), Some(model))
        .map_err(|e| anyhow!("{e}"))?;
    let rr_json = resource_report_to_json(&resource_report)?;
    write_text_file(&resource_report_path, &rr_json)?;

    // 3. Sampled evidence: attach a pre-sampled JSON or shell out to Python.
    let sampled_text = if let Some(attach) = options.attach_sampled {
        std::fs::read_to_string(attach)
            .with_context(|| format!("read attached sampled JSON {}", attach.display()))?
    } else {
        sampler.sample(&qec_json_path, &sampled_path)?;
        std::fs::read_to_string(&sampled_path)
            .with_context(|| format!("read sampled JSON {}", sampled_path.display()))?
    };
    let sampled: SampledEvidence = serde_json::from_str(&sampled_text)
        .with_context(|| "parse sampled evidence JSON (quon_qec_sinter.py --json output)")?;

    // 4. Fuse with provenance checking (refuse or warn on mismatch).
    let provenance = Provenance::from_experiment(
        &experiment,
        request.source_path.display().to_string(),
        request.target.id.clone(),
        experiment_sha256,
    );
    let fused = fuse(
        provenance,
        resource_report,
        sampled,
        options.allow_sampled_mismatch,
    )
    .map_err(|e| anyhow!("{e}"))?;

    // 5. Write the separate JSON + Markdown validation artifacts.
    let json = validation_report_to_json(&fused)?;
    write_text_file(validation_path, &json)?;
    let md = validation_report_to_markdown(&fused);
    write_text_file(&markdown_path, &md)?;

    if !fused.mismatch_warnings.is_empty() {
        eprintln!(
            "{warning}warning{warning:#}: sampled data provenance mismatch (attached anyway):"
        );
        for w in &fused.mismatch_warnings {
            eprintln!("  - {w}");
        }
    }

    if !options.quiet {
        eprintln!(
            "{ok}wrote QEC validation report{ok:#} → {} (+ .md; separate QEC / resource / sampled primaries)",
            validation_path.display()
        );
    }

    Ok(())
}

/// Strip a trailing `.validation.json` / `.json` / `.validation` to a stem path.
fn validation_base(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("report");
    let stem = name
        .strip_suffix(".validation.json")
        .or_else(|| name.strip_suffix(".json"))
        .or_else(|| name.strip_suffix(".validation"))
        .unwrap_or(name);
    path.with_file_name(stem)
}

/// Append `suffix` to the file name of `base` (e.g. `out` + `.qec.json`).
fn with_suffix(base: &Path, suffix: &str) -> PathBuf {
    let name = base
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("report");
    base.with_file_name(format!("{name}{suffix}"))
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_text_file(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut contents = body.to_string();
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Durable Wait barrier cycles from [`round_barrier_cuts`], fail-closed on count.
fn memory_round_barrier_cycles(
    layers: &[ScheduleLayer],
    expected_memory_rounds: usize,
) -> Result<Vec<u32>> {
    let cuts = round_barrier_cuts(layers);
    let mut cycles = Vec::new();
    for &(idx, _) in &cuts {
        let layer = layers.get(idx as usize).ok_or_else(|| {
            anyhow!(
                "round_barrier_cuts index {idx} out of range ({} layers)",
                layers.len()
            )
        })?;
        let is_wait = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Wait { .. }));
        if is_wait {
            cycles.push(layer.cycle);
        }
    }
    if cycles.len() != expected_memory_rounds {
        bail!(
            "QEC na_refs barrier_cycle: found {} durable Wait barrier(s) via \
             round_barrier_cuts, expected {} barrier round(s); refusing unchecked Wait mapping",
            cycles.len(),
            expected_memory_rounds
        );
    }
    Ok(cycles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quon_na::{AtomId, MeasurementBasis};

    #[test]
    fn validation_paths_strip_known_suffixes() {
        assert_eq!(
            validation_base(Path::new("out/report.validation.json")),
            PathBuf::from("out/report")
        );
        assert_eq!(
            validation_base(Path::new("out/report.json")),
            PathBuf::from("out/report")
        );
        assert_eq!(
            validation_base(Path::new("out/report.validation")),
            PathBuf::from("out/report")
        );
        assert_eq!(
            with_suffix(Path::new("out/report"), ".qec.json"),
            PathBuf::from("out/report.qec.json")
        );
        assert_eq!(
            with_suffix(Path::new("out/report"), ".sampled.json"),
            PathBuf::from("out/report.sampled.json")
        );
        assert_eq!(
            with_suffix(Path::new("out/report"), ".validation.md"),
            PathBuf::from("out/report.validation.md")
        );
    }

    #[test]
    fn sha256_hex_is_lowercase() {
        assert_eq!(
            sha256_hex_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn write_text_file_adds_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.json");
        write_text_file(&path, "{\"a\":1}").expect("write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "{\"a\":1}\n");
        write_text_file(&path, "already\n").expect("rewrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "already\n");
    }

    fn wait(cycle: u32) -> ScheduleLayer {
        ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
        }
    }

    fn measure(cycle: u32) -> ScheduleLayer {
        ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Measure {
                atom: AtomId(0),
                basis: MeasurementBasis::Z,
                duration_us: 1,
            }],
        }
    }

    #[test]
    fn barrier_cycles_follow_durable_waits_and_refuse_a_count_mismatch() {
        let layers = vec![wait(10), measure(11), wait(20), measure(21)];
        assert_eq!(
            memory_round_barrier_cycles(&layers, 2).expect("cycles"),
            vec![10, 20]
        );
        let err = memory_round_barrier_cycles(&layers, 1).expect_err("count");
        let message = format!("{err:#}");
        assert!(message.contains("found 2 durable Wait barrier"));
        assert!(message.contains("expected 1 barrier round"));
    }
}
