# Adversarial code review — Issue #107

**Branch**: `issue-107` (stacked on `issue-105` / PR #158)  
**Verdict**: APPROVED TO SUBMIT (scoped RAP cut)

## Spec

| Criterion | Status |
| --------- | ------ |
| Placement cost = routing cost (Eq. 1) | ✅ `routing_cost_eq1` / aware search |
| Dual placer (agnostic + aware) | ✅ `PlacerMode` |
| Entangle only in entanglement zone | ✅ validator + schedule places into EZ |
| Readout constraint (AbstractModel) | ✅ `require_readout_zone` |
| Reuse skip | ✅ zero-distance skip |
| RAP section citations | ✅ module rustdoc |
| Soft-unblock vs #106 | ✅ RAP-local grouping |

## Residual

- Full Sec. V-A BST AOD encoding and Eqs. (3)–(5) inadmissible heuristic deferred for #111 tuning.
- Occupancy tracking on moves is best-effort; capacity overflow path exists.
- Human review of RAP fidelity still recommended (former HITL flag).
