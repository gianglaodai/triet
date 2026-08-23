# ADR-0041: Nullable (`T?`) Representation — Tier A

**Status:** CLOSED — Mentor O SIGNED (semantics & soundness, 2026-06-06) + Mentor G SIGNED (layout/ABI/codegen, 2026-06-07). Implementation Steps 1–4 verified, 43 fixtures, 1070 tests, 0 warnings. Implementation complete at `28c1a5f`.
**Date:** 2026-06-06
**Author:** AI (investigation + proposal), final decision: Giang Hoang
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Pure `T?` only. EXCLUDES `T~E` / `T?~E` (Outcome requires packed ABI, deferred to Tier C — active guard in `triet-jit/src/mir_lower.rs:758-789`).

---

## Summary

Decides the runtime representation for `T?` in Tier A ("all values = 1×i64"), unblocking the `get(vector, index) -> Integer?` builtin — the initial consumer of nullables in the new backend. This ADR presents **6 alternative approaches** (PA-1 … PA-6) with soundness analyses for each. Following two review rounds, both mentors approved **PA-3c (uniform sentinel)**: `NULL_SENTINEL = i64::MIN` for **all** `T?` — scalar and heap alike. Coupled with: read-shim trap-on-0 (defense-in-depth for heap), canary test N1 tied to `triet_core::Integer::MIN`, and an addendum to ADR-0001 amending both the trit assignment table and the `T??` clause.

## Motivation

1. **`get` is the next gateway.** Vector Tier A (4.3b) has `push`/`len` but NO way to read elements. `get` must be total (never panic — safety contract `feedback_explicit_strictness`: property access 100% safe), meaning it returns `Integer?`. Without a representation for `Integer?`, `get` cannot exist.
2. **ADR-0039 (`?`-family) design-locked but implementation deferred** because "Backend currently cannot lower even `?.`" — all `?`-family operators await this representation.
3. **ADR-0040 §6 flagged the pending debt:** "Nullable String: **representation unfinalized.** Note sentinel-0 conflict (moved-out ≡ null value)." This ADR settles that debt — adopting a decisive solution: uniform MIN so moved-out (0) and null (MIN) **never collide**.

---

## §0 — Verified Facts (Not speculation)

| # | Fact | Location | Design Consequence |
|---|------|----------|-------------------|
| F1 | `Integer` = 27 trits, range `±3_812_798_742_493` ≈ ±2^41.8 | `triet-core/src/integer.rs:39-42` | Carrier i64 is ~4 million times wider than valid range → massive "niche" available for sentinels |
| F2 | `Tryte` range `±9_841`, `Trit` `±1`, `Trilean` 3 values | `triet-core/src/tryte.rs:39-42`, `trit.rs` | All Triet scalars have niches within i64 |
| F3 | `Long` = 81 trits, MAX ≈ 2.2×10³⁸ — **does not fit in i64** | `triet-core/src/long.rs:53-54` | Long was never a valid Tier A value → `Long?` unconditionally deferred |
| F4 | JIT arithmetic is **raw i64**: `BinOp::Add → iadd`, `Mul → imul`, `__triet_pow` uses `wrapping_mul` | `triet-jit/src/mir_lower.rs:1124-1126,1251-1270` | Ternary range is NOT enforced at runtime → niches are unguarded (debt D1, §6.2) |
| F5 | Heap value = 1×i64 body_ptr; moved-out zeroed (M1–M4); `free(0)` = no-op | ADR-0040 §1.3, §2.5; `mir_lower.rs` Drop handler | `free(0)` no-op RETAINED — Dropping moved values must succeed quietly (C4) |
| F6 | Enums compile: `StackSlot` + `EnumLayout` (disc i64@0, payload@8), pattern match works (fixtures 25–32) | `mir_lower.rs:168-173,511-546`; ADR-0037 | Tagged-union machinery is available if two-word repr is chosen |
| F7 | Shim C ABI: fixed signatures `fn_1_0/fn_1_1/fn_2_1` — return **exactly 1×i64** | `triet-driver/src/main.rs:123-141` | Shims cannot return 2-word values; requires out-params if multi-word |
| F8 | Typecheck has `Type::Nullable(Box<T>)` + widening `T ⊂ T?`; separate `Type::Outcome{allow_null_state}` | `triet-typecheck/src/types.rs:44,97-104,165-203` | Frontend ready; only lowering + repr missing |
| F9 | MIR type is **string** (`LocalDecl::ty: String`); `is_copy` default-Move for unknown; canonical `is_vec_type()` in triet-mir (lesson 4.3c) | `triet-mir/src/lib.rs:163-174,2047-2073` | `"Integer?"` falls into default-Move without rules — must have canonical `is_nullable_type()`. **Collision:** `is_vec_type("Vector<Integer>?")` = true (§5.1) |
| F10 | MIR has `OutcomeDiscriminant/OutcomeUnwrap/OutcomeUnwrapError` but JIT **rejects** them (unreachable) | `triet-mir/src/lib.rs:245-274`; `mir_lower.rs:758-789` | Landing spot exists if dedicated statements needed; existing guard not lifted by this ADR |
| F11 | Discriminator semantics LOCKED: `Trit::Positive` = value, `Trit::Zero` = null, `Trit::Negative` = reserved (T?) / error (T?~E) | ADR-0020 §10.1 | Encoding logic must follow these trit poles |
| F12 | Pattern matching `T?` requires explicit arms `~+ binding` / `~0` (E1032) | ADR-0020 §10.4 | Match codegen needs only 2 arms, no pattern widening |
| F13 | `is_copy` returns `true` for bare `"?"` (type-unknown); `is_vec_type` uses `starts_with("Vector<")` — swallowing `"Vector<Integer>?"` | `triet-mir/src/lib.rs:2047-2054` | Helper `is_nullable_type` must be queried BEFORE all other classifiers; bare `"?"` must be pinned as **non-nullable** (§5.1) |

### §0.1 — Note on ADR-0001 vs ADR-0020 Contradiction (Addendum needed)

ADR-0001 (2026-04, original body) contained **two** clauses overridden by later locked ADRs:

1. **Trit assignment table:** ADR-0001 assigned `is_null: -1 = null, +1 = present, 0 reserved "uninitialized"`. ADR-0020 §10.1 (2026-05-17, newer, LOCKED) assigned `+1 = value, 0 = null, -1 = reserved/error`. The two tables conflict; the v0.7.4.3 addendum of ADR-0001 stated "no change to memory layout" but **the trit position of null actually shifted** (−1 → 0).

2. **Clause regarding `T??`:** ADR-0001 "Consequences" stated verbatim: "`T??` is not flattened — two layers are distinct". ADR-0039 (2026-06-05, LOCKED, newer) + C6 of this ADR: "`T??` does not exist — auto-flattened".

ADR-0041 adheres to the newer sources (ADR-0020 §10.1, ADR-0039). **Resolution:** with this ADR approved, an addendum is recorded in ADR-0001 locking both items: (a) trit assignments follow ADR-0020 §10.1; (b) `T??` auto-flattens per ADR-0039.

---

## §1 — Design Constraints

| # | Constraint | Source |
|---|------------|--------|
| C1 | Semantic model of `T?` is **discriminator + payload** (ADR-0001). Sentinel/niche allowed **solely as semantics-preserving optimization** — ADR-0001 explicitly permits: "like Rust niche optimization … does not change semantics" | ADR-0001 |
| C2 | Discriminator poles: `+` = value, `0` = null (per §0.1) | ADR-0020 §10.1 |
| C3 | Tier A ABI: every value is 1×i64; shims return 1×i64 (F7) | ADR-0040 §2.5 |
| C4 | M1–M4 zeroing uses value **0** on heap ptrs; `free(0)` no-op (F5). **Dropping moved variables must be quiet.** | ADR-0040 §1.3 |
| C5 | `is_copy` decided via type-string, default-Move, canonical helpers in triet-mir (F9, lesson 4.3c) | HANDOFF 4.3c |
| C6 | `T??` does not exist — auto-flattened | ADR-0039 |
| C7 | JIT arithmetic DOES NOT wrap to ternary range (F4) — all "out-of-range value impossible" arguments hold only *modulo* existing arithmetic-fidelity debt | F4 |
| C8 | Compiler never panics: unsupported cases → `Err(LowerError)` with span | Track B rule 1 |
| C9 | **Shim trap-on-0:** under PA-3c, 0 = dead value (moved-out / OOM) — never a valid heap value. All shims receiving heap inputs (read or consume) **trap** on 0. Sole exception: `free(0)` **remains no-op** — Dropping dead values must be quiet. | PA-3c (§6.1) |

---

## §2 — Solution Space (All considered options recorded)

### PA-1 — Pure Sentinel/Niche: `T?` = 1×i64, null = out-of-range value of T

- **Scalar** (`Integer?`, `Tryte?`, `Trit?`, `Trilean?`): null = constant `NULL_SENTINEL = i64::MIN`. Valid because the ternary range of all Triet scalars leaves the vast majority of the i64 carrier unoccupied (F1, F2) — sacrificing zero valid values of T → NOT violating the "symmetric range" argument used in ADR-0001 (which applied to native ternary hardware where every n-trit pattern is meaningful; Tier A's i64 carrier is not).
- **Heap** (`String?`, `Vector?`): null = ptr 0 (precedent Rust `Option<Box<T>>` null-niche).

**Key property:** widening `T ⊂ T?` and constructor `~+ e` are **identity** — zero codegen. `let x: Integer? = 5` is simply the integer 5. Function boundaries, shim ABI, M1–M4, borrowck — all remain unchanged as 1×i64.

| Pros | Cons |
|------|------|
| Zero new machinery in JIT/ABI; `get` shim returns direct 1×i64 | **D1:** raw i64 arithmetic (C7) can theoretically generate `i64::MIN` → phantom null (§6.2) |
| Widening/`~+` = no-op | Heap: null 0 ≡ moved-out 0 → loses defense-in-depth distinction (§6.1) |
| `Integer?` crosses user-fn boundaries freely (Copy, 1×i64) | Each kind needs distinct sentinel (heap 0, scalar MIN) — lowerer needs type knowledge at `~0` |
| Match/Elvis = 1 comparison | Debugger sees `-9223372036854775808` instead of "null" — poor ergonomics |

### PA-2 — Tagged Two-word: `T?` = 2-variant intrinsic enum via ADR-0037 machinery

Lowerer synthesizes `EnumLayout` for each `T?`: disc i64@0 (value **+1/0** per C2 — `EnumLayout.discriminant_value: i64` allows arbitrary assignment, not restricted to 0/1/2), payload i64@8, living in `StackSlot` like standard enums (F6).

| Pros | Cons |
|------|------|
| **No D1 debt** — disc is distinct, independent of range | `T?` cannot cross call boundaries (enum param/return not yet supported — open item "sret-like by-pointer") → requires new B-refusal for `T?` user-fn param/return |
| Match `~+/~0` falls out almost free from enum match codegen | `get` shim cannot return 2-word values (F7) → requires out-param: `__triet_vector_get(vec, idx, out_ptr) -> disc`, adding arity `fn_3_1` |
| Strict fidelity to ADR-0001 (physical disc exists) | `String?` hits B8 barrier (enum payloads of Move-type rejected) → requires lifting B8 or hybridizing |
| Disc uses trit poles +1/0 → ready for `T?~E` adding −1 pole later | Widening/`~+` becomes stack-slot stores; Copy semantics of `Integer?` requires copying 16-byte slots |

### PA-3 — Hybrid by Kind (Per-type niche selection — matching Rust layout strategy)

Partitioned by payload nature:

- **PA-3a (hybrid 2-sentinel):** heap `T?` = null-ptr 0; scalar `T?` = sentinel `i64::MIN`. Each kind has its own sentinel — lowerer must inspect type to choose the right sentinel at `~0`. **Rejected by both mentors (Q3).**

- **PA-3b (zero-debt):** heap `T?` = null-ptr 0; scalar `T?` = enum-slot PA-2. Zero D1 debt, at the cost of: scalar nullables trapped inside functions (new B-refusal) + out-param shims. **Rejected:** paying real implementation cost for throwaway code discarded when Tier C packed ABI arrives.

- **PA-3c (uniform sentinel — CHOSEN):** `NULL_SENTINEL = i64::MIN` for **all** `T?` — scalar and heap. Moved-out remains 0 (C4) → null (MIN) and dead (0) are permanently distinct. Advantages over PA-3a: (i) uniform → `~0` lowering is type-agnostic and simple; (ii) defense-in-depth for heap — 0 is a dead value, and read-shims trap upon receiving it. Detailed in §5–§6.

### PA-4 — Boxed Nullable: all `T?` = heap object {disc, payload}, ptr-or-0

**Rejected immediately:** allocation on every `Integer?`; `Integer?` becomes a Move type (breaking Copy expectations of scalars, infecting borrowck); requires Drop machinery for scalar nullables; gratuitously slow. Offers zero advantages over PA-2.

### PA-5 — SSA Pair Splitting: lowerer splits `T?` into 2 MIR locals (disc, payload)

Everything remains i64 Variables (no StackSlots). Lowerer manages local pairs as a logical unit.

| Pros | Cons |
|------|------|
| No StackSlots, no sentinels, no D1 | MIR `Place`/`Assign` is 1-place — each nullable assignment becomes 2 statements; **all compiler passes must know the pair is unified** |
| Low JIT codegen cost | Borrowck: 2 locals = 1 logical variable → E2420 reporting, VarState, local_names all become pair-aware — deeply invasive |
| | Call boundaries still blocked (2 values); shims still require out-params |
| | Premature scalar replacement of aggregates — 3-crate complexity for a single type |

### PA-6 — Avoid Nullables: ship `get_or(vec, i, default)` / `has(vec, i)` first

**Rejected:** (1) roadmap established "nullable repr → get"; (2) `get -> T?` is the SPEC-aligned shape, `get_or` is a temporary API requiring deprecation — violating "stability over speed"; (3) leaves ADR-0039 blocked.

---

## §3 — Comparison Matrix

| Criterion | PA-1 | PA-2 | PA-3c (CHOSEN) | PA-3b | PA-4 | PA-5 |
|---|---|---|---|---|---|---|
| New soundness debt | D1 | 0 | D1 | 0 | 0 | 0 |
| New machinery (lower/JIT) | ~0 | medium | small | medium | large | large (3 crates) |
| `get` shim | direct i64 | out-param fn_3_1 | direct i64 | out-param | returns ptr | out-param |
| `Integer?` across user-fn | ✅ free | ❌ refuse | ✅ | ❌ refuse | ✅ (but Move!) | ❌ |
| `String?` | ✅ (0-niche) | ❌ B8 | ✅ (MIN-niche) | ✅ | ✅ | ⚠️ |
| Elvis `?:` | sentinel compare | reuse enum match | MIN compare | mixed | load+compare | compare disc local |
| Widening `T ⊂ T?` | **no-op** | slot store | **no-op** | mixed | allocate! | copy disc+payload |
| Defense-in-depth heap moved≢null | ❌ | n/a | ✅ (trap-on-0) | ❌ | ❌ | ✅ |
| `~0` lower needs type? | yes (2 sentinels) | no | **no** (uniform) | yes | no | no |
| Path to Tier C packed ABI | sentinel→packed | repr near packed | sentinel→packed | mixed | discard | discard |

---

## §4 — Final Decision (Consensus of both mentors after 2 review rounds)

**Chosen Approach: PA-3c (uniform sentinel).** `NULL_SENTINEL = i64::MIN` for all `T?` — scalar and heap.

| # | Question | Outcome | Mentor |
|---|----------|---------|--------|
| Q1 | Accept D1 (phantom-null bounded by arithmetic debt)? | **Accepted.** §6.2 rationale holds: all paths to `i64::MIN` traverse programs that are already invalid per SPEC (F4), and D1 disappears naturally when arithmetic wraps mod-3²⁷. Condition: satisfy 3 obligations in §6.2 + canary N1. | G ✓, O ✓ |
| Q2 | Sentinel: `i64::MIN`? | **`i64::MIN`.** Canary N1 tied to `triet_core::Integer::MIN` — non-negotiable from G, co-signed by O. No duplicate hardcoding. | G ✓, O ✓ |
| Q3 | Heap null: 0 (PA-3a) or `i64::MIN` (PA-3c)? | **PA-3c — uniform `i64::MIN`.** G's conclusion prevails; defense-in-depth powered by trap-on-0 (§6.1). Obligation: all read-shims trap on 0; `free(0)` remains no-op. | G ✓, O ✓ |
| Q4 | Match `~+/~0` in Tier A, or Elvis `?:` only? | **Elvis + widening + `~0` only.** Match → Tier B. Scope discipline: verify `get(v,2) ?: -1` statically and dynamically first. O yields to G. | G ✓, O ✓ |
| Q5 | ADR-0001 addendum? | **Yes.** Correct both the trit assignment table (per ADR-0020 §10.1) and the `T??` clause (auto-flatten per ADR-0039) in a single update. | G ✓, O ✓ |

---

## §5 — Detailed Specification (PA-3c Uniform Sentinel)

### 5.1 — Constants + canonical helper (triet-mir, single source of truth — lesson 4.3c)

```rust
// triet-mir/src/lib.rs
/// Sentinel encoding the `~0` (null) state of **all** `T?` at Tier A
/// (scalar and heap — uniform). INVARIANT: lies outside every Triet
/// scalar range (see canary test N1, §9).
pub const NULL_SENTINEL: i64 = i64::MIN;

/// `"Integer?"` → true. Canonical — all crates use this; ad-hoc
/// `ends_with("?")` is prohibited (matching `is_vec_type` pattern).
///
/// **Ordering rule:** is_nullable_type MUST be called BEFORE any other
/// type-string classifier (is_vec_type, etc.) at every consumer.
/// Reason: `"Vector<Integer>?"` starts with `"Vector<"` and would be
/// misclassified as a bare Vector by `is_vec_type`.
///
/// **Pin:** `is_nullable_type("?")` MUST return `false`. The bare `"?"`
/// type-string means "type unknown" (is_copy treats it as Copy) — it
/// must NOT be classified as "nullable of empty string."
pub fn is_nullable_type(ty: &str) -> bool;

/// `"Integer?"` → `Some("Integer")`; non-nullable → `None`.
///
/// **Pin:** `nullable_payload("Vector<Integer>?")` MUST return
/// `Some("Vector<Integer>")` — `is_vec_type` must NOT consume
/// a nullable type-string. Verify in N2.
pub fn nullable_payload(ty: &str) -> Option<&str>;
```

Type-string format: `<payload>?` — `"Integer?"`, `"String?"`. (Outcome type-strings do not yet exist in MIR and are NOT defined here.)

`is_copy` adds a branch **before** default-Move and **before** all other classifiers:

```text
is_nullable_type(ty) → nullable_payload(ty) = Some(p) → is_copy(p, body)
```

→ `"Integer?"` is Copy, `"String?"` is Move. `"String?"` as Move ⇒ existing B7/B8 refusals **automatically** apply (zero new code).

### 5.2 — Sentinel (uniform)

| Kind | Null encoding | Notes |
|------|---------------|-------|
| **All `T?`** (scalar + heap) | `NULL_SENTINEL` (`i64::MIN`) | Uniform — `~0` lowering is type-agnostic |
| `Long?` | — | **Refuse** (`Err`): Long does not exist in Tier A (F3) |

Consequences of uniform sentinel:

- **0 is never null.** 0 = dead value (moved-out or OOM). Under PA-3c, valid heap pointers are never 0 (null is MIN, not 0).
- **`~0` lowering is type-agnostic:** always `iconst(i64::MIN)`, without needing payload type inspection.
- **Elvis null-check:** `icmp eq val, i64::MIN` — single comparison, type-independent.

### 5.3 — Lowering per construct

| Construct | Lowering | Notes |
|---|---|---|
| `~0` (`Expr::NullLiteral`) | `iconst(NULL_SENTINEL)` | Type-agnostic (always MIN). **Still requires expected type** to verify this is `T?` rather than `T~E` (Outcome guard). Missing expected type → `Err(LowerError)` (C8). Supported Tier A positions: `let x: T? = ~0`, `return ~0` from function returning `T?`. Other positions → `Err`. |
| Widening `let x: Integer? = e` | identity — standard copy | No additional codegen |
| `~+ e` | identity | Same as widening; exists for syntactic symmetry |
| `e ?: default` (Elvis) | `brif(icmp eq e, NULL_SENTINEL)` → branch default / branch e | **Branch, not `select`** — RHS of Elvis is a full Expression including blocks/returns (ADR-0039 clause 2) with side-effects, cannot be eagerly evaluated |
| `match m { ~+ x => …, ~0 => … }` | **NOT lowered in Tier A** → `Err` | Deferred to Tier B (Q4). Under PA-3c it is simple compare+branch — added after Elvis + get are established |
| `?.`, `?+>`, `.unwrap_value(msg)` | **NOT lowered in Tier A** → `Err` | Scope-out §7 |

### 5.4 — `get` builtin

```text
Typecheck prelude:  get(Vector, Integer) -> Integer?     (overload infra 4.3b)
Shim:               __triet_vector_get(vec: i64, idx: i64) -> i64
                    borrow, copy → scalar-or-sentinel
                    idx < 0 || idx >= len  →  NULL_SENTINEL
                    otherwise              →  data[idx]
BUILTIN_SHIM_META:  arg_consumes = [false, false]        (borrows vec — DOES NOT consume,
                                                          unlike push; vec remains usable)
```

- Total function: negative indices / out-of-bounds → `~0`, **never panics** — fulfilling the "property access 100% safe" contract.
- `fn_2_1` available (F7) — no new arity required.
- `get` on `Vector<String>` does not exist yet (Vector Tier A is monomorphic `Vector<Integer>`, 4.3b) → avoids returning heap nullables from shims for now.

### 5.5 — Drop / borrowck / trap-on-0

**Philosophy:** *"reading dead values → explode; dropping dead values → quiet."*

- **Drop(`"Integer?"`)**: Copy → no-op (existing branch).
- **Drop(`"String?"`)**: Move. Lowerer emits NO additional branch instructions. It calls `__triet_string_free(val)` unconditionally. The free shim checks both 0 (dead value) and MIN (null value), treating both as no-ops. Keeps codegen simple and complies with Track B rule 4 (no dead codegen for heap-nullables before producers exist).
- **Shim trap-on-0 (C9):** All shims accepting heap values as input (both read and consume: `__triet_string_len`, `__triet_vector_push`, `__triet_vector_get`, …) **trap** on receiving 0: `if val == 0 { SIGABRT }`. 0 = dead value — never a valid heap pointer under PA-3c. Sole exception is the free shim. Defense-in-depth: if borrowck suffers a soundness bug leaking a read-after-move, the program aborts noisily rather than silently reading garbage.
- **Borrowck unchanged:** extending `is_copy` is sufficient. `VarState`/E2420/E2450 apply identically to `String?`.

---

## §6 — Soundness Analysis

### 6.1 — Defense-in-depth: trap-on-0 (Why PA-3c over PA-3a)

`places_conflict(a, b, conservative=true)` in `triet-borrowck/src/checker.rs:64-66`: when `conservative=true` and `a.local != b.local`, the function returns `true` (conflict) — resulting in **over-rejection** (safe programs rejected with E2440), **not leakage** (soundness hole). Borrowck has teeth: E2420 verified across 8/8 fixtures, E2450 Drop-while-borrowed works.

However, E2420/VarState is young (2 phases old). PA-3a chose heap null = 0, meaning: if borrowck ever suffered an under-rejection bug, programs would see null silently instead of crashing. **PA-3c provides cheap insurance:**

1. **Uniform MIN:** null = `i64::MIN` for all types → 0 is never null. 0 = dead value (moved-out or OOM).
2. **Read-shim trap-on-0:** all read-shims encountering 0 → SIGABRT. Borrowck bugs abort loudly.
3. **`free(0)` remains no-op:** Dropping moved values must be quiet (C4). Consistent philosophy: read dead → explode; drop dead → quiet.
4. **No penalty on null-checks:** null-checks in Elvis/`~0`/match test ONE magic value (`i64::MIN`). Only free shims guard two values (0 and MIN), and the 0 guard already existed.

### 6.2 — Scalar: Debt D1 (phantom null via non-wrapping arithmetic)

**Formal statement:** under PA-3c, a sequence of raw-i64 operations (F4) could theoretically generate bit-pattern `i64::MIN` and be misinterpreted as `~0` when stored into an `Integer?` slot.

**Why bounded:**
1. To reach `i64::MIN` using `iadd/imul` from valid literals, intermediate values must exit the valid range `±3.8×10¹²` — an `imul` of two valid Integers (3.8e12 × 3.8e12 ≈ 1.45×10²⁵) overflows i64 in a single step. The program has ALREADY violated SPEC mod-3²⁷ semantics, producing mathematically incorrect results. D1 merely alters the *form* of the error.
2. In-range values never collide with the sentinel: distance from valid range to `i64::MIN` is ~2^62.
3. **Natural roadmap exit:** when Tier B arithmetic wraps correctly mod-3²⁷, all runtime values stay in range → niche is enforced → D1 closes permanently without representation changes.

**Obligations upon approving PA-3c:** (a) record D1 in debt register; (b) canary test N1 binding `NULL_SENTINEL` outside triet-core range (§9); (c) comments at `NULL_SENTINEL` pointing to §6.2.

### 6.3 — What this ADR does NOT weaken

- JIT guard rejecting `OutcomeDiscriminant/Unwrap/UnwrapError` (F10) **remains in place** — `T?` in Tier A bypasses those statements (comparing directly in Elvis codegen).
- `T??` remains non-existent (C6) — blocked by typecheck.

---

## §7 — Tier A Scope (IN / OUT)

| IN | OUT (defer) | Deferred to |
|----|-------------|-------------|
| `~0` literal (type-agnostic, requires expected type in supported positions) | `?.` safe call, `?+>` map/flatMap (ADR-0039) | Tier B |
| Widening `T ⊂ T?`, `~+ e` | `.unwrap_value(message)` (requires trap + message ABI) | Tier B |
| `get(vector, index) -> Integer?` | `match` 2-arm `~+/~0` (Q4) | Tier B |
| `e ?: default` (Elvis, branch-based) | `T~E` / `T?~E` Outcome (packed ABI) | Tier C |
| `String?`/`Vector?` **repr defined** (null=MIN) | `Long?` (F3) | Tier B+ |
| | … **heap-nullable wiring deferred**: no Tier A producer returns `String?` (`get` is `Integer?`) — repr defined for specification completeness without shipping dead code (Track B rule 4) | when producer lands |

---

## §8 — Migration Path

| Milestone | Task | Repr Change? |
|-----------|------|--------------|
| Tier B: arithmetic wrap mod-3²⁷ | D1 closes naturally | No |
| Tier B: real refcounting (`&+` lowering) | Heap nullable: null=MIN → not 0, refcount logic untangled | No |
| Tier B: match `~+/~0` | Add compare+branch — mechanical | No |
| Tier C: packed Outcome ABI (`T~E` 2-word) | `T?` MAY migrate to packed for uniformity with `T?~E`, or retain niche fast-path (Rust keeps both) — decided via new ADR | Possible, localized |

---

## §9 — Verification Plan

### Fixtures (40+)

| # | Fixture | Expectation |
|---|---------|-------------|
| 40 | `get` in-bounds: push 3 elements, `get(v,1) ?: -1` | EXPECT: 42 |
| 41 | `get` out-of-bounds: `get(v,99) ?: -1` | EXPECT: -1 |
| 42 | `get` negative index: `get(v,-1) ?: 7` | EXPECT: 7 |
| 43 | Elvis with widening: `let x: Integer? = 5; x ?: 0` | EXPECT: 5 |
| 44 | widening + `~0` across return: fn returns `Integer?`, caller Elvis | EXPECT: 7 |
| 45 | N6: `Integer?` narrowing rejected (using `Integer?` where `Integer` required) | ERROR: type mismatch |
| 46 | E2420: using `get` as use-site after vector move (push then get) | ERROR: E2420 |
| 47 | `get` does not consume vec: get then `len(v)` succeeds | EXPECT: 3 |

### Teeth (Fails when wrong, not just passes when right)

| What is removed | Which test MUST FAIL |
|-----------------|---------------------|
| Bounds-check in `__triet_vector_get` | 41/42 (reads garbage or crashes) |
| Sentinel comparison in Elvis codegen | 41 (returns raw sentinel instead of -1) |
| `nullable_payload` branch in `is_copy` | unit: `"String?"` must be Move — test both `"String?"` (Move) and `"Integer?"` (**Copy**) |
| Trap-on-0 in shims | unit: calling `__triet_string_len(0)` or `__triet_vector_push(0, x)` → SIGABRT (not returning 0) |

### Unit / Invariants

| # | Invariant | Verification Method |
|---|-----------|---------------------|
| N1 | Canary: `NULL_SENTINEL < Integer::MIN.0` && outside Tryte/Trit/Trilean range — tied to triet-core constants | unit test in triet-mir |
| N2 | `is_nullable_type`/`nullable_payload` round-trip; `is_nullable_type("Vector<Integer>?")` = true and `nullable_payload("Vector<Integer>?")` = `Some("Vector<Integer>")` — `is_vec_type` does not swallow; `is_nullable_type("?")` = false (bare "?" pin); `"Integer??"` handled cleanly | unit |
| N3 | `~0` without expected type → `Err(LowerError)` with span, no panic | lowerer unit |
| N4 | `get` arg_consumes [false,false]: borrowck DOES NOT mark vec Moved | borrowck unit (contrast with push) |
| N5 | `~0` lowering type-agnostic: identical `iconst(i64::MIN)` for `Integer?` and `String?` | lowerer unit |
| N6 | Typecheck: using `Integer?` where `Integer` expected (reverse widening) rejected | typecheck unit |
| N7 | Shim trap-on-0: `__triet_string_len(0)`, `__triet_vector_push(0, _)` → SIGABRT; `__triet_string_free(0)` → no-op | unit in harness |

---

## §10 — Related ADRs / Documents

| Document | Relationship |
|----------|--------------|
| ADR-0001 | Semantic model disc+payload; permits niche-as-optimization; **requires addendum amending trit assignments + `T??` clause** (§0.1, Q5) |
| ADR-0020 §10 | Trit poles `+/0/−`, canonical `~0`, E1032 match arms — semantics baseline |
| ADR-0037 | Enum StackSlot machinery — used if PA-2/PA-3b won review (not chosen) |
| ADR-0039 | `?`-family awaits this repr; Elvis RHS = Expression (mandating branch-based lowering §5.3); `T??` auto-flattens (C6) |
| ADR-0040 | §2.5 ABI 1×i64; §1.3 M1–M4; §6 flags sentinel-0 conflict — resolved by uniform MIN |
| `feedback_explicit_strictness` | `get` is total returning `T?`, never panics |
| `spec/plans/REPORT-2026-06-04.md` | D1 debt recording location |

---

## §11 — 5-minute Review Summary

1. **PA-3c (uniform sentinel):** `NULL_SENTINEL = i64::MIN` for all `T?` — scalar and heap. Uniform → `~0` lowering is type-agnostic.
2. **D1 (phantom null):** bounded by EXISTING arithmetic-fidelity debt (F4), resolves naturally when Tier B wraps mod-3²⁷. Accepted by both mentors.
3. **Trap-on-0 (defense-in-depth):** under uniform MIN, 0 = dead value — never null. All read-shims trap on 0 → borrowck bugs become SIGABRT. `free(0)` remains no-op → Dropping moved-out values stays quiet.
4. **Tier A Scope:** widening + `~0` + Elvis `?:` + `get`. Match `~+/~0` → Tier B.
5. **R1 (type-string collision):** `is_nullable_type` must be queried BEFORE other classifiers; bare `"?"` pinned as non-nullable — recorded in §5.1.
6. **Addendum ADR-0001 (R2):** amend both trit assignment table and `T??` clause simultaneously — §0.1.
7. **Implementation Order:** canary N1 + helpers → widening + `~0` + Elvis → `get` + fixtures 40–46. Register shims in driver and harness.

---

## §12 — Addendum (2026-06-07, Tier B slice a)

Match `~+/~0` 2-arm for `T?` (deferred from Tier A Q4) is implemented in Tier B.

- **E1026** extended: exhaustiveness check for `Type::Nullable` — requires `~+` (present) and `~0` (null), or wildcard `_`. Shares `NonExhaustiveOutcomeMatch` with `Type::Outcome`, generic message: "non-exhaustive match: missing arm(s) …".
- **E1035** allocated: `NegativeArmOnNullable` — rejects `~-` on `T?` (nullable lacks error state, `~-` valid only on `T~E` / `T?~E`).
- **Lowering:** branch-based comparison against `NULL_SENTINEL` (Elvis pattern), 3 guards (duplicate arm, wildcard-last, sub-pattern Variable/Wildcard) ensure slot-model ≡ first-match-wins.
- **Fixtures:** 48–57 (10 fixtures: present, null, wildcard, wildcard-fallback×2, non-exhaustive E1026, `~-` rejection E1035, wildcard-not-last, duplicate-`~+`, literal-subpattern).

---

## §AMEND-Slice2c (2026-07-27, ADR-0089 Slice 2c) — `!!` ForceUnwrap Tier-A Lowering

`!!` (force-unwrap on `T?` — historical primitive per ADR-0020 §hist, operator family per ADR-0039) lowers **isomorphic to Elvis `?:`** (§5 PA-3c identity, lowerer `lib.rs Expr::ElvisOp`). Front-half (lexer `BangBang`, parser `Expr::ForceUnwrap`, typecheck `check_force_unwrap`) was already wired; this amendment records the MIR lowering (`lib.rs Expr::ForceUnwrap` arm):

1. **Trap-on-Null.** operand `== NULL_SENTINEL` → `Terminator::Trap` (SIGILL at runtime). No fallback, no UB, no silent garbage. The null branch has **no merge edge** — control never returns from it.
2. **Present = PA-3c identity.** `result = operand` (single-i64 repr). For non-Copy heap-scalar (`String?`/`Vector?`/`HashMap?`) the `Assign` consumes the operand ⇒ borrowck marks the source `Moved` (checker.rs Δ1) ⇒ reuse of a named-local operand is **E2420 UseAfterMove** — this is the signature that proves `!!` is a move-out, eliminating the alias-double-free hazard. Copy scalar (`Integer?`/`Trit?`/`Trilean?`) is non-consuming.
3. **Scope Tier-A (Slice 2c).** Scalar + heap-scalar (single-slot repr) ONLY. Non-Copy Aggregate (`Struct?`/`Enum?`, multi-field sret) is **refused via E1100** (`unsupported_expr`) until ownership projection lands. Fence predicate keys off `matches!(payload, MirType::Struct(_) | MirType::Enum(_))` — verified independently that `String`/`Vector`/`HashMap` are **distinct MirType variants** (mir/lib.rs:490/530/532), NOT `Struct("String")`, so they pass the fence naturally; an `is_string_repr()` belt is redundant-but-harmless (implementer's choice).
- **Fixtures:** 495–504 (present ×4 scalar+heap, Trap-on-null ×2, rvalue-temp no-leak, canary E2420 move-out, fence E1100 Struct?/Enum? ×2) + counting teeth.

Sign-off: O ✅ (2026-07-27, independent verification: poison Trap→trap teeth RED, poison fence→corpus 501/502 RED, canary E2420, MIR dump `move _0`, gate 0·clean·0·496·0) / G ✅ (2026-07-27, independent verification: counting teeth clean, null trap SIGILL clean, corpus 496 clean)
