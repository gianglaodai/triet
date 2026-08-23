---
name: campaign_drain_fifo_teeth
description: "✅ CLOSED 2026-07-27(f) — WO-Drain-FIFO-Teeth: carving the FIFO contract for .drain() + locking the E1056/E1054/E1057 cascade order. G REJECTED the O(N) cursor drain INDEFINITELY (the bb2/bb3 edge). 6 fixtures 531-536, ADR-0089 §AMEND-4. 9d59892+04cb5d3, gate 0·clean·0·528·0."
metadata: 
  node_type: memory
  type: project
  originSessionId: f8391b98-1c93-4cb6-a8cf-8e5d66f4073c
  modified: 2026-07-27T17:33:54.746Z
---

## ✅ CLOSED — `9d59892` (fixtures) + `04cb5d3` (ADR §AMEND-4). O✅/G✅/Giang✅ 2026-07-27(f)

Gate `0·clean·0·528·0`. Fixtures 522 → **528**.

## 🎯 STARTING POINT: Giang chose the "O(N) cursor drain for Vector" lane → O RECONNED AND REFUTED ITSELF

O had recommended that lane on the grounds of "mirroring HashMap.drain()'s proven cursor shim".
**The recon reversed O's own recommendation:**

1. **`pop_front:6210` B2 `ptr::copy(data+stride, data, new_len*stride)`** = a memmove of the
   entire tail on EVERY pop ⇒ draining N elements = **O(N²)** — TRUE.
2. 💀 **HashMap's cursor CANNOT be mirrored.** A HashMap has a **state byte per slot**
   ⇒ the drop glue filters on `state==1` = a free per-element tombstone. A Vector buffer is only
   `{len@0, cap@8, data@16}` — **there is no state cell at all**. O's "mirror the proven design"
   label was WRONG, and O withdrew it.
3. 🔑 **The invariant currently carrying soundness:** `buffer[0..len)` is the exactly-live set **at
   every moment between two steps** ⇒ every exit edge is sound **FOR FREE**. The O(N²) cost is
   precisely **the price being paid** for that invariant.
4. 💀 **THE DECIDING MINEFIELD — a mid-loop `return` is its OWN exit edge.** Measured inside
   `fn drain_it` of fixture 534:
   ```
   bb2: { Drop(_1) Drop(_0) Drop(_5) Return(_1) }   // return-mid — HAS Drop(_5)
   bb3: { Drop(_1) Drop(_0)          Return(_1) }   // exit ext  — NO Drop(_5)
   bb4: { If(_4) → +:bb3, -:bb2 }
   ```
   **TWO exit edges with DIFFERENT DROP SETS** ⇒ an epilogue placed at `ext` is **skipped entirely**
   by a mid-loop `return` ⇒ the drop glue stomps a cell already moved out = **a double free**.

## ⚖ G'S RULING: THE O(N) VERSION IS DEFERRED INDEFINITELY; FIFO IS A CARVED-IN-STONE CONTRACT

- **V-C (LIFO, switching `pop_front`→`pop`) REJECTED** — it changes observable semantics.
- **V-D (reverse the buffer then pop the tail) REJECTED** — the survivors of a mid-loop break end up reversed.
- **V-A/V-A′ (cursor + epilogue) REJECTED** — they require cutting into the lowerer's `Stmt::Return` so the
  epilogue runs on EVERY exit edge. *"We are not gambling the correctness of the entire return path to repay
  a performance debt nobody has complained about."*
- 🔑 **O(N²) is a PERFORMANCE DEBT, NOT a soundness hole.** Tenet 3.

## 🩸 SECONDARY FINDING: THE CORPUS WAS BLIND TO ORDER — and O corrected its own claim

O declared *"flipping FIFO→LIFO turns NO test red"*. **WRONG.** O poisoned it personally
(`lib.rs:2639` `pop_front`→`pop`) → **1 of 522 went red**: `490_drain_break_continue.tri`.

But it went red **BY ACCIDENT**, not by design: 490 tests break/continue with a value-matching condition
(`if x == 100 { break }`), and LIFO pushes `100` to the front ⇒ an immediate break ⇒ `sum=0`.
**Editing that constant in 490 would silently destroy a protective layer.**

The core held: **the 6 order-related fixtures (486/487/488/505/506/509) were 100% GREEN** under the flip.
`509` is blind because **every String has `length == 1`**.

🦷 **Lesson: even a RED test must be asked WHY it is red before you trust it as a guard.**

## 🦷 TEETH (O pre-poisoned BEFORE writing the WO — law 21)

A **position-weighted** oracle `acc = acc*10 + v`; **a running sum is FORBIDDEN** (a sum is blind to order).

| Fixture | Shape | EXPECT | Under the LIFO poison |
|---|---|---|---|
| 531 | owned `Vector<Integer>` [1,2,3] | 123 | **321** ✅ |
| 532 | owned `Vector<String>` lengths 1/2/4 (heap move-out) | 124 | **421** ✅ |
| 533 | `&0 mutable` + a **mid-loop break** + reading the survivors | 1024 | **4012** ✅ |
| 534 | `&0 mutable` + a **mid-loop return** + reading the survivors | 1024 | **4012** ✅ |
| 535 | `HashMap<KP,Integer?>` + `for x in` (wrong on ALL 3 axes) | E1056 | key before pattern → **E1054** ✅ |
| 536 | `HashMap<KP,Integer?>` + `for (k,v) in` (wrong key + value) | E1054 | value before key → **E1057** ✅ |

🔑 **534 is the FIRST fixture in the whole corpus to touch a `return` inside a drain loop** — the very
`bb2` edge that killed the O(N) proposal. From now on it has a guard.

The Lane 1 poison turned **531-534 red AT THE HARNESS LAYER** (the corpus went from 1 red to 5 red), not just
at the driver layer. Lane 2's two spears are **orthogonal**: spear A turns only 535 red (510/527/528 stay
green ⇒ 535 is the SOLE guard on the pattern-before-key edge); spear B turns only 536 red.

## ⚔ O'S BLEMISHES THIS SESSION (three, all caught by measurement)

1. Recommending the O(N) lane → self-refuted after measuring `bb2`/`bb3`.
2. "The corpus is blind to order, 0 tests go red" → really 1 of 522.
3. Nearly reported "the `is_struct_widening` branch has zero coverage" when the corpus **printed nothing** —
   **no output ≠ green**: the process had died (SIGILL) taking the `test result` line with it. Law 15 saved it.

⚔ **O's fourth blemish — a `bb9` label from the wrong source:** O pasted block numbers from its own `/tmp`
probe (an owned receiver, inside `main`) into the WO; D copied them. 533/534 have a different shape, hence
different numbers (`bb2`/`bb3`). **O sent it back to D, and D's correction was BETTER than the original**
(the evidence lives in the corpus and is self-verifiable, instead of pointing at a `/tmp` probe that will vanish).

## ⚖ D: 0 blemishes, 1 justified deviation

**LAW 5 approved:** D made §AMEND-4 a standalone section at the end of the ADR instead of inserting it into
the Slice 2b preamble — matching the §AMEND-2/§AMEND-3 template, and its content covers BOTH 2b and 2d.
The raw gate was complete on round 1 (the infrastructure decree is working). D dumped the MIR itself to
verify O's finding **before** making the change, instead of copying blindly.

## Debts recorded per G's ruling

- `len()` is missing the `Vector<String>` overload — backlog, **FORBIDDEN to touch** in this campaign.
- Fixture `490` is an accidental protective layer — **FORBIDDEN to edit** (surgical discipline).
- The O(N) cursor drain — **deferred indefinitely**, not to be reopened until an ADR proves soundness on
  EVERY exit edge.

[[campaign_hashmap_drain_pa2]] [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice2d_borrow_drain]] [[campaign_aggregate_move_tombstone]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
