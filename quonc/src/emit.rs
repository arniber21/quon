//! Sequential emit-flag wiring for the quonc driver.
//!
//! [`emit_artifacts`] is the library seam for `--emit-*` output. It keeps the
//! CLI's stdout-ownership order: OpenQASM claims stdout first, then quantum.na
//! MLIR, then schedule JSON, the interaction graph, the resource report, and
//! compiler stats. A later `-` path goes to stderr once an earlier artifact
//! already owns stdout. Naviz and QEC paths are filesystem-only.
//!
//! QEC experiment dual-emit and validation live in [`crate::qec_emit`]. This
//! module calls them through [`QecArtifactSink`] so the `emitted` flag and the
//! success hint keep their original place in the sequence.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anstyle::Style;
use anyhow::{Context as _, Result, anyhow, bail};
use backend::TargetKind;
use quon_na::{
    attach_qec_error_budget, na_stats_to_json, require_target_error_model, resource_report_to_json,
    resource_report_to_markdown,
};

use crate::compile::{
    CompileReport, CompileRequest, build_mapping_trace, build_na_schedule_view, schedule_to_json,
    schedule_to_mlir,
};

/// Resource-report body chosen by `--resource-report-format` or the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportFormat {
    Json,
    Markdown,
}

/// Emit-flag selection, independent of the clap `Cli` struct.
///
/// Paths borrow the CLI's owned strings for the duration of one emit.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactFlags<'a> {
    pub emit_qasm: bool,
    pub emit_mapping_json: Option<&'a str>,
    pub emit_na_mlir: Option<&'a str>,
    pub emit_na_schedule: Option<&'a str>,
    pub emit_na_graph: Option<&'a str>,
    pub emit_resource_report: Option<&'a str>,
    pub emit_na_stats: Option<&'a str>,
    pub emit_naviz: Option<&'a Path>,
    pub emit_qec_experiment: Option<&'a Path>,
    pub emit_qec_validation: Option<&'a Path>,
    pub resource_report_format: Option<ReportFormat>,
    pub verify_na: bool,
    pub quiet: bool,
    pub metrics: bool,
    /// `--metrics-json` was set. The hint is suppressed; the file itself is
    /// written by the CLI after [`emit_artifacts`] returns.
    pub has_metrics_json: bool,
    /// `--metrics-snapshot` was set. Same hint suppression as metrics JSON.
    pub has_metrics_snapshot: bool,
}

/// Filesystem QEC artifacts, invoked only when the corresponding flag is set
/// and only after the stdout artifacts, so validation still sees the same
/// `emitted` state.
///
/// The CLI implements this by calling [`crate::qec_emit`]. Stim/Sinter sampling
/// stays in the binary and is passed into validation from there.
pub trait QecArtifactSink {
    fn emit_experiment(
        &self,
        request: &CompileRequest,
        report: &CompileReport,
        json_path: &Path,
    ) -> Result<()>;

    fn emit_validation(
        &self,
        request: &CompileRequest,
        report: &CompileReport,
        validation_path: &Path,
    ) -> Result<()>;
}

/// Emit mapping-trace JSON. Also used on the compile-failure path when a
/// partial trace was recorded.
pub fn emit_mapping_json(
    path: &str,
    qasm_owns_stdout: bool,
    request: &CompileRequest,
    report: &CompileReport,
) -> Result<()> {
    let trace = build_mapping_trace(report, request)?;
    let json = trace
        .to_json_string_pretty()
        .context("serializing mapping trace")?;
    write_output(path, &json, route_dash_to_stderr(path, qasm_owns_stdout))?;
    Ok(())
}

/// Write every selected artifact. `dim` styles the success hint the same way
/// the CLI styles other stderr notes.
pub fn emit_artifacts(
    flags: &ArtifactFlags<'_>,
    request: &CompileRequest,
    report: &CompileReport,
    dim: Style,
    qec: &impl QecArtifactSink,
) -> Result<()> {
    let mut emitted = false;
    // When OpenQASM already owns stdout, subsequent `-` emits go to stderr.
    let qasm_owns_stdout = flags.emit_qasm;

    if flags.emit_qasm {
        if let Some(qasm) = &report.qasm {
            print!("{qasm}");
            emitted = true;
        } else {
            bail!("OpenQASM emission produced no output (is the target fixed?)");
        }
    }

    if let Some(path) = flags.emit_mapping_json {
        emit_mapping_json(path, qasm_owns_stdout, request, report)?;
        emitted = true;
    }

    // quantum.na MLIR is the primary NA artifact (ADR-0011): it takes stdout
    // ahead of the JSON debug view when both target `-`.
    if let Some(path) = flags.emit_na_mlir {
        let spec = report.na_schedule_spec.as_ref().ok_or_else(|| {
            anyhow!("no quantum.na schedule available (compile with a neutral-atom target)")
        })?;
        let mlir = schedule_to_mlir(spec)?;
        // Prefer verifying the emitted text so dump drift cannot slip past the
        // in-memory `verify_schedule_spec` path (ADR-0021 nit).
        if flags.verify_na || report.qec_backed {
            quon_na::verify_mlir_text(&mlir)
                .map_err(|e| anyhow!("emitted quantum.na failed verification: {e}"))?;
        }
        write_output(path, &mlir, route_dash_to_stderr(path, qasm_owns_stdout))?;
        emitted = true;
    }
    let mlir_owns_stdout = flags.emit_na_mlir.is_some_and(|p| p == "-");

    if let Some(path) = flags.emit_na_schedule {
        let view = build_na_schedule_view(report, request)?;
        let json = schedule_to_json(&view)?;
        write_output(
            path,
            &json,
            route_dash_to_stderr(path, qasm_owns_stdout || mlir_owns_stdout),
        )?;
        emitted = true;
    }
    let schedule_on_stdout = flags.emit_na_schedule.is_some_and(|p| p == "-");

    if let Some(path) = flags.emit_na_graph {
        let graph = report.na_graph.as_ref().ok_or_else(|| {
            anyhow!("no interaction graph available (compile with a neutral-atom target)")
        })?;
        let dot = graph.to_dot();
        write_output(
            path,
            &dot,
            route_dash_to_stderr(
                path,
                qasm_owns_stdout || mlir_owns_stdout || schedule_on_stdout,
            ),
        )?;
        emitted = true;
    }
    let graph_on_stdout = flags.emit_na_graph.is_some_and(|p| p == "-");

    if let Some(path) = flags.emit_resource_report {
        let report_body = report.resource_report.as_ref().ok_or_else(|| {
            anyhow!("no resource report available (compile with a neutral-atom target)")
        })?;
        // ADR-0017: NA resource-report emit always attaches analytic error_budget
        // and hard-fails when the target has no error_model (never 1−fidelity).
        let na = match &request.target.kind {
            TargetKind::NeutralAtomReconfigurable(na) => na,
            _ => bail!(
                "--emit-resource-report requires a neutral_atom_reconfigurable target \
                 (see targets/neutral_atom/)"
            ),
        };
        let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
        let report_body = attach_qec_error_budget(report_body.clone(), Some(model))
            .map_err(|e| anyhow!("{e}"))?;
        let text = match resolve_report_format(flags.resource_report_format, path) {
            ReportFormat::Json => resource_report_to_json(&report_body)?,
            ReportFormat::Markdown => resource_report_to_markdown(&report_body),
        };
        // If MLIR / schedule / graph already printed to stdout on `-`, send the
        // report to stderr so all artifacts remain recoverable without
        // interleaving values.
        write_output(
            path,
            &text,
            route_dash_to_stderr(
                path,
                qasm_owns_stdout || mlir_owns_stdout || schedule_on_stdout || graph_on_stdout,
            ),
        )?;
        emitted = true;
    }
    let resource_report_on_stdout = flags.emit_resource_report.is_some_and(|p| p == "-");

    if let Some(path) = flags.emit_na_stats {
        let stats = report.na_stats.as_ref().ok_or_else(|| {
            anyhow!(
                "no NA compiler stats available for this compile (the neutral-atom \
                 pipeline failed to populate NaStats — this should not happen; see \
                 issue #307)"
            )
        })?;
        let json = na_stats_to_json(stats)?;
        // If an earlier artifact already printed to stdout on `-`, send stats
        // to stderr so all artifacts remain recoverable without interleaving.
        write_output(
            path,
            &json,
            route_dash_to_stderr(
                path,
                qasm_owns_stdout
                    || mlir_owns_stdout
                    || schedule_on_stdout
                    || graph_on_stdout
                    || resource_report_on_stdout,
            ),
        )?;
        emitted = true;
    }

    if let Some(path) = flags.emit_naviz {
        let layers = report.na_schedule.as_ref().ok_or_else(|| {
            anyhow!("no neutral-atom schedule available (compile with a neutral-atom target)")
        })?;
        let layout = report.na_layout.as_ref().ok_or_else(|| {
            anyhow!("no neutral-atom layout available (compile with a neutral-atom target)")
        })?;
        let na = match &request.target.kind {
            TargetKind::NeutralAtomReconfigurable(na) => na,
            _ => bail!(
                "--emit-naviz requires a neutral_atom_reconfigurable target \
                 (see targets/neutral_atom/)"
            ),
        };
        // NAViz machine id = the sibling .namachine file-name stem.
        let machine_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("--emit-naviz requires a file path with a stem"))?;
        let namachine_path = path.with_extension("namachine");
        let naviz = quon_na::naviz::schedule_to_naviz(layers, layout, machine_id);
        let namachine = quon_na::naviz::target_to_namachine(na, layout, &request.target.id);
        write_atomic(path, &naviz)?;
        write_atomic(&namachine_path, &namachine)?;
        emitted = true;
    }

    if let Some(json_path) = flags.emit_qec_experiment {
        qec.emit_experiment(request, report, json_path)?;
        emitted = true;
    }

    if let Some(validation_path) = flags.emit_qec_validation {
        qec.emit_validation(request, report, validation_path)?;
        emitted = true;
    }

    if !emitted
        && !flags.has_metrics_json
        && !flags.has_metrics_snapshot
        && !flags.metrics
        && !flags.quiet
    {
        match &report.snapshot.target.id {
            id if report.na_schedule.is_some() => {
                eprintln!(
                    "{dim}(compiled successfully for neutral-atom target `{id}`; \
                     pass --emit-na-mlir for quantum.na MLIR, or \
                     --emit-na-schedule / --emit-na-graph / --emit-resource-report / \
                     --emit-na-stats / --emit-naviz / --emit-qec-experiment / \
                     --emit-qec-validation for debug / QEC evaluation artifacts){dim:#}"
                );
            }
            id => {
                eprintln!(
                    "{dim}(compiled successfully for `{id}`; pass --emit-qasm to print OpenQASM 3.0, \
                     or --emit-mapping-json for the routing trace){dim:#}"
                );
            }
        }
    }

    Ok(())
}

/// `true` when `path` is stdout and an earlier artifact already claimed it.
fn route_dash_to_stderr(path: &str, earlier_owns_stdout: bool) -> bool {
    earlier_owns_stdout && path == "-"
}

/// Write `contents` via temp file + rename so readers never see a partial artifact.
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("artifact");
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, contents).with_context(|| format!("write temp {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn resolve_report_format(explicit: Option<ReportFormat>, path: &str) -> ReportFormat {
    if let Some(fmt) = explicit {
        return fmt;
    }
    if path != "-" && path.to_ascii_lowercase().ends_with(".md") {
        ReportFormat::Markdown
    } else {
        ReportFormat::Json
    }
}

/// Write an artifact to a file, stdout, or stderr.
///
/// Stdout (`path == "-"` and `prefer_stderr` is false) writes the bytes and
/// appends a newline when the body does not already end in one. Stderr uses
/// `eprintln!`. A filesystem path creates missing parents and always ends in
/// a newline.
pub fn write_output(path: &str, body: &str, prefer_stderr: bool) -> Result<()> {
    if path == "-" {
        if prefer_stderr {
            eprintln!("{body}");
        } else {
            io::stdout().write_all(body.as_bytes())?;
            if !body.ends_with('\n') {
                io::stdout().write_all(b"\n")?;
            }
        }
    } else {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut contents = body.to_string();
        if !contents.ends_with('\n') {
            contents.push('\n');
        }
        std::fs::write(&path, contents)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_stdout_yields_only_when_an_earlier_artifact_owns_it() {
        assert!(!route_dash_to_stderr("-", false));
        assert!(route_dash_to_stderr("-", true));
        assert!(!route_dash_to_stderr("out.json", true));
        assert!(!route_dash_to_stderr("out.json", false));
    }

    #[test]
    fn report_format_explicit_flag_wins_over_extension() {
        assert_eq!(
            resolve_report_format(Some(ReportFormat::Json), "out.md"),
            ReportFormat::Json
        );
        assert_eq!(
            resolve_report_format(Some(ReportFormat::Markdown), "out.json"),
            ReportFormat::Markdown
        );
    }

    #[test]
    fn report_format_follows_md_extension_except_stdout() {
        assert_eq!(
            resolve_report_format(None, "out.md"),
            ReportFormat::Markdown
        );
        assert_eq!(
            resolve_report_format(None, "out.MD"),
            ReportFormat::Markdown
        );
        assert_eq!(resolve_report_format(None, "out.json"), ReportFormat::Json);
        assert_eq!(resolve_report_format(None, "-"), ReportFormat::Json);
        assert_eq!(
            resolve_report_format(None, "notes.md.bak"),
            ReportFormat::Json
        );
    }

    #[test]
    fn write_output_file_adds_trailing_newline_and_parents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("artifact.json");
        let path_str = path.to_str().expect("utf8 path");
        write_output(path_str, "{\"a\":1}", false).expect("write");
        let body = std::fs::read_to_string(&path).expect("read");
        assert_eq!(body, "{\"a\":1}\n");

        write_output(path_str, "already\n", false).expect("rewrite");
        let body = std::fs::read_to_string(&path).expect("read");
        assert_eq!(body, "already\n");
    }
}
