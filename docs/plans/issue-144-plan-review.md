# Issue #144 Plan Review — Adversarial Approval

**Plan:** `docs/plans/issue-144-plan.md`  
**Reviewed against:** issue #144, current website, all eight `test/verify/*.qn` fixtures and Python verifiers, `frontend/tests/fixtures/`, issues #137 and #139  
**Reviewer stance:** Assume documentation can compile while still misleading users.

## Grade: A-

## Decision: APPROVED

The plan is executable and keeps scope narrow. Approval depends on retaining the safeguards below during implementation.

## Adversarial findings

### Fixture duplication could silently drift

Copying eight programs into MDX would create untested forks. Raw `?raw` imports from `test/verify/` remove that drift class and make missing fixtures a website build failure.

### “Expected Aer outcome” can overstate what is verified

QFT's histogram cannot expose phases; Ising at `t=0` checks the identity boundary; the Shor fixture uses schematic modular multiplication. The plan accurately limits all three claims and must preserve those caveats on their pages.

### Generic simulation commands are weaker than repository verifiers

A plain Aer pipeline shows counts but does not enforce tolerances, bit ordering, seed stability, or both teleportation bases. Each page must feature the checked-in Python verifier as the authoritative simulation command.

### Upstream docs are absent

#137 and #139 remain open, so a complete site navigation rewrite or a substitute language guide would violate scope. A cookbook-only sidebar group and forward links to `/language/` keep the integration boundary explicit. Route fragments remain a known merge-time risk.

### Raw imports cross the website package boundary

Vite may reject files outside `website/` depending on workspace-root discovery. This is acceptable only if the actual production build proves the imports work. If it fails, configure a narrow read allowance for repository fixtures; do not copy sources as a workaround.

### Shor naming invites overclaiming

The page title and prose must say “Shor kernel,” describe the checked composition, and explicitly deny full period finding/factorization.

## Approval checklist

- [x] Eight distinct cookbook routes are planned.
- [x] Tested fixtures remain the source of truth.
- [x] Compile and assertion-bearing simulation commands are specified.
- [x] Verification claims match verifier thresholds.
- [x] Language-guide cross-links are included without implementing #139.
- [x] Sidebar work is limited to this cookbook.
- [x] Website build and all eight Aer checks are mandatory.
- [x] Residual dependency-route risk is explicit.

No blocking plan changes remain.
