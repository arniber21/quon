//! Unit tests for metrics snapshot schema and regression comparison.

use proptest::prelude::*;
use quon_core::{
    CircuitMetrics, CompileInfo, CompileStatus, MetricTolerance, MetricTolerances, MetricsSnapshot,
    ProgramInfo, RegressionConfig, TargetInfo, ToolchainInfo, compare,
};

fn snapshot_with_metrics(metrics: CircuitMetrics, compile_ms: u64) -> MetricsSnapshot {
    MetricsSnapshot {
        schema_version: quon_core::SCHEMA_VERSION,
        program: ProgramInfo {
            source: "test.qn".into(),
            source_sha256: "abc".into(),
            entry: "main".into(),
        },
        target: TargetInfo {
            id: "generic_openqasm".into(),
            descriptor_path: None,
        },
        toolchain: ToolchainInfo {
            quonc_version: "0.1.0".into(),
            git_commit: None,
            git_dirty: false,
        },
        compile: CompileInfo {
            status: CompileStatus::Ok,
            compile_ms,
            error: None,
        },
        metrics: Some(metrics),
        simulation: None,
    }
}

#[test]
fn default_regression_rejects_depth_drift() {
    let baseline = snapshot_with_metrics(
        CircuitMetrics {
            depth: 10,
            ..CircuitMetrics::default()
        },
        100,
    );
    let current = snapshot_with_metrics(
        CircuitMetrics {
            depth: 11,
            ..CircuitMetrics::default()
        },
        100,
    );
    let report = compare(&baseline, &current, &RegressionConfig::default()).expect("compare");
    assert!(!report.passed);
    assert!(report.violations.iter().any(|v| v.field == "depth"));
}

#[test]
fn absolute_tolerance_passes_small_gate_drift() {
    let baseline = snapshot_with_metrics(
        CircuitMetrics {
            gate_count: 100,
            ..CircuitMetrics::default()
        },
        100,
    );
    let current = snapshot_with_metrics(
        CircuitMetrics {
            gate_count: 103,
            ..CircuitMetrics::default()
        },
        100,
    );
    let config = RegressionConfig {
        metrics: MetricTolerances {
            gate_count: MetricTolerance {
                absolute: 5.0,
                relative: 0.0,
            },
            ..MetricTolerances::default()
        },
        ..RegressionConfig::default()
    };
    let report = compare(&baseline, &current, &config).expect("compare");
    assert!(report.passed);
}

#[test]
fn compile_ms_relative_tolerance() {
    let baseline = snapshot_with_metrics(CircuitMetrics::default(), 200);
    let current = snapshot_with_metrics(CircuitMetrics::default(), 210);
    let report = compare(&baseline, &current, &RegressionConfig::default()).expect("compare");
    assert!(report.passed);
}

proptest! {
    #[test]
    fn identical_snapshots_always_pass(
        depth in 0u64..1000,
        gates in 0u64..5000,
        compile_ms in 0u64..5000,
    ) {
        let metrics = CircuitMetrics {
            depth,
            gate_count: gates,
            ..CircuitMetrics::default()
        };
        let baseline = snapshot_with_metrics(metrics.clone(), compile_ms);
        let current = snapshot_with_metrics(metrics, compile_ms);
        let report = compare(&baseline, &current, &RegressionConfig::default()).expect("compare");
        prop_assert!(report.passed);
    }
}

#[test]
fn deny_unknown_fields_on_regression_config_json() {
    let err = serde_json::from_str::<RegressionConfig>(
        r#"{"metrics":{"depth":{"absolute":0}},"extra":true}"#,
    )
    .expect_err("unknown field");
    assert!(err.to_string().contains("unknown field"));
}
