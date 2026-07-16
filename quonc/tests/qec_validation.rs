//! Integration: `--emit-qec-validation` fused report (#280 / ADR-0020).
//!
//! Golden coverage: one successful fused report and one provenance-mismatch
//! refusal. The success/mismatch cases below use `--attach-sampled` so they run
//! deterministically without the Stim/Sinter stack; a separate end-to-end test
//! shells out to Python and soft-skips when the stack is unavailable.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn repo_root() -> PathBuf {
    workspace_path("..")
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

fn rep_source() -> PathBuf {
    workspace_path("../examples/na_qec/repetition_d3_memory.qn")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Emit the QEC experiment to `<dir>/out.qec.json` so the validation run (same
/// base) re-emits byte-identical JSON — its SHA is a stable provenance key.
fn emit_experiment_and_sha(dir: &std::path::Path) -> String {
    let json_path = dir.join("out.qec.json");
    let output = quonc()
        .arg(rep_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-experiment")
        .arg(&json_path)
        .arg("--quiet")
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "emit stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&json_path).expect("read qec.json");
    sha256_hex(&bytes)
}

fn sampled_json(sha: &str, distance: u32, rounds: u32) -> String {
    format!(
        r#"{{
  "schema_version": 1,
  "evidence_kind": "sampled",
  "disclaimer": "Sampled Stim/Sinter logical failures — validation evidence, not a threshold claim (ADR-0020).",
  "decoder": "pymatching",
  "seed": 7,
  "tick_us": 1.0,
  "confidence_level": 0.95,
  "experiments": [
    {{
      "experiment": "out.qec.json",
      "experiment_sha256": "{sha}",
      "family": "repetition",
      "code_family": "repetition_code_toy",
      "distance": {distance},
      "rounds": {rounds},
      "logical_observables": ["obs0:L0:z"],
      "results": [
        {{
          "shots": 256,
          "error_scale": 1.0,
          "noise_model": {{"rydberg": 0.002, "measurement": 0.003, "reset": 0.004, "movement": 0.0005, "transfer": 0.0007, "idle_per_us": 2e-09}},
          "logical_failures": 1,
          "logical_failure_rate": 0.00390625,
          "confidence_interval": {{"low": 0.0, "high": 0.0217, "level": 0.95, "method": "wilson"}}
        }}
      ]
    }}
  ]
}}
"#
    )
}

#[test]
fn validation_fuses_analytic_and_sampled_sections() {
    let dir = std::env::temp_dir().join(format!("quon-qec-280-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");

    let sha = emit_experiment_and_sha(&dir);
    let sampled_path = dir.join("sampled.json");
    std::fs::write(&sampled_path, sampled_json(&sha, 3, 2)).expect("write sampled");

    let validation_path = dir.join("out.validation.json");
    let output = quonc()
        .arg(rep_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-validation")
        .arg(&validation_path)
        .arg("--attach-sampled")
        .arg(&sampled_path)
        .arg("--quiet")
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Separate primaries are kept beside the fused report (ADR-0020).
    assert!(validation_path.is_file(), "missing validation.json");
    assert!(dir.join("out.validation.md").is_file(), "missing .md");
    assert!(dir.join("out.qec.json").is_file(), "missing qec.json");
    assert!(dir.join("out.stim").is_file(), "missing sibling stim");
    assert!(
        dir.join("out.resource_report.json").is_file(),
        "missing analytic resource report"
    );

    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&validation_path).expect("read"))
            .expect("parse validation json");

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["kind"], "qec_validation_report");
    assert!(
        doc["disclaimer"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("not a threshold claim"),
        "top-level disclaimer must disclaim thresholds"
    );

    // Provenance ties sampled evidence to the compiled artifact.
    assert_eq!(doc["provenance"]["distance"], 3);
    assert_eq!(doc["provenance"]["rounds"], 2);
    assert_eq!(doc["provenance"]["experiment_sha256"], sha);
    assert_eq!(doc["provenance"]["code_family"], "repetition_code_toy");

    // Separate analytic + sampled sections with evidence_kind labels.
    let analytic = &doc["analytic"];
    assert_eq!(analytic["evidence_kind"], "analytic");
    assert!(
        analytic["resource_report"]["estimated_cycles"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(analytic["resource_report"]["error_budget"].is_object());

    let sampled = &doc["sampled"];
    assert_eq!(sampled["evidence_kind"], "sampled");
    assert_eq!(sampled["decoder"], "pymatching");
    assert_eq!(sampled["seed"], 7);
    let exp0 = &sampled["experiments"][0];
    assert_eq!(exp0["logical_observables"][0], "obs0:L0:z");
    let result0 = &exp0["results"][0];
    assert_eq!(result0["shots"], 256);
    assert!(result0["noise_model"]["rydberg"].as_f64().unwrap() > 0.0);
    assert_eq!(result0["logical_failures"], 1);
    assert!(result0["confidence_interval"]["high"].as_f64().is_some());
    assert_eq!(result0["confidence_interval"]["method"], "wilson");

    // Fused report must not carry mismatch warnings on a clean match.
    assert!(doc.get("mismatch_warnings").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validation_refuses_mismatched_sampled_data() {
    let dir = std::env::temp_dir().join(format!("quon-qec-280-mismatch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");

    let sha = emit_experiment_and_sha(&dir);
    // Distance disagrees with the compiled artifact (d=3) → incompatible.
    let sampled_path = dir.join("sampled.json");
    std::fs::write(&sampled_path, sampled_json(&sha, 5, 2)).expect("write sampled");

    let validation_path = dir.join("out.validation.json");
    let output = quonc()
        .arg(rep_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-validation")
        .arg(&validation_path)
        .arg("--attach-sampled")
        .arg(&sampled_path)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        !output.status.success(),
        "expected refusal on provenance mismatch; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match") && stderr.contains("distance"),
        "stderr should explain the distance mismatch: {stderr}"
    );
    assert!(
        !validation_path.exists(),
        "must not write a fused report on refusal"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validation_allow_mismatch_downgrades_to_warning() {
    let dir = std::env::temp_dir().join(format!("quon-qec-280-warn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");

    let sha = emit_experiment_and_sha(&dir);
    let sampled_path = dir.join("sampled.json");
    std::fs::write(&sampled_path, sampled_json(&sha, 5, 2)).expect("write sampled");

    let validation_path = dir.join("out.validation.json");
    let output = quonc()
        .arg(rep_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-validation")
        .arg(&validation_path)
        .arg("--attach-sampled")
        .arg(&sampled_path)
        .arg("--allow-sampled-mismatch")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "allow-mismatch should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&validation_path).expect("read"))
            .expect("parse");
    let warnings = doc["mismatch_warnings"].as_array().expect("warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("distance")),
        "warnings must record the distance mismatch: {warnings:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end: quonc shells out to the Python Stim/Sinter harness. Soft-skips
/// when the stack (or venv) is unavailable, mirroring `qec_experiment_emit.rs`.
#[test]
fn validation_end_to_end_shells_out_to_python() {
    let venv = repo_root().join(".venv/bin/python");
    if !venv.is_file() {
        eprintln!(
            "skip validation e2e: no repo .venv python ({})",
            venv.display()
        );
        return;
    }
    // Confirm the Stim stack is importable before asserting success.
    let probe = Command::new(&venv)
        .args(["-c", "import stim, sinter, pymatching"])
        .output();
    match probe {
        Ok(out) if out.status.success() => {}
        _ => {
            eprintln!("skip validation e2e: stim stack not importable in .venv");
            return;
        }
    }

    let dir = std::env::temp_dir().join(format!("quon-qec-280-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let validation_path = dir.join("rep.validation.json");

    let output = quonc()
        .current_dir(repo_root())
        .arg(rep_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-validation")
        .arg(&validation_path)
        .arg("--validation-shots")
        .arg("32")
        .arg("--python")
        .arg(&venv)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "e2e stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&validation_path).expect("read"))
            .expect("parse");
    assert_eq!(doc["kind"], "qec_validation_report");
    assert_eq!(doc["analytic"]["evidence_kind"], "analytic");
    assert_eq!(doc["sampled"]["evidence_kind"], "sampled");
    let result0 = &doc["sampled"]["experiments"][0]["results"][0];
    assert_eq!(result0["shots"], 32);
    assert!(result0["confidence_interval"]["method"] == "wilson");

    let _ = std::fs::remove_dir_all(&dir);
}
