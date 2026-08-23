---
name: campaign_cfg_tail_drop_ordering
description: ★ IN PROGRESS 2026-06-19 — Campaign CFG-Tail Drop Ordering (opened by G). Bug A = block-tail-expr returning a let-bound heap local gets dropped TOO EARLY by pop_scope → E2421. O's recon done, awaiting WO→D.
metadata:
  node_type: memory
  type: project
  originSessionId: cfg-tail-drop-ordering
---

**Campaign CFG-Tail Drop Ordering** (opened by G 2026-06-19, prioritized ahead of Slice 5 `?+>`). Sprang from the heap-nullable deep-recon: discovered **Bug A block-init E2421** which breaks even plain non-nullable Vector (orthogonal to heap-nullable + map).

## Bug A — root cause (O dug it out, file:line, NOT a guess)
**Symptom:** `let v: Vector<Integer> = { let mutable t = vector_new(); t = push(t,7); t }; len(v)` → **E2421 "use after storage end"**. Breaks `String` too (`{ let x="hi"; x }`) + plain non-nullable.
**MIR:** bb2 `_2 = move _4` (reassign t) → `Drop(_2)` IMMEDIATELY → `len(_2)` used after Drop.
**Root (triet-lower/src/lib.rs):** `Expr::Block` arm (2401-2434): push_scope → lower statements → lower tail `final_expr`→`result` → **`pop_scope()` (261-276) drops EVERY owned-local in the scope INCLUDING `result`** if result is a `let`-bound heap local. `let mutable t` push_owned(_2) (1236); tail `t`=variable-ref returns _2 directly (no copy); pop_scope drops _2; then the consuming Let push_owned(_2) again at the outer scope. **NO escape/remove mechanism for owned_locals exists** (only push_owned adds more + pop_scope drains — grep confirms 0 remove/retain).
**Why function `mk()=...{...;t}` ESCAPES:** tail→`Return(t)` + M4 return-escape (JIT skips Drop for locals in Return). An expression-block has NO Return → no escape → bug.

## EXACT trigger (clean control, full-output)
| Case | Result |
|---|---|
| block-init `{ let mutable t=...; t }` Vector/String | E2421 |
| if-arm let-bound heap tail | E2421 |
| double block-init | E2421 |
| if-arm tail = direct call `{ vector_new() }` | OK (call-result does NOT push_owned) |
| block tail = call `{ vector_new() }` | OK |
| block-init scalar `{ let a=5; a }` | OK (scalar no Drop) |
| match-arm let-bound heap tail | **UNVERIFIED** (match Trilean-literal lowering not yet supported — a separate limitation, could not build a repro) |
**Trigger = tail is a variable-ref to a `let`-bound HEAP-owned local within the scope.** Scopes shared by push_scope/pop_scope: block 2403/2432, if-arm 2914/2951, match-arm 3102.

## Relation to SIGILL 132 (G asked)
Siblings (same block-tail-expr family) BUT different mechanism: SIGILL 132 = struct-by-value tail-RETURN routing through sret (`emit_struct_sret_copy` 1090, function-body return path, ALREADY fixed). Bug A = owned-local drop-escape within an expression-block (NO Return, NO M4-escape). Function-body escapes via M4; expression-block does not → Bug A.

## ★ FINAL fix shape (O verified the fix-point, CORRECTED the initial proposal)
**The SOLE fix-point = `Expr::Block` (lib.rs:2401-2434, pop_scope at 2432).** Verdict across 4 constructs: Block returns the tail-local directly (`result=lower_expr(tail)`) → pop_scope drops it = BUG. `Expr::If` (2435, NO scope, Assign-to-outer), Trit-match arm (2914/2951), Nullable-match arm (3102) **are ALREADY CORRECT** — all do `Assign(result, body_val)` BEFORE pop_scope (move→M1 tombstone→pop-drop becomes no-op). If-arm E2421 only happens because the arm CONTAINS an Expr::Block — fixing Block clears the if-arm automatically.
**Do NOT invent `pop_scope_escaping` (unnecessary invention).** The correct idiom already exists in 3 places. Fix = mirror it: within Expr::Block, after `lower_expr(tail)→tail_val`, allocate a fresh `result` typed from tail_val, `Assign(result, tail_val)` (move→M1 tombstone the tail-local), THEN pop_scope (drops the tombstoned tail-local = no-op), return result. SSOT-consistent, 0 new mechanism.
**Teeth: N7 counting block-init heap freed-EXACTLY-once (leak vs double-free, both directions); poison the Assign-move (remove it → revert to direct-return) → E2421/double-free go RED. Covers block + if-arm-containing-block, Vector + String. match-arm enum UNVERIFIED (match-literal limitation).**
(the OLD proposal pop_scope_escaping = dropped; G had approved it but O's fix-point verification found the Assign-to-outer idiom already exists → an SSOT mirror instead of a new helper.)

## ✅ Slice 1 (Bug A) — SIGNED by O, awaiting G finalize+push — commit `159fd68`
Fix in `Expr::Block` (2437-2449): non-reference tail → `Assign(fresh_result, tail_val)` move (M1 tombstone) before pop_scope; **reference tail → direct-return `Ok(tail_val)`** (guard 2438). 3 fixtures 207/208/209 + N7 `block_tail_drop_counting` (Vector+String).
**O verified with blood (committed tree):** baseline 207→1/208→5/209→1/102→E2450; N7 2/2 green. Poison the else→direct-return → 207/208/209 E2421 + N7 count2 for both Vector and String (left:2 right:1). **MISALIGNED ORDER on the `is_reference` guard — O VERIFIED IT WAS CORRECT with blood:** removing the guard (unconditional) → 102 regresses E2450→**E2440** (weakens the A1 live-bomb guard). D's flag was correct, data backs it up. Reference = Copy, pop_scope doesn't drop it → no Bug A → the direct-path is correct (byte-identical to the old code). build/clippy 0, tree reverted clean.

## ⚠️ NEW HOLE O DUG UP (pre-existing, OUTSIDE Slice 1 scope) — Expr::If/match reference-arm MISSES E2450
**Proven with blood:** `let r = if true { let inner="hello"; id(&0 inner) } else {...}; length(r)` → returns **5, NOT E2450** = a running UAF (length reads freed memory). Fixture 102 (plain block, same pattern) catches E2450; the if-wrapped version MISSES it. **Worse than D's flag** ("in theory E2450→E2440" — actually MISSES the diagnostic entirely, exploitable). **Pre-existing (proven by diff):** D only touched Block; the reference-arm returning `Ok(tail_val)` is byte-identical old code; Expr::If's `Assign(result, then_val)` (2476) was NOT touched by D. The culprit = unconditional `Assign(result, reference)` at If(2476)/match breaking loan-propagation → loses E2450 — exactly what D's guard blocks at Block. **→ Slice 2 CFG-tail (or borrowck debt): apply the is_reference guard to Assign-to-result in If/match.** match-arm still UNVERIFIED (literal limitation).

## ★ Slice 2 RECON (O dug in, OVERTURNED G's outline "carry the is_reference shield over from Block")
**G's outline does NOT translate:** Block returns ONE tail directly (skipping Assign is OK). If/match **MERGE 2 branches into 1 `result` via Assign, MANDATORY** — skipping the Assign means result never gets written → breaks the merge. There is no lowerer-side fix.
**The real root (borrowck, MIR-confirmed):** `Call id(_2)→[_3]` creates a PropagatedLoan with source=_1 dest=_3. Merge: `Drop(_1)` then `_4 = move _3`. The `Drop` check (checker.rs:780-784) uses **block-level `live_out`**; _3 is consumed IMMEDIATELY by `_4=move _3` within the block → not ∈ live_out → check misses it → E2450 missed. **The `Statement::Assign` handler (609-695) does NOT follow the loan**: it only sets dest→Owned (694), the loan still points at the now-dead dest=_3, never transfers to _4.
## ★★ Slice 2 ADR-0063 DRAFT (O's EMPIRICAL recon — trial tree + revert, clean tree `159fd68`)
**G decided ADR-first + deep-recon-regression. O trial-implemented-measured-reverted 3 options, REFUTING G's framing "loan-follow Duplicate vs Retarget":**
- **(a) Duplicate loan @ Assign handler** → FAIL: headline still 5. Timing — `Drop(_1)` is processed BEFORE `_4=move _3` in the dataflow; at Drop-time, duplication hasn't happened yet. Retarget suffers the same disease.
- **(b) point-level naive (dest used-after, COUNTS Drop)** → E2450 fixed but **2 FALSE-POS on 84/101** (return-borrow; `Drop(msg)` happens before `Drop(r)` → r gets computed as "used after").
- **(c) CHOSEN — point-level READ-after-Drop (exclude Drop from the scan)** → headline E2450 fixed + **204/204 integration + workspace 0 FAILED**.
**Real fix = BORROWCK Drop-check (NOT lowerer, NOT loan-follow):** checker.rs:780-784 adds `dest_used_after` = loan-dest is READ (as Assign/Borrow/BinaryOp/GetDiscriminant source, NOT Drop) in a LATER statement within the SAME block → `has_active_loans |= dest_used_after`. Invariant: reading a ref after the source has dropped in the same frame is always a UAF = always E2450; Dropping that ref is safe → **0 false-pos by construction**. Construct-agnostic → covers If+match+every merge with 1 point (G's outline "guard 3 lowering sites" = breaks the merge, REFUTED). Corrects ADR-0046 (block live_out = an approximation, add same-block read-after).
**ADR-0063 LOCKED (signed by O+G, commit `fed21fc` local).** New ADR supersedes ADR-0046 (not an amendment — G's ruling doesn't erase history). match-arm UNVERIFIED sign stays up.

## ✅ Slice 2 (UAF through merge) — SIGNED by O, awaiting G finalize+push — commit `51e401b`
Fix in checker.rs Drop-check (806): `|| dest_used_after(loan.dest)` — the point-level READ-after-Drop clause per ADR-0063 §3 (Drop excluded from the scan). 1 borrowck point, NOT lowerer. Fixture 210 (If-ref-arm UAF).
**O verified with blood (committed tree):** baseline 210→E2450, 84/101→5, 205/205 integration + workspace 0 FAILED + borrowck 23/23. **Poison removing clause 806 → 210→5 (UAF returns) RED while 84/101 STAY at 5** (proves the clause ONLY ADDS the merge-UAF catch, doesn't affect return-borrow → 0 false-pos by construction). Tree reverted clean, build/clippy 0. Matches the recon's 204/204 (now 205/205 +fixture 210). The UAF class is closed for every merge with 1 construct-agnostic point.

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_heap_nullable]] [[campaign_cfg_tail_expression_kickoff]]

## ✅ match-arm UNVERIFIED RETIRED (pushed `cef6b4c`) + NEXT match-on-literal
**O's sniff overturned it: UNVERIFIED = TOO CONSERVATIVE** (O gave up too early, never tried a Trit-param). Fixture `214_match_arm_uaf_e2450.tri` (`match t:Trit { -1_trit => {let a; id(&0 a)} ...}` ref ESCAPES the arm → used after merge) → **E2450**; poisoning `dest_used_after` (checker.rs:806) → 214 returns 2 (UAF returns) RED → ADR-0063 covers If + match-arm, proven with blood. ADR-0063 §5 removes the warning flag + §7 amendment syncs G's signature (keeps the original wording). 0 production code. **Lesson for O (G scolded): exhaust every direction before slapping on UNVERIFIED — refuse-fabricate is good, but giving up too early is bad.**
**NEXT — match-on-literal campaign (a pure FEATURE, G's call):** match is refused at LOWER (Expr::Match enum-path fallthrough lib.rs:3797-3800); 4 dispatch branches Trit(2924 value-SwitchInt)/nullable(3040)/Outcome(3288)/else-enum. Integer/Trilean literals have NO value-path → mirror the Trit-path 2924 (value-keyed SwitchInt). Exhaustiveness: Trilean=3 values (true/false/unknown) can be exhaustive; Integer=needs a wildcard. The "unblock teeth" rationale is moot — pure feature per Giang's vision.

## ★ IN PROGRESS — Campaign Match-on-Literal (ADR-0064 LOCKED `1c26010` local, WO handed to D)
ADR-0064 (signed O+G): Exhaustiveness rule — Integer needs a wildcard, Trilean/Trit needs all-3-faces-or-wildcard. Encoding Trilean True=1/False=-1/Unknown=0 (lower:1464). **Temporarily trap GAP-2 at lower; the Typecheck-Exhaustiveness debt (compile-time) is a SEPARATE campaign (G forbids bundling them).** WO: Expr::Match lowering adds 2 value-keyed branches (scrut_ty==Trilean, ==Integer) BEFORE the enum-path 3792, **mirroring the Trit-path 2924** (forbidden to invent a new branching pattern): cases Vec<(i64,bb)> + wildcard-last + SwitchInt + default→wildcard-body-else-Trap. Teeth: correct branch for correct value · trap for missing-branch SIGILL · poison default-Trap→goto-merge→garbage value RED · Trit 174+209 corpus no-regress.

## ✅ Match-on-Literal CLOSED — SIGNED by O, awaiting G finalize+push — commit `d85b794` (ADR-0064 `1c26010`)
3 lower branches: Trit(2924)/Trilean(3045 key True=1/False=-1/Unknown=0)/Integer(3161 key=value), mirrored shape, BEFORE the enum-fallthrough. GAP-2 default→wildcard-else-Trap(3253). Fixtures 215→129/216→123. **O verified with blood:** Integer/Trilean take the correct branch; classify(9) no-wildcard → SIGILL 132; poisoning Integer Trap(3253)→Goto → exit 0 returns 0 (garbage no-trap) RED; reverted→SIGILL. Trit 174→111, 211 corpus, workspace 0 FAILED, build/clippy 0. **Limitation (D flagged, O verified = CLEAN refuse not silent-wrong):** bare `let x=2`→Unknown→enum-fallthrough refuses; needs an Integer/Trilean-typed scrutinee (type-inference literal-default-Unknown gap, pre-existing, out of scope). The Typecheck-Exhaustiveness (compile-time) debt is still pending = a separate campaign (ADR-0064 §4).
