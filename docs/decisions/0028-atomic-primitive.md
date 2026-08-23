# ADR 0028 — Atomic Primitive Design (refines ADR-0026 v2 §4)

**Status:** **Locked** (v0.9.0.1, author sign-off 2026-05-29). Refines [ADR-0026 v2 §4](0026-actor-boundary-send-rules.md) placeholder. The author confirmed 3 architecturally significant decisions: §1 builtin shim strategy; §5 ownership reference form fix (resolves ADR-0026 v2 §4.3 contradiction); §10 conservative E2530 enforcement.

> **2026-05-29 Addendum (v0.9.0.1.c):** Self-review post-lock identified 1 naming concern violating [VISION §6](../../VISION.md) "**Ternary is default, not auxiliary**" + "Explicit > implicit":
>
> **Gap — Bitwise operations on `Atomic<Integer>` leak binary semantics into Triet-ternary API.** §4.3 originally defined `fetch_and`/`fetch_or`/`fetch_xor` for `Atomic<Integer>`. But Triet `Integer` = 27-trit ternary value; Cranelift binary CPU backing slot = 64-bit. Bitwise ops operate on the 64-bit slot, NOT the 27-trit ternary value. Calling these "and/or/xor" implicitly suggests they are standard logical ops on Integer's ternary semantics — they are not. They are escape hatches for FFI scenarios where `Atomic<Integer>` stores a packed binary value (kernel flag bytes, etc.).
>
> **Resolution: rename to `fetch_bitwise_and` / `fetch_bitwise_or` / `fetch_bitwise_xor`.** Explicit `_bitwise_` prefix forces the caller to acknowledge: "I am using binary semantics on Triet Integer's 64-bit slot". Matches VISION §6 "Explicit > implicit" + "Refuse over guess".
>
> **Revised §4.3 signatures (replacing original):**
>
> ```triet
> public function fetch_bitwise_and(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
> public function fetch_bitwise_or(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
> public function fetch_bitwise_xor(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
> ```
>
> Behavior unchanged — same builtin shim, same wire format, same FFI use case. Only the user-visible name carries the binary-semantic warning explicitly.
>
> **Cross-platform note:** On v∞ trytecode native hardware (per [VISION §4.5](../../VISION.md)), these ops MAY have no natural mapping — a ternary CPU does not have bitwise instructions. Recommendation: the v∞ backend ADR should ship a compile-time error suggesting `fetch_trit_min` / `fetch_trit_max` (Łukasiewicz Ł3 conjunction/disjunction analogs) as ternary-native alternatives. Defer naming + design to v∞ scope.
>
> §4.2 arithmetic ops (`fetch_add`, `fetch_sub`) are NOT renamed — these represent arithmetic on ternary Integers (sum/difference make sense in balanced ternary per SPEC §3). Only the `bitwise_` rename applies to §4.3 binary-leak ops.
>
> **Addendum scope:** §4.3 user-visible API names. The ADR-0028 body is NOT edited per the project ADR immutability rule. The v0.9.x.atomic implementation uses renamed signatures. The stdlib `sys.atomic.*` module ships with renamed function names from day one.

**Issue:** ADR-0026 v2 §4 placeholder-locked Atomic primitive type family (`Atomic<Integer/Tryte/Trit/Trilean/Pointer>`) + Ordering enum (Relaxed/Synchronized/Strict) + skeleton API surface (load/store/swap/compare_exchange) + E2530 sketch. Open questions left for ADR-0028:

1. **Implementation pattern** — VM opcodes vs. Rust-shim builtins (per [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md))?
2. **Ordering $\leftrightarrow$ Trit mapping** — which polarity maps to which level?
3. **Full operation set** — fetch_add/sub/and/or/xor + type-specific ops (e.g., Trilean Ł3 atomic ops)?
4. **`AtomicValue` trait** — marker or with methods? Which types qualify?
5. **Reference form for store/swap/compare_exchange** — ADR-0026 v2 §4.3 wrote `&+ mutable Atomic<T>` for store, but cross-thread atomic sharing requires `&+` frozen (per §2.1 row 7 Send rule). This is a **contradiction** ADR-0028 must resolve.
6. **Constructor** — function form? Trait method? Auto-init?
7. **VM dev tier behavior** — single-threaded VM has no real concurrency; how do atomic ops behave?
8. **Capability boundary** — when is `sys.atomic` capability required vs. `sys.atomic` only for non-default ordering?
9. **E2530 InvalidAtomicOrdering fire conditions** — when does the compiler refuse Relaxed?
10. **Stdlib `sys.atomic.*` module shape** — top-level functions or methods on `Atomic<T>`?

ADR-0028 locks decisions for §1-§9; §10 ships in conservative form with refinement deferred to corpus exposure (per ADR-0025 enforcement-needs-corpus precedent).

---

## §1 — Implementation strategy: Rust-shim builtins (per ADR-0019 §5)

**Decision:** Atomic operations are **Rust-shim builtins** in the VM dispatcher, NOT new IR opcodes. Sits at opcode IDs 27-39 (next available after v0.7 builtins 4-26 per ADR-0019 §5).

**Rationale:**

- **Pattern consistency.** ADR-0019 §5 established Rust-shims for Vec/HashMap/file IO/path/string ops. Atomic follows the same shape — semantic operation backed by Rust implementation, exposed via stable opcode ID in `.triv` v6 wire format.
- **VM dev tier feasibility.** VM is single-threaded per [VISION §4.3](../../VISION.md). Atomic ops can be implemented as plain reads/writes; Ordering is a no-op semantically until real thread integration (v0.10+ stdlib). The shim approach makes this trivial — `Ordering::Relaxed` does identical work as `Ordering::Strict` on the single-threaded VM.
- **AOT / JIT lowering clarity.** When v2.0 LLVM AOT lands, builtin IDs lower 1:1 into LLVM atomic intrinsics (`@llvm.atomicrmw.add`, etc.). When v∞ trytecode native ships, builtins lower into the ternary atomic ISA.
- **Future-proof.** Adding new operations (e.g., `fetch_min`/`fetch_max` in v0.10+) requires only a new builtin ID, avoiding IR opcode churn.

**Wire format:** `.triv` v5 $\rightarrow$ v6 patch bump (ADR-0028 §1 lock). Pre-v6 readers see new builtin IDs and refuse with `UnknownOpcode` per ADR-0010 backward-compatibility rules.

**Rejected alternative:** Dedicated `Atomic` IR opcode family (~15 new opcodes). Pros: more explicit at IR inspection. Cons: violates ADR-0019 §5 builtin pattern; couples IR to a specific operation set; harder to extend.

---

## §2 — Type family + `AtomicValue` trait

**Decision:** `Atomic<T>` is parameterized on `T: AtomicValue` where `AtomicValue` is a **marker trait** (no required methods). The compiler enforces membership at type-check.

**Members of `AtomicValue` (locked v0.9):**

| Type | Bit width on binary CPU | Trit width on ternary | Operation set |
|---|---|---|---|
| `Trit` | 8 bits (atomic byte) | 1 trit | load / store / swap / compare_exchange |
| `Tryte` | 16 bits | 9 trits | load / store / swap / compare_exchange / fetch_add / fetch_sub |
| `Integer` | 64 bits (atomic u64) | 27 trits | load / store / swap / compare_exchange / fetch_add / fetch_sub / fetch_and / fetch_or / fetch_xor |
| `Long` | NOT atomic-able | 81 trits | — (exceeds hardware atomic width) |
| `Trilean` | 8 bits | 1 trit (`{-1,0,+1}` = `{false, unknown, true}`) | load / store / swap / compare_exchange |
| `Pointer` | 64 bits (`usize`) | 27 trits | load / store / swap / compare_exchange — **requires `dev.raw_memory` capability** |

**Note:** `Long` (81-trit) is explicitly excluded — it exceeds hardware atomic width on both 64-bit binary CPUs and forecast trytecode hardware. Users requiring atomic 81-trit values must use a Mutex (planned `std.concurrency.Mutex` v0.10) or split into 3× 27-trit `Atomic<Integer>` with manual ordering.

**Marker trait declaration:**

```triet
public trait AtomicValue {}
// Implementations: provided by compiler intrinsic, not user-defined.
```

Users **cannot** implement `AtomicValue` for custom types per v0.9 lock (no struct Atomic, no enum Atomic, no nested `Atomic<Atomic<T>>`). A future ADR can extend this if corpus demand arises.

**Trit/Trilean ops note:** Trit is a 3-state number ({-1, 0, +1}); Trilean is a 3-state truth value. Bitwise `fetch_and`/`fetch_or`/`fetch_xor` only make sense for binary types (Tryte/Integer). Trit + Trilean receive the safe minimum: load/store/swap/compare_exchange.

---

## §3 — `Ordering` enum + Trit mapping

**Decision:** `Ordering` is a 3-variant enum mapping into `Trit` polarity per the Triet identity rule:

```triet
public enum Ordering {
    Relaxed,        // Trit::Negative (-1) — weakest
    Synchronized,   // Trit::Zero      ( 0) — middle
    Strict,         // Trit::Positive (+1) — strongest
}
```

**Mapping rationale:**

- **Polarity = strength.** Negative = relaxed/weak; Zero = neutral/middle; Positive = strict/strong. Matches Triet's `&+`/`&0`/`&-` ownership polarity convention.
- **C++ equivalence:**
  - `Relaxed` $\equiv$ `memory_order_relaxed` (no synchronization, atomic only).
  - `Synchronized` $\equiv$ `memory_order_acq_rel` (acquire on load, release on store).
  - `Strict` $\equiv$ `memory_order_seq_cst` (total order across all threads).
- **C++ 5-level $\rightarrow$ Triet 3-level:** `Consume` and `Acquire` merge into `Synchronized`. Consume is rarely usefully distinct from Acquire in practice (most compilers lower Consume $\rightarrow$ Acquire anyway). Kernel writers needing finer control go through the `dev.raw_memory` capability to use raw hardware intrinsics.

**Default for store/swap/compare_exchange (no explicit ordering):** `Ordering.Synchronized`. Strong default — covers 95% of use cases safely. The author opts into `Relaxed` explicitly (signaling intentional weakness).

**Default for load (no explicit ordering):** `Ordering.Synchronized`. Same reasoning.

**Default for fetch_add/sub/and/or/xor:** `Ordering.Synchronized`.

---

## §4 — API surface (full operation set)

**Decision:** Functions are exposed via the stdlib `sys.atomic.*` module (NOT methods on `Atomic<T>` — see §8 for rationale). Each function takes an explicit `Ordering` argument; default-ordering overloads are provided for ergonomics.

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

// Same overloads for Atomic<Tryte>:
public function fetch_add(self: &+ Atomic<Tryte>, delta: Tryte, ordering: Ordering) -> Tryte
public function fetch_sub(self: &+ Atomic<Tryte>, delta: Tryte, ordering: Ordering) -> Tryte
```

All `fetch_*` return the **previous** value (pre-modification). Overflow: per balanced ternary §3.2 (no overflow within range; out-of-range = E2010 RuntimeOverflow).

### 4.3 — Bitwise (Integer only — Tryte excluded because 9-trit width clashes with binary atomic intrinsics)

```triet
public function fetch_and(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
public function fetch_or(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
public function fetch_xor(self: &+ Atomic<Integer>, mask: Integer, ordering: Ordering) -> Integer
```

Note: bitwise operations on balanced ternary are semantically unusual (Triet is ternary-first; "bitwise" is a binary-CPU lowering detail). These ops are escape hatches for FFI scenarios where `Atomic<Integer>` stores a packed binary value. **A future ADR may add ternary-native ops (`fetch_trit_and` Ł3-semantic) — deferred until corpus demand arises.**

### 4.4 — Trit/Trilean — load/store/swap/compare_exchange only

No `fetch_*` ops for Trit or Trilean per the §2 type table. Use a compare_exchange loop for transitions.

---

## §5 — Reference form for atomic operations (RESOLVES ADR-0026 v2 §4.3 contradiction)

**Author review required.**

**Issue:** ADR-0026 v2 §4.3 wrote `store(self: &+ mutable Atomic<T>, ...)`. But cross-thread atomic sharing REQUIRES `&+` frozen (per §2.1 row 7 Send rule — `&+` Send via refcount-mediated share, whereas `&+ mutable` is exclusive move-only). Contradiction: an atomic cannot be BOTH cross-thread-shared AND mutable-via-exclusive-borrow.

**Decision (this ADR):** All `Atomic<T>` operations take `&+ Atomic<T>` (frozen owner). Atomicity is **internal interior mutability** — the implementation uses raw hardware atomic instructions to mutate without violating owner immutability. Mirrors Rust's `&AtomicU64` (shared borrow) + interior mutation pattern.

**Implication:** `&+ Atomic<T>` is the canonical handle. Many threads can hold `&+ Atomic<T>` simultaneously (refcount-mediated sharing). Each thread can call store/swap/fetch_* on it; race conditions are resolved by Ordering semantics, not by Triet's borrow checker (atomic operations are inherently race-tolerant per the memory model).

**Borrow checker rule:** `&+ Atomic<T>` is treated specially — it is the ONE case where the borrow checker permits "mutation through frozen ref" because mutation occurs at the atomic-instruction level rather than via arbitrary write. Compiler whitelist via the `AtomicValue` marker; non-AtomicValue types continue to follow strict ownership.

**ADR-0026 v2 §4.3 retroactive fix:** The ADR-0026 v2 §4.3 signature `&+ mutable Atomic<T>` is **superseded** by this ADR-0028 §5. The ADR-0026 v2 file receives an Addendum noting the supersedence (single-line per the immutability rule — not editing the body of v2 §4.3).

**Rationale for §5 choice:**

- Matches Rust's `&AtomicU64` precedent (a proven model since 2015).
- Resolves the §4.3 contradiction without inventing a new reference form.
- Does not compromise the borrow checker — `AtomicValue` is a whitelist constraint with narrow scope.
- Future-friendly: if ADR-0028 v2 introduces explicit "atomic-write capabilities" for non-shared atomics (rare), it can extend cleanly.

**Alternatives considered + rejected:**

- (a) `&0 mutable Atomic<T>` (exclusive borrow): eliminates cross-thread sharing (the core purpose of atomics). Rejected.
- (b) New reference form `&* Atomic<T>` (atomic-share-mutable): adds language complexity; whitelist solution is simpler.
- (c) Move-only atomic (each store takes ownership): violates the atomicity model. Rejected.

---

## §6 — Constructor + drop

**Decision:** Constructor function exposed via stdlib `sys.atomic.new`:

```triet
public function new<T: AtomicValue>(initial_value: T) -> Atomic<T>
```

Returns a stack-allocated `Atomic<T>` initialized to `initial_value`. The caller stores ownership via `let mutable counter: Atomic<Integer> = sys.atomic.new(0)` then borrows as `&+ counter` for cross-thread sharing.

**Drop:** `Atomic<T>` is stack-allocatable per ADR-0026 v2 §4.1 + SPEC §10.5. Standard scope-end drop, no special semantics. No heap allocation $\rightarrow$ no ObjectHeader.

**Initial value:** Caller-provided, required argument (no default `Atomic::zero()` form to enforce explicit initialization per [feedback_explicit_strictness](../../#feedback)).

---

## §7 — Send rule integration

**Decision:** No change from ADR-0026 v2 §2.1 row 4. `Atomic<T>` is always Send. Already implemented in `triet-typecheck::types::Type::is_send()` ([crates/triet-typecheck/src/types.rs:245](../../crates/triet-typecheck/src/types.rs)).

ADR-0028 documents the rationale: Atomic types are built for cross-thread sharing — Send-by-design.

Test coverage: `checks_send_bound_atomic_success` (existing v0.8.x.completion.3).

---

## §8 — Stdlib `sys.atomic.*` module shape

**Decision:** Free functions in the `sys.atomic` namespace, NOT methods on `Atomic<T>`.

**Rationale:**

- Triet does not yet have impl block / method dispatch syntax beyond builtin trait dispatch. ADR-0003 iterator + ADR-0026 v2 §4.3 sketched method forms but no method syntax has shipped.
- Free functions are parsable today, requiring no SPEC §6 grammar addition.
- Capability gating is cleaner — `sys.atomic.load` is one capability path, vs `Atomic.load` which couples to the type system.
- If method syntax lands (v0.10+ stdlib expansion), method wrappers calling these functions can be added cleanly. Forward-compatible.

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

**Capability gate:** `sys.atomic` capability is required for **any** non-default-Ordering call OR `Atomic<Pointer>` use (which also requires `dev.raw_memory`). Default-Ordering calls do not require capabilities — they are "ambient safe" per the ADR-0016 §3 ambient pattern. Rationale: default `Synchronized` is the safe choice; only `Relaxed` (explicit weakening) or `Strict` (explicit strengthening) require explicit capability acknowledgment.

---

## §9 — VM dev tier behavior + capability gate

**Decision:** On the single-threaded VM (current dev tier per VISION §4.3):

- All atomic operations execute as plain non-atomic reads/writes.
- The `Ordering` argument is validated at typecheck (must be a valid enum value) but acts as a no-op at runtime.
- Per-op capability gates fire per §8.
- The test corpus exercises type-level + Send + capability flow correctness.
- Cross-thread synchronization is NOT exercisable until `std.concurrency.*` ships (v0.10) with real OS thread integration.

**Implication:** v0.9 ships Atomic with full **type-level + API + Send rule + capability gate** correctness. Cross-thread runtime correctness is deferred to the v0.10 stdlib (when actual threading becomes available).

**Test gate for v0.9 Atomic closure:**

1. Type-level: all 5 `AtomicValue` types accepted; non-AtomicValue rejected (E1024-style or new E25XX subvariant).
2. API: load/store/swap/compare_exchange round-trip on single-threaded VM (correctness, not concurrency).
3. fetch_* arithmetic semantics correct.
4. Send rule: all `Atomic<T>` Send (existing test).
5. Capability gate: non-default Ordering requires `sys.atomic` grant (`capability_gate_e2e.rs` extension).
6. compare_exchange success + failure paths.

Cross-thread real-execution tests fall under the v0.10+ stdlib scope.

---

## §10 — E2530 InvalidAtomicOrdering fire conditions

**Author review required.** Conservative default in v0.9 — strict refinement deferred to corpus.

**Decision v0.9 (conservative):**

E2530 fires only on **two narrowly-defined patterns**:

1. **Compare-exchange success_ordering weaker than failure_ordering** — semantically nonsensical; a failure path stronger than success makes no sense.

```triet
// E2530 fires:
sys.atomic.compare_exchange(a, 0, 1, Ordering.Relaxed, Ordering.Strict)
//                                    ^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^
//                                    success weaker than failure
```

2. **fetch_add/sub/and/or/xor with `Ordering.Relaxed` on `Atomic<Pointer>`** — Pointer is publish-like by nature; Relaxed publish is almost always wrong.

```triet
// E2530 fires (requires dev.raw_memory + sys.atomic anyway):
sys.atomic.fetch_add(ptr_atomic, 8, Ordering.Relaxed)
```

**Patterns NOT enforced in v0.9 (deferred):**

- Generalized "Relaxed publish" detection — requires data-flow analysis to spot "store + load pair where store is Relaxed but load expects published data". A complex problem (analogous to Rust borrowck NLL); deferred until corpus exposes real cases.
- Cross-op ordering consistency (e.g., warning if mixing Relaxed and Strict on the same atomic across functions).

**Rationale:** v0.9 ships skeleton enforcement matching the ADR-0025 borrow checker pattern (`enforcement defers until real-world corpus first`). The two narrow patterns above are mechanical / always-wrong cases — safe to enforce immediately. Generalized analysis waits for corpus data.

**Future ADR (post-v1.0):** May add Rust's `MaybeUninit`-style analysis, fence operations, etc.

---

## §11 — Migration from ADR-0026 v2 §4 placeholder

**ADR-0026 v2 §4** is now **superseded in part** by this ADR-0028:

- §4.0/§4.1 (type family + AtomicValue): **superseded** — full lock in ADR-0028 §2. ADR-0026 v2 §4.1 listed types; ADR-0028 §2 locks the AtomicValue trait + per-type operation set.
- §4.2 (Ordering): **superseded** — ADR-0028 §3 adds Trit mapping + default ordering rule.
- §4.3 (API surface): **superseded with FIX** — ADR-0026 v2 wrote `&+ mutable Atomic<T>` for store; ADR-0028 §5 corrects to `&+ Atomic<T>` (interior mutability pattern, resolves cross-thread contradiction).
- §4.4 (E2530): **superseded** — ADR-0028 §10 locks conservative fire conditions.

The ADR-0026 v2 file receives a one-line Addendum at the top of the file:

> **2026-05-29 Addendum:** §4 placeholder design refined by [ADR-0028](0028-atomic-primitive.md). The `&+ mutable` signature in §4.3 store/swap/compare_exchange is superseded by `&+` per ADR-0028 §5 (interior mutability).

Per project ADR immutability rules: the ADR-0026 v2 body is NOT edited; the Addendum points to ADR-0028 as the source of truth.

---

## Consequences

**Positive Outcomes:**

- The v0.9.x.atomic implementation phase can begin with concrete API and semantic targets.
- The atomic counter demo (`examples/atomic_counter/`) gains runtime backing — `dao run` actually exercises `fetch_add` once the `sys.atomic` stdlib file lands.
- Send rules and capability flows are already in place from v0.8 $\rightarrow$ v0.9 implementation is purely additive (builtins + stdlib file).
- Future v2.0 LLVM AOT can lower builtins 1:1 into LLVM atomic intrinsics — no opcode redesign.
- Future v∞ trytecode native maps builtin IDs into the ternary atomic ISA — Triet identity is preserved at the hardware level.

**Constraints & Costs:**

- `.triv` wire format bumps v5 $\rightarrow$ v6 (additive: new builtin IDs 27-39 reserved). Pre-v6 readers refuse `.triv` files using them per ADR-0010 backward-compatibility rules.
- The borrow checker gains 1 special-case rule (`&+ Atomic<T>` permits interior mutation via atomic ops). Documented narrowly, does not generalize.
- `Long` is excluded from AtomicValue. Users wanting an 81-trit atomic must wait for Mutex (v0.10) or use 3× `Atomic<Integer>` manually.
- E2530 is conservative — will not catch all Relaxed publish bugs in v0.9.

**Risks & Verification Needs:**

- VM single-threaded dev tier means concurrency CORRECTNESS is unverifiable until the v0.10 stdlib ships real threading. Type-level + API + Send + capability are all v0.9 verifiable; race conditions are not.
- Cross-thread test corpus growth is tied to v0.10 stdlib milestones.

---

## Rejected Alternatives

- **`Atomic<struct T>` or `Atomic<enum T>`** — composite types are not atomic per ADR-0026 v2 §4.1. Users wrap them in a Mutex (v0.10 stdlib) or design lock-free data structures manually. v0.9 lock: `AtomicValue` membership is compiler-controlled, not user-extensible.
- **`Consume`/`Acquire`/`Release` ordering separately** — merged into `Synchronized` per the ADR-0026 v2 §4.2 rationale. Kernel writers needing fine control go through `dev.raw_memory` capability + raw hardware intrinsics (out of scope for stdlib Atomic).
- **Fence operations** (`atomic_thread_fence`) — deferred to post-v1.0. Primarily useful in lock-free DS authoring; can be added cleanly later as builtin IDs 40+.
- **`MaybeUninit<T>`-style placeholder** — Triet SPEC §10 ownership model + Outcome ADR-0020 cover "value may be absent" cases. No need for a separate MaybeUninit.
- **User-defined `AtomicValue` impl** — compiler-controlled whitelist. A future ADR may extend this if the corpus shows narrow demand.
- **Lock-free queue / stack in core library** — implementation detail belonging to stdlib `std.concurrency.*` (v0.10+) or external crates (per BYOS — [ADR-0026 v2](0026-actor-boundary-send-rules.md)).
- **Atomic floating-point** — Triet v0.9 does not have FP. When FP lands (post-v1.0), a separate ADR will address atomic FP semantics.

---

## Prior Art

| Source | What We Adopted | What We Changed |
|---|---|---|
| Rust `std::sync::atomic` | Interior mutability via shared `&` (= our `&+ Atomic<T>`); separate `compare_exchange` + `compare_exchange_weak`; per-op explicit Ordering | Triet: 3-level Ordering (vs. Rust's 5); free functions vs. methods; AtomicValue marker vs. `Atomic*` type-per-primitive |
| C++ `std::atomic<T>` | Memory model (`memory_order_relaxed` etc.); `fetch_*` op naming | Triet: 3-level merge; built-in interior-mutability; explicit capability gate |
| Java `AtomicInteger/Long/Reference` | Class-per-type API surface; `compareAndSet` naming | Triet: free functions; AtomicValue marker + generic; Trit/Trilean atomics that Java lacks |
| Swift `Atomic` (proposed 2022, accepted 2024) | Newer "Send rule" + Atomic interaction pattern | Triet: BYOS philosophy, no language-level scheduler |
| Setun (1958) historical | Ternary-native atomics if hardware was concurrent (it was not) | Triet is the first ternary-with-concurrency design |

**Novel Contributions in Triet:**

- **Trit-mapped Ordering** — `{-1, 0, +1}` polarity carries synchronization strength. Matches Triet identity rules; no prior art.
- **Capability-gated non-default ordering** — `sys.atomic` capability is required for explicit Relaxed/Strict; default Synchronized is ambient. Novel — combines the capability system (ADR-0016) with ordering hazard warnings.
- **AtomicValue marker with per-type op restrictions** — Trit/Trilean get load/store/swap/CAS only (no fetch_add since Ł3 numeric ops are subtle); Tryte/Integer get full arithmetic; Pointer is gated. Per-type ops are unique compared to other languages.

---

## References

- [ADR-0026 v2](0026-actor-boundary-send-rules.md) — Concurrency Primitives & Send Rules (parent ADR, this ADR-0028 refines §4 placeholder).
- [ADR-0025](0025-borrow-checker-rules.md) — Borrow Checker Rules (interior mutability pattern interaction).
- [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md) — Rust-shim builtin pattern (ADR-0028 §1 follows).
- [ADR-0019 Addendum §A7.5](0019-self-hosting-compiler-bootstrap.md) — `.triv` wire format version bump policy.
- [ADR-0010](0010-ternary-native-ir.md) — Ternary IR (Trit semantics).
- [ADR-0016](0016-capability-type-system.md) — Capability system (`sys.atomic` gate).
- [ADR-0018](0018-capability-loader-semantics.md) — `dao.package` capability claim grammar.
- [SPEC §10.6](../../SPEC.md) — Concurrency boundary + Send rules (Atomic mentioned at type level).
- [VISION §4.3](../../VISION.md) — Multi-backend execution model (VM dev tier, AOT/JIT production).
- Rust RFC #2585 (2019) — `Atomic*` interior mutability formalization.
- C++ ISO N4860 (2020) — `std::atomic` standard.
- Sewell et al. (2010) — "Mathematizing C++ Concurrency" (PLDI) — formal memory model.
