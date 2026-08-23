---
name: handoff-2026-07-16-getref-campaign
description: "Session 2026-07-16 — 5 commits pushed (enum-payload sizing, D1, D2, get_ref Slices 1a+1b). Slice 2 is ALREADY G-SIGNED and ready to hand to D. Backlog parked."
metadata: 
  node_type: memory
  type: project
  originSessionId: 8ee07cdd-6a77-4e51-9e53-9eee20fbfe63
---

# Handover 2026-07-16 — the get_ref value-aggregate campaign (Front 2 of E1049)

origin/main = `006b6c7` (cleanly synced). Gate `0·0·381·0`. The session started at `bf2ed16`.

## The 5 commits pushed this session (in order)
1. `9a1799c` **feat ADR-0067 §AMEND enum-payload-aggregate sizing** — a joint struct+enum co-fixpoint in triet-lower; an enum variant holding a Copy-aggregate payload >8B was already constructible but mis-sized (pinned at 8B) → memory stomping (O's poison: 2 adjacent 16B enums → SIGILL 132). The fix: a shared `resolve_aggregate_size` + a Gauss-Seidel co-fixpoint (capped at 64 → Err). It lifts E1048 (aggregate enum keys become hashable). The ABI is unchanged. Fixtures 368-373.
2. `51f1da7` **fix: refuse nullable enum payloads (ADR-0065 §12.7)** — an `E?` where the enum has ANY payload variant (a 16B scalar or a 24B aggregate) → SIGILL 132 (the discriminant niche is implemented for unit-only enums). Refused at the `OutcomeConstructor` nullable chokepoint (`lib.rs:1898`). Unit-only `E?` is kept (249/250). Fixtures 374-377. **O's original D1 diagnosis was WRONG** (the 1-line struct_map→enum_map fix was inert; the bug was deeper and wider) — O self-corrected through verify-before-WO.
3. `219dc56` **fix: the D2 SwitchInt synth_base collision (a P0)** — EVERY function with ≥2 multi-case matches → a Cranelift verifier crash, exit 4 (NOT only nested ones). Root cause at `mir_lower.rs:4663`: synth_base uses the shared `cfg.blocks.len()` for every switch → switch #2 overwrites switch #1's synthetic block. The fix: a per-block `switch_synth_base` map. Fixtures 378-380 + re-inlining 369/371/372 (dropping the split-function workaround). D caught the `n_cases==1` boundary itself (O had forgotten it in the WO) → made lazy with an Option.
4. `d02c0c4` **feat ADR-0084 Slice 1a** — reading a scalar field through a `&0`: `(&0 Point).x`→value. One level of auto-deref, scalars only, + wiring Projection::Deref + **the Blocker-B patch** (`Statement::Borrow` now uses stack_addr for every struct/enum local, not just String — previously a SIGSEGV). Fixtures 381/382. WART: borrowck is LEXICAL (not NLL); a borrowing local must die before the owner returns (a block or param) → E2450 (ADR-0046, 21/24). NLL is a black hole, deferred.
5. `006b6c7` **feat ADR-0084 Slice 1b** — sub-borrowing an aggregate/heap field: `(&0 h).name`→`&0 String` zero-copy (pointer arithmetic, 0 copies). 4 layers: typecheck (aggregate/heap→Reference), the lowerer (Borrow [Deref,Field]), the JIT (walk_projections offset + base address), borrowck (whole-object fallback + a **reborrow chase** anchoring the loan onto the real h instead of a temporary). Fixtures 383-387. **A nuance O verified:** 386 asserts E2450 (from the chase) but the robust pin is E2400 (independent return inference); the chase is load-bearing for E2450 (proved by a cargo-test poison) but not for soundness — although it is principled and forward-looking for Slice 2. **O suspected the chase was dead code (using the binary) → cargo test corrected it → O admitted the error.** D was transparent throughout.

## 🎯 SLICE 2 — ALREADY G-SIGNED, READY TO HAND TO D (the first task next session)
The final battle sweeping E1041/E1049. **O's recon is done and G signed it; the WO for D has NOT been written.**

**The essence:** the infrastructure is ready — it is just "pulling the trigger". The shim
`__triet_{vector,hashmap}_get_ref` is TYPE-AGNOSTIC (an 8B slot pointer, ADR-0079's "no JIT change"); the
container loan `returns_borrow_of` (MIR Body metadata, `checker.rs:1199`) already locks
mutate-while-borrowed into E2440 for heap V (ADR-0079 U3, tested); and Slice 1b's sub-borrow already reads fields.

**3 layers of work:**
1. **Typecheck dispatch** (`exprs.rs:~1222`, the get_ref arm, and the Vector arm at ~1241): open get_ref to
   aggregate V (UserStruct/UserEnum) → `(&0 V)?`. Currently it only allows `v.is_heap()`. ⚠️ **THE DEATH LINE
   (G stressed it):** branch on `is_ref`→get_ref versus non-ref→get-by-value §AMEND-3. **ABSOLUTELY DO NOT MIX
   THEM** (a get_ref that sneaks in a copy destroys zero-copy). Cover `get(&0 Vector<Agg>,i)` and
   `get(&0 HashMap<K,Agg>,k)`.
2. **JIT call lowering:** guarantee that 100% of aggregate-V `&0`-form calls go to `__triet_*_get_ref` (the
   element's cell pointer inside the buffer) and **NOT get_copy** (G: "drifting into get_copy is a shaved
   head"). The element stride is total_size (correct after ADR-0067 §AMEND).
3. **Borrowck:** set `returns_borrow_of` for the aggregate-V get_ref overload → the loan covers the container.

**THE TEETH G DEMANDS (bloody):**
- **Poison-1 (G's worry):** `let r=get(&0 c,k); c.remove(k)/pop(c); r.field` → E2440. The poison = remove
  `returns_borrow_of` → a SIGSEGV must slip through (proving the metadata is the life-or-death shield).
- **Poison-2 (against a sneaky copy):** force the JIT to run get_copy instead of get_ref → it must go red
  (E2440 or a crash) → proving the right shim is used.
- **The triumphal arch (a positive, the E1049 tombstone):** take a `&0 Tagged` out of a
  `Vector<Tagged{String}>` → read the length of `.name`. **remove is HashMap-only
  (`remove(HashMap,K)→V?`, mutating in place); Vector uses pop/push.**
- The ADR: **§AMEND to ADR-0079** (opening get_ref to aggregate V), composed with ADR-0084. NO new ADR.

## PARKED backlog (awaiting G and Giang to open)
- 🔴 **Deep Clone for heap-bearing aggregates** (Front 1, parked by G — "the lazy line that eats RAM";
  get_ref is zero-copy and philosophically correct). It needs an explicit `.clone()` ADR + a carve-out from
  ADR-0042 move-only + recursive clone codegen. It would open get-by-value for heap-bearing values (currently
  E1049 REFUSE).
- **Full support for nullable enum payloads** (parked by G — there is a sound workaround: a struct wrapper
  `W?` or adding a None variant). Line 491's `Nullable(Enum)→struct_map` (which should be enum_map) is
  latently masked by the D1 refusal; fold it into the full-support work (it is untestable standalone from .tri).
- **The borrowck NLL/lexical wart** (G: deferred indefinitely, do NOT touch ADR-0046/flush_all_for_return).
- drain (the Iteration ADR) · allowing aggregate values in contains · get_ref with V=Nullable · hash caching ·
  `&+ T` borrow params · AOT · self-hosting · the `public use` facade. ⚰️ ADR-0068 Box is BARRED.

[[campaign_typed_collections]] [[feedback_poison_must_be_red]] [[feedback_verify_producer_before_consumer]] [[colleague_d_persona]] [[mentor_o_persona]]
