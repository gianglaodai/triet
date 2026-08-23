# ADR-0081 — Get-Borrow-Mutable from Container (`get(&0 mutable c, k) → (&0 mutable V)?`)

- **Status:** ❄️ **FROZEN / DEFERRED (Mentor G order on 2026-07-04).** WO A2 CANCELLED. Shifted to **Cluster D (Phase 3: Ownership — sub-path reassign)**. Re-opened ONLY WHEN core handles `deref-assign` (`*ref = new_val`) + safe handle updates (drop-in-place via pointers). Rationale for freezing: §7 below.
- **Date:** 2026-07-04
- **Deciders:** Author (Giang) · Mentor O · Mentor G
- **Note:** Analyses in §1-§6 REMAIN INTACT (valuable when re-opened). Borrowck architecture (Q1: `returns_borrow_form`; exclusive-loan core conflicting with READs) is sound — the issue is that the API would be VACUOUS on the current functional-mutate core, not a flaw in loan design.
- **Supersedes / extends:** ADR-0079 (get-borrow READ-ONLY) — this is the mutable twin.
- **Related:** ADR-0022 (S6 5-form reference), ADR-0059 (`&0` stack-borrow), ADR-0077/0078 (typed containers).

> **This is an ADR-lite: it DOES NOT open a campaign.** The sole objective is answering 2 soundness questions posed by G and locking P1 SCOPE before D is issued a WO. A1 (get-borrow READ generic-V) proceeds in parallel via direct WO WITHOUT an ADR — because it does not touch borrowck core (loans remain read-shared, propagated). A2 requires an ADR because it plugs directly into the heart of the borrow checker: loans must be **exclusive**.

---

## 1. Context

ADR-0079 introduced read-side borrowing: `get(&0 c, k) → (&0 V)?`, where loans are **read-shared** (`ReferenceForm::BorrowReadOnly`, `is_propagated: true`) over the entire container. Multiple concurrent reads are valid; only **mutate-while-borrowed** triggers E2440.

A2 seeks to introduce its **mutable** twin: `get(&0 mutable c, k) → (&0 mutable V)?` — acquiring a **writable reference** into the slot of element V inside the container, allowing in-place mutation of V (e.g. `push` to an inner Vector borrowed out) without having to move V out and re-insert it.

## 2. Two Soundness Questions from G (and Answers)

### Q1 — How does `checker.rs` know whether the callee returns a read or mutable reference?

**Current State (Reconnaissance):** `BuiltinShimMeta` (`triet-mir/src/lib.rs:1006`) only contains `returns_borrow_of: Option<usize>` — encoding **which argument** is borrowed, WITHOUT encoding the **form**. The form is **hardcoded** at `checker.rs:1173`:

```rust
state.active_loans.insert(Loan {
    source: real_source,
    dest: ret_temp,
    form: ReferenceForm::BorrowReadOnly,   // ← HARDCODED. This is what A2 must fix.
    is_propagated: true,
    ...
});
```

**Decision:** Add field `returns_borrow_form: ReferenceForm` (or boolean `returns_mutable_borrow` to avoid a `triet-mir → triet-syntax` dependency; exact encoding is implementer's choice) to `BuiltinShimMeta`.
- `__triet_{hashmap,vector}_get_ref` → `BorrowReadOnly` (preserved, byte-compatible).
- NEW shims `__triet_{hashmap,vector}_get_mut_ref` → `BorrowExclusiveMutable`.
- `checker.rs:1173` reads `meta.returns_borrow_form` instead of hardcoding.

JIT: `get_mut_ref` shims are **identical** to `get_ref` in codegen — returning slot pointers zero-copy. The distinction between read and mutable is **purely borrowck semantics**, not codegen. (This is why A1 touches `env.rs` while A2 touches borrowck core — two orthogonal axes.)

### Q2 — Mutable borrow through slot: WRITE-BACK permitted (replacing entire V) or IN-PLACE MUTATE ONLY?

The returned pointer references an **8B slot** holding the handle/fat-pointer of V inside the container. Two distinct capabilities:

| | In-place mutate | Write-back |
|---|---|---|
| Example | `push(inner, x)` — where inner is `&0 mutable Vector` | `*ref = new_v` — replacing entire V |
| Touches slot? | NO (handle unchanged) — only V's heap object expands | OVERWRITES 8B slot with new handle |
| Additional Requirements | 0 — only requires exclusive loan keeping slot stable | (a) DROP old V (no leaks) · (b) MOVE `new_v` in · (c) **`deref-assign` syntax** |

**Hazards of Write-Back:** Overwriting the slot without dropping old V = LEAK; dropping old + moving in both occur **through references** — a new move-tracking surface. Furthermore, `*ref = ...` (`deref-assign`) **is not yet wired** in the current language.

**Decision for P1 (G LOCKED 2026-07-04):** **IN-PLACE MUTATE ONLY. WRITE-BACK IS FORBIDDEN.**
- Rationale: Write-back requires `deref-assign` (not implemented) + drop-old-through-ref + move-in-through-ref = distinct machinery warranting its own ADR when use cases arise; without custom allocators and with incomplete trait systems, diving into write-back now is self-destructive.
- In-place mutation covers real needs (mutating inner containers/strings inside parent containers — `push` into inner vectors, `insert` into inner maps) and is **sound purely via exclusive loans** — no new mechanisms required.

> **⚠️ VACUOUS-SCOPE WARNING (ADR-0079 §AMEND-1, discovered during A1 — resolve BEFORE issuing WO A2):**
> Triet's `push`/`insert` are **functional** (clone + free old + return NEW handle), NOT truly in-place. Mutating an inner Vector/HashMap through a mutable borrow requires **write-back** of the new handle into the cell — but P1 FORBIDS write-back. ⇒ "In-place only" risks being **VACUOUS for V=Vector/HashMap**: only `pop`/`remove` (shrinking, genuinely in-place) would work; `push`/`insert` (growing) would be blocked. **Must confirm actual A2 scope with G before issuing WO** — either P1 narrows to "mutate-shrink only" or permits a narrow write-back-handle exception (thin 8B, NO drop-old-V since pop/remove already truncate). TBD.
>
> **⛔ LIMITATION P1 (Signed by G):** FORBID write-back through mutable references. `*ref = new_val` (deref-assign replacing entire V) **IS DISALLOWED** in P1 — parser/typecheck lack deref-assign wiring so it naturally fails; if deref-assign is wired later, it must explicitly refuse `(&0 mutable V)?` until a dedicated write-back ADR lands. `(&0 mutable V)?` in P1 is EXCLUSIVELY for calling in-place functions on V.

## 3. Borrowck Core Modifications (Why A2 Requires an ADR While A1 Does Not)

Read-shared loans (A1/ADR-0079) conflict only with **mutations**. Exclusive loans must conflict with **ALL accesses** to the container during the borrow's lifetime — including READs.

Rooted in schema (`triet-schema.yaml:426`, `BorrowExclusiveMutable`):
> "Scope-limited exclusive mutable borrow. **ONLY ONE `&0 mutable T`** [at a time]."

`Loan::conflicts_with` (`checker.rs:117-122`) is ALREADY correct: `BorrowExclusiveMutable` conflicts with all. But the **use-site check** currently fires only at mutate-sites (U3, `checker.rs:1180-1210`, filtering by `arg_consumes`/`mutates_arg`). With exclusive loans, any **read** (`len(c)`, another `get(&0 c,…)`, or a second `get_mut`) on the same source must trigger E2440/E2410.

**Core Task:** When an active loan has form `BorrowExclusiveMutable`, broaden the check so that **any use** of the source (read or write) = conflict, not merely mutations.

## 4. P1 Scope (G LOCKED 2026-07-04)

1. **Value Types — MIRROR ALL OF A1.** A2 covers EXACTLY the set of V delivered by A1:
   V ∈ {String, Vector, HashMap} + Nullable **if A1 successfully delivers Nullable** (if A1 defers Nullable due to construction constraints → A2 symmetrically defers Nullable). Avoid creating fragmented APIs.
2. **Container Forms:** `&0 mutable Vector<V>` + `&0 mutable HashMap<K,V>`, with **K ∈ {Integer, String}** — **key-parity is MANDATORY**: `HashMap<String,V>` receives identical `get_mut_ref` rights as `HashMap<Integer,V>`.
3. **In-Place Mutate Only** (Q2) — write-back FORBIDDEN (§2 LIMITATION).
4. **Return:** `(&0 mutable V)?` — present slot / `~0` not-found.

## 5. Mandatory Teeth (O will verify upon WO issuance)

- **T1 Exclusive-Read Conflict:** `let r = get(&0 mutable c, k); let n = len(c);` (reading container while mutable borrow is live) → **E2440**. Poison: downgrade form to `BorrowReadOnly` → missing E2440 → RED.
- **T2 Double-Mut:** Two concurrent `get(&0 mutable c, ·)` borrows → **E2440** (ONLY ONE).
- **T3 Mutate-Through-Ref Soundness:** `push(inner, x)` with `inner` borrowed from `get_mut` → runs correctly, container does not double-free or leak (counting).
- **T4 Negative:** Mutable borrow dropped BEFORE reading container → clean compilation and execution (no error).

## 6. Three Locks from G (RESOLVED 2026-07-04)

1. **In-place only in P1, write-back FORBIDDEN** → ✅ APPROVED (§2 LIMITATION).
2. **Value-set = MIRRORS ALL OF A1** (String/Vector/HashMap + Nullable-if-A1) → ✅ LOCKED. No fragmented APIs.
3. **`&0 mutable HashMap<String,V>` key-parity** → ✅ **MANDATORY** (100% parity with `HashMap<Integer,V>`).

---

**Status:** APPROVED architecturally. WO A2 **gated after A1 merges** — because §4.1 mirrors the exact V-set delivered by A1 (knowing whether Nullable lives allows locking A2 overload sets cleanly). O issues WO A2 as soon as A1 lands + O verifies + G signs.

---

## §7 — Rationale for FREEZING (Mentor G Order 2026-07-04)

During A1 implementation (ADR-0079 §AMEND-1), it became evident that `push`/`insert` in the core are **functional-style** (clone → mutate → return NEW handle, freeing old). Returning `(&0 mutable V)?` without write-back capabilities over the parent container's cell makes mutable borrows **VACUOUS** for Vector/HashMap (only `pop`/`remove` shrinking works — largely useless). Meanwhile, P1 strictly forbids write-back because `deref-assign` (`*ref = new_val`) + drop-in-place through pointers **are not yet built**.

**G Ruling:** Half-baked features (working only for String) or "messy compromise loopholes" are rejected. A2 is **frozen** until the core comprehensively supports `deref-assign` + safe handle updates. Shifted to **Cluster D (Phase 3 Ownership — sub-path reassign)** — the same family of "writing through references/paths". Re-opened from there.

**Re-opening Conditions (Definition of Ready for A2 v2):**
1. `deref-assign` (`*ref = new_val`) wired in parser + typecheck.
2. Drop-in-place through pointers (dropping old V before writing new handle) — sound, verified by counting teeth.
3. Only then is A2 sound for V=Vector/HashMap (writing back new handles into cells).
