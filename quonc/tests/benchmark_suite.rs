//! CI smoke test for the neutral-atom QEC benchmark suite (#284).
//!
//! Compiles each benchmark with quonc and checks that the resource report
//! contains the expected comparable metrics. The non-Clifford placeholder
//! is skipped until #283 lands.

use std::path::PathBuf;
use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel)
}

fn compile_benchmark(source: &str) -> String {
    let source_path = workspace_path(source);
    let target = workspace_path("targets/neutral_atom/generic_rna_v0.json");
    let output = quonc()
        .arg(&source_path)
        .arg("--target")
        .arg(&target)
        .arg("--emit-resource-report")
        .arg("-")
        .output()
        .expect("quonc binary not found — run cargo build -p quonc");

    assert!(
        output.status.success(),
        "quonc failed for {source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn check_metric(report: &str, field: &str) -> u64 {
    let prefix = format!("\"{field}\": ");
    let start = report
        .find(&prefix)
        .unwrap_or_else(|| panic!("missing field `{field}` in resource report"));
    let rest = &report[start + prefix.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end]
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("field `{field}` is not a u64"))
}

#[test]
fn benchmark_surface_d3_memory_compiles() {
    let report = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_memory.qn");
    assert!(check_metric(&report, "rydberg_stages") > 0);
    assert!(check_metric(&report, "rearrangement_steps") > 0);
    assert!(check_metric(&report, "estimated_cycles") > 0);
}

#[test]
fn benchmark_surface_d3_measure_compiles() {
    let report = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_measure.qn");
    assert!(check_metric(&report, "rydberg_stages") > 0);
    assert!(check_metric(&report, "rearrangement_steps") > 0);
}

#[test]
fn benchmark_surface_d3_cx_compiles() {
    let report = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_cx.qn");
    assert!(check_metric(&report, "rydberg_stages") > 0);
    assert!(check_metric(&report, "rearrangement_steps") > 0);
}

#[test]
fn benchmark_surface_d3_ghz_compiles() {
    let report = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_ghz.qn");
    assert!(check_metric(&report, "rydberg_stages") > 0);
    assert!(check_metric(&report, "rearrangement_steps") > 0);
}

#[test]
fn benchmark_cx_has_more_rydberg_stages_than_memory() {
    let memory = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_memory.qn");
    let cx = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_cx.qn");
    let mem_stages = check_metric(&memory, "rydberg_stages");
    let cx_stages = check_metric(&cx, "rydberg_stages");
    assert!(
        cx_stages > mem_stages,
        "CX should have more Rydberg stages than memory: {cx_stages} vs {mem_stages}"
    );
}

#[test]
fn benchmark_ghz_has_more_stages_than_cx() {
    let cx = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_cx.qn");
    let ghz = compile_benchmark("samples/neutral-atom/benchmarks/surface_d3_ghz.qn");
    let cx_stages = check_metric(&cx, "rydberg_stages");
    let ghz_stages = check_metric(&ghz, "rydberg_stages");
    assert!(
        ghz_stages > cx_stages,
        "GHZ should have more Rydberg stages than CX: {ghz_stages} vs {cx_stages}"
    );
}
