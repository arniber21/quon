use std::process::Command;

#[test]
fn list_rules_prints_twelve() {
    let bin = env!("CARGO_BIN_EXE_quonlint");
    let output = Command::new(bin).arg("--list-rules").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let count = stdout.lines().filter(|l| l.contains('/')).count();
    assert!(count >= 12, "expected >= 12 rules, got {count}:\n{stdout}");
}

#[test]
fn json_output_smoke() {
    let bin = env!("CARGO_BIN_EXE_quonlint");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../frontend/tests/fixtures/bell_state.qn"
    );
    let output = Command::new(bin)
        .args(["--format", "json", fixture])
        .output()
        .unwrap();
    assert!(output.status.success());
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(v.get("diagnostics").is_some());
}
