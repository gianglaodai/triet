---
name: campaign_iteration_slice1_2a
description: "✅ CLOSED — the ADR-0084 verification (386 vacuous → sole guard) + ADR-0089 Iteration Slice 1 (loop/break/continue + for-Range) + Slice 2a (for-item Vector copy sugar). Session 2026-07-26, 3 fortresses."
metadata:
  node_type: memory
  type: project
  originSessionId: cc94f7c9-d642-45c2-aa5f-f17917fd833b
  modified: 2026-07-26T10:44:40.186Z
---

# Session 2026-07-26 — 3 fortresses: the ADR-0084 verification + ADR-0089 Slice 1 + Slice 2a

origin/main **`adfe8f9`** (synced), gate `0·clean·0·479·0`.

## 🏁 The ADR-0084 verification (`9ff47c1`) — 386 went from vacuous to sole guard
A verification debt hanging from the previous session: the code had landed and the corpus was green, but the
ADR was a DRAFT and tooth-386 was VACUOUS.
**O exposed the harness:** `integration_test`'s `run_fixture` merges phases (it does not stop on a fatal
typecheck error) → E2450 only appears through the harness; **the real CLI gives 386 → E2400, fatal in
typecheck** (main.rs:58-64 stops before borrowck), so a user would NEVER see E2450. G ordered "stab past
typecheck": add a `dummy: &0 String` param → tie the return borrow → dodge E2400 → E2450 fires CLEANLY in
borrowck (measured twice, by D and O). Replacing 386's guts gave it real teeth.
**O's poison of the chase (checker.rs:710-714, the plain strip)** → 386 compiles CLEANLY with exit 0 (a
would-dangle case reaching the JIT = a UAF) = the chase is the **sole guard on the path to runtime** (unlike
the old 386, which E2400 masked). The ADR's layering theorem: typecheck E2400 guards an UNBOUND return
escape; the borrowck chase is the SOLE GUARD for E2440 (move-while-borrowed, 387) and E2450 (a BOUND return
escape with a param tie, 386).

## 🏁 ADR-0089 Slice 1 (`85371a6`) — loop/break/continue + for-Range (Scope B, amending ADR-0003)
G settled on Scope B (a concrete CFG desugar, FORBIDDING a generic trait — ADR-0003's trait Iterator is
deferred indefinitely + the "AI-first" tombstone). A **loop-context stack** (break_bb/continue_bb/
drop_snapshot). for-Range is the while shape + a **step block** (continue→step, NOT hdr, to avoid an
infinite loop). break/continue call emit_scope_drops over `owned_locals[snapshot..]`, emitting without
clearing (mirroring flush_all_for_return Case-D) → each path drops exactly once.
**Borrowck was NOT touched** (a CFG-generic fixpoint over back edges). 3 guards: **E1052** for a non-Range
in typecheck (killing a silent Unknown), **E0009** in parse_break rejecting a break value (G spotted
`stmt.rs:169` swallowing it silently), and **E1143** for break/continue outside a loop (D refuted the ADR's
assumption that "the parser constrains break" as FALSE with data; changed from the borrowed E1140).
Permanent teeth in **`break_drop_counting.rs`** (O's poison → FREE 3→2, a leak). SPEC §7.2 no longer lies.

## 🏁 ADR-0089 Slice 2a (`adfe8f9`) — for-item Vector copy sugar (scalars + bare Copy structs)
`for item in v` desugars into an index loop with an **infallible in-bounds get** (binding `item:T`, with no
`!!` and no nullable), reusing the raw shims `__triet_vector_get`/`_get_copy` (dropping the nullable wrap).
**No move-out, no tombstone** — it copies bytes and leaves v intact.
**G's minefield #1 (a heap element by value = an alias → a double free):** refused in typecheck with **E1053**.
**G's minefield #2 (handle aliasing → a container double free):** the desugar reuses v's own local (it does
NOT alias the handle into a new owned local). Permanent teeth in
`vector_iter_container_free_counting.rs`, FREE=1 for both lvalues and rvalues.
**🚩 O dug out 2 loose ends after D's first submission:** (a) **an asymmetry creating a NEW silent trap** —
typecheck's `is_copy_aggregate()` broadly allows Vector<CopyEnum>/Nullable while the lowerer gives E1100
(O's probe: Vector<Color>→E1100 is LIVE). G ordered typecheck tightened to match the lowerer EXACTLY:
`is_scalar() || (UserStruct && is_copy_aggregate())`. (b) **dead code** — `if !is_lvalue push_owned` is
REDUNDANT (O poisoned it in 2 directions with no change in FREE → traced `emit_shim_call:1783` push_owning
the arg → D's line was superfluous; D's map trace had missed that layer). Cleanup removed it and fixed the
comment. Poisoning the guard back to broad → 485 E1100 (the trap reopens) = load-bearing.

## Session lessons (Mentor O)
1. **The harness is not the real world** (386): teeth that only bite through a phase-merging test harness can
   be VACUOUS — a real user sees a different code.
2. **Law #12, the stale binary, saved us twice:** probes 478/485 gave E1140/E1100 because
   `./target/release` had not been rebuilt; rebuild first every time you run a binary. It nearly planted a
   false flag (like P1 in the previous session).
3. **push_owned is idempotent** (lib.rs:651) → poisoning it with "double-push the same local" is a no-op and
   never goes red; the real minefield is "allocate a fresh local + Assign". Choose the right poison shape.
4. **emit_shim_call push_owns a non-consumed argument** (lib.rs:1783) — the implicit ownership source for
   iter_local; anyone designing ownership around a shim call must account for that layer.
5. **D (Sonnet 5) was MVP:** it produced the map trace itself, poisoned the handle-alias case itself
   (→SIGABRT) before submitting, and honestly declared the asymmetry and the dead code for O to dig into.
   Blemish: leaving a turn hanging while waiting on the gate (law 17, infrastructure) — O verified with its
   own blood regardless.

→ [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[campaign_typed_collections]] [[campaign_aggregate_nullable]]
