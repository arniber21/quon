//! Interim multi-file input for the circuit stdlib (issue #195).
//!
//! Quon has no `import` or module system. `--include` concatenates extra
//! `.qn` files in front of the main source and typechecks one program with a
//! flat top-level scope. This is not namespacing or separate compilation.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use frontend::ast::Decl;

/// One file's byte range inside a concatenated source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunk {
    pub path: PathBuf,
    /// Inclusive start offset into the concatenated buffer.
    pub start: usize,
    /// Exclusive end offset into the concatenated buffer.
    pub end: usize,
}

/// Join `parts` in order. A newline is inserted between adjacent files when
/// the previous file does not already end with one, so tokens cannot glue
/// across a boundary.
pub fn concatenate_sources(parts: &[(&Path, &str)]) -> (String, Vec<SourceChunk>) {
    let mut source = String::new();
    let mut chunks = Vec::with_capacity(parts.len());
    for (index, (path, text)) in parts.iter().enumerate() {
        if index > 0 && !source.ends_with('\n') {
            source.push('\n');
        }
        let start = source.len();
        source.push_str(text);
        let end = source.len();
        chunks.push(SourceChunk {
            path: path.to_path_buf(),
            start,
            end,
        });
    }
    (source, chunks)
}

/// Chunk containing `offset`. An offset equal to a non-empty chunk's `end`
/// (the last byte of that file) maps to that chunk.
pub fn chunk_at(chunks: &[SourceChunk], offset: usize) -> Option<&SourceChunk> {
    if let Some(chunk) = chunks
        .iter()
        .find(|chunk| offset >= chunk.start && offset < chunk.end)
    {
        return Some(chunk);
    }
    chunks
        .iter()
        .rev()
        .find(|chunk| chunk.start < chunk.end && offset == chunk.end)
}

/// Read `includes` then `main` into one Quon source.
///
/// An empty `includes` list reads `main` alone and returns no chunks, which
/// keeps single-file diagnostics on `main`. Duplicate canonical paths are
/// rejected. When any include is present, two top-level functions with the
/// same name, or two top-level type aliases with the same name, are rejected.
/// A function and a type alias may share a name; the typechecker keeps those
/// namespaces separate.
pub fn load_quon_sources(main: &Path, includes: &[PathBuf]) -> Result<(String, Vec<SourceChunk>)> {
    if includes.is_empty() {
        let source =
            fs::read_to_string(main).with_context(|| format!("reading {}", main.display()))?;
        return Ok((source, Vec::new()));
    }

    let mut paths: Vec<&Path> = includes.iter().map(PathBuf::as_path).collect();
    paths.push(main);

    let mut seen = HashSet::new();
    let mut owned: Vec<(PathBuf, String)> = Vec::with_capacity(paths.len());
    for path in paths {
        let canon = path
            .canonicalize()
            .with_context(|| format!("reading {}", path.display()))?;
        if !seen.insert(canon) {
            bail!(
                "--include lists {} more than once (same file as another input)",
                path.display()
            );
        }
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        owned.push((path.to_path_buf(), text));
    }

    let refs: Vec<(&Path, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_path(), text.as_str()))
        .collect();
    let (source, chunks) = concatenate_sources(&refs);
    if let Some(message) = duplicate_toplevel_message(&source, &chunks) {
        bail!("{message}");
    }
    Ok((source, chunks))
}

/// `Some` when `source` parses and two top-level functions, or two top-level
/// type aliases, share a name. A `fn` and a `type` with the same name are
/// legal. Parse failures return `None` so the compiler's diagnostic path can
/// report them.
pub fn duplicate_toplevel_message(source: &str, chunks: &[SourceChunk]) -> Option<String> {
    let decls = frontend::parse_program(source).ok()?;
    let mut fns: HashMap<String, usize> = HashMap::new();
    let mut types: HashMap<String, usize> = HashMap::new();
    for (decl, _) in &decls {
        let (seen, kind, name, span) = match decl {
            Decl::Fn { name, .. } => (&mut fns, "function", &name.0, name.1),
            Decl::TypeAlias { name, .. } => (&mut types, "type", &name.0, name.1),
        };
        if let Some(prev) = seen.insert(name.clone(), span.start) {
            return Some(format!(
                "duplicate top-level {kind} `{name}` ({} and {})",
                locate(source, chunks, prev),
                locate(source, chunks, span.start),
            ));
        }
    }
    None
}

fn locate(source: &str, chunks: &[SourceChunk], offset: usize) -> String {
    let Some(chunk) = chunk_at(chunks, offset) else {
        return format!("byte {offset}");
    };
    let local = offset.saturating_sub(chunk.start);
    let slice = source.get(chunk.start..chunk.end).unwrap_or("");
    let prefix = slice.get(..local).unwrap_or("");
    let line = prefix.matches('\n').count() + 1;
    format!("{}:{line}", chunk.path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concatenate_inserts_a_newline_between_files() {
        let a = PathBuf::from("a.qn");
        let b = PathBuf::from("b.qn");
        let (source, chunks) =
            concatenate_sources(&[(&a, "fn a(): Int = 1"), (&b, "fn b(): Int = 2")]);
        assert_eq!(source, "fn a(): Int = 1\nfn b(): Int = 2");
        assert_eq!(chunks[0].end, "fn a(): Int = 1".len());
        assert_eq!(chunks[1].start, chunks[0].end + 1);
        assert_eq!(
            chunk_at(&chunks, 0).map(|c| c.path.as_path()),
            Some(a.as_path())
        );
        assert_eq!(
            chunk_at(&chunks, chunks[1].start).map(|c| c.path.as_path()),
            Some(b.as_path())
        );
    }

    #[test]
    fn duplicate_toplevel_names_name_both_files() {
        let a = PathBuf::from("a.qn");
        let b = PathBuf::from("b.qn");
        let (source, chunks) =
            concatenate_sources(&[(&a, "fn a(): Int = 1\n"), (&b, "fn a(): Int = 2\n")]);
        let message = duplicate_toplevel_message(&source, &chunks).expect("duplicate name");
        assert!(
            message.contains("duplicate top-level function `a`"),
            "{message}"
        );
        assert!(message.contains("a.qn:1"), "{message}");
        assert!(message.contains("b.qn:1"), "{message}");
    }

    #[test]
    fn duplicate_type_aliases_name_both_files() {
        let a = PathBuf::from("a.qn");
        let b = PathBuf::from("b.qn");
        let (source, chunks) =
            concatenate_sources(&[(&a, "type Foo = Int\n"), (&b, "type Foo = Qubit\n")]);
        let message = duplicate_toplevel_message(&source, &chunks).expect("duplicate type");
        assert!(
            message.contains("duplicate top-level type `Foo`"),
            "{message}"
        );
        assert!(message.contains("a.qn:1"), "{message}");
        assert!(message.contains("b.qn:1"), "{message}");
    }

    #[test]
    fn function_and_type_alias_may_share_a_name() {
        let a = PathBuf::from("a.qn");
        let b = PathBuf::from("b.qn");
        let (source, chunks) =
            concatenate_sources(&[(&a, "type Foo = Int\n"), (&b, "fn Foo(): Int = 1\n")]);
        assert!(
            duplicate_toplevel_message(&source, &chunks).is_none(),
            "a function and a type alias with the same name are legal"
        );
    }

    #[test]
    fn repeated_include_path_is_rejected() {
        let dir = std::env::temp_dir().join(format!("quon-include-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let main = dir.join("main.qn");
        let lib = dir.join("lib.qn");
        fs::write(&main, "fn entry(): Int = helper()\n").expect("main");
        fs::write(&lib, "fn helper(): Int = 1\n").expect("lib");

        let err = load_quon_sources(&main, &[lib.clone(), lib]).expect_err("duplicate path");
        assert!(err.to_string().contains("more than once"), "{err}");

        let (source, chunks) =
            load_quon_sources(&main, &[dir.join("lib.qn")]).expect("distinct files");
        assert!(source.contains("fn helper"));
        assert!(source.contains("fn entry"));
        assert_eq!(chunks.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }
}
