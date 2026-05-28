# ADR 0028 — Atomic Primitive Design (refines ADR-0026 v2 §4)

**Trạng thái:** **Locked** (v0.9.0.1, author sign-off 2026-05-29). Refines [ADR-0026 v2 §4](0026-actor-boundary-send-rules.md) placeholder. Author confirmed 3 architecturally-significant decisions: §1 builtin shim strategy; §5 ownership ref form fix (resolves ADR-0026 v2 §4.3 contradiction); §10 conservative E2530 enforcement.

**Issue:** ADR-0026 v2 §4 placeholder-locked Atomic primitive type family (`Atomic<Integer/Tryte/Trit/Trilean/Pointer>`) + Ordering enum (Relaxed/Synchronized/Strict) + skeleton API surface (load/store/swap/compare_exchange) + E2530 sketch. Open questions left for ADR-0028:

1. **Implementation pattern** — VM opcodes vs Rust-shim builtins (per [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md))?
2. **Ordering ↔ Trit mapping** — which polarity maps to which level?
3. **Full operation set** — fetch_add/sub/and/or/xor + type-specific ops (e.g., Trilean Ł3 atomic ops)?
4. **`AtomicValue` trait** — marker or with methods? Which types qualify?
5. **Reference form for store/swap/compare_exchange** — ADR-0026 v2 §4.3 wrote `&+ mutable Atomic<T>` for store, but cross-thread atomic share requires `&+` frozen (per §2.1 row 7 Send rule). This is a **contradiction** ADR-0028 must resolve.
6. **Constructor** — function form? Trait method? Auto-init?
7. **VM dev tier behavior** — single-thread VM has no real concurrency; how do atomic ops behave?
8. **Capability boundary** — when `sys.atomic` cap required vs `sys.atomic` only for non-default ordering?
9. **E2530 InvalidAtomicOrdering fire conditions** — when does compiler refuse Relaxed?
10. **Stdlib `sys.atomic.*` module shape** — top-level functions or methods on `Atomic<T>`?

ADR-0028 locks decisions for §1-§9; §10 ships in conservative form with refinement deferred to corpus exposure (per ADR-0025 enforcement-needs-corpus precedent).

---

## §1 — Implementation strategy: Rust-shim builtins (per ADR-0019 §5)

**Decision:** Atomic operations are **Rust-shim builtins** in the VM dispatcher, NOT new IR opcodes. Sits at opcode IDs 27-39 (next available after v0.7 builtins 4-26 per ADR-0019 §5).

**Rationale:**

- **Pattern consistency.** ADR-0019 §5 established Rust-shim for Vec/HashMap/file IO/path/string ops. Atomic follows same shape — semantic operation backed by Rust impl, exposed via stable opcode ID in `.triv` v6 wire format.
- **VM dev tier feasibility.** VM is single-thread per [VISION §4.3](../../VISION.md). Atomic ops can be implemented as plain reads/writes; Ordering is no-op semantically until real thread integration (v0.10+ stdlib). Shim approach makes this trivial — `Ordering::Relaxed` does identical work as `Ordering::Strict` on single-thread VM.
- **AOT / JIT lowering clarity.** When v2.0 LLVM AOT lands, builtin IDs lower 1:1 vào LLVM atomic intrinsics (`@llvm.atomicrmw.add`, etc.). When v∞ trytecode native ships, builtins lower vào ternary atomic ISA.
- **Future-proof.** Adding new operations (e.g., `fetch_min`/`fetch_max` v0.10+) = new builtin ID, no IR opcode churn.

**Wire format:** `.triv` v5 → v6 patch bump (ADR-0028 §1 lock). Pre-v6 readers see new builtin IDs and refuse với `UnknownOpcode` per ADR-0010 backward-compat rule.

**Alternative rejected:** Dedicated `Atomic` IR opcode family (~15 new opcodes). Pros: more explicit at IR inspection. Cons: violates ADR-0019 §5 builtin pattern; couples IR to specific operation set; harder to extend.

---

## §2 — Type family + `AtomicValue` trait

**Decision:** `Atomic<T>` parameterized on `T: AtomicValue` where `AtomicValue` is a **marker trait** (no required methods). Compiler enforces membership at type-check.

**Members of `AtomicValue` (locked v0.9):**

| Type | Bit width on binary CPU | Trit width on ternary | Operation set |
|---|---|---|---|
| `Trit` | 8 bits (atomic byte) | 1 trit | load / store / swap / compare_exchange |
| `Tryte` | 16 bits | 9 trits | load / store / swap / compare_exchange / fetch_add / fetch_sub |
| `Integer` | 64 bits (atomic u64) | 27 trits | load / store / swap / compare_exchange / fetch_add / fetch_sub / fetch_and / fetch_or / fetch_xor |
| `Long` | NOT atomic-able | 81 trits | — (exceeds hardware atomic width) |
| `Trilean` | 8 bits | 1 trit (`{-1,0,+1}` = `{false, unknown, true}`) | load / store / swap / compare_exchange |
| `Pointer` | 64 bits (`usize`) | 27 trits | load / store / swap / compare_exchange — **requires `dev.raw_memory` capability** |

**Note:** `Long` (81-trit) explicitly excluded — exceeds hardware atomic width on both 64-bit binary CPUs and forecast trytecode hardware. User wanting atomic 81-trit value must use Mutex (planned `std.concurrency.Mutex` v0.10) or split into 3× 27-trit Atomic<Integer> with manual ordering.

**Marker trait declaration:**

```triet
public trait AtomicValue {}
// Implementations: provided by compiler intrinsic, not user-defined.
```

Users **cannot** implement `AtomicValue` for custom types per v0.9 lock (no struct Atomic, no enum Atomic, no nested `Atomic<Atomic<T>>`). Future ADR can extend if corpus demands.

**Trit/Trilean ops note:** Trit is a 3-state number ({-1, 0, +1}); Trilean is 3-state truth value. Bitwise `fetch_and`/`fetch_or`/`fetch_xor` only make sense for binary types (Tryte/Integer). Trit + Trilean get the safe minimum: load/store/swap/compare_exchange.

---

## §3 — `Ordering` enum + Trit mapping

**Decision:** `Ordering` is a 3-variant enum mapping into `Trit` polarity per Triết identity rule:

```triet
public enum Ordering {
    Relaxed,        // Trit::Negative (-1) — weakest
    Synchronized,   // Trit::Zero      ( 0) — middle
    Strict,         // Trit::Positive (+1) — strongest
}
```

**Mapping rationale:**

- **Polarity = strength.** Negative = relaxed/weak; Zero = neutral/middle; Positive = strict/strong. Matches Triết's `&+`/`&0`/`&-` ownership polarity convention.
- **C++ equivalence:**
  - `Relaxed` ≡ `memory_order_relaxed` (no synchronization, atomic only).
  - `Synchronized` ≡ `memory_order_acq_rel` (acquire on load, release on store).
  - `Strict` ≡ `memory_order_seq_cst` (total order across all threads).
- **C++ 5-level → Triết 3-level:** `Consume` and `Acquire` merge into `Synchronized`. Consume is rarely usefully distinct from Acquire in practice (most compilers lower Consume → Acquire anyway). Kernel writers needing finer control go through `dev.raw_memory` capability to use raw hardware intrinsics.

**Default for store/swap/compare_exchange (no explicit ordering):** `Ordering.Synchronized`. Strong default — covers 95% use cases safely. Author opts into `Relaxed` explicitly (signaling intentional weakness).

**Default for load (no explicit ordering):** `Ordering.Synchronized`. Same reasoning.

**Default for fetch_add/sub/and/or/xor:** `Ordering.Synchronized`.

---

## §4 — API surface (full operation set)

**Decision:** Functions exposed via stdlib `sys.atomic.*` module (NOT methods on `Atomic<T>` — see §8 for rationale). Each function takes an explicit `Ordering` argument; default-ordering overloads provided for ergonomics.

### 4.1 — Universal operations (all `AtomicValue` types)

```triet
// Load value from atomic. Caller must have read access (any ref form).
public function load<T: AtomicValue>(self: &+ Atomic<T>, ordering: Ordering) -> T
public function load<T: AtomicValue>(self: &+ Atomic<T>) -> T   // defaults Synchronized

// Store value into atomic. Atomicity is internal — owner ref is &+ frozen.
public function store<T: AtomicValue>(self: &+ Atomic<T>, value: T, ordering: Ordering) -> Unit
public function store<T: AtomicValue>(self: &+ Atomic<T>, value: T) -> Unit

// Swap atomic with new value, return previous.
public function swap<T: AtomicValue>(self: &+ Atomic<T>, value: T, ordering: Ordering) -> T
public function swap<T: AtomicValue>(self: &+ Atomic<T>, value: T) -> T

// Compare-exchange. Returns ~+ previous_value if expected matched and replaced;
// ~- CompareExchangeFailed { actual: T } if expected did NOT match (no replace).
public function compare_exchange<T: AtomicValue>(
    self: &+ Atomic<T>,
    expected: T,
    new_value: T,
    success_ordering: Ordering,
    failure_ordering: Ordering,
) -> T~CompareExchangeFailed

public function compare_exchange<T: AtomicValue>(
    self: &+ Atomic<T>,
    expected: T,
    new_value: T,
) -> T~CompareExchangeFailed   // both default Synchronized
```

### 4.2 — Numeric arithmetic (Tryte / Integer only)

```triet
public function fetch_add(self: &+ Atomic<Integer>, delta: Integer, ordering: Ordering) -> Integer
public function fetch_add(self: &+ Atomic<Integer>, delta: Integer) -> Integer
public function fetch_sub(self: &+ Atomic<Integer>, delta: Integer, ordering: Ordering) -> Integer
public function fetch_sub(self: &+ Atomic<Integer>, delta: Integer) -> Integer

// Same overloads cho Atomic<Tryte>:
public function fetch_add(self: &+ Atomic<Tryte>, delta: Tryte, ordering: Ordering) -> Tryte
public function fetch_sub(self: &+ Atomic<Tryte>, delta: Tryte, ordering: Ordering) -> Tryte
```

All `fetch_*` return the **previous** value (pre-modification). Overflow: per balanced ternary §3.2 (no overflow within range; out-of-range = E2010 RuntimeOverflow).

### 4.3 — Bitwise (Integer only — Tryte excluded because 9-trit width clashes với binary atomic intrinsics)

```triet
public function fetch_and(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
public function fetch_or(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
public function fetch_xor(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
```

Note: bitwise on balanced ternary is semantically odd (Triết is ternary-first; "bitwise" is binary-CPU lowering detail). These ops are escape hatches for FFI scenarios where Atomic<Integer> stores a packed binary value. **Future ADR may add ternary-native ops (`fetch_trit_and` Ł3-semantic) — defer until corpus shows demand.**

### 4.4 — Trit/Trilean — load/store/swap/compare_exchange only

No `fetch_*` ops for Trit or Trilean per §2 type table. Use compare_exchange loop for transitions.

---

## §5 — Reference form for atomic operations (RESOLVES ADR-0026 v2 §4.3 contradiction)

**Author review required.**

**Issue:** ADR-0026 v2 §4.3 wrote `store(self: &+ mutable Atomic<T>, ...)`. But cross-thread atomic share REQUIRES `&+` frozen (per §2.1 row 7 Send rule — `&+` Send via refcount-mediated share, but `&+ mutable` is exclusive move-only). Contradiction: cannot have atomic that is BOTH cross-thread-shared AND mutable-via-exclusive-borrow.

**Decision (this ADR):** All `Atomic<T>` operations take `&+ Atomic<T>` (frozen owner). Atomicity is **internal interior mutability** — implementation uses raw hardware atomic instructions to mutate without violating owner immutability. Mirrors Rust's `&AtomicU64` (shared borrow) + interior mutation pattern.

**Implication:** `&+ Atomic<T>` is the canonical handle. Many threads can hold `&+ Atomic<T>` simultaneously (refcount-mediated share). Each thread can call store/swap/fetch_* on it; race conditions resolved by Ordering semantics, not by Triết's borrow checker (atomic ops are inherently race-tolerant per memory model).

**Borrow checker rule:** `&+ Atomic<T>` is treated specially — it's the ONE case where borrow checker permits "mutation through frozen ref" because the mutation is atomic-instruction-level, not arbitrary write. Compiler whitelist via `AtomicValue` marker; non-AtomicValue types still follow strict ownership.

**ADR-0026 v2 §4.3 retroactive fix:** ADR-0026 v2 §4.3 signature `&+ mutable Atomic<T>` is **superseded** by this ADR-0028 §5. ADR-0026 v2 file gets Addendum noting the supersedence (single-line per immutability rule — not edit the body of v2 §4.3).

**Rationale for §5 choice:**

- Matches Rust's `&AtomicU64` precedent (proven model since 2015).
- Resolves the §4.3 contradiction without inventing new reference form.
- Doesn't compromise borrow checker — `AtomicValue` is whitelist constraint, narrow scope.
- Future-friendly: if ADR-0028 v2 wants to introduce explicit "atomic-write capability" for non-shared atomic (rare), can extend cleanly.

**Alternatives considered + rejected:**

- (a) `&0 mutable Atomic<T>` (exclusive borrow): kills cross-thread share (the whole point of atomic). Refused.
- (b) New reference form `&* Atomic<T>` (atomic-share-mutable): adds complexity; whitelist solution simpler.
- (c) Move-only atomic (each store takes ownership): violates atomicity model. Refused.

---

## §6 — Constructor + drop

**Decision:** Constructor function exposed via stdlib `sys.atomic.new`:

```triet
public function new<T: AtomicValue>(initial_value: T) -> Atomic<T>
```

Returns a stack-allocated `Atomic<T>` initialized to `initial_value`. Caller stores ownership via `let mutable counter: Atomic<Integer> = sys.atomic.new(0)` then borrows as `&+ counter` for cross-thread share.

**Drop:** `Atomic<T>` is stack-allocatable per ADR-0026 v2 §4.1 + SPEC §10.5. Standard scope-end drop, no special semantics. No heap allocation → no ObjectHeader.

**Initial value:** Caller-provided, required argument (no default `Atomic::zero()` form to force explicit initialization per [feedback_explicit_strictness](../../#feedback)).

---

## §7 — Send rule integration

**Decision:** No change from ADR-0026 v2 §2.1 row 4. `Atomic<T>` always Send. Already implemented in `triet-typecheck::types::Type::is_send()` ([crates/triet-typecheck/src/types.rs:245](../../crates/triet-typecheck/src/types.rs)).

ADR-0028 documents the rationale: Atomic types are built for cross-thread sharing — Send-by-design.

Test coverage: `checks_send_bound_atomic_success` (existing v0.8.x.completion.3).

---

## §8 — Stdlib `sys.atomic.*` module shape

**Decision:** Free functions in `sys.atomic` namespace, NOT methods on `Atomic<T>`.

**Rationale:**

- Triết doesn't yet have impl block / method dispatch syntax beyond builtin trait dispatch. ADR-0003 iterator + ADR-0026 v2 §4.3 sketched method form but no method syntax shipped.
- Free functions parsable today, no SPEC §6 grammar addition needed.
- Capability gating cleaner — `sys.atomic.load` is one capability path, vs `Atomic.load` which couples to type system.
- If method syntax lands (v0.10+ stdlib expansion), can add method wrappers calling these functions. Forward-compatible.

**Module structure:**

```
std/sys/atomic.tri        // Stdlib file (filesystem-resolved per ADR-0005)
└── ambient (no capability gate at module level; per-op gate)
    ├── public function new<T: AtomicValue>(value: T) -> Atomic<T>
    ├── public function load<T>(self: &+ Atomic<T>, ord: Ordering?) -> T
    ├── public function store<T>(self: &+ Atomic<T>, value: T, ord: Ordering?) -> Unit
    ├── public function swap<T>(self: &+ Atomic<T>, value: T, ord: Ordering?) -> T
    ├── public function compare_exchange<T>(...) -> T~CompareExchangeFailed
    ├── public function fetch_add(self: &+ Atomic<Integer|Tryte>, ...) -> ...
    ├── public function fetch_sub(...) -> ...
    └── public function fetch_and/or/xor(self: &+ Atomic<Integer>, ...) -> Integer
```

**Capability gate:** `sys.atomic` capability required for **any** non-default-Ordering call OR `Atomic<Pointer>` use (which also requires `dev.raw_memory`). Default-Ordering calls don't require capability — they're "ambient safe" per ADR-0016 §3 ambient pattern. Rationale: default `Synchronized` is safe choice; only `Relaxed` (explicit weakening) or `Strict` (explicit strengthening) require explicit capability acknowledgment.

---

## §9 — VM dev tier behavior + capability gate

**Decision:** On single-thread VM (current dev tier per VISION §4.3):

- All atomic operations execute as plain non-atomic reads/writes.
- `Ordering` argument validated at typecheck (must be valid enum value) but no-op at runtime.
- Per-op capability gate fires per §8.
- Test corpus exercises type-level + Send + capability flow correctness.
- Cross-thread synchronization NOT exercisable until `std.concurrency.*` ships (v0.10) with real OS thread integration.

**Implication:** v0.9 ships Atomic full **type-level + API + Send rule + capability gate** correctness. Cross-thread runtime correctness deferred to v0.10 stdlib (when actual threading available).

**Test gate cho v0.9 Atomic close:**

1. Type-level: all 5 `AtomicValue` types accepted; non-AtomicValue rejected (E1024-style or new E25XX subvariant).
2. API: load/store/swap/compare_exchange round-trip on single-thread VM (correctness, not concurrency).
3. fetch_* arithmetic semantics correct.
4. Send rule: all `Atomic<T>` Send (existing test).
5. Capability gate: non-default Ordering requires `sys.atomic` grant (capability_gate_e2e.rs extension).
6. compare_exchange success + failure paths.

Cross-thread real-execution test = v0.10+ stdlib scope.

---

## §10 — E2530 InvalidAtomicOrdering fire conditions

**Author review required.** Conservative default v0.9 — strict refinement deferred to corpus.

**Decision v0.9 (conservative):**

E2530 fires only on **two narrowly-defined patterns**:

1. **Compare-exchange success_ordering weaker than failure_ordering** — semantically nonsensical; failure path stronger than success makes no sense.

```triet
// E2530 fires:
sys.atomic.compare_exchange(a, 0, 1, Ordering.Relaxed, Ordering.Strict)
//                                    ^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^
//                                    success weaker than failure
```

2. **fetch_add/sub/and/or/xor with `Ordering.Relaxed` on `Atomic<Pointer>`** — Pointer is publish-like by nature; Relaxed publish almost always wrong.

```triet
// E2530 fires (requires dev.raw_memory + sys.atomic anyway):
sys.atomic.fetch_add(ptr_atomic, 8, Ordering.Relaxed)
```

**Patterns NOT enforced in v0.9 (deferred):**

- Generalized "Relaxed publish" detection — requires data-flow analysis to spot "store + load pair where store is Relaxed but load expects published data". Hard problem (analogous to Rust borrowck NLL); defer until corpus exposes real cases.
- Cross-op ordering consistency (e.g., warn if mixing Relaxed and Strict on same atomic across functions).

**Rationale:** v0.9 ships skeleton enforcement matching ADR-0025 borrow checker pattern (`enforcement defers until real-world corpus first`). Two narrow patterns above are mechanical / always-wrong cases — safe to enforce immediately. Generalized analysis waits for corpus.

**Future ADR (post-v1.0):** May add Rust's `MaybeUninit`-style analysis, fence operations, etc.

---

## §11 — Migration from ADR-0026 v2 §4 placeholder

**ADR-0026 v2 §4** is now **superseded in part** by this ADR-0028:

- §4.0/§4.1 (type family + AtomicValue): **superseded** — full lock in ADR-0028 §2. ADR-0026 v2 §4.1 listed types; ADR-0028 §2 locks AtomicValue trait + per-type operation set.
- §4.2 (Ordering): **superseded** — ADR-0028 §3 adds Trit mapping + default ordering rule.
- §4.3 (API surface): **superseded with FIX** — ADR-0026 v2 wrote `&+ mutable Atomic<T>` for store; ADR-0028 §5 corrects to `&+ Atomic<T>` (interior mutability pattern, resolves cross-thread contradiction).
- §4.4 (E2530): **superseded** — ADR-0028 §10 locks conservative fire conditions.

ADR-0026 v2 file gets one-line Addendum at file top:

> **2026-05-29 Addendum:** §4 placeholder design refined by [ADR-0028](0028-atomic-primitive.md). The `&+ mutable` signature in §4.3 store/swap/compare_exchange is superseded by `&+` per ADR-0028 §5 (interior mutability).

Per project ADR immutability rule: ADR-0026 v2 body NOT edited; Addendum points to ADR-0028 as source-of-truth.

---

## Hệ quả

**Possible (positive):**

- v0.9.x.atomic implementation phase can begin với concrete API + semantic targets.
- Atomic counter demo (`examples/atomic_counter/`) gets runtime backing — `dao run` actually exercises `fetch_add` once `sys.atomic` stdlib file lands.
- Send rule + capability flow already in place from v0.8 → v0.9 implementation is purely additive (builtins + stdlib file).
- Future v2.0 LLVM AOT can lower builtins 1:1 vào LLVM atomic intrinsics — no opcode redesign.
- Future v∞ trytecode native maps builtin IDs vào ternary atomic ISA — Triết identity preserved at hardware level.

**Constrained (cost):**

- `.triv` wire format bumps v5 → v6 (additive: new builtin IDs 27-39 reserved). Pre-v6 readers refuse `.triv` files using them per ADR-0010 backward-compat.
- Borrow checker gains 1 special-case rule (`&+ Atomic<T>` permits interior mutation via atomic ops). Documented narrowly, doesn't generalize.
- `Long` excluded from AtomicValue. User wanting 81-trit atomic must wait for Mutex (v0.10) or use 3× `Atomic<Integer>` manually.
- E2530 conservative — won't catch all Relaxed publish bugs in v0.9.

**Costly (need verify):**

- VM single-thread dev tier means concurrency CORRECTNESS unverifiable until v0.10 stdlib ships real threading. Type-level + API + Send + capability are all v0.9 verifiable; race conditions are not.
- Cross-thread test corpus growth tied to v0.10 stdlib milestones.

---

## Không làm (explicitly rejected)

- **`Atomic<struct T>` or `Atomic<enum T>`** — composite types not atomic per ADR-0026 v2 §4.1. User wraps trong Mutex (v0.10 stdlib) or designs lock-free DS manually. v0.9 lock: `AtomicValue` membership is compiler-controlled, not user-extensible.
- **`Consume`/`Acquire`/`Release` ordering separately** — merged into `Synchronized` per ADR-0026 v2 §4.2 rationale. Kernel writers needing fine control go through `dev.raw_memory` capability + raw hardware intrinsics (out of scope for stdlib Atomic).
- **Fence operations** (`atomic_thread_fence`) — defer post-v1.0. Mostly useful in lock-free DS authoring; can be added cleanly later as builtin IDs 40+.
- **`MaybeUninit<T>`-style placeholder** — Triết SPEC §10 ownership model + outcome ADR-0020 cover "value may be absent" cases. No need for separate MaybeUninit.
- **User-defined `AtomicValue` impl** — compiler-controlled whitelist. Future ADR may extend if corpus shows narrow demand.
- **Lock-free queue / stack in core library** — implementation detail belonging to stdlib `std.concurrency.*` (v0.10+) or external crates (per BYOS — [ADR-0026 v2](0026-actor-boundary-send-rules.md)).
- **Atomic floating-point** — Triết v0.9 doesn't have FP. When FP lands (post-v1.0), separate ADR for atomic FP semantics.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| Rust `std::sync::atomic` | Interior mutability via shared `&` (= our `&+ Atomic<T>`); separate `compare_exchange` + `compare_exchange_weak`; per-op explicit Ordering | Triết: 3-level Ordering (vs Rust's 5); free functions vs methods; AtomicValue marker vs `Atomic*` type-per-primitive |
| C++ `std::atomic<T>` | Memory model (`memory_order_relaxed` etc.); fetch_* op naming | Triết: 3-level merge; built-in interior-mutability; explicit capability gate |
| Java `AtomicInteger/Long/Reference` | Class-per-type API surface; compareAndSet name | Triết: free functions; AtomicValue marker + generic; Trit/Trilean atomic Java doesn't have |
| Swift `Atomic` (proposed 2022, accepted 2024) | Newer "Send rule" + Atomic interaction pattern | Triết: BYOS philosophy, no language-level scheduler |
| Setun (1958) historical | Ternary-native atomics if hardware was concurrent (it wasn't) | Triết is the first ternary-with-concurrency design |

**What we invented:**

- **Trit-mapped Ordering** — `{-1, 0, +1}` polarity carries strength. Matches Triết identity rule; no prior art.
- **Capability-gated non-default ordering** — `sys.atomic` capability required for explicit Relaxed/Strict; default Synchronized is ambient. Novel — combines capability system (ADR-0016) with ordering hazard warnings.
- **AtomicValue marker with per-type op restriction** — Trit/Trilean get load/store/swap/CAS only (no fetch_add since Ł3 numeric ops are subtle); Tryte/Integer get full arithmetic; Pointer is gated. Per-type ops not seen in other languages.

---

## Tham chiếu

- [ADR-0026 v2](0026-actor-boundary-send-rules.md) — Concurrency Primitives & Send Rules (parent ADR, this ADR-0028 refines §4 placeholder).
- [ADR-0025](0025-borrow-checker-rules.md) — Borrow Checker Rules (interior mutability pattern interaction).
- [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md) — Rust-shim builtin pattern (ADR-0028 §1 follows).
- [ADR-0019 Addendum §A7.5](0019-self-hosting-compiler-bootstrap.md) — `.triv` wire format version bump policy.
- [ADR-0010](0010-ternary-native-ir.md) — Ternary IR (Trit semantics).
- [ADR-0016](0016-capability-type-system.md) — Capability system (sys.atomic gate).
- [ADR-0018](0018-capability-loader-semantics.md) — `dao.package` capability claim grammar.
- [SPEC §10.6](../../SPEC.md) — Concurrency boundary + Send rules (Atomic mentioned at type level).
- [VISION §4.3](../../VISION.md) — Multi-backend execution model (VM dev tier, AOT/JIT production).
- Rust RFC #2585 (2019) — `Atomic*` interior mutability formalization.
- C++ ISO N4860 (2020) — `std::atomic` standard.
- Sewell et al. (2010) — "Mathematizing C++ Concurrency" (PLDI) — formal memory model.
