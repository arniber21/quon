//! Compile metrics wire types, snapshot persistence, and regression comparison.
//!
//! This module is the **snapshot/regression DTO** of the compiler — the
//! `MetricsSnapshot` / `ComparisonReport` wire types, the `MetricTolerances`
//! regression gate, and the pure snapshot save/load/compare logic. It is
//! MLIR-free so snapshot/compare can be unit-tested without linking LLVM.
//!
//! The *metric collector* — the pass that walks the IR to populate
//! [`CircuitMetrics`] — does **not** live here; it lives in `mlir_bridge`,
//! next to the IR it inspects. `quon_core::metrics` owns only the DTO and its
//! pure tests; collecting, emit, and CLI wiring stay in their owning crates.
//! See the `quon_core` glossary entry in CONTEXT.md.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current metrics snapshot schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Numeric circuit metrics collected from the final IR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircuitMetrics {
    pub depth: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_bound: Option<String>,
    pub gate_count: u64,
    pub t_count: u64,
    pub qubit_count: u64,
    pub swap_count: u64,
}

/// Provenance for the compiled program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInfo {
    pub source: String,
    pub source_sha256: String,
    pub entry: String,
}

/// Backend target metadata embedded in a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor_path: Option<String>,
}

/// Toolchain provenance for reproducibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainInfo {
    pub quonc_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    pub git_dirty: bool,
}

/// Compile outcome and timing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInfo {
    pub status: CompileStatus,
    pub compile_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileStatus {
    Ok,
    Error,
}

/// One compile's metrics document (snapshot file contents).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsSnapshot {
    pub schema_version: u32,
    pub program: ProgramInfo,
    pub target: TargetInfo,
    pub toolchain: ToolchainInfo,
    pub compile: CompileInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<CircuitMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation: Option<serde_json::Value>,
}

impl MetricsSnapshot {
    pub fn ok(
        program: ProgramInfo,
        target: TargetInfo,
        toolchain: ToolchainInfo,
        compile_ms: u64,
        metrics: CircuitMetrics,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            program,
            target,
            toolchain,
            compile: CompileInfo {
                status: CompileStatus::Ok,
                compile_ms,
                error: None,
            },
            metrics: Some(metrics),
            simulation: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("unsupported schema version {found}, expected {expected}")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error("snapshot compile status is not ok")]
    NotSuccessful,
    #[error("missing metrics in snapshot")]
    MissingMetrics,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Per-metric tolerance bounds for regression comparison.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricTolerance {
    #[serde(default)]
    pub absolute: f64,
    #[serde(default)]
    pub relative: f64,
}

/// Regression configuration loaded from TOML or JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionConfig {
    #[serde(default = "default_metric_tolerances")]
    pub metrics: MetricTolerances,
    #[serde(default)]
    pub ignore: IgnoreConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricTolerances {
    #[serde(default = "zero_tolerance")]
    pub depth: MetricTolerance,
    #[serde(default = "zero_tolerance")]
    pub gate_count: MetricTolerance,
    #[serde(default = "zero_tolerance")]
    pub t_count: MetricTolerance,
    #[serde(default = "zero_tolerance")]
    pub swap_count: MetricTolerance,
    #[serde(default = "compile_ms_tolerance")]
    pub compile_ms: MetricTolerance,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IgnoreConfig {
    #[serde(default)]
    pub fields: Vec<String>,
}

fn zero_tolerance() -> MetricTolerance {
    MetricTolerance {
        absolute: 0.0,
        relative: 0.0,
    }
}

fn compile_ms_tolerance() -> MetricTolerance {
    MetricTolerance {
        absolute: 0.0,
        relative: 0.10,
    }
}

fn default_metric_tolerances() -> MetricTolerances {
    MetricTolerances {
        depth: zero_tolerance(),
        gate_count: zero_tolerance(),
        t_count: zero_tolerance(),
        swap_count: zero_tolerance(),
        compile_ms: compile_ms_tolerance(),
    }
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self {
            metrics: default_metric_tolerances(),
            ignore: IgnoreConfig::default(),
        }
    }
}

impl RegressionConfig {
    pub fn load(path: &Path) -> Result<Self, MetricsError> {
        let text = fs::read_to_string(path)?;
        if path.extension().is_some_and(|ext| ext == "toml") {
            Ok(toml::from_str(&text)?)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }
}

/// One metric field that failed regression tolerance.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    pub field: String,
    pub baseline: f64,
    pub current: f64,
    pub delta: f64,
    pub tolerance: MetricTolerance,
}

/// Outcome of comparing two snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonReport {
    pub passed: bool,
    pub violations: Vec<Violation>,
}

fn within_tolerance(baseline: f64, current: f64, tol: &MetricTolerance) -> bool {
    let delta = current - baseline;
    if delta.abs() <= tol.absolute {
        return true;
    }
    if tol.relative > 0.0 {
        let bound = tol.relative * baseline.abs().max(1.0);
        if delta.abs() <= bound {
            return true;
        }
    }
    false
}

fn check_field(
    field: &str,
    baseline: f64,
    current: f64,
    tol: &MetricTolerance,
    ignored: &HashSet<&str>,
    violations: &mut Vec<Violation>,
) {
    if ignored.contains(field) {
        return;
    }
    if !within_tolerance(baseline, current, tol) {
        violations.push(Violation {
            field: field.to_string(),
            baseline,
            current,
            delta: current - baseline,
            tolerance: tol.clone(),
        });
    }
}

/// Compare numeric metrics between `baseline` and `current` using `config`.
pub fn compare(
    baseline: &MetricsSnapshot,
    current: &MetricsSnapshot,
    config: &RegressionConfig,
) -> Result<ComparisonReport, MetricsError> {
    if baseline.schema_version != SCHEMA_VERSION {
        return Err(MetricsError::UnsupportedSchema {
            found: baseline.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    if current.schema_version != SCHEMA_VERSION {
        return Err(MetricsError::UnsupportedSchema {
            found: current.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    let baseline_metrics = baseline
        .metrics
        .as_ref()
        .ok_or(MetricsError::MissingMetrics)?;
    let current_metrics = current
        .metrics
        .as_ref()
        .ok_or(MetricsError::MissingMetrics)?;

    let ignored: HashSet<&str> = config.ignore.fields.iter().map(String::as_str).collect();
    let mut violations = Vec::new();

    check_field(
        "depth",
        baseline_metrics.depth as f64,
        current_metrics.depth as f64,
        &config.metrics.depth,
        &ignored,
        &mut violations,
    );
    check_field(
        "gate_count",
        baseline_metrics.gate_count as f64,
        current_metrics.gate_count as f64,
        &config.metrics.gate_count,
        &ignored,
        &mut violations,
    );
    check_field(
        "t_count",
        baseline_metrics.t_count as f64,
        current_metrics.t_count as f64,
        &config.metrics.t_count,
        &ignored,
        &mut violations,
    );
    check_field(
        "swap_count",
        baseline_metrics.swap_count as f64,
        current_metrics.swap_count as f64,
        &config.metrics.swap_count,
        &ignored,
        &mut violations,
    );
    check_field(
        "compile_ms",
        baseline.compile.compile_ms as f64,
        current.compile.compile_ms as f64,
        &config.metrics.compile_ms,
        &ignored,
        &mut violations,
    );

    Ok(ComparisonReport {
        passed: violations.is_empty(),
        violations,
    })
}

/// Write a snapshot atomically (temp file + rename).
pub fn save_snapshot(path: &Path, snapshot: &MetricsSnapshot) -> Result<(), MetricsError> {
    if snapshot.compile.status != CompileStatus::Ok {
        return Err(MetricsError::NotSuccessful);
    }
    let json = serde_json::to_string_pretty(snapshot)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, format!("{json}\n"))?;
    fs::rename(temp, path)?;
    Ok(())
}

/// Load a snapshot from disk.
pub fn load_snapshot(path: &Path) -> Result<MetricsSnapshot, MetricsError> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Human-readable tolerance description for compare output.
pub fn format_tolerance(tol: &MetricTolerance) -> String {
    let mut parts = Vec::new();
    if tol.absolute > 0.0 {
        parts.push(format!("abs≤{}", tol.absolute));
    }
    if tol.relative > 0.0 {
        parts.push(format!("rel≤{}%", tol.relative * 100.0));
    }
    if parts.is_empty() {
        "abs≤0".to_string()
    } else {
        parts.join(" ")
    }
}

/// Render the regression diff table to a string (for stderr).
pub fn format_comparison_table(
    baseline: &MetricsSnapshot,
    current: &MetricsSnapshot,
    report: &ComparisonReport,
    config: &RegressionConfig,
) -> String {
    let baseline_metrics = baseline.metrics.as_ref();
    let current_metrics = current.metrics.as_ref();
    let ignored: HashSet<&str> = config.ignore.fields.iter().map(String::as_str).collect();

    let rows: [(&str, f64, f64, &MetricTolerance); 5] = [
        (
            "depth",
            baseline_metrics.map(|m| m.depth as f64).unwrap_or(0.0),
            current_metrics.map(|m| m.depth as f64).unwrap_or(0.0),
            &config.metrics.depth,
        ),
        (
            "gate_count",
            baseline_metrics.map(|m| m.gate_count as f64).unwrap_or(0.0),
            current_metrics.map(|m| m.gate_count as f64).unwrap_or(0.0),
            &config.metrics.gate_count,
        ),
        (
            "t_count",
            baseline_metrics.map(|m| m.t_count as f64).unwrap_or(0.0),
            current_metrics.map(|m| m.t_count as f64).unwrap_or(0.0),
            &config.metrics.t_count,
        ),
        (
            "swap_count",
            baseline_metrics.map(|m| m.swap_count as f64).unwrap_or(0.0),
            current_metrics.map(|m| m.swap_count as f64).unwrap_or(0.0),
            &config.metrics.swap_count,
        ),
        (
            "compile_ms",
            baseline.compile.compile_ms as f64,
            current.compile.compile_ms as f64,
            &config.metrics.compile_ms,
        ),
    ];

    let mut lines = vec![format!(
        "{:<14} {:<9} {:<9} {:<9} {:<11} {}",
        "metric", "baseline", "current", "delta", "tolerance", "status"
    )];

    for (field, b, c, tol) in rows {
        if ignored.contains(field) {
            continue;
        }
        let delta = c - b;
        let delta_str = if delta > 0.0 {
            format!("+{}", delta as i64)
        } else if delta < 0.0 {
            format!("{}", delta as i64)
        } else {
            "0".to_string()
        };
        let failed = report.violations.iter().any(|v| v.field == field);
        let status = if failed { "FAIL" } else { "ok" };
        lines.push(format!(
            "{:<14} {:<9} {:<9} {:<9} {:<11} {}",
            field,
            b as u64,
            c as u64,
            delta_str,
            format_tolerance(tol),
            status
        ));
    }
    lines.join("\n")
}

/// Stable single-line metrics summary for stderr.
pub fn format_metrics_line(snapshot: &MetricsSnapshot) -> String {
    let metrics = snapshot.metrics.as_ref();
    format!(
        "[quonc] depth={} gates={} t={} swaps={} compile={}ms target={}",
        metrics.map(|m| m.depth).unwrap_or(0),
        metrics.map(|m| m.gate_count).unwrap_or(0),
        metrics.map(|m| m.t_count).unwrap_or(0),
        metrics.map(|m| m.swap_count).unwrap_or(0),
        snapshot.compile.compile_ms,
        snapshot.target.id,
    )
}

/// Watch-mode line with ISO timestamp and deltas vs a previous snapshot.
pub fn format_watch_metrics_line(
    snapshot: &MetricsSnapshot,
    previous: Option<&MetricsSnapshot>,
) -> String {
    let now = fmt_timestamp();
    let current = snapshot.metrics.as_ref();
    let line = format_metrics_line(snapshot);
    let Some(prev) = previous else {
        return format!("[quonc] {now} {}", line.trim_start_matches("[quonc] "));
    };
    let prev_metrics = prev.metrics.as_ref();
    let delta = |field: fn(&CircuitMetrics) -> u64| -> String {
        let p = prev_metrics.map(field).unwrap_or(0);
        let c = current.map(field).unwrap_or(0);
        if c > p {
            format!("(+{})", c - p)
        } else if c < p {
            format!("(-{})", p - c)
        } else {
            "(=)".to_string()
        }
    };
    let compile_delta = {
        let p = prev.compile.compile_ms;
        let c = snapshot.compile.compile_ms;
        if c > p {
            format!("(+{}ms)", c - p)
        } else if c < p {
            format!("(-{}ms)", p - c)
        } else {
            "(=)".to_string()
        }
    };
    format!(
        "[quonc] {now} depth={} {} gates={} {} t={} {} swaps={} {} compile={}ms {} target={}",
        current.map(|m| m.depth).unwrap_or(0),
        delta(|m| m.depth),
        current.map(|m| m.gate_count).unwrap_or(0),
        delta(|m| m.gate_count),
        current.map(|m| m.t_count).unwrap_or(0),
        delta(|m| m.t_count),
        current.map(|m| m.swap_count).unwrap_or(0),
        delta(|m| m.swap_count),
        snapshot.compile.compile_ms,
        compile_delta,
        snapshot.target.id,
    )
}

fn fmt_timestamp() -> String {
    // Best-effort ISO-8601 without external deps: use local wall-clock seconds.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Enough for watch-mode delta lines; full calendar formatting is non-essential.
    format!("t={secs}")
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: baseline={} current={} delta={:+} (tol: {})",
            self.field,
            self.baseline,
            self.current,
            self.delta,
            format_tolerance(&self.tolerance)
        )
    }
}
