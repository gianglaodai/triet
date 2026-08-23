---
name: idea_ternary_placement_syntax
description: "Giang's ternary-sigil memory-placement syntax proposal (+T/T/-T = heap/stack/static) — parked until ADR-0068 (Box) reopens; O's critique attached"
metadata: 
  node_type: memory
  type: project
  originSessionId: a98c1da0-2248-497b-81b9-03efe94816cf
---

**Giang's idea (2026-07-10, a discussion outside the Slice C scope, PARKED until ADR-0068 Box reopens).**

A placement syntax tied to balanced ternary, meant to avoid Rust's "Box zoo":
- `+T{}` → data on the **heap**
- `T{}` (implicitly `0T{}`; the leading 0 is dropped because the stack is the common case) → the **stack**
- `-T{}` → a **global immutable** OR an allocation in a predetermined **memory area/pool** (a performance optimization)

**O's ruling (rejected — not ripe, 4 holes; hole #1 is fatal):**

1. **★ FATAL — placement is not a "polarity", so +/0/− here is imitation, not coherence (it violates VISION §8).** In this project a trit is ALWAYS a POLARITY on one axis with two opposite ends: `~+/~0/~-` (present/absent/error), Trilean (+1/0/−1 true/unknown/false), `&+/&0/&-` (strong/shared/weak). Heap/stack/static are **three parallel categories, not two ends of one axis**. The proof: try the real lifetime axis → stack(−)/heap(0)/static(+); Giang's assignment is stack(0)/heap(+)/static(−) → it **matches no real axis** (mutability is only binary) → the assignment is aesthetic. Aesthetics without meaning **dilute** the very coherence that is the value anchor.
2. **Syntax collision — there is no `&` anchor.** The 5 reference forms survive because of `&` (longest match separates `&+` from `&&`). A bare `+`/`−` collides with unary plus/minus: is `-Point{}` a negation or a placement? A parsing pit.
3. **A false premise.** Rust's Box zoo is NOT caused by ugly syntax but by *genuinely complex placement polymorphism*. The test question: what TYPE does `+Point{}` have? If it is `Point` (placement erased) → borrowck and the drop glue go blind → **unsound**. If it is `Heap<Point>` (placement carried) → the zoo is back (signatures must declare placement, conversions are needed, generics become placement-polymorphic). The syntax is only paint.
4. **`-T{}` merges two orthogonal axes:** `'static`+immutable (lifetime + mutability) AND pool/arena (allocator strategy) — which are independent. The trit runs out of states as soon as you need a mutable global or heap-in-a-pool.

**O's suggested direction if Giang wants to keep the flame:** do not force *placement* (categorical) onto a trit; find a genuinely POLAR axis related to memory (escape/ownership, or lifetime length), let the trit measure that axis, and let placement be **derived** from it → then +/0/− would be consistent with `~`/`&`/Trilean.

**Status:** Giang accepted the PARK: "we can discuss it more when we come back to ADR-0068". ADR-0068 currently BARS Box/recursive → reopening it requires an ADR (ADR before code). Related: [[campaign_truc_b_heap_in_aggregate]] (heap in aggregates) · [[project_vision_os_capable]] (OS-capable = manual placement control with no mandatory GC — the INTENT is right, the syntax is not).
