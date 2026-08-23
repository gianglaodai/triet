# ADR-0051: Unifying Borrowck into a Single NLL MIR Tier (Retiring AST-Based Borrowck)

## 1. Status
**Approved (O + G, 2026-06-09)** — Crusade #2. B2.0 spike CLOSED (O self-verified).
Proceeding to B2.1 implementation.

**G Rulings (invariants — must not be violated during implementation):**
1. **NLL MIR borrowck exclusively owns 100% of lifecycle control** (lifetimes, exclusivity,
   use-after-move, drop, aliasing). Typecheck is restricted solely to type checking + name resolution.
2. **E25XX (actor/Send/Sync/Scope, ADR-0026) REMAINS in typecheck** — concurrency semantics,
   NOT NLL dataflow. Excluded from B2.
3. **FORBIDDEN to blindly delete.** 99 green fixtures only prove existing cases
   are covered by MIR — NOT that all typecheck guard checkpoints are redundant. Every checkpoint
   must pass the verification protocol in §5 prior to deletion.

## 2. Context & Motivation

### 2.1. Two cops overlapping jurisdiction on the same road
| | Cop #1 (Typecheck) | Cop #2 (MIR borrowck) |
|---|---|---|
| Mechanism | AST walk + live-range (syntactic) | NLL dataflow on CFG |
| Location | `typecheck/borrow_check.rs` (502 lines) + move-state machine in `check.rs` | `borrowck/checker.rs` |
| Execution | Phase 2 — **FATAL, blocks before MIR** | Phase 4 — only runs if typecheck is clean |
| Emitted Codes | E2400/E2402/E2403/E2410/E2411/**E2420**/E2421/E2422/E2430/**E2440** + E25XX | **E2420**/E2423/**E2440**/E2450 |

- **Overlap:** E2420 (UseAfterMove), E2440 (Exclusivity).
- **MIR-only (true NLL, beyond typecheck):** E2423, E2450.
- **Typecheck-only (MIR not yet covering):** E2400 lifetime, E2410 mutability, E2430 namespace, E25XX actor.

### 2.2. The bomb: driver fatal-stop blinds the MIR cop
`driver/main.rs:58` — typecheck emits error → `return ExitCode(3)`, **STOPS before phase 4**.
Consequence: programs where typecheck catches E2440 **never reach MIR borrowck**. MIR
E2440 (`checker.rs`) is **shadowed dead-code** for any case caught by typecheck →
"E2440 cannot be teeth-isolated" (poisoning MIR → fixture still fails via typecheck). This
is B2's motivation: the MIR cop was blinded, unverifiable, and prone to rot.

### 2.3. Why MIR is the correct tier
MirType (ADR-0050) laid a correct-by-construction foundation. NLL dataflow on CFG is the
standard model for lifetime/exclusivity/aliasing (Polonius-style). Typecheck AST live-range
is an over-strict syntactic approximation ("any-branch-moves => moved",
`borrow_check.rs` admits §loop-conservatism). Maintaining 2 tiers = redundant + prone to drift
(over/under-reject when out of sync).

## 3. Architectural Decisions
**Retire AST-based borrowck. NLL MIR is the SOLE authority for lifecycle checks.**
- Delete module `typecheck/borrow_check.rs` (E2440 AST live-range, 502 lines).
- Delete move-state machine E2420 in `check.rs` (`MoveState` enum + `move_states`
  map + `mark_moved`/`check_used` — **1 emit site** `check.rs:178`, NOT 18
  separate sites as raw surveys suggested; "18" included comments+tests+call-sites).
- Migrate E2400/E2410 (and E2430 if lifecycle-related) to MIR in B2.2+.
- `driver` flow remains unchanged: typecheck (now without borrowck) → lower → MIR verify →
  borrowck. Typecheck remains fatal for TYPE ERRORS; lifecycle checks descend to phase 4.

## 4. Scope & Phasing (G FULL COMMITMENT)

### B2.1 — Eliminate E2420 + E2440 overlap (confidence-building milestone)
- **E2440:** delete `borrow_check.rs` (502 lines) + 1 consumer (`check.rs:435` `analyze_function`).
- **E2420:** delete move-state machine (1 emit + `mark_moved`/`check_used` calls).
- Delete/migrate typecheck E2420/E2440 unit tests (testing the deleted emitter).
- RETAIN `.tri` fixtures (`// ERROR: E2440` matches CODE not NAME — typecheck
  `BorrowExclusivityViolation` vs MIR `NllExclusivityViolation` share error code E2440).

### B2.2+ — Migrate E2400 (lifetime) + E2410 (mutability) to MIR
Each code = 1 slice: build MIR coverage + §5 protocol + teeth + delete typecheck equivalent. Ordered
by complexity (mutability low, lifetime high). E2430 namespace: evaluate lifecycle status
when reached (may belong to name resolution → stays in typecheck).

### OUTSIDE B2
E25XX (actor/Send/Sync/Scope, ADR-0026) — remains in typecheck.

## 5. No-Blind-Delete Verification Protocol (G Mandate, applies to all checkpoints)
Before deleting ANY typecheck checkpoint:
1. **Cluster logic** — identify what edge cases the checkpoint catches (e.g. move-out-of-struct,
   move-from-immutable-ref, reassign-after-move, branch-join-move).
2. **Audit fixture coverage** for each cluster — does the current fixture suite have tests?
3. **Clusters lacking tests → ENFORCE D TO WRITE FIXTURES FIRST** (negative `// ERROR: EXXXX`).
4. **Disable checkpoint → run fixtures → MIR teeth must bite THAT EXACT cluster** (reporting error
   from MIR). Prove 100% coverage BEFORE deleting.
5. **After deletion: teeth-isolate** — poison MIR EXXXX → fixture ACTUALLY fails (no longer
   masked by typecheck). This is B2's victory: first time E2440/E2420 can be teeth-isolated.

## 6. B2.0 Spike findings (O self-verified, did not rely on D's numbers)
- **O self-verified E2440 teeth:** stubbed typecheck `detect_conflicts` → no-op → fixture corpus
  **99/99**, 6 E2440 fixtures caught exact codes **from MIR**. MIR covers E2440 ✓ (hardest
  case — distinct names, identical code).
- **Fixtures match CODE not NAME** (`integration_tests.rs:36`) → switching emitters causes no breakages.
- **Harness collects across all phases** (runs to borrowck even if typecheck errors,
  `integration_tests.rs:65`) → fixtures accurately modeled post-B2.1 behavior.
- **Caveat:** O only personally verified E2440. E2420 (move-state machine) — 5 spike fixtures
  passed but §5 protocol is MANDATORY prior to deletion (G: no blind delete).

## 7. Consequences
- **Positive:** Single lifecycle authority (NLL MIR), teeth-isolated, deleted ~600 lines of
  over-strict AST borrowck, eliminated 2-tier drift risk. Frontend remains clean (types + names only).
- **Negative:** B2.2+ must build MIR coverage for E2400/E2410 (not yet in MIR) — real effort,
  not just deletions. Under-reject risk if §5 is neglected.
- **Related Debt (monitored, not reintroduced):** `conservative=true` (B3) · `is_propagated`
  (A1) in `checker.rs` — B2 must not awaken these landmines when modifying them.
- **Foundation from B1a:** MirType + Struct/Enum split ready to drive NLL.
