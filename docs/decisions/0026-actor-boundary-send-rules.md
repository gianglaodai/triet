# ADR 0026 — Actor Boundary & Send Rules (Luật Vượt Ranh Giới Actor)

**Trạng thái:** **Draft**. Sibling của [ADR-0022](0022-trit-balanced-ownership.md) + [ADR-0025](0025-borrow-checker-rules.md). Locks compile-time rules cho cái gì pass được qua message giữa các actor — interplay giữa ownership model (ADR-0022) và concurrency primitive (v0.8 phase). Định nghĩa namespace error code mới **E25XX** cho concurrency-boundary diagnostics. Diagnostic format follow [ADR-0027](0027-diagnostic-format-standard.md).

**Issue:** [ROADMAP §v0.8](../../ROADMAP.md) chốt favor **Actor + structured concurrency** cho concurrency model. Author 2026-05-26 trong design session với AI assistant chốt ownership = S6 (Rust-strict static + ternary brand + capability-as-unsafe + no annotations). Hai mảng này gặp nhau tại 1 điểm cốt lõi: **cái gì pass được qua actor message boundary** và **lifecycle của ownership khi cross boundary**.

Decisions D6, D7 đã chốt design session:

- **D6** — Refcount **tự động ngầm** cho `&+ T` (frozen owner) khi cross-actor send. Frozen ≡ immutable ≡ no race ≡ safe share. Sender giữ handle, receiver có handle riêng, mem release khi cả hai cùng drop. **Không expose refcount semantics ra user-facing language.**
- **D7** — `&+ mutable T` **move trực tiếp** qua send. Không cần keyword `iso` riêng. Sender mất quyền (E2420 use-after-move nếu dùng tiếp).

ADR này lock rules cho 2 decision trên + intra-actor aliasing policy + Send derivation + escape hatch policy. **Out of scope**: actor syntax cụ thể (`actor`, `receive`, `spawn`), mailbox impl, scheduler, supervision — tách thành sub-ADRs trong v0.8 phase.

---

## §1 — Context: 2 mảng gặp nhau ở message boundary

### 1.1 — Hai concept giao thoa

Ownership (ADR-0022 §2 — 5 reference forms) + actor concurrency = câu hỏi: khi 1 message bay từ actor A sang actor B, dữ liệu trong message phải tuân thủ rule gì?

Rust giải bằng `Send` + `Sync` traits + lifetime annotations. Triết không có lifetime annotations (ADR-0025 §4) nên Send model phải tự derive từ ownership.

### 1.2 — Nguyên tắc chốt

Bốn lock chính:

1. **Send là compile-time property** — không runtime check.
2. **Send tự derive** từ type structure — user không gõ trait bound thủ công.
3. **Send phụ thuộc ownership form** — `&+ T` immutable vs `&+ mutable` vs `&0` vs `&-` cho kết quả khác nhau.
4. **Intra-actor không relax** — bên trong 1 actor, NLL exclusivity rules từ ADR-0025 vẫn áp dụng đầy đủ.

### 1.3 — Actor syntax: placeholder cho ADR này

ADR-0026 dùng syntax minh họa dưới đây để diễn đạt rule, **không lock cú pháp**. Cú pháp chính thức sẽ vào ADR riêng (v0.8 sub-task):

```triet
public actor Worker {
    mutable jobs: Vector<&+ Job>,
    
    receive Process(job: &+ mutable Job) {
        push(self.jobs, job)
    }
    
    receive Query(question: &+ String, reply: ReplyTo<Integer>) {
        reply.send(self.compute(question))
    }
}

let w: Worker = spawn(Worker::new())
w.send(Process(my_job))
```

`actor`, `receive`, `spawn`, `ReplyTo<T>`, `send` đều **placeholder**. Rule trong ADR này áp dụng dù cú pháp cuối cùng có ra sao.

### 1.4 — Error code namespace E25XX

Reserved range **E2500–E2599** cho actor boundary + concurrency diagnostics. Phân bổ:

| Range | Category |
|---|---|
| E2500–E2509 | Send derivation violations |
| E2510–E2519 | Scope-ref / weak-ref boundary violations |
| E2520–E2529 | Mutable-share anti-pattern |
| E2530–E2539 | Reply channel violations (chi tiết defer) |
| E2540–E2599 | Reserved future expansion (supervision, structured concurrency) |

Module path: `triet::actor::E25XX`. CLAUDE.md cập nhật khi ADR land.

---

## §2 — Send Derivation Rules (compile-time auto-derive)

**Lock:** Mỗi type T có property compile-time `Send(T)` — boolean, derived theo structure. User không gõ trait bound; compiler suy.

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
| `&+ T` (frozen owner) | ✅ Send iff T Send. **D6 path** — refcount ngầm. |
| `&+ mutable T` | ✅ Send iff T Send. **D7 path** — linear move. |
| `&0 T`, `&0 mutable T` (scope borrow) | ❌ Never Send |
| `&- T` (weak observer) | ❌ Never Send |
| Function types `fn(...) -> ...` | ✅ Send iff captures all Send (defer closure ADR) |
| Actor handle types (e.g., `Worker`) | ✅ Always Send — actor refs đều safe pass |

### 2.2 — Tại sao `&0` và `&-` NEVER Send

`&0` là scope-bound (ADR-0022 §2). Scope thuộc về 1 actor cụ thể — không khái niệm "scope" cross-actor. Cho phép `&0` cross actor sẽ phá compile-time invariant của ADR-0025 §2 (NLL exclusivity per-place trong cùng CFG — không thể track CFG cross-actor).

`&-` weak observer là compile-time tracked (ADR-0022 §9). Trace từ weak về `&+` chỉ valid trong 1 actor. Cross actor → owner trail không liên tục → invariant của ADR-0025 §8.3 bị phá.

### 2.3 — E2500 NotSendCannotCrossActorBoundary

```text
E2500 NotSendCannotCrossActorBoundary
    Type `Foo` cannot cross actor boundary because field `bar: &0 String`
    is a scope borrow. Scope borrows are bound to a single actor's
    control-flow graph (ADR-0025 §2).
    
    --> src/example.tri:12:18
       |
    12 |     worker.send(Process(payload))
       |                         ^^^^^^^ payload contains non-Send field
       |
    8  |     public struct Foo {
    9  |         bar: &0 String
       |         ----- this field makes `Foo` non-Send
    10 |     }
    
    Suggested fixes:
    
    [Fix 1] Take ownership of the borrowed data before sending:
    Change `bar: &0 String` to `bar: &+ String`
    
    [Fix 2] Restructure so the borrow stays within the sending actor:
    Refactor `Process` handler to derive `payload` from values, not borrows
    
    [Fix 3] Pass only the necessary owned data through the message:
    Replace `payload` with a message that carries just the owned fields needed
```

### 2.4 — Send với generic type parameter

Khi function/struct generic, Send derives qua bound:

```triet
public function send_anything<T>(target: Worker, msg: T) {
    // Compile-time: nếu T không Send, error tại call site khi T concrete
    target.send(Wrap(msg))
}
```

Compiler check Send tại monomorphization. Nếu user gọi `send_anything<&0 String>(...)` → E2500 tại call site. Không cần explicit `where T: Send` (defer generics ADR cho explicit syntax post-v0.8).

---

## §3 — Mechanics của Move vs Share

**Lock:** `&+ T` (frozen) → refcount share (D6). `&+ mutable T` → linear move (D7). Cả 2 đều cùng cú pháp `send()` nhưng semantics khác.

### 3.1 — D6: Frozen owner cross-actor (refcount ngầm)

```triet
let config: &+ Config = load_config()         // frozen owner, refcount=1
worker_a.send(UseConfig(config))               // refcount=2 (A's handler param + sender)
worker_b.send(UseConfig(config))               // refcount=3 (B's handler param too)
print(config.timeout_ms)                       // OK — sender still has &+ Config
// Refcount drops as each handler returns + sender's variable goes out of scope
```

Behind the scenes:
1. Sender's `&+ Config` is a frozen owner.
2. `send()` takes `&+ Config` (frozen) as argument.
3. Send machinery: clone the handle (refcount++ atomic) and place new handle in receiver's mailbox.
4. Receiver's handler param receives a fresh `&+ Config` handle (refcount counted).
5. When receiver's handler returns: handler param drops → refcount−− atomic.
6. When sender's local variable goes out of scope: refcount−− atomic.
7. Refcount = 0 → destructor runs, memory freed.

**User-visible:** `send()` không consume `&+ T` (frozen). Sender giữ handle. Cả hai bên cùng "own" theo cách user perceive — không thấy refcount.

**Trade-off vs Rust:** Rust `Arc<T>` requires explicit `.clone()` mỗi lần share. Triết hidden refcount **chỉ tại boundary** — local code vẫn linear (D1 từ ADR-0022). Refcount op overhead chỉ phát sinh khi thực sự cross actor. **Không mâu thuẫn D1**: D1 cấm shared ownership trong user-facing language; D6 cho phép refcount **ngầm** ở implementation layer chỉ tại actor boundary, không expose cú pháp clone.

### 3.1.1 — Memory layout: Object header trên phần cứng nhị phân (current target)

**Lock:** Mọi heap allocation trên target nhị phân (v0.8 VM, v0.9 JIT Cranelift, v2.0 AOT LLVM) mang 1 **object header 8 bytes** đặt trước body data. User-visible `&+ T` pointer luôn trỏ vào **body** (sau header), nhất quán với Objective-C / Swift pattern và FFI-friendly.

```text
Heap allocation layout (64-bit binary target):

  Address: HEADER_ADDR          BODY_ADDR = HEADER_ADDR + 8
           |                    |
           v                    v
           [ refcount: u32 | reserved: u32 ] [ user fields ... ]
           |<--- 8 bytes ----------->|       |<-- sizeof(T) -->|
           
&+ T user-visible pointer points HERE ----^
Refcount manipulation goes back 8 bytes to find header.
```

**Trường refcount: u32** — đếm số `&+ T` handle đang sống. Max 2^32 ≈ 4.3 tỷ refs — đủ cho mọi pattern thực tế.

**Trường reserved: u32** — slot tương lai cho: type tag bits (runtime reflection), drop flags, capability audit metadata, hoặc cycle collector mark bit nếu future ADR cần. Hiện tại để 0.

Layout cho phần cứng tam phân native (v∞ trytecode) khác — xem §3.1.5.

### 3.1.2 — Tại sao Scenario A thay vì lazy box-wrapping (Scenario B)

Design session 2026-05-26 evaluate 2 alternative:

| Scenario | Cách | Pros | Cons |
|---|---|---|---|
| **A — Header always (CHỌN)** | Mọi heap alloc có 8-byte header sẵn từ đầu | Layout consistent, FFI-friendly, không pointer relocation, simple runtime | 8 bytes/alloc overhead kể cả khi không cross-actor |
| **B — Box-wrap on first send** | Raw pointer thuần. Khi lần đầu send cross-actor, runtime wrap vào Arc-like struct, update pointer | Zero overhead cho actor-free code | **Fatal:** pointer relocation phá invariant compile-time của `&-` weak refs (ADR-0022 §9.3). Cũng cần whole-program escape analysis |

Scenario B bị refuse vì:

1. **`&-` weak refs** track address compile-time. Pointer relocation khi wrap → mọi `&-` đang tồn tại trỏ sai address. Phá hoàn toàn invariant ADR-0025 §8.3.
2. **Whole-program escape analysis** không khả thi trong VM v0.8 (interpreter + bytecode tier per VISION §4.3). AOT v2.0+ may revisit.
3. **Runtime complexity** — 2 allocation paths (with/without header) phức tạp hơn 1 path.

Scenario A đánh đổi 8 bytes/alloc lấy:
- Consistent invariant cho mọi heap object.
- Simple runtime model.
- FFI predictability (extern C code thấy `&+ T` = pointer to body, just like any C pointer).

### 3.1.3 — Atomicity của refcount op

Refcount op (inc khi send, dec khi handler return / sender variable expire) **bắt buộc atomic** vì:
- Sender's actor và receiver's actor có thể chạy trên 2 OS thread khác nhau.
- Refcount field truy cập từ ≥ 2 threads → race nếu không atomic.

Triết dùng atomic ops (LL/SC trên ARM, LOCK XADD trên x86) cho refcount inc/dec ở boundary. Cost: ~5-15 ns per op trên hardware hiện đại.

**So sánh với Rust `Arc<T>`:** Rust dùng atomic ops mỗi `.clone()` — every code path that wants to share. Triết atomic op **chỉ tại actor send/drop** — local code không touch refcount. Trên workload "share once, use many times" (phổ biến cho actor pattern), Triết overhead thấp hơn Rust Arc rõ rệt.

### 3.1.4 — Trade-off cho small heap objects

Object header 8 bytes là **fixed overhead per allocation**. Cho object lớn (struct với 10+ fields, Vector, String) → overhead < 5%. Cho object nhỏ (heap-allocated Tryte 1 byte) → overhead 800%.

Mitigation: **value types** (Trit, Tryte, Integer, Long, Trilean, Unit, tuples nhỏ — danh sách ở SPEC §10.3) **không** heap allocate. Chúng nằm trên stack/register, không có header. Chỉ heap-allocated types (String, Vector, struct lớn, user-defined heap types) chịu overhead này.

Post-v1.0 có thể evaluate "small object optimization" (header-elision cho specific size classes) nếu benchmark chỉ ra cần thiết. v0.8-v1.0 giữ uniform 8-byte header — simplicity over micro-optimization, đúng tinh thần "stability over speed".

### 3.1.5 — Memory layout trên phần cứng tam phân native (v∞ trytecode)

**Lock:** Khi phần cứng tam phân xuất hiện (per [VISION §4.3 + ROADMAP v∞](../../VISION.md)), object header **tự nhiên align vào word size 27-trit** của ternary architecture. Header tam phân = **54 trit = 6 Tryte**, dùng 2 trường mỗi 1 Integer (27-trit) — mirror layout nhị phân nhưng đo đếm bằng trit.

Theo SPEC §1.5.1 canonical sizes: `Tryte = 9 trit = 3²`, `Integer = 27 trit = 3³`, `Long = 81 trit = 3⁴`. Word size tam phân tự nhiên là Integer (27 trit) — alignment chuẩn cho header.

```text
Heap allocation layout (ternary native target, v∞):

  Trit-Addr: HEADER_ADDR         BODY_ADDR = HEADER_ADDR + 54 trit
             |                   |
             v                   v
             [ refcount: Integer | reserved: Integer ] [ user fields ... ]
             |<--- 54 trit (6 Tryte) ----------->|     |<-- sizeof(T) -->|
             
&+ T user-visible pointer points HERE ----^
Refcount manipulation goes back 6 Tryte (54 trit) to find header.
```

**Trường refcount: Integer (27-trit signed balanced ternary)** — đếm số `&+ T` handle đang sống. Phạm vi positive: `0..3^26` ≈ **3.8 nghìn tỷ refs** — nhiều hơn 880× so với u32 nhị phân (`2^32` ≈ 4.3 tỷ).

**Trường reserved: Integer** — slot tương lai, tương tự binary layout.

**Bonus của balanced ternary signed:** vì Integer là **signed** (range `[-3^26, +3^26]`), refcount có thể dùng **negative sentinels**:

| Refcount value | Semantic |
|---|---|
| `> 0` | Object alive với N strong refs |
| `= 0` | Trong process of being freed (destructor running) |
| `= -1` | **Static allocation** — never free (e.g., string literals, compile-time constants) |
| `= -2` | **Frozen forever** — refcount disabled, share unbounded (e.g., `&+ T` immutable shared via capability) |
| `< -2` | Reserved future sentinels |

Negative sentinel space là 1 điểm Triết **đi trước Rust và Swift**: Rust `Arc::new(static_val)` vẫn dùng atomic refcount runtime; Swift static strings có header đặc biệt. Triết native ternary mã hóa "không refcount needed" trực tiếp trong same field — atomic op kiểm tra `current < 0` skip toàn bộ refcount machinery.

#### Comparison: binary vs ternary header

| Architecture | Refcount field | Reserved | Total header | Capacity (positive refcount) |
|---|---|---|---|---|
| Binary 64-bit | `u32` (4 byte) | `u32` (4 byte) | **8 byte = 64 bit** | `2^32` ≈ 4.3 × 10⁹ |
| Ternary native (v∞) | `Integer` (27 trit) | `Integer` (27 trit) | **54 trit = 6 Tryte** | `3^26` ≈ 3.8 × 10¹² |
| Ratio | — | — | Ternary 6 Tryte ≈ 54 trit / 64 bit (in trit equivalent log₂3·64 ≈ 101 trit-bits of info) | Ternary +880× capacity tại same word-alignment |

Khi storage tam phân được pack vào memory nhị phân (intermediate phase — VM hiện tại pack 5 trit/byte per SPEC §1.5.1): 54 trit ≈ ceil(54/5) = **11 byte packed**, tăng 3 byte so với 8-byte binary header. Trade-off chấp nhận được — packing chỉ dùng ở intermediate VM/serialize tier, không phải production ternary hardware.

#### Hệ quả thiết kế cho ABI v∞

ABI metadata (per [ADR-0011](0011-abi-metadata-format.md)) sẽ cần thêm field `target_arch: enum { Binary64, TernaryNative }` để compiler/linker chọn đúng layout. Trytecode emit (planned post-v2.0) sẽ dùng ternary layout này; binary AOT (v2.0 LLVM) dùng layout §3.1.1. Cross-arch shared library: cấm hoặc require explicit re-pack — defer ADR riêng khi v∞ scope rõ.

### 3.2 — D7: Mutable owner cross-actor (linear move)

```triet
let mutable job: &+ mutable Job = build_job()
worker.send(Process(job))                      // job MOVED
print(job.priority)                            // E2420 UseAfterMove
```

Behind the scenes:
1. Sender's `&+ mutable Job` is unique linear owner.
2. `send()` takes `&+ mutable Job` by move (consuming the binding).
3. Receiver's handler param receives the same allocation, gains exclusive ownership.
4. Sender's variable is "moved" — accessing → E2420 từ ADR-0025 §5.1.
5. No refcount machinery.

**User-visible:** Send consumes the variable. Mirror Rust `Send + !Sync` types semantics. Zero runtime cost — same allocation just changes owner.

### 3.3 — Vì sao không có `iso` keyword (D7 rationale)

Pony language dùng `iso` để mark unique-sendable. Triết refuse vì:

1. **`&+ mutable T` đã ngầm linear** (D1 từ ADR-0022). Move semantics đã có.
2. **Thêm keyword `iso`** sẽ duplicate concept — user phải học cả `&+ mutable` lẫn `iso`.
3. **Compile-time check** với 5 reference forms đã đủ enforce — không cần qualifier thứ 7.

Trade-off chấp nhận: user không thể declare "này phải move, không thể frozen-share". Workaround: nếu cần force move semantics, declare type as `&+ mutable T` ngay từ đầu — không thể auto-promote sang frozen (E2411 từ ADR-0025).

### 3.4 — Value types (always Send, copy semantics)

```triet
let count: Integer = 42
worker.send(Tick(count))                        // count copied; sender still has count
print(count)                                    // OK — primitive copy
```

Primitive + value types không tham gia ownership system. Copy luôn khi send. Zero cost (copy 1 register-size word).

---

## §4 — Intra-Actor Aliasing: Rust-strict, không relax

**Lock:** Bên trong 1 actor, NLL exclusivity từ [ADR-0025 §2](0025-borrow-checker-rules.md) vẫn áp dụng đầy đủ. **Không relax.**

### 4.1 — Tại sao không relax intra-actor

Tóm tắt 1 lý thuyết hấp dẫn nhưng bị refuse trong design session 2026-05-26: "vì actor là single-threaded, có thể bỏ NLL exclusivity intra-actor — nhiều `&0 mutable` đồng thời OK."

Refuse vì 3 lý do:

1. **Reasoning simpler** với 1 set rules áp dụng mọi nơi. User không cần học "trong actor luật A, ngoài luật B".
2. **Iterator invalidation** vẫn xảy ra nội bộ actor — `&0 mutable Vector` + `&0 Iterator` đồng thời = corruption dù single-thread.
3. **Future actor pooling** (post-v1.0 có thể có virtual thread style execution) sẽ rất khó nếu intra-actor đã giả định no exclusivity.

Decision này khác Pony (Pony relax intra-actor) — chấp nhận intra-actor verbose hơn để giữ rule consistency.

### 4.2 — Cross-call ownership transfer trong cùng actor

Method calls trong cùng actor follow ADR-0025 borrow rules:

```triet
public actor Worker {
    mutable jobs: Vector<&+ Job>,
    
    receive Process(job: &+ mutable Job) {
        // job is owned by this handler scope
        self.enqueue(job)             // move into enqueue (consuming)
        // job no longer accessible here — E2420 if used
    }
    
    function enqueue(self: &+ mutable Worker, j: &+ mutable Job) {
        push(self.jobs, j)             // move into Vector
    }
}
```

Same as non-actor code — actors don't change ownership semantics for local code, only for cross-actor boundaries.

---

## §5 — Reply channel pattern

**Lock (scaffold only — full ADR defer v0.8 sub-task):** Reply channels là sub-class của message send. Carry `ReplyTo<T>` qualifier; same Send rules apply.

### 5.1 — Pattern minh họa

```triet
receive Query(question: &+ String, reply: ReplyTo<&+ String>) {
    let answer: &+ String = self.lookup(question)
    reply.send(answer)
}

// Caller side
let answer: &+ String = worker.ask(Query("status?"))
//                              ^^^ ask returns a future-like that awaits reply
```

`ReplyTo<T>` về mặt type system là **handle** ngược về sender. Send-ness của reply payload follow rules §2.

### 5.2 — Out of scope cho ADR này

Full reply mechanics — futures, await, structured concurrency, timeout handling — defer ADR riêng (v0.8 sub-task). ADR-0026 chỉ lock: payload Send rules áp dụng cả forward message lẫn reply payload.

---

## §6 — No Escape Hatch (refuse `dev::cross_actor_mut`)

**Lock:** Triết **không có** capability cho phép pass `&+ mutable T` shared (multiple actors mutable cùng object). Nếu user cần shared mutable state cross-actor, **bắt buộc refactor** thành actor sở hữu state + message-based access.

### 6.1 — Lý do refuse

Java/C# có `synchronized` blocks → Java synchronized hell (deadlock, race vẫn xảy ra do reorder, performance disaster). Rust có `Arc<Mutex<T>>` → vẫn complex, runtime panic on lock poisoning. Triết refuse-over-guess (VISION §6).

### 6.2 — Pattern chính tắc thay thế

Thay vì shared mutable, gom mutable state vào 1 actor:

```triet
// Anti-pattern (refused — không có cú pháp expressing this):
// let shared: ArcMutex<Counter> = ...
// worker_a.send(Increment(shared))
// worker_b.send(Increment(shared))

// Pattern chính tắc:
public actor CounterActor {
    mutable count: Integer,
    
    receive Increment {
        self.count = self.count + 1
    }
    
    receive Get(reply: ReplyTo<Integer>) {
        reply.send(self.count)
    }
}

let counter: CounterActor = spawn(CounterActor::new())
worker_a.send(IncrementVia(counter))    // worker_a sẽ forward đến counter
worker_b.send(IncrementVia(counter))
```

Counter là actor → mailbox tự linearize. Không race. Không deadlock (mỗi actor 1 mailbox, no nested lock).

### 6.3 — E2520 SharedMutableAcrossActor (detection sketch)

Khi compiler detect attempt to construct `Vector<&+ mutable T>` được sent qua message → E2520 với suggest refactor sang actor pattern. Chi tiết algorithm defer implementation v0.9+.

```text
E2520 SharedMutableAcrossActor
    Cannot send `Vector<&+ mutable Counter>` across actor boundary.
    Mutable shared state across actors is prohibited (ADR-0026 §6 — no escape hatch).
    
    --> src/example.tri:9:18
       |
    9  |     worker.send(BatchUpdate(counters))
       |                             ^^^^^^^^ Vector of mutable owners cannot be shared
    
    Suggested fixes:
    
    [Fix 1] Wrap the mutable state in its own actor (recommended pattern):
    Refactor Counter into a CounterActor with `Increment` and `Get` messages
    
    [Fix 2] Move ownership entirely to the receiver (no longer shared):
    Change `Vector<&+ mutable Counter>` semantics so each counter has a single owning actor
    
    [Fix 3] Pass frozen snapshots when only reading is needed:
    Change `&+ mutable Counter` to `&+ Counter` (frozen) — refcount-share at boundary
```

---

## §7 — Worked Examples (4 message patterns)

### 7.1 — Send primitive value (always works)

```triet
worker.send(Tick(42))                    // Integer is value type, always Send
worker.send(Heartbeat(now()))            // tuple of primitives, always Send
```

### 7.2 — Send frozen share (D6 path — refcount ngầm)

```triet
let config: &+ Config = load_config()
worker_a.send(Configure(config))         // refcount tăng cho A's handler
worker_b.send(Configure(config))         // refcount tăng tiếp cho B's handler
print(config.version)                    // sender still has handle — OK
// Refcount drops as handlers complete + sender variable expires
```

Compile check: `Config` toàn field Send + `&+ Config` frozen → Send. ✅

### 7.3 — Move ownership (D7 path — linear move)

```triet
let mutable job: &+ mutable Job = build_job()
worker.send(Process(job))                // job MOVED into Process message
// print(job.priority)                   // E2420 UseAfterMove (ADR-0025 §5.1)
```

Compile check: `Job` toàn field Send + `&+ mutable Job` → Send (move). ✅

### 7.4 — Request-reply pattern

```triet
public actor SymbolTable {
    receive Lookup(name: &+ String, reply: ReplyTo<Symbol?>) {
        reply.send(self.find(name))
    }
}

let symbols: SymbolTable = spawn(SymbolTable::new())
let result: Symbol? = symbols.ask(Lookup("main"))
```

Compile check: `&+ String` Send (D6) + `ReplyTo<Symbol?>` Send (reply handle) + `Symbol?` Send (nullable of Send type). ✅

---

## §8 — Implementation Phasing

| Version | Scope |
|---|---|
| **v0.8** | Parser tokens: `actor`, `receive`, `send`, `spawn`, `ReplyTo<T>`. AST nodes. Basic Send derivation §2.1. E2500 fires for obvious `&0`/`&-` violations. Test infrastructure cho 4 patterns §7. |
| **v0.9** | Full Send derivation including generics (monomorphization-time check). E2510 scope-ref leakage. E2520 shared mutable detection. |
| **v1.0** | Actor runtime (mailbox, scheduler) — separate ADR. ReplyTo channels — separate ADR. Supervision tree — separate ADR. |
| **post-v1.0** | Structured concurrency (scope-based actor lifetime), virtual thread pooling, distributed actors. Each separate ADR. |

v0.8 ưu tiên **compile-time rules vào sớm** để self-hosting compiler (v0.7 trở đi) có thể bắt đầu dùng actor primitives. Runtime defer vì semantic correctness mới là critical path, runtime tối ưu hóa được sau.

---

## §9 — Out of Scope

- **Actor syntax cụ thể** (`actor`, `receive`, `spawn` keyword forms) — sub-ADR v0.8.
- **Mailbox implementation** (queue type, bounded vs unbounded, backpressure) — sub-ADR v0.8.
- **Scheduler** (cooperative vs preemptive, fairness, priority) — sub-ADR v0.8.
- **Reply channel mechanics** (futures, await, timeout) — sub-ADR v0.8.
- **Supervision tree** (let-it-crash, restart strategy) — sub-ADR post-v0.8.
- **Distributed actors** (cross-node, network transport) — sub-ADR post-v1.0.
- **Structured concurrency** (scope-based actor lifetime, cancellation) — defer evaluation.
- **Actor-local storage** (thread-local equivalent) — defer.
- **Capability `dev::cross_actor_mut`** — explicitly refused per §6.

---

## §10 — Tham chiếu

- [ADR-0022 — Trit-Balanced Ownership](0022-trit-balanced-ownership.md) (parent — 5 reference forms, D1 linear, frozen distinct from mutable)
- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (sibling — intra-actor enforcement, E2420 use-after-move reused in §3.2)
- [ADR-0027 — Diagnostic Format Standard](0027-diagnostic-format-standard.md) (E2500-E2599 diagnostics follow §2 format)
- [ADR-0016 — Capability type system](0016-capability-type-system.md) (refused `dev::cross_actor_mut` per §6)
- [ADR-0017 — Trilean policy hook](0017-trilean-policy-hook.md) (capability resolver — actor lifecycle may interact post-v1.0)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (actor spawn may require future capability `sys::concurrency` — defer)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` for reply payload, propagate across reply channels)
- [VISION §3.5 — Capability + namespace](../../VISION.md)
- [VISION §6 — Refuse over guess](../../VISION.md) (philosophical alignment with §6 no-escape-hatch)
- [ROADMAP §v0.8 — Concurrency Model](../../ROADMAP.md) (this ADR is foundational for v0.8 phase)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (cập nhật E25XX `triet::actor::*` khi ADR land)
