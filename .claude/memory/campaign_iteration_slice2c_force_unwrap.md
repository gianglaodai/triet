---
name: campaign_iteration_slice2c_force_unwrap
description: "🏁 CLOSED — ADR-0089 Slice 2c: `!!` ForceUnwrap lowering isomorphic to Elvis (trap-on-null SIGILL + a PA-3c identity move-out for heap scalars + an aggregate fence, E1100). Session 2026-07-27, fortress #5. SHA aa500ab (code) + c77d674 (ADR)."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-26T18:04:53.752Z
  originSessionId: 1687df81-24d3-40a4-91cb-f1662466d6f7
---

# Session 2026-07-27 — 🏁 Fortress #5: ADR-0089 Slice 2c (`!!` ForceUnwrap)

origin/main **`c77d674`** (synced), gate **`0·clean·0·496·0`** (+8 fixtures: 495/497/499-504).
Code `aa500ab` (D), ADR-0041 §AMEND-Slice2c `c77d674` (O). Giang set the direction (choosing Slice 2c
out of 5 candidates), G approved Scope A (Mirror-Elvis) + 4 steel conditions, O did the recon, WO, and
blood verification, D implemented.

## 🏁 What closed
`expr!!` (force-unwrapping a `T?` — a historical primitive from ADR-0020, operator family ADR-0039) now
LOWERS (the lexer/parser/typecheck front half was already wired; the lowerer was EMPTY → E1100 = the
classic gap shape).
**Absolutely isomorphic to Elvis `?:`** (`triet-lower/lib.rs:4415`), differing only in the null arm:
- **Trap on null:** the operand `== NULL_SENTINEL` → `Terminator::Trap` (SIGILL), with **NO merge** from the null side.
- **Present = a PA-3c identity:** `result = obj_val` (an Assign). The source is a named local, a non-Copy
  heap scalar (String?/Vector?/HashMap?) ⇒ borrowck marks it Moved (`checker.rs:975`) ⇒ reading it again
  → **E2420** (the move-out signature, killing the alias double free). A Copy scalar
  (Integer?/Trit?/Trilean?) is non-consuming.
- **The fence:** a payload matching `matches!(MirType::Struct(_)|Enum(_))` → **E1100** (ownership
  projection deferred).
- **1 touch point:** the `Expr::ForceUnwrap` arm at `lib.rs:4508`. Nothing in jit/mir/typecheck/schema/borrowck was touched.

## 🔑 Recon-first caught the gap and verified G's claims (verify-don't-trust cuts BOTH ways)
- The `!!` front half was already wired (the lexer's BangBang at token.rs:323 · the parser at expr.rs:786 ·
  typecheck's check_force_unwrap at exprs.rs:1795, whose test uses `String?` → a commitment to heap
  support, so retreating to scalars only was not allowed). The lowerer was missing its arm.
- **Refuted G's claim of E2403** → measured the real code, **E2420** (`checker.rs:296` UseAfterMove). G accepted and confirmed.
- **Refuted G's claim that "a String sometimes carries `Struct("String")`"** → measured:
  MirType::String/Vector/HashMap are **their OWN variants** (mir:490/530/532), not Structs. The fence
  `matches!(Struct|Enum)` therefore excludes String by construction; the extra `!is_string_repr()` belt
  G suggested is a harmless redundant belt. D chose to drop it (simplicity). Correct.
- Verified the final link: `lower_expr(Expr::Identifier x)` returns the local x DIRECTLY (`lib.rs:3306`
  `return Ok(local)`) → the present Assign has source=x → a real move. The canary is feasible WITHOUT
  adding anything to borrowck (the scope does not inflate).

## 🩸 Two poison spears (O independently, ordered by name from G) — DECISIVELY RED
`cp` snapshot to /tmp → poison → rebuild → measure → restore with a matching md5 (`78a3478`) + an EMPTY
`git diff` (NO git checkout).
1. **Removing `Terminator::Trap`→Goto present:** the 2 trap tests in `force_unwrap_null_trap.rs` FAILED
   (`expected signal 4, got None, success=true`). The trap teeth bite.
2. **Removing the fence (`if false &&`):** the corpus reported `FAIL 501/502: pipeline succeeded with 3/7`
   (E1100 lost); as a BONUS D had already measured: a heap-bearing struct local → **a double free, exit 134**.
   The fence really is load-bearing.

## 🥅 A harness trap that nearly sprang (a lesson at the measurement layer)
The FIRST poison-2 run: `cargo test -p triet-driver 2>&1 | grep ...` then `cat tail -25` → showed ALL "ok",
NO FAILED → nearly concluded "the poison did not go red = the fixtures are vacuous". WRONG: the corpus's
FAILED line was OUTSIDE the tail-25 window (many other test binaries in between). Running the target alone
with `--test integration_tests integration_test_corpus` → FAIL 501/502 appeared immediately. Lesson:
**grep-then-tail truncates the evidence — run the exact test target in isolation and read ITS full output**;
never trust "silence = green" when the harness bundles several binaries. (A relative of ritual #15.)

## ⚙️ The full 5-phase procedure + infrastructure discipline
Giang decided (an AskUserQuestion over 5 candidates) → G approved Scope A + 4 steel conditions + required an
ADR amendment → O reconned file:line and verified G's claims → wrote the WO (with the handcuffs: one
foreground gate command, a 600s timeout, no background/Monitor, a raw 5-line block) → G approved the WO →
O spawned D (Sonnet 5, a background agent) → D submitted `aa500ab` + a raw gate + poisoned two spots itself
→ O verified independently with blood (rebuild-first per law #12, 2 poison spears, canary/trap/fence through
the driver, a MIR dump) → O signed → G signed (verifying independently) → O committed the ADR and pushed.
**D refuted O 0 times** (the code was sound immediately) and obeyed the infrastructure handcuffs (a raw
block, no summary). D subprocess-isolated the 496/498 trap teeth per convention (a SIGILL inside the corpus
kills the harness — the Slice 2b lesson).

## Remaining debts (7 candidates, blockaded by G — awaiting Giang and O to open one)
🔴 ADR-0088 double nullables T?? (a heavy cliff, ADR first) · HashMap.drain() · deep Clone · §15.6
Vector<Leaf?> · N1 widening (awaiting ADR-0065) · `&mutable Vector` drain · the O(N) cursor drain (perf).
⚰️ ADR-0068 Box/recursive is BARRED.

→ [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice1_2a]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]
