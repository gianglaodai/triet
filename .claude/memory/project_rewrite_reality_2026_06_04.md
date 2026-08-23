---
name: project_rewrite_reality_2026_06_04
description: READ FIRST — the reality of the repo after the 2026-06-04 rewrite; every handoff or state doc from before that date is OBSOLETE.
metadata: 
  node_type: memory
  type: project
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

**READ THIS FILE BEFORE ANY OLD HANDOFF.** On 2026-06-04 the author **permanently
deleted the backend of the shipped v0.2–v0.10 compiler** and restarted from the
backend (what used to be called "Track B"). Every memory/handoff/state written
BEFORE that date (v0.11 jit.4 at 96%, the AOT cache, two tracks, 1637 tests, a
living self-host) **describes a world that is dead** — read them as history only,
and never base a recommendation on them.

## Deleted (HEAD `6a6bd93`)
Crates: `triet-ir`, `triet-interpreter`, `triet-bootstrap`, `triet-cli` + 5500
lines of JIT legacy. Git history retains them. The 1637-test safety net vanished
with the VM.

## Still alive — 13 crates
Foundation: core, logic, syntax. Reused front end (well tested): lexer, parser,
modules, typecheck. New backend: lower, mir, borrowck, jit, driver. Packaging
(not wired into the new pipeline): pack.

## REAL maturity (do not claim 96%) — updated 2026-06-06
**Closed since the milestone below:** Phase 4.3a String + 4.3b Vector + 4.3c
(Tier A heap, M1-M4, BuiltinShimMeta) and **ADR-0041 Nullable `T?` Tier A**
(PA-3c uniform `i64::MIN`, widening/`~0`/Elvis/`get`, trap-on-0). HEAD `28c1a5f`,
43 fixtures, 1070 tests.

### The 2026-06-05 milestone (kept as history)
**Phase 3 (the Cranelift backend) CLOSED at "Tier A complete":** scalars +
arithmetic + logic ops + control flow + calls + **native flat structs** (StackSlot
+ sret + by-pointer field access; Gate A) + NLL borrowck + the MIR verifier
(INV-1/INV-2). Clean refusals (defence in depth): nested fields `a.b.c`,
Deref/Index (provably unreachable — the lowerer only emits `Projection::Field`),
Outcome ops, multi-value returns. **NOT BUILT:** aggregate literals
(String/Vector/HashMap/Enum/`match` → `Err` in the lowerer = a **phase 4** job,
not the backend's); the Outcome 2-register ABI + multi-return = deferred to
**Tier C**; self-hosting; the AOT cache. A 16-fixture integration corpus (driver)
is the safety net replacing the deleted 1637-test oracle. Report:
`spec/plans/REPORT-2026-06-04.md` + the phase status lines (de-inflated, honest).

## Mines — HANDLED (updated 2026-06-05)
1. ✅ **The orphaned `compiler/` → DELETED** (10 of 10 files rejected by the front end, 23.4K lines).
2. ✅ **Version → `0.1.0-dev`** (a new line, admitting the restart); ROADMAP synced.
3. ✅ **TODO.md → overwritten as the Track B backlog.**
4. ✅ **The JIT Outcome miscompile → GUARDED** (`mir_lower.rs`: 3 ops return `Err` +
   a regression test that goes red when the guard is removed). Provably unreachable
   (the lowerer does not yet produce Outcome).
5. ✅ **Legacy docs/ → `docs/ARCHIVE.md`** (a digest + a catalog of 36 ADRs tagged
   LIVE/TOOLING/HISTORICAL); the semantic ADRs stay alive. The README was rewritten in English.
6. ✅ **spec/plans status de-inflated** (a reconciliation pass): phases 1-6 have honest status.
7. ⚠️ **The schema `Type` enum is DEAD** (typecheck uses its own hand-written Type) —
   tagged spec-only and the SSOT claim was downgraded; migrating Type → schema is a
   **conscious deferral**, on the phase backlog. examples/ and demos/ from the VM era
   are stale fixtures, not yet pruned.

## A memorable recurring pattern (coaching)
When the author reports "done/green", exactly one spot is usually missed, exposed
when the mentor greps: fixture-21 (wrong premise), SSOT (2 spots missed), Gate A
(the `ReturnShape` warning, twice), "the build is green" wrong twice. The cause:
declaring before running the gate command. The cure: run THE EXACT command that
will grade it (`cargo build|grep warning:`; the new test must exist) BEFORE typing
"done". See [[feedback_verify_semantics_before_asserting]].

## The lesson the mentor gave the author
The mistake was not the DECISION to rewrite (a MIR+NLL+native architecture is
cleaner than the old delegate-to-VM JIT — that is reasonable). The mistake was the
ORDER: deleting the VM oracle (1637 tests) BEFORE the new JIT reached parity. The
correct procedure: build in parallel, keep the differential oracle, push to parity,
and only then delete. This was the second time the author followed the pattern
"build to ~90%, then smash it and start over" and reframed the discarded part as
"a draft". See [[feedback_stability_over_speed]].
