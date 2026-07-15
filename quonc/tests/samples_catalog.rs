//! Validates `samples/catalog.yaml` against the schema locked in ADR-0025
//! (issue #185): required fields, `ci: smoke|none` enum, every `path`
//! resolves, every top-level taxonomy category has at least one entry
//! (and every `id` prefix is a known category), no `path`/`artifacts`
//! escapes the repo via an absolute path or `..`, and every required
//! README carries its "## Status" section as an actual heading line.
//! `ci: smoke` entries are additionally compiled with `quonc` (a real
//! typecheck + lowering, not just a schema check) — see `just ci-samples`
//! (run as part of `just ci-rust`'s workspace test suite, not a separate
//! CI step).
//!
//! Mirrors `lit.rs`'s "assert the corpus, not just the binary" shape: this
//! is the hard gate other sample packs (#188-#192) register rows against.
//!
//! The semantic (non-filesystem) validations — unique ids, known category
//! prefixes, safe relative paths — are factored into standalone `validate_*`
//! functions so the negative-fixture tests below can exercise them directly
//! against inline YAML, without needing files on disk.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
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
    // Not read directly by any assertion below: its only job is to make
    // deserialization fail on an invalid tier (see
    // `negative_fixtures::bad_difficulty_enum_fails_to_parse`), which is a
    // real check, just not one that needs the parsed value afterwards.
    #[allow(dead_code)]
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

/// The category is the first `/`-separated segment of `id`.
fn category_of(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

/// Every entry's `id` prefix must be a taxonomy category from `CATEGORIES` —
/// this is what makes category coverage below meaningful rather than
/// gameable by any string containing a `/`.
fn validate_known_categories(entries: &[CatalogEntry]) -> Result<(), String> {
    for entry in entries {
        let category = category_of(&entry.id);
        if !CATEGORIES.contains(&category) {
            return Err(format!(
                "catalog entry `{}` has an unknown category prefix `{category}` (expected one of {CATEGORIES:?})",
                entry.id
            ));
        }
    }
    Ok(())
}

fn validate_unique_ids(entries: &[CatalogEntry]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !seen.insert(entry.id.clone()) {
            return Err(format!("duplicate catalog id: {}", entry.id));
        }
    }
    Ok(())
}

/// Reject an absolute path or any `..` component — catalog paths must stay
/// relative to the repo root and inside the repo.
fn validate_relative_path(field: &str, id: &str, candidate: &str) -> Result<(), String> {
    let p = Path::new(candidate);
    if p.is_absolute() {
        return Err(format!(
            "catalog entry `{id}` has an absolute {field}: {candidate}"
        ));
    }
    if p.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "catalog entry `{id}` has a path-traversal `..` in {field}: {candidate}"
        ));
    }
    Ok(())
}

fn validate_safe_paths(entries: &[CatalogEntry]) -> Result<(), String> {
    for entry in entries {
        validate_relative_path("path", &entry.id, &entry.path)?;
        for artifact in &entry.artifacts {
            validate_relative_path("artifacts", &entry.id, artifact)?;
        }
    }
    Ok(())
}

/// When `path` lives under `samples/`, its category segment must match the
/// `id` prefix. Entries whose canonical home lives elsewhere (e.g.
/// `neutral-atom/*` linking into `examples/na_qec/`, per ADR-0025) are
/// exempt — the check simply doesn't apply outside `samples/`.
fn validate_path_category_alignment(entries: &[CatalogEntry]) -> Result<(), String> {
    for entry in entries {
        let path = Path::new(&entry.path);
        let mut components = path.components();
        let Some(first) = components.next().and_then(|c| c.as_os_str().to_str()) else {
            continue;
        };
        if first != "samples" {
            continue;
        }
        let Some(second) = components.next().and_then(|c| c.as_os_str().to_str()) else {
            continue;
        };
        let category = category_of(&entry.id);
        if second != category {
            return Err(format!(
                "catalog entry `{}` has id prefix `{category}` but path `{}` lives under samples/{second}/",
                entry.id, entry.path
            ));
        }
    }
    Ok(())
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
    validate_unique_ids(&catalog.entries).unwrap_or_else(|e| panic!("{e}"));
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
            .any(|e| category_of(&e.id) == *category);
        assert!(
            has_entry,
            "category `{category}` has no catalog entry (need >=1 seed, stub OK)"
        );
    }
}

/// Category coverage above is only meaningful if `id` prefixes are drawn
/// from the fixed taxonomy — otherwise a typo'd or made-up prefix would
/// silently satisfy no real category while still "passing" other checks.
#[test]
fn every_entry_id_prefix_is_a_known_category() {
    let catalog = load_catalog();
    validate_known_categories(&catalog.entries).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn entry_path_category_matches_id_prefix_when_under_samples() {
    let catalog = load_catalog();
    validate_path_category_alignment(&catalog.entries).unwrap_or_else(|e| panic!("{e}"));
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
fn entry_paths_and_artifacts_are_safe_relative_paths() {
    let catalog = load_catalog();
    validate_safe_paths(&catalog.entries).unwrap_or_else(|e| panic!("{e}"));
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
fn samples_readme_has_required_sections() {
    let root = workspace_root();
    let readme = root.join("samples").join("README.md");
    let text = std::fs::read_to_string(&readme)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", readme.display()));
    for heading in ["## Taxonomy", "## Catalog", "## Contributing", "## CI"] {
        assert!(
            has_heading_line(&text, heading),
            "samples/README.md is missing required section {heading} as a heading line"
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
            has_heading_line(&text, "## Status"),
            "samples/{category}/README.md is missing a `## Status` heading line"
        );
    }
}

/// True if `text` has a line whose trimmed content is exactly `heading` —
/// a line-anchored match, so prose that merely *mentions* "## Status" in a
/// sentence or code fence can't spoof the check.
fn has_heading_line(text: &str, heading: &str) -> bool {
    text.lines().any(|line| line.trim() == heading)
}

/// The real gate: `ci: smoke` entries must actually typecheck (and, since
/// this drives the full `compile()` pipeline, lower successfully too) via
/// the built `quonc` binary — not just satisfy the YAML schema above. Also
/// the sole assertion that at least one `ci: smoke` entry exists (the
/// standalone `at_least_one_entry_opts_into_ci_smoke` test used to
/// duplicate this; folded in here so there's one clear assert).
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
        // Run from the repo root so both `entry.path` and any relative
        // `quonc_args` (e.g. `--target targets/...`) resolve the way a
        // contributor following samples/CONTRIBUTING.md's `quonc <args>
        // samples/<category>/<file>` recipe would expect — `cargo test`
        // otherwise runs test binaries with the crate dir as cwd, not the
        // workspace root.
        let output = Command::new(&quonc_bin)
            .current_dir(&root)
            .args(&entry.quonc_args)
            .arg(&entry.path)
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

/// Negative fixtures: prove the checks above actually fail closed on bad
/// catalog YAML, not just pass open on the real (already-valid) catalog.
mod negative_fixtures {
    use super::*;

    /// A minimal, fully valid single-entry catalog, used as a base that
    /// each negative test mutates just enough to trip one check.
    const VALID_CATALOG: &str = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;

    #[test]
    fn valid_fixture_parses_and_validates_clean() {
        let catalog: Catalog = serde_yaml::from_str(VALID_CATALOG).expect("fixture should parse");
        validate_unique_ids(&catalog.entries).expect("fixture should have unique ids");
        validate_known_categories(&catalog.entries).expect("fixture category should be known");
        validate_safe_paths(&catalog.entries).expect("fixture paths should be safe");
        validate_path_category_alignment(&catalog.entries)
            .expect("fixture path should align with its id prefix");
    }

    #[test]
    fn duplicate_id_fails_uniqueness_check() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
  - id: learning/example
    path: samples/learning/other.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_unique_ids(&catalog.entries)
            .expect_err("duplicate id must fail the uniqueness check");
        assert!(err.contains("duplicate catalog id"));
    }

    #[test]
    fn unknown_field_fails_to_parse() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
    not_a_real_field: oops
"#;
        let result: Result<Catalog, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "an unknown field must fail schema validation (deny_unknown_fields)"
        );
    }

    #[test]
    fn missing_path_fails_to_parse() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let result: Result<Catalog, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "a missing required `path` field must fail schema validation"
        );
    }

    #[test]
    fn bad_ci_enum_fails_to_parse() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: always
"#;
        let result: Result<Catalog, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "an invalid `ci` value must fail schema validation"
        );
    }

    #[test]
    fn bad_difficulty_enum_fails_to_parse() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: expert
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let result: Result<Catalog, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "an invalid `difficulty` value must fail schema validation"
        );
    }

    #[test]
    fn absolute_path_fails_safe_path_check() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: /etc/passwd
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_safe_paths(&catalog.entries)
            .expect_err("an absolute path must fail the safe-path check");
        assert!(err.contains("absolute"));
    }

    #[test]
    fn path_traversal_fails_safe_path_check() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/../../../etc/passwd
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_safe_paths(&catalog.entries)
            .expect_err("a `..` path-traversal component must fail the safe-path check");
        assert!(err.contains("path-traversal"));
    }

    #[test]
    fn path_traversal_in_artifacts_fails_safe_path_check() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: ["../outside-repo.png"]
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_safe_paths(&catalog.entries)
            .expect_err("a `..` in artifacts must fail the safe-path check");
        assert!(err.contains("artifacts"));
    }

    #[test]
    fn unknown_category_prefix_fails_known_category_check() {
        let yaml = r#"
version: 1
entries:
  - id: not-a-real-category/example
    path: samples/learning/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_known_categories(&catalog.entries)
            .expect_err("an id prefix outside the fixed taxonomy must fail");
        assert!(err.contains("unknown category prefix"));
    }

    #[test]
    fn mismatched_samples_path_category_fails_alignment_check() {
        let yaml = r#"
version: 1
entries:
  - id: learning/example
    path: samples/algorithms/example.qn
    tags: [learning]
    difficulty: beginner
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        let err = validate_path_category_alignment(&catalog.entries).expect_err(
            "a path under samples/<other-category>/ that disagrees with the id prefix must fail",
        );
        assert!(err.contains("but path"));
    }

    /// The `neutral-atom` -> `examples/na_qec/` link (ADR-0025) is the
    /// concrete case the alignment check must NOT flag.
    #[test]
    fn linked_examples_na_qec_path_is_exempt_from_alignment_check() {
        let yaml = r#"
version: 1
entries:
  - id: neutral-atom/example
    path: examples/na_qec/example.qn
    tags: [neutral-atom]
    difficulty: intermediate
    quonc_args: []
    artifacts: []
    ci: none
"#;
        let catalog: Catalog = serde_yaml::from_str(yaml).expect("schema-valid YAML should parse");
        validate_path_category_alignment(&catalog.entries)
            .expect("a linked examples/na_qec/ path must not be flagged");
    }
}
