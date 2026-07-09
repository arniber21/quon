# Adversarial review — Issue #107 RAP plan

**Plan**: `docs/plans/issue-107-plan.md`  
**Verdict**: APPROVED FOR IMPLEMENTATION (scoped cut)

## Attacks

### A1. Claiming full RAP A* while shipping greedy
**Resolution**: Document routing-aware mode as best-first / A*-style search over partial pair assignments with Eq. (1) cost; if heuristic is simplified vs Eqs. (3)–(5), say “inspired by” and leave full inadmissible heuristic for #111 tuning. Still satisfy “placement cost = routing cost”.

### A2. Hard-blocking on #106
**Resolution**: Soft-unblock; RAP-local grouping only.

### A3. Attributing readout rules to RAP
**Resolution**: Cite AbstractModel for readout; RAP for storage+entanglement.

### A4. Scope explosion (full BST AOD)
**Resolution**: Row/column non-crossing compatibility checks; defer full Sec. V-A BST.

## Verdict
**APPROVED** — implement zone types, Eq. (1) cost, dual placers, validators, tests.
