use std::process::Command;

#[test]
fn quonc_help_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_quonc"))
        .arg("--help")
        .output()
        .expect("failed to run quonc");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Quon quantum compiler"));
}
