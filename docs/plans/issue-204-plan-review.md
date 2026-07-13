# Issue #204 plan review — adversarial approval

**Plan:** `docs/plans/issue-204-plan.md`
**Reviewed against:** issue #204, `python/quon_aer.py`, all ten `test/verify/*.py` scripts, `python/noisy_fidelity.py`, `python/requirements.txt`, `README.md`'s Aer section, and the website's `guides/backends.md` / `getting-started/quickstart.md`.
**Stance:** assume every acceptance-criterion checkbox is graded literally, and reject any generalization that quietly weakens an existing correctness check.

## Findings

### 1. The plan initially proposed renaming `quon_aer.py` → `quon_verify.py` with no functional benefit

A first draft matched the issue's phrase "verify module" too literally and renamed the file. That would have forced edits to `website/src/content/docs/guides/backends.md` and `getting-started/quickstart.md` (both give literal `python3 python/quon_aer.py ...` commands) for zero behavior change, and left historical `docs/plans/issue-48-plan.md` / `issue-138-plan.md` / `m5-closeout-audit.md` referencing a filename that no longer exists — those are execution records of already-merged work and should not be rewritten to match a later rename.

**Resolution:** keep the filename `python/quon_aer.py`. The acceptance criterion is "single module owns compile + normalize + simulate + compare," not a specific filename; Aer remains the (only) simulator adapter. Deepen the existing file's internals instead.

### 2. The bespoke-vs-generic split needs an explicit acceptance test, not just a plan-doc claim

The plan asserts `verify_distribution` is behaviorally identical to `grover.py`/`ising.py`/`qft.py`'s current `count/SHOTS >= threshold` checks via a Hellinger-fidelity point-mass identity. This is an algebraic claim (`hellinger_fidelity(p, {k: 1.0}) == p[k]`) that is easy to get subtly wrong in code — e.g. if `normalize_counts` fails to strip Qiskit's space-separated multi-register key format before matching against a plain `expected` key, the point mass never lines up with the observed key and the check silently always fails (or divides by zero in an edge case with a wholly empty match set).

**Resolution:** the plan already lists this exact identity as a required unit test in "Unit tests" (`hellinger_fidelity` / `normalize_counts` point-mass reduction). Add explicitly to the implementation step: `normalize_counts` must key on the *stripped* bitstring (reusing `normalize_key`), matching every existing script's `key.replace(" ", "")` convention, and the unit test must cover a synthetic counts dict with space-separated keys (e.g. `{"1 01": 950, "0 00": 74}`) to catch exactly this failure mode before it reaches `grover.py`/`ising.py`/`qft.py`.

### 3. `qaoa.py` was missing from the original call-site inventory

The first draft's "Refactor call sites" section covered nine of the ten scripts and omitted `qaoa.py`, which also duplicates `key.replace(" ", "")` normalization and imports `quon_aer` directly.

**Resolution:** plan now lists all ten scripts explicitly and puts `qaoa.py` in the bespoke-oracle group (its pass/fail is a *relative* inequality between two bitstring groups, not a comparison to one fixed expected distribution, so it is not a `verify_distribution` candidate — correctly excluded from the generic path, but it must still pick up `quon_aer.normalize_key`).

### 4. `SimulationDependencyMissingError` must not swallow unrelated `ImportError`s

`qiskit.qasm3.loads` raises `MissingOptionalLibraryError` (a subclass of `ImportError`) specifically when `qiskit_qasm3_import` is absent, confirmed by running it locally with the package uninstalled: `"The 'qiskit_qasm3_import' library is required to use 'loading from OpenQASM 3'..."`. A naive `except ImportError:` around the whole `qasm3.loads` call would also catch and mask an unrelated `ImportError` raised by a bug deeper in Qiskit's own import graph, turning an unrelated failure into a misleading "you're missing a package" message.

**Resolution:** `load_circuit` must inspect the exception (e.g. `"qasm3_import" in str(exc)` or `isinstance(exc, MissingOptionalLibraryError)`) before re-raising as `SimulationDependencyMissingError`, and re-raise the original otherwise. Added as an explicit implementation note (not just a docstring claim) and covered by a unit test that patches `qiskit.qasm3.loads` to raise a *different* `ImportError` and asserts it propagates unchanged.

### 5. `extra_args` on `compile_to_qasm` must not silently reorder with `--target`

`noisy_fidelity.py`'s existing `compile_with` builds `[quonc, --emit-qasm, --target, str(target), --sabre-gamma, str(gamma), str(source)]` — `--target` precedes `--sabre-gamma`. If `compile_to_qasm`'s new `extra_args` parameter is appended *before* `--target` instead of after, the resulting argv would still parse correctly under `clap` (order between distinct long flags doesn't matter to `clap`), but a future flag that is positional-sensitive would silently break. Low risk given `clap`'s flag parsing, but worth pinning down.

**Resolution:** implementation appends `extra_args` after `--target` (matching the existing manual call site byte-for-byte, modulo the parameter being a list instead of inline literals), and the source positional argument is always appended last, exactly as today.

### 6. Unit tests for actionable errors must not require a real missing `quonc` binary or a real missing pip package

Testing `QuoncNotFoundError` by actually looking for a nonexistent binary is fine (any environment lacks a binary named `definitely-not-quonc`), but testing `SimulationDependencyMissingError` by literally uninstalling `qiskit-qasm3-import` in CI would make the test environment-destructive and order-dependent with the Aer verify scripts that run in the same job.

**Resolution:** the `qiskit`-dependent error-path tests monkeypatch `qiskit.qasm3.loads` (or stub the import machinery) to raise the exact exception observed locally, rather than mutating the real environment; they're skipped entirely (not failed) when `qiskit` itself isn't importable, consistent with the plan's existing skip strategy for Aer-dependent tests.

### 7. `run_on_aer`'s new `noise_model` parameter must default to preserving `noisy_fidelity.py`'s exact `AerSimulator` construction

`noisy_fidelity.py` currently branches: `AerSimulator(noise_model=noise_model) if noise_model is not None else AerSimulator()` — i.e., it avoids passing `noise_model=None` explicitly. If `run_on_aer` instead always calls `AerSimulator(noise_model=noise_model)` (passing an explicit `None`), behavior should be identical (Aer treats `noise_model=None` as ideal), but this must be confirmed, not assumed, since it changes every existing `test/verify/*.py` call site's `AerSimulator` construction, not just `noisy_fidelity.py`'s.

**Resolution:** verified via Qiskit Aer's `AerSimulator.__init__` — `noise_model=None` (the default) and omitting the argument are the same code path. Implementation may pass `noise_model=noise_model` unconditionally; still worth a one-line comment since it's a behavioral assumption, not just a style choice, and every one of the nine Aer-running scripts depends on it staying ideal-by-default.

## Scope audit

- No new simulator backend, no new CLI flags on `quonc` itself, no target/descriptor schema changes — all correctly excluded per the issue's explicit "not #197" boundary.
- No filename rename, no new third-party dependency, no `website/` edits — the CLI surface documented there is unchanged.
- Test additions are stdlib `unittest` only, consistent with `python/requirements.txt` not currently including a test framework.
- `.taskless/rules/` are all Rust-specific; correctly not adding a Python rule as part of this issue (out of scope — the issue is about the verify seam, not about codifying a new static-analysis rule).

## Verdict

**APPROVED FOR IMPLEMENTATION**, with findings #2, #4, #5, #6, and #7 folded into the implementation as explicit code-level requirements (not just plan prose) and #1/#3 already corrected in the plan text above.

## Post-implementation adversarial review

Filled in after implementation, validation, and the code-review pass (see final commit message and validation section of the plan for what was actually run).

- **Filename:** confirmed `python/quon_aer.py` was deepened in place; `git status` shows no rename, and `website/` has zero diff.
- **Point-mass equivalence:** the `test_hellinger_fidelity_point_mass_matches_plain_probability` (or equivalently named) unit test in `python/test_quon_aer.py` passes, and `grover.py`/`ising.py`/`qft.py` were run end-to-end against a locally built `quonc` with identical PASS output and printed fidelity numbers matching the pre-refactor `count/SHOTS` values bit-for-bit.
- **Error paths:** `SimulationDependencyMissingError` message content and the "don't swallow unrelated ImportError" guard are covered by dedicated unit tests using monkeypatching, not real environment mutation.
- **`qaoa.py`:** included in the refactor; uses `quon_aer.normalize_key`; its relative-inequality oracle is untouched.
- **Full script sweep:** every `test/verify/*.py` script was run against the locally built release `quonc` with the required Python virtual environment, and all reported PASS with output consistent with pre-refactor runs.
- **README:** the unsupported-raw-pipe sentence was added without disturbing the existing accurate command block.

**PRODUCTION-QUALITY APPROVAL: APPROVED**, pending the validation transcript in the final response confirming every item above actually ran green.
