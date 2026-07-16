# Workspace boundaries

What each crate owns, and the known boundary tensions to respect (or resolve
via the linked issues) before starting cross-crate refactors. Gate commands and
CI mapping live in [docs/agents/validation.md](agents/validation.md); the
domain glossary lives in [CONTEXT.md](../CONTEXT.md).

## Crate ownership

| Crate | Owns |
| --- | --- |
| `frontend` | Surface syntax: lexer, parser, AST, desugar, typechecker (linear + value-dependent + QEC kinds), lowering entrypoints (feature `full`) |
| `quonc` | CLI orchestration: argument surface, emit flags, verification gates, report plumbing |
| `quon_core` | MLIR-free shared kernels: `DepthExpr`, typed OpenQASM model, Flux-specced kernels |
| `mlir_bridge` | Melior dialect builders/verifiers, passes, metrics, OpenQASM emitter |
| `backend` | Target descriptors: topology, native gates, noise/error models, JSON loader |
| `quon_na` | Neutral-atom pipeline: interaction graph, placement, movement, zoned scheduling, compaction, `quantum.na` schedule dialect (ADR-0011), resource reports |
| `quon_qec` | QEC workload IR: code families, expansion, Stim/experiment emit |
| `zx` | ZX graph + rewrite engine |
| `quonfmt` / `quonlint` / `quonlint-cli` / `quon_lsp` | Source tooling over the frontend AST |
| `flux_verify` | Nightly-only Flux refinement examples (excluded from workspace gates; checked in `flux.yml`) |

## Boundary tensions

1. **No shared AST traversal.** `quonfmt`, `quonlint`, and `quon_lsp` each
   hand-roll exhaustive `Expr`/`Decl` matches, so every AST addition breaks
   all three independently — the QEC AST additions (ADR-0014) did exactly
   that (#276). A visitor/walker in `frontend` would turn future additions
   into one compile error in one place.
2. **`frontend` optionally depends on `mlir_bridge`** (feature `full`) so
   lowering entrypoints live behind the parser/typechecker. It works, but
   inverts the expected layering; #206 (Melior-free `SpecializedCircuit`
   between elaborate and lower) is the seam issue.
3. **The backend↔quon_qec↔quon_na triangle** around targets and error
   budgets is held together by type aliasing (`quon_qec::ErrorModelSnapshot`
   is re-exported/aliased by `backend`). Further consolidation belongs to
   #216 (quon_core packing) and #215 (NA monolith carving) — don't grow new
   snapshot types ad hoc.
4. **CLI verification glue** (`quonc/src/compile.rs` linearity /
   `quantum.na` verify gates) is thin and acceptable; growth should move
   behind a library seam per #201/#206.
5. **AST leakage: none.** `quon_na`, `quon_qec`, and `mlir_bridge` do not
   import `frontend::ast`; IR crates consume lowered forms only. Keep it
   that way.

Architecture-deepening work is tracked under the #201 epic (#205–#216).
