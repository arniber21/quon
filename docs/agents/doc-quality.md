# Documentation quality-gate contract

This page defines the public contract for documentation quality and how it is
mechanically enforced. Readers and contributors need to know which links,
examples, commands, diagnostic snapshots, artifact claims, and rendered pages
are checked locally and in CI, rather than treating prose as unverified. The
discipline is inspired by Swift's source-compatibility suite: protect real
user-facing material against regressions with a maintained, explicit corpus.

**Authoritative commands** (orchestrator source of truth — ADR-0012):

| Command | What it checks |
| ------- | -------------- |
| `just ci-docs-assert` | Runs `scripts/assert-validation-docs.sh`, which now includes the docs corpus validator (`scripts/assert-docs-corpus.py`). Local CI parity. |
| `just ci-website` | Starlight `pnpm build` under `website/` — confirms the site renders. |
| `just test-ci` | Local CI parity: `ci-rust` + `ci-tooling` + `ci-docs-assert`. |

CI: the `docs` job in `.github/workflows/ci.yml` runs `just ci-docs-assert`
(failures report the originating page and line) followed by `just ci-website`.

## What is checked

The manifest at `docs/doc-manifest.yaml` declares the checked corpus. Every
page listed there receives the page-level checks it declares, and every
non-default fence receives its per-fence check:

### Page-level checks

- **`commands`** — every `sh`/`bash` fenced block is scanned:
  - `just <recipe>` — the recipe must exist as a public recipe in the
    `Justfile` (private recipes, `set`/`export` directives, and `:=`
    assignments are excluded).
  - `./scripts/<file>` — the script must exist in the repo.
  - Repo-relative path arguments (first component a known top-level directory
    like `test/`, `samples/`, `website/`) must resolve. Illustrative
    placeholders like `src/main.qn` or `path/to/config.toml` are not flagged
    because their first component is not a repo directory.
- **`links`** — every Markdown link and reference-definition URL:
  - Internal relative links (`./sibling/`, `../sibling/`, `/section/`) must
    resolve to a `.md`/`.mdx` file or a directory containing doc pages
    (Starlight file-path semantics — relative links resolve against the source
    file's directory, not its URL).
  - `https://github.com/arniber21/quon/blob/<ref>/<path>` links (the
    "artifact claims" — links to source fixtures) must resolve to a
    checked-in file.
  - External non-GitHub URLs and `mailto:` are not fetched.

### Per-fence checks (the four modes)

Every fenced code block on a declared page falls into one of four modes.
Fences not listed in the manifest are **illustrative** by default.

#### `executable` — real Quon that compiles

The fence's code, with `--` line comments stripped and whitespace collapsed,
must be a substring of a canonical tested fixture (`source` field). The
fixture is compiled and lowered by `just ci-rust` (the `cargo test --workspace`
step, or the sample catalog smoke test). This is the source-compatibility
discipline: the doc cannot drift from the tested source without the gate
failing.

If the fence elides intermediate definitions (a blank line stands in for
omitted code), each blank-line-separated chunk is checked independently — so
a curated excerpt that skips helper functions still passes, but drift in any
shown fragment is caught.

Example manifest entry:

```yaml
- lang: qn
  ordinal: 1
  mode: executable
  verify: mirror
  source: test/verify/bell.qn
```

#### `generated` — tool output (MLIR, QASM, JSON reports)

The fence must declare a `regen` field naming the command or `just` recipe
that regenerates the output. Byte-fidelity is not asserted for excerpts that
contain elisions or prose annotations (common in `mlir` fences that show
schematic "before/after optimization" IR); the `regen` declaration ensures a
maintainer can reproduce the output. A `golden` field may optionally name a
checked-in artifact for future byte-exact comparison.

```yaml
- lang: json
  ordinal: 2
  mode: generated
  regen: "devbox run -- cargo run -p quonc --emit-qec-validation"
```

#### `stale` — intentionally-retained historical material

Material kept for historical context (e.g. a deprecated CLI invocation, an
old IR shape) must declare a `reason` so stale prose is explicit, not
accidental. The gate does not verify the content; it only enforces that the
reason is recorded.

```yaml
- lang: bash
  ordinal: 1
  mode: stale
  reason: "Pre-#297 routing invocation; kept to document the OOM that motivated the A* rewrite."
```

#### `illustrative` (default) — pedagogical pseudocode and schematics

Hand-written pseudocode, schematic IR, type-error examples, and pedagogical
fragments not claimed to compile. These are **not** mechanically verified —
they carry no correctness contract. Listing them in the manifest is optional
(the default); the manifest exists to record what IS checked, not to enumerate
every illustrative fence.

This includes the `kotlin`-fenced fragments in the language reference (syntax
illustrations), the `mlir` "schematic" before/after blocks in cookbook pages,
and the `clone`/`leak` type-error examples in the linearity-borrow lesson.

## The manifest

`docs/doc-manifest.yaml` is the maintained registry. It records:

- Every page in `website/src/content/docs/` and the page-level checks applied.
- Every `executable`, `generated`, and `stale` fence with its verification
  mode and source/regen/reason.

A fence not listed is illustrative. The manifest is the single source of truth
for what the gate enforces — `just ci-docs-assert` validates exactly the
declared corpus, and a failure reports the originating page and line.

## Contributor workflow: adding a checked example

1. **Write or update the doc page** under `website/src/content/docs/`.

2. **If the example is real Quon that should compile:**
   - Add or identify the canonical fixture (a `test/verify/*.qn`, `test/na/*.qn`,
     `samples/**/*.qn`, or `frontend/tests/fixtures/*.qn` file) that the fence
     mirrors. The fixture must be compiled by `just ci-rust`.
   - Add a manifest entry under the page:
     ```yaml
     - lang: qn
       ordinal: <1-based index among qn fences on the page>
       mode: executable
       verify: mirror
       source: <repo-relative path to the fixture>
     ```
   - If the fence is a non-contiguous excerpt (elides code with blank lines),
     the mirror check handles it automatically — each chunk is checked.

3. **If the example is generated tool output:**
   - Add a manifest entry with `mode: generated` and a `regen` field naming
     the command (e.g. `devbox run -- cargo run -p quonc --emit-qasm ...`) or
     `just` recipe that reproduces it.
   - Optionally add a `golden` field pointing at a checked-in artifact.

4. **If the example is intentionally stale historical material:**
   - Add a manifest entry with `mode: stale` and a `reason` field.

5. **Run the gate locally:**
   ```bash
   just ci-docs-assert
   ```
   A failure reports the page and line. Fix the drift or adjust the manifest.

6. **Commit** the page, the fixture (if new), and the manifest together.

### Tips

- To find the ordinal of a fence, count `qn` (or other lang) fences from the
  top of the page, 1-based. Run the validator — a wrong ordinal produces a
  "not found in page" failure.
- The mirror check strips `--` comments and collapses whitespace, so
  formatting differences are tolerated. A fence that shows a subset of a
  fixture's functions (with blank-line elision) also passes.
- New pages are automatically picked up for `commands` and `links` checks —
  add them to the manifest's `pages` list with `checks: [commands, links]`.
