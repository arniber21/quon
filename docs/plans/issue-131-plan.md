# Issue #131 — VS Code extension: Quon language support

**Planner:** planning agent (not implementer, not reviewer).  
**Branch / worktree:** `issue-131` at `.worktrees/issue-131` (created from `origin/main`, **independent — no Graphite stack**).  
**Issue:** https://github.com/arniber21/quon/issues/131  
**Sibling editors:** #132 (Zed), #133 (Neovim) — share Tree-sitter grammar.  
**Viz follow-ups (explicitly out of scope):** #134–#136.  
**Plan status:** amended for reviewer `4bd07d03` blockers 1–7 — ready for re-review.

Read first: this plan, `quon_lsp/src/lib.rs`, `quon_lsp/src/server.rs`, `quonfmt/README.md`, `docs/quonfmt-style.md`, `website/src/content/docs/guides/tooling.md`, `docs/agents/code-quality.md`, `docs/agents/validation.md`, `frontend/src/lexer.rs` (keyword / comment surface).

---

## 0. Role and constraints

This document is an **implementation plan only**. Do not treat it as permission to ship unrelated refactors. The implementer should:

- Land work on branch `issue-131` from this worktree (or recreate equivalently from `origin/main`).
- Keep the PR **independent of #132/#133 stacks** — no Graphite parent other than `main`.
- Prefer landing the **shared Tree-sitter grammar in this PR** (or a tiny precursor PR on the same branch lineage) so Zed/Neovim are not blocked.
- **Never** add circuit/topology/mapper webviews, custom editors, or Webview panels.

---

## 1. Goal

Ship a first-party VS Code extension that makes `.qn` files editable with:

1. Lexical syntax highlighting (TextMate for VS Code today + a **canonical Tree-sitter grammar** shared with #132/#133).
2. Full LSP client wiring to the existing `quon_lsp` stdio server (diagnostics, hover, definition, completion, code actions, semantic tokens).
3. Format Document via external `quonfmt` (server has **no** `documentFormattingProvider`); format-on-save available but **default OFF**.
4. Lint via LSP-merged `quonlint` diagnostics (no separate Problems-panel scraper required for MVP).
5. A buildable `.vsix` (Apache-2.0 LICENSE), README/dev docs, settings schema, and a CI smoke path (`xvfb-run` on Linux).

---

## 2. Acceptance criteria mapping

| Acceptance criterion (issue / agent brief) | How this plan satisfies it | Primary artifacts |
| ------------------------------------------ | -------------------------- | ----------------- |
| `.qn` files get syntax highlighting and semantic tokens | TextMate grammar + language config; LSP semantic tokens enabled in client | `extensions/vscode-quon/syntaxes/`, `language-configuration.json`, LSP client options |
| `quon_lsp` starts automatically; type errors appear while editing | Extension activates on `.qn` / `onLanguage:quon`; spawns stdio server; publishes diagnostics from server | `src/extension.ts`, `src/lsp.ts` |
| Hover, go-to-definition, completion on `frontend/tests/fixtures/bell_state.qn` | Client requests default LSP features; manual + CI smoke against fixture | README walkthrough + `vscode-test` suite |
| Format document runs `quonfmt` and matches CI style | Register `DocumentFormattingEditProvider` that shells out to `quonfmt` (stdin/stdout); **format-on-save default OFF** (comment-stripping hazard; align #133); Format Document still works | `src/format.ts`, `contributes.configurationDefaults` |
| Quick-fix code actions from lightbulb | Enable `codeAction` client capability; **CI asserts** `vscode.executeCodeActionProvider` returns a titled quick-fix on the borrow-discard fixture (see §12.2) | LSP client + smoke test |
| Buildable `.vsix` (marketplace optional) | `vsce package` / `npm run package` produces artifact with SPDX `Apache-2.0` LICENSE; not published in this issue | `package.json` `license`/`repository`, `LICENSE`, CI artifact |
| No embedded circuit/topology webviews | Explicit non-goal; no `webview` contribution points; README states viz is #134–#136 | `package.json` contributes audit |
| Tree-sitter grammar shareable with Zed/Neovim | Introduce `tree-sitter-quon/` as canonical grammar; VS Code ships TextMate *and* documents Tree-sitter consumption; #132/#133 consume the same tree | `tree-sitter-quon/` + cross-links in READMEs |

---

## 3. Current state (repo evidence)

### Already on `main` (blockers closed)

| Component | Status | Notes for the extension |
| --------- | ------ | ----------------------- |
| `quon_lsp` | Ready | stdio; incremental sync; hover / definition / completion (`@`, `:`, `<`); semantic tokens full; code actions; debounce via `QUON_LSP_DEBOUNCE_MS` (default 100 ms); `QUON_LOG` / `RUST_LOG` |
| Lint in LSP | Ready | After clean `frontend::analyze`, server runs `LintConfig::discover_for_file` + `lint_source` and merges diagnostics (`quon_lsp/src/analysis.rs`) |
| `quonfmt` | Ready | CLI: stdin→stdout, `-w`, `--check`; **no LSP formatting provider** |
| `quonlint` / `quonlint-cli` | Ready | Config upward discovery: `quonlint.toml` / `.quonlintrc.toml` |
| CI tooling job | Ready | `./scripts/tooling-check.sh --ci` + LSP smoke tests |
| Editor packages | **Missing** | No `extensions/`, no grammar, no `.vsix` |
| Docs | Partial | `website/.../guides/tooling.md` says “no first-party editor package yet” and links #131–#133 |

### Explicit server gaps (do **not** fix in #131)

- No `textDocument/formatting` / range formatting.
- No workspace/multi-file project model beyond single-buffer analysis + upward lint config discovery.
- No binary download / release packaging for `quon_lsp` itself.

If a gap blocks acceptance, file a follow-up issue; do not reimplement analysis in TypeScript.

---

## 4. Package layout

Propose this tree (new paths only; do not move Rust crates):

```text
tree-sitter-quon/                 # SHARED grammar (owned initially by #131)
  grammar.js
  src/                            # generated parser.c / scanner if needed
  queries/
    highlights.scm
    locals.scm                    # optional
    indents.scm                   # optional; useful for Neovim/Zed
  test/
    corpus/                       # CANONICAL tree-sitter CLI corpus (.txt) — NOT corpus/ at package root
  package.json                    # tree-sitter CLI metadata
  Cargo.toml                      # optional rust binding crate (defer if unused)
  README.md                       # consumption contract for #131/#132/#133
  binding.gyp / Makefile          # as generated by tree-sitter init

extensions/
  vscode-quon/
    package.json                  # contributes languages, grammars, commands, config;
                                  # license: "Apache-2.0"; repository.url → github.com/arniber21/quon
    package-lock.json             # or pnpm-lock — pick one; prefer npm for vsce simplicity
    tsconfig.json
    .vscodeignore
    README.md
    CHANGELOG.md                  # minimal
    LICENSE                       # Apache-2.0 full text (monorepo has no root LICENSE — ship one here)
    language-configuration.json
    syntaxes/
      quon.tmLanguage.json        # TextMate (VS Code primary lexical highlight)
    src/
      extension.ts                # activate / deactivate
      lsp.ts                      # LanguageClient + binary discovery
      format.ts                   # quonfmt DocumentFormattingEditProvider
      paths.ts                    # resolve quon_lsp / quonfmt (shared discovery; see §6.3 / §7.2)
      config.ts                   # settings helpers → env
    test/
      runTest.ts                  # @vscode/test-electron entry (launchArgs + workspace root — §12.2)
      suite/
        index.ts
        extension.test.ts         # diagnostics + code action + format assertions
      fixtures/                   # optional copies if workspace-relative open is awkward
        code_action_borrow_discard.qn
    scripts/
      package-vsix.sh             # thin wrapper around vsce
```

**Why both `tree-sitter-quon/` and TextMate?**

- VS Code stable highlighting for custom languages is still primarily **TextMate** (or the newer built-in Tree-sitter path which is not a drop-in for third-party extensions the way Zed/Neovim are).
- Zed (#132) and Neovim (#133) **require** Tree-sitter.
- One shared Tree-sitter package is the coordination artifact; TextMate is VS Code–local and may be a thinner keyword/operator mirror of the same lexical surface.

**Ownership:** #131 introduces and owns the initial `tree-sitter-quon/` commit. #132/#133 consume it by path (git submodule is **not** required inside a monorepo — use relative path / copy of `queries/` as documented).

---

## 5. Syntax strategy

### 5.1 Lexical surface (source of truth)

Mirror `frontend/src/lexer.rs` + SPEC keywords:

**Keywords:** `fn`, `type`, `let`, `in`, `return`, `match`, `circuit`, `run`, `borrow`, `for`, `if`, `then`, `else`, `true`, `false`, `adjoint`, `controlled`, `par`

**Comments:** line `-- …`, nested block `{- … -}`

**Operators (highlight as punctuation/operator):** `|>`, `<-`, `@`, `->`, `-o`, `=>`, and arithmetic / delimiters

**Literals:** ints, floats, identifiers; significant newlines are a parser concern — grammars need not emit a Newline token for highlighting.

### 5.2 TextMate (VS Code)

- File: `extensions/vscode-quon/syntaxes/quon.tmLanguage.json`
- Scope name: `source.quon`
- Register in `package.json`:

```json
"languages": [{
  "id": "quon",
  "aliases": ["Quon"],
  "extensions": [".qn"],
  "configuration": "./language-configuration.json"
}],
"grammars": [{
  "language": "quon",
  "scopeName": "source.quon",
  "path": "./syntaxes/quon.tmLanguage.json"
}]
```

- `language-configuration.json`: line comment `--`, block `{-` / `-}`, brackets `{}()[]`, auto-closing pairs, indentation rules aligned with `docs/quonfmt-style.md` (4 spaces).

### 5.3 Tree-sitter (shared)

**Package:** repo-root `tree-sitter-quon/`

**MVP grammar goals (highlighting-grade, not a second frontend parser):**

- Parse enough structure for `highlights.scm`: comments, keywords, identifiers, numbers, strings if any, operators, `circuit`/`run`/`borrow`/`par` blocks, `fn`/`type` decls.
- Prefer **error-tolerant** rules; do not attempt full type-expression fidelity in v1.
- Corpus tests under **`tree-sitter-quon/test/corpus/`** (tree-sitter CLI standard path — **not** `tree-sitter-quon/corpus/`) covering at least:
  - `frontend/tests/fixtures/bell_state.qn`
  - one `match` / `if` fixture from `quonfmt/tests/corpus/input/`
  - comments (line + nested block)

**Canonical corpus path (binding for #132/#133):** `tree-sitter-quon/test/corpus/`. Do not introduce a second corpus directory. Sibling plans that mention `corpus/` at package root must follow this path.

**Queries:**

| Query | Consumers |
| ----- | --------- |
| `queries/highlights.scm` | Zed, Neovim, (optional VS Code if/when TS highlighting is wired) |
| `queries/indents.scm` | Neovim / Zed (optional in #131; stub empty or minimal) |
| `queries/locals.scm` | Optional; defer if unused |

**Build:** Document `tree-sitter generate` + `tree-sitter test`. Commit generated `src/parser.c` (and scanner if any) so consumers do not need the CLI at runtime.

**VS Code usage of Tree-sitter in #131:**

- **Required:** land the shared package + README consumption contract.
- **Optional stretch:** do **not** block on embedding Tree-sitter WASM in the VS Code extension for v1. TextMate + LSP semantic tokens meet the highlighting acceptance criterion. Document clearly: “VS Code uses TextMate; Tree-sitter is canonical for Zed/Neovim.”

### 5.4 Sharing with #132 / #133 without blocking

**Recommended path (single PR for #131):**

1. Land `tree-sitter-quon/` + TextMate + VS Code extension together.
2. In the PR description / `tree-sitter-quon/README.md`, state the contract:

```text
Canonical grammar: /tree-sitter-quon
Corpus (tree-sitter test): /tree-sitter-quon/test/corpus/   # CLI standard — do not use /corpus at package root
Zed (#132): point grammar path at ../../tree-sitter-quon (or copy queries into extension as build step)
Neovim (#133): nvim-treesitter local parser or :TSInstall from path
Do not fork grammar.js
Do not relocate or duplicate the corpus directory
```

**Alternative if review wants a smaller first merge:**

1. Tiny precursor PR: **only** `tree-sitter-quon/` (+ corpus + README).
2. Follow-up PR on same independent branch lineage: `extensions/vscode-quon/`.

Either way, **#131 owns the initial grammar**. #132/#133 must not invent a second `grammar.js`.

If #132 starts first: same rule inverted — whoever lands first owns the path `tree-sitter-quon/`; the other issues rebase onto it. Prefer #131 as owner per agent brief.

---

## 6. LSP client wiring

### 6.1 Stack

- TypeScript extension
- `vscode-languageclient` (v9.x compatible with current VS Code engine)
- Transport: **stdio** only
- Server module: discovered `quon_lsp` binary (not an npm-bundled WASM rewrite)

### 6.2 Activation

```json
"activationEvents": [
  "onLanguage:quon"
]
```

(Modern VS Code also activates from `contributes.languages`; keep explicit `onLanguage:quon` for clarity.)

### 6.3 Binary discovery (dev → release)

Shared helper in `src/paths.ts`. Resolve **`quon_lsp`** and **`quonfmt`** with **symmetric** ordered lookup (stop at first executable hit).

#### `quon_lsp` discovery order

1. Setting `quon.lsp.path` (absolute or workspace-relative).
2. Environment `QUON_LSP_PATH` if set and non-empty.
3. `PATH` lookup (`which` / `where` → `quon_lsp`).
4. Workspace heuristics (walk up from workspace folder(s) for Cargo targets):
   - `${workspaceFolder}/target/release/quon_lsp`
   - `${workspaceFolder}/target/debug/quon_lsp`
5. On failure: show a **modal error** with copy-pasteable build command (below).

#### `quonfmt` discovery order (must mirror LSP)

1. Setting `quon.fmt.path` (absolute or workspace-relative).
2. Environment **`QUON_FMT_PATH`** if set and non-empty. (**Required** — CI and local smoke set this; implementers must read it in `paths.ts`, not invent a one-off only in test scripts.)
3. `PATH` lookup (`quonfmt`).
4. Workspace heuristics:
   - `${workspaceFolder}/target/release/quonfmt`
   - `${workspaceFolder}/target/debug/quonfmt`
5. On failure: show an error (format provider returns `[]`) with the same build command.

```sh
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
```

**Release / download strategy (document, do not fully automate in MVP):**

- Marketplace users will not have a Cargo tree. v1 README documents:
  - build from source and set `quon.lsp.path` / `quon.fmt.path`, **or**
  - install binaries onto `PATH` when GitHub Releases exist (follow-up), **or**
  - export `QUON_LSP_PATH` / `QUON_FMT_PATH` for CI / scripted installs.
- Do **not** block #131 on implementing GitHub Release downloaders / checksum verification. Stub settings + env + clear errors are enough.

### 6.4 Client options

| Concern | Behavior |
| ------- | -------- |
| Document selector | `{ scheme: 'file', language: 'quon' }` (also `untitled` if cheap) |
| Sync | Full client defaults; server expects incremental |
| Middleware | None required |
| Trace | Map `quon.lsp.trace` → language client trace |
| Env | Pass through: `QUON_LSP_DEBOUNCE_MS`, `QUON_LOG` / `RUST_LOG` from settings |
| Semantic tokens | Ensure client does not disable server semantic tokens |
| Code actions | Enable `quickfix` + `refactor.rewrite` |
| Formatting | **Disable** LSP formatting provider on client if any; use `quonfmt` provider instead |

### 6.5 Root / lint config

- Server discovers `quonlint.toml` by walking up from the file URI path — no special client root protocol required for MVP.
- Setting `quon.lint.configPath` (optional): if set, document that **today the server does not take a CLI flag for config**; either (a) leave as documentation-only / future server flag, or (b) set cwd / rely on placing config in the project. **Recommendation:** expose the setting in the schema as “reserved / documented discovery behavior” and do **not** invent a fake env var. Prefer documenting upward discovery; file a server follow-up if an explicit override is needed.

### 6.6 Lifecycle

- `activate`: start `LanguageClient`, register formatter.
- `deactivate`: `client.stop()`.
- Restart command: `quon.restartServer` (useful when binary path changes).

---

## 7. Format via `quonfmt`

### 7.1 Why external

`quon_lsp` does **not** advertise formatting. Using `quonfmt` keeps CI style (`docs/quonfmt-style.md`, `./scripts/tooling-check.sh --ci`) identical to the editor.

### 7.2 Provider design

Register `vscode.languages.registerDocumentFormattingEditProvider('quon', …)`:

1. Resolve `quonfmt` via **`paths.ts`** using the **§6.3 `quonfmt` discovery order** (`quon.fmt.path` → `QUON_FMT_PATH` → `PATH` → `target/release|debug/quonfmt`). Do not hard-code only settings/`PATH`.
2. Prefer **stdin → stdout**: write document text to stdin, read formatted stdout.
3. Exit codes: `0` success; `2` parse error → show error message, return `[]` edits; other non-zero → show stderr.
4. Replace full document range with formatted text (single `TextEdit`).
5. **Caution (must document in README):** v1 `quonfmt` **strips comments**. Surface this in the extension README and optionally a first-run information message (once).

### 7.3 Format on save — **default OFF (locked)**

**Product decision (binding; aligns with #133):** do **not** enable format-on-save by default. `quonfmt` strips comments (`docs/quonfmt-style.md`); silent save would destroy user comments.

Wire via `package.json` **`contributes.configurationDefaults`** (not a vague README sketch only):

```json
"configurationDefaults": {
  "[quon]": {
    "editor.defaultFormatter": "quon.quon-vscode",
    "editor.formatOnSave": false
  }
}
```

Also set setting `quon.fmt.formatOnSave` default to **`false`** (documentation / future toggle helper only — the authoritative VS Code behavior is `configurationDefaults` above).

- **Format Document** (explicit command / palette) remains fully supported and is what AC “format document runs `quonfmt`” means.
- README must show how to opt in: set `"[quon].editor.formatOnSave": true` after reading the comment-stripping warning.
- CI must **not** assert format-on-save; assert Format Document / provider edits only.
- Publisher/name for `defaultFormatter` must match locked identity in §11.4 (`quon` / `quon-vscode` → id `quon.quon-vscode`).

### 7.4 Range formatting

Out of scope for MVP (full-document only).

---

## 8. Lint via LSP

- Do **not** shell out to `quonlint` on every save for MVP.
- Rely on `quon_lsp` merged diagnostics (type + lint when analysis is clean).
- Optional later: a command `quon.lintWorkspace` that runs `quonlint --project` — **not** required for acceptance.
- Settings may still document `quonlint.toml` discovery for users configuring severity.

---

## 9. Extension settings schema

Contribute under `quon.*`:

| Setting | Type | Default | Maps to |
| ------- | ---- | ------- | ------- |
| `quon.lsp.path` | string | `""` | Server executable |
| `quon.lsp.debounceMs` | number | `100` | Env `QUON_LSP_DEBOUNCE_MS` |
| `quon.lsp.logLevel` | enum `off\|error\|warn\|info\|debug\|trace` | `info` | Env `QUON_LOG` / `RUST_LOG=quon_lsp=…` |
| `quon.lsp.trace` | enum `off\|messages\|verbose` | `off` | Language client trace |
| `quon.fmt.path` | string | `""` | `quonfmt` executable (also see env `QUON_FMT_PATH`) |
| `quon.fmt.formatOnSave` | boolean | **`false`** | Docs/helper only; real default is `contributes.configurationDefaults` `"[quon].editor.formatOnSave": false` |
| `quon.lint.configPath` | string | `""` | Documented discovery; no server wire in MVP unless follow-up lands |

Also contribute a command palette entry:

- `Quon: Restart Language Server`
- `Quon: Show Server Status` (optional: path resolved + version if `--version` exists; skip if binary has no version flag)

**No webview commands.** Optional future: `Quon: Open in Viewer` that runs an external CLI — stub only if trivial; otherwise omit until #134.

---

## 10. README, docs, and website touch-ups

### 10.1 `extensions/vscode-quon/README.md`

Must cover:

1. Prerequisites: build `quon_lsp` + `quonfmt` (or set `quon.lsp.path` / `quon.fmt.path` / `QUON_LSP_PATH` / `QUON_FMT_PATH`).
2. Install from `.vsix` (`code --install-extension quon-*.vsix`) and/or “Run Extension” F5 launch config.
3. Open `frontend/tests/fixtures/bell_state.qn` and verify diagnostics / hover / format.
4. Settings table (including **format-on-save default OFF** and how to opt in).
5. Comment-stripping warning for `quonfmt` (why format-on-save is off by default).
6. Pointer to shared grammar `tree-sitter-quon/` (corpus at `test/corpus/`).
7. Explicit: no embedded visualizations (#134–#136).
8. License: Apache-2.0 (extension `LICENSE` + `package.json` `license` field).

### 10.2 `tree-sitter-quon/README.md`

Consumption contract for #132/#133:

- Canonical path: repo-root `tree-sitter-quon/`
- **Corpus:** `tree-sitter-quon/test/corpus/` (tree-sitter CLI default; do not use `tree-sitter-quon/corpus/`)
- Commands: `npx tree-sitter generate` then `npx tree-sitter test`
- Hard rule: do not fork `grammar.js` / `src/parser.c`

### 10.3 Repo docs

- Update `website/src/content/docs/guides/tooling.md` “Editor integration status” to point at `extensions/vscode-quon/` once present (keep Zed/Neovim as “in progress” if not landed).
- Optional one-liner in root `README.md` under tooling table.

### 10.4 `.vscode/launch.json` (extension package)

Provide an Extension Development Host launch config for F5 debugging.

---

## 11. `.vsix` build

### 11.1 Tooling

- `npm` + `@vscode/vsce` as a devDependency (or `npx @vscode/vsce package`).
- Scripts in `extensions/vscode-quon/package.json`:

```json
"scripts": {
  "compile": "tsc -p ./",
  "watch": "tsc -watch -p ./",
  "lint": "tsc -p ./ --noEmit",
  "package": "npm run compile && vsce package --out dist/",
  "test": "node ./out/test/runTest.js"
}
```

### 11.2 `.vscodeignore`

Exclude `src/`, `tsconfig`, tests, maps as appropriate; include `out/`, `syntaxes/`, `language-configuration.json`, `README.md`, **`LICENSE`**.

### 11.3 License / SPDX (locked — monorepo has no root `LICENSE`)

The Quon monorepo currently ships **no** root `LICENSE` file. The extension must still be packable with `vsce`.

| Field / file | Locked value |
| ------------ | ------------ |
| SPDX / `package.json` `"license"` | **`Apache-2.0`** |
| `extensions/vscode-quon/LICENSE` | Full Apache-2.0 text (copy from https://www.apache.org/licenses/LICENSE-2.0.txt) |
| `package.json` `"repository"` | `{ "type": "git", "url": "https://github.com/arniber21/quon.git", "directory": "extensions/vscode-quon" }` |
| `package.json` `"publisher"` / `"name"` | `quon` / `quon-vscode` (extension id `quon.quon-vscode`) |

Do **not** leave LICENSE as “match repo” — there is nothing to match. Do **not** omit `license`/`repository` (vsce warns/fails and marketplace follow-ups need them). If the monorepo later adds a root LICENSE with a different SPDX, file a follow-up to realign; **v1 ships Apache-2.0**.

### 11.4 CI artifact (optional but nice)

Upload `.vsix` as a workflow artifact on PRs touching `extensions/vscode-quon/**`. Marketplace publish is **out of scope**.

### 11.5 Publisher / identity

Stable id: **`quon.quon-vscode`**. Document that marketplace branding may change later; do not bikeshed beyond a working package.

---

## 12. CI smoke test approach

### 12.1 Goals

Prove in CI that:

1. Extension TypeScript compiles.
2. `.vsix` packages successfully (`vsce package` with LICENSE + `license`/`repository` set).
3. Headless VS Code opens a `.qn` fixture and receives diagnostics from `quon_lsp`.
4. **Code actions** are returned for a named quick-fix fixture (lightbulb AC — automated; see §12.2 step 4).
5. Format Document via `quonfmt` produces expected edits (not format-on-save).

### 12.2 Recommended job: `vscode-extension` (new workflow or new job in `ci.yml`)

**Constraints:** full Quon `quon_lsp` build needs LLVM/MLIR/Z3 like the `tooling` job. Reuse that install pattern.

**Linux display (blocker #1 — required):** `@vscode/test-electron` needs a display on headless `ubuntu-latest`. Wrap the test step with:

```yaml
- name: Extension smoke
  run: xvfb-run -a npm test
  working-directory: extensions/vscode-quon
  env:
    QUON_LSP_PATH: ${{ github.workspace }}/target/release/quon_lsp
    QUON_FMT_PATH: ${{ github.workspace }}/target/release/quonfmt
    QUON_LSP_DEBOUNCE_MS: "0"
```

Install `xvfb` in the job (`sudo apt-get install -y xvfb`) if the runner image lacks it. macOS/Windows self-hosted paths (if any) do not need `xvfb-run`; Linux CI **must**.

**Steps:**

1. Checkout (default: full repo at `${{ github.workspace }}`).
2. Install LLVM 22 / MLIR / Z3 (copy from existing `tooling` job).
3. `cargo build --release -p quon_lsp -p quonfmt`.
4. Setup Node 22.
5. `npm ci` in `extensions/vscode-quon`.
6. `npm run compile`.
7. `npm run package` (produce `.vsix`; must succeed with Apache-2.0 LICENSE present).
8. `xvfb-run -a npm test` with env `QUON_LSP_PATH`, `QUON_FMT_PATH`, `QUON_LSP_DEBOUNCE_MS=0` as above.

### 12.2a `runTests` launch / workspace / fixture resolution (blocker #5 — locked)

`extensions/vscode-quon/src/test/runTest.ts` must call `@vscode/test-electron` `runTests` with an explicit **repo-root workspace**:

```ts
import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main() {
  // extensions/vscode-quon/ → repo root (../..)
  const extensionDevelopmentPath = path.resolve(__dirname, "../../");
  const extensionTestsPath = path.resolve(__dirname, "./suite/index");
  const repoRoot = path.resolve(extensionDevelopmentPath, "../..");

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    // Open the monorepo root so relative fixture paths resolve
    launchArgs: [repoRoot, "--disable-extensions"],
  });
}
```

**Fixture path resolution (binding):**

| Fixture | How tests open it |
| ------- | ----------------- |
| Bell state | `vscode.Uri.file(path.join(vscode.workspace.workspaceFolders![0].uri.fsPath, "frontend/tests/fixtures/bell_state.qn"))` — requires `launchArgs: [repoRoot, …]` so `workspaceFolders[0]` **is** the Quon checkout |
| Type-error buffer | Untitled or temp file under `os.tmpdir()` with language id `quon` (no repo path required) |
| Code-action fixture | Prefer in-memory / temp `.qn` with the **exact** borrow-discard source from `quon_lsp/tests/diagnostics.rs` (`code_action_returns_borrow_discard_fix`); optionally also ship `extensions/vscode-quon/test/fixtures/code_action_borrow_discard.qn` as a named file for manual lightbulb checks |
| Format buffer | Temp / untitled poorly spaced `.qn` |

If `workspaceFolders` is empty, fail the suite immediately with a message that `runTest.ts` must pass `repoRoot` in `launchArgs`.

Do **not** rely on `process.cwd()` alone for `bell_state.qn` — the Extension Host cwd is not guaranteed to be the monorepo root.

### 12.2b Test body (`extension.test.ts`) — required assertions

| # | Assertion | Maps to AC |
| - | --------- | ---------- |
| 1 | Open `frontend/tests/fixtures/bell_state.qn` → language id `quon`; wait until client ready; assert **no error-severity** diagnostics (or empty diagnostics) | highlighting activation + clean LSP |
| 2 | Open temp `.qn` with deliberate type error → `vscode.languages.getDiagnostics` non-empty | type errors while editing |
| 3 | On bell_state (or same doc): `vscode.executeHoverProvider` / `vscode.executeDefinitionProvider` / `vscode.executeCompletionItemProvider` each return a non-empty result for a known position (pick a stable identifier span in the fixture) | hover / definition / completion |
| 4 | **Code action / lightbulb AC (automated — locked choice):** open buffer with source `fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}`; wait for diagnostics; call `vscode.commands.executeCommand('vscode.executeCodeActionProvider', uri, diagnosticRange)`; assert at least one action whose `title` contains `discard(a)` (same contract as `quon_lsp/tests/diagnostics.rs`). This is the CI stand-in for “invokable from the lightbulb menu” — VS Code’s lightbulb UI is the same provider. | quick-fix lightbulb |
| 5 | Invoke Format Document / formatting provider on a poorly spaced buffer → result matches `quonfmt` stdin expectation (snapshot or golden one-liner). Do **not** toggle format-on-save. | format document |

Prefer a **short timeout** (≤ 60s for the suite) and fail clearly if `QUON_LSP_PATH` / `QUON_FMT_PATH` binaries are missing or not executable.

**AC ↔ CI mapping (locked):** lightbulb/quick-fix is **in CI** via `executeCodeActionProvider` (choice: automate, do **not** demote to manual-only). Manual checklist may still visually confirm the lightbulb glyph, but CI is the gate.

### 12.3 Grammar CI (cheap, always-on)

```sh
cd tree-sitter-quon && npm ci && npx tree-sitter test
# corpus lives at test/corpus/ (tree-sitter CLI standard)
```

Pin `tree-sitter-cli` as a devDependency of `tree-sitter-quon`.

### 12.4 What not to do in CI

- Do not launch a GUI manually.
- Do **not** omit `xvfb-run -a` on Linux.
- Do not download marketplace VS Code Insiders unless `@vscode/test-electron` requires it (it downloads a test electron build itself).
- Do not embed viz smoke tests.
- Do not assert format-on-save (default is OFF).

### 12.5 Local validation commands (implementer)

```sh
# Rust tools
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
./scripts/tooling-check.sh --ci

# Grammar (corpus: tree-sitter-quon/test/corpus/)
cd tree-sitter-quon && npm ci && npx tree-sitter generate && npx tree-sitter test

# Extension
cd extensions/vscode-quon
npm ci
npm run compile
npm run package
QUON_LSP_PATH=../../target/release/quon_lsp \
QUON_FMT_PATH=../../target/release/quonfmt \
QUON_LSP_DEBOUNCE_MS=0 \
npm test
# On Linux without a display:
# QUON_… xvfb-run -a npm test
```

---

## 13. Explicit non-goals

| Out of scope | Tracking |
| ------------ | -------- |
| Circuit diagram / stage webviews | #134 |
| Mapping visualization | #135 |
| Hardware topology / schedule panels | #136 |
| Marketplace listing, icons polish, walkthrough UX beyond README | follow-up |
| Reworking `quon_lsp` protocol (formatting provider, multi-file) | follow-up issues |
| Bundling MLIR-linked binaries inside the `.vsix` | follow-up / releases |
| Zed / Neovim packages | #132 / #133 |
| Second Tree-sitter grammar fork | forbidden |
| Range formatting, organize-imports, debug adapter | not requested |

**Hard rule:** `package.json` must not contribute `webview`, custom editors for viz, or notebook renderers for circuits.

---

## 14. Implementation phases (suggested commits)

Keep commits small and reviewable. Suggested sequence on `issue-131`:

1. **`tree-sitter-quon` scaffold** — `grammar.js`, generate parser, **`test/corpus/`** for bell_state + comments, `highlights.scm`, README consumption contract (path + corpus layout for #132/#133).
2. **VS Code package skeleton** — `extensions/vscode-quon/package.json` (Apache-2.0 `license` + `repository`), `LICENSE`, language id, TextMate grammar, language-configuration, `configurationDefaults` with **formatOnSave: false**, empty extension activate.
3. **LSP client** — `paths.ts` discovery (`QUON_LSP_PATH` / `QUON_FMT_PATH` symmetric), stdio `LanguageClient`, settings → env, restart command.
4. **Formatter** — `quonfmt` provider (reads `QUON_FMT_PATH`); format-on-save **off** by default; comment warning + opt-in docs in README.
5. **Tests + CI** — `@vscode/test-electron` with `launchArgs: [repoRoot]`; suite covers diagnostics + **code action** + format; Linux job uses **`xvfb-run -a`**; `.vsix` package script.
6. **Docs** — extension README, tooling guide update, root README one-liner.

Optional split: land (1) as a tiny precursor PR if reviewers want grammar isolated; otherwise one PR is fine.

**Graphite:** branch is independent from `main` — `gt submit --no-interactive --no-edit` (or `--draft`) without stacking on other issue branches.

---

## 15. Risks and mitigations

| Risk | Impact | Mitigation |
| ---- | ------ | ---------- |
| Tree-sitter grammar incomplete vs chumsky parser | Highlight gaps / ERROR nodes | MVP = highlighting-grade; corpus on real fixtures under `test/corpus/`; LSP semantic tokens cover semantic color |
| TextMate and Tree-sitter drift | Inconsistent colors across editors | Single keyword/operator list in `tree-sitter-quon/README.md`; TextMate generated or hand-synced from that list in the same PR |
| `quon_lsp` / `quonfmt` not on PATH for users | Extension appears broken | Ordered discovery including `QUON_LSP_PATH` / `QUON_FMT_PATH` + actionable error; README build instructions |
| `quonfmt` strips comments | Surprising data loss on format-on-save | **Default format-on-save OFF** via `configurationDefaults`; loud README caution; Format Document still available |
| CI flaky `@vscode/test-electron` / missing display | Red CI | **`xvfb-run -a` on Linux**; cache VS Code test binary if supported; retry once; keep suite tiny |
| Fixture path unresolved in Extension Host | Flaky / false-fail smoke | `runTests({ launchArgs: [repoRoot, "--disable-extensions"] })`; open fixtures via `workspaceFolders[0]` |
| LLVM-heavy job time | Slow PR feedback | Separate `vscode-extension` job with 15–20m timeout; only run on paths `extensions/**`, `tree-sitter-quon/**`, or workflow_dispatch / tooling changes |
| #132/#133 race on grammar path / corpus layout | Duplicate grammars or broken `tree-sitter test` | This plan claims `tree-sitter-quon/` + **`test/corpus/`**; comment on #132/#133 when PR opens |
| Accidental webview scope creep | Violates issue | Checklist in PR template / acceptance audit of `package.json` contributes |
| Missing LICENSE breaks `vsce package` | Cannot ship `.vsix` | Ship Apache-2.0 `LICENSE` + `package.json` `license`/`repository` (§11.3) |

**Format-on-save default (locked):** `false` (align #133). Not negotiable in implementation without a plan amendment.

---

## 16. Validation checklist (before PR ready)

### Functional (manual)

- [ ] Open `frontend/tests/fixtures/bell_state.qn` → TextMate colors visible; semantic tokens active.
- [ ] Introduce a type error → Problems panel updates after debounce.
- [ ] Hover on `bell_state` / `CNOT` / `measure` works.
- [ ] Go-to-definition on a local binding works.
- [ ] Completion triggers on `@` / `:` / `<`.
- [ ] Lightbulb appears on borrow-discard fixture (`extensions/vscode-quon/test/fixtures/code_action_borrow_discard.qn` or equivalent buffer); applying fix inserts `discard(a)` — **CI already gates** via `executeCodeActionProvider`; manual confirms glyph UX.
- [ ] Format Document rewrites to `quonfmt` style; `quonfmt --check` clean afterward.
- [ ] Confirm format-on-save is **off** by default; opt-in documented.
- [ ] No webview contributions in `package.json`.

### Automated

- [ ] `npx tree-sitter test` in `tree-sitter-quon/` (corpus under `test/corpus/`)
- [ ] `npm run compile && npm run package` in `extensions/vscode-quon/` (LICENSE + Apache-2.0 present)
- [ ] `xvfb-run -a npm test` (Linux CI) / `npm test` (local with display) with `QUON_LSP_PATH` + `QUON_FMT_PATH`
- [ ] Smoke asserts: diagnostics, hover/definition/completion, **code action title contains `discard(a)`**, format provider
- [ ] `./scripts/tooling-check.sh --ci` still green (no Rust regressions)
- [ ] `cargo fmt` / clippy / tests unchanged unless a tiny Rust fix was required (prefer zero Rust changes)

### Process

- [ ] PR links #131; mentions grammar ownership for #132/#133.
- [ ] Branch `issue-131` based on `main` only (no stack).
- [ ] Plan file committed or left in worktree per team norm (`docs/plans/issue-131-plan.md`).

---

## 17. Open decisions (resolved by this plan unless review overrides)

| Decision | Choice |
| -------- | ------ |
| Shared grammar path | `tree-sitter-quon/` at repo root |
| Grammar corpus path | **`tree-sitter-quon/test/corpus/`** (CLI standard; binding for #132/#133) |
| Grammar owner | #131 (initial land) |
| VS Code lexical highlight | TextMate in-extension |
| Tree-sitter in VS Code runtime | Document only for v1 (not required to embed WASM) |
| Format path | External `quonfmt`, not LSP |
| Format-on-save default | **`false`** via `contributes.configurationDefaults` (align #133) |
| Lint path | LSP-merged only |
| Binary discovery | Settings → **`QUON_LSP_PATH` / `QUON_FMT_PATH`** → PATH → `target/{release,debug}` (symmetric in `paths.ts`) |
| Binary bundling in `.vsix` | No — discover / PATH / setting / env |
| Lightbulb / code-action AC | **CI-automated** via `executeCodeActionProvider` on borrow-discard fixture |
| Extension Host workspace | `runTests` `launchArgs: [repoRoot, "--disable-extensions"]` |
| Linux CI display | **`xvfb-run -a npm test`** |
| Extension license | **Apache-2.0** (`LICENSE` + `package.json` `license` + `repository`) |
| Publisher / name | `quon` / `quon-vscode` (id `quon.quon-vscode`) |
| Webviews | Forbidden |
| Branching | Independent `issue-131` from `origin/main` |

---

## 18. Appendix — capability matrix (server vs extension)

| Feature | Provided by | Extension responsibility |
| ------- | ----------- | ------------------------ |
| Diagnostics (parse/type/lint) | `quon_lsp` | Start client; show Problems |
| Hover | `quon_lsp` | Default middleware |
| Definition | `quon_lsp` | Default |
| Completion | `quon_lsp` | Default; triggers `@ : <` |
| Semantic tokens | `quon_lsp` | Ensure enabled |
| Code actions | `quon_lsp` | Ensure quickfix enabled |
| Formatting | `quonfmt` CLI | Custom provider |
| Syntax (VS Code) | TextMate | Ship grammar |
| Syntax (Zed/Nvim) | `tree-sitter-quon` | Land package; siblings consume |
| Circuit viz | — | **Do not implement** |

---

## 19. Appendix — example `package.json` contribution sketch

Implementer should expand versions; this is structural guidance only:

```json
{
  "name": "quon-vscode",
  "displayName": "Quon",
  "description": "Quon language support — syntax, LSP, formatter",
  "version": "0.1.0",
  "publisher": "quon",
  "license": "Apache-2.0",
  "repository": {
    "type": "git",
    "url": "https://github.com/arniber21/quon.git",
    "directory": "extensions/vscode-quon"
  },
  "engines": { "vscode": "^1.85.0" },
  "categories": ["Programming Languages", "Formatters", "Linters"],
  "activationEvents": ["onLanguage:quon"],
  "main": "./out/extension.js",
  "contributes": {
    "languages": [
      {
        "id": "quon",
        "aliases": ["Quon"],
        "extensions": [".qn"],
        "configuration": "./language-configuration.json"
      }
    ],
    "grammars": [
      {
        "language": "quon",
        "scopeName": "source.quon",
        "path": "./syntaxes/quon.tmLanguage.json"
      }
    ],
    "configurationDefaults": {
      "[quon]": {
        "editor.defaultFormatter": "quon.quon-vscode",
        "editor.formatOnSave": false
      }
    },
    "configuration": {
      "title": "Quon",
      "properties": {
        "quon.lsp.path": { "type": "string", "default": "" },
        "quon.lsp.debounceMs": { "type": "number", "default": 100 },
        "quon.lsp.logLevel": {
          "type": "string",
          "enum": ["off", "error", "warn", "info", "debug", "trace"],
          "default": "info"
        },
        "quon.fmt.path": { "type": "string", "default": "" },
        "quon.fmt.formatOnSave": {
          "type": "boolean",
          "default": false,
          "description": "Informational; authoritative default is configurationDefaults [quon].editor.formatOnSave = false"
        }
      }
    },
    "commands": [
      {
        "command": "quon.restartServer",
        "title": "Quon: Restart Language Server"
      }
    ]
  }
}
```

---

*End of plan. Implementation should follow this document. Reviewer blockers 1–7 (xvfb, `QUON_FMT_PATH` symmetry, format-on-save OFF, `test/corpus/`, runTests workspace, CI code-action assertion, Apache-2.0 LICENSE) are locked above — do not re-open without a plan amendment.*
