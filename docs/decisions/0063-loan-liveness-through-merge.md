# ADR-0063 — Borrowck: Point-Level Loan Liveness at Drop (UAF Across Block-Merge)

- **Status:** 🔒 LOCKED — G sign-off 2026-06-19. Drafted by Mentor O 2026-06-19, empirical recon (3 candidate implementations measured and reverted).
- **Date:** 2026-06-19
- **Author:** Mentor O (deep-recon of Bug A heap-nullable → unearthed pre-existing UAF in `Expr::If`/match reference-arms).
- **Related:** [ADR-0046](0046-propagated-loan-liveness.md) (PropagatedLoan return-borrow bounded by dest liveness — this ADR REFINES its Drop-check) · ADR-0045 (Reference = Copy) · CFG-tail Slice 1 (`159fd68`, Bug A block-tail drop escape).

---

## 1. Context — UAF Across Block-Merge, Silent Borrowck

`let r = if c { let inner = "hello"; id(&0 inner) } else { … }; length(r)` → **returns 5, NO E2450** = silent use-after-free execution. Borrowck missed it. Fixture 102 (same pattern, plain block) catches E2450; the if-wrapped variant slipped through.

**MIR (then-arm + merge):**
```
Call id(_2) → [_3]      ; PropagatedLoan source=_1(inner) dest=_3
Drop(_1)                ; block-pop drops inner
_4 = move _3            ; If-merge: result _4 = then_val _3
… length(_4) …          ; uses _4 after _1 has died → UAF
```

**Root Cause (`checker.rs:780-784`, ADR-0046 Drop-check):**
```rust
loan.source.local == *l && (!loan.is_propagated
    || liveness.blocks[block.0].live_out.contains(&loan.dest))
```
The Drop-check used **block-level `live_out`**. `_3` (loan dest) was consumed by `_4 = move _3` IMMEDIATELY within the block → NOT in `live_out` → check missed. The borrow remained live through `_4` (live_out) while the loan pointed to `_3` → missed.

**Why Block (fixture 102) caught it:** Slice 1 provided a **direct-return** for reference-tails (no Assign-to-merge); `_3` WAS the block result, used via `length(_3)` outside the block → `_3 ∈ live_out` → E2450. In If/match, **merge MANDATES an Assign** `_4 = move _3` (CFG converging two branches) → `_3` died within the block → missed.

## 2. Alternatives Considered (Empirical Recon — NOT Guessed)

> Initial framing (G): "loan-follow through reference Assign — Duplicate loan (since Reference is Copy), not Retarget". **Recon DISPROVED both** experimentally.

- **(a) Duplicate loan in Assign handler** (dest==source → copy loan with new dest): **FAILED empirically** — headline case still returned 5. Timing issue: `Drop(_1)` precedes `_4 = move _3` in dataflow order; when Drop was processed, duplication had not yet occurred; loan still targeted dest=_3, still missed. Retarget suffered from the same flaw.
- **(b) NAIVE Point-Level Liveness** (dest used-later in block, COUNTING `Drop(dest)`): **2 false-positives** — fixtures 84/101 (valid return-borrow) erroneously failed with E2450. Because `Drop(msg)` preceded `Drop(r)` at scope-end → `Drop(r)` was counted as "r used later".

## 3. Decision — Point-Level READ-after-Drop Liveness in Drop-Check

The Drop-check adds a condition: loan dest **is READ** (not Dropped) at a subsequent statement within the SAME block:
```rust
let dest_used_after = body.blocks[block.0].statements[stmt_idx+1..].iter().any(|s| match s {
    Assign{source,..} | Borrow{source,..} | GetDiscriminant{source,..} => source.local == dest,
    BinaryOp{left,right,..} => left.local==dest || right.local==dest,
    _ => false,   // Drop(dest) is NOT a use — dest is dying
});
has_active_loans = loan.source.local == *l && (!loan.is_propagated
    || live_out.contains(&loan.dest) || dest_used_after);
```

**Core Invariant:** *Reading a reference AFTER its borrowed source has been Dropped (within the same frame) = UAF — always E2450. Dropping that reference itself (dying together) = safe.* → the rule produces NO false-positives **by construction**: no valid program reads a reference after its source is dead.

**Why this is the correct location (not lowerer, not loan-following):**
- The fix resides in **borrowck Drop-check**, construct-AGNOSTIC → covers If + match + ALL future merges at a SINGLE point, WITHOUT modifying lowering (G's outline of "guarding If/match lowering" = 3 sites + breaks merges).
- `live_out OR read-after-same-block` = complete point-level coverage: cross-block escape (live_out: terminator/successor) + same-block consumption (read-after). Perfectly bridges the gap left by ADR-0046.

## 4. Empirical Evidence (Experimental Branch, Reverted)

| Alternative | Headline If-ref | Regression 204 + Workspace |
|---|---|---|
| (a) Duplicate loan | ❌ still 5 | — |
| (b) Point-level naive (Drop=use) | ✅ E2450 | ❌ 84/101 false-positive |
| **(c) READ-after, Drop excluded** | ✅ E2450 | ✅ **204/204 + workspace 0 FAILED** |

Clean-tree confirmation: UAF=5; with fix=E2450 → load-bearing.

## 5. Mandatory Teeth (Upon Implementation)
- **Headline Fixture:** If-reference-arm UAF → E2450. Poison: remove `dest_used_after` → returns 5 (UAF returns) → RED.
- **Strict Regression:** 84/101 (return-borrow) RETAIN passing status (no false-positives); 102/20/21/24 (E2450/borrow) RETAIN correctness; full 204 + workspace pass.
- **Match-Arm:** ✅ VERIFIED via **Trit-param match** (fixture `214_match_arm_uaf_e2450.tri`) — E2450 fires; poison `dest_used_after` → UAF returns (compiles + runs returning 2) → RED. Drop-check construct-agnosticism covers **If + match (Trit-param)** at the same point. *Note: scrutinee literal Integer/Trilean is not yet supported (separate feature — value-keyed SwitchInt), does not affect the construct-agnostic nature of the fix.*

## 6. Consequences
- **Positive:** Closes UAF class (reading ref after source-drop) for all merges; 1-point borrowck fix; 0 regressions measured; DOES NOT touch lowerer/loan-model (less risky than loan-duplication).
- **Cost:** Drop-check adds an O(remaining-statements) scan per Drop — bounded, acceptable.
- **Frozen Scope:** Only same-block read-after; cross-block is already handled by live_out. If comprehensive point-level liveness (per-statement liveness) is needed later → separate ADR.
- **ADR-0046 Correction:** Its Drop-check (block live_out) was an approximation; this ADR formalizes same-block read-after as a valid liveness condition.

## 7. Signatures
- O: ✅ (empirical recon, 3 options measured, grounded MIR fix + 0-regression)
- G: ✅ (approved 2026-06-19 — new ADR supersedes ADR-0046 [no amendments, history preserved]; match-arm kept UNVERIFIED transparently; fix point-level READ-after-Drop in borrowck Drop-check, not lowerer/loan-follow)
- **Amendment 2026-06-19 (Post-Signing, Traceable Edit):** The phrase "match-arm kept UNVERIFIED" in G's signature above was cleared by O on the same day — verified via Trit-param match (fixture `214_match_arm_uaf_e2450.tri`, §5): E2450 fires, poison `dest_used_after` → UAF returns (yields 2). The original signature is retained as historical record; **§5 represents the current state** (match-arm VERIFIED, construct-agnostic covering If + match). Integer/Trilean literal scrutinees remain a separate, unimplemented feature.
