# ADR 0079 — Get-Borrow Heap Value from Container

> # 🩸 CORE PRINCIPLE (Carved in stone by G, 2026-07-01)
> # "Read values inside the box WITHOUT smashing the box, and WITHOUT biting RAM behind the user's back."
> # `get` ABSOLUTELY does not clone implicitly (hidden heap allocation = garbage of lazy devs).
> # Borrowing (zero-copy `&0 V`) is the RIGHT way. Whoever needs a copy can explicitly write
> # `m.get(k).clone()` — taking full personal responsibility for performance. The critical risk = Borrow Checker:
> # borrowing heap-values from containers = dancing with dangling pointers (drop / realloc / rehash).

**Status:** ✅ **IMPLEMENTED / CLOSED — Signed off by G 2026-07-01.** Applicable to Tier C+. Unlocks `get(&0 container, key) → (&0 V)?`
for heap V (P1: V=String), **zero-copy borrow**, replacing the E1047 refusal in borrow positions.
Slice A (borrowck core, `a970540`): U2 builtin return-borrow PropagatedLoan (whole-container) + U3 mutate-while-borrowed E2440 (consuming insert/push + in-place mutate remove/pop). Slice B (surface+JIT): U1 `get(&0 container,k)→(&0 V)?` overload + U4 `__triet_{hashmap,vector}_get_ref` shim returning zero-copy slot pointer (not-found→NULL_SENTINEL) + F-d Copy-source skip-conflict + U2 source-tracing.
Blood-verified by O: zero-copy reads correct content (`length(ref_str)`→2/5, 0 allocations) · not-found→`~0` clean · source-level E2440 (insert/remove while borrowed) · 5 borrowck poison-sensitive safeguards. **Deferred:** generic V-overload (P1 String only) · get-borrow-mutable (`&0 mutable`) · key-typed containers.
**G's rulings:** loan = whole-container (absolute safety, no fine-grained per-key loans) · not-found = `(&0 V)?`
nullable-borrow (reusing PA-3c, no traps, no error codes) · retain name `get` (overloaded by Borrow vs Value form).

**Siblings / Precedents:**
- **ADR-0046** — PropagatedLoan (return-borrow at call-site, bounded by dest liveness) = REUSED machinery.
- **ADR-0059** — stack-borrow `&0` for heap Vector/HashMap; scaffolding for `&0 get`/`len`/`contains`/`is_empty` (scalar) ALREADY PRESENT.
- **ADR-0077 / 0078** — Typed Vector / HashMap P1 (sound value-typed storage). Heap `get` → **E1047 refusal** (read-side hole closed here).
- **ADR-0025** — borrow checker error codes E24XX. **ADR-0022 / SPEC §10** — 5 reference forms.

**Untouched:** get-borrow **mutable** (`&0 mutable V` into slot — deferred), key-typed `HashMap<String,V>` (Tier 2),
cloning heap values (future explicit method only, NEVER implicit in `get`).

---

## Issue — Why an Immediate Decision was Required

Following ADR-0077/0078, heap-valued containers were **write-and-destroy only**: `insert`/`push` worked, `remove`/`pop` (move-out) worked,
but **reading a `String` inside a map while RETAINING it** triggered `get` → **E1047 refusal** (`check/exprs.rs:1147-1160`).
All practical lookup tables require repeated reads. This was a critical blocker for real-world usability. Design options:

- **Clone** — `get` returns deep copy (fresh allocation). ❌ **Vetoed by G**: hides allocations behind user's back, violating explicit/zero-cost philosophy.
- **Borrow** — `get` returns `&0 V` (read-only borrow, zero-copy). ✅ Reuses ADR-0046 + ADR-0059. **Chosen decision.**

Borrowing heap-values = **dancing with dangling pointers**: if after borrowing, the container drops, or an `insert`/`remove`
triggers rehash/realloc (in C layer), the existing `&0 V` becomes a wild dangling pointer. The Borrow Checker MUST enforce safety.

## Decision

### 1. New Signatures (typecheck/env)

```
get(&0 Vector<T>, Integer) -> (&0 T)?           // T heap; not-found → ~0
get(&0 HashMap<Integer, V>, Integer) -> (&0 V)?  // V heap (key=Integer hardcoded P1); missing key → ~0
```

`(&0 V)?` = nullable-borrow (PA-3c sentinel pointer): present → slot pointer; missing → `~0`. Users MUST
check explicitly (`?:` / `match ~+/~0`) — compiler errors otherwise. Symmetrical with `remove → V?`.

- **Value-position** (`get(map, k)` owned/copy-out heap) RETAINS **E1047 refusal** — only the **borrow position** (`get(&0 map, k)`) is unlocked.
- Scalar `get(&0 container, k) -> Integer` (copy-out) from ADR-0059 REMAINS UNCHANGED (Integer is Copy, producing no loan).

### 2. Loan Model — **Borrowing VALUE = Borrowing entire CONTAINER** (conservative, refuse-over-guess)

The borrow checker **cannot name** `map[k]` as an independent `Place` — slots are accessed via opaque hash shims (`places_conflict`
in `checker.rs:66` already treats Index as cannot-prove-disjoint → conservative). Therefore:

> **The loan of `&0 V` sets `source` = ENTIRE container** (Place of arg 0), `dest` = return-temp `&0 V`,
> `is_propagated = true`, bounded by the liveness of `&0 V` (identical to ADR-0046 PropagatedLoan).

Consequence: borrowing ONE value locks the ENTIRE map against drops + mutations until `&0 V` dies. Sound; users needing
concurrent access can explicitly call `.clone()`.

### 3. G's Three Mandates — Enforcement Rules

| # | Mandate | Rule | Code | Mechanism |
|---|---|---|---|---|
| 1 | **Lifetime obligation** — `&0 V` must not outlive container | drop/return container while `&0 V` alive → error | **E2450** DropWhileBorrowed | PropagatedLoan + E2450 (`checker.rs:1013/1091`) ALREADY EXIST — merely set loan source = container |
| 2 | **Mutate-while-borrowed** — while borrowed, forbid `insert`/`remove`/mutations | consume/move container with active loan → error | **E2440** (or new code) | ⚠️ **UPGRADE REQUIRED** — see U3 |
| 3 | **ReferenceForm interaction** | `&0 V` = `BorrowReadOnly`: composes with other `&0` (e.g. `length(&0 String)`, printing); move / `&0 mutable` → conflict | E2440 | `conflicts_with` (`checker.rs:115`) ALREADY covers this |

## Analysis of Existing NLL → UPGRADE REQUIREMENTS

Existing borrowck engine (`crates/triet-borrowck/src/checker.rs`) provides: `Loan{source:Place, dest, form, is_propagated}`
(`:95`), field-level `places_conflict` (`:66`), `conflicts_with` (`:115`), E2440 (`:328`), E2450 (`:350`),
cross-call PropagatedLoan (`:1100-1143`), M3 builtin consume-marking (`:1148-1161`).

| U# | Area | Current State | Required Upgrade |
|---|---|---|---|
| **U1** | Typecheck/env (`env.rs:441-477`) | `&0 get` overload only supports **monomorphic scalars** (returns Integer copy). Heap → E1047. | Add heap-value `get(&0 container,k) → &0 V` overload (generic V / per-element). Distinguish value position (retain E1047) vs borrow position (unlocked). |
| **U2** | Borrowck **builtin return-borrow** | PropagatedLoan (`:1105-1111`) ONLY runs for **user signatures** (`callee_sigs` + `return_borrow_map`). Builtins use `builtin_shim_meta` (`:1151`) — **NO return-borrow tracking**. | **Declare builtin return-borrow**: `get`(heap) → return borrows arg 0. Mechanism: add `returns_borrow_of: Option<usize>` to `BuiltinShimMeta`, or synthesize a signature. Loan source = entire container. |
| **U3** | Borrowck **move/mutate-while-borrowed** | M3 consume-marking (`:1153-1160`) marks arg Moved **WITHOUT checking active loans first**. E2450 only fires at Drop/Return — NOT at consume-via-builtin. | **Insert check**: before M3 marks a consumed-arg Moved, if Place has active loans (`places_conflict(loan.source, arg)`) → emit **E2440** (mutate-while-borrowed). This fulfills G's Rule #2. |
| **U4** | Lower/JIT | `get` heap not lowered (E1047). | `get(&0 map,k)` → shim returns **slot pointer** (`&0 V` = address of value inside container), **zero-copy** (NO memcpy, NO alloc). New JIT routing for get-heap-borrow. Not-found → sentinel (see Risks). |

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **Get-borrow, loan = entire container** (chosen) | Zero-copy; reuses ADR-0046/0059; conservative soundness | Borrowing 1 value locks whole map | ✅ **CHOSEN** (G mandate: borrow-only) |
| 2 | Get-clone (`get` returns deep copy) | Evades lifetimes; simplifies borrowck | Hides implicit allocations; requires new clone-shim | ❌ **Vetoed by G** |
| 3 | Per-slot loans (`source = map[k]`) | Borrowing 1 value does not lock others | Borrowck **cannot name** dynamic slots (opaque hash shims); heavy region analysis | ❌ Out of scope, unsound with opaque shims |
| 4 | `&0 mutable` get (borrow for in-place mutation) | In-place value updates | Doubles complexity (exclusive loans); no current use case | ⏸️ **Deferred** |

## Consequences

### Positive
- Heap-valued containers **readable** end-to-end, zero-copy — completes usability cycle post ADR-0077/0078.
- Reuses NLL machinery (PropagatedLoan + E2440/E2450) — localized surgical update across 4 points (U1-U4), no architectural overhaul.
- Philosophical coherence: explicit, zero-cost, no hidden allocations. Clone remains an explicit future method.

### Negative
- Borrowing 1 value **freezes the entire container** (no insertions/removals during loan). Users must `.clone()` if concurrent read-and-modify is required. (Intentional conservatism — trading granularity for soundness.)

### Risks to Mitigate
- **Dangling pointers via realloc/rehash:** U3 (mutate-while-borrowed → E2440) is critical. **Safeguard:** borrowing `&0 V` then performing `insert`/`remove` → MUST fail RED with E2440; disabling U3 → poison test fails RED (preventing silent UB).
- **Not-found semantics:** `get(&0 map, k)` when key missing — what is returned? `&0 V` has no "null borrow". Resolution: return `(&0 V)?` (nullable borrow = sentinel pointer). **Approved by G**.
- **Loan source = whole-base:** verify E2450 fires accurately when loan source is a container-local (as opposed to standard locals).

## Finalized Decisions (Signed by G 2026-07-01)

1. **Not-found → `(&0 V)?` nullable-borrow** (Option a). G: runtime traps (b) waste 2 lookups with poor UX; error codes (c) add clutter. Nullable-borrow is sound + symmetrical with `remove→V?` + reuses PA-3c (architectural dividend).
2. **Retain name `get`** — overloaded by form (Borrow vs Value). Value-position heap already refused under E1047 → heap users naturally use `get(&0 map,k)`. Avoids keyword clutter.

## Effective Date

- Tier C+ — get-borrow heap value activated following G sign-off + D implementation + O verification (E2440 mutate-while-borrowed · E2450 drop-while-borrowed · zero-copy content correctness).
- Not retroactive. Scalar `&0 get` (ADR-0059) + value-position E1047 REMAIN UNCHANGED.

---

## §AMEND-1 — Get-ref representation MUST MATCH `&0 V` from locals (thin-handle deref)

- **Date:** 2026-07-04 · **Trigger:** A1 (get-borrow generic-V, ADR-0081 Cluster A) POISON-1 content-read safeguard failed RED.
- **Deciders:** Mentor O (recon+ruling) · **Mentor G SIGNED OFF 2026-07-04** ("The invariant is an ABSOLUTE LAW: `&0 V` must be bit-for-bit identical whether from local or get_ref"). Merged alongside A1.

### Discovered Defect (POISON-1, caught by content-read test rather than routing test)

`get(&0 HashMap<Integer,Vector<Integer>>, k) → (&0 Vector<Integer>)?` followed by
`len(ref_vec)` returned a **garbage heap address** rather than the actual length. Root cause:
the representation of "`&0 V`" was **inconsistent** between pathways when V was a
thin-handle container (Vector/HashMap, 8B handle):

| V | Value Model | `&0 V` from **local** | `get_ref` returned (BEFORE amend) | Expected by `len`/`length` |
|---|---|---|---|---|
| **String** (fat 24B) | 24B inline `{ptr,len,cap}` | address-of-fat (cell) | cell_ptr (address of fat slot) | reads len@+8 from cell → ✅ matches |
| **Vector/HashMap** (thin 8B) | body_ptr (handle) | **body_ptr** (handle by-value) | **cell_ptr** (address of cell holding handle) | reads `*ptr`=len; receiving cell → reads `*cell`=body_ptr = **garbage** ❌ |

`__triet_{vector,hashmap}_len(ptr)` dereferenced `*ptr` expecting a **body_ptr**. `get_ref` returned
a **cell_ptr** (an extra level of indirection for thin V). String avoided this defect because its length
resided INLINE in the fat-struct at the cell_ptr itself.

### Decision — Fix PRODUCER (get_ref), stride-conditional. DO NOT fix consumer.

Locked invariant: **`&0 V` MUST have the identical representation whether obtained from local or from
`get_ref`** — otherwise, all shims consuming `&0 V` (`len`, `get`, …) would need to inspect reference
provenance (unviable).

- **Thin V (`value_stride ≤ 8`, handle):** `get_ref` returns `*cell` (dereferencing cell → body_ptr) —
  matching `&0 Vector`=body_ptr from local.
- **Fat V (`value_stride > 8`, String 24B):** `get_ref` returns `cell_ptr` (address of inline fat struct) —
  RETAINS existing behavior, matching `&0 String`=address from local.
- Applied only on **found-slot** branch; missing keys still return `NULL_SENTINEL`.
- Available accessors: `vector_stride(body)` (mir_lower.rs:4018) · `hashmap_value_stride(body)`
  (mir_lower.rs:4345). ~2 lines per shim.

**Rejected alternative "patching `len` to dereference cells":** would break `len(&0 v)` from locals (where
locals pass body_ptr directly, not cell pointers) — `len` cannot distinguish between body_ptr and cell_ptr.
Fixing consumers was the wrong direction.

### Safeguards (Blood-verified by O)

- Post-fix: `len(ref_vec)` from `get(&0 HashMap<Integer,Vector<Integer>>,k)` returns the ACTUAL
  length (e.g. 3). Poison test: reverting dereference (returning cell for thin V) → garbage → **FAILS RED**.
- String path (fixture 327) UNCHANGED (stride 24 > 8, cell branch preserved) — regression safeguard.

### ⚠️ Warning Bridge to A2 (ADR-0081) — NOTED, resolved BEFORE WO A2

`get_MUT_ref` (A2) **CANNOT** simply dereference — mutable-borrow requires **cell_ptr** in order to
write new handles back into the slot. However, Triet's `push`/`insert` operations are **functional**
(clone + free-old + return NEW handle), not in-place mutations → mutating an inner container via
mutable-borrow REQUIRES writing the new handle back to the cell. But ADR-0081 P1 **FORBIDS write-back**.
⇒ "in-place mutate only" in A2 risks being **VACUOUS for Vector/HashMap values** (only pop/remove —
shrinkage — are true in-place mutations; push/insert — growth — require write-back).
**Must clarify A2 scope with G before issuing WO A2.** ADR-0081 §2 will pin this warning.
