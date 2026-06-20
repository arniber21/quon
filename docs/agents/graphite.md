# Pull requests: Graphite

Use the [Graphite CLI](https://graphite.com/docs) (`gt`) for branch stacking and opening/updating pull requests. GitHub Issues stay on `gh` — see [issue-tracker.md](./issue-tracker.md).

## Prerequisites

- Graphite CLI installed (`brew install withgraphite/tap/graphite` or [official install docs](https://graphite.com/docs/install-the-cli))
- Authenticated: create a token at https://app.graphite.com/settings/cli, then `gt auth --token <token>`
- Repo initialized once per clone: `gt init --trunk main --no-interactive`

## Terminology

| Term | Meaning |
| ---- | ------- |
| **trunk** | Base branch stacks merge into — `main` in this repo |
| **stack** | Ordered PRs where each branch builds on its parent: `main ← PR A ← PR B` |
| **downstack** | Ancestors of the current branch (closer to trunk) |
| **upstack** | Descendants of the current branch |

## Agent workflow

### Starting work

1. Sync trunk: `gt sync --no-interactive` (or `git fetch origin && git checkout main && git pull --ff-only origin main`)
2. Create a stacked branch: `gt create <branch-name> -m "commit message"` (stages, commits, and branches in one step)
3. For follow-on work on the same stack: `gt create <next-branch> -m "..."` from the tip branch

Never commit directly on `main`. If you accidentally do, move commits to a feature branch before submitting:

```bash
git branch <feature-branch>
git checkout main
git reset --hard origin/main
git checkout <feature-branch>
gt track --parent main --no-interactive
```

### Updating a branch

- Amend the current branch and restack upstack: `gt modify -m "updated message"`
- Rebase the stack onto latest trunk: `gt sync --no-interactive` then `gt restack` if needed

### Submitting PRs

Submit the current branch and all downstack branches:

```bash
gt submit --no-interactive --no-edit
```

Useful flags for agents:

| Flag | Purpose |
| ---- | ------- |
| `--stack` / `gt ss` | Also submit upstack branches with open PRs |
| `--draft` | Open new PRs as drafts |
| `--dry-run` | Show what would be submitted without pushing |
| `--restack` | Restack before push if trunk moved |

Prefer `gt submit` over `gh pr create` or raw `git push`. Graphite handles force-with-lease, stack metadata, and PR creation/update on GitHub.

### Reading stack state

```bash
gt log          # graphical stack view
gt branch info  # current branch metadata
```

### Issue tracker integration

- Reference issues in PR titles/descriptions (`Fixes #123`, `Refs #456`) — Graphite passes metadata to GitHub.
- Use `gh issue view` / `gh issue comment` for issue reads and updates; use `gt submit` for the PR itself.
- After submit, fetch the PR URL with `gh pr view --json url -q .url` on the branch, or open Graphite / GitHub in the browser.

## Git safety

- Do **not** force-push `main` or any branch you do not own.
- Do **not** run `git push --force` manually when Graphite is configured — use `gt submit`.
- Do **not** skip hooks (`--no-verify`) unless the user explicitly requests it.
- Do **not** update git config.

## Troubleshooting

- **Not authenticated**: `gt auth --token <token>` from https://app.graphite.com/settings/cli
- **Branch untracked**: `gt track --parent main --no-interactive` (or `--parent <parent-branch>`)
- **Restack conflicts**: resolve in the conflicted branch, then `gt continue` / `gt restack`
- **Trunk out of date**: `gt sync --no-interactive`

More: https://graphite.com/docs/cheatsheet
