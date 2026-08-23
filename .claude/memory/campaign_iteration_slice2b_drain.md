---
name: campaign_iteration_slice2b_drain
description: "🏁 CLOSED — ADR-0089 Slice 2b: for-item-in-Vector.drain() consuming move-out (heap opened up), a zero-shim desugar to pop_front. GOLD: the tombstone is DOUBLY LOAD-BEARING. Session 2026-07-26(b), the fourth fortress. SHA 780e081."
metadata: 
  node_type: memory
  type: project
  originSessionId: e83f091f-aeba-435d-b4e9-a103c024fd1c
  modified: 2026-07-26T15:51:00.082Z
---

# Session 2026-07-26(b) — 🏁 Fortress #4: ADR-0089 Slice 2b (drain & consuming iteration)

origin/main **`780e081`** (synced), gate **`0·clean·0·488·0`** (+9 fixtures 486-494, +4 counting teeth).
Following the three-fortress session (adfe8f9): Slice 1 + Slice 2a → now Slice 2b closes the **consuming** direction.

## 🏁 ADR-0089 Slice 2b (`780e081`) — `for item in v.drain()` move-out
`for item in v.drain()` consumes a `Vector<T>` element by element, **moving out by value** for EVERY T
(including **heap** types — `Vector<String>`, `Vector<User{String}>` — the difference from Slice 2a, which
refuses with E1053 because a copy is an alias: draining transfers ownership → no alias remains). The
container drops **buffer-only** after the loop.
- **Zero shims:** it desugars into a `pop_front` loop — 100% proven pieces (fixtures 347/351/338). It does
  NOT touch the JIT, borrowck, or the schema. The architecture G approved: accept **O(N²)** (the pop_front
  shift) correctness-first, with an O(N) cursor drain as future perf debt.
- **2 touch points:** (1) typecheck `check.rs check_for_stmt` — `.drain()` is a for-guard-ONLY pseudo-method,
  matched FIRST, inferring ONLY the receiver (a standalone `v.drain()` still gives E1041). (2) the lowerer's
  `lib.rs Stmt::For` drain arm — a header `pop_front` + a present test `If(Eq opt,NULL_SENTINEL)` + a PA-3c
  identity unwrap; break→ext, continue→hdr (with NO step block, since pop_front advances by itself).
- **error.rs +62:** 2 variants `DrainNullableElement`/`DrainBorrowedReceiver` — both on the SAME code E1053
  (with a drain-context message), no new code.

## 🥇 THE GOLDEN FINDING — the tombstone is DOUBLY LOAD-BEARING (measured by O's poison)
Removing `len--` from `__triet_vector_pop_front` (`mir_lower:6162`) produces **TWO distinct failure modes**:
- (a) **a full drain HANGS FOREVER** — pop_front never reports empty (len never moves) → the present test
  never ends the loop. So `len--` is the loop CFG's **termination condition**.
- (b) **a mid-loop break FAILS** — the Drop re-walks an already moved-out slot → a survivor re-free mismatch
  (a double free). So `len--` is also the **double-free latch**.
→ `len--` carries a **DOUBLE LOAD** (carved into ADR §2b.5). The teeth in `drain_iter_counting.rs` guard both.

## The 6 steel conditions (G's mandate) — ✅ verified with blood
491 standalone→E1041 · 492 `&0 Vector`→E1053 · 493 `Vector<T?>`→E1053 (double nullables deferred) ·
494 an unknown method→E1052 · 487 `Vector<String>` + 488 `Vector<User{String}>` with the REAL allocator,
total=5 · counting: rvalue FREE=1 + mid-break=5 + container=1.
A bonus O ruled out itself: **sentinel collision** (a `Vector<Integer>` containing `i64::MIN`) is IMPOSSIBLE
— the PA-3c sentinel lies outside the valid Integer range (ADR-0044/E1036). `Deinit(opt_local)` = **zeroing,
NOT freeing** (JIT `mir_lower:2928`).

## ⚙️ The session's procedure — all 5 phases (recon→ADR→WO→D→verify)
O reconned file:line → presented the map (drain = proven pieces) → G approved a Vector-only scope
(REJECTING HashMap.drain, "one fortress at a time") + 6 steel conditions → Giang signed the order +
**O spawned D (Sonnet 5)** → blood verification.

## 🩸 Lessons (Mentor O)
1. **D was cut off mid-poison-verification** (`<result>`="I'll stop here and wait for background task…")
   — this was NOT a fake submission. It left poisoned code in the tree (`mir_lower:6164`, the tombstone
   commented out) = a half-finished verification state, with no malice. O restored it to HEAD `da3a0d80`
   (never committed).
2. **D's docstring contained a wrong hypothesis:** it claimed the poison gives `STR_FREES==6`, when the
   REALITY was an infinite hang (the loop never reaches the Drop that would count 6). O corrected the
   docstring to the measured truth. → A hypothesis written before running is not a verification.
   O touched ONLY D's test COMMENT and never D's logic (check.rs/lib.rs/error.rs untouched — "no fixing
   things for them").
3. **An infinite loop IS the poison signal** for drain (unlike the double free of a single pop). Giang
   himself noticed "this is running too long" → O must **bound everything with a timeout and time** any
   command that can hang (the poison makes the loop infinite). New law: when poisoning a loop structure,
   wrap it in a hard `timeout`; exit 124 = a hang.
4. **Verify-don't-trust saved it again:** O ran the gate itself and planted its own poison independently
   (cp snapshot → poison → measure red → restore with a matching md5), trusting nothing D said. That
   peeled off 2 loose ends (the left-behind poison + the false docstring).
5. G's rule "evidence is king, we do not worship procedure" — the code was sound and O had verified
   independently → sign and commit, with no pointless ceremonial recall of D.

## Remaining debts (their own campaigns, awaiting a fresh start-of-session recon)
🔴 ADR-0088 double nullables `T??` (`Vector<T?>` drain currently gives E1053) · HashMap.drain() (its own
bucket state gate) · `&mutable Vector` drain (a borrow receiver, currently E1053) · the O(N) cursor drain
(perf) · deep Clone · §15.6 Vector<Leaf?> · N1 widening · `!!` ForceUnwrap (Slice 2c — G's suggested next spear).

→ [[campaign_iteration_slice1_2a]] [[campaign_typed_collections]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]
