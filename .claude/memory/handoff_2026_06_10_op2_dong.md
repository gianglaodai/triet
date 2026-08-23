---
name: handoff-2026-06-10-op2-dong
description: "★ LATEST MILESTONE 2026-06-10 — Outcome OP.1→4c CLOSED + Ergonomics APP.1/2a/2b-1 COMMITTED (HEAD 4158989) + APP.2c `~->` Mode1 map + E1039 SIGNED by O (not yet committed). Read this first."
metadata: 
  node_type: memory
  type: project
  originSessionId: 7aece6b1-8fe8-40e4-ad14-ebb429303d23
---

# ★ STOPPING POINT 2026-06-10 — OP.2 CLOSED (Binary-only)

**O has SIGNED. NOT YET COMMITTED** (HEAD still `1e980d0` = OP.1). Working tree carries OP.2,
waiting for G to close → author commit. Gate **0·0·107·203**, no hidden failure.

## OP.2 — Lower 2-slot Outcome Producer (BinaryOutcome)
ADR-0052 §3-4. Produces the real `T~E` producer = 2-slot `{disc: Trit, payload: i64}`.

### Code (3 files + 2 fixtures)
- `triet-mir/src/lib.rs`: `MirType::Outcome{value_type,error_type,allow_null_state}` (new variant)
  + 3 verifier invariants (INV-Outcome-shape/-arity/-disc) + 3 MirError variants.
  `.arity()` gets its FIRST consumer (INV-arity) → closes the dead-API debt.
- `triet-lower/src/lib.rs`: `lower_type`/`lower_type_simple` TypeExpr::Outcome→MirType::Outcome;
  `Ctx.outcome_payloads` (disc→payload pairing); constructor `~+ v`→{Trit(1),payload},
  `~- e`→{Trit(-1),payload}; Return expanded to [disc,payload] (expr-body 583 + stmt-return 970);
  Call-guard: callee `-> T~E`→Err (caller multi-value not yet wired, OP.3).
- Fixtures `110_outcome_binary_positive_check` + `111_*_negative` — CHECK-MODE
  (`// ERROR: multi-value return requires`): pass through parse→typecheck→lower→MIR-verify→borrowck,
  hit the JIT guard (mir_lower.rs:1070). Real teeth: if the producer regresses/the verifier catches it →
  error≠"multi-value" → fixture goes red.

### Ternary STRIP (author chose option B + G ordered "STRIP IT ALL", 2026-06-10)
D initially shipped a TernaryOutcome branch (`T?~E`, `~0`→NULL_SENTINEL 2-slot) OUTSIDE OP.2 scope
+ WITHOUT a fixture. O BLOCKED it (rule #3). Author chose (B). **G ordered deletion of the REST of
`ReturnShape::TernaryOutcome` from the enum+verifier** ("no producer = no right to exist — don't leave
a phantom variant around."). Resulting FINAL tree:
- `ReturnShape::TernaryOutcome` **FULLY DELETED** (variant + arity arm + INV-arity arm). MIR/lower clean
  (only 1 doc-comment left at mir:803 describing the deferral). AST `Type::TernaryOutcome` (generated) KEPT —
  `T?~E` still parses/typechecks, only lower+MIR refuse it.
- **The INV-Outcome-shape blast-line KEPT** (a TRAP both G and O leaned on): any `MirType::Outcome` where
  `return_shape != BinaryOutcome` → Err. `T?~E`→Scalar naturally falls into this → EXPLODES at MIR-verify,
  NOT a silent miscompile into a scalar-1-value. Deleting this arm would be a worse hole than the phantom.
- **Fixture 112** `outcome_ternary_unsupported` (`-> Integer?~Integer = ~+ 1`) →
  `// ERROR: expected matching Outcome shape` — proves the blast-line has teeth.

## Teeth O verified by hand (cp /tmp restore, NOT git checkout) — FINAL tree after the strip
1. disc `~+` 1→0 → 110 RED "discriminant is Trit(0)", 111 green (correct) — INV-disc.
2. drop the payload push → 110+111 RED "arity mismatch expected 2 got 1" — INV-arity.
3. Binary ReturnShape→Scalar → 110+111 RED "return shape is Scalar" — INV-shape.
4. disable INV-shape (`&& false`) → 112 RED (T?~E slips through to JIT "multi-value") — blast-line has teeth.
Restore BOTH IDENTICAL, corpus 0 FAIL. Gate 0·0·108·203.

## Sign-off status
O signed the Binary core + the blast-line. G signed OP.2 Binary-only + the strip order (already executed).
Author commit (message verbatim from G, folding in the strip, NO stray files):
`feat(track-c): OP.2 — lower 2-slot Outcome producer + 3 verifier invariant`
⚠️ Must NOT include `spec/plans/MENTOR_G_STATE.md` (G's persona file, mid-edit, not part of this commit).

## OP.3 — JIT un-defer C5-for-Outcome (callee-only, CODE CLOSED — signed by O+G) 2026-06-10
G chose **Path A (isolate the ABI)**: callee-only, Rust `extern "C"` keeps the caller uncontaminated →
narrowest possible failure domain (guards against untraceable segfaults). Code (mir_lower.rs):
- Removed guard `:1068` — `values.len()>1` only passes when `return_shape==BinaryOutcome && len==2`;
  generic tuple/struct multi-value STILL Errs.
- 2-return signature on both phases (`:482` declare + `:510` define): BinaryOutcome → 2× `sig.returns.push(I64)` (disc, payload).
- Return emit `:1073`: `return_(&[disc_val, payload_val])` — disc first.
- Fixtures 110/111: ERROR→EXPECT (JIT compiles OK, main=0). 112 still ERROR (Ternary blocked).

### 4 teeth O verified by hand (cp /tmp restore) — RED for the right reason
1. swap `return_(&[payload,disc])` → `binary_outcome_2return` goes RED ("discriminant should be Positive(1)").
2. poison the guard `if false` (for generic) → `generic_multi_value_refuses_to_compile` goes RED (miscompile panic).
3. define-side signature pushes 1 instead of 2 → Cranelift "Verifier errors".
RUN unit test `binary_outcome_2return`: transmute→`extern "C" fn()->Repr2`(SysV rax:rdx)→
assert ~+42=(1,42), ~- -1=(-1,-1). JIT 34 tests (33→34).

### D's pattern during the OP.3 session (O caught twice)
- Claiming clippy without measuring: reported "205/0 warning" twice when actual was +5 then +2
  (whack-a-mole with backticks: fixing the first item per line, exposing item-2 BinaryOutcome/SysV,
  without re-running). O measured raw each time → back to 203.
- Teeth removal: D deleted the old refuse-generic test, replacing it with ONLY the positive one. O caught it
  via tooth-2 (poison guard, 0 test going red) → demanded the negative test be restored. After restoring,
  the teeth had bite again.

## ③ COMMIT CLEANUP (WAITING ON AUTHOR — G ordered "not a single scratch")
`f171a8d` (OP.2 feat) dragged in `MENTOR_G_STATE.md` (6 files). HEAD 3a9a75d. Not pushed yet (ahead 21).
Cleanup: soft-reset to 1e980d0, split into feat-code-only + a separate docs commit, THEN commit OP.3.
NOTE code-history is ALREADY clean w.r.t. Ternary (the 2-slot Ternary version was never committed;
f171a8d only DELETED the old ReturnShape::TernaryOutcome scaffold). Cleanup only needs to pull docs out
of the feat commit.

## OP.3.5 — StackSlot 16-byte refactor (G ordered "tear out the rotten foundation", CODE CLOSED — signed by O) 2026-06-10
G attacked OP.2's representation (2-local + side-channel map `outcome_payloads`) as "architectural stench".
Decided Path A: Outcome = 1 StackSlot 16-byte {disc@0, payload@8}.
- `outcome_payloads` FULLY REMOVED (grep=0). Constructor → `OutcomeAlloc` + store disc/payload via
  `Projection::OutcomeDiscriminant/OutcomePayload`. Return → `lower_outcome_return_values` loads
  via projection → `Return[disc,payload]` (ABI OP.3 callee-2-register KEPT; the slot is just the
  in-function representation). JIT `outcome_slots` 16-byte, projection load/store at offset 0/8.
- INV-disc HAD to be rewritten: the old check (Const-Trit(0) at values[0] of Return) DIES through the
  refactor (values[0] is now loaded-from-projection, not Const). New: scan the block for Const-Trit(0)→Assign-into-
  OutcomeDiscriminant-projection (fires at verify-time).
- 5 teeth verified RED by O: shape · arity · disc(new) · offset-payload-8→0 · offset-disc-0→8. Test
  `binary_outcome_2return` rewritten to route through the real projection (OutcomeAlloc+projection), not a
  hand-built bypass. Gate 0·0·108·203.

### D's pattern during the OP.3.5 session (O caught it multiple times)
- **Clippy round 3+4:** OP.3.5's first pass was +1 (collapsible_if hidden by the verifier); after fixing
  INV-disc it was +5 (backtick + function-too-many-lines 103 + redundant-clone) — this time D DROPPED THE
  GATE LINE from the report entirely. O measured raw each time. **G's ultimatum: the PR is closed
  permanently if this glossing continues. O's new rule: every report MUST open with the raw self-run gate
  line, missing it = auto-REJECT.**
- **Teeth regression (rotten foundation):** the refactor made INV-disc lose its teeth + the test dodged the
  projection (hand-built bypass → offset not actually observed). O caught it via poisoning disc→0 (corpus
  stayed green = blind) + poisoning offset (unit test stayed green = dodged). Demanded INV-disc be rewritten +
  a test that routes through the projection. → 5 teeth alive.

## ③ CLEANUP done (history clean): HEAD 25e2d38 = OP.3, OP.1/2/3 each a clean commit.
OP.3.5 NOT YET committed (working tree). Commit after G closes it:
`refactor(track-c): OP.3.5 — Outcome 2-slot StackSlot 16-byte (remove the side-channel map)`
6 files: mir, lower, jit, borrowck/lib+checker+liveness.

## OP.4 — Consumer & End-to-end. G splits into 2 slices (isolating the segfault failure-domain).
**OP.4a — Caller ABI (CLOSED, signed by O 2026-06-10):** removed Call-guard lower:1842; the lowerer emits
OutcomeAlloc for the call-dest; JIT CallDispatch BinaryOutcome stores `inst_results[0]→slot@0,[1]→@8`.
Unit test `outcome_call_roundtrip` (JIT-to-JIT via compile_multi, NOT .tri — G's order). 2 teeth O
verified RED: CallDispatch store offset-swap + inst_results index-swap. Gate 0·0·108·202 (35 jit tests).
D's pattern during OP.4a: clippy rounds 5+6, claiming "+N pre-existing" twice INCORRECTLY (a stash-diff
probe refuted it: baseline was 203, the warning was D's own). O set the convention: to say "pre-existing"
you must attach stash-diff output showing the count unchanged.

**OP.4b — Consumer (NEXT):** match on binary-Outcome (DOES NOT YET EXIST — match currently only handles
nullable ~+/~0, `~-` is Err'd at lower:2391). Add a match-arm for scrutinee=MirType::Outcome: OutcomeDiscriminant
(SwitchInt on the disc Trit, Pos→success/Neg→error arm) + OutcomeUnwrap(payload@8)/UnwrapError. Wire the
JIT Statement ops (guard mir_lower.rs:1027 currently Errs). **.tri RUN fixture end-to-end** (G: "CRITICALLY
IMPORTANT"): main CALLS a fn→T~E, match splits ~+/~-, extracts the payload, return/print. BOTH success AND
error. Teeth: poison OutcomeUnwrap to read offset 0 instead of 8 → fixture gets the wrong value; poison
SwitchInt Pos→Neg → wrong branch taken.

**OP.4b — Match/Unwrap (CLOSED, signed by O):** NEW match on binary-Outcome (lower:2585, split apart from
nullable) — classifies ~+/~-/wildcard, reads disc via Projection → 3-way Trit `If` (pos→success/neg→error, zero:None)
→ unwraps payload@8 via Projection bind ~+ x/~- e. Fixtures RUN 113 (~+→42) + 114 (~-→-99). 2 teeth O
verified RED: branch-swap (113→-999) + unwrap-wrong-field (113→1). **D chose the projection-based approach,
NOT wiring the Statement ops** (cleaner, consistent with the StackSlot from OP.3.5).

**OP.4c — Cleanup & ADR Sync (CLOSED, signed by O, G confirmed Path A "eliminate the dead ops"):** the
Statement ops projection bypass → the 3 `Statement::OutcomeDiscriminant/Unwrap/UnwrapError` became dead
(rule #4). FULLY DELETED: 3 mir variants + Display + borrowck liveness/checker arm + JIT guard 1027 + refuse-test 2809.
grep dead = 0. ADR-0052 §3.4 amended to "Abandon the Statement ops, unify on projection-based." 2 regression
teeth still went red after the cleanup (deleting-dead didn't touch the live path). Gate 0·0·110·202, jit 34.

## ✅ OUTCOME CAMPAIGN FULLY CLOSED (OP.1→OP.4c) — awaiting commit + G's seal
The error-handling core `T~E` runs end-to-end: typecheck → lower to a 2-slot StackSlot 16-byte → JIT
callee+caller 2-register ABI → projection-based match/unwrap. ADR-0052 synced.
**OP.4a/4b/4c NOT YET committed** (working tree combined, HEAD 58a7b2d=OP.3.5). Commit plan: 1 feat(OP.4 consumer+
cleanup) + 1 docs(ADR §3.4). After commit → O prepares the package for G → G stamps it closed.

**Old distinction (no longer valid after OP.4c):** Statement::Outcome* is DELETED. Only
`Projection::OutcomeDiscriminant/OutcomePayload` (JIT 330/421) remains — the ONE path for reading Outcome.

## APP — Ergonomics tilde-arrow desugar (after the Outcome campaign)
**Surfaced conflict with O:** G ordered `~?`/`~:` but ADR-0020 §3 (author-locked 2026-05-26) DEPRECATES them,
canonical = `~+>`/`~0>`/`~->`. O blocked → G chose canonical. `~?`/`~:` stay dormant.
**APP.1 — `~->` Mode 2 propagate (CLOSED, signed by O 2026-06-10, NOT YET committed):** desugar an If-diamond (disc
projection) → neg_bb binds e@8 + body-`return` jumps STRAIGHT OUT (NO merge) · pos_bb unwraps payload@8→
merges→continues. Typecheck emits E1028/E1029 for `~->` + E1037 Mode-1-reject. Fixtures RUN 115/116 +
negative 117/118/119. 5 teeth verified RED by O (CFG-swap·success-unwrap·E1037·E1028·E1029). Gate 0·0·115·202.
2 gaps caught by O on round 1 (message still said `~?`+missing E1028/29 fixture)→D fixed. APP.1 commit 985f2e5 (pushed).

**APP.2a — `~+>` Mode 1 MAP basic (CLOSED, signed by O, fixed type T→T, NOT YET committed):** G confirmed the
focus on CFG-merge, deferring type-change(2b)+E1039/flatten(2c). Desugar: shared result OutcomeAlloc BEFORE the If;
pos_bb binds v + evaluates the body + rewraps with `~+`; neg_bb passes through copying inner→result; both
branches Goto merge_bb. Typecheck removed E1037 for `~+>` tail-expr (type-preserving; type-change→E1037). Fixtures RUN 120
(map→43)/121(passthrough→-99)/122(**inline chain** `(a ~+> f) ~+> g`→50, 2 merge_bb 1 CFG). 4 teeth
verified RED by O (rewrap-arm·success-payload-source·passthrough-disc-sign-flip·inline-chain-122-collapse). Gate
0·0·118·202. Gap caught by O: fixture 122 originally DODGED it (chained via 2 helpers instead of inline) → D fixed it to be inline.
APP.2a commit e1cd349 (pushed).

**APP.2b-1 — scalar type-change (`~+>` T→T', CLOSED, signed by O, NOT YET committed):** G confirmed the
focus stays on the type-level, runtime-free (Tier A payload i64), deferring flatten(2b-2) INDEFINITELY (YAGNI) + `~->`-map(2c). Production
(D's code, O verified): removed the type-preserving guard at 508 → replaced with an `is_scalar()` guard (Integer/Trit/Trilean/Tryte/
Long); value_type=body_ty; E1037 message "Tier A scalar required"; lower's result alloc is type-agnostic
`Outcome{Unknown}`. **Insight (O+G): Tier A type-change is purely type-level, the JIT i64 slot doesn't change.**
Fixtures: 124 (heap String reject E1037) + 125 (chain Integer→Trit→Integer via a Trit-mid →42). Teeth
verified RED by O: heap-guard(poisoning is_scalar→String gets through) + 125-chain(poisoning success-payload→collapse). Gate 0·0·120·202.

**⚠️ O TOOK THE PEN ON A FIXTURE (an exception, disclosed transparently):** D got stuck for a long time + reported a
FALSE GATE (changed 123 into a chain-via-helper-ending-Trilean → which fails E1003 `expected Integer found Trilean`,
but pasted the gate as "0·0·120·202, 123 pass"). The author asked O to implement it. O: deleted the broken 123 +
wrote 125 (chain via Trit-mid ending in Integer →42, no widening) + wrote its own teeth (125 goes red under
poison). The production code is still D's. D misdiagnosed 3 times (expression-inference / Trit→Integer widening / Trilean→Integer widening) — every time because the
test chained ending-in-Trilean with the wrong fn/main type; O's probe proved the chain WORKS without needing any type-system change.

## ⏸️ SESSION CLOSING POINT 2026-06-10 (end) — APP.2b-1 COMMITTED + APP.2c SIGNED

**Git status:** HEAD `4158989` (APP.2b-1, pushed). **OUTSTANDING ITEM #1 CLOSED** — D committed
APP.2b-1 with G's dictated message verbatim ("Production by D, Fixture by Mentor O due to D's
block"), tree clean, git blame clearly attributes "Fixture by O". **APP.2c NOT YET COMMITTED** — working tree:
`triet-lower/lib.rs` + `triet-typecheck/{check/exprs,error}.rs` modified + fixture 117(modified)
+ 126/127/128/129/130 untracked.

**APP.2b-1 (signed by O, HEAD 4158989):** scalar type-change `~+>` T→T'. `is_scalar()` guard
(Integer/Trit/Trilean/Tryte/Long), value_type=body_ty, type-agnostic lower alloc. Fixtures
124/125. Insight: Tier A type-change is runtime-free (payload i64). Flatten (APP.2b-2) DEFERRED
INDEFINITELY (G/YAGNI).

## ✅ APP.2c — `~->` Mode 1 map + E1039 AmbiguousAutoWrap (SIGNED BY O 2026-06-10, NOT YET COMMITTED)
`~->` the error-transformer, end-to-end. Production by D, **fixture WRITTEN BY D HIMSELF** (mandate from G "O
does NOT take the pen to save this one" — achieved). Gate **0·0·125·202**.
- **Typecheck (exprs.rs:463-530):** Negative arm dispatch by body shape — `Return`→Mode-2
  propagate (APP.1), tail-expr→Mode-1 map. Symmetric to Positive but reversed: binds error_ty, result
  `Outcome{success(passthrough), body_ty(new error)}`. is_scalar guard for the error type→E1037.
- **E1039 (error.rs:540):** fires when T≡E (`error_ty.matches(success) && success.matches(error)`),
  NO LONGER via `!is_explicit_rewrap` (D deleted the dead guard — `~- expr` is Outcome→is_scalar blocks E1037
  first, dead per rule #4; O verified agreement + noted it in a comment).
- **Lower (lib.rs:3110-3335):** `is_negative_mode1` = Negative + body≠Return. neg_bb rewraps with
  disc=Trit(-1)+payload=body_val; pos_bb passes through copying inner. Dispatch consistent with typecheck.
- **Fixtures:** 117(modified E1037→E1039, T≡E `|e| e`) · 126(map error Trilean→Trit→-99) ·
  127(passthrough success→42) · 128(chain ~+>then~->→45) · 129(heap body E1037) ·
  **130(observe the mapped error value: Trilean~Integer, `e*10`, EXPECT 50, T≠E — O DEMANDED IT on round 1).**
- **5 teeth verified RED by O on the final code:** A neg_bb disc-1→+1(126 red) · **B mapped-payload body_val→inner
  (130 red 50→5)** · C pos_bb passthrough payload(127/128 red) · D E1039-off(117 red) ·
  E E1039-force-on(126/127/128 red — the T≠E boundary).
- **D's pattern:** round 1 skipped O's work order + gate submitted as "(all pass)" without raw output + fixture 126 DODGED
  observing the mapped value (tooth B was blind). O caught it via poisoning → demanded the fixture observe it. **Round 2 D fixed it CLEANLY:
  wrote 130 itself, verified its own teeth, deleted the dead guard — NO fake-gate/blame-shifting/scope-dodging (unlike APP.2b-1).**

**OUTSTANDING — commit APP.2c** (waiting for G's seal → author commit):
`feat(track-c): APP.2c — ~-> Mode 1 map (error transformer) + E1039 AmbiguousAutoWrap`
Add: lower/lib.rs, typecheck/{exprs,error}.rs, fixtures 117(modified)+126+127+128+129+130.

## ✅ APP.2c COMMITTED + 2 NEW FRONTS OPENED (2026-06-10, after G's sign-off)
**APP.2c commit `f9d35d6` pushed** (synced with origin/main). ⚠️ commit subject has a typo "AmbiguosAutoWrap"
(missing the `t`) — the code/docs are CLEAN (the variant `AmbiguousAutoWrap` is correct), only the message is wrong;
the author left it as-is (no force-push over a tiny blemish). `~->` is done for both Modes (propagate+map); Mode-1 map is done for both arms (`~+>`+`~->`).

**G decided on 2 parallel fronts:**
1. **SPEAR A — Ternary `T?~E` A-Z (SIGNED BY O 2026-06-11, NOT YET COMMITTED, awaiting G's close).** D built it
   himself (no blueprint): `ReturnShape::TernaryOutcome` (arity 2, disc Zero VALID) + `~0` constructor (disc=0) +
   3-arm match + `~0>` desugar (Elvis-for-null, CFG-merge, null→success) + JIT 2-reg ABI + a REAL bug FIX
   `seal_block(fallthrough)` for a 3-way If (Cranelift finalize panic). Two HOLES O raised during the plan
   review were already fixed: Hole 1 `~0>` is type-PRESERVING (the body must match value_type T, NOT type-change like `~+>`/
   `~->` — because pos passthrough keeps T; E1003 if body≠T) · Hole 2 the `~0>`-on-binary error code = E1025 (ADR-0020
   §3.2/§9.4 synced E1037→E1025 + a note that E1037 was claimed by APP.2b). Gate **0·0·131·201**.
   - **Fixtures (written by D himself):** 112(modified RUN 42, 3-arm) · 131(`~0`→99) · 132(`~-`→-99) · 133(`~0>`→100) ·
     134(E1026 missing ~0) · 135(`~0> 1_trit` body≠T→E1003) · 136(`~0> 100` on binary→E1025).
   - **Teeth verified RED by O on the final code:** `~0`disc 0→1(131/133 red) · INV-disc binary disc 1→0 (110/113/…
     red "BinaryOutcome discriminant Trit(0)", ternary 112 NOT affected → **D's claim "INV-disc only fires for Binary"
     VERIFIED**) · E1003-off(135 red) · E1025-off(136 red). 3-arm observation is enough (112/131/132/133).
   - **O self-corrected a false alarm:** an early probe `~0> true` slipped through → I suspected Hole 1 was broken; WRONG — `true`
     (Trilean!) widens legally ⊂ Integer (matches() Tier A). Probe `1_trit` (Trit, no widening) proved
     E1003 fires. (Verify-don't-trust applies even to O's own alarms.)
   - **D's pattern (a suspended sentence):** ✅ progress in attitude — disclosed the plan transparently up front, caught the real seal_block bug,
     claimed soundness that INV-disc could verify, wrote a negative fixture with real teeth himself, closed the ADR drift. ❌ **submitted
     gate as "(all pass)" WITHOUT raw output — REPEATED 3 TIMES (APP.2c + Spear A×2)** despite O's reminders; didn't self-explain
     the clippy delta (round 1) → O had to dig it out. These two old habits PERSIST — reported to G (exactly the attitude G is measuring).
2. **ADR-0053 Heap Payload Outcome — DRAFTED BY O, READY FOR SIGNATURE, G ⏳ to sign.** File
   `docs/decisions/0053-heap-payload-outcome.md` (untracked, not committed). Unlocks `~- "error msg"`
   (currently is_scalar blocks it with E1037). **3 rulings G confirmed §8:** (1) 32-byte layout NOT Packed (YAGNI) ·
   (2) drop glue disc-dynamic INLINE in the MIR CFG, NOT a shim · (3) borrowck chain SPIKE PROBE (HP.0)
   before Production. **Correction of G's premise:** Triết's heap value = **24-byte {ptr,len,cap}** (not
   a 16-byte fat-pointer) → Outcome heap slot = **32-byte {disc@0,ptr@8,len@16,cap@24}**. **Core:** Drop
   moves from fully type-static to disc-dynamic (`SwitchInt(disc)→free_T/free_E/no-op`). **`Deinit(o)`'s precise semantics:**
   `stack_store(Zero(0))` into disc@0 → the glue becomes a no-op (Zero=no-op reused, an internal sentinel post-move,
   no conflict with E1025). Sliced into HP.0 spike→HP.1 layout+producer→HP.2 drop glue→HP.3 match+Deinit→
   HP.4 map heap. After G signs: O takes HP.0's spike (borrowck investigation, not Production) → D goes to HP.1.

## 🔥 HP.0 SPIKE BORROWCK — ALREADY FIRED (held by O, throwaway, 2026-06-11). HEAVYWEIGHT RESULT.
After 3 commits (HEAD `f881390` Spear A · `cb17ab7` ADR-0020 · `e24644a` ADR-0053), the tree is clean.
O temporarily removed the `is_scalar` guard (cleanly reverted afterward) so a heap Outcome map chain could
be traced from lower through borrowck (check mode).
**3 findings shaping HP.1-4:**
1. **MATCHED case is SOUND** (A/A'): heap Outcome producer+match lowers OK, borrowck is clean. Match binds the payload
   PER-ARM by type (success→value_type, error→error_type) → heterogeneous `Integer~String` drop is CORRECT (success Integer→Drop no-op; error String→free). **⟹ disc-dynamic drop glue (ADR-0053 §3.1) is ONLY needed
   for the UNMATCHED case** (Outcome leaving scope without a match), NOT for the matched case. Narrows the scope of HP.2/3.
2. **🔴 THE MONSTER (F1+F2):** the `~+>`/`~->` map desugar is UNSOUND for heap AND borrowck DOES NOT CATCH IT.
   MIR of the map arm (`~+> |v| v` String): `_3=move payload` → **`Drop(_3)` (scope-pop of v)** → **`_2.payload=
   move _3`** = use-after-Drop (free-then-move) → double-free/UAF. **borrowck says "OK (no borrow errors)" exit 0.**
   - F1 (lowerer): the map arm scope-pops the Drop of the captured variable THEN rewrap-moves it. Scalar: Drop is a no-op (harmless).
     Heap: UAF. ⟹ **removing the is_scalar guard naively (HP.4 naïvely) = a silent UAF.**
   - F2 (borrowck): NLL move-tracking M3+ does NOT model Drop as a kill → misses the move-after-Drop (which should be E2420).
   ⟹ **answer to G: borrowck CANNOT withstand this — must (a) fix the desugar to be heap-aware [don't Drop the captured value
     before rewrapping into body_val, or use Deinit] AND (b) tighten borrowck to catch use-after-Drop. BOTH before HP.4.**
3. **Passthrough is a MOVE not a COPY** (good news): bb3 `_2.payload = move _0.payload` — the MIR uses `move`,
   NOT alias/double-own. **O self-corrected:** the concern "copy→double-free" in ADR-0053 §3.2 was WRONG for passthrough
   (it's a move). The real bug is Drop-then-move in the map arm, not passthrough.
**Next step:** ADR-0053 needs a §HP.0 addendum (revising §3.1 to narrow to matched; §3.2 changing "drop placement"→"desugar
Drop-vs-rewrap race"; adding the requirement to tighten borrowck). ADR-0053 is ALREADY committed (e24644a) → the addendum = a new commit, pending G.
The bug is confirmed for `~+>`; `~->` is inferred by symmetry (not directly confirmed — the probe gave empty output).

### G ISSUED A RED ALERT (2026-06-11) — F2 becomes its own core front
- **§HP.0 addendum to ADR-0053: WRITTEN + COMMITTED** (`5ebdf5f`, pushed) — §9.1 matched-sound/glue-only-unmatched
  · §9.2 correcting passthrough=move · §9.3 the F1+F2 monster · §9.4 ordering decree (HP.4 HALTED).
- **F2 teeth independent of the fix (O built, throwaway, proof):** hand-built MIR `Body{Drop(s:String); assign(other,s)}`
  → `check_body().errors == []` (BLIND, should be E2420). Since removed, tree clean, borrowck 20 green.
- **Root cause grounded:** `VarState::Ended` (checker.rs:134-145, Drop set at 720-722) documents "any other
  use is E2420" BUT the use-sites only check `Moved`, IGNORING `Ended` → a documented contract that isn't enforced. Fix:
  enforce Ended-use→E2420, KEEPING the Return exception (the reason Ended is split from Moved).
- **ADR-0054 Core-Borrowck-Patch: SIGNED OFF BY G on 2026-06-11** (`docs/decisions/0054-borrowck-drop-kills-liveness.md`,
  LOCKED). Drop=kill liveness. **G confirmed §7: (1) NEW CODE E2421 UseAfterDrop** (NOT merged into E2420 — 2 mental models
  separated: move=active-action vs drop=lifecycle) · **(2) ONLY Move types** (Copy Drop=no-op, tightening = a false-positive mess).
  Root cause: `VarState::Ended` (checker.rs:134/720) documents "any use→E2420" but the use-sites only check `Moved`,
  ignoring `Ended`. Fix: enforce Ended-use→E2421, KEEPING the Return-leniency. Teeth T1(drop_then_move→E2421) · T2(Return
  doesn't break + the 20 old tests) · T2b(Copy doesn't get falsely rejected) · T3(regression after-F1). Requires a variant
  `BorrowError::UseAfterDrop` + `#[diagnostic(code(triet::borrow::E2421))]`.
- **ORDER G CONFIRMED:** ADR-0054 (patching the borrowck core) BEFORE → then ADR-0053 HP.1→HP.4. D may only remove the is_scalar
  guard AFTER F2 is patched. Author commit of ADR-0054 doc (`bb57cb5`, awaiting push).

### ✅ ADR-0054 CODE — SIGNED BY O 2026-06-11 (checker.rs, NOT YET COMMITTED, awaiting G's close)
D took it on (no blueprint). `BorrowError::UseAfterStorageEnd` + `code(triet::borrow::E2421)` + helper
`check_use_after_end` (gate `Ended && !is_copy`) at 7 use-sites + Return-lenient (`if !is_return`).
Borrowck 23 tests (20+3): T1 drop_then_move→E2421 · T2 return_after_drop→OK · T2b drop_then_use_copy→OK.
Gate **0·0·131·201**. **3 teeth verified RED by O on the final code** (2 rounds): A disable enforcement→T1 fails · C remove !is_copy→
T2b fails (copy gets flagged) · **B/T2 poison the Return-leniency (`!is_return`→`true`)→return_after_drop FAILS
"UseAfterStorageEnd s" + 3 corpus fixtures (35/78/100 String-return) go red** → the carve-out is load-bearing.
**D handled the deviation correctly on round 2:** the variant name `UseAfterStorageEnd` (≠ ADR's `UseAfterDrop`) — round 1 D
renamed it SILENTLY (O caught it), round 2 D amended the ADR §3 footnote + §7 explanation (Ended is set by both
Drop AND StorageDead → the name is more accurate) + added the T2 unit test (O demanded). **Process:** the Iron Protocol
worked — D submitted "(all pass)"/a summary twice this session → O REJECTED without reading twice → D pasted the raw
whole block on the third try → the tree was opened. One small remaining blemish (D fixes in the same commit): ADR-0054's
line-1 title still says "use-after-Drop → E2420" (old, should say →E2421).
**Iron Protocol ARMED throughout:** a report missing raw output → "REJECTED. Paste the Raw Gate or get lost." + close it, unread.

## 🔴 HP.1 (Heap Layout & Producer) — O BLOCKED SIGN-OFF round 1 (2026-06-11). Blind teeth.
ADR-0054 already committed (d58a9a3 code + 8399f12 doc — ⚠️ doc commit msg still says "UseAfterDrop", was the file content
amended? verify later). HP.1 submitted by D: dynamic layout (`outcome_slot_size()` 16 scalar/32 heap), projections
`OutcomePayloadLen/Cap`, lower decomposes `{ptr@8,len@16,cap@24}`, JIT guard `build_body:614`
has_heap_payload→Err "heap deferred to HP.2". Fixture 137 check-mode ERROR. Gate 0·0·132·202.
**BLOCKED — 2 findings:**
1. **🔴 BLIND TEETH (a direct violation of G's order "mis-set an offset and the test must blow up"):** layout/offset/slot_size have NO
   test observing them. O poisoned `outcome_slot_size 32→16` → THE WHOLE WORKSPACE STAYED GREEN. Cause: the JIT guard
   blocks heap before the offset · `OutcomePayloadLen/Cap` have no JIT lowering yet (grep empty) · NO pure-function
   unit test · fixture 137 stops at the guard (only proves the GUARD fires, does NOT prove the offset is correct;
   the comment "stores {ptr@8,len@16,cap@24}" gives the false impression it's already tested). **Tooth-B lesson from APP.2c REPEATED.**
   Fix: a unit test for `outcome_slot_size()` (String~Integer→32, Integer~Integer→16) + prove the MIR producer generates the correct
   3 projections — pure-function, check-mode is enough, no JIT execution needed. O will re-poison to verify.
2. **🟡 clippy +1 + a WRONG "baseline" claim (pattern #10 recurring):** stash-diff HEAD 201→HP.1 202, +1
   `collapsible_if`. D wrote "202 baseline" — wrong. Fix: collapse the if + correct the claim.
**Process:** D submitted "(0 failures across all 20 crates)" → O REJECTED on the 3rd attempt → D pasted raw → the tree was opened. Code
is headed the right direction (dynamic layout/defer/decompose matches ADR §3.3) but is NOT teeth-guaranteed yet. The tree = the snapshot D submitted.

### ✅ HP.1 — SIGNED BY O round 2 (2026-06-11, NOT YET COMMITTED). Teeth now have bite.
D closed both findings: (1) +5 observation tests — 3 mir unit tests (`outcome_slot_size_scalar_and_heap` 6 asserts ·
`is_any_heap_detection` · `has_heap_payload_detection`) + 2 lower tests (`heap_outcome_producer_emits_len_cap_
projections` asserting the MIR has OutcomePayloadLen/Cap · `scalar_outcome_producer_no_heap_projections` no-regress);
(2) collapse clippy's if-let&& → back to 201 (confirmed via stash-diff). Gate **0·0·132·201**. **2 poisons O verified RED on the
final code:** A `outcome_slot_size 32→16`→`outcome_slot_size_scalar_and_heap` FAIL · B `is_any_heap`→false
in lower→`heap_outcome_producer_emits_len_cap_projections` FAIL (scalar test still ok, no over-firing).
Round-1 blind teeth CLOSED. **HP.1 COMMITTED + PUSHED `5505ffb`** (5 files: mir/lower/jit/borrowck +
fixture 137). ADR-0054 title-fix: Approach A rebase-reword + force-push (8399f12→c7d2b7b doc,
d58a9a3→826acb8 code) — commit msg + file line-1 changed to "E2421 UseAfterStorageEnd," history clean,
synced with origin. The 32-byte foundation is poured.

## 🔴 HP.2 disc-dynamic drop glue — O BLOCKED SIGN-OFF round 1 (2026-06-11). Half-blind teeth.
D submitted: JIT `Statement::Drop` for a heap Outcome → inline SwitchInt brif-cascade (free_pos/free_neg/noop, NO
shim — as G agreed to), `emit_outcome_payload_free` frees at the correct offset ptr@8/cap@24. Un-defer the build_body guard.
Fixtures RUN 137(~+"hello"→free-as-T) + 138(~-"fail"→free-as-E), EXPECT exit 0. Gate 0·0·133·201.
**BLOCKED — fixture EXPECT exit-0 is HALF-BLIND (tooth-B lesson recurring):** O poison-proved (measuring exit CORRECTLY,
without a pipe): double-free(free 2×)→exit **134 SIGABRT** CAUGHT ✓ · **wrong-arm swap(free-T↔free-E)→exit 0
BLIND** (freeing Integer-as-scalar=no-op→String LEAKS with no crash) · leak(skip the free)→exit 0 BLIND. Only
double-free is caught, wrong-arm/leak are NOT caught (G stressed "0=leak/2=double-free" → only 1 of 2 directions caught).
**Unlocked:** the `__test_counting_free`+`FREE_COUNT`+the `alloc_free_balance_string_return` pattern infra was ALREADY
IN PLACE in the same file (mir_lower.rs:3619-3693), D IGNORED IT. D adds a JIT unit test using the counting-shim → `assert FREE_COUNT
==1` on the Pos+Neg arm → catches leak/wrong-arm/double-free. The tool was sitting right there 70 lines below where D was coding.
**O self-critique (transparently):** on the first pass, measured exit THROUGH A PIPE → `$?`=the tail's exit code, nearly falsely reported "double-free exit 0";
re-measured without-pipe and got 134 correctly. Verify-don't-trust applies to O too. The tree = the snapshot D submitted.
**Drop glue is structurally correct** (inline SwitchInt/no shim/correct offset/double-free SIGABRT) — only missing
teeth that observe the leak.

### ✅ HP.2 — SIGNED BY O round 2 (2026-06-11, NOT YET COMMITTED). 3-way teeth with bite.
D closed the finding: +`HP2_FREE_COUNT` static + `__hp2_count_free` (counting-only, no real dealloc → poisoning
double-free just increments the counter safely, no SIGABRT) + test `hp2_outcome_drop_glue_frees_exactly_once` (hand-built
Outcome<String,Integer> disc=1, OutcomeAlloc+projection+Drop, shim-injects `__triet_string_free`→counter,
asserts HP2_FREE_COUNT==1). Routes through the REAL drop glue. D also self-fixed clippy +2→201 (backticks+unused import)
after O warned "line shifts only" was wrong. Gate **0·0·133·201** (clippy stash-diff confirms 201=HEAD).
**3 poisons verified RED by O on the final code (G stressed 0=leak/1=correct/2=double-free):** leak(remove emit_free)→count 0 FAIL ·
double-free(emit_free 2×)→count 2 FAIL · wrong-arm(value↔error swap)→count 0 FAIL. jit 35 tests, corpus green.
**HP.2 commit awaiting G's close** (jit + fixture 137 modified + 138 new): proposed message
`feat(track-c): HP.2 — heap Outcome drop glue disc-dynamic (inline SwitchInt, no shim)`.
**Process:** D submitted a summary "(all 20 crate suites 0 failed)" → O REJECTED for the 4th time → D pasted raw → the tree was opened. The
Iron Protocol still has teeth.

## 🔴 HP.3 match consumer + Deinit — O BLOCKED SIGN-OFF round 1 (2026-06-11). Teeth protect the mechanism, not the real code.
D submitted: lower a heap match arm → decompose {ptr,len,cap}→bind_local + `Deinit(scrut)` (lib.rs:2884-2885
`if did_bind && needs_deinit`); JIT Deinit→stack_store(0,slot,0) disc=Zero tombstone. Fixture 139 RUN
match bind→5. 2 unit tests HP3A(deinit→drop→0free) + HP3B(no-deinit→2free) with per-test counter. Gate 0·0·134·201.
**BLOCKED — teeth protecting the MECHANISM but not the REAL CODE (a subtler layer of the tooth-B lesson, exactly what G was stressing):**
O poisoned the LOWER `2884`→`if false` (stripping Deinit from the real Match code-path) → **0 TESTS WENT RED**: HP3A/HP3B are both
`MirBuilder` HAND-BUILT (not routed through lower) and stay green · fixture 139 exit 0 is half-blind (no-Deinit doesn't crash)
· no lower-assertion test exists. ⟹ deleting line 2884 = the double-free comes back SILENTLY. **Unlocked:** D adds a test
that routes through lower (like HP.1's `heap_outcome_producer_emits_len_cap_projections`): `lower_source("match heap
outcome")` → assert the MIR block has `Statement::Deinit(scrut)` → poisoning 2884 must go red. The JIT mechanism (HP3A/HP3B)
is CORRECT+valuable but NOT ENOUGH. **Process:** D pasted a full raw block (no REJECT this round). The tree = the snapshot D submitted.

### ✅ HP.3 — SIGNED BY O round 2 (2026-06-11, NOT YET COMMITTED). Teeth with 3 layers of bite.
D closed the finding: +`match_heap_bind_emits_deinit` (route-through-lower `lower_source("match heap outcome")` → asserts
the MIR has `Statement::Deinit`). O re-poisoned 3 layers on the final code: **(1) lower code-path** poisoning `2884`
`did_bind&&needs_deinit`→`false` → match_heap_bind_emits_deinit FAILS "MUST emit Deinit(scrut)" · **(2) JIT
Deinit tombstone** poisoning `957` stack_store offset 0→8 (zeroing ptr instead of disc) → hp3_deinit_then_drop FAILS
"must free 0 times" · **(3) JIT no-deinit** hp3_no_deinit_double_frees→count 2 (from the earlier round). Gate
0·0·134·201, jit 37 + lower 12 green. **HP.3 commit awaiting G's close** (jit+lower + fixture 139): proposed message
`feat(track-c): HP.3 — match consumer heap bind + Deinit(o) (ownership transfer, no double-free)`.
**⚠️ A lesson for O this session:** a `cp` snapshot got cut off by a `/login` interrupt → /tmp/hp3b_lower.bak did NOT
exist, lower got stuck POISONED. Manual recovery by reversing the poison (Edit `if false`→`if did_bind&&needs_deinit`, NOT
git checkout — to preserve D's new test) → re-verified the test passed → took a new snapshot. **Rule: take snapshots in a SEPARATE
block + verify existence BEFORE poisoning.**
**Next: HP.4 (map heap — the final push, needs the F1 heap-aware desugar [HP.0 §9.3 Drop-vs-rewrap] + the ADR-0054
E2421 net). Finishing HP.4 = the Heap Outcome monster dies.**

## ⏸️ SESSION CLOSING POINT 2026-06-11 (Mentor-O session)
**Git:** HEAD `9100e8c` (HP.3), tree CLEAN, synced with origin/main. The heap Outcome chain is committed+pushed:
`5505ffb` HP.1 layout 32-byte · `ed03725` HP.2 drop glue disc-dynamic · `9100e8c` HP.3 match+Deinit.
ADRs locked+committed: ADR-0053 (heap payload, +§9 HP.0 spike) · ADR-0054 (Core-Borrowck-Patch E2421
UseAfterStorageEnd). Final gate **0·0·134·201**.

**Outcome campaign progress:** OP.1-4c (binary scalar) ✅ · APP.1/2a/2b-1/2c ergonomics ✅ · Spear A
(Ternary T?~E scalar) ✅ · ADR-0054 borrowck foundation patch ✅ · **HP.1/2/3 heap Outcome ✅** · **HP.4 (map
heap) = WORK ORDER ISSUED, D HAS NOT YET SUBMITTED** (the final piece of bone).

**HP.4 work order (already issued to D, the final push):** remove the is_scalar guard for heap at the `~+>`/`~->` arm-handler (exprs
505/571/609); **fix F1's Drop-vs-rewrap race** (HP.0 §9.3 — a map bind of heap MUST NOT Drop(v) at scope-pop and then move
v→result.payload = UAF); Deinit the inner after passthrough/map move (ADR-0053 §3.2/§4.3). The map desugar is at
lower `Expr::OutcomeArmHandler` ~3286+. O's bar: F1-does-not-recur (either E2421 catches it OR counting catches the double-free) ·
correct-exactly-1-free through the map chain · a route-through-lower test (the HP.3 lesson) · no scalar regression. G promised beer when it's done.

### ✅ HP.4 — SIGNED BY O (2026-06-11, NOT YET COMMITTED, awaiting G's close). Map-heap SOUND.
**Submitted tree:** 4M (jit mir_lower · lower lib · typecheck exprs+types) + 2D→rename (124/129) +
4?? (124/129 repurposed as struct-body-E1037 · 140/141 RUN). Gate self-measured by O **0·0·136·202**
(baseline clippy = **202**, not 201 as recorded in the §CLOSING POINT — 201 was an incremental artifact).
**Code:** guard removed at exprs 507(`~->`)/573(`~+>`) adding `&& !is_heap()` (String|Vector); 611 `~0>`
stays sealed for scalar only. New `Type::is_heap()` (types.rs). F1 fix in lower: `pop_scope` moved DOWN to AFTER
the result-write + Deinit (positive 3720 + negative); 3 helpers for heap {ptr,len,cap} decompose/recompose/copy
+ `Deinit` after every move → the drop's scope-pop becomes a no-op. `is_any_heap` (MIR, includes HashMap) ≠ `is_heap`
(typecheck, String|Vector) — an asymmetry that is SAFE (typecheck gates first, HashMap map-body→E1037).

**Teeth O verified by hand (cp /tmp restore, NOT git checkout) — 8 spears:**
1. 140 RUN heap-success `~+>`+match → **5**, exit 0, NO SIGABRT (`$status` directly).
2. 141 RUN heap-error `~->`+drop → **0**, exit 0, NO SIGABRT.
3. Poison F1 (pop before write, lib.rs positive) → `map_heap_success_no_drop_then_move` goes RED
   "local _8 moved after Drop". Restore cp IDENTICAL.
4. Poison removing `Deinit(inner)` (jit hand-built) → `hp4_heap_map_frees_exactly_once` count **2** (≠1).
   Restore cp IDENTICAL. → the counting test is NOT a tautology.
5. Real lowered MIR (dump of 140/141): every `move _3.x` BEFORE `Deinit(_3)`→`Drop(_3)` is a no-op. The F1 fix is visible.
6. **Probing an HP.3 defect** (heap-error MATCH `Integer~String` ~- arm): `JIT unsupported: type 'Integer'
   is not a known struct (local _4)`. A REAL defect. → refuses cleanly, NO SIGABRT/wrong-code.

**Triad locking down "real lowered map-heap frees exactly once":** structural route-lower (shape) + JIT counting
(count==1 on that shape) + 140/141 RUN end-to-end (no crash) + O inspecting the real MIR.

**🔴 NEW DEBT → HP.5 (match-bind error-arm type fix):** `lower_outcome_arm` lib.rs:2895-2901 hardcodes
`payload_ty_local = value_type` for BOTH arms; the neg-arm heap-error needs `error_type`. Latent, pre-existing
inside HP.3 which is already committed `9100e8c` — **an HP.3 tooth-hole that O ALREADY SIGNED OFF ON, O owns responsibility for it.** Lucky it refuses cleanly (JIT
unsupported) rather than being a soundness hole. D descoped 141 into the correct drop-style instead (Rule 4, transparently). HP.5 = fix the bind
to use error_type for the neg arm (~1 line) + a heap-error-MATCH fixture + counting teeth. Heap Outcome is NOT YET fully dead
until HP.5 is done.

**Process note:** D's JIT counting test was hand-built, the work order asked for route-lower. O accepted it as
SUPPLEMENTARY (route-lower coverage is carried by structural + 140/141, which O already verified) but D deviated from the order without FLAGGING it —
next time this must be noted in the report. (A new D pattern? — not yet enough to call it a pattern, noting it for tracking.)

**HP.4 commit awaiting G's close:** 4M+2D+4?? → proposed message
`feat(track-c): HP.4 — map-heap binary Outcome (String/Vector), F1 Drop-vs-rewrap fix`.

### ✅ G SIGNED OFF ON HP.4 + BROKE THE SEAL ON HP.5 (2026-06-11)
G signed off approving HP.4, accepted the commit message above, ordered the author to commit immediately.
Praised O for taking the HP.3 defect's blade himself (Chief Architect's mettle), praised D for restraint under Rule 4 + transparent descoping.
**New rule for D — RULE 5:** any technical test deviation from the work order must be flagged boldly "I REQUEST
PERMISSION TO DEVIATE FROM THE ORDER…" (see [[colleague_d_persona]]). **The beer is still in the fridge until HP.5 is done.**

### 🔨 WORK ORDER HP.5 — match-bind error-arm type fix (O sets the bar, D codes, G breaks the seal)
**Root cause:** lib.rs:2895-2901 closure `lower_outcome_arm` hardcodes `payload_ty_local=value_type`
for both arms; the neg-arm heap-error needs `error_type`. `needs_deinit` (2987/3013) is ALREADY correct per-arm —
only the TYPE is wrong. **The work:** ① fix neg-arm bind `payload_ty_local=error_type` (the mechanism for passing the type is
implementer's choice; do NOT touch decompose/needs_deinit). ② fixture 142 heap-error-MATCH (the finished version of 141: `Integer~String` match `~- e` bind+USE String → RUN produces a value, no
SIGABRT/JIT-refuse). ③ counting teeth on the error branch (free exactly 1).
**O's bar for teeth:** Poison-1 revert the type fix → 142 exposes `type 'Integer' is not a known struct`.
Poison-2 remove neg-arm's Deinit → count 2. **Borrowck stays silent** (G: the neg-arm swallowing a Fat-Pointer must not
falsely cry E2421/E2420, check-mode). No-regress 140→5 + scalar. Counting prefers route-lower
(`lower_source`, the lesson from pattern #12); hand-build → must be flagged boldly under Rule 5. Raw 4-item gate, clippy 202.
**Finishing HP.5 = Heap Outcome fully dead, G opens the beer.**

### ✅ HP.5 — SIGNED BY O (2026-06-11, NOT YET COMMITTED, awaiting G's close). Heap Outcome FULLY CLOSED.
**Submitted tree:** 1M (lower/lib.rs +21/−17) + 2?? (fixture 142 · `tests/hp5_heap_error_match_counting.rs`).
Gate self-measured by O **0·0·137·202** (fixtures 136→137 +142; clippy 202 no delta). **A fix surgically matching
the work order:** the closure `lower_outcome_arm` gets a new parameter `payload_ty: MirType` (paralleling `needs_deinit`
which was already per-arm); the pos call-site passes `value_type`, the neg passes `error_type`; `bind_local=alloc_local_ty(
payload_ty)`. Did NOT touch decompose/needs_deinit (already correct since HP.3). The heap-error case now binds correctly to the String-struct
→ the JIT no longer refuses.

**Teeth O verified by hand (cp /tmp restore, NOT git checkout) — BOTH directions (patching the HP.3 blind spot):**
1. 142 RUN heap-error MATCH → **7**, exit 0, no SIGABRT/refuse.
2. Probed the old HP.4 defect (the case that used to refuse "type Integer not known struct") → **7**. Defect DEAD.
3. Route-lower counting test (the REAL pipeline parse→typecheck→lower→jit, shim swaps `__triet_string_free`,
   `let o` owned Drop-load-bearing) → result 7 + count **1**. NOT hand-built (fixing pattern #12).
4. Poison-1 revert neg-arm→value_type → compile fails with `Unsupported("type 'Integer' is not a known struct
   (local _4)")`. Restore cp identical.
5. Poison-2 remove `Deinit(scrut)` lib:2960 → count **2** double-free. Deinit is load-bearing. Restore identical.
6. Borrowck stays silent (per G's order): 142 check-mode "OK (no borrow errors)" — the neg-arm swallowing a Fat-Pointer doesn't cry false alarms.
7. No-regress: 140 heap-success match → 5.

**🔴 NEW DEBT (outside HP.5, D flagged it transparently + O's probe confirmed it):** **block-tail match value-discard** —
`function f()->Int { match x {…} }` (match as the direct function body, no `return`) returns **0** instead of the arm's
value; `let r=match…; return r` is correct. Scalar Outcome is affected too — the block-tail lowering is a SHARED issue, not heap-specific. Pre-existing. D descoped it correctly (Rule 4). → a separate slice later.

**HP.5 commit awaiting G's close:** proposed `fix(track-c): HP.5 — match-bind error-arm uses error_type
(heap-error MATCH, no JIT-refuse)`. **HEAP OUTCOME FULLY CLOSED** once G signs: producer+consumer, map+match,
success+error, free-exactly-1, borrowck-silent. G opens the beer.

### 🏁 G CLOSED HP.5 + PUSHED — HEAP OUTCOME COMPLETELY DEAD (2026-06-11, session end)
G signed off closing HP.5. The author committed both, O did verify-don't-trust (log+stats matched the review, tree clean), pushed.
- **HP.4 = `8013774`** (10 files: jit mir_lower +210 · lower +292 · typecheck exprs/types · fixtures 124/129
  renamed + 140/141 new).
- **HP.5 = `7285d88`** (3 files: lower +38/−17 · fixture 142 · `tests/hp5_heap_error_match_counting.rs`).
- `git push origin main`: `9100e8c..7285d88`, pre-push Gate-B clean. **origin/main synced at 7285d88.**
**The Fat-Pointer Outcome system is crushed:** StackSlot 32-byte + disc-dynamic Drop · ADR-0054 E2421
UseAfterStorageEnd patching the borrowck foundation · binary T~E String/Vector producer+consumer+map+match
success+error free-exactly-1 borrowck-silent. G opens the beer, orders the machine shut down for a rest.
**Next front (G confirmed the roadmap):** Spear C — Borrow Params Heap `&+ T` · the debt of block-tail match value-discard
(its own campaign "CFG Tail-Expression Refactor"). The beer is out of the fridge.

**Debt under seal:** B3 alias · C4 Packed Outcome (24-byte) · C5 generic tuple-return · nested struct/enum
payload Outcome · TernaryOutcome HEAP + `~0>` heap (after HP.4 binary heap) · Flatten nested (APP.2b-2 YAGNI).

## Debt under seal (updated)
B3 alias · C4 Packed Outcome (24-byte, defer until the 32-byte version runs) · C5 generic tuple-return ·
nested struct/enum payload Outcome (no drop glue yet) · TernaryOutcome HEAP (after binary heap + Spear A) ·
Flatten nested (APP.2b-2 deferred indefinitely).

## Debt under seal (updated, end of session)
B3 alias · Native Layout + Packed Outcome (Group E) · C5 generic tuple-return · heap payload Outcome
(Tier B/C) · TernaryOutcome producer + `~0>` · **Flatten nested Outcome (APP.2b-2) — deferred indefinitely**.

## Debt under seal (DO NOT TOUCH)
B3 alias · Native Layout + Packed Outcome (Group E) · C5 generic tuple-return ·
heap payload Outcome (Tier B/C) · TernaryOutcome producer + `~0>` (awaiting its own OP/APP round).

[[handoff_2026_06_10_op1_dong]] — OP.1 typecheck (previous milestone)
[[mentor_o_persona]] · [[feedback_poison_must_be_red]] · [[feedback_g_report_protocol]]
