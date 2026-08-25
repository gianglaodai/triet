---
name: idea_ternary_placement_syntax
description: "Balanced-ternary memory placement +T/T/-T (heap / stack / heap-immortal-frozen) — UNPARKED and ruled 2026-08-25; the axis is deallocation obligation, -T is the prize, +T is the price"
metadata: 
  node_type: memory
  type: project
  originSessionId: f1274d46-292b-4dbb-b77f-aa8ea2d0f365
---

# Placement trit `+T` / `T` / `-T`

**Status: UNPARKED 2026-08-25** (discussion-only session, Giang & O — no code, no ADR written).
Supersedes the 2026-07-10 PARK. Iron law restated by Giang: **every outstanding debt must be paid
before the Box/placement work starts. No debt may be left behind.**

## What the three mean (Giang, LOCKED 2026-08-25)

| Form | Storage | Semantics |
|---|---|---|
| `+T{}` | heap | unique owning pointer, freed at scope exit. Replaces `Box<T>`. |
| `T{}` | stack | frame slot, today's implicit default. The leading `0` is never written. |
| `-T{}` | **`.rodata`, immortal** | **comptime-evaluated, GLOBAL declaration only.** |

### ⛔ RETRACTED 2026-08-25 (after merging `c537b89`) — DO NOT FOLLOW THIS SECTION

**Giang ruled: take the parallel session's position.** The authoritative design is
[[campaign_placement_polarity_adr]] (L1–L26, G BLOCKED B1–B6). Under **L10** `-T` is **IMMORTAL, not
`.rodata`** — welding the semantics to one implementation was the error. Under **L11** the shipping
constructor is **C2: `immortal <expr>`, a one-way `+T → -T` promotion** (needs nothing, ships v1);
**C1 (comptime `.rodata`) is DEFERRED INDEFINITELY** because it additionally requires transitive
const-placement of every interior allocation.

The leak fear that drove the reversal below is answered more cheaply by **L15**: `immortal` may appear
**only as a top-level statement of the root module's `main`**, and `main` may not be called ⇒ every
immortal allocation site is statically enumerable by reading `main`, each running exactly once. That
keeps runtime data (startup config, interners, symbol tables) — the three cases the reversal below
would have thrown away — **and** bounds the leak. Plus **L17②**: the decision to leak always belongs to
the application, never to a library.

Kept below only as the record of a reversal made without knowledge of L15. Everything after this
paragraph is superseded.

### ~~REVERSED by Giang, later on 2026-08-25~~ (superseded — see above)

An earlier ruling in the same session had `-T` as a **runtime expression** (`Box::leak`, explicitly
"NOT `.rodata`, NOT comptime"). Giang **overturned it** after spotting the leak: an expression's
cardinality is the number of times it executes, so `-T` inside a function called in a loop grows RAM
without bound and without any signal — the "silent wrong" shape one layer up. O had proposed a
declared arena budget + a loop lint; Giang rejected mitigation in favour of removing the failure mode:

> **`-T` is GLOBAL-declaration-only and must be computable at COMPILE TIME.**

Cardinality is then the number of declaration sites — known statically, leak impossible. Type-position
`-T` still works anywhere (`struct Parser { keywords: -KeywordTable }`); only the `-T{...}`
construction is confined to a global initialiser.

**What the reversal BUYS:** leak impossible · **safe publication dissolves** (the loader writes
`.rodata` before `main`, so there is no construction to publish — most of §6d's problem evaporates) ·
**`mprotect` for free** (the OS maps `.rodata` read-only; a stray write faults instead of corrupting —
no segregated arena, no page-sealing code) · **fastest possible read** (link-time address ⇒
RIP-relative, **no pointer load at all**) · the allocator is never involved.

**What it COSTS:**
1. **`-T` can no longer hold runtime data.** Config read at startup · interned strings from input ·
   a symbol table built while parsing — **all three fall out**, and they were the flagship arguments
   for `-T` being the third leg of the no-`Rc` architecture. **The `Rc`/`Arc` gap is only HALF closed**
   — the compile-time-known half (lookup tables, grammars, keyword sets, error tables).
2. **The build order REVERSES.** `-T` was ranked cheapest-do-first; it is now gated behind const-eval
   plus `.rodata` emission, both measured at **0 lines** in the tree (`const_eval`/`comptime`: no hits
   in `crates/`; `.rodata` appears only as an AOT comment at `crates/triet-jit/src/mir_lower.rs:3056`).
   ⇒ **`+T` is now the ONLY route to recursive types, hence to self-hosting.** Note it needs only a
   *minimal* const-evaluator (literals, arithmetic on literals, all-const composites) — **not** a
   Zig-scale comptime subsystem, so it must not be exiled to post-self-hosting by association.
3. **ADR-0026 §3.2's refcount is NOT retired** by this: a comptime `-T` cannot carry runtime-built
   frozen data, so cross-thread sharing of runtime objects still needs it. Item M's prize shrinks.

**The clean resolution of cost 1 — split the duties.** One sigil had been carrying two jobs with
opposite safety profiles. They separate cleanly:

| Need | Mechanism |
|---|---|
| Immutable, known at compile time | **`-T`** (`.rodata`) |
| Immortal, but built at runtime | **Arena (+ generational index)** |

Disjoint sets ⇒ **open item K dissolves** (it existed only because `-T` and Arena overlapped on the
graph case; Pillar ③ is satisfied without a ruling).

**`-T` is FROZEN, transitively** — and now enforced by hardware rather than by `mprotect` bookkeeping.

### 🔜 Next session's question (Giang, 2026-08-25)

> **Can `Rc` be dropped entirely? And is `Arc` a question only from self-hosting onward — i.e. only
> once real multithreading exists?**

Giang's own assessment: **`Arc` was his premature decision** — reaching for multithreading before
self-hosting, driven by a real worry but landed too early. Re-open both from first principles:
what actually needs shared ownership once `-T` covers compile-time constants and Arena covers
runtime-built immortal data.

## Why O's 2026-07-10 rejection no longer applies

The old critique's fatal hole #1 ("placement is not a polarity") and the `&+ T` ≈ `Box<T>` collision
both rested on **conflating ownership with placement**. Giang's decomposition separates them:
`&+`/`&0`/`&-` answers *who owns*; `+`/`0`/`-` answers *where the bytes live*. Rust's `Box<T>` fuses
both, which is the actual source of the wrapper zoo. Hole #2 (unary `+`/`-` collision) was **measured
and withdrawn**: `crates/triet-parser/src/expr.rs:247-248` has no unary `+` at all, and a placement
reading of `y + T{}` would leave two operands with no operator, so it is not ambiguous; only `-T{}`
vs negate needs 2-token lookahead. Hole #4 was closed by moving pool/arena out to the separate
Arena+NodeID pillar.

**Naming requirement (must be in the ADR):** "heap / stack / static" is three parallel categories,
not a polarity — writing it that way makes the VISION §8 coherence claim unearned. The real axis is
**deallocation obligation**: `+T` = must call `free` (max) · `T` = frame does it (auto) · `-T` = never
(none). Monotone, two opposite ends, 0 correctly in the middle, and it maps 1:1 onto drop glue. It
also lines up with the `&` family's own ordering (`+` = most responsibility, `-` = least).

## The rule that decides the whole design

**Placement is ERASED in representation, RETAINED in the type checker.**
- Erased: `&0 T`, `&0 +T`, `&0 -T` are all one 8-byte pointer, identical codegen. Zero cost.
- Retained: `&0 -T` may escape and may live in a long-lived struct field; `&0 T` may not. The escape
  rules differ, so the checker must still tell them apart.
- Kept clean by **one-way widening**: `&0 -T` is usable anywhere `&0 T` is expected (immortal is
  strictly more permissive). No reverse direction. 90% of signatures never mention placement.

Deeper consequence, and the strongest argument for placement living in the type system:
**placement is what feeds the borrow checker.** It is what makes escape analysis decidable without
lifetime variables. `-T` is not a reader's annotation.

## The honest ceiling — the zoo does NOT fully die

Placement cannot be erased at *owning* positions, because representation differs: owning a `T`
means holding N bytes inline, owning a `+T` means holding 8 bytes. Erasing that would need a tag or
forced indirection ⇒ runtime cost ⇒ violates Pillar ②. Therefore `Vector<+Node>` and `Vector<Node>`
stay distinct types forever, and functions taking ownership must still pick a placement. What dies is
the zoo in *reading* signatures. Also note Rust already erases placement at its `&` boundary via
deref coercion — the novelty is not there.

## Value ranking (O's assessment)

| | Novelty | Value | Cost |
|---|---|---|---|
| **`-T` immortal + frozen** | **High** — no language puts deliberate leak in the type system with a frozen guarantee | **High** — replaces `Arc` on the immutable branch, 0 atomics | **Lowest** — no-op drop glue |
| Placement trit erased at borrow | Medium — Rust erases at `&` already | Medium — one less wrapper name, clearer reads | Medium |
| **`+T`** | Low — `Box` renamed | Needed for recursive types; not a differentiator | **Highest** — re-walks the whole representation matrix |

**`+T` is the part that must be built; `-T` is the part that is worth building.**

### Why `-T` is load-bearing, not a bonus

The no-`Rc` architecture stands on three legs and G only named two: ownership down (`+T`) and
reference up (`&-`). The third — **immutable data used in many places that belongs to no tree**
(interned strings, symbol tables, config loaded once, lookup tables) — is exactly where people reach
for `Rc`/`Arc`. Without `-T` the alternatives are all bad: hoist to the top of the tree and thread
`&0` through every signature; Arena+ID for a single object; or duplicate. Users hit this in week one
and demand `Rc`. **`-T` is what keeps the no-`Rc` promise from breaking.**

Precise formulation: **`&0 -T` is the only reference that can sit in a long-lived struct field with
no lifetime reasoning and no counter.**

### Access-speed profile (asked 2026-08-25)

Not the fastest for a single touch — `T` (stack, inline, no indirection, frame already in L1) wins,
and a true `.rodata` constant (RIP-relative, no pointer load) would beat `-T` too, but cannot hold
runtime data. `-T` wins decisively on **repeated and multi-core reads**, and it wins because of
*frozen*, not *immortal*:
- Loads are provably invariant (transitively frozen, no interior mutability ever) ⇒ Cranelift
  `MemFlags::readonly` ⇒ hoist out of loops, CSE across calls. `+T` cannot: it is mutable through
  `&0 mutable`, so every intervening call forces a reload.
- Its cache line stays Shared in MESI and is **never invalidated** — nobody writes. Contrast `Arc`,
  whose refcount line ping-pongs between cores on every clone/drop. That is `Arc`'s scaling wall.
- Its own arena bump-allocates contiguously ⇒ good cold locality. The arena needed for `mprotect`
  sealing pays for itself twice.

## Entry price — debts that BLOCK `+T`

> ⚠️ **The LIVE, ordered queue is `TODO.md` §🎯 HÀNG ĐỢI NỢ TRƯỚC BOX** (items A→E code, F→M design)
> — that is the tracking authority and what O recites at session start. The list below keeps the
> *reasoning* for why each blocks; if the two ever disagree, `TODO.md` wins and this section is stale.

From `campaign_reference_operand_eq_refuse.md:94-113` (G's order, 2026-07-30(b)):

1. **`&0 T?` bypasses E1033** (recorded debt #5) + **`Reference{Reference{…}}` unmeasured**. Same
   disease: an outer wrapper blinds a `matches!` shape check inside. **`+T` is a third wrapper** —
   every site blind to `Reference` will be blind to `+T` identically. This is the exact shape that
   produced 5 "silent wrong, exit 0" campaigns. Fix as a pattern (one shared peel-wrappers helper),
   not per site — that pays for `Reference`, for `T?`, and **prepays for `+T`**.
2. **P2 `WO-Literal-Temp-Drop-Leak`** — rvalue temporaries are never dropped. `+T{...}` *is* a heap-
   owning rvalue temporary; same root, so every temporary `+T{}` would leak.
3. **P3 `WO-Harness-Subprocess-Isolation`** — in-process harness, one SIGILL kills every later
   assertion. `+T` development will produce SIGSEGV/SIGILL constantly (recursive types, recursive
   drop glue). A lying gate is more dangerous here than the leak.
4. **Enum sret with a String payload unmeasured** (E2423 blocks the probe) — `enum` with a `+T`
   payload is the 90% polymorphism path locked by `ADR-0061:123`.

Partially blocking: **P5 Nullable-Eq-Unknown spec gap** — `+T?` inherits whatever `Nullable` does, so
settle the SPEC first or breed another variant of the same hole. Not blocking: P4 `String <` spec gap,
Outcome 2-reg ABI, multi-value return, native layout, AOT.

Debts this session ADDED (design level, zero code): the placement ADR itself · the `&0 -T <: &0 T`
widening rule · choosing one main road between `-T` and Arena+ID for the graph case (Pillar ③) · plus
the reference-form debts in [[idea_reference_forms_and_refcount_debt]].

## Open

- ⚠️ **`ADR-0068` does not exist as a file.** `0070:271`, `0077:17`, `0078:20`, `0086:60` all cite it
  as the ban on recursive types / Box. The prohibition is **unwritten**. Write it or lift it — do not
  leave a branch of the architecture hanging on an empty number. Next free ADR number is **0090**
  (0068 and 0073-0075 are gaps; do not fill them).
- Drop glue for a deep `+T` chain recurses ⇒ stack overflow on a long list. Iterative drop or accept?
- `-T` in a loop leaks without bound. Accept (the syntax says so out loud) or restrict where `-T` is
  allowed?
- `-T` overlaps Arena+NodeID on the cyclic-graph case: `-T` is more ergonomic (real references),
  Arena+ID has better cache behaviour (`u32`, contiguous). Pillar ③ forbids keeping both as the
  default road.
- Long-running processes: an immortal AST is right for a batch compiler (rustc arenas do this) and
  **wrong for an LSP server** — Triết targets OS-capable, so long-lived processes are the main case.
- Arena+NodeID is not counter-free either: a reused slot makes an old `u32` silently address a
  different node (ABA) — the exact "exit 0 but wrong" shape dug up 5 times. The fix is a
  **generational index**, which moves the counter from the object to the index rather than removing it.

Related: [[idea_reference_forms_and_refcount_debt]] · [[idea_post_rust_architecture_and_ternary_foundations]] ·
[[campaign_truc_b_heap_in_aggregate]] · [[project_vision_os_capable]]
