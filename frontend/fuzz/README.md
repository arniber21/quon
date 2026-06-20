# frontend fuzz targets

Continuous fuzzing for the lexer and parser. Detached from the workspace (needs nightly +
libfuzzer), mirroring `backend/fuzz` and `mlir_bridge/fuzz`.

| Target | Invariant |
| ------ | --------- |
| `fuzz_lex` | `lex` never panics on arbitrary bytes (Ok/Err only). |
| `fuzz_parse` | `lex` then `parse` never panics on arbitrary text. |
| `fuzz_roundtrip` | A generated program prints, re-lexes, re-parses, and re-prints to byte-identical source — the precedence/associativity/desugaring oracle. |

## Running

Needs the local z3/LLVM env (see the repo build notes) because the targets link `frontend`:

```bash
export MLIR_SYS_220_PREFIX=$(brew --prefix llvm@22)
export BINDGEN_EXTRA_CLANG_ARGS="-I$(brew --prefix z3)/include"
export LIBRARY_PATH="$(brew --prefix z3)/lib"

cargo +nightly fuzz run fuzz_lex       -- -runs=100000
cargo +nightly fuzz run fuzz_parse     -- -runs=100000
cargo +nightly fuzz run fuzz_roundtrip -- -runs=100000
```

Seed corpora for `fuzz_lex` and `fuzz_parse` are the SPEC §12 reference programs under
`corpus/`.
