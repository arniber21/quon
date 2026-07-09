# Issue #144 — Examples cookbook

**Branch:** `issue-144`  
**Blocked by:** #137 (site IA), #139 (language guide)  
**Scope:** Bell, teleportation, Bernstein–Vazirani, Grover, QFT, Ising, QAOA, and the Shor kernel

## Goal

Add a copy-pasteable Starlight cookbook whose eight pages show the exact Aer-verified Quon programs, the commands that compile and verify them, and a short explanation of each expected result.

## Repository evidence

- `test/verify/<name>.qn` is the executable source of truth for all eight programs.
- `test/verify/<name>.py` compiles with `quonc`, runs Qiskit Aer with a fixed seed, and asserts the documented outcome.
- `frontend/tests/fixtures/` contains parser/typechecker reference forms, but several are more general than the concrete end-to-end programs.
- `test/verify/qft.qn`, `ising.qn`, `qaoa.qn`, and `shor.qn` document deliberate verification boundaries that the cookbook must preserve.
- The website currently has only `index.mdx`; #137 and #139 have not landed.

## Design

### Routes and navigation

Create `website/src/content/docs/cookbook/` with an index and one page per program. Add a single explicit `Cookbook` sidebar group in `astro.config.mjs`. Do not implement the broader #137 navigation.

### Source ownership

Each MDX page imports its matching `test/verify/*.qn` file with Vite's `?raw` loader and renders it with Astro's `Code` component. This provides:

1. exact checked-in, tested source rather than a prose copy;
2. verbatim, selectable source in the built site without duplicating it;
3. build-time failure if a fixture is renamed or removed.

Each page also links to the Quon fixture, Python verifier, and related frontend fixture in GitHub.

### Commands

Every page provides:

```sh
./target/release/quonc test/verify/<name>.qn --emit-qasm > /tmp/<name>.qasm
QUONC=target/release/quonc python test/verify/<name>.py
```

The first command demonstrates direct compilation. The second is the repository's authoritative compile-and-simulate check.

### Language-guide links

Pages link concepts to `/language/` fragments expected from #139. Because #139 is still open, this slice owns only the links, not the guide destinations. Link labels name concrete concepts such as circuits, `run`, composition, linear registers, parameterized loops, recursion, and value-dependent types.

## Program-specific outcomes

1. **Bell:** only `00` and `11`, approximately 50/50.
2. **Teleportation:** recovered `|1>` and `|+>` exceed 99% fidelity.
3. **Bernstein–Vazirani:** query bits recover secret `(1, 1, 0)` on every shot.
4. **Grover:** marked state `11` exceeds 90% probability (the exact two-qubit case reaches 1.0 ideally).
5. **QFT:** `qft |> adjoint(qft)` recovers `101` above 99%; histogram does not claim phase verification.
6. **Ising:** `t = 0` evolution recovers `0000` above 99%; this is an identity-boundary check.
7. **QAOA:** every K3 MaxCut=2 bitstring is more probable than `000` and `111`.
8. **Shor kernel:** fixed-seed output is reproducible and confined to `00`/`01`; explicitly state that schematic `modmul` is not full period finding.

## Implementation phases

1. Add this plan and adversarial review.
2. Add cookbook index, eight pages, and scoped sidebar entries.
3. Install website dependencies and run `pnpm build`.
4. Run all eight `test/verify` scripts against the release compiler.
5. Cross-check every embedded source import and documented threshold against fixtures/verifiers.
6. Run formatting, repository validation appropriate to changed files, and adversarial diff review.
7. Commit and submit through Graphite.

## Acceptance checks

- Exactly eight program pages appear under `/cookbook/`.
- Every page has source, source/verifier pointers, compile and Aer verification commands, expected behavior, and language-guide links.
- No program source is duplicated into the website.
- The Shor page does not overclaim factorization or period finding.
- `pnpm build` succeeds from `website/`.
- `./test/verify/run_e2e.sh bell teleport bernstein_vazirani grover qft ising qaoa shor` succeeds with the release compiler.

## Residual integration risk

#137 may later replace the sidebar structure, and #139 may choose different route fragments. Resolve those path-level conflicts when the prerequisite branches land; do not expand this PR into their scope.
