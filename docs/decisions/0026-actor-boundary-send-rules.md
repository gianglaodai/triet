# ADR 0026 — Concurrency Primitives & Send Rules (Bring Your Own Scheduler)

**Trạng thái:** **Locked v2** (promoted via v0.8.x.review 2026-05-28; supersedes 2026-05-26 v1 "Actor Boundary & Send Rules" per BYOS pivot). Sibling của [ADR-0022](0022-trit-balanced-ownership.md) + [ADR-0025](0025-borrow-checker-rules.md). v0.8 đã ship Send derivation algorithm cho 13 type categories (`triet-typecheck::types::Type::is_send()`) + E2500 fires + capability schema mở rộng. Locks language-level concurrency primitives + compile-time Send rules. **Refuses** baking scheduler/runtime vào core language — kernel writers bring their own. Diagnostic format follow [ADR-0027](0027-diagnostic-format-standard.md).

> **2026-05-29 Addendum (v0.9.0.1):** §4 placeholder design refined by [ADR-0028](0028-atomic-primitive.md). The `&+ mutable Atomic<T>` signature in §4.3 store/swap/compare_exchange is **superseded** by `&+ Atomic<T>` per [ADR-0028 §5](0028-atomic-primitive.md#5--reference-form-for-atomic-operations-resolves-adr-0026-v2-43-contradiction) — atomicity = interior mutability via raw hardware atomic instructions, mirroring Rust `&AtomicU64` pattern. Cross-thread atomic share REQUIRES frozen ref (per §2.1 row 7 Send rule); ADR-0026 v2 §4.3's `&+ mutable` form was an internal contradiction not exercised by v0.8 placeholder ship. ADR-0026 v2 body NOT edited per project ADR immutability rule; ADR-0028 §5 is source-of-truth for atomic operation signatures.

**Issue:** v1 (2026-05-26) đã propose `actor`/`receive`/`send`/`spawn` keywords + mailbox runtime. Author 2026-05-26 (cùng ngày, sau khi reviewing) chỉ ra contradiction cốt lõi: **Triết mục tiêu viết kernel** (per VISION §3.5), nhưng v1 bake green-thread/actor runtime vào core language — kernel không có user-space runtime để host scheduler. Linux Rust modules phải dùng C scheduler (kthread, workqueue). Rust async không có trong kernel. Triết v1 lặp lại sai lầm này.

Insight cốt lõi:

> **"Chúng ta cần giả định là giải pháp đa luồng của chúng ta không tốt, sẽ luôn có lập trình viên làm tốt hơn. Phần đa luồng này có thể coi như tiện ích, chúng ta phải cung cấp khả năng để 1 lập trình viên khác triển khai được hệ thống quản lý luồng và bản thân người sử dụng Triết sau này sẽ dùng hệ thống quản lý luồng đó."**

v2 reframe hoàn toàn: **Bring Your Own Scheduler (BYOS)**. Core language chỉ provide universal primitives + compile-time safety rules. Scheduler/runtime là stdlib (cho usr:: app) hoặc external (cho kernel/embedded). Mọi `actor`/`spawn`/`async`/`await`/`parallel` keyword bị refuse khỏi core.

---

## §1 — Goals & non-goals

### 1.1 — Goals

1. **Universal Send rules** — compile-time derivation cho mọi scheduler.
2. **Linear ownership across thread boundary** — không có shared mutable cross-thread (compile-time enforced từ ADR-0022 D1).
3. **Atomic primitives** — language-level types cho lock-free programming.
4. **Capability gates** — `sys::raw_thread`, `sys::atomic`, `dev::ffi` cho thread primitives.
5. **No-mandate-scheduler** — kernel writers tự build scheduler, share lại cho cộng đồng.
6. **Compile-time race-freedom** — bất kể scheduler nào, data race là impossible.

### 1.2 — Non-goals (refused, không phải defer)

- ❌ `async`/`await` keywords — viral coloring, không kernel-safe
- ❌ `spawn` keyword — assumes runtime
- ❌ `parallel { }` block — assumes scheduler
- ❌ `actor`/`receive` keywords — assumes mailbox runtime
- ❌ Channels as built-in syntax — channels = stdlib types, không phải language keyword
- ❌ Implicit heap allocator coupling — kernel có allocator riêng
- ❌ Built-in green thread scheduler trong core — stdlib reference impl only

Tất cả các từ trên có thể xuất hiện trong stdlib library code, **không phải language keyword**.

### 1.3 — Error code namespace E25XX

Reserved range **E2500–E2599** cho concurrency diagnostics. Phân bổ:

| Range | Category |
|---|---|
| E2500–E2509 | Send derivation violations |
| E2510–E2519 | Scope-ref / weak-ref boundary violations |
| E2520–E2529 | Mutable-share anti-pattern |
| E2530–E2539 | Atomic memory ordering violations |
| E2540–E2549 | Reserved: capability mismatch ở thread primitives |
| E2550–E2599 | Reserved future expansion |

Module path: `triet::concurrency::E25XX`. CLAUDE.md cập nhật khi ADR land.

---

## §2 — Send Derivation Rules (compile-time, universal)

**Lock:** Mỗi type T có property compile-time `Send(T)` — boolean, derived theo structure. User không gõ trait bound; compiler suy. Áp dụng tại **mọi function boundary** mà param có annotation `: Send` (hoặc tương đương trait bound).

### 2.1 — Rules quy nạp

| Type | Send(T) |
|---|---|
| Primitive value types (`Trit`, `Tryte`, `Integer`, `Long`, `Trilean`, `Unit`) | ✅ Always Send |
| Tuples `(A, B, ...)` | ✅ Send iff all components Send |
| `T?` nullable | ✅ Send iff T Send |
| `T~E`, `T?~E` outcome | ✅ Send iff T Send and E Send |
| `Vector<T>`, `Map<K, V>`, `Set<T>` | ✅ Send iff elements Send |
| User struct `S { f1: T1, f2: T2, ... }` | ✅ Send iff all fields Send |
| User enum / variant | ✅ Send iff all variants' payload Send |
| `&+ T` (frozen owner) | ✅ Send iff T Send. Refcount ngầm at boundary (per §7) |
| `&+ mutable T` | ✅ Send iff T Send. Linear move (single owner thread) |
| `&0 T`, `&0 mutable T` (scope borrow) | ❌ Never Send |
| `&- T` (weak observer) | ❌ Never Send |
| `Atomic<T>` (where T is value type) | ✅ Always Send (atomic by definition) |
| Function types `fn(...) -> ...` | ✅ Send iff all captures Send (defer closure ADR) |
| Raw thread handles (`sys::raw_thread.Handle`) | ✅ Send (kernel concern) |

### 2.2 — Tại sao `&0` và `&-` NEVER Send

`&0` là scope-bound (ADR-0022 §2). Scope thuộc về 1 execution context cụ thể — không khái niệm "scope" cross-thread. Cho phép `&0` cross thread sẽ phá compile-time invariant của ADR-0025 §2 (NLL exclusivity per-place trong cùng CFG).

`&- T` weak observer là compile-time tracked (ADR-0022 §9). Trace từ weak về `&+` chỉ valid trong 1 execution context. Cross thread → owner trail không liên tục.

### 2.3 — Application site: trait bound `: Send`

Send rules áp dụng tại function boundary với explicit annotation. Ví dụ stdlib `std.concurrency.green.spawn`:

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

Captures inside closure `||` typecheck against `Send` bound. Sai → E2500.

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

Khi function generic, Send check tại call site monomorphization. Compile-time, không runtime.

```triet
let r: &0 Vector<UserId> = &0 ids
spawn(|| process(r))              // E2500 — &0 Vector not Send (monomorphization-time)
```

---

## §3 — Linear Ownership Across Boundary

**Lock:** Linear ownership từ ADR-0022 D1 + move semantics từ ADR-0025 §5 áp dụng nguyên trạng tại thread boundary. Không có shared mutable cross-thread — period.

### 3.1 — `&+ mutable T` qua boundary = move

```triet
let mutable job: &+ mutable Job = build_job()
spawn(|| process(job))            // job MOVED into closure
print(job.priority)                // E2420 UseAfterMove
```

Mirror Rust `Send + !Sync` types. Zero runtime cost — same allocation, different owner thread.

### 3.2 — `&+ T` (frozen) qua boundary = refcount-mediated share

Khi `&+ T` (frozen owner) được capture vào Send closure:

```triet
let config: &+ Config = load_config()
spawn(|| use_config(config))      // refcount tăng atomic, sender giữ handle
print(config.version)             // OK — sender still has handle
```

Behind the scenes:
- ObjectHeader refcount (per §7) tăng atomic khi closure được Send (= cross boundary).
- Sender thread + spawned thread đều có `&+ Config` handle.
- Refcount giảm khi mỗi handle drop. Memory freed khi refcount = 0.

**User-visible:** không thấy refcount. Chỉ thấy share-able vì frozen.

### 3.3 — Refuse shared mutable cross-thread

```triet
let mutable counter: &+ mutable Counter = Counter.new()
spawn(|| increment(counter))      // OK — counter moved
spawn(|| increment(counter))      // E2420 UseAfterMove
```

Để share mutable state, dùng:
1. **Atomic primitive** (xem §4) — lock-free, hardware-supported
2. **Wrap trong dedicated "owner thread"** — gom mutable state vào 1 execution context, communicate qua message passing (stdlib `std.concurrency.channel`)
3. **Stdlib `Mutex<T>`** — built on Atomic, không phải language built-in

### 3.4 — Refuse list (no language-level escape hatch)

Triết core **không có** capability để bypass §3.3. Không có `dev::cross_thread_mut`. Lý do: Java synchronized hell + Rust Arc<Mutex> panic — refuse-over-guess (VISION §6).

Nếu user **thực sự** cần shared mutable (kernel-level shared state, lock-free queue), dùng:
- Atomic primitives (§4) — compile-time safe
- Capability `dev::raw_memory` + `sys::atomic` — kernel responsibility

---

## §4 — Atomic Primitive Types

**Lock (placeholder design — chi tiết ADR-0028 hoặc Addendum):** Triết core có `Atomic<T>` family cho lock-free programming. T phải là value type với hardware atomic support. Memory ordering enum.

### 4.1 — Type family

```triet
Atomic<Integer>     // 27-trit atomic on ternary native, i32/i64 on binary
Atomic<Tryte>       // 9-trit atomic on ternary native, i8/i16 on binary
Atomic<Trit>        // 1-trit atomic
Atomic<Trilean>     // logic atomic (3-state)
Atomic<Pointer>     // for raw_memory capability — kernel only
```

Composite types (struct, Vector, Outcome) **không** atomic-able trực tiếp. User wrap trong Mutex hoặc design lock-free DS.

### 4.2 — Memory ordering

3 levels (mapping hardware concepts):

| Triết | C++ equivalent | Hardware semantics |
|---|---|---|
| `Ordering.Relaxed` | `memory_order_relaxed` | No synchronization, atomic only |
| `Ordering.Synchronized` | `memory_order_acq_rel` | Acquire on load, Release on store |
| `Ordering.Strict` | `memory_order_seq_cst` | Total order across all threads |

5-level C++ model (Relaxed/Consume/Acquire/Release/AcqRel/SeqCst) giảm xuống 3 — đủ cho 95% use case. Kernel writer cần Consume riêng → capability `dev::raw_memory` mở quyền dùng raw hardware intrinsics.

**Tại sao 3 thay vì 5?** Brand-fit ternary identity. Trade-off: Consume + Acquire merged (Consume rarely useful in practice — most compilers compile it as Acquire anyway).

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

Note: `Atomic<T>` itself **always Send** (per §2.1 table). Cho phép share atomic handle giữa threads — đó là toàn bộ điểm của atomic.

### 4.4 — E2530 InvalidAtomicOrdering

```text
E2530 InvalidAtomicOrdering
    Atomic operation `store` with `Ordering.Relaxed` is unsafe when the
    store publishes data accessed by other threads. Use `Ordering.Synchronized`
    (Release) or `Ordering.Strict` (SeqCst).
    
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

(Note: chi tiết khi nào E2530 fires sẽ design ở ADR-0028 — quá phức tạp cho ADR-0026 v2.)

---

## §5 — Capability Gates

**Lock:** Mọi access tới thread primitives qua capability declared trong `dao.package`. Audit-friendly per [ADR-0018](0018-capability-loader-semantics.md).

### 5.1 — Capability inventory cho concurrency

| Capability | Cho phép | Audience |
|---|---|---|
| `sys::raw_thread` | OS thread creation, syscall wrapper (clone, pthread_create) | Kernel/embedded |
| `sys::atomic` | Atomic primitive operations với non-default ordering | Lock-free authors |
| `dev::ffi` | Call C concurrency APIs (pthread, semaphore, condvar) | FFI bindings |
| `dev::raw_memory` | Raw pointer arithmetic, bypass `&+` tracking | Kernel-level shared state |
| `dev::reinterpret` | Bit-cast giữa atomic and non-atomic | Niche kernel work |

Stdlib `std.concurrency.green` declare `sys::raw_thread` capability internally. User app code dùng stdlib **không cần** declare capability — capability boundary là tại stdlib level.

### 5.2 — Application code (usr::) doesn't see raw thread

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
    sys::atomic: grant
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

Capability `sys::raw_thread` chỉ cấp khi `dao.package` declare. Auditor đọc 1 file = biết module nào touch thread primitives.

---

## §6 — Refuse List (NO scheduler keywords in core)

**Lock:** Các keyword sau **không có** trong Triết core language. Bất kỳ ai muốn semantics tương đương phải implement trong stdlib hoặc external library.

| Keyword | Lý do refuse | Alternative |
|---|---|---|
| `async` | Viral coloring problem | Functions stay functions (uniform color) |
| `await` | Same as `async` | Block naturally (runtime handles) |
| `spawn` | Assumes runtime | stdlib function: `std.concurrency.green.spawn(...)` |
| `parallel { }` | Assumes scheduler | stdlib function: `std.concurrency.scope.run(\|s\| { ... })` |
| `actor` | Assumes mailbox runtime | stdlib struct: `std.concurrency.actor.Actor<T>` |
| `receive` | Same as `actor` | Method on Actor type |
| `select` | Assumes specific channel impl | stdlib function: `std.concurrency.channel.select(...)` |
| `yield` (for coroutine) | Assumes coroutine runtime | Generators built on stdlib |
| `go` (Go-style) | Assumes goroutine runtime | Same as `spawn` |

`actor`/`spawn`/`send`/`receive` xuất hiện trong code Triết = identifier hoặc function/method name từ stdlib, **không phải language keyword**.

### 6.1 — Tại sao refuse cứng thay vì optional keyword

Optional keyword (chỉ enable khi có `#![feature(async)]`) bị refuse vì:

1. **Brand consistency:** Triết là 1 ngôn ngữ, không phải feature soup
2. **AI-friendly:** ít concept hơn = AI dễ generate đúng
3. **Kernel writability:** keyword nào cũng có hidden runtime assumption
4. **Long-term simplicity:** mỗi feature compiler không có là 1 feature không cần document/maintain

---

## §7 — Memory Layout (ObjectHeader Reuse)

**Lock:** Mọi heap allocation trên binary target có 8-byte ObjectHeader [refcount: u32 | reserved: u32] per [ADR-0022 §4.4 + crate `triet-core::memory`]. Refcount tự động atomic increment/decrement tại Send boundary cho `&+ T` frozen.

### 7.1 — Binary target

```text
HEADER (8 bytes)        BODY (sizeof(T))
[ refcount | reserved ] [ user fields ... ]
```

Atomic ops (LL/SC ARM, LOCK XADD x86) cost ~5-15 ns. Skip cho static / frozen-forever via sentinels (u32::MAX / u32::MAX-1) — xem `triet-core::memory`.

### 7.2 — Ternary native target (v∞)

54-trit header (6 Tryte = 2 Integer):

```text
HEADER (54 trit)                BODY
[ refcount: Integer | reserved: Integer ] [ user fields ... ]
```

Negative sentinels: -1 = static, -2 = frozen forever. Atomic op kiểm tra `current < 0` skip refcount entirely.

880× capacity vs binary tại same word-alignment (3²⁶ ≈ 3.8 × 10¹² vs 2³² ≈ 4.3 × 10⁹).

### 7.3 — Layout invariant across all schedulers

Layout này **không phụ thuộc** scheduler. Green-thread scheduler, OS-thread scheduler, kernel scheduler đều thấy cùng ObjectHeader. Cross-scheduler interop (vd: app thread send frozen owner to kernel thread) hoạt động đúng vì layout invariant.

---

## §8 — BYOS Philosophy

**Lock:** Triết core language **không mandate** scheduler. Cung cấp primitives, không cung cấp policy.

### 8.1 — 3-tier architecture

| Tier | Audience | Provides |
|---|---|---|
| **Core language** | Compiler + runtime authors | Send rules + Atomic + capability + linear ownership |
| **stdlib `std.concurrency.*`** | usr:: app developers | Reference scheduler (green-thread) + channels + scope |
| **Kernel/embedded** | sys::/dev:: developers | Custom scheduler (Linux kthread, RTOS, interrupt handler) |

stdlib tier viết bằng Triết itself + dùng capability `sys::raw_thread`. Kernel tier bypass stdlib hoàn toàn, dùng raw capability + FFI.

### 8.2 — Compile-time guarantees (universal)

Bất kể scheduler nào, compiler enforce:

1. **No data race** — linear ownership (`&+` unique) + Send rules (cấm `&0`/`&-` cross-thread)
2. **No use-after-free** — Send rules + lifetime tracking
3. **Atomic ordering** — wrong ordering = E2530 (planned)
4. **Capability audit** — mọi thread primitive declared explicitly

### 8.3 — Scheduler determines (runtime)

- Thread creation cost (1KB green vs 8KB OS thread)
- Scheduling policy (FIFO, priority, work-stealing, cooperative, preemptive)
- Cancellation semantics
- Channel buffer behavior
- Memory allocator interaction

### 8.4 — So sánh với Rust kernel work

| Aspect | Rust kernel | Triết v0.8 BYOS |
|---|---|---|
| async runtime | Refuse (chỉ embassy cho embedded) | Refuse (BYOS) |
| Thread primitives | Linux kernel C wrappers (kthread, workqueue) | Capability `sys::raw_thread` + `dev::ffi` |
| Atomic primitives | `core::sync::atomic` | Triết core `Atomic<T>` family |
| Race safety | Borrow check + `Send + Sync` traits | Linear ownership + Send rules (ADR-0026 §2) |
| Custom scheduler | Bare metal scheduler implementations rare | Encouraged — share via stdlib alternatives |

Triết đi xa hơn Rust: ngay cả `async`/`await` cũng không phải keyword. **App developer + kernel writer cùng dùng cú pháp Triết, khác nhau ở stdlib vs raw capability.**

### 8.5 — Trust + verify

Triết **tin** kernel writer biết tốt hơn ngôn ngữ.

- **Trust:** scheduler correctness (fairness, deadlock-freedom, priority logic)
- **Verify:** memory safety + race-freedom (compile-time, từ ADR-0022/0025 + §2 này)

User có thể viết broken scheduler (vd: priority inversion bug), nhưng:
- Send rules vẫn enforce → no data race regardless
- Linear ownership vẫn enforce → no use-after-free
- Capability `sys::raw_thread` là audit point

Đây là **đúng level of trust**: trust expert kernel writer, nhưng compiler vẫn enforce memory safety boundary.

---

## §9 — stdlib Reference (pointer, không spec ngữ nghĩa)

**Lock:** stdlib `std.concurrency.*` là **reference implementation**, không phải language spec. Người dùng có thể replace bằng custom scheduler.

### 9.1 — Planned stdlib modules (v0.9+)

| Module | Provides |
|---|---|
| `std.concurrency.green` | M:N green thread scheduler (Go-style) |
| `std.concurrency.channel` | Typed channels (bounded/unbounded MPMC) |
| `std.concurrency.scope` | Structured concurrency wrapper (no goroutine leak) |
| `std.concurrency.actor` | Actor pattern (struct + message-passing API) |
| `std.concurrency.mutex` | `Mutex<T>` + `RwLock<T>` built on Atomic |
| `std.concurrency.future` | Future abstraction (NOT tied to async/await) |

Implementation defer post-v0.8. v0.8 chỉ ship core primitives (§2 Send rules + §4 Atomic placeholder + §5 capabilities).

### 9.2 — Alternative scheduler examples

Cộng đồng có thể publish:
- `triet-rtos` — RTOS-style scheduler (priority-based preemptive)
- `triet-embassy` — embedded async-style (no heap, no thread)
- `triet-linux` — Linux kernel module wrapper (kthread + workqueue)
- `triet-uring` — io_uring-based async I/O

Mỗi alternative là crate-pack độc lập, dùng cùng Send rules + Atomic + capability. Cross-crate-pack interop nhờ layout invariant (§7).

---

## §10 — Implementation Phasing

| Version | Scope |
|---|---|
| **v0.8** | §2 Send rules + §4 Atomic placeholder (type signatures only) + §5 capabilities declared (no enforcement). E2500 NotSendCannotCrossBoundary fires for obvious `&0`/`&-` violations. |
| **v0.9** | Full Send derivation including generics (monomorphization-time check). E2510 scope-ref leakage. E2520 mutable-share anti-pattern. Atomic primitive types implemented (ADR-0028). |
| **v0.10** | stdlib `std.concurrency.*` reference implementation (green-thread scheduler + channels + scope). E2530 atomic ordering. |
| **v1.0** | Stable concurrency primitives API. Multiple scheduler alternatives encouraged. |
| **post-v1.0** | Kernel-specific examples (Triết-on-Linux as kernel module proof of concept). |

v0.8 ưu tiên **lock semantic** vào sớm, defer **enforcement implementation** sang v0.9+. Send rules là gate quan trọng nhất — verify ngay v0.8.

---

## §11 — Out of Scope (defer riêng ADRs)

- **Atomic primitive design chi tiết** — ADR-0028 (TBD)
- **`std.concurrency.green` scheduler implementation** — stdlib doc (post-v0.9)
- **`std.concurrency.channel` semantics** — stdlib doc (post-v0.9)
- **Actor pattern as stdlib** — stdlib doc (post-v0.9)
- **Cancellation propagation mechanism** — depends on scheduler (per-scheduler choice)
- **Distributed actors / cross-node** — post-v1.0
- **io_uring / epoll integration** — alternative scheduler authors
- **Structured concurrency formal model** — stdlib doc

---

## §12 — Tham chiếu

- [ADR-0022 — Trit-Balanced Ownership](0022-trit-balanced-ownership.md) (parent — 5 reference forms, linear ownership)
- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (sibling — intra-context enforcement, E2420 use-after-move)
- [ADR-0027 — Diagnostic Format Standard](0027-diagnostic-format-standard.md) (E2500-E2599 follow §2 format)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (dao.package declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` for thread handle results)
- [VISION §3.5 — Capability + namespace](../../VISION.md)
- [VISION §6 — Refuse over guess](../../VISION.md) (philosophical alignment with §6 refuse list)
- [ROADMAP §v0.8 — Concurrency Foundation](../../ROADMAP.md) (this ADR foundational for v0.8 phase)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (cập nhật `triet::concurrency::E25XX` khi ADR land)
- Future ADR-0028 — Atomic Primitives (TBD)
- `triet-core::memory::ObjectHeader` (crate, layout per §7)
