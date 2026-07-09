# Issue #132 — Zed extension: Quon language support (Tree-sitter, LSP, formatter)

**Role of this document:** implementation plan for an agent (or human). This issue is **planning-complete** when this file lands; **do not implement** the extension in the planning PR unless explicitly asked.

**Branch / worktree:** `issue-132` at `.worktrees/issue-132` (created from `origin/main`).  
**Parent:** #1  
**Related:** #131 (VS Code — **owns shared Tree-sitter grammar**), #133 (Neovim — consumes same grammar)  
**Blocked by (closed):** #43–#46, #49 (`quon_lsp`, `quonfmt`, smoke tests on `main`)

**Plan revision:** amended after plan review `779f2971` (CHANGES REQUIRED) — see §15 for blocker disposition.

---

## 0. Planner identity

This plan was produced by the **PLANNING AGENT** (not implementer, not reviewer). Scope is a full implementation blueprint only.

---

## 1. Goal

Ship an in-repo **Zed language extension** under `extensions/zed-quon/` that:

1. Registers `.qn` as language **Quon** with Tree-sitter syntax highlighting (and optional brackets/indents).
2. Attaches **`quon_lsp`** over stdio via a WASM `language_server_command` implementation.
3. Ships a **committed** format-on-save example for external **`quonfmt`**, plus a manual verification step.
4. Provides a README for **Install Dev Extension**, binary discovery, and log troubleshooting.

**Non-goals (v1):**

- Zed extension registry publish (optional follow-up; note license + ID rules).
- Embedding circuit / topology / mapping UIs (#134–#136).
- Changing `quon_lsp` or `quonfmt` protocol/CLI surface.
- Inventing a **competing** Tree-sitter grammar (see §3 — soft-create-if-missing is **forbidden**).

---

## 2. Verified current state (as of plan authoring)

| Artifact | Status on `main` |
| -------- | ---------------- |
| `quon_lsp` | Present; stdio LSP; diagnostics, hover, definition, completion (`@`, `:`, `<`), semantic tokens, code actions |
| `quonfmt` | Present; stdin when no file args; `-w` / `--check`; 4-space indent; comments stripped |
| Tree-sitter / `extensions/` | **Absent** — no `tree-sitter-quon`, no editor extension packages |
| #131 worktree | `.worktrees/issue-131` exists at `origin/main` but **no** `docs/plans/issue-131-plan.md` and **no** grammar yet |
| #133 worktree | Exists; no grammar plan yet |
| Fixture for ACs | `frontend/tests/fixtures/bell_state.qn` |

**LSP launch contract** (`quon_lsp/src/lib.rs`):

- Binary: `cargo build -p quon_lsp` → `target/debug/quon_lsp` or `target/release/quon_lsp`
- Env: `QUON_LOG` / `RUST_LOG=quon_lsp=debug`; optional `QUON_LSP_DEBOUNCE_MS`
- Transport: stdio only

**Formatter stdin contract** (`quonfmt/src/main.rs`):

- No file args → read stdin, write formatted source to stdout
- No `--stdin-filepath` flag (unlike Prettier) — Zed external formatter should pass **empty args** (or omit path flags) and feed buffer on stdin

**Zed constraints (from [language extensions](https://zed.dev/docs/extensions/languages) + [developing extensions](https://zed.dev/docs/extensions/developing-extensions)):**

- Language lives under `languages/<lang>/config.toml` with `path_suffixes`, `grammar`, etc.
- Grammars are registered in `extension.toml` as `[grammars.<name>]` with `repository` + `rev`/`commit`, and optional **`path`** for monorepo subdirectory grammars ([zed#9901](https://github.com/zed-industries/zed/discussions/9901) / [PR #9965](https://github.com/zed-industries/zed/pull/9965)).
- LSP requires Rust → WASM (`zed_extension_api`) implementing `language_server_command`.
- Extensions **must not** ship the language-server binary; discover via PATH / settings / worktree (see §4.4).
- Registry IDs must **not** contain the substrings `zed`, `Zed`, or `extension` → use **`id = "quon"`** even if the in-repo directory is `extensions/zed-quon/`.
- Semantic tokens from LSP are **off by default** in Zed; document `"semantic_tokens": "combined"` for Quon.

---

## 3. Shared Tree-sitter grammar ownership (#131 / #132 / #133)

### 3.1 Decision (binding for this plan)

| Concern | Owner | Location |
| ------- | ----- | -------- |
| Canonical grammar (`grammar.js`, generated `src/`, corpus tests) | **#131** (VS Code brief: “introduce the canonical grammar”) | Repo root: **`tree-sitter-quon/`** |
| Shared highlight / indent queries (Neovim-compatible) | Same package | `tree-sitter-quon/queries/{highlights,indents,folds}.scm` |
| Zed-specific query copies / tweaks | **#132** | `extensions/zed-quon/languages/quon/*.scm` (synced from shared queries; do not fork node names) |
| Neovim install docs / nvim-treesitter registration | **#133** | Consumes `tree-sitter-quon` |

**Hard rule:** Do **not** put a one-off `grammar.js` only under `extensions/zed-quon/` that diverges from `tree-sitter-quon/`. Zed’s grammar slot must point at the shared package.

### 3.2 Binding rule — no competing grammar; stack on #131 (BLOCKER FIX)

**Soft-create-if-missing is forbidden.** The #132 implementer must **not** invent, scaffold, or land a competing `tree-sitter-quon/` “just so Zed can proceed.”

Required workflow:

1. **Check** whether `tree-sitter-quon/` already exists on `main`, on a merged #131 PR, or on an open Graphite branch for #131 (or a dedicated grammar precursor branch owned by #131).
2. **If present:** consume it only — pin via §3.4; do not rewrite productions without coordinating with #131.
3. **If absent:** **stop and stack/rebase** this branch onto the #131 grammar branch (or a dedicated grammar precursor that #131 owns). Do **not** soft-create the grammar inside the #132 PR.
4. If no #131 / precursor branch exists yet, leave #132 blocked (or land **plan-only** commits) until the grammar precursor is available to stack on. Coordinate via Graphite parent = grammar branch, not bare `main`.

**Allowed on #132:** copy/sync query `.scm` files into `extensions/zed-quon/languages/quon/` from the shared package; Zed-only capture renames where Zed’s capture set differs — **never** a second `grammar.js`.

### 3.3 Grammar scope (v1 — surface highlighting, not a second frontend)

Owned and defined by **#131**. For implementer awareness only (do not re-specify a divergent grammar here):

**Keywords:** `fn`, `type`, `let`, `in`, `return`, `match`, `circuit`, `run`, `borrow`, `for`, `if`, `then`, `else`, `true`, `false`, `adjoint`, `controlled`, `par`

**Operators / punctuation:** `|>`, `<-`, `@`, `->`, `-o`, `=>`, arithmetic, `:`, `,`, `.`, `_`, `` ` ``, `|`, brackets `(){}[]<>`

**Literals:** ints, floats  
**Comments:** `--` line, `{- -}` block (nested if lexer allows; match frontend)  
**Structure (recommended nodes):** `source_file`, `fn_decl`, `type_decl`, `circuit_block`, `run_block`, `borrow_block`, `identifier`, `type_identifier` (heuristic: leading uppercase), comments, numbers, operators

**Out of scope for grammar v1:** full type-system fidelity, error-recovery parity with chumsky, significant-newline semantics beyond “extra” whitespace. Highlighting may be imperfect on exotic forms; LSP semantic tokens cover intelligence.

### 3.4 How Zed loads the monorepo grammar (portable pin — BLOCKER FIX)

**Do not** use absolute `file://…` paths or `rev = "local"`. Those are non-portable and fail for other machines / CI / Install Dev Extension clones.

Use Zed’s supported monorepo subdirectory recipe (`path` under `[grammars.<name>]`):

```toml
[grammars.quon]
repository = "https://github.com/arniber21/quon"
rev = "<COMMIT_SHA_THAT_CONTAINS_tree-sitter-quon>"
path = "tree-sitter-quon"
```

| Field | Rule |
| ----- | ---- |
| `repository` | Quon monorepo (or the same remote the checkout uses). |
| `rev` / `commit` | A real git commit SHA that **contains** `tree-sitter-quon/` (typically the stacked #131 grammar commit, or later `main`). Update the pin when the grammar changes. |
| `path` | **`"tree-sitter-quon"`** — subdirectory of that repo root where `grammar.js` / `src/` live. |

**Dev workflow notes:**

- Install Dev Extension still clones/fetches per `extension.toml`; the `path` key is what makes a monorepo grammar work ([zed discussion #9901](https://github.com/zed-industries/zed/discussions/9901)).
- After #131 lands grammar commits, bump `rev` in the same PR stack (or a follow-up commit) so the pin matches the grammar the queries expect.
- **Forbidden:** `repository = "file:///ABS/..."`, `rev = "local"`, or pointing `repository` at the monorepo root **without** `path = "tree-sitter-quon"`.

Registry publish (follow-up) can keep the same `repository` + `path` + pinned `rev` shape; no separate grammar mirror is required for v1 correctness.

---

## 4. Extension package layout

```
extensions/zed-quon/
  extension.toml          # id = "quon", language_servers (+ language_ids), grammars
  Cargo.toml              # cdylib + zed_extension_api
  Cargo.lock
  src/
    lib.rs                # Extension + language_server_command
  languages/
    quon/
      config.toml
      highlights.scm
      brackets.scm        # recommended
      indents.scm         # recommended
      outline.scm         # optional (fn names)
      # semantic_token_rules.json — only if custom token types appear (not needed for v1; quon_lsp uses standard types)
  settings.example.json   # REQUIRED — committed format-on-save + LSP/semantic-token example
  README.md
  LICENSE                 # MIT or Apache-2.0 (required before registry publish)
```

**Also required (project example):** a committed **`.zed/settings.json`** at the **quon repo root** (or documented equivalent project settings file) that enables Quon format-on-save for contributors who open the monorepo in Zed — see §5. This is part of the deliverable, not README-only.

**Not** a Cargo workspace member of the root `Cargo.toml` (Zed builds the extension crate separately when installing a dev extension). Keep it isolated like other editor packages.

### 4.1 `extension.toml` (target shape — BLOCKER FIX: `language_ids`)

```toml
id = "quon"
name = "Quon"
description = "Quon language support — Tree-sitter, quon_lsp, and quonfmt."
version = "0.0.1"
schema_version = 1
authors = ["Arnab Ghosh <...>"]
repository = "https://github.com/arniber21/quon"

[language_servers.quon-lsp]
name = "Quon LSP"
languages = ["Quon"]

[language_servers.quon-lsp.language_ids]
"Quon" = "quon"

[grammars.quon]
repository = "https://github.com/arniber21/quon"
rev = "<COMMIT_SHA_THAT_CONTAINS_tree-sitter-quon>"
path = "tree-sitter-quon"
```

**Required mappings (do not omit):**

| Key | Value | Why |
| --- | ----- | --- |
| `languages = ["Quon"]` | Zed language name from `languages/quon/config.toml` `name` | Registers which language(s) this server attaches to |
| `[language_servers.quon-lsp.language_ids] "Quon" = "quon"` | LSP `languageId` string sent to `quon_lsp` | Bridges Zed’s display name `Quon` to the server’s expected id `quon` |

Language server key **`quon-lsp`** matches the issue’s `.zed/settings.json` example (`lsp.quon-lsp.binary`).

### 4.2 `languages/quon/config.toml` (target shape)

```toml
name = "Quon"
grammar = "quon"
path_suffixes = ["qn"]
line_comments = ["-- "]
tab_size = 4
hard_tabs = false
brackets = [
  { start = "{", end = "}", close = true, newline = true },
  { start = "[", end = "]", close = true, newline = true },
  { start = "(", end = ")", close = true, newline = true },
  { start = "<", end = ">", close = true, newline = false },
]
```

Align `tab_size = 4` with `docs/quonfmt-style.md`.

### 4.3 Queries

- **`highlights.scm`:** map grammar nodes to Zed captures (`@keyword`, `@function`, `@type`, `@number`, `@operator`, `@comment`, `@punctuation.bracket`, …). Prefer staying compatible with Neovim capture names where possible; adapt only where Zed’s capture set differs.
- **`brackets.scm`:** `{}`, `()`, `[]`, `<>` (and comment delimiters if useful).
- **`indents.scm`:** indent inside `circuit` / `run` / `borrow` / `match` / block nodes; align with 4-space style.
- Keep a one-line header comment: `;; synced from tree-sitter-quon/queries/… — update both`.

### 4.4 WASM extension (`src/lib.rs`) — mandatory LSP discovery (BLOCKER FIX)

Implement `language_server_command` with this **mandatory, ordered** discovery. Do **not** skip steps or reorder:

1. **Settings** — `LspSettings::for_worktree("quon-lsp", worktree)` → if `binary.path` is set, use it (plus optional `arguments`; merge `env` with `worktree.shell_env()`).
2. **PATH** — else `worktree.which("quon_lsp")`.
3. **Worktree targets** — else, **only if** the worktree is a **quon checkout** (definition below), try in order:
   - `{worktree.root_path()}/target/release/quon_lsp`
   - `{worktree.root_path()}/target/debug/quon_lsp`
   (Use the platform-appropriate executable name; on Windows append `.exe` if the API does not.)
4. **Clear error** — else return `Err(...)` with an actionable message: build `cargo build -p quon_lsp --release`, put it on `PATH`, or set `lsp.quon-lsp.binary.path` in settings (include the JSON snippet).

#### Quon-checkout detection (normative)

A worktree **is** a quon checkout iff **all** of the following hold relative to `worktree.root_path()`:

1. `Cargo.toml` exists at the root, **and**
2. That manifest is a virtual workspace whose `[workspace].members` (or equivalent member list) includes the crates **`quon_lsp`** and **`frontend`** (string match on member entries such as `"quon_lsp"` / `"frontend"`), **and**
3. At least one of these marker paths exists: `frontend/src/lib.rs`, `SPEC.md`, or `tree-sitter-quon/` (grammar may be present once #131 has landed on the stacked parent).

**Must not** treat arbitrary projects as quon checkouts based only on “some `Cargo.toml`” or “a `target/` directory.” If detection fails, skip step 3 and go straight to the clear error (step 4).

Pass through env so `QUON_LSP_DEBOUNCE_MS` / `RUST_LOG` from settings work.

**Do not** download GitHub release binaries in v1 unless Releases already publish `quon_lsp` artifacts (they do not as of this plan).

Example settings override (also appears in committed examples — §5):

```json
{
  "lsp": {
    "quon-lsp": {
      "binary": {
        "path": "/ABS/PATH/TO/quon/target/release/quon_lsp",
        "env": { "RUST_LOG": "quon_lsp=debug" }
      }
    }
  }
}
```

Reference implementations:

- [zed-extensions/nix](https://github.com/zed-extensions/nix) — `nil` PATH + settings
- [zed-industries/zed `extensions/proto`](https://github.com/zed-industries/zed/tree/main/extensions/proto) — multi-server + `LspSettings`
- Issue mention: [zed-customlsp](https://github.com/zhcn000000/zed-customlsp) — arbitrary LSP wiring pattern

---

## 5. Formatter hook (`quonfmt`) — committed example + verification (BLOCKER FIX)

Zed does **not** register external formatters inside `extension.toml`. Format-on-save must be delivered as **committed project artifacts**, not README-only prose.

### 5.1 Required committed files

**A. `extensions/zed-quon/settings.example.json`** (always ship with the extension package):

```json
{
  "languages": {
    "Quon": {
      "tab_size": 4,
      "hard_tabs": false,
      "format_on_save": "on",
      "formatter": {
        "external": {
          "command": "quonfmt",
          "arguments": []
        }
      },
      "semantic_tokens": "combined"
    }
  },
  "lsp": {
    "quon-lsp": {
      "binary": {
        "path": "",
        "env": { "RUST_LOG": "quon_lsp=debug" }
      }
    }
  }
}
```

(Empty `binary.path` in the example means “omit or fill in”; README must say: delete `path` to use discovery, or set an absolute path.)

**B. Repo-root `.zed/settings.json`** (committed project example for the quon monorepo):

Same Quon `languages.Quon` block as above (format_on_save + external `quonfmt` + `semantic_tokens`), so opening this repo in Zed formats `.qn` on save without copy-paste. LSP `binary.path` may be omitted here so mandatory discovery (§4.4) applies for contributors who built `target/release/quon_lsp`.

### 5.2 Contract notes

- Empty `arguments` → `quonfmt` reads stdin (buffer) and writes stdout.
- `quonfmt` must be on `PATH` (or use an absolute `command` in settings).
- Document comment-stripping behavior (`quonfmt/README.md`) so format-on-save is not a surprise.

### 5.3 Manual verification step (required AC)

Implementer **must** perform and check off:

1. Ensure `quonfmt` is on `PATH` (`cargo build -p quonfmt --release` + `target/release` on `PATH`).
2. Install Dev Extension → open `frontend/tests/fixtures/bell_state.qn`.
3. Confirm project `.zed/settings.json` (or merged user settings from `settings.example.json`) has Quon `format_on_save` + external `quonfmt`.
4. Introduce deliberate whitespace drift (e.g. extra spaces / broken indent) that `quonfmt` would fix.
5. Save (or Format Document) → buffer matches `cargo run -p quonfmt -- frontend/tests/fixtures/bell_state.qn` / `--check` clean.
6. Record in the PR body that this manual format-on-save verification was done.

README still documents the snippet, but **docs alone do not satisfy** this blocker.

---

## 6. README requirements (acceptance)

`extensions/zed-quon/README.md` must cover:

1. **Build tools:** `cargo build -p quon_lsp --release` and `cargo build -p quonfmt --release`; add `target/release` to `PATH`.
2. **Install Dev Extension:** Zed → Extensions → **Install Dev Extension** → select `extensions/zed-quon` (Rust via **rustup** required).
3. **Grammar pin:** explain `repository` + `rev` + `path = "tree-sitter-quon"` (no absolute `file://`); how to bump `rev` after grammar updates.
4. **LSP discovery order:** settings → PATH → quon-checkout `target/{release,debug}` → clear error; link to override via `.zed/settings.json`.
5. **Format on save:** point at committed `settings.example.json` and repo `.zed/settings.json`; do not treat README as the only delivery vehicle.
6. **Semantic tokens:** enable `"combined"` for Quon.
7. **Troubleshoot:** `zed: open log`, `zed --foreground`, `RUST_LOG=quon_lsp=debug`, confirm LSP attached, common failures (binary not found, stale grammar `rev`, published extension overriding dev).
8. **Fixture smoke:** open `frontend/tests/fixtures/bell_state.qn`.

Optional pointer from root `README.md` or `docs/agents/` — one short link only; do not expand into a full editor-setup doc unless #133 wants a shared `docs/agents/editor-setup.md` later.

---

## 7. Acceptance criteria (checklist)

From #132 + agent brief + review `779f2971`:

- [ ] Opening `.qn` in Zed attaches `quon-lsp` and shows diagnostics (e.g. introduce a type error in `bell_state.qn`).
- [ ] Tree-sitter syntax highlighting renders for keywords, blocks, operators, comments, numbers.
- [ ] With semantic tokens `combined` (or `full`), LSP semantic highlighting visible for Quon constructs.
- [ ] Hover / go-to-definition / completion work on `frontend/tests/fixtures/bell_state.qn`.
- [ ] **Committed** format-on-save example exists (`extensions/zed-quon/settings.example.json` **and** repo `.zed/settings.json`); manual verification (§5.3) passes against `quonfmt` / `--check`.
- [ ] README documents build `quon_lsp`, Install Dev Extension, formatter settings, log troubleshooting.
- [ ] Grammar is **consumed** from #131 / precursor via portable `repository` + `rev` + `path = "tree-sitter-quon"` — **no** divergent Zed-only grammar; **no** soft-create-if-missing.
- [ ] `extension.toml` includes `languages = ["Quon"]` and `[language_servers.quon-lsp.language_ids] "Quon" = "quon"`.
- [ ] LSP discovery is **settings → PATH → worktree targets (quon-checkout only) → clear error**.

---

## 8. Implementation phases

### Phase 0 — Worktree / branch hygiene + stack on grammar

```bash
cd /Users/arnabghosh/projects/quon
gt sync --no-interactive
cd .worktrees/issue-132
```

**Before any extension code:** confirm `tree-sitter-quon/` exists on the Graphite parent. If not, restack onto the #131 grammar branch (or dedicated grammar precursor). **Do not** scaffold grammar on this branch (§3.2).

### Phase 1 — Consume shared grammar (no create)

1. Verify `tree-sitter-quon/` on the stacked parent.
2. Copy/sync queries into `extensions/zed-quon/languages/quon/`.
3. Pin `[grammars.quon]` with `repository` + `rev` + `path = "tree-sitter-quon"` (§3.4).

### Phase 2 — Extension skeleton

1. Create `extensions/zed-quon/` with `extension.toml` (**including** `languages` + `language_ids`), `Cargo.toml` (`crate-type = ["cdylib"]`, `zed_extension_api` latest compatible), `src/lib.rs` stub.
2. Add `languages/quon/config.toml` + query files.
3. Add `settings.example.json`.

### Phase 3 — LSP wiring

1. Implement `language_server_command` with **mandatory** order: settings → PATH → quon-checkout worktree targets → clear error (§4.4).
2. Implement quon-checkout detection exactly as specified.
3. Forward `shell_env` + settings `env`.
4. Manually verify initialize + diagnostics on bell fixture.

### Phase 4 — Formatter artifacts + docs

1. Commit `extensions/zed-quon/settings.example.json` and repo-root `.zed/settings.json` (§5).
2. Run manual format-on-save verification (§5.3); note in PR body.
3. Write README; note registry publish + license as follow-up.

### Phase 5 — Validation + PR

1. Manual Zed checklist (below).
2. `cargo check` inside `extensions/zed-quon` (host check; full WASM build happens on Install Dev Extension).
3. Do **not** run grammar authoring tasks on this branch unless only consuming; grammar tests belong to #131.
4. Repo checks for touched Rust/docs per `docs/agents/validation.md` / code-quality (extension crate may be outside workspace — do not force adding it to workspace members).
5. Graphite: commit on `issue-132` stacked on grammar parent, `gt submit --no-interactive --no-edit` (draft OK).

---

## 9. Validation plan

### Automated / CLI

| Check | Command / action |
| ----- | ---------------- |
| LSP still healthy | `cargo test -p quon_lsp` (no extension regressions expected) |
| Formatter | `cargo run -p quonfmt -- --check frontend/tests/fixtures/bell_state.qn` |
| Extension compiles | `cd extensions/zed-quon && cargo check` |
| Grammar | Owned by #131 — run `tree-sitter test` there / on stacked parent if needed; #132 does not author grammar |
| Fmt / clippy | Root workspace as usual for any Rust outside the extension; extension uses its own `Cargo.toml` |
| Settings artifacts present | `test -f extensions/zed-quon/settings.example.json && test -f .zed/settings.json` |

### Manual Zed (required for AC)

1. Install Dev Extension → `extensions/zed-quon`.
2. Open `frontend/tests/fixtures/bell_state.qn`.
3. Confirm language is Quon; Tree-sitter highlights visible (grammar pin via `path = "tree-sitter-quon"`).
4. Confirm `quon_lsp` starts (Zed language server UI / logs); diagnostics update on edit.
5. Hover `bell_state` / `CNOT`; go-to-def; trigger completion after `@` or `:`.
6. **Format-on-save verification (§5.3)** — dirty whitespace → save → matches `quonfmt` / `--check`.
7. Enable semantic tokens `combined`; confirm richer highlighting.
8. Break binary path / unset PATH with non-quon-looking root → readable error in logs; with quon checkout + built `target/release/quon_lsp` and no PATH entry → discovery step 3 succeeds.

**CI note:** Headless Zed extension tests are optional/out of scope for v1 (unlike VS Code `vscode-test` on #131). Prefer documenting manual AC.

### Flux / Taskless

- **Flux:** N/A (no refinement kernels).
- **Taskless:** N/A for extension/grammar JS unless new Rust under workspace `src/` is added; if shared Rust helpers appear, run Taskless on those paths.

---

## 10. Risks and mitigations

| Risk | Mitigation |
| ---- | ---------- |
| Divergent grammars across #131/#132/#133 | Single `tree-sitter-quon/`; #132 **consumes only**; soft-create forbidden |
| #131 grammar not ready | Stack/rebase on #131 / precursor; block implementation rather than invent grammar |
| Absolute `file://` grammar pins break portability | Portable `repository` + `rev` + `path = "tree-sitter-quon"` only |
| Missing `language_ids` → LSP not attached / wrong languageId | Required `languages` + `"Quon" = "quon"` in `extension.toml` |
| Format-on-save only in docs → AC miss | Committed `settings.example.json` + `.zed/settings.json` + §5.3 verification |
| Worktree discovery false-positives | Normative quon-checkout detection (§4.4) |
| Extension ID `zed-quon` rejected by registry | Use `id = "quon"`; keep directory name `extensions/zed-quon/` for humans |
| `quon_lsp` / `quonfmt` not on PATH | Mandatory discovery + settings override + clear error |
| Semantic tokens appear “missing” | Document Zed default `off` → `combined` in committed settings |
| Format-on-save strips comments | Document `quonfmt` contract up front |
| Significant newlines / layout confuse Tree-sitter | #131 grammar treats newlines as whitespace; rely on LSP for structure |
| WASM / rustup install failures | Document rustup requirement from Zed developing-extensions docs |
| Stale grammar `rev` after #131 updates | Bump pin in same stack / follow-up commit; README documents bump |

---

## 11. Expected diff shape

**In scope**

- `docs/plans/issue-132-plan.md` (this file)
- `extensions/zed-quon/**` (extension package + README + LICENSE + **`settings.example.json`**)
- **Committed** repo-root **`.zed/settings.json`** (Quon format-on-save + related language settings)
- **Consume** `tree-sitter-quon/**` from stacked #131 / precursor only — **do not** add a competing grammar tree in the #132 commit set
- Optional one-line link from root README

**Out of scope**

- Authoring / soft-creating `tree-sitter-quon/` on the #132 branch
- Changes to `quon_lsp`, `quonfmt`, `frontend` analyzer
- VS Code / Neovim packages (#131 / #133)
- `zed-industries/extensions` registry PR
- Visualization extensions

---

## 12. Coordination messages for sibling issues

**To #131 implementer:** You own canonical `tree-sitter-quon/`. #132 will **stack on your branch** and pin `path = "tree-sitter-quon"`. Do not embed a second incompatible Tree-sitter grammar inside the VS Code extension. If #132 appears blocked, land the grammar precursor first.

**To #133 implementer:** Point nvim-treesitter / docs at the same `tree-sitter-quon` package and its `queries/`. Prefer shared queries; only add Neovim-only captures in a documented overlay.

---

## 13. Open questions (resolved defaults)

| Question | Default for implementer |
| -------- | ----------------------- |
| Extension directory name vs id | Dir `extensions/zed-quon/`; id `quon` |
| Download LSP from GitHub Releases? | **No** in v1 |
| Grammar pin style | **`repository` + `rev` + `path = "tree-sitter-quon"`** — never absolute `file://` / `rev=local` |
| Soft-create grammar if #131 missing? | **Forbidden** — stack/rebase on #131 / precursor |
| Auto-discover `target/release/quon_lsp` in repo worktrees? | **Yes**, step 3 after settings + PATH, **only** if quon-checkout detection passes |
| `language_ids` required? | **Yes** — `languages = ["Quon"]` and `"Quon" = "quon"` |
| Format-on-save delivery | **Committed** `settings.example.json` + `.zed/settings.json` + manual verification |
| Commit generated parser.c? | **Yes** (standard Tree-sitter practice) — owned by #131 |
| Registry publish in this issue? | **No** — follow-up |

---

## 14. Implementation sequence (summary)

1. Stack/rebase onto #131 grammar (or precursor); **confirm** `tree-sitter-quon/` — do not create it.
2. Scaffold `extensions/zed-quon` (`extension.toml` with `language_ids`, WASM LSP discovery per §4.4, language config, queries, portable grammar pin).
3. Commit `settings.example.json` + repo `.zed/settings.json`; verify format-on-save (§5.3).
4. Manually validate remaining ACs on `bell_state.qn`.
5. Submit Graphite PR from `issue-132` stacked on the grammar parent.

---

## 15. Review `779f2971` blocker disposition

| # | Blocker | Plan resolution |
| - | ------- | --------------- |
| 1 | Grammar pin must be portable (`path = "tree-sitter-quon"` against quon repo); no absolute `file://` + `rev=local` | §3.4, §4.1, §6, §13 |
| 2 | Required `languages = ["Quon"]` and `[language_servers.quon-lsp.language_ids] "Quon" = "quon"` | §4.1, §7 |
| 3 | Do not invent competing grammar; stack/rebase on #131; soft-create-if-missing forbidden | §3.2, §8 Phase 0–1, §11, §14 |
| 4 | Commit format-on-save example + manual verification step | §5, §7, §9 |
| 5 | Mandatory LSP discovery settings → PATH → worktree targets → clear error; define quon-checkout detection | §4.4 |

**Status after this amendment:** ready for plan re-review (implementation still out of scope for the planning pass).
