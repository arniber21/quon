# Issue #46 — `quonfmt`: canonical formatter + style spec

**Audience**: an agent (or human) implementing issue #46 in branch `issue-46-quonfmt`.
**Objective**: ship a deterministic Quon source formatter (`quonfmt` CLI + library) with a
documented style spec, golden tests, and idempotency guarantees. Formatting must preserve
program semantics; it may discard comments and insignificant whitespace.

Read first: `CLAUDE.md`, `docs/agents/code-quality.md`, `docs/agents/graphite.md`,
`SPEC.md` §2 (lexical/syntax), `frontend/src/pretty.rs`, `frontend/src/parser.rs`.

---

## 1. Verified current state

### What exists today

| Component | Role | Formatter-ready? |
| --------- | ---- | ---------------- |
| `frontend::parse_program` | Lex + parse → `Vec<Sp<Decl>>` | Yes — canonical entry point |
| `frontend/src/pretty.rs` | Roundtrip-faithful debug printer | **No** — over-parenthesizes every operator form |
| `frontend/tests/pretty_roundtrip.rs` | AST roundtrip via `pretty` | Tests debug printer, not canonical style |
| `frontend/tests/roundtrip_props.rs` | Proptest idempotency of `pretty` | Same — validates debug printer invariants |
| `frontend/tests/reference_algorithms.rs` | Insta snapshots of `pretty` output | Will need separate golden corpus for `quonfmt` |
| `frontend/tests/fixtures/*.qn` | 9 representative programs | Seed corpus for golden tests |
| `quonc` | Compiler driver CLI | Pattern for clap-based binary (`quonc/tests/cli.rs`) |

### Why `pretty.rs` is not the formatter

The module header states its contract explicitly:

```1:7:frontend/src/pretty.rs
// Pretty-printer — emits valid Quon source from an AST.
//
// The printer is *roundtrip-faithful*: `parse(lex(pretty(d)))` equals `d` up to spans
// (see frontend/tests/support). It achieves this by parenthesizing every operator/binding
// form uniformly, so precedence and associativity can never be misread on re-parse. The
// output is intentionally explicit rather than minimal — it backs the generative fuzzer
// (frontend/fuzz/fuzz_roundtrip) and doubles as a debug dumper.
```

Concrete divergences from a canonical formatter:

- **Parentheses**: wraps every non-atom operand (`atom()` → `({})` for most forms).
- **Layout**: no line wrapping; `for` loops are single-line; `match` arms use 4-space indent only.
- **Alignment**: no bind-column alignment (fixtures like `teleport.qn` use hand-aligned `<-`).
- **Composition**: `|>` chains are flat single-line regardless of length.
- **Comments**: never preserved (AST has no comment nodes — acceptable for v1).
- **Float literals**: uses `Debug` formatting (`float_str`) — deterministic but not a style choice.

The formatter must be a **separate code path** with precedence-aware, width-aware layout.
Keep `frontend::pretty` unchanged for fuzz/proptest/debug.

### Parser precedence (must drive paren elision)

From `frontend/src/parser.rs` (tight → loose):

1. Atoms, application (juxtaposition, same-line only)
2. `@` gate application
3. Unary `-`
4. `^` (right-assoc)
5. `* /` and `par { } * count` (left-assoc; `par` at multiplicative tier)
6. `+ -` (left-assoc)
7. Backtick-style application chain (method/UFCS desugaring tier)
8. `|>` composition (left-assoc, **bridges newlines**)
9. `: τ` ascription
10. Prefix forms: `fn(…) ->`, `let … in`, `if … then … else`, `match`, `for`, blocks

The formatter's parenthesization rules must mirror this table exactly.

---

## 2. Goals and non-goals

### Goals (acceptance criteria)

- [ ] `quonfmt <file>` rewrites files deterministically (same input → same output on every run).
- [ ] `quonfmt --check <file>` exits `0` if formatted, non-zero if not; prints diff-friendly message.
- [ ] Idempotency: `format(format(src)) == format(src)` (byte-identical).
- [ ] Golden tests lock representative syntax forms.
- [ ] README + contributor docs describe formatter usage and CI integration path.

### Non-goals (v1)

- Comment preservation (line `--` and block `{- -}` are stripped — document explicitly).
- Format-on-save editor integration (document hook points; no VS Code plugin in this issue).
- Type-checking or desugaring during format (`parse_program` only, not `desugar_program`).
- Fixing parse errors (report diagnostics and exit non-zero, like `rustfmt` on bad syntax).
- `--emit-stdout` vs in-place write ambiguity beyond the rustfmt-like flags defined below.

### Semantic preservation contract

```
parse(src) = d
parse(format(d)) = d   (up to spans)
```

Comments and whitespace are not part of `d`. Formatting must never run desugar or
elaboration passes that would rewrite surface syntax (`run` → `bind`, etc.).

---

## 3. Style spec document outline

Create **`docs/quonfmt-style.md`** (normative spec referenced by tests). Suggested sections:

### 3.1 Scope and principles

- UTF-8 source; `\n` line endings (LF only in output; normalize CRLF on read).
- Deterministic: no user config file in v1 (fixed style constants in code).
- Parse-format-parse AST stability is the correctness oracle.

### 3.2 Lexical formatting

| Topic | Rule |
| ----- | ---- |
| Indentation unit | 4 spaces; no tabs |
| Line endings | LF (`\n`) |
| Trailing whitespace | Stripped |
| Final newline | Required (POSIX text file) |
| Max line width | 100 columns (Unicode scalar counts, not grapheme clusters) |
| Spaces around binary ops | `+ - * / ^`: one space each side |
| Spaces around `@` | one space each side (`H @0`, `CNOT @(0, 1)`) |
| Spaces around `\|>` | one space each side |
| Spaces around `<-` | one space each side |
| Spaces around `->` / `-o` | one space each side in types and lambdas |
| Unary `-` | space after minus: `- x` (matches lexer disambiguation from `-o`/ `->`) |
| Tuple/list commas | `,` + single space |
| No trailing comma | In single-line lists/tuples/parameter lists (match parser) |

### 3.3 Top-level declarations

- One blank line between declarations (`\n\n` separator).
- Function decl: `fn name(params): Ret = body` all on one line if body fits; otherwise break after `=`.
- Type alias: `type Name = …` or `type Name<n> = …`; break after `=` if needed.
- Parameter lists: break after `(`, one param per line, indent +4, closing `)` on its own line when broken.

### 3.4 Statement blocks: `circuit { }`, `run { }`, `borrow … in { }`

These forms use a **statement block** (`Vec<Sp<Stmt>>`): one statement per line, not a single
expression body. Do not apply statement-block rules to `par { }` or `for … { }` (see §3.4.1).

- Opening brace on same line as keyword (`circuit {`, `run {`, `borrow … in {`).
- One statement per line, indent +4.
- Closing brace on its own line, dedented to block header level.
- Empty block: `keyword {\n}` (no trailing space inside).

**`run { }` binds** (canonical layout):

```
run {
    (a, b) <- bell_state() @ (alice, bob)
    x      <- measure(a)
    return x
}
```

- `<-` alignment: when a block has 2+ bind statements, align `<-` columns to the longest LHS
  (measured in display width). Single bind: no padding.
- `let` inside run blocks: `let pat = rhs` (no alignment with `<-`).
- Final statement: `return expr` or bare expr (preserve stmt kind from AST).

**`borrow` blocks**:

```
borrow anc: Qubit in {
    body_stmt
    reset(anc)
}
```

- Bindings: comma-separated on one line if ≤100 cols; otherwise one binding per line.
- Body uses same stmt rules as `run` (but no `<-` alignment with `borrow` keyword line).

### 3.4.1 Expression blocks: `par { }`, `for … in … { }`

These forms use an **expression block** (`brace_expr` — a single `Sp<Expr>` inside braces),
not a statement list. The printer must not emit one-statement-per-line layout here.

| Form | Body AST | Layout rule |
| ---- | -------- | ----------- |
| `par { body } * n` | `Expr::Par(body, count)` | `par { expr } * count` on one line if ≤100 cols; else break after `{`, indent body +4, closing `}` on own line, `* count` on following line |
| `for p in iter { body }` | `Expr::For { …, body }` | `for p in iter { expr }` on one line if fits; else break before `{`, indent expr +4 inside braces |

Example (expression body, not statements):

```
fn tower(n: Nat): Circuit<n, n, 1, Clifford> = par { had_one() } * n
```

### 3.5 Composition (`|>`) layout

- Short chains (fit in 100 cols): single line (`a |> b |> c`).
- Long chains: break after each `|>`; continuation lines indent +4 with **leading `\|>`**
  at the start of each continuation line (locked — no trailing-op variant). This mirrors
  parser newline-bridging and matches hand-written fixtures:

  ```
  H @0
      |> CNOT @(0, 1)
      |> Rz(theta) @1
  ```

- Never insert/remove parens beyond precedence requirements.

### 3.6 Other expression forms

| Form | Rule |
| ---- | ---- |
| `par { body } * n` | Prefer single line; break before `*` if over width |
| `if c then t else e` | Single line if fits; break `then`/`else` branches each on own line when broken |
| `match e { … }` | Scrutinee + opening brace; one arm per line; arms indented +4; trailing comma on last arm **only if** parser accepts (currently no — omit) |
| `for p in iter { body }` | Expression block (§3.4.1): break before `{` if over width; single expr inside braces |
| `fn(p): T -> e` lambdas | Same breaking rules as top-level fn |
| `let p = rhs in body` | Single line if fits; else break after `in` |
| Types `Circuit<n,m,d,C>` | No spaces inside `<>` angle lists; break after `<` if over width |
| Nat/type arithmetic | Minimal parens per nat precedence (`+ -` looser than `* /` looser than `^`) |
| Floats | Shortest round-trippable literal (re-parse equals); prefer decimal with `.` |
| Ints | Decimal, no separators |
| `adjoint(…)` / `controlled(…)` | No space before `(` |

### 3.7 Comments (v1 policy)

- All comments removed on format. Document in style spec §3.7 and CLI `--help`.
- Future issue: comment trivia attachment in parser (non-goal here).

### 3.8 Configuration constants (code, not user-facing)

```rust
pub const INDENT: &str = "    ";
pub const MAX_WIDTH: usize = 100;
pub const DECL_SEP: &str = "\n\n";
```

### 3.9 Application and postfix forms (`Expr::App`)

The parser desugars all call/postfix surface syntax into nested `Expr::App` nodes. The AST has
**no** juxtaposition, index, backtick, or dot-method nodes — the formatter must define a
canonical re-surface policy.

| Surface syntax | Parser desugaring | Canonical print form |
| -------------- | ----------------- | -------------------- |
| Juxtaposition `f x` (same line) | `App(f, x)` | Prefer juxtaposition when both sides are atoms/postfix-safe and fit on one line; otherwise parenthesize per precedence |
| Call `f(a, b, …)` | curried `App(App(f, a), b)` | Uncurry to `f(a, b, …)` — comma-separated args in parens |
| Call `f()` | `App(f, Unit)` | `f()` |
| Index `q[i]` | `App(index, App(q, i))` | `index(q, i)` — always desugared name, no `[]` sugar recovery in v1 |
| Backtick `` x `f` y `` | `App(f, App(x, y))` | `f(x, y)` — no backtick sugar recovery in v1 |
| Method `x.m(a)` | `App(m, App(x, a))` | `m(x, a)` — no dot sugar recovery in v1 |

**Juxtaposition rules** (tightest tier — same-line only in parser):

- Emit `f x` (no parens) when `f` is atom/postfix-safe and `x` is atom/postfix-safe.
- Gate application `@` binds tighter than juxtaposition on the right: `H @0` not `H@0` with space rules from §3.2.
- Never emit juxtaposition across a line break; if a break is required, use `f(x)` or break at a higher tier (`|>`, etc.).
- When `App` appears as operand of `@`, `|>`, `+`, etc., parenthesize only when child precedence is looser than context (standard climb).

**Golden corpus**: add `application.qn` covering juxtaposition (`H q`), multi-arg calls,
`index(q, i)`, method/backtick desugar forms, and nested `App` under `@` and `|>`.

---

## 4. Crate structure

Add workspace member **`quonfmt/`**:

```
quonfmt/
├── Cargo.toml
├── README.md                 # short: usage, link to docs/quonfmt-style.md
├── src/
│   ├── lib.rs                # pub API: format_str, format_decls, check_str
│   ├── config.rs             # StyleConfig (fixed defaults; struct for test overrides)
│   ├── doc.rs                # Doc/DocBuilder (Wadler-style pretty algebra)
│   ├── print/
│   │   mod.rs
│   │   ├── decl.rs
│   │   ├── expr.rs           # precedence-aware expr printing
│   │   ├── ty.rs
│   │   ├── pat.rs
│   │   ├── stmt.rs
│   │   └── nat.rs
│   └── error.rs              # FormatError (parse diagnostics wrapper)
└── tests/
    ├── golden.rs             # insta golden corpus
    ├── idempotency.rs        # format(format(x)) == format(x) on fixtures
    ├── cli.rs                # subprocess tests (mirror quonc/tests/cli.rs)
    └── support/mod.rs        # parse_stripped re-export or copy from frontend/tests
```

**`Cargo.toml` dependencies**:

```toml
[package]
name    = "quonfmt"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "quonfmt"
path = "src/main.rs"

[dependencies]
frontend = { path = "../frontend", default-features = false, features = ["parser-only"] }
thiserror = { workspace = true }
clap      = { workspace = true }
anyhow    = { workspace = true }   # CLI driver only (quonc pattern)
ariadne   = { workspace = true }   # parse diagnostics in CLI

[dev-dependencies]
insta     = { workspace = true }
proptest  = { workspace = true }
arbitrary = { workspace = true }
```

Library crate uses `thiserror` for `FormatError`; CLI binary uses `anyhow::Result` per
code-quality conventions (`quonc` pattern).

CLI binary: either `[[bin]] name = "quonfmt"` in this crate (preferred) or a thin
`quonfmt-cli` — prefer single crate with lib + bin like `quonc`.

**Workspace root `Cargo.toml`**: add `"quonfmt"` to `[workspace].members`.

**Public library API** (stable for future editor integrations):

```rust
/// Parse and format source; returns formatted string or diagnostics.
/// Normalizes CRLF → LF, strips trailing whitespace, ensures final newline.
pub fn format_str(src: &str) -> Result<String, FormatError>;

/// Format already-parsed decls.
pub fn format_decls(decls: &[Sp<Decl>]) -> String;

/// Returns Ok(()) if `src` is already formatted; Err with unified diff hint otherwise.
/// Compares **normalized** representations (LF line endings, no trailing WS, required final
/// newline) — not raw input bytes. CRLF input that differs only by line endings passes.
pub fn check_str(src: &str) -> Result<(), FormatError>;
```

**Normalization helper** (shared by `format_str` and `check_str`):

```rust
fn normalize_for_compare(s: &str) -> String {
    // CRLF → LF, strip trailing whitespace per line, ensure single final newline
    …
}
```

---

## 5. Pretty-printer refactor plan

Do **not** repurpose `frontend/src/pretty.rs`. Instead:

### Phase A — Extract shared rendering helpers (optional, minimal)

If `quonfmt` needs literal rendering identical to the debug printer (floats, names),
extract **pure leaf helpers** into `frontend/src/render/lit.rs`:

- `render_float(f: f64) -> String`
- `render_int(n: i64) -> String`
- `binop_str`, `class_str`

Both `pretty.rs` and `quonfmt` import these. Keep diff small; only extract what diverges
today (float formatting is the main shared concern).

### Phase B — Implement `quonfmt` Doc printer

Use a **`Doc` algebra** (in `quonfmt/src/doc.rs`) rather than ad-hoc `format!` chains:

```rust
enum Doc {
    Nil,
    Text(String),
    Concat(Box<Doc>, Box<Doc>),
    Nest(isize, Box<Doc>),
    Break(BreakKind),   // flat | broken
    Group(Box<Doc>),    // try flat, else break
}
```

Benefits: deterministic layout, width-aware breaking, testable flatten pass.

**Precedence printing** (`quonfmt/src/print/expr.rs`):

```rust
enum Prec {
    Top,
    Compose,    // |>
    Ascribe,
    IfLet,
    Backtick,
    Add,
    Mul,        // includes par { } * n
    Pow,
    Neg,
    GateApp,    // @
    App,
    Atom,
}

fn print_expr(e: &Sp<Expr>, ctx: &mut Context, min_prec: Prec) -> Doc { … }
```

Parenthesize child only when `child_prec < min_prec` (standard precedence climb).

**Block alignment** (`quonfmt/src/print/stmt.rs`):

1. First pass: render each stmt as `Doc`.
2. For bind stmts in a `run`/`circuit` block, compute max LHS width.
3. Second pass: pad LHS + single space + `<-` column.

### Phase C — Leave `frontend::pretty` untouched

- Fuzz target `frontend/fuzz/fuzz_targets/fuzz_roundtrip.rs` keeps using `pretty`.
- `roundtrip_props.rs` continues testing debug printer idempotency.
- Add a one-line comment in `pretty.rs` pointing to `quonfmt` for canonical formatting.

### Phase D — Wire `frontend` as parser dependency (with build isolation)

`quonfmt` calls `frontend::parse_program` — no typecheck, no desugar, no lowering at
runtime. **However**, `frontend` today pulls `mlir_bridge`, `melior`, and `z3`
unconditionally — a path dependency on the full crate compiles the MLIR/LLVM/Z3 graph even
if `quonfmt` never calls lowering.

**Required (PR 1):** add a `frontend` feature `parser-only` (default off for existing
consumers) that gates out `lower`, `mlir_bridge`, `melior`, and optionally Z3/refinement
modules. `quonfmt` depends on `frontend` with `default-features = false, features = ["parser-only"]`.

Fallback if feature gate slips: document LLVM 22 + Z3 prerequisites in `quonfmt/README.md`
(same as workspace root) and remove any “No MLIR env required” messaging.

---

## 6. CLI design

Follow rustfmt conventions (familiar to contributors):

```
quonfmt [OPTIONS] [FILE]...

Options:
  -c, --check          Exit 1 if any file would change (no writes)
  -w, --write          Write formatted output back to files (in-place)
  -h, --help
  -V, --version
```

**Behavior matrix**:

| Invocation | stdout | files | exit code |
| ---------- | ------ | ----- | --------- |
| `quonfmt file.qn` | formatted text | unchanged | 0 (parse ok) |
| `quonfmt -w file.qn` | silent | rewritten | 0 |
| `quonfmt --check file.qn` | silent (or `--check` lists paths) | unchanged | 0 if formatted, 1 if not |
| CRLF input + `--check` | silent | unchanged | 0 if content is formatted modulo LF normalization |
| parse error | diagnostics to stderr | unchanged | non-zero (suggest 2) |
| no args | read stdin, write stdout | — | 0 |

**Diagnostics**: reuse `frontend::diagnostics` + `ariadne` like `quonc` does for parse
failures (path + source snippet).

**Implementation sketch** (`quonfmt/src/main.rs`):

```rust
#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    check: bool,
    #[arg(short, long)]
    write: bool,
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.files.is_empty() {
        let mut src = String::new();
        io::stdin().read_to_string(&mut src)?;
        return emit(&cli, &src, Path::new("-"), &src);
    }
    for path in &cli.files {
        let src = fs::read_to_string(path)?;
        emit(&cli, &src, path, &src)?;
    }
    Ok(())
}
```

**Tests** (`quonfmt/tests/cli.rs`): copy pattern from `quonc/tests/cli.rs` — help, version,
`--check` exit code, CRLF input passes `--check` after normalization, unknown flags, missing file.

---

## 7. Golden test corpus

### Directory layout

```
quonfmt/tests/corpus/
├── input/                    # unformatted (intentionally messy) sources
│   ├── decls.qn
│   ├── circuit_compose.qn
│   ├── run_binds.qn
│   ├── borrow.qn
│   ├── types.qn
│   ├── expr_precedence.qn
│   ├── par_repeat.qn
│   ├── match_if.qn
│   └── lambdas.qn
└── expected/                 # insta-managed snapshots (generated on first run)
    └── *.snap
```

### Corpus cases (minimum)

| File | Covers |
| ---- | ------ |
| `decls.qn` | fn + type alias, blank-line separation |
| `circuit_compose.qn` | `circuit { }`, `@`, short and long `\|>` chains |
| `run_binds.qn` | `<-` alignment, `return`, inner `let` |
| `borrow.qn` | single/multi binding, terminal `reset`/`discard` |
| `types.qn` | `Circuit<…>`, `Q<…>`, `-o`, `Matrix`, nat arithmetic |
| `expr_precedence.qn` | mixed `+ * ^ @ \|>` needing minimal parens |
| `par_repeat.qn` | `par { } * n`, nested |
| `match_if.qn` | `match`, `if then else`, `for` |
| `lambdas.qn` | `fn(…) ->`, application, ascription |
| `application.qn` | juxtaposition, multi-arg calls, `index`/method/backtick desugar forms |

### Also run formatter on existing fixtures

In `quonfmt/tests/golden.rs`, add parameterized tests over
`frontend/tests/fixtures/*.qn` (9 files). Snapshots live under `quonfmt/tests/snapshots/`.
These differ from `reference_algorithms.rs` insta snapshots (which capture debug `pretty`).

### Test harness

```rust
#[test]
fn golden_circuit_compose() {
    let input = include_str!("corpus/input/circuit_compose.qn");
    let formatted = quonfmt::format_str(input).expect("parse");
    insta::assert_snapshot!("circuit_compose", formatted);
    // AST stability
    assert_ast_stable(input, &formatted);
}
```

`assert_ast_stable`: parse both, `strip_decls`, compare (reuse `frontend/tests/support` via
`#[path]` or move strip helpers to `frontend/tests/support` exported for integration tests).

---

## 8. Idempotency strategy

Three layers (all required):

### Layer 1 — Unit/property on formatter

`quonfmt/tests/idempotency.rs`:

```rust
#[test]
fn idempotent_on_corpus() {
    for input in corpus_inputs() {
        let once = format_str(input).unwrap();
        let twice = format_str(&once).unwrap();
        assert_eq!(once, twice, "idempotency failed for {name}");
    }
}
```

### Layer 2 — Proptest (reuse generator)

Mirror `frontend/tests/roundtrip_props.rs` but call `quonfmt::format_str`:

```rust
proptest! {
    #[test]
    fn format_is_idempotent(bytes in …) {
        if let Ok(decls) = generator::arb_program(&mut u) {
            let src = frontend::pretty::pretty(&decls); // valid source seed
            let f1 = quonfmt::format_str(&src)?;
            let f2 = quonfmt::format_str(&f1)?;
            prop_assert_eq!(f1, f2);
        }
    }
}
```

Using debug `pretty` output as seed is intentional: it generates valid parseable programs
with arbitrary AST shapes.

### Layer 3 — AST stability (semantic preservation)

For every corpus file and proptest case:

```
strip(parse(src)) == strip(parse(format(src)))
```

This is **stricter than idempotency** and catches precedence paren bugs.

### CI assertion

Add to pre-PR checklist in PR description (and later `docs/agents/code-quality.md`):

```bash
cargo test -p quonfmt
```

With `parser-only` feature gate (§5 Phase D): no MLIR/LLVM env required for formatter tests.
Without the gate: same LLVM/Z3 toolchain as the rest of the workspace.

---

## 9. Documentation updates

| File | Change |
| ---- | ------ |
| `docs/quonfmt-style.md` | **New** — full style spec (§3 expanded) |
| `README.md` | Add `quonfmt` to workspace table; Usage section with examples |
| `docs/agents/code-quality.md` | Add `cargo test -p quonfmt` to optional fast checks |
| `docs/agents/validation.md` | Note formatter is separate from `cargo fmt` (Rust vs Quon) |
| `quonfmt/README.md` | Quick start, link to style spec |

**README usage snippet**:

```bash
# Format to stdout
quonfmt program.qn

# Format in place
quonfmt -w program.qn

# CI check
quonfmt --check program.qn
```

**Contributor note**: Rust code uses `cargo fmt`; Quon sources use `quonfmt`. Do not
conflate the two in CI (future workflow addition is out of scope unless requested).

---

## 10. Recommended execution order (Graphite stack)

| PR | Scope | Depends on |
| -- | ----- | ---------- |
| 1 | Scaffold `quonfmt` crate: `Doc` algebra, `format_decls` stub, CLI skeleton, `--help` tests, `frontend` `parser-only` feature | — |
| 2 | Style spec doc + leaf printers (types, nats, pats, literals) + golden `types.qn` | 1 |
| 3 | Expr printer with precedence + `expr_precedence.qn` / `circuit_compose.qn` goldens | 2 |
| 4 | Stmt/block printer: `circuit`/`run`/`borrow`, bind alignment + goldens | 3 |
| 5 | Decl printer, top-level driver, `--check` / `-w`, full CLI tests | 4 |
| 6 | Idempotency proptest + fixture snapshots + README/docs | 5 |

Each PR should keep `cargo test -p quonfmt` green and not break existing `frontend` tests.

---

## 11. Validation checklist (pre-submit)

```bash
cargo fmt --all -- --check
cargo clippy -p quonfmt --all-targets -- -D warnings
cargo test -p quonfmt
cargo test -p frontend   # ensure pretty/fuzz paths untouched
npx @taskless/cli@latest check   # on changed files
```

Manual smoke:

```bash
cargo build -p quonfmt
quonfmt frontend/tests/fixtures/bell_state.qn
quonfmt --check frontend/tests/fixtures/bell_state.qn
quonfmt -w /tmp/messy.qn && quonfmt --check /tmp/messy.qn
```

---

## 12. Risks and open decisions

| Decision | Recommendation | Rationale |
| -------- | -------------- | --------- |
| `\|>` break style | **Leading `\|>` on continuation lines** (locked) | Matches parser newline-bridging idiom; no trailing-op variant |
| Bind alignment | Align `<-` when ≥2 binds | Matches reference fixtures; deterministic max-width |
| Comment stripping | Accept for v1 | No trivia in AST; document clearly |
| Shared float renderer | Extract to `frontend/src/render/lit.rs` | Avoid drift between debug and canonical |
| `--write` default | Require explicit `-w` for mutation | Safer default; rustfmt-compatible |
| Insta snapshot churn | Separate snapshot dir under `quonfmt/` | Decouple from debug `pretty` snapshots |

**Risk**: precedence table drift if parser changes — mitigate with `expr_precedence.qn`
golden + AST-stability asserts tied to parser tests.

**Risk**: width counting with Unicode identifiers — use char/byte counts consistently
(document in style spec; ASCII identifiers today per SPEC §2).

---

## 13. Acceptance criteria mapping

| Criterion | Implementation | Test |
| --------- | -------------- | ---- |
| `quonfmt <file>` rewrites deterministically | `format_str` + CLI stdout/`-w` | golden + manual |
| `quonfmt --check` non-zero on unformatted | `check_str` compares normalized bytes (LF, no trailing WS) | `cli.rs` + unit + CRLF test |
| Idempotency | Doc printer + fixed rules | `idempotency.rs` + proptest |
| Golden tests | `quonfmt/tests/corpus/` + fixture snapshots | `golden.rs` |
| README + contributor docs | §9 files | review |

---

## 14. Files to create/modify (summary)

**Create**:

- `quonfmt/Cargo.toml`, `quonfmt/src/{lib,main,doc,config,error}.rs`
- `quonfmt/src/print/{mod,decl,expr,ty,pat,stmt,nat}.rs`
- `quonfmt/tests/{golden,idempotency,cli}.rs`, `quonfmt/tests/corpus/input/*.qn` (incl. `application.qn`)
- `docs/quonfmt-style.md`
- `docs/plans/issue-46-plan.md` (this file)
- `docs/plans/issue-46-plan-review.md`

**Modify**:

- `Cargo.toml` (workspace members)
- `frontend/Cargo.toml` + `frontend/src/lib.rs` (`parser-only` feature gate)
- `README.md`, `docs/agents/code-quality.md`
- Optionally `frontend/src/render/lit.rs` + `pretty.rs` import refactor
- One-line pointer comment in `frontend/src/pretty.rs`

**Do not modify**:

- `frontend/fuzz/*` (continues using debug `pretty`)
- `frontend/tests/reference_algorithms.rs` snapshots (debug printer baseline)
