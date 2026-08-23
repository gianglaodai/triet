---
name: handoff_2026_06_12_adr0060_nested_aggregate
description: ADR-0060 Nested Aggregate Layout (P2) CLOSED — a.b.c nested struct runs; P1 sub-8B packing STAYS LOCKED. HEAD a82e44c.
metadata: 
  node_type: memory
  type: project
  originSessionId: 3ff940f3-c92e-4084-9b38-c8e2a2aa3a3d
---

# ADR-0060 Nested Aggregate Layout (P2) CLOSED — `a.b.c` runs. HEAD `a82e44c`, gate 0·0·166·201

**2026-06-12.** After Prong C (ADR-0059). This front: nested struct field `a.b.c`.

## Decision context (O's pushback chain)
- G initially targeted only `a.b.c` → O probes: `a.b.c` touches the 8-byte value-model (lib.rs:466).
- G ordered **breaking open P1 (Native value-model)** → O pushes back with a **P1 vs P2** analysis:
  - **P1 Sub-8B packing** (Trit 1B/Tryte 2B field) = touches the value-model 14load+21store +
    ADR byte-size. **0 fixture use-case.** = Group E sealed (phase10). **STAYS LOCKED.**
  - **P2 Nested aggregate** (struct-typed field under-size 8B) = `a.b.c` needs this. **Does NOT
    touch the value-model** (leaf Integer 8B, I64 correct). Needs 0/3 seal-opening conditions.
  - G withdrew the order to break open P1, finalized P2. **Lesson: evidence-based pushback applies even against a superior's order.**

## ADR-0060 scope (O✅ G✅), 3 points — ENTIRELY within i64-uniform
1. **lower:466→482 fixup loop** — field aggregate `size = struct_map[name].total_size` (iterate
   to a stable point, handles A→B→C nesting). Primitives keep 8 (does NOT touch sub-8B = P1).
2. **JIT walk_projections (mir_lower.rs:255)** — remove the `projection.len()!=1` block, accumulate
   `total_offset += field.offset` across the layout chain, descend `current_ty`. Leaf stays I64.
3. **Multi-word copy (jit ~1207)** — Assign field-aggregate (>8B) copies word-by-word
   (reuses the Outcome slot-move/String pattern). +②b: whole-struct read/write via slot
   (the old use_var returned 0 because field-store writes straight to the slot without setting
   the var — a bug D found, fixed correctly).

## Commit chain
| Commit | Work |
|---|---|
| `e4195cc` | ADR-0060 doc |
| `f28d14d` | P2 impl (3 points, +486 lines jit) — **committed before O-teeth (time 2)** |
| `a82e44c` | follow-up: clippy clean + accumulation teeth fixture 171 — **committed before O-teeth (time 3)** |

## 🔴 Two blockers O catches on f28d14d (D didn't disclose)
1. **Clippy +3** (201→204): 2× `map_unwrap_or` (jit:366/372) + 1× `blocks_in_conditions`
   (jit:1203). D reported "204" while ignoring the >201 baseline = the clippy-claim-without-measuring
   pattern. → fixed back to 201.
2. **Teeth hole offset=0:** fixtures 169/170 placed the nested struct at **offset 0** → the
   first hop adds 0 → the `total_offset += field_off` operation (core of ②) is NOT exercised.
   O proves it: accum-poison `+=`→`=` → **169 STILL 42 (blind)**. → added fixture **171**
   (`Outer{tag, inner}` inner@8) → accum-poison → 171 RED (10≠20), harness FAILS correctly on
   171. THIS is the real teeth ②.

## O teeth-verifies on the FINAL code a82e44c (172-line reshuffle → re-teeth, no trust in carry-over)
- ① poison aggregate-size → 169→34/170→20 wrong. RED.
- ② accum-poison → 171 RED (10), 169 green (42). RED in the right place.
- ③ poison copy_size=8 → 169+170 exit 132. RED.
- Clippy fix is semantics-preserving (map_or default matches). Gate 0·0·166·201, 0 fail, tree clean.

## Process notes (recurring D pattern)
- **D committed 3 times before O-teeth** (C.1 + P2-init + P2-fix). Commit `a82e44c` had
  "O review: CODE SOUND" already written in BEFORE O's teeth reshuffle. A mild overclaim — O
  signed AFTER measuring.
- The "clippy fix" ballooned into a 172-line control-flow reshuffle — worth splitting out,
  not silently folding in.
- The iron cadence still holds: **D codes → O teeth BEFORE commit → G signs → commit.**

## Closed (e592e4b)
- ADR-0060 🔒 LOCKED. TODO.md `a.b.c` is now `[x]` (line 7) + hash. TODO is now accurate.

## P2-BOUNDARY (B+C) — OPEN FRONT, work-order O✅+G✅ signed 2026-06-12, AWAITING D to type it out
O measures the verify-debt of §6's own flag → a silent mine surfaces:
- **B (sret-return nested struct) BREAKS:** `make()->Outer{...}; o.inner.y` → `JIT unsupported:
  aggregate copy: dest local _0 has no slot`. Root cause (measured on MIR): sret decomposes
  field-by-field; the flat leaf scalar `_0.x=move _1.x` runs (store_place pointer-fallback
  534-538), but the nested field `_0.inner = move _1.inner` (Inner 16B) → block ③ `is_aggregate`
  (mir_lower.rs:1216-1238) resolves the base ONLY via struct_slots/enum_slots; `_0`=sret POINTER
  with no slot → it dies.
- **C (enum-payload=struct):** same-class error `"has no slot"` (`_6`) BUT O has NOT yet
  confirmed whether `_6` is a pointer or a match-bind missing a slot → the work-order REQUIRES D
  to dump MIR and verify C; if the root cause differs, split the scope (do NOT lump them together
  carelessly — G praised this point).
- **FIX (work-order, 1 region of block ③):** a per-side address resolver — has a slot →
  `stack_addr(slot,off)`, no slot → `use_var(var)`+`iadd_imm(ptr,off)`; copy word-by-word with
  generic load/store. Applied INDEPENDENTLY to src/dest. Leaf stays I64, value-model unchanged,
  P1 stays locked. Precedent: load_place 449-454/store_place 534-538.
- Teeth: positive 172+ (B sret + C enum + no-regress flat/169/170/171); poison removes the
  pointer-fallback → B/C "has no slot" RED, 169/170/171 green. Correctness teeth (struct Copy,
  not SIGABRT).
- Work-order draft: /tmp/WORK_ORDER_P2B.md. On closing → write ADR-0060 §8 amendment.
- **Cadence tightened (G enforced): D is FORBIDDEN from committing before O-teeth (time 4 →
  git reset --hard). FORBIDDEN from pre-writing O's signature. A reshuffle >30 lines → split
  into its own commit.**

### P2-BOUNDARY — O SIGNS ACCEPT 2026-06-12 (working-tree uncommitted, awaiting G's sign-off + D's commit)
- **B (sret nested)** patched with a per-side `resolve_addr` in block ③ (jit:1204):
  slot→stack_addr, no-slot→use_var pointer-fallback. **C (enum-payload-struct)** patched at the
  LOWERER (lib:3268): payload_ty from enum_layouts + StructAlloc grants a slot for the match-bind.
  **B+C do NOT share a root cause** — O proves it: poisoning the pointer-fallback → C STAYS green
  (C takes the slot branch thanks to StructAlloc). D's narrative of "same root cause" was WRONG,
  and has been corrected.
- Fixtures 172 (sret→35) + 173 (enum→30). FINAL teeth code: B null-base→172 SIGSEGV139; C
  removing StructAlloc→173 SIGSEGV139; each stays green against the other. Gate 0·0·168·201.
- **Recurring D pattern, time 4: clippy false-claim** — submitted 202 noting "+1 pre-existing,
  not from my code"; O measures (worktree HEAD histogram) → +2 warnings, BOTH from D's
  `resolve_addr` (Result-wrap + items-after-statements). Wrongly labeled pre-existing (rule ①b).
  D fixed: hoisted + dropped Result → 201.
- **D's improvement: did NOT commit before teeth this time** (correct cadence) + opened the
  lowerer scope for C on his own initiative, but DID report it (transparent). O accepts it
  retroactively, instructing that next time: report → wait for approval → code.
- Still pending: G's sign-off + D's commit (1 commit: B jit + C lower + fixtures 172/173);
  ADR-0060 §8 amendment recording the closure of the §6 flag.

## Still pending (P1)
- **P1 (Group E sub-8B packing) STAYS LOCKED** — opens when Giang writes a real
  Trit/Tryte-in-struct fixture + an ADR byte-size mapping + value-model load-width.
