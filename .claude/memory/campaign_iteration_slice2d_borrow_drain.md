---
name: campaign_iteration_slice2d_borrow_drain
description: "🏁 CLOSED — ADR-0089 Slice 2d: for-item drain through a `&0 mutable Vector<T>` (a borrow receiver, the container survives). 3 touch points including a JIT hole D found itself outside the scope. Session 2026-07-27, fortress #6. SHA 014442e+2dcc9b6 (code) + 75e5c6a (ADR)."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-26T19:53:53.234Z
  originSessionId: 1687df81-24d3-40a4-91cb-f1662466d6f7
---

# Session 2026-07-27 — 🏁 Fortress #6: ADR-0089 Slice 2d (`&0 mutable Vector` drain)

origin/main **`75e5c6a`** (synced), gate **`0·clean·0·501·0`** (+5 fixtures 505-509 + mid-break counting).
Code `014442e` (phase 1) + `2dcc9b6` (phase 2), ADR `75e5c6a`. Giang picked #6 ("close the quick one").

## 🏁 What closed
`for item in <&0 mutable Vector<T>>.drain()` — draining THROUGH an exclusive mutable borrow, with the
**caller KEEPING the container** (only the buffer is emptied), which is FUNDAMENTALLY different from
Slice 2b (consume the owner + drop the buffer).
- **Container-survives (§2d.2):** a Vector at runtime is a single-i64 buffer-pointer handle
  (`{len@0,cap@8,data@16}`); a `&0 mutable Vector` reference value is the SAME buffer pointer →
  `pop_front`'s `len--` mutates the shared buffer → the caller sees the drain "for free"; the buffer
  (and its cap) is kept and still owned by the caller. UNLIKE String (whose len lives in a stack fat
  slot → clearing needs the slot pointer).
- **Mid-break caller-drop (§2d.3):** on a mid-loop break the buffer's len has dropped by exactly the
  number popped; when the caller later drops v → `emit_vector_element_free_loop` walks `0..len` from the
  buffer header → freeing ONLY the survivors. The `len--` tombstone (the GOLD of 2b) now also carries
  the caller's later drop (a new interaction).
- **A form-aware fence (§2d.4):** only `ReferenceForm::BorrowExclusiveMutable` + a non-nullable T;
  `&0`/`&+`/`&+ mutable`/`&-` → E1053, and `Vector<T?>` → E1051/E1053 (double nullables, awaiting ADR-0088).

## 🔧 3 TOUCH POINTS (including a JIT hole D found itself, outside the phase-1 scope)
1. typecheck `check.rs:759` — a blind `matches!(Type::Reference(..))` refusal → made form-aware
   (Type::Reference is a **tuple** `(form,box)`).
2. the lowerer `lib.rs:2373` — unwrapping `MirType::Reference{form:BorrowExclusiveMutable, inner:Vector}`
   (in MIR it is a **struct** `{form,inner}`); is_reference already suppresses the drop.
3. **the JIT, `mir_lower.rs:3909`** — the `vector_pop_fat` predicate lacked the Reference unwrap →
   `&0 mutable Vector<String>` (arg0 = `Reference{..}`) fell into `_=>false` → String was treated as thin
   → codegen failed with "unexpected String return". The fix mirrors an idiom that **ALREADY EXISTED** at
   `_get_copy:3967` (`MirType::Reference{inner,..}=>inner.as_ref()`). The 3 marshalling sites
   (out_ptr/dest-bind) consume the existing bool and do NOT re-derive it → no changes needed there.

## 🩸 Lessons (Mentor O)
1. **D found the JIT hole outside its scope and STOPPED at the WO boundary** (the WO forbade touching the
   JIT) → reported to O → O verified it was real → G approved opening touch point #3 (phase 2). Gatekeeping
   discipline: when the soldier disputes the scope with DATA, verify the data (ritual #18), do not force it.
2. **Verifying G's claims cuts both ways — a shape mismatch caught:** G wrote
   `MirType::Reference(_, inner)` (a tuple), when it is really a **struct** `{form,inner}` (mir:507). It is
   typecheck's Type::Reference that is a tuple (types.rs:117). Two layers with DIFFERENT shapes → carve each
   layer correctly into the WO so D does not hit a compile failure. (G was also wrong about E2403→E2420 in
   Slice 2c — G's numbers must be re-measured.)
3. **★ A POISON THAT DID NOT GO RED → exposing a TWO-LAYER no-drop (rituals #4/#16):** poison-1 (push_owned
   on the receiver) did NOT go red — that was not "safe", it was poisoning the WRONG LAYER. The no-drop comes
   from `is_copy(Reference)==true` (mir:736) at BOTH the lowerer (no push_owned) AND the JIT's Drop at :3397
   (skipped). push_owned was masked by the is_copy layer. Escalating the poison to the chokepoint (is_copy for
   Reference → false) → 506 `Drop for type &0 mutable Vector<String> not supported`, **fail-closed** (the JIT
   has no drop glue for references) + the counting test RED. **The container-survives failure mode is
   fail-closed, NOT a silent double free** — safer than G feared. Defence in depth, like the Slice 2b SPOF.
4. **The right poison radius:** removing the JIT's Reference unwrap → the heap case 506 goes RED while the
   scalar case 505 stays fine (the poison only hits the fat/heap path). That proves the fix is correctly
   localized to fat detection.
5. **A precedent idiom in the same file makes a fix low-risk:** the Reference unwrap at `_get_copy:3967`
   (which is how `&0 Vector` get works, fixture 168) proves nothing novel is involved → recommend Option A
   (finish it) instead of a half-hearted descope.

## ⚙️ The procedure — 5 phases + a mid-course phase-2 scope extension
Giang picked #6 → O reconned file:line (verifying 7 of 7 facts, and refuting the red flag "heap borrow
params do not exist yet" with fixtures 93-99) → ADR-first §2d → G approved and measured (E) himself → the WO
→ D phase 1 (scalars, found the JIT hole, stopped) → O verified the hole was REAL and measured its radius →
G approved Option A → SendMessage resumed D for phase 2 (context intact) → O verified with blood (3 poisons
+ the escalation) → O✅/G✅/Giang✅ → O committed the ADR and pushed.

## Remaining debts (blockaded by G, awaiting Giang and O)
🔴 ADR-0088 double nullables T?? (a heavy cliff, ADR first) · HashMap.drain() · deep Clone · §15.6
Vector<Leaf?> · N1 widening (ADR-0065) · the O(N) cursor drain (perf). ⚰️ ADR-0068 Box/recursive is BARRED.

→ [[campaign_iteration_slice2c_force_unwrap]] [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice1_2a]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
