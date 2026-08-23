# ADR 0026 — Concurrency Primitives & Send Rules (Bring Your Own Scheduler)

**Status:** **Locked v2** (promoted via v0.8.x.review 2026-05-28; supersedes 2026-05-26 v1 "Actor Boundary & Send Rules" per BYOS pivot). Sibling to [ADR-0022](0022-trit-balanced-ownership.md) + [ADR-0025](0025-borrow-checker-rules.md). v0.8 has shipped the Send derivation algorithm for 13 type categories (`triet-typecheck::types::Type::is_send()`) + E2500 fires + expanded capability schema. Locks language-level concurrency primitives + compile-time Send rules. **Refuses** baking the scheduler/runtime into the core language — kernel writers bring their own. Diagnostic format follows [ADR-0027](0027-diagnostic-format-standard.md).

> **2026-05-29 Addendum (v0.9.0.1):** §4 placeholder design refined by [ADR-0028](0028-atomic-primitive.md). The `&+ mutable Atomic<T>` signature in §4.3 store/swap/compare_exchange is **superspend** by `&+ Atomic<T>` per [ADR-0028 §5](0028-atomic-primitive.md#5--reference-form-for-atomic-operations-resolves-adr-0026-v2-43-contradiction) — atomicity = interior mutability via raw hardware atomic instructions, mirroring the Rust `&AtomicU64` pattern. Cross-thread atomic sharing REQUIRES a frozen reference (per §2.1 row 7 Send rule); ADR-0026 v2 §4.3's `&+ mutable` form was an internal contradiction not exercised by the v0.8 placeholder ship. ADR-0026 v2 body NOT edited per project ADR immutability rule; ADR-0028 §5 is the source-of-truth for atomic operation signatures.

**Issue:** v1 (2026-05-26) proposed `actor`/`receive`/`send`/`spawn` keywords + a mailbox runtime. The author on 2026-05-26 (on the same day, after reviewing) identified a core contradiction: **Triet's objective is kernel development** (per VISION §3.5), but v1 baked a green-thread/actor runtime into the core language — kernels do not have a user-space runtime to host a scheduler. Linux Rust modules must use the C scheduler (kthread, workqueue). Rust async is unavailable in the kernel. v1 repeated this mistake.

Core Insight:

> **"We must assume that our concurrency solution is not the best; there will always be developers who can do better. This concurrency component can be treated as a utility; we must provide the capability for another developer to implement a thread management system, and future Triet users will utilize that thread management system."**

v2 completely reframes this: **Bring Your Own Scheduler (BYKS)**. The core language only provides universal primitives + compile-time safety rules. The scheduler/runtime is part of the stdlib (for `usr::` apps) or external (for kernel/embedded). All `actor`/`spawn`/`async`/`await`/`parallel` keywords are refused from the core.

---

## §1 — Goals & Non-goals

### 1.1 — Goals

1. **Universal Send rules** — compile-time derivation for any scheduler.
2. **Linear ownership across thread boundaries** — no shared mutable cross-thread state (compile-time enforced via ADR-0022 D1).
3. **Atomic primitives** — language-level types for lock-free programming.
4. **Capability gates** — `sys::raw_thread`, `sys::atomic`, `dev::ffi` for thread primitives.
5. **No-mandate-scheduler** — kernel writers build their own schedulers and share them with the community.
6. **Compile-time race-freedom** — regardless of the scheduler, data races are impossible.

### 1.2 — Non-goals (refused, not deferred)

- ❌ `async`/`await` keywords — viral coloring, not kernel-safe
- ❌ `spawn` keyword — assumes a runtime
- ❌ `parallel { }` block — assumes a scheduler
- ❌ `actor`/`receive` keywords — assumes a mailbox runtime
- ❌ Channels as built-in syntax — channels are stdlib types, not language keywords
- ❌ Implicit heap allocator coupling — kernels have their own allocators
- ❌ Built-in green thread scheduler in core — stdlib reference implementation only

All the above may appear in stdlib library code, but **not as language keywords**.

### 1.3 — Error code namespace E25XX

Reserved range **E2500–E02599** for concurrency diagnostics. Allocation:

| Range | Category |
|---|---|
| E2500–E2509 | Send derivation violations |
| E2510–E2519 | Scope-ref / weak-ref boundary violations |
| E2520–E2529 | Mutable-share anti-pattern |
| E2530–E2539 | Atomic memory ordering violations |
| E2540–E2549 | Reserved: capability mismatch in thread primitives |
| E2550–E2599 | Reserved for future expansion |

Module path: `triet::concurrency::E25XX`. CLAUDE.md to be updated when the ADR lands.

---

## §2 — Send Derivation Rules (compile-time, universal)

**Lock:** Every type `T` has a compile-time property `Send(T)` — a boolean, derived based on its structure. Users do not write trait bounds; the compiler infers them. This applies at **every function boundary** where a parameter has the `: Send` annotation (or equivalent trait bound).

### 2.1 — Inductive Rules

| Type | Send(T) |
|---|---|
| Primitive value types (`Trit`, `Tryte`, `Integer`, `Long`, `Trilean`, `Unit`) | ✅ Always Send |
| Tuples `(A, B, ...)` | ✅ Send iff all components are Send |
| `T?` (nullable) | ✅ Send iff T is Send |
| `T~E`, `T?~E` (outcome) | ✅ Send iff T is Send and E is Send |
| `Vector<T>`, `Map<K, V>`, `Set<T>` | ✅ Send iff elements are Send |
| User-defined struct `S { f1: T1, f2: T2, ... }` | ✅ Send iff all fields are Send |
| User-defined enum / variant | ✅ Send iff all variant payloads are Send |
| `&+ T` (frozen owner) | ✅ Send iff T is Send. Implicit refcount at boundary (per §7) |
| `&+ mutable T` | ✅ Send iff T is Send. Linear move (single owner thread) |
/
| `&0 T`, `&0 mutable T` (scope borrow) | ❌ Never Send |
| `&- T` (weak observer) | ❌ Never Send |
| `Atomic<T>` (where T is a value type) | ✅ Always Send (atomic by definition) |
| Function types `fn(...) -> ...` | ✅ Send iff all captures are Send (see closure ADR) |
| Raw thread handles (`sys::raw_thread.Handle`) | ✅ Send (kernel concern) |

### 2.2 — Why `&0` and `&-` are NEVER Send

`&0` is scope-bound (ADR-0022 §2). A scope belongs to a specific execution context — there is no concept of "scope" cross-thread. Allowing `&0` cross-thread would break the compile-time invariant of ADR-0025 §2 (NLL exclusivity per-place within the same CFG).

`&- T` (weak observer) is compile-time tracked (ADR-0022 §9). Tracing from a weak reference back to `&+` is only valid within a single execution context. Cross-thread $\rightarrow$ the owner trail becomes discontinuous.

### 2.3 — Application site: trait bound `: Send`

Send rules apply at function boundaries with explicit annotations. Example stdlib `std.concurrency.green.spawn`:

```triet
// std/concurrency/green.tri
public function spawn<F: Send>(work: F) -> JoinHandle~ThreadError
where F: function() -> Unit {
    // implementation uses sys::raw_thread capability
}
```

User code:

```triet
let buffer: &+ mutable Buffer = make_buffer()
spawn(|| write_data(buffer))    // ✅ &+ mutable Buffer is Send
```

Captures inside the closure `||` typecheck against the `Send` bound. Failure $\rightarrow$ E2500.

### 2.4 — E2500 NotSendCannotCrossBoundary

```text
E2500 NotSendCannotCrossBoundary
    Type `Foo` cannot cross thread/scheduler boundary because field
    `bar: &0 String` is a scope borrow. Scope borrows are bound to a
    single execution context's control-flow graph (ADR-0025 §2).
    
    --> src/example.tri:12:18
       |
    12 |     spawn(|| process(payload))
       |                       ^^^^^^^ payload contains non-Send field
       |
    8  |     public struct Foo {
    9  |         bar: &0 String
       |         ------- this field makes `Foo` non-Send
    10 |     }
    
    Suggested fixes:
    
    [Fix 1] Take ownership of the borrowed data before passing it across:
    Change `bar: &0 String` to `bar: &+ String`
    
    [Fix 2] Restructure so the borrow stays within the originating context:
    Refactor the spawned closure to derive `payload` from values, not borrows
    
    [Fix 3] Pass only the necessary owned data through the boundary:
    Replace `payload` with a struct that carries just the owned fields needed
```

### 2.5 — Generic enforcement at monomorphization

For generic functions, the Send check occurs at call-site monomorphization. This is compile-time, not runtime.

```triet
let r: &0 Vector<UserId> = &0 ids
spawn(|| process(r))              // E2500 — &0 Vector not Send (at monomorphization-time)
```

---

## §3 — Linear Ownership Across Boundary

**Lock:** Linear ownership from ADR-0022 D1 + move semantics from ADR-0025 §5 apply unaltered at thread boundaries. No shared mutable cross-thread state — period.

### 3.1 — `&+ mutable T` across boundary = move

```triet
let mutable job: &+ mutable Job = build_job()
spawn(|| process(job))            // job is MOVED into the closure
print(job.priority)                // E2420 UseAfterMove
```

Mirrors Rust `Send + !Sync` types. Zero runtime cost — same allocation, different owner thread.

### 3.2 — `&+ T` (frozen) across boundary = refcount-mediated share

When `&+ T` (frozen owner) is captured into a `Send` closure:

```triet
let config: &+ Config = load_config()
spawn(|| use_config(config))      // refcount increases atomically; sender retains handle
print(config.version)             // OK — sender still has the handle
```

Behind the scenes:
- The `ObjectHeader` refcount (per §7) increases atomically when the closure is `Send` (= cross boundary).
- Both the sender thread and the spawned thread hold a `&+ Config` handle.
- The refcount decreases when each handle is dropped. Memory is freed when refcount reaches 0.

**User-visible:** The refcount is invisible. The object is simply shareable because it is frozen.

### 3.3 — Prohibition of shared mutable cross-thread state

```triet
let mutable counter: &+ mutable Counter = Counter.new()
spawn(|| increment(counter))      // OK — counter is moved
spawn(|| increment(counter))      // E2420 UseAfterMove
```

To share mutable state, use:
1. **Atomic primitives** (see §4) — lock-free, hardware-supported.
2. **Wrap in a dedicated "owner thread"** — encapsulate mutable state within one execution context and communicate via message passing (stdlib `std.concurrency.channel`).
3. **Stdlib `Mutex<T>`** — built on Atomics, not a language built-in.

### 3.4 — Refused List (no language-level escape hatch)

The Triet core **does not provide** a capability to bypass §3.3. There is no `dev::cross_thread_mut`. Reason: Avoiding "Java synchronized hell" and "Rust Arc<Mutex> panic" — we prefer refusal over guessing (VISION §6).

If a user **truly** needs shared mutable state (e.g., kernel-level shared state, lock-free queues), use:
- Atomic primitives (§4) — compile-time safe.
- Capability `dev::raw_memory` + `sys::atomic` — a kernel responsibility.

---

## §4 — Atomic Primitive Types

**Lock (placeholder design — see ADR-0028 or Addendum):** The Triet core provides an `Atomic<T>` family for lock-free programming. `T` must be a value type with hardware atomic support. Includes a memory ordering enum.

### 4.1 — Type family

```triet
Atomic<Integer>     // 27-trit atomic on ternary native, i32/i64 on binary
Atomic<Tryte>       // 9-trit atomic on ternary native, i8/i16 on binary
Atomic<Trit>        // 1-trit atomic
Atomic<Trilean>     // logic atomic (3-state)
Atomic<Pointer>     // for raw_memory capability — kernel only
```

Composite types (struct, Vector, Outcome) **cannot** be directly made atomic. Users must wrap them in a `Mutex` or design a lock-free data structure.

### 4.2 — Memory ordering

3 levels (mapping to hardware concepts):

| Triet | C++ equivalent | Hardware semantics |
|---|---|---|
| `Ordering.Relaxed` | `memory_order_relaxed` | No synchronization, atomic only |
| `Ordering.Synchronized` | `memory_order_acq_rel` | Acquire on load, Release on store |
| `Ordering.Strict` | `int memory_order_seq_cst` | Total order across all threads |

The 5-level C++ model (Relaxed/Consume/Acquire/Release/AcqRel/SeqCst) is reduced to 3 — sufficient for 95% of use cases. Kernel writers requiring `Consume` can use the `dev::raw_memory` capability to access raw hardware intrinsics.

**Why 3 instead of 5?** Aligns with ternary brand identity. Trade-off: `Consume` + `Acquire` are merged (as `Consume` is rarely useful in practice — most compilers treat it as `Acquire` anyway).

### 4.3 — API surface

```triet
public struct Atomic<T: AtomicValue> {
    // implementation defined
}

public function Atomic<T>.load(self: &0 Atomic<T>, ordering: Ordering) -> T
public function Atomic<T>.store(self: &+ mutable Atomic<T>, value: T, ordering: Ordering) -> Unit
public function Atomic<T>.swap(self: &+ mutable Atomic<T>, value: T, ordering: Ordering) -> T
public function Atomic<T>.compare_exchange(
    self: &+ mutable Atomic<T>,
    expected: T,
    new_value: T,
    success_ordering: Ordering,
    failure_ordering: Ordering
) -> T~CompareExchangeFailed
```

Note: `Atomic<T>` itself is **always Send** (per §2.1 table). This allows sharing atomic handles between threads — which is the entire purpose of atomics.

### 4.4 — E2530 InvalidAtomicOrdering

```text
E2530 InvalidAtomicOrdering
    Atomic operation `store` with `Ordering.Relaxed` is unsafe when the
    store publishes data accessed by other threads. Use `Ordering.Synchronized`
    (Release) or `int Ordering.Strict` (SeqCst).
    
    --> src/lockfree.tri:42:5
       |
    42 |     atomic_flag.store(true, Ordering.Relaxed)
       |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ relaxed publish
    43 |     // Other thread reads atomic_flag and expects published data visible
    
    Suggested fixes:
    
    [Fix 1] Use Release ordering for publish (most common):
    Change `Ordering.Relaxed` to `Ordering.Synchronized`
    
    [Fix 2] Use SeqCst when total order matters across all threads:
    Change `Ordering.Relaxed` to `Ordering.Strict`
```

(Note: The specific conditions for when E2530 fires will be designed in ADR-0028 — too complex for ADR-0026 v2.)

---

## §5 — Capability Gates

**Lock:** All access to thread primitives must be via a capability declared in `dao.package`. Audit-friendly per [ADR-0018](0018-capability-loader-semantics.md).

### 5.1 — Capability inventory for concurrency

| Capability | Allows | Audience |
|---|---|---|
| `sys::raw_thread` | OS thread creation, syscall wrappers (clone, pthread_create) | Kernel/embedded |
| `sys::atomic` | Atomic primitive operations with non-default ordering | Lock-free authors |
| `dev::ffi` | Calling C concurrency APIs (pthread, semaphore, condvar) | FFI bindings |
| `dev::raw_memory` | Raw pointer arithmetic, bypassing `&+` tracking | Kernel-level shared state |
| `dev::reinterpret` | Bit-casting between atomic and non-atomic types | Niche kernel work |

The stdlib `std.concurrency.green` declares the `sys::raw_thread` capability internally. User application code using the stdlib **does not need** to declare the capability — the capability boundary exists at the stdlib level.

### 5.2 — Application code (usr::) does not see raw thread

```triet
// dao.package — NO capabilities needed
module usr.app

from std.concurrency.green import spawn, scope
from std.concurrency.channel import Channel

function load_users(ids: &0 Vector<UserId>) -> Vector<User> = {
    // No capability declaration. stdlib handles it internally.
    scope.run(|s| {
        let (tx, rx): Channel<User> = Channel.bounded(16)
        for id in ids {
            s.spawn(|| { tx.send(fetch_user(id)) })
        }
        rx.collect(length(ids))
    })
}
```

### 5.3 — Kernel code (sys::) sees raw thread

```triet
// dao.package
capabilities {
    sys::raw_thread: grant
    dev::ffi: grant
ast   sys::atomic: grant
}

module sys.kernel.driver.net

// Custom scheduler — bypasses stdlib entirely
public function spawn_kthread<F: Send>(
    name: &+ String,
    work: F
) -> KThreadHandle~KernelError where F: function() -> Unit = {
    // Direct syscall — capability gate enforced
    sys.raw_thread.create_with_name(name, work)
}
```

The `sys::raw_thread` capability is only granted when `dao.package` declares it. An auditor reading a single file can identify which modules touch thread primitives.

---

## §6 — Rejected Alternatives (NO scheduler keywords in core)

**Lock:** The following keywords **do not exist** in the Triet core language. Anyone requiring equivalent semantics must implement them in the stdlib or an external library.

| Keyword | Reason for Refusal | Alternative |
|---|---|---|
| `async` | Viral coloring problem | Functions remain functions (uniform color) |
| `await` | Same as `async` | Block naturally (runtime handles it) |
| `spawn` | Assumes a runtime | stdlib function: `std.concurrency.green.spawn(...)` |
| `parallel { }` | Assumes a scheduler | stdlib function: `std.concurrency.scope.run(\|s\| { ... })` |
| `actor` | Assumes a mailbox runtime | stdlib struct: `std.concurrency.actor.Actor<T>` |
| `receive` | Same as `actor` | Method on the `Actor` type |
| `select` | Assumes a specific channel implementation | stdlib function: `std.concurrency.channel.select(...)` |
| `yield` (for coroutine) | Assumes a coroutine runtime | Generators built on stdlib |
| `go` (Go-style) | Assumes a goroutine runtime | Same as `spawn` |

`actor`/`spawn`/`send`/`receive` appearing in Triet code are identifiers or function/method names from the stdlib, **not language keywords**.

### 6.1 — Rationale for hard refusal vs. optional keywords

Optional keywords (enabled only via `#![feature(async)]`) were refused because:

1. **Brand consistency:** Triet is a language, not a "feature soup."
2. **AI-friendly:** Fewer concepts mean AI can generate correct code more easily.
3. **Kernel writability:** Every keyword carries a hidden runtime assumption.
4. **Long-term simplicity:** Every feature the compiler does *not* have is a feature that does not need to be documented or maintained.

---

 $\text{§7 — Memory Layout (ObjectHeader Reuse)}$

**Lock:** Every heap allocation on a binary target contains an 8-byte `ObjectHeader` [`refcount: u32 | reserved: u32`] per [ADR-0022 §4.4 + crate `triet-core::memory`]. The refcount is automatically atomically incremented/decremented at the `Send` boundary for `&+ T` (frozen).

### 7.1 — Binary target

```text
HEADER (8 bytes)        BODY (sizeof(T))
[ refcount | reserved ] [ user fields ... ]
```

Atomic ops (LL/SC ARM, LOCK XADD x86) cost ~5-15 ns. We skip this for static / frozen-forever objects via sentinels (`u32::MAX` / `u32::MAX-1`) — see `triet-core::memory`.

### 7.2 — Ternary native target (v∞)

54-trit header (6 Tryte = 2 Integer):

```text
HEADER (54 trit)                BODY
[ refcount: Integer | reserved: Integer ] [ user fields ... ]
```

Negative sentinels: `-1` = static, `-2` = frozen forever. Atomic ops check `current < 0` to skip the refcount entirely.

This provides 880× capacity vs. binary at the same word-alignment ($3^{26} \approx 3.8 \times 10^{12}$ vs. $2^{32} \approx 4.3 \times 10^9$).

### 7.3 — Layout invariant across all schedulers

This layout **does not depend** on the scheduler. Green-thread schedulers, OS-thread schedulers, and kernel schedulers all see the same `ObjectHeader`. Cross-scheduler interoperability (e.g., an app thread sending a frozen owner to a kernel thread) works correctly because the layout is invariant.

---

## §8 — BYOS Philosophy

**Lock:** The Triet core language **does not mandate** a scheduler. It provides primitives, not policy.

### 8.1 — 3-tier architecture

| Tier | Audience | Provides |
|---|---|---|
| **Core language** | Compiler + runtime authors | Send rules + Atomic + capability + linear ownership |
| **stdlib `std.concurrency.*`** | `usr::` app developers | Reference scheduler (green-thread) + channels + scope |
| **Kernel/embedded** | `sys::`/`dev::` developers | Custom scheduler (Linux kthread, RTOS, interrupt handler) |

The stdlib tier is written in Triet itself and uses the `sys::raw_thread` capability. The kernel tier bypasses the stdlib entirely, using raw capabilities and FFI.

### 8.2 — Compile-time guarantees (universal)

Regardless of the scheduler, the compiler enforces:

1. **No data race** — linear ownership (`&+` is unique) + Send rules (prohibiting `&0`/`&-` cross-thread).
2. **No use-after-free** — Send rules + lifetime tracking.
3. **Atomic ordering** — incorrect ordering triggers E2530 (planned).
4. **Capability audit** — every thread primitive is explicitly declared.

### 8.3 — Scheduler determines (runtime)

- Thread creation cost (1KB green vs. 8KB OS thread).
- Scheduling policy (FIFO, priority, work-stealing, cooperative, preemptive).
- Cancellation semantics.
- Channel buffer behavior.
- Memory allocator interaction.

### 8.4 — Comparison with Rust kernel work

| Aspect | Rust kernel | Triet v0.8 BYOS |
|---|---|---|
| async runtime | Refused (only Embassy for embedded) | Refused (BYAS) |
| Thread primitives | Linux kernel C wrappers (kthread, workqueue) | Capability `sys::raw_thread` + `dev::ffi` |
| Atomic primitives | `core::sync::atomic` | Triet core `Atomic<T>` family |
| Race safety | Borrow checker + `Send + Sync` traits | Linear ownership + Send rules (ADR-0026 §2) |
| Custom scheduler | Bare metal scheduler implementations are rare | Encouraged — share via stdlib alternatives |

Triet goes further than Rust: even `async`/`await` are not keywords. **App developers and kernel writers both use Triet syntax; they differ only in the use of stdlib vs. raw capabilities.**

### 8.5 — Trust and Verification

Triet **trusts** that the kernel writer knows better than the language.

- **Trust:** Scheduler correctness (fairness, deadlock-freedom, priority logic).
- **Verify:** Memory safety + race-freedom (compile-time, via ADR-0022/0025 + §2 of this ADR).

A user can write a broken scheduler (e.g., a priority inversion bug), but:
- Send rules are still enforced $\rightarrow$ no data race regardless.
- Linear ownership is still enforced $\rightarrow$ no use-after-free.
- The `sys::raw_thread` capability serves as the audit point.

This is the **correct level of trust**: trust the expert kernel writer, but let the compiler enforce the memory safety boundary.

---

## §9 — stdlib Reference (pointer, not semantic spec)

**Lock:** The stdlib `std.concurrency.*` is a **reference implementation**, not a language specification. Users may replace it with a custom scheduler.

### 9.1 — Planned stdlib modules (v0.9+)

| Module | Provides |
|---|---|
| `std.concurrency.green` | M:N green thread scheduler (Go-style) |
| `std.concurrency.channel` | Typed channels (bounded/unbounded MPMC) |
| `std.concurrency.scope` | Structured concurrency wrapper (no goroutine leaks) |
| `std.concurrency.actor` | Actor pattern (struct + message-passing API) |
| `std.concurrency.mutex` | `Mutex<T>` + `RwLock<T>` built on Atomics |
| `std.concurrency.future` | Future abstraction (NOT tied to async/await) |

Implementation is deferred until post-v0.8. v0.8 only ships core primitives (§2 Send rules + §4 Atomic placeholder + §5 capabilities).

### 9.2 — Alternative scheduler examples

The community may publish:
- `triet-rtos` — RTOS-style scheduler (priority-based preemptive).
- `triet-embassy` — embedded async-style (no heap, no thread).
- `triet-linux` — Linux kernel module wrapper (kthread + workqueue).
- `triet-uring` — io_uring-based async I/O.

Each alternative is an independent crate-pack, using the same Send rules, Atomics, and capabilities. Cross-crate-pack interoperability is enabled by the layout invariant (§7).

---

## §10 — Implementation Phasing

| Version | Scope |
|---|---|
| **v0.8** | §2 Send rules + §4 Atomic placeholder (type signatures only) + §5 capabilities declared (no enforcement). E2500 `NotSendCannotCrossBoundary` fires for obvious `&0`/`&-` violations. |
| **v0.9** | Full Send derivation including generics (monomorphization-time check). E2510 scope-ref leakage. E2520 mutable-share anti-pattern. Atomic primitive types implemented (ADR-0028). |
| **v0.10** | stdlib `std.concurrency.*` reference implementation (green-thread scheduler + channels + scope). E2530 atomic ordering. |
| **v1.0** | Stable concurrency primitives API. Multiple scheduler alternatives encouraged. |
| **post-v1.0** | Kernel-specific examples (Triet-on-Linux as a kernel module proof of concept). |

v0.8 prioritizes bringing **semantic locks** in early, deferring **enforcement implementation** to v0.9+. Send rules are the most critical gate — they must be verified in v0.8.

---

## §11 — Out of Scope (deferred to separate ADRs)

- **Detailed Atomic primitive design** — ADR-0028 (TBD)
- **`std.concurrency.green` scheduler implementation** — stdlib doc (post-v0.9)
- **`std.concurrency.channel` semantics** — stdlib doc (post-v0.9)
- **Actor pattern as stdlib** — stdlib doc (post-v0.9)
- **Cancellation propagation mechanism** — depends on the scheduler (per-scheduler choice)
- **Distributed actors / cross-node** — post-v1.0
- **io_uring / epoll integration** — left to alternative scheduler authors
- **Structured concurrency formal model** — stdlib doc

---

## §12 — References

- [ADR-0022 — Trit-Balanced Ownership](0022-trit-balanced-ownership.md) (parent — 5 reference forms, linear ownership)
- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (sibling — intra-context enforcement, E2420 use-after-move)
- [ADR-0027 — Diagnostic Format Standard](0027-diagnostic-format-standard.md) (E2500-E2599 follow §2 format)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (dao.package declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` for thread handle results)
- [VISION §3.5 — Capability + namespace](../../VISION.md)
- [VISION §6 — Refuse over guess](../../VISION.md) (philosophical alignment with §6 refused list)
- [ROADMAP §v0.8 — Concurrency Foundation](../../ROADMAP.md) (this ADR is foundational for v0.8 phase)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (update `triet::concurrency::E25XX` when ADR lands)
- Future ADR-0028 — Atomic Primitives (TBD)
- `triet-core::memory::ObjectHeader` (crate, layout per §7)
