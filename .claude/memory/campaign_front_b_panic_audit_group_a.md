---
name: campaign_front_b_panic_audit_group_a
description: "Front-B WO-1 — the mir_lower.rs panic audit, Group A (internal invariants) + the JitError::Internal taxonomy; Group B got a tombstone, Group C was deferred"
metadata: 
  node_type: memory
  type: project
  originSessionId: 1f77ea83-6290-499f-8cde-98ba80a1d51b
  modified: 2026-07-24T16:40:11.886Z
---

# FRONT-B WO-1 — THE `mir_lower.rs` PANIC AUDIT, GROUP A (closed 2026-07-24(c))

origin/main **`e2db04d`** (synced after a timeout and a retry). Gate `0·clean·0·452·0`.
7 commits pushed (`6ae6cc3` the fmt gate + 6 for the WO). D = a Sonnet 5 subagent (Giang ordered the spawn),
O gatekept with blood verification, G signed twice (approving the WO and closing the slice).

## Context
The opening of Front-B (pinned by G during the Front-A session). The infrastructure task at the start:
**slam `cargo fmt --all --check` into `scripts/gate.sh`** — the Front-A safety-net hole (unformatted code
reached O's first review, caught only by the pre-commit hook). The gate now mirrors the hook **verbatim**
(the same command, the same stable channel, `2>/dev/null` silencing the nightly-only options
`imports_granularity`/`group_imports` that stable skips, in BOTH the hook AND the gate → the two nets cannot
drift). Commit `6ae6cc3`.

## Recon TRIAGE (O read file:line personally, NOTHING guessed)
`crates/triet-jit/src/mir_lower.rs`, 10062 lines, `JitError{Unsupported,Module}`.
- **The `#[cfg(test)]` boundary is line 6782** (a snapshot: it was 6753 during recon and drifted after edits).
  **42 of the 51 sites are ≥ the boundary = test infrastructure → UNTOUCHED.** Only **9 production sites**.
- **A trap nearly sprung (the memory warning about the 7→8 table habit):** the Front-A memory said "51 sites",
  counting the test ones; O nearly mapped 12 production sites because 6877/6912/6942 sit RIGHT after the test
  boundary. **Reading gave 9; guessing would not have.** This is exactly the "mapping from imagination" that
  the poison/verify discipline saves us from.
- **3 groups:**
  - **A. Internal invariants (4):** 1066 (`Nullable(_)` after stripping — a `T??` is impossible),
    1516 (`compile()` with an empty map), 2883 (String already caught by the if-let above), 5031 (an inner
    re-match `_`).
  - **B. Environment preconditions (2):** 1466/1469, host ISA detection — platform-level, not user input.
  - **C. Runtime-shim OOM (3):** 5232/5535/6075 `Layout::from_size_align().unwrap()` —
    `string/vector/hashmap_layout`, running at EXECUTION time, exploding only when `total>isize::MAX`.

## G's ruling (an ADR-lite, settled BEFORE the WO)
1. **Group A:** add `JitError::Internal(String)` mirroring the **E1190 philosophy** (ADR-0086's "please
   report"), and do NOT spawn a `triet::jit::EXXXX` code namespace (scope creep; that needs its own ADR).
   1066/1516 → `Err(Internal)`. **2883/5031 are RESTRUCTURED to prove exhaustiveness at the type level**
   instead of leaving filler. → [[campaign_front_a_lower_error_codes]]
2. **Group B:** KEEP `expect` + the comment `// RATIONALE: fatal environment error, abort intended`.
3. **Group C:** DEFER and record the debt in TODO.md (`D-JIT-OOM`, to be upgraded to a null return during the
   sandboxing phase).

## D's execution (no deadlocks, no fallbacks, no scope creep)
- **3c:** `if-let String / else{match}` → **one `match value{String|Integer|Trit|Unit}`** with 4 arms; the 3
  scalar arms share a new helper `store_scalar_const` (mirroring the ~0/NULL_SENTINEL niche of
  ADR-0062/0065).
- **3d:** the inner `match op{... _=>unreachable!()}` was deleted outright → 6 explicit arms
  `BinOp::Eq/Ne/Lt/Le/Gt/Ge` calling a nested fn `cmp` (icmp + select → Trilean! +1/-1).
- 6 commits: `1d6bc14`(the enum) `db716c8`(3a/3b) `9afef87`(3c) `d48ed18`(3d) `3867408`(4) `e2db04d`(5).

## O's blood teeth (independent, verify-don't-trust)
- Ran the gate itself and got CLEAN (never trusting D's raw output). The acceptance grep: below 6782 only
  Group B (expect + RATIONALE) and Group C (unwrap) remain, with 0 `unreachable!`, and the test
  infrastructure untouched.
- **Tooth 3c was REAL (textbook):** re-planting `_ => unreachable!("TOOTH PROBE")` into the `match value`
  made the compiler emit **`warning: unreachable pattern … collectively making this unreachable`** ⟹ the 4
  arms cover ConstValue exhaustively AT THE TYPE LEVEL. Restored with `cp` (md5 `25132ec…`), NEVER git
  checkout. → [[feedback_teeth_never_git_checkout]] [[feedback_poison_must_be_red]]
- **3a/3b:** provably unreachable by construction → **NO fabricated mock poison test** (G's praise: defence
  in depth is submerged armour; do not haul it up for a circus act). → [[feedback_failure_mode_precision]]

## Lessons / blemishes
- **O accepted a PRESENTATION-ORDER blemish:** putting the triage map first buried the gate report; G scolded
  "you skipped the procedure" EVEN THOUGH the gate had been run and committed correctly. New law: **report
  the gate FIRST**, and an infrastructure task is always Task 1 in the WO (even when already done: mark it
  ✅ + the hash; do NOT make D redo it = honesty).
- **The push timed out with 143 the first time (2 minutes):** the pre-push hook runs clippy + tests, over 2
  minutes. Retrying with `timeout 300` worked. ls-remote confirmed before and after (never trust a lone push
  exit code). The remote went `77fdbe3→e2db04d`.
- Tiering: mir_lower.rs is normally Opus-only (ABI/IR), but a WO for a local refactor with a settled contract
  plus O's blood verification → Sonnet sufficed; the 3c/3d risk was contained by "if you get stuck, STOP and
  report to O". D did it cleanly with 0 stops.

## Outstanding debts (G's black book, awaiting an opening)
push_owned-vs-M3 isolation (defence in depth) · **D-JIT-OOM** (new, Group C) · the container
Nullable(Struct-heap) refusal §15.6 · N1 widening · the method-call Struct?/Enum? return over-refusal ·
deep Clone · drain · `&0 Enum` consumption · `&+ T` borrow params · the Front-B panic audit still has
Groups B/C (handled so far: A closed, B tombstoned, C deferred).
