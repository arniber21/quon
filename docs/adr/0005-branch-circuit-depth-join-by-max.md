# Join branch circuit depths by `max`, keep annotation depths strict

When an `if`/`match` is type-checked in **synthesis** mode and its arms are circuits that
agree on input/output widths, the result circuit's depth is the **maximum** of the arm
depths (`DepthExpr::par`), not a value all arms must share. A classically-selected correction
like `if bit then X else identity(1)` therefore synthesizes `Circuit<1, 1, max(1,0), Clifford>`
= depth `1`, with `identity(1)` keeping its honest depth of `0`. The Clifford class joins
(Universal absorbs Clifford); widths must still match structurally.

This rule lives in `join_branch_types` (`frontend/src/typecheck/mod.rs`) and is applied by
`branch_if` and `check_match` only in their `None` (synthesis) arms.

## Context

Issue #13 ("Z3 refinement bridge") describes the `match`-branch case as: branches that
"produce circuits with different symbolic depths … must be shown equal", erroring otherwise.
Issue #14 requires the spec's `teleport` reference algorithm to type-check end-to-end, and
`teleport` contains:

```kotlin
let b2 = (if x_bit then X else identity(1)) @ b
```

`X : Circuit<1,1,1,Clifford>` (depth 1) but `identity(1) : Circuit<1,1,0,Clifford>` (depth 0).
Under a strict equal-depth rule for branches the two arms (1 ≠ 0) make `teleport` ill-typed —
directly contradicting #14. The conflict is real, not a fixture bug: a classical conditional
runs *one* arm depending on a measurement outcome, so its honest worst-case depth is the
`max` of the arms. Padding the `else` arm to depth 1 (e.g. using the depth-1 `I` *gate*
instead of the depth-0 `identity(1)` *combinator*) would mean physically executing a no-op
gate purely to satisfy the type system — less faithful than `max`.

## Considered Options

**Strict equal-depth for branches (issue #13's literal reading), rework `teleport`.** Require
arm depths provably equal via Z3 and error on mismatch. Rejected: it forces edits to a spec
reference algorithm to insert depth-padding gates, encoding a wasteful runtime no-op to please
the checker, and it conflates *inferring* a conditional's depth with *verifying* a user's
annotation.

**`max`-join for branches; Z3 strictness reserved for annotation boundaries (chosen).** Branch
depth is *inference*; the depth a programmer writes in `Circuit<n,m,d,C>` is *verification*.
We keep the two separate:
- Synthesizing a branch's depth → `max` (this ADR), which is total and never errors.
- Checking an inferred depth against a written annotation → strict equality via Z3
  (`verify_depth`/`RefinementCtx::verify_equal`), which errors on a genuine mismatch.

A function whose body branches still has its *annotated* return depth strictly verified, so
#13's guarantee — "the depth you wrote is the depth you get" — is fully preserved; `max` only
fills in a depth that no annotation pins down.

## Consequences

- `teleport` type-checks with **no change** to the reference algorithm or to `identity`.
- `if`/`match` in **checking** mode (an expected type is pushed down, e.g. a function's
  annotated circuit return) is unaffected: every arm is checked against the expected type
  directly, with no `max` relaxation. Strictness there is unchanged.
- #13's Z3 bridge is invoked for the depth obligations that genuinely require proof —
  annotation-vs-inferred equality, including symbolic depths like `n_steps * n` — rather than
  for branch joining. The literal "error if branch depths differ" behavior is intentionally
  not implemented; branch depth difference is resolved by `max`, not rejected.
- A symbolic branch join can leave a `max(...)` in the depth (e.g. `max(d1, d2)` when neither
  arm dominates); this is carried forward symbolically and only discharged by Z3 if it later
  meets a concrete annotation.
