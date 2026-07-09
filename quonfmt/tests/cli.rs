//! CLI integration tests for `quonfmt`.

use std::process::Command;

fn quonfmt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonfmt"))
}

#[test]
fn help_shows_usage() {
    let out = quonfmt().arg("--help").output().expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("quonfmt"));
    assert!(stdout.contains("--check"));
    assert!(stdout.contains("--write"));
    assert!(stdout.contains("Comments are stripped"));
}

#[test]
fn version_flag_works() {
    let out = quonfmt().arg("--version").output().expect("spawn");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("quonfmt"));
}

#[test]
fn check_exits_zero_on_formatted() {
    let src = include_str!("corpus/input/decls.qn");
    let formatted = quonfmt::format_str(src).expect("format");
    let path = std::env::temp_dir().join("quonfmt_fmt_test.qn");
    std::fs::write(&path, &formatted).expect("write");
    let out = quonfmt().arg("--check").arg(&path).output().expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn check_exits_one_on_unformatted() {
    let path = std::env::temp_dir().join("quonfmt_messy_test.qn");
    std::fs::write(&path, "fn  foo():Unit=circuit{\n}\n").expect("write");
    let out = quonfmt().arg("--check").arg(&path).output().expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let _ = std::fs::remove_file(path);
}

#[test]
fn check_accepts_crlf_when_content_formatted() {
    let src = quonfmt::format_str(include_str!("corpus/input/decls.qn")).expect("format");
    let crlf = src.replace('\n', "\r\n");
    let path = std::env::temp_dir().join("quonfmt_crlf_test.qn");
    std::fs::write(&path, &crlf).expect("write");
    let out = quonfmt().arg("--check").arg(&path).output().expect("spawn");
    assert!(out.status.success());
    let _ = std::fs::remove_file(path);
}

#[test]
fn unknown_flag_is_error() {
    let out = quonfmt()
        .args(["--not-a-flag", "x.qn"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
}

#[test]
fn missing_file_is_error() {
    let out = quonfmt().arg("/no/such/file.qn").output().expect("spawn");
    assert!(!out.status.success());
}

#[test]
fn write_reformats_in_place() {
    let path = std::env::temp_dir().join("quonfmt_write_test.qn");
    std::fs::write(&path, "fn  foo():Unit=circuit{\n}\n").expect("write");
    let out = quonfmt().arg("-w").arg(&path).output().expect("spawn");
    assert!(out.status.success());
    let written = std::fs::read_to_string(&path).expect("read");
    assert!(quonfmt::check_str(&written).is_ok());
    let _ = std::fs::remove_file(path);
}
