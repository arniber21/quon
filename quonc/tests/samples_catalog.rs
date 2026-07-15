//! Validates `samples/catalog.yaml` against the schema locked in ADR-0025
//! (issue #185): required fields, `ci: smoke|none` enum, every `path`
//! resolves, every top-level taxonomy category has at least one entry, and
//! every required README carries its "## Status" section. `ci: smoke`
//! entries are additionally compiled with `quonc` (a real typecheck +
//! lowering, not just a schema check) — see `just ci-samples`.
//!
//! Mirrors `lit.rs`'s "assert the corpus, not just the binary" shape: this
//! is the hard gate other sample packs (#188-#192) register rows against.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Fixed taxonomy from ADR-0025 — do not add categories here without also
/// updating `samples/README.md` and the directory tree.
const CATEGORIES: &[&str] = &[
    "learning",
    "algorithms",
    "workflows",
    "visualization",
    "applications",
    "research",
    "neutral-atom",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntry {
    id: String,
    path: String,
    tags: Vec<String>,
    difficulty: Difficulty,
    quonc_args: Vec<String>,
    artifacts: Vec<String>,
    ci: CiMode,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Difficulty {
    Beginner,
    Intermediate,
    Advanced,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum CiMode {
    Smoke,
    None,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("quonc crate has a workspace parent")
        .to_path_buf()
}

fn load_catalog() -> Catalog {
    let path = workspace_root().join("samples").join("catalog.yaml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_yaml::from_str(&raw)
        .unwrap_or_else(|e| panic!("samples/catalog.yaml failed schema validation: {e}"))
}

#[test]
fn catalog_parses_and_has_expected_version() {
    let catalog = load_catalog();
    assert_eq!(
        catalog.version, 1,
        "bump this test alongside a schema version change"
    );
    assert!(!catalog.entries.is_empty(), "catalog.yaml has no entries");
}

#[test]
fn every_entry_id_is_unique() {
    let catalog = load_catalog();
    let mut seen = BTreeSet::new();
    for entry in &catalog.entries {
        assert!(
            seen.insert(entry.id.clone()),
            "duplicate catalog id: {}",
            entry.id
        );
    }
}

#[test]
fn every_entry_path_exists() {
    let catalog = load_catalog();
    let root = workspace_root();
    for entry in &catalog.entries {
        let resolved = root.join(&entry.path);
        assert!(
            resolved.is_file(),
            "catalog entry `{}` points at a missing file: {}",
            entry.id,
            entry.path
        );
    }
}

#[test]
fn every_top_level_category_has_at_least_one_entry() {
    let catalog = load_catalog();
    for category in CATEGORIES {
        let has_entry = catalog
            .entries
            .iter()
            .any(|e| e.id.starts_with(&format!("{category}/")));
        assert!(
            has_entry,
            "category `{category}` has no catalog entry (need >=1 seed, stub OK)"
        );
    }
}

#[test]
fn entry_tags_are_non_empty() {
    let catalog = load_catalog();
    for entry in &catalog.entries {
        assert!(
            !entry.tags.is_empty(),
            "catalog entry `{}` has no tags",
            entry.id
        );
    }
}

#[test]
fn entry_artifacts_exist_when_declared() {
    let catalog = load_catalog();
    let root = workspace_root();
    for entry in &catalog.entries {
        for artifact in &entry.artifacts {
            let resolved = root.join(artifact);
            assert!(
                resolved.is_file(),
                "catalog entry `{}` declares a missing artifact: {artifact}",
                entry.id
            );
        }
    }
}

#[test]
fn entry_difficulty_matches_a_known_tier() {
    let catalog = load_catalog();
    for entry in &catalog.entries {
        let tier = match entry.difficulty {
            Difficulty::Beginner => "beginner",
            Difficulty::Intermediate => "intermediate",
            Difficulty::Advanced => "advanced",
        };
        assert!(!tier.is_empty());
    }
}

#[test]
fn at_least_one_entry_opts_into_ci_smoke() {
    let catalog = load_catalog();
    let smoke_count = catalog
        .entries
        .iter()
        .filter(|e| e.ci == CiMode::Smoke)
        .count();
    assert!(smoke_count >= 1, "expected at least one `ci: smoke` entry");
}

#[test]
fn samples_readme_has_required_sections() {
    let root = workspace_root();
    let readme = root.join("samples").join("README.md");
    let text = std::fs::read_to_string(&readme)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", readme.display()));
    for heading in ["## Taxonomy", "## Catalog", "## Contributing", "## CI"] {
        assert!(
            text.contains(heading),
            "samples/README.md is missing required section {heading}"
        );
    }
}

#[test]
fn every_category_readme_declares_pack_status() {
    let root = workspace_root();
    for category in CATEGORIES {
        let readme = root.join("samples").join(category).join("README.md");
        let text = std::fs::read_to_string(&readme).unwrap_or_else(|e| {
            panic!("category `{category}` is missing samples/{category}/README.md: {e}")
        });
        assert!(
            text.contains("## Status"),
            "samples/{category}/README.md is missing a `## Status` section"
        );
    }
}

/// The real gate: `ci: smoke` entries must actually typecheck (and, since
/// this drives the full `compile()` pipeline, lower successfully too) via
/// the built `quonc` binary — not just satisfy the YAML schema above.
#[test]
fn ci_smoke_entries_typecheck_with_quonc() {
    let catalog = load_catalog();
    let root = workspace_root();
    let quonc_bin = PathBuf::from(env!("CARGO_BIN_EXE_quonc"));

    let smoke_entries: Vec<&CatalogEntry> = catalog
        .entries
        .iter()
        .filter(|e| e.ci == CiMode::Smoke)
        .collect();
    assert!(
        !smoke_entries.is_empty(),
        "expected at least one `ci: smoke` entry"
    );

    for entry in smoke_entries {
        let source: &Path = Path::new(&entry.path);
        let output = Command::new(&quonc_bin)
            .args(&entry.quonc_args)
            .arg(root.join(source))
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn quonc for `{}`: {e}", entry.id));
        assert!(
            output.status.success(),
            "quonc typecheck failed for catalog entry `{}` ({}):\nstdout:\n{}\nstderr:\n{}",
            entry.id,
            entry.path,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
