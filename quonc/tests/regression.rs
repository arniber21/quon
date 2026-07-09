//! Snapshot save/compare integration tests (issue #48).

use std::path::PathBuf;
use std::process::Command;

use quon_core::{RegressionConfig, compare, load_snapshot};

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn snapshot_save_writes_baseline() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let out = std::env::temp_dir().join(format!("quon-baseline-{}.json", std::process::id()));

    let output = quonc()
        .arg("--metrics-snapshot")
        .arg("save")
        .arg(&out)
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = load_snapshot(&out).expect("load saved snapshot");
    assert_eq!(saved.metrics.as_ref().unwrap().gate_count, 2);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn snapshot_compare_passes_against_committed_baseline() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let baseline = workspace_path("tests/fixtures/metrics/bell_baseline.json");
    let config_path = std::env::temp_dir().join(format!(
        "quon-compare-ignore-ms-{}.toml",
        std::process::id()
    ));
    std::fs::write(
        &config_path,
        r#"
[ignore]
fields = ["compile_ms"]
"#,
    )
    .expect("write config");

    let output = quonc()
        .arg("--metrics-snapshot")
        .arg("compare")
        .arg(&baseline)
        .arg("--regression-config")
        .arg(&config_path)
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(
        output.status.success(),
        "compare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_file(&config_path);
}

#[test]
fn snapshot_compare_fails_on_gate_regression() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let baseline = workspace_path("tests/fixtures/metrics/bell_baseline.json");
    let loaded = load_snapshot(&baseline).expect("baseline");
    let mut perturbed = loaded.clone();
    perturbed.metrics.as_mut().unwrap().gate_count = 999;
    let tmp = std::env::temp_dir().join(format!("quon-bad-baseline-{}.json", std::process::id()));
    quon_core::save_snapshot(&tmp, &perturbed).expect("write perturbed baseline");

    let output = quonc()
        .arg("--metrics-snapshot")
        .arg("compare")
        .arg(&tmp)
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gate_count"));
    assert!(stderr.contains("FAIL"));

    let config = RegressionConfig::default();
    let current = quonc()
        .arg("--metrics-json")
        .arg("-")
        .arg(&source)
        .output()
        .expect("spawn");
    let current_snap: quon_core::MetricsSnapshot =
        serde_json::from_slice(&current.stdout).expect("json");
    let report = compare(&perturbed, &current_snap, &config).expect("compare");
    assert!(!report.passed);

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn regression_config_allows_gate_count_drift() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let baseline = workspace_path("tests/fixtures/metrics/bell_baseline.json");
    let mut loaded = load_snapshot(&baseline).expect("baseline");
    loaded.metrics.as_mut().unwrap().gate_count = 999;
    let tmp = std::env::temp_dir().join(format!("quon-loose-baseline-{}.json", std::process::id()));
    quon_core::save_snapshot(&tmp, &loaded).expect("write baseline");

    let config_path =
        std::env::temp_dir().join(format!("quon-tolerances-{}.toml", std::process::id()));
    std::fs::write(
        &config_path,
        r#"
[metrics.gate_count]
absolute = 1000

[ignore]
fields = ["compile_ms"]
"#,
    )
    .expect("write config");

    let output = quonc()
        .arg("--metrics-snapshot")
        .arg("compare")
        .arg(&tmp)
        .arg("--regression-config")
        .arg(&config_path)
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&config_path);
}
