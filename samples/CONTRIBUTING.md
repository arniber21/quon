# Contributing a sample

## Before you start

1. Pick the right category from the taxonomy in [`README.md`](./README.md).
   If your story belongs to a pack with an owner issue (workflows #188,
   visualization #189, research #190, neutral-atom #192), coordinate there
   first — one canonical artifact per story; don't fork an existing `.qn`
   into a second copy.
2. If your program already has a canonical home (a `test/verify/*.qn`
   fixture or an `examples/na_qec/*.qn` file), link to it from your
   category's `README.md` instead of copying it into `samples/`.

## Steps

1. Add your file under `samples/<category>/` (usually a `.qn` program;
   `research/` may use a literate `.md`/notebook instead).
2. Add a row to [`catalog.yaml`](./catalog.yaml) with all required fields:

   ```yaml
   - id: <category>/<slug>
     path: samples/<category>/<file>
     tags: [ ... ]
     difficulty: beginner | intermediate | advanced
     quonc_args: []            # extra flags a reader should pass to quonc
     artifacts: []             # checked-in generated artifacts, if any
     ci: none                  # or `smoke` — see below
   ```

3. Add or update your category's `README.md` seeds table.
4. Decide on `ci`:
   - **`none`** (default) — no CI cost.
   - **`smoke`** — CI will run `quonc <your quonc_args> samples/<category>/<file>`
     and require it to exit successfully (a real typecheck, and — since
     `quonc` runs its full pipeline regardless of emit flags — a real
     lowering too). Only claim a numerical result (e.g. "Aer confirms...")
     if you also wire an Aer check; see `test/verify/` for the pattern.
5. Run the catalog checks locally:

   ```bash
   just ci-samples
   ```

## Schema and CI enforcement

`quonc/tests/samples_catalog.rs` (via `just ci-samples`, part of
`just test-ci`) enforces:

- `catalog.yaml` parses against the schema (unknown fields, missing
  required fields, or an invalid `ci`/`difficulty` value fail the build).
- Every `id` is unique.
- Every entry's `path` exists.
- Every top-level category has at least one entry.
- `samples/README.md` and every category `README.md` carry their required
  sections (see the test for the exact list).
- Every `ci: smoke` entry actually compiles with the release `quonc`
  binary.
