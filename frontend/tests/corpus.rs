//! Real-world quantum program corpus (50 programs).
//!
//! A quality/regression bar beyond the eight SPEC §12 reference algorithms: each program is a
//! self-contained, physically-motivated quantum routine (variational optimization, Hamiltonian
//! simulation, core algorithms, state preparation, error correction, fermionic chemistry) written
//! at the depth and fidelity of the reference fixtures — symbolic depth, `Matrix`/`for`/`fold`,
//! faithful `Circuit<n,m,d,C>` annotations. Many of the core-algorithm and state-prep programs
//! exercise the value-dependent machinery (issues #57–#60): recursive kernels, dependent-match
//! base cases, and call-site substitution.
//!
//! The harness asserts every `corpus/*.qn` file **parses and type-checks** through the public
//! `frontend::check_program` facade. The files are discovered at runtime so adding a program needs
//! no edit here.

use std::fs;
use std::path::PathBuf;

use frontend::check_program;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus")
}

/// Every `.qn` file in the corpus directory, sorted for deterministic ordering.
fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("corpus directory should exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "qn"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_corpus_program_parses_and_type_checks() {
    let files = corpus_files();
    assert!(!files.is_empty(), "expected at least one corpus program");
    let mut failures = Vec::new();
    for path in &files {
        let src = fs::read_to_string(path).expect("readable corpus file");
        if let Err(diags) = check_program(&src) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
            failures.push(format!("{name}: {msgs:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} corpus program(s) failed to type-check:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn corpus_has_exactly_fifty_programs() {
    // The deliverable is exactly 50 programs (issue: value-dependent corpus).
    assert_eq!(
        corpus_files().len(),
        50,
        "expected exactly 50 corpus programs, found {}",
        corpus_files().len()
    );
}
