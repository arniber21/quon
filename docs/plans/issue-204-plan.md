# Issue #204 — Deepen the Aer verify seam

**Branch:** `issue-204-aer-verify-seam`
**Scope:** the Python verification seam only (compile → dialect-normalize → simulate → oracle). Not #197's broader interop toolkit.

## Problem

`test/verify/*.py` and `python/noisy_fidelity.py` each import `python/quon_aer.py` for `compile_to_qasm` and `run`, but every other concern is duplicated or hidden:

- `_qiskit_qasm3_compat` — the regex that rewrites `c[i] == 1` to `c[i] == true` so `qiskit_qasm3_import` accepts quonc's spec-valid integer bit conditions — is a **private** function. `noisy_fidelity.py` already reaches across the module boundary to call `quon_aer._qiskit_qasm3_compat` directly, so the "hidden regex" the issue calls out is not even contained to one file.
- `clbit(key, k, nbits)` (Qiskit prints bits high-index-first) is copy-pasted verbatim in `bernstein_vazirani.py`, `routing.py`, and `teleport.py`, and the `key.replace(" ", "")` normalization it depends on is repeated ad hoc in six more scripts.
- `hellinger_fidelity` / `normalize` (counts → probabilities) are implemented once in `noisy_fidelity.py` and nowhere else, even though they are the natural general-purpose oracle for the single-expected-outcome scripts (`grover.py`, `ising.py`, `qft.py`), which instead hand-roll `count / SHOTS` threshold checks.
- Failure modes are not actionable: a missing `quonc` binary surfaces as a bare `FileNotFoundError: [Errno 2] No such file or directory: 'quonc'`; a missing `qiskit-qasm3-import` surfaces as a raw `MissingOptionalLibraryError` from deep inside `qiskit.qasm3.loads`, with no pointer to `python/requirements.txt` or the underscore/hyphen package-name confusion the issue names.
- `spin_glass_qaoa.py` calls `qiskit.qasm3.loads(qasm)` directly (no compat rewrite) for its optional parse-check — it happens not to need the rewrite today (no mid-circuit bit compares in that fixture), but that is an accident of the fixture, not a guarantee, and it's a second undocumented "raw pipe."

The result: the compat rewrite is unowned, untested, and unnamed; comparison logic is duplicated; and there is no single place documenting that raw `quonc --emit-qasm | qiskit` is unsupported.

## What "deep module" means here

Per the issue, the module's primary interface is: **Quon source (or QASM) + expected distribution → pass/fail.** Four adapters sit behind that seam:

1. **Compile** — resolve `QUONC`, invoke with consistent flags (`--emit-qasm`, `--target`, plus pass-through extra args for things like `--sabre-gamma`).
2. **Dialect normalize** — the Qiskit-importer adapter, named and unit-tested (was `_qiskit_qasm3_compat`).
3. **Simulate** — `AerSimulator` with shots/seed, and an optional noise model (needed by `noisy_fidelity.py`, which is otherwise reduced to hand-rolling its own `AerSimulator` call).
4. **Oracle** — shared bit-extraction/normalization helpers plus a Hellinger-fidelity-based `verify_distribution(source, expected, ...)` that implements the issue's literal interface for scripts whose acceptance criterion is genuinely "observed distribution close to an expected one."

Not every verify script fits `verify_distribution`'s exact-distribution shape (Bernstein-Vazirani/routing/teleport assert *exact*, all-shots bit recovery; Shor asserts reproducibility + an outcome subset, not a distribution; Bell asserts *zero* 01/10 leakage, which a fidelity threshold alone would not catch as strictly — see "Why not force everything through one function" below). Those scripts keep their bespoke pass/fail logic but stop reimplementing `clbit`/normalize/compile/simulate — they call the module for those, and the module remains the single owner of "compile + normalize + simulate + compare" as the acceptance criterion asks, without erasing algorithm-specific correctness invariants that predate this issue and are documented in each fixture's docstring.

### Why not force everything through one function

Hellinger fidelity between an empirical distribution `p` and a **point-mass** expected distribution `{key: 1.0}` reduces exactly to `p[key]` (every cross term with the zero-probability keys vanishes, leaving `sqrt(p[key] * 1.0)` squared). That means `grover.py`, `ising.py`, and `qft.py` — whose current oracle is precisely "count of one expected key / shots >= threshold" — can move to `verify_distribution(..., expected={key: 1.0}, min_fidelity=threshold)` with **zero behavioral change**. `bell.py`'s oracle is not point-mass (`{"00": 0.5, "11": 0.5}`) and additionally asserts *exactly* zero leakage into `01`/`10`, which a fidelity bound alone would tolerate at low leakage rates. Weakening that specific regression lock to fit a generic function would reduce test rigor for no benefit, so `bell.py` keeps its explicit leakage assertion, built from the module's `compile_to_qasm` / `run_on_aer` / `normalize_counts`.

## Design

### 1. `python/quon_aer.py` stays the module, deepened in place

The filename stays `quon_aer.py` — its CLI (`--shots`, `--seed`, source-or-stdin) is documented verbatim in `README.md` and the website (`quickstart.md`, `guides/backends.md`), and Aer genuinely is the one simulator adapter this module has today, so renaming it would force edits to user-facing docs for no functional gain and risks drifting from the still-accurate website copy. What changes is the module's internal shape: four labeled sections plus a small error hierarchy, replacing today's two-function file:

- **Errors:** `VerificationError` (base), `QuoncNotFoundError`, `QuoncCompileError`, `SimulationDependencyMissingError` (covers both missing `qiskit-aer` and missing `qiskit-qasm3-import`, naming the exact pip package and the hyphen/underscore distinction).
- **Compile:** `quonc_binary()`, `compile_to_qasm(source_file, target=None, extra_args=None)` — raises `QuoncNotFoundError` on `FileNotFoundError` from `subprocess.run`, `QuoncCompileError` (with captured stderr) on nonzero exit. `extra_args` lets `noisy_fidelity.py` pass `--sabre-gamma` without a second hand-rolled `subprocess.run`.
- **Dialect normalize:** `normalize_bit_int_conditions(qasm_src) -> str` — the renamed, public, docstringed, unit-tested compat rewrite. `load_circuit(qasm_src)` — the only supported bridge from quonc's QASM to a `qiskit.QuantumCircuit`: applies the rewrite, calls `qasm3.loads`, falls back to `measure_all()` for unitary-only circuits, and wraps `ImportError` from either `qiskit` or `qiskit_qasm3_import` into `SimulationDependencyMissingError`.
- **Simulate:** `run_on_aer(qasm_src, shots=4096, seed=None, noise_model=None) -> dict[str, int]` — builds on `load_circuit`; the `noise_model` parameter is new (absorbs `noisy_fidelity.py`'s duplicate `run_counts`).
- **Oracle:** `normalize_key`, `clbit` (dedups the three copies), `normalize_counts`, `hellinger_fidelity` (dedups `noisy_fidelity.py`'s copy), `VerifyResult` (small `ok`/`counts`/`fidelity`/`message` record, truthy on `ok`), `verify_distribution(source_or_qasm, expected, *, shots, seed, min_fidelity, target, is_qasm)` implementing the issue's literal interface.
- **CLI (`main`)** unchanged in behavior (`--shots`, `--seed`, source-or-stdin), reimplemented against the renamed public functions.

### 2. Refactor call sites

All ten `test/verify/*.py` scripts (`bell`, `bernstein_vazirani`, `grover`, `ising`, `qaoa`, `qft`, `routing`, `shor`, `spin_glass_qaoa`, `teleport`) keep `import quon_aer` (unchanged import line) but stop duplicating logic the module now owns:

- `bell.py`, `teleport.py`, `bernstein_vazirani.py`, `routing.py`, `shor.py`, `qaoa.py` — replace local `clbit`/`key.replace(" ", "")` duplicates with `quon_aer.clbit` / `quon_aer.normalize_key`. Each keeps its bespoke pass/fail assertion (exact secret recovery, strict Bell leakage, reproducibility + outcome subset, strict optimal-vs-non-optimal inequality) because none of those are a single "distribution close to one expected reference" comparison — see "Why not force everything through one function" above.
- `spin_glass_qaoa.py` — its optional Qiskit parse-check switches from raw `qasm3.loads(qasm)` to `quon_aer.load_circuit(qasm)`, so it goes through the same normalize step as every simulated script (removing the second undocumented raw pipe) and gets the actionable dependency error for free.
- `grover.py`, `ising.py`, `qft.py` — switch to `quon_aer.verify_distribution(SOURCE, expected={key: 1.0}, shots=SHOTS, seed=SEED, min_fidelity=threshold)`, proven behaviorally identical above. `qft.py` keeps its separate structural (`non_native_ops`) check, which is unrelated to the Aer oracle.
- `python/noisy_fidelity.py` — drop the reach-through to the (soon-public) compat function and the local `hellinger_fidelity`/`normalize` copies in favor of `quon_aer.normalize_bit_int_conditions` / `.hellinger_fidelity` / `.normalize_counts`; `run_counts` becomes a thin call to `quon_aer.run_on_aer(..., noise_model=...)`; `compile_with` becomes a call to `quon_aer.compile_to_qasm(..., extra_args=[...])` instead of a second hand-rolled `subprocess.run`.
- `test/verify/run_e2e.sh` — no change; it already just shells out to each `.py` file's `main`.

### 3. Unit tests

New `python/test_quon_aer.py`, next to the module it tests (stdlib `unittest`, no new dependency — `python -m unittest` needs nothing beyond the interpreter for these; the handful of tests that need `qiskit`/`qiskit-aer` are skipped, not failed, when those imports are unavailable, mirroring `noisy_fidelity.py`'s own optional-dependency handling):

- `normalize_bit_int_conditions`: integer `==`, one-hot `c[i] == 0` and `== 1`, multiple conditions in one line, does not touch whole-register integer comparisons (e.g. `c == 3`, no `[index]`), idempotent on already-boolean input. (The rewrite is a purely syntactic regex on `identifier[index] == 0|1`, same as the code it replaces — it does not and cannot distinguish a `bit` register from any other indexed integer array by name alone; quonc only ever emits this pattern for bit conditions in practice, so this is a documented, unchanged limitation, not a new one.)
- `clbit` / `normalize_key`: high-index-first extraction, tolerates Qiskit's space-separated multi-register keys.
- `hellinger_fidelity` / `normalize_counts`: identical distributions → 1.0; disjoint supports → 0.0; point-mass reduction (the algebraic fact the plan's "why not force everything" section relies on) — this is the regression lock for that equivalence, so a future change to either function can't silently break `grover`/`ising`/`qft`.
- `QuoncNotFoundError` / `SimulationDependencyMissingError` message content (actionable-error contract), driven by monkeypatching `subprocess.run` / import machinery rather than requiring a real missing binary.

### 4. README

The "Simulate with Qiskit Aer" section already names `python/quon_aer.py` and its filename doesn't change, so add one explicit sentence rather than rewriting the section: piping `quonc --emit-qasm` straight into `qiskit.qasm3.loads` (or any other raw Qiskit entry point) without going through `quon_aer`'s dialect-normalize step is unsupported, because quonc's spec-valid `bit[i] == 1` integer conditions are rejected by `qiskit_qasm3_import`'s indexed-bit grammar.

## Validation

- `python -m unittest python/test_quon_aer.py` (stdlib, always runnable).
- `QUONC=target/release/quonc python test/verify/bell.py` (and at least one more script) against a locally built `quonc`, in a venv with `python/requirements.txt` installed, to confirm the refactor is behavior-preserving end-to-end.
- Re-run the full `test/verify/*.py` sweep if time allows, to catch any accidental behavior change in the `grover`/`ising`/`qft` migration.
- `cargo fmt --check` / `clippy` / `cargo test --workspace --exclude flux_verify` — unaffected by a Python-only change, but run to confirm no incidental drift (e.g. from an editor auto-format) crept into tracked Rust files.
- `npx @taskless/cli@latest check` on changed files — expected no findings; every existing rule targets Rust (`unwrap`/`expect`, `anyhow`, serde DTOs, MLIR builders).

## Edge cases

- `QUONC` unset and no `quonc` on `PATH`: `QuoncNotFoundError` names both the env var and the build command.
- `qiskit` installed but `qiskit-qasm3-import` is not: `SimulationDependencyMissingError` names the exact pip package (hyphens) and the import name (underscores) so the issue's stated confusion can't recur silently.
- Circuit with zero classical bits (pure-unitary fixtures, if any are added later): `load_circuit` keeps the existing `measure_all()` fallback.
- `qft.py`'s structural "did the round trip cancel" check is independent of the Aer run and is left untouched.
- `spin_glass_qaoa.py` has no Aer run at all (compile-shape test); only its optional parse-check moves onto the shared `load_circuit`.

## Risks and mitigations

- **Silent behavior change in `grover`/`ising`/`qft`:** mitigated by the point-mass-reduction unit test plus running those three scripts locally before/after.
- **Removing the private `_qiskit_qasm3_compat` name breaks an external caller:** it is underscore-prefixed (no public contract) and every in-repo caller (`noisy_fidelity.py`) is updated in the same change; nothing outside the repo is documented as depending on it.
- **Scope creep toward #197:** no new simulator backend, no new CLI, no new target/descriptor changes, no filename rename — only the four named adapters plus tests and docs, inside the file that already owned two of the four.

## Expected diff shape

- `python/quon_aer.py` deepened in place (same filename, same CLI, extended internals).
- New `python/test_quon_aer.py`.
- Edits to all ten `test/verify/*.py` scripts (excluding `run_e2e.sh`) and `python/noisy_fidelity.py`.
- `README.md` Aer section (naming the module's now-complete responsibility and the unsupported-raw-pipe rule).
- This plan and its adversarial review as execution records.
- No changes to `website/` — its documented CLI surface (`python/quon_aer.py <source> --shots --seed`, stdin mode) is unaffected by internal restructuring.

## Implementation sequence

1. Record adversarial plan review and resolve every blocking finding.
2. Deepen `python/quon_aer.py` in place.
3. Write `python/test_quon_aer.py`.
4. Refactor every `test/verify/*.py` script and `python/noisy_fidelity.py`.
5. Update `README.md`.
6. Validate: unit tests, then at least one real Aer run against a locally built `quonc`.
7. Adversarial code review pass.
8. Commit on `issue-204-aer-verify-seam` referencing #204.
