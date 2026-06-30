//! E2E tests for backend JSON fixtures and generic_openqasm bootstrap.

use std::path::Path;

#[test]
fn e2e_load_all_json_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            e.path().extension().is_some_and(|x| x == "json")
                && !name.contains("missing")
                && !name.contains("bad")
        })
        .collect();
    assert!(!entries.is_empty(), "expected at least one .json fixture");
    for entry in entries {
        let path = entry.path();
        let target = backend::json::load(&path)
            .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()));
        assert!(target.num_qubits > 0);
        assert!(!target.native_gates.is_empty());
    }
}

#[test]
fn e2e_generic_openqasm_scales() {
    for n in [1, 2, 4, 8, 16] {
        let t = backend::generic_openqasm::target(n);
        assert_eq!(t.num_qubits, n);
        for i in 0..n {
            for j in 0..n {
                let d = t.topology.dist(i, j);
                assert!(d <= 1 || i == j);
            }
        }
    }
}
