# Validation

Static analysis and refinement-type checks for the Quon workspace.

## Taskless (ast-grep rules)

[Taskless](https://github.com/taskless/skills) ships project-specific validation rules under `.taskless/rules/`. Rules are checked with ast-grep via the `@taskless/cli` package — no API auth required for `check`.

### Prerequisites

- Node.js 20+ (for `npx`)

### Run locally

```bash
# Full workspace scan
npx @taskless/cli@latest check

# Changed files only (e.g. before opening a PR)
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)

# JSON output (for agents / scripts)
npx @taskless/cli@latest check --json
```

### Add or change rules

1. Read the canonical skill at `.taskless/skills/taskless/SKILL.md` (or invoke `/tskl` in Cursor).
2. Author rules in `.taskless/rules/*.yml` with matching tests in `.taskless/rule-tests/`.
3. Verify a rule before committing:

   ```bash
   npx @taskless/cli@latest rule verify <rule-id> --json
   ```

4. Re-run `check` against the codebase.

### CI

`.github/workflows/taskless.yml` runs a diff-scoped check on pull requests and a full scan on pushes to `main`. No secrets required.

### Agent skill

Cursor and Claude Code hold thin stubs (`.cursor/skills/taskless/`, `.claude/skills/taskless/`) that delegate to `.taskless/skills/taskless/SKILL.md`. Re-install after upgrading the CLI:

```bash
npx @taskless/cli@latest init --no-interactive
```

## Flux (refinement types)

See [README.md](../../README.md#flux-refinement-types) for installing `cargo-flux` and running refinement checks on the `flux_verify` crate.
