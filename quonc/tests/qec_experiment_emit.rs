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

fn python() -> Command {
    // Prefer workspace venv (CI / just setup-python), else PATH python3.
    let venv = workspace_path("../.venv/bin/python");
    if venv.is_file() {
        Command::new(venv)
    } else {
        Command::new("python3")
    }
}

#[test]
fn repetition_d3_emits_qec_json_and_sibling_stim() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let dir = std::env::temp_dir().join(format!("quon-qec-255-{}", std::process::id()));
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
    assert!(
        stim_path.is_file(),
        "missing sibling {}",
        stim_path.display()
    );

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
    assert_eq!(
        doc["check_graph"]["data_atoms"],
        serde_json::json!([0, 2, 4])
    );
    assert!(doc["na_refs"].as_array().unwrap().len() >= 4);

    // na_refs barrier_cycle set on memory rounds and matches count.
    let na_refs = doc["na_refs"].as_array().expect("na_refs");
    let memory_barriers: Vec<_> = na_refs
        .iter()
        .filter(|r| r["kind"] == "memory_round")
        .collect();
    assert_eq!(memory_barriers.len(), 2);
    for r in &memory_barriers {
        assert!(
            r.get("barrier_cycle").and_then(|v| v.as_u64()).is_some(),
            "memory_round missing barrier_cycle: {r}"
        );
    }
    assert!(
        na_refs
            .iter()
            .filter(|r| r["kind"] != "memory_round")
            .all(|r| r.get("barrier_cycle").is_none() || r["barrier_cycle"].is_null()),
        "non-memory rounds must not set barrier_cycle"
    );

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
    assert_eq!(
        loaded.logical_observables[0].basis,
        quon_qec::LogicalBasis::Z
    );

    let stim = std::fs::read_to_string(&stim_path).expect("read stim");
    assert!(stim.contains("DETECTOR"), "{stim}");
    assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
    assert!(stim.contains("MR 1 3"), "{stim}");
    assert!(stim.contains("MZ 0 2 4"), "{stim}");
    assert!(stim.contains("CX 0 1 2 3"), "{stim}");
    assert!(stim.contains("CX 2 1 4 3"), "{stim}");
    assert!(
        !stim.contains("CX 0 1 2 1 2 3 4 3"),
        "overlapping packed CX must not appear:\n{stim}"
    );
    assert!(
        !stim.contains("DEPOLARIZE") && !stim.contains("X_ERROR"),
        "structure-only Stim must omit noise:\n{stim}"
    );

    // Stim Python smoke (ADR-0022): parse, detector count, noiseless sample.
    let smoke = python()
        .arg("-c")
        .arg(format!(
            r#"
import stim
c = stim.Circuit.from_file({stim_path:?})
assert c.num_detectors > 0, c.num_detectors
assert c.num_observables == 1, c.num_observables
s1 = c.compile_sampler(seed=0).sample(shots=8)
s2 = c.compile_sampler(seed=0).sample(shots=8)
assert (s1 == s2).all(), "noiseless sample must be deterministic for fixed seed"
print(f"ok detectors={{c.num_detectors}} observables={{c.num_observables}}")
"#
        ))
        .output();
    match smoke {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("ModuleNotFoundError") || stderr.contains("No module named 'stim'") {
                eprintln!("skip stim smoke: stim not installed ({stderr})");
            } else {
                panic!(
                    "stim smoke failed: status={} stderr={} stdout={}",
                    out.status,
                    stderr,
                    String::from_utf8_lossy(&out.stdout)
                );
            }
        }
        Err(e) => eprintln!("skip stim smoke: python unavailable ({e})"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn surface_d3_emits_qec_json_and_sibling_stim() {
    let source = workspace_path("../examples/na_qec/surface_d3_memory.qn");
    let dir = std::env::temp_dir().join(format!("quon-qec-249-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let json_path = dir.join("surface_d3.qec.json");
    let stim_path = dir.join("surface_d3.stim");

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
    assert!(
        stim_path.is_file(),
        "missing sibling {}",
        stim_path.display()
    );

    let json_text = std::fs::read_to_string(&json_path).expect("read json");
    let doc: Value =
        serde_json::from_str(&json_text).unwrap_or_else(|e| panic!("parse JSON: {e}\n{json_text}"));

    assert_eq!(doc["family"], "surface");
    assert_eq!(doc["code_family"], "surface_code_like");
    assert_eq!(doc["distance"], 3);
    assert_eq!(doc["rounds"], 2);
    assert_eq!(doc["check_graph"]["atoms"].as_array().unwrap().len(), 17);
    assert_eq!(
        doc["check_graph"]["check_atoms"].as_array().unwrap().len(),
        8
    );
    assert_eq!(
        doc["logical_observables"][0]["atoms"],
        serde_json::json!([0, 1, 2])
    );

    let loaded: quon_qec::QecExperiment = serde_json::from_value(doc).expect("strict DTO load");
    assert_eq!(loaded.family, "surface");
    assert_eq!(
        loaded
            .check_graph
            .stabilizers
            .iter()
            .filter(|s| s.basis == quon_qec::LogicalBasis::X)
            .count(),
        4
    );

    let stim = std::fs::read_to_string(&stim_path).expect("read stim");
    assert!(stim.contains("family=surface"), "{stim}");
    assert!(stim.contains("H 9 11 14 16"), "{stim}");
    assert!(stim.contains("MR 9 10 11 12 13 14 15 16"), "{stim}");
    assert!(stim.contains("MZ 0 1 2 3 4 5 6 7 8"), "{stim}");
    assert!(stim.contains("DETECTOR"), "{stim}");
    assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
    assert!(
        !stim.contains("DEPOLARIZE") && !stim.contains("X_ERROR"),
        "structure-only Stim must omit noise:\n{stim}"
    );

    let smoke = python()
        .arg("-c")
        .arg(format!(
            r#"
import stim
c = stim.Circuit.from_file({stim_path:?})
assert c.num_detectors > 0, c.num_detectors
assert c.num_observables == 1, c.num_observables
d1, o1 = c.compile_detector_sampler(seed=0).sample(shots=8, separate_observables=True)
d2, o2 = c.compile_detector_sampler(seed=0).sample(shots=8, separate_observables=True)
assert (d1 == d2).all() and (o1 == o2).all(), "noiseless detector sample must be deterministic"
assert not d1.any() and not o1.any(), "Z-memory under |0…0⟩ must yield zero detectors/obs"
print(f"ok detectors={{c.num_detectors}} observables={{c.num_observables}}")
"#
        ))
        .output();
    match smoke {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("ModuleNotFoundError") || stderr.contains("No module named 'stim'") {
                eprintln!("skip stim smoke: stim not installed ({stderr})");
            } else {
                panic!(
                    "stim smoke failed: status={} stderr={} stdout={}",
                    out.status,
                    stderr,
                    String::from_utf8_lossy(&out.stdout)
                );
            }
        }
        Err(e) => eprintln!("skip stim smoke: python unavailable ({e})"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn emit_qec_experiment_fails_without_error_model() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let mut target: Value =
        serde_json::from_str(&std::fs::read_to_string(na_target()).expect("read target"))
            .expect("parse target");
    target
        .as_object_mut()
        .expect("object")
        .remove("error_model");

    let dir = std::env::temp_dir().join(format!("quon-qec-255-no-em-{}", std::process::id()));
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

#[test]
fn emit_qec_experiment_fails_for_non_qec_program() {
    let source = workspace_path("../test/na/bell.qn");
    let dir = std::env::temp_dir().join(format!("quon-qec-255-non-qec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let json_path = dir.join("bell.qec.json");

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
        !output.status.success(),
        "expected failure for bare-qubit program; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("QEC-backed") || stderr.contains("experiment IR") || stderr.contains("qec"),
        "stderr: {stderr}"
    );
    assert!(!json_path.exists(), "must not write JSON on failure");
    assert!(
        !dir.join("bell.stim").exists(),
        "must not write Stim on failure"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn error_model_snapshot_matches_backend_alias() {
    // Field-equivalence: backend NeutralAtomErrorModelSnapshot is a type alias
    // of quon_qec::ErrorModelSnapshot — same wire JSON.
    let snap = quon_qec::ErrorModelSnapshot {
        rydberg: 0.002,
        measurement: 0.003,
        reset: 0.004,
        movement: 0.0005,
        transfer: 0.0007,
        idle_per_us: 2e-9,
    };
    let qec_json = serde_json::to_value(snap).expect("qec");
    let backend_snap: backend::NeutralAtomErrorModelSnapshot = snap;
    let backend_json = serde_json::to_value(backend_snap).expect("backend");
    assert_eq!(qec_json, backend_json);
    let back: quon_qec::ErrorModelSnapshot =
        serde_json::from_value(backend_json).expect("round-trip");
    assert_eq!(back, snap);
}

#[test]
fn surface_d3_cx_emits_qec_json_and_sibling_stim() {
    let source = workspace_path("../examples/na_qec/surface_d3_cx.qn");
    let dir = std::env::temp_dir().join(format!("quon-qec-250-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let json_path = dir.join("surface_d3_cx.qec.json");
    let stim_path = dir.join("surface_d3_cx.stim");

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
    assert!(
        stim_path.is_file(),
        "missing sibling {}",
        stim_path.display()
    );

    let json_text = std::fs::read_to_string(&json_path).expect("read json");
    let doc: Value =
        serde_json::from_str(&json_text).unwrap_or_else(|e| panic!("parse JSON: {e}\n{json_text}"));

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["kind"], "qec_experiment");
    assert_eq!(doc["family"], "surface");
    assert_eq!(doc["distance"], 3);
    let logical_ids = doc["logical_ids"].as_array().expect("logical_ids");
    assert!(logical_ids.len() >= 2, "{logical_ids:?}");

    let na_refs = doc["na_refs"].as_array().expect("na_refs");
    assert!(
        na_refs.iter().any(|r| r["kind"] == "merge_rough"),
        "missing merge_rough: {na_refs:?}"
    );
    assert!(
        na_refs.iter().any(|r| r["kind"] == "merge_smooth"),
        "missing merge_smooth: {na_refs:?}"
    );
    assert!(
        na_refs.iter().any(|r| r["kind"] == "frame_update"),
        "missing frame_update: {na_refs:?}"
    );

    let barrier_refs: Vec<_> = na_refs
        .iter()
        .filter(|r| {
            matches!(
                r["kind"].as_str(),
                Some("merge_rough")
                    | Some("merge_smooth")
                    | Some("split_rough")
                    | Some("split_smooth")
                    | Some("measure_ancilla")
                    | Some("memory_round")
            )
        })
        .collect();
    assert!(!barrier_refs.is_empty());
    for r in &barrier_refs {
        assert!(
            r.get("barrier_cycle").and_then(|v| v.as_u64()).is_some(),
            "barrier round missing barrier_cycle: {r}"
        );
    }

    let stim = std::fs::read_to_string(&stim_path).expect("read stim");
    assert!(stim.contains("lattice-surgery CX"), "{stim}");
    assert!(stim.contains("L-shaped"), "{stim}");
    assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
    assert!(stim.contains("OBSERVABLE_INCLUDE(1)"), "{stim}");
    assert!(stim.contains("CX "), "{stim}");
    assert!(
        doc["measurement_schedule"]
            .as_array()
            .expect("sched")
            .iter()
            .any(|e| e["kind"] == "frame_update"
                && e["frame_updates"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)),
        "frame_updates missing from measurement_schedule: {json_text}"
    );
    assert!(
        !stim.contains("DEPOLARIZE") && !stim.contains("X_ERROR"),
        "structure-only Stim must omit noise:\n{stim}"
    );

    let smoke = python()
        .arg("-c")
        .arg(format!(
            r#"
import stim
c = stim.Circuit.from_file({stim_path:?})
assert c.num_observables >= 2, c.num_observables
dets, obs = c.compile_detector_sampler(seed=0).sample(shots=32, separate_observables=True)
if dets.size:
    assert not dets.any(), f"noiseless detector fired: {{dets.sum()}}"
assert not obs.any(), f"noiseless |00> observables not zero: {{obs.sum()}}"
obs1 = [l for l in str(c).splitlines() if l.startswith("OBSERVABLE_INCLUDE(1)")]
assert obs1 and obs1[0].count("rec[") > 3, obs1
print(f"ok detectors={{c.num_detectors}} observables={{c.num_observables}}")
"#
        ))
        .output();
    match smoke {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("ModuleNotFoundError") || stderr.contains("No module named 'stim'") {
                eprintln!("skip stim smoke: stim not installed ({stderr})");
            } else {
                panic!(
                    "stim smoke failed: status={} stderr={} stdout={}",
                    out.status,
                    stderr,
                    String::from_utf8_lossy(&out.stdout)
                );
            }
        }
        Err(e) => eprintln!("skip stim smoke: python unavailable ({e})"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
