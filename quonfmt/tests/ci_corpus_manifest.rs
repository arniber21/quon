//! CI corpus manifest paths must exist and remain `.qn` files.

use std::path::PathBuf;

#[test]
fn ci_corpus_manifest_paths_exist() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest = root.join("test/tooling/ci-corpus.txt");
    let text = std::fs::read_to_string(&manifest).expect("read manifest");
    let mut count = 0usize;
    for line in text.lines() {
        let path = line.split('#').next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        count += 1;
        let full = root.join(path);
        assert!(full.is_file(), "missing corpus file: {path}");
        assert!(
            path.ends_with(".qn"),
            "corpus entries must be .qn files: {path}"
        );
    }
    assert_eq!(count, 16, "expected 16 corpus entries");
}
