//! `--emit-mapping-json` and the standalone mapping viewer (issue #135).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

const SPAN: &str = "\
fn span(): Circuit<3, 3, 4, Clifford> = circuit {
    H @0 |> H @1 |> H @2 |> CNOT @(0, 2)
}

fn main(): Q<List<Bit>> = run {
    q <- span() @ qreg(3)
    measure_all(q)
}
";

fn write_span() -> PathBuf {
    let path = std::env::temp_dir().join(format!("quon-mapping-span-{}.qn", std::process::id()));
    std::fs::write(&path, SPAN).expect("write span source");
    path
}

fn emit_mapping(source: &PathBuf, target: Option<&PathBuf>) -> std::process::Output {
    let mut cmd = quonc();
    cmd.arg("--emit-mapping-json").arg("-").arg(source);
    if let Some(target) = target {
        cmd.arg("--target").arg(target);
    }
    cmd.output().expect("spawn quonc")
}

fn swap_events(trace: &Value) -> usize {
    trace["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|event| event["kind"] == "swap")
        .count()
}

#[test]
fn all_to_all_span_inserts_no_swaps() {
    let source = write_span();
    let output = emit_mapping(&source, None);
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(trace["kind"], "mapping_trace");
    assert_eq!(trace["schema_version"], 1);
    assert_eq!(swap_events(&trace), 0);
    let summary = trace["summary"].as_str().expect("summary");
    assert!(summary.contains("no SWAPs"), "{summary}");
    let stages = trace["stages"].as_array().expect("stages");
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[0]["id"], "layout");
    assert_eq!(stages[1]["id"], "routing");
    assert_eq!(stages[2]["id"], "native_decomp");
    assert!(!stages[0]["summary"].as_str().unwrap_or("").is_empty());
}

#[test]
fn line_span_records_swaps_that_final_metrics_drop() {
    let source = write_span();
    let target = workspace_path("../targets/ibm/fake_manila_v2.json");
    let output = emit_mapping(&source, Some(&target));
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace: Value = serde_json::from_slice(&output.stdout).expect("json");
    let swaps = swap_events(&trace);
    assert!(swaps >= 1, "trace: {trace}");
    let stages = trace["stages"].as_array().expect("stages");
    let routing_swaps = stages[1]["metrics"]["swap_count"].as_u64().expect("swaps");
    let final_swaps = stages[2]["metrics"]["swap_count"].as_u64().expect("final");
    assert_eq!(routing_swaps, swaps as u64);
    assert_eq!(final_swaps, 0, "final metrics should drop decomposed SWAPs");
    let layout_depth = stages[0]["metrics"]["depth"]
        .as_u64()
        .expect("layout depth");
    let routing_depth = stages[1]["metrics"]["depth"]
        .as_u64()
        .expect("routing depth");
    assert!(
        layout_depth > 0 && routing_depth > layout_depth,
        "non-adjacent CNOT should deepen the routing stage (layout {layout_depth}, routing {routing_depth})"
    );
    let decomp = stages[2]["summary"].as_str().expect("decomp summary");
    assert!(decomp.contains("CX triples"), "{decomp}");
    assert!(trace["summary"].as_str().unwrap_or("").contains("SWAP"));

    let json_path =
        std::env::temp_dir().join(format!("quon-mapping-span-{}.json", std::process::id()));
    std::fs::write(&json_path, &output.stdout).expect("write trace");
    let script = workspace_path("../python/visualize_mapping.py");
    let rendered = Command::new("python3")
        .arg(&script)
        .arg(&json_path)
        .arg("--ascii")
        .output()
        .expect("python3");
    assert!(
        rendered.status.success(),
        "viewer failed: {}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let ascii = String::from_utf8_lossy(&rendered.stdout);
    assert!(ascii.contains("mapping_trace v1"), "{ascii}");
    assert!(ascii.contains("swap insertions:"), "{ascii}");
    assert!(ascii.contains("CX triples"), "{ascii}");
}

const BRANCH: &str = "\
fn place(): Circuit<3, 3, 3, Clifford> = circuit { I @0 |> I @1 |> I @2 }
fn far_cx(): Circuit<2, 2, 2, Clifford> = circuit { CNOT @(0, 1) }
fn idle2(): Circuit<2, 2, 2, Clifford> = circuit { I @0 |> I @1 }

fn main(): Q<Bit> = run {
    (q0, q1, q2) <- place() @ qreg(3)
    bit          <- measure(q1)
    (a, c)       <- (if bit then far_cx() else idle2()) @ (q0, q2)
    _ba          <- measure(a)
    bc           <- measure(c)
    return bc
}
";

#[test]
fn if_arms_are_alternatives_on_a_line() {
    let source = std::env::temp_dir().join(format!("quon-mapping-if-{}.qn", std::process::id()));
    std::fs::write(&source, BRANCH).expect("write if source");
    let target = workspace_path("../targets/ibm/fake_manila_v2.json");
    let output = emit_mapping(&source, Some(&target));
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace: Value = serde_json::from_slice(&output.stdout).expect("json");
    let events = trace["events"].as_array().expect("events");
    let branches: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == "branch")
        .collect();
    assert_eq!(branches.len(), 2, "trace events: {events:?}");
    assert_eq!(branches[0]["arm"], "then");
    assert_eq!(branches[1]["arm"], "else");
    let then_swaps = branches[0]["events"]
        .as_array()
        .expect("then events")
        .iter()
        .filter(|event| event["kind"] == "swap")
        .count();
    let else_swaps = branches[1]["events"]
        .as_array()
        .expect("else events")
        .iter()
        .filter(|event| event["kind"] == "swap")
        .count();
    assert!(
        then_swaps >= 1,
        "then arm should route the non-adjacent CNOT: {events:?}"
    );
    assert_eq!(else_swaps, 0, "else arm is idle: {events:?}");
    let flat_swaps = events
        .iter()
        .filter(|event| event["kind"] == "swap")
        .count();
    assert_eq!(
        flat_swaps, 0,
        "arm SWAPs must not sit on the top-level list"
    );
    let summary = trace["summary"].as_str().expect("summary");
    assert!(summary.contains("exactly one arm runs"), "{summary}");
    assert!(
        branches[0]["layout"].is_array(),
        "then arm records its layout: {events:?}"
    );
    assert!(
        branches[1]["layout"].is_array(),
        "else arm records its layout: {events:?}"
    );
}

const BRANCH_THEN_FOLLOW: &str = "\
fn place(): Circuit<3, 3, 3, Clifford> = circuit { I @0 |> I @1 |> I @2 }
fn far_cx(): Circuit<2, 2, 2, Clifford> = circuit { CNOT @(0, 1) }
fn idle2(): Circuit<2, 2, 2, Clifford> = circuit { I @0 |> I @1 }
fn follow(): Circuit<2, 2, 2, Clifford> = circuit { CNOT @(0, 1) }

fn main(): Q<Bit> = run {
    (q0, q1, q2) <- place() @ qreg(3)
    bit          <- measure(q1)
    (a, c)       <- (if bit then far_cx() else idle2()) @ (q0, q2)
    (a2, c2)     <- follow() @ (a, c)
    _ba          <- measure(a2)
    bc           <- measure(c2)
    return bc
}
";

fn assignment_pairs(rows: &Value) -> Vec<(u64, u64)> {
    rows.as_array()
        .expect("layout array")
        .iter()
        .map(|row| {
            (
                row["logical"].as_u64().expect("logical"),
                row["physical"].as_u64().expect("physical"),
            )
        })
        .collect()
}

#[test]
fn two_qubit_after_diverging_branch_is_not_routed_on_pre_branch_map() {
    let source =
        std::env::temp_dir().join(format!("quon-mapping-follow-{}.qn", std::process::id()));
    std::fs::write(&source, BRANCH_THEN_FOLLOW).expect("write follow source");
    let target = workspace_path("../targets/ibm/fake_manila_v2.json");
    let output = emit_mapping(&source, Some(&target));
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace: Value = serde_json::from_slice(&output.stdout).expect("json");
    let events = trace["events"].as_array().expect("events");
    let branches: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == "branch")
        .collect();
    assert_eq!(branches.len(), 2, "trace events: {events:?}");
    let then_layout = assignment_pairs(&branches[0]["layout"]);
    let else_layout = assignment_pairs(&branches[1]["layout"]);
    assert!(
        !then_layout.is_empty() && !else_layout.is_empty(),
        "each arm records the permutation it finished on: then={then_layout:?} else={else_layout:?}"
    );
    assert_ne!(
        then_layout, else_layout,
        "then swaps and else does not: then={then_layout:?} else={else_layout:?}"
    );
    let then_swaps = branches[0]["events"]
        .as_array()
        .expect("then events")
        .iter()
        .filter(|event| event["kind"] == "swap")
        .count();
    assert!(then_swaps >= 1, "then arm swaps: {events:?}");
    let top_routed = events
        .iter()
        .filter(|event| event["kind"] == "swap" || event["kind"] == "interaction")
        .count();
    assert_eq!(
        top_routed, 0,
        "the follow-up CNOT must not be routed on the pre-branch map: {events:?}"
    );
    let final_layout = trace["final_layout"].as_array().expect("final");
    assert!(
        final_layout.is_empty(),
        "diverging arms leave no single post-branch layout: {final_layout:?}"
    );
    let summary = trace["summary"].as_str().expect("summary");
    assert!(
        summary.contains("no single post-branch layout"),
        "{summary}"
    );

    let json_path =
        std::env::temp_dir().join(format!("quon-mapping-follow-{}.json", std::process::id()));
    std::fs::write(&json_path, &output.stdout).expect("write trace");
    let script = workspace_path("../python/visualize_mapping.py");
    let rendered = Command::new("python3")
        .arg(&script)
        .arg(&json_path)
        .arg("--ascii")
        .output()
        .expect("python3");
    assert!(
        rendered.status.success(),
        "viewer failed: {}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let ascii = String::from_utf8_lossy(&rendered.stdout);
    assert!(ascii.contains("branch then"), "{ascii}");
    assert!(ascii.contains("layout:"), "{ascii}");
}

#[test]
fn viewer_rejects_unknown_field_and_matches_golden() {
    let script = workspace_path("../python/visualize_mapping.py");
    let fixture = workspace_path("../python/testdata/toy_mapping_trace.json");
    let golden = workspace_path("../python/testdata/toy_mapping_trace.ascii");
    let html_golden = workspace_path("../python/testdata/toy_mapping_trace.html");
    let rendered = Command::new("python3")
        .arg(&script)
        .arg(&fixture)
        .arg("--ascii")
        .output()
        .expect("python3");
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let expected = std::fs::read_to_string(&golden).expect("golden");
    assert_eq!(String::from_utf8_lossy(&rendered.stdout), expected);

    let html_out =
        std::env::temp_dir().join(format!("quon-mapping-html-{}.html", std::process::id()));
    let html = Command::new("python3")
        .arg(&script)
        .arg(&fixture)
        .arg("--html")
        .arg(&html_out)
        .output()
        .expect("python3 html");
    assert!(
        html.status.success(),
        "{}",
        String::from_utf8_lossy(&html.stderr)
    );
    let html_expected = std::fs::read_to_string(&html_golden).expect("html golden");
    let html_actual = std::fs::read_to_string(&html_out).expect("html out");
    assert_eq!(html_actual, html_expected);

    let mut bad: Value = serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap()).unwrap();
    bad.as_object_mut()
        .unwrap()
        .insert("extra".into(), Value::from(1));
    let bad_path =
        std::env::temp_dir().join(format!("quon-mapping-bad-{}.json", std::process::id()));
    std::fs::write(&bad_path, bad.to_string()).unwrap();
    let rejected = Command::new("python3")
        .arg(&script)
        .arg(&bad_path)
        .arg("--ascii")
        .output()
        .expect("python3");
    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("unknown field"), "{stderr}");
}

#[test]
fn neutral_atom_target_rejects_mapping_json() {
    let source = workspace_path("../test/verify/bell.qn");
    let target = workspace_path("../targets/neutral_atom/generic_rna_v0.json");
    let output = quonc()
        .arg("--emit-mapping-json")
        .arg("-")
        .arg("--target")
        .arg(&target)
        .arg(&source)
        .output()
        .expect("spawn");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("fixed target"), "stderr: {stderr}");
}
