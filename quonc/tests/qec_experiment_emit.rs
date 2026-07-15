//! Integration: `--emit-qec-experiment` dual-emit (#255 / ADR-0018).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

#[test]
fn repetition_d3_emits_qec_json_and_sibling_stim() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let dir = std::env::temp_dir().join(format!(
        "quon-qec-255-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let json_path = dir.join("repetition_d3.qec.json");
    let stim_path = dir.join("repetition_d3.stim");

    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-qec-experiment")
        .arg(&json_path)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(json_path.is_file(), "missing {}", json_path.display());
    assert!(stim_path.is_file(), "missing sibling {}", stim_path.display());

    let json_text = std::fs::read_to_string(&json_path).expect("read json");
    let doc: Value =
        serde_json::from_str(&json_text).unwrap_or_else(|e| panic!("parse JSON: {e}\n{json_text}"));

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["kind"], "qec_experiment");
    assert_eq!(doc["family"], "repetition");
    assert_eq!(doc["code_family"], "repetition_code_toy");
    assert_eq!(doc["distance"], 3);
    assert_eq!(doc["rounds"], 2);
    assert_eq!(doc["logical_ids"], serde_json::json!([0]));
    assert_eq!(doc["stim_file"], "repetition_d3.stim");
    assert!(doc["error_model"]["rydberg"].as_f64().unwrap() > 0.0);
    assert_eq!(doc["check_graph"]["check_atoms"], serde_json::json!([1, 3]));
    assert_eq!(doc["check_graph"]["data_atoms"], serde_json::json!([0, 2, 4]));
    assert!(doc["na_refs"].as_array().unwrap().len() >= 4);

    // Strict load: unknown fields must fail (DTO contract for Python #253).
    let mut strict = doc.clone();
    strict
        .as_object_mut()
        .unwrap()
        .insert("not_a_field".into(), Value::Bool(true));
    assert!(
        serde_json::from_value::<quon_qec::QecExperiment>(strict).is_err(),
        "deny_unknown_fields must reject extras"
    );
    let loaded: quon_qec::QecExperiment =
        serde_json::from_value(doc).expect("strict DTO load of emitted JSON");
    assert_eq!(loaded.distance, 3);

    let stim = std::fs::read_to_string(&stim_path).expect("read stim");
    assert!(stim.contains("DETECTOR"), "{stim}");
    assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
    assert!(stim.contains("MR 1 3"), "{stim}");
    assert!(
        !stim.contains("DEPOLARIZE") && !stim.contains("X_ERROR"),
        "structure-only Stim must omit noise:\n{stim}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn emit_qec_experiment_fails_without_error_model() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let mut target: Value = serde_json::from_str(
        &std::fs::read_to_string(na_target()).expect("read target"),
    )
    .expect("parse target");
    target
        .as_object_mut()
        .expect("object")
        .remove("error_model");

    let dir = std::env::temp_dir().join(format!(
        "quon-qec-255-no-em-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let target_path = dir.join("no_error_model.json");
    std::fs::write(&target_path, serde_json::to_string_pretty(&target).unwrap())
        .expect("write target");
    let json_path = dir.join("out.qec.json");

    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(&target_path)
        .arg("--emit-qec-experiment")
        .arg(&json_path)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        !output.status.success(),
        "expected failure without error_model; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing error_model") || stderr.contains("error_model"),
        "stderr: {stderr}"
    );
    assert!(!json_path.exists(), "must not write JSON on failure");

    let _ = std::fs::remove_dir_all(&dir);
}
