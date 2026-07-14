# Homebrew tap scaffolding (Phase D / #235)

Quon’s end-user Homebrew install is a **bottle-only** formula: it downloads the
self-contained GitHub Release tarball for the host arch. There is **no**
runtime `depends_on "llvm@22"` or `z3`.

## User install (once the tap exists)

```bash
brew install arniber21/quon/quon
# or: brew tap arniber21/quon && brew install quon
```

## Files in this directory

| File | Role |
|------|------|
| `quon.rb` | Formula template with `__VERSION__` / `__SHA256_*__` placeholders |
| `README.md` | This publish guide |

Regenerate a filled formula after a release (or from local `dist/` archives):

```bash
./scripts/generate-homebrew-formula.sh [--version 0.1.0] [--dist-dir dist]
# writes dist/quon.rb (placeholder template packaging/homebrew/quon.rb unchanged)
```

Tag builds attach `quon.rb` (with real checksums) via `.github/workflows/release.yml`.

## Publishing to `arniber21/homebrew-quon`

The external tap repo is **not** created by this PR (separate GitHub repo). After
the first tagged release that uploads archives + `quon.rb`:

1. Create an empty public repo `arniber21/homebrew-quon` (Homebrew requires the
   `homebrew-` prefix for `brew tap arniber21/quon`).
2. Copy the generated formula to `Formula/quon.rb` in that repo:

   ```bash
   gh release download vX.Y.Z -R arniber21/quon -p quon.rb
   mkdir -p Formula
   mv quon.rb Formula/quon.rb
   git add Formula/quon.rb
   git commit -m "quon X.Y.Z"
   git push
   ```

3. Verify:

   ```bash
   brew untap arniber21/quon 2>/dev/null || true
   brew install arniber21/quon/quon
   quonc --version
   # Confirm no LLVM/Z3 brew deps:
   brew info arniber21/quon/quon
   ```

4. On each subsequent tag, replace `Formula/quon.rb` with the formula asset from
   the GitHub Release (or run `generate-homebrew-formula.sh` against the release
   assets and open a PR in the tap).

Optional later: a GitHub App / PAT with write access to the tap can automate the
commit from `release.yml`. Until then, the formula release asset is the handoff.

## Why not `depends_on "llvm@22"`?

Phase C release binaries statically link MLIR/LLVM and libz3. Adding those
Homebrew deps would force users to install multi‑GB toolchains they never need
at runtime and would fail Homebrew’s “runtime dependency” intent for bottles.
