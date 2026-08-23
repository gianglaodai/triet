# ADR-0062 — Heap-Nullable repr: ptr-sentinel (`T?` for T ∈ {String, Vector, HashMap})

- **Status:** 🔒 LOCKED — Approved by G on 2026-06-18. Drafted by Mentor O on 2026-06-18, grounded in MIR/JIT line-citations + runtime no-op foundation.
- **Date:** 2026-06-18
- **Drafted by:** Mentor O (dissected slot layout of String/Vector/HashMap + mapped against runtime free-shim no-op foundation).
- **Signatures:** O ✅ (repr grounded, runtime foundation already in place) · G ✅ (approved 2026-06-18 — scope narrowly locked to String/Vector/HashMap, defer Struct?/Enum?; tombstone mandatory for all Drop slices; invariant `ptr==NULL_SENTINEL` carved in stone, `ptr==0` strictly forbidden).
- **Related:** [ADR-0041](0041-nullable-representation-bac-a.md) (scalar `T?` PA-3c `i64::MIN` sentinel — this repr EXTENDS to heap) · [ADR-0049](0049-fat-pointer-abi.md) (String 24-byte slot `{ptr@0,len@8,cap@16}`, "slot is sole truth") · [ADR-0042](0042-ownership-across-boundary.md) (Deinit tombstone — Slice 5 avoids double-free) · [ADR-0044](0044-arithmetic-range-enforcement.md) (NULL_SENTINEL canary lies outside all ranges).

---

## 1. Context — Stdlib Demands It, Compiler Cannot Lower

`T?` for **scalar** `T` (Integer/Trit/Tryte/Long/Trilean/Unit) has been functional since ADR-0041: single-i64 sentinel `NULL_SENTINEL = i64::MIN` (`triet-mir/src/lib.rs:2334`), with canary N1 proving it lies outside all scalar ranges.

`T?` for **heap** `T` (String/Vector/HashMap) is currently **hard-blocked** in `Body::verify()` (`triet-mir/src/lib.rs:1440-1464`, `MirError::HeapNullableNotLowered`): `find_heap_nullable` (1380) scans for `Nullable(inner)` where `inner` is outside the scalar whitelist (`is_scalar_nullable_payload` 1362) → refuses. Rationale for gate (ruling β, signed by G on 2026-06-18): stdlib **declares** heap-nullable in APIs (`env.get`/`path.parent`/`fs.read -> String?`); declarations are harmless (stubbed as `= ~0`), but **compilation** would miscompile — a single-i64 sentinel cannot hold a 24-byte fat pointer. The gate was placed in the LOWERER (not typecheck) so declarations pass while compilation is caught.

**Consequence:** a function `function read() -> String? = ...` passes typecheck but cannot be JIT-compiled. This is a genuine feature capability gap blocking all stdlib I/O optional-return APIs.

## 2. Decision — Repr (a) ptr-sentinel, LOCKED

**`T?` for heap types uses the EXACT SAME slot layout as `T`, adding zero bytes.** The null state is encoded by **the `ptr` field holding `NULL_SENTINEL`** (`i64::MIN`). No boolean flags, no discriminant words, no boxing.

- Null-check = **A SINGLE i64 comparison** on the `ptr` field, NOT a memcmp over the full slot.
- Widening `T → T?` = **NO-OP at the repr level** (same slot; non-null implies `ptr` points to an actual allocation).
- `~0` (null) = writes `NULL_SENTINEL` into the `ptr` field.
- Dropping null = safe and **zero-cost** because runtime free shims already treat `NULL_SENTINEL` as a no-op (§4).

## 3. Memory Layout — Dissecting MIR (G Demands Explicit Offsets)

The three heap types possess TWO distinct slot formats — but ptr-sentinel applies **uniformly** because all three possess a dedicated pointer field:

### 3.1 String — 24-Byte Stack Slot (Fat-Pointer)
```
offset:  0        8        16
        +--------+--------+--------+
slot:   |  ptr   |  len   |  cap   |     (mir_lower.rs:2301 "Must match StackSlot: ptr@0,len@8,cap@16")
        +--------+--------+--------+
        ↑
   null-check inspects EXACTLY this field: stack_load(I64, slot, 0) == NULL_SENTINEL ?
```
- `String?` null  → `ptr@0 = NULL_SENTINEL`; `len@8`/`cap@16` = don't-care.
- `String?` non-null → identical to standard String (ptr points to buffer; len/cap in slot — ADR-0049 "slot is sole truth").
- Null-check = `stack_load(I64, slot, 0)` followed by `icmp eq NULL_SENTINEL` — **1 load + 1 cmp**, without touching len/cap.

### 3.2 Vector / HashMap — Single i64 Handle
```
handle (i64): ptr to [header | len | cap | data...]    (__triet_vector_alloc/__triet_hashmap_alloc -> i64)
              ↑
        handle == NULL_SENTINEL ? = null
```
- `Vector?`/`HashMap?` = the raw i64 handle itself. Null → handle = `NULL_SENTINEL`.
- len/cap/data reside inside heap headers (not in stack slots) → null-check = compare handle directly, **0 dereferences**.
- This is the SIMPLEST case: the i64 handle is itself the pointer field.

### 3.3 Why ptr-sentinel Applies Uniformly
Every heap type reduces to "having an i64 field carrying a pointer" (String: `slot[0]`; Vector/HashMap: handle). Null = that field == `NULL_SENTINEL`. No type requires additional storage for null state → **0 byte overhead**, fulfilling G's mandate ("no redundant 8-byte boolean flags").

## 4. Seamless Fit with Runtime No-op Foundation (Pre-existing — NOT Built Anew)

All free shims ALREADY treat `ptr == NULL_SENTINEL` (and `ptr == 0`) as a no-op — the foundation for Slice 3 (conditional Drop) **already exists**, as measured:

| Shim | Location | Behavior on NULL_SENTINEL |
|---|---|---|
| `__triet_string_free` | `mir_lower.rs:4024` + test `4786` | no-op (confirmed by tests) |
| `__triet_vector_free` | `mir_lower.rs:2469-2470` | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| `__triet_hashmap_free` | `mir_lower.rs:2692-2693` | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| string ops (append…) | `mir_lower.rs:2198` | guards against NULL_SENTINEL |
| vector get OOB / hashmap key-miss | `mir_lower.rs:2575/2848` | RETURNS NULL_SENTINEL (already produces null) |

**Design Consequence:** JIT can invoke `free(ptr)` **unconditionally** on a null heap-nullable WITHOUT crashing — the shim absorbs it. Dropping a null `String?`/`Vector?`/`HashMap?` = free shim call = no-op. Slice 3 (conditional Drop) is primarily **validation + teeth**, not building new mechanisms. (Conditionals in borrowck/lowerer remain necessary for move-out semantics, not relying solely on shims — see §8.)

## 5. Rejected Alternatives

- **(b) Separate boolean flag** (`{is_null: i64, ptr, len, cap}` = 32 bytes): +8 bytes/value, memory bloat, extra field to synchronize. G rejected outright ("wasteful 8-byte boolean flag"). Rejected.
- **(c) Discriminant word** (Outcome style `{disc@0, payload}`): turns `String?` into 32 bytes like heap-Outcome. Wasteful — `String?` DOES NOT require distinguishing 3 states like `T?~E`, only null/non-null, which `ptr` already encodes. Rejected.
- **(d) Boxing / Option-tag on the heap:** introduces an indirection layer + allocation for nulls. Unjustified when sentinel is zero-cost. Rejected.
- **(a) ptr-sentinel:** 0 byte overhead, 1-cmp null-check, matches runtime no-op foundation. **SELECTED.**

## 6. Scope — HARD LOCKED Against Scope-Creep

**IN SCOPE (`is_any_heap()` = `triet-mir:602`):** `String?`, `Vector?`, `HashMap?`. This is what stdlib requires (`fs.read -> String?`, etc.).

**OUT OF SCOPE — DEFERRED (requires separate ADRs):**
- **`Struct?` / `Enum?`** — multi-word aggregates lacking a natural single `ptr` field to seat a sentinel. Requires dedicated design decisions (discriminant word, niche-filling first field, or boxing). MUST NOT be bundled into this campaign. `find_heap_nullable` continues to reject them correctly.
- **`T?~E` ternary heap** (Outcome with null-state + heap payload) — handled independently in the ADR-0053/0058 series (32-byte slot discriminant). Untouched here.
- **Gap #2** (`~0` / nested constructors in block-finals/if-arms not receiving expected types) — type-propagation, a consumer of this repr but an INDEPENDENT lowering slice.

## 7. Campaign Plan (5 Slices — Post 2 Signatures)

1. **Slice 1 — Repr Foundation:** `MirType::Nullable(heap)` accepted by lowerer/JIT; slot equals inner slot; `is_copy` delegates (`654: Nullable(inner) → inner.is_copy`). Whitelist `is_scalar_nullable_payload`/`find_heap_nullable` for heap-nullable (gate transitions from "refuse" to "pass, repr established"). Teeth: `String?` compiles + RUNS.
2. **Slice 2 — Widening + `~0`:** `String → String?` (no-op repr); `~0` materializes `ptr=NULL_SENTINEL`.
3. **Slice 3 — Conditional Drop:** validate free-no-op-on-null (foundation §4) + double-free teeth.
4. **Slice 4 — Elvis `?:` + Match `~+/~0`:** null-check projects `ptr` field (String slot[0] / handle), moves payload on non-null arm.
5. **Slice 5 — `?+>` Map/FlatMap Heap:** unwrap move + Deinit/tombstone (ADR-0042) to prevent double-free.
6. **Remove Gate:** `HeapNullableNotLowered` + cleanup `find_heap_nullable`/`is_scalar_nullable_payload`.

## 8. Risks + Mandatory Teeth (Disarming ABI Mines)

- **Double-Free on Non-Null Drop:** a non-null `String?` moved out and subsequently Dropped → double-free. Slices 4/5 MUST tombstone (writing `ptr=NULL_SENTINEL` post-move) — related to ADR-0057/0058 hazards (dirty-slot → SIGABRT 134). **Mandatory teeth:** poison tombstone → re-drop → SIGABRT on exact arm (success vs null).
- **Sentinel-vs-Zero:** fresh slots init to `ptr=0` (ADR-0049 Slice 3); both `0` and `NULL_SENTINEL` are free-no-ops. MUST distinguish "uninit (0)" vs "explicit null (SENTINEL)" in null-check semantics (uninitialized memory must not be read as valid null). Teeth: probe both.
- **Borrowck:** moving out of a heap-nullable must kill liveness like regular heap types (ADR-0051). Teeth: use-after-move → E2420.
- **All Slices:** teeth must test BOTH String (24-byte slot) AND Vector/HashMap (single handle) — two distinct slot layouts constitute two verification fronts (blind-spot rule).

### 8.1 Amendment (2026-06-18, Slice 4) — Double-Free Reachable IN SLICE 4, NOT Deferred

> **Ruling History (Traceable Correction):** During Slice 4 acceptance, Mentor O initially ruled "double-free is vacuous in Slice 4, defer to Slice 5" based on probes using `match f() {…}` (where scrutinee was a **temporary** → MIR emitted only `Drop(arm_local)` → single free). O reversed this finding when checking real fixtures: they use `let x = f(); match x` (where scrutinee is **named**). For named scrutinees, drop elaboration emits `Drop` for BOTH the arm-local `s` AND the scrutinee `x` at merge blocks — causing two `Drop`s on the same pointer. **The double-free hazard is REAL and reachable DIRECTLY in Slice 4** (match / Elvis), without waiting for Slice 5.

- **Lifesaving Mechanism = M1 Zeroing-on-Move Tombstone** (`triet-jit/src/mir_lower.rs` non-aggregate Assign path, `stack_store(zero, slot, 0)`): when `s = move x`, ptr@0 of the scrutinee is overwritten with `0` → `Drop(x)` reads ptr@0 == 0 → free shim no-ops → exactly ONE live free remains. `String?` follows the non-aggregate path because `ty_total_size` for `MirType::String` = 8 (`is_aggregate` false), making M1 applicable.
- **Tombstone writes `0`, NOT `NULL_SENTINEL`** (ruling (b), G+O 2026-06-18, KEPT UNCHANGED): safe because (1) free shims no-op on both `0` and `NULL_SENTINEL`; (2) moved-out slots are **dead/unreachable** — borrowck E2420 blocks all use-after-move (fixture 191), so the §2 invariant "ptr==SENTINEL ⟺ null, forbid ptr==0" applies to **LIVE** values; dead slots are immune. M1 shares `layout.name=="String"` with non-nullable Strings → DO NOT change (avoids perturbing non-nullable move semantics).
- **★ COUPLING Recorded:** Soundness of tombstone-`0` **depends on borrowck soundness** (use-after-move blocked for all Move types). If borrowck ever permitted a use-after-move on a heap-nullable, tombstone-`0` would expose the dead slot as "uninit (0)" rather than "null (SENTINEL)" — but that would be a borrowck defect, not a repr defect.
- **Mandatory Tooth (NOT Incidental Crash):** Explicit free counting — `present_arm_move_out_freed_once` (`crates/triet-driver/tests/string_nullable_match_move_counting.rs`): non-null present arm → count == 1; poison M1 (slot@0 → slot@8) → count == 2 → RED. Uses counting shims, NOT relying on SIGABRT (forgiving allocators might not abort → turning double-frees into silent leaks). 192/196 value fixtures catching broken M1 only incidentally (crash ≠ EXPECT) is insufficient for memory safety teeth; counting tests provide the true safety net.
- **Deferral to Slice 5 = DIFFERENT Double-Free Path:** Slice 5 (`?+>` map/flatMap) moves payloads into map functions while scrutinees may be dropped independently = double-drop on LIVE pointers (tombstone-on-move cannot save if move-target escapes) — separate front, separate teeth.

## 9. Consequences

- **Positive:** stdlib optional-returns (`fs.read`/`env.get -> String?`) can be compiled; 0 byte overhead; reuses runtime free-shim no-op foundation (minimal new code); consistent with scalar PA-3c (ADR-0041).
- **Costs:** 5 slices + gate removal; each slice backed by lethal double-free teeth.
- **Frozen:** `Struct?`/`Enum?` deferred transparently — no false promises, no dead skeleton code.
- **New Invariant:** "field `ptr` == NULL_SENTINEL ⟺ heap-nullable is null" — locked firmly; all consumers (lowerer/JIT/borrowck) check the pointer field, NOT the entire slot.
