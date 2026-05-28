# ADR 0022 — Trit-Balanced Ownership (Sở hữu Tam phân Cân bằng)

**Trạng thái:** **Locked** (promoted via v0.8.x.review 2026-05-28; supersedes 2026-05-22 initial sketch). Foundation cho v0.8 Ownership + Concurrency Model — đã ship ObjectHeader (`triet-core::memory`), 5-form lexer tokens, parser AST `ReferenceForm`, type-system resolve transparently per v0.8.3–v0.8.6. Locks ngữ nghĩa 5-form reference syntax + mutability + aliasing + cycle policy + self-ref capability + Outcome integration. Detailed enforcement algorithm tách ra [ADR-0025](0025-borrow-checker-rules.md). Concurrency Send rules tách ra [ADR-0026](0026-actor-boundary-send-rules.md).

**Issue:** Triết tham vọng OS-capable per [VISION §3.5 + §5](../../VISION.md) — phải có memory model đủ chặt như Rust nhưng:
1. **Không có keyword `unsafe`** — mọi nguy hiểm đi qua [capability system (ADR-0018)](0018-capability-loader-semantics.md), audit-friendly.
2. **Không có lifetime annotation `<'a>`** — viral annotations là rào cản nhận thức lớn nhất của Rust.
3. **Bản sắc tam phân** — reference syntax phải map vào trit `{+1, 0, -1}` để nhất quán với [VISION §5 ternary first-class](../../VISION.md).
4. **Zero runtime overhead** — không refcount runtime, không cycle collector, không generational check.

Author 2026-05-25 design session với AI assistant xét 5 kịch bản: (S1) Rust-renamed, (S2) Hylo mutable value semantics, (S3) Vale generational references, (S4) Pony reference capabilities, (S5) hybrid gen-refs + actor isolation. Author 2026-05-26 chốt **S6 — Rust-strict static borrow check + cú pháp tam phân + capability-as-unsafe**, ưu tiên priority: strict, compile-time error catching, performance, AI-friendly. Generational references (S3/S5) bị từ chối vì shift errors to runtime + 1-2% overhead, ngược với priority.

ADR này lock conceptual model. Implementation phase v0.8 chỉ parser tokens; full enforcement defer v0.9-v1.0 per [ADR-0025](0025-borrow-checker-rules.md) §10.

---

## §1 — Context & vấn đề được giải

### 1.1 — Vấn đề Triết cần giải

Triết phải viết được kernel/HĐH, đồng nghĩa với:

| Vấn đề system programming | Rust giải bằng | Triết phải tốt hơn ở đâu |
|---|---|---|
| Doubly-linked list, graph cycles | `Rc<RefCell>` + `Weak` | Verbose tương đương nhưng KHÔNG cần keyword `unsafe` |
| Self-referential struct (parsers, future state) | `Pin` + `unsafe` | Capability `dev::self_ref` thay `unsafe` |
| MMIO, FFI, raw pointer | `unsafe` block | Capability `sys::io.memory` / `dev::ffi` |
| Viral lifetime annotation `<'a, 'b>` | Elision rules covers ~70% | **Bỏ hoàn toàn annotation syntax** |
| Custom collection internals | `unsafe` extensively | Capability `dev::raw_memory` |

### 1.2 — 4 priority của author (chốt 2026-05-26)

1. **Strict, chặt chẽ** — refuse-over-guess per VISION §6.
2. **Bắt lỗi nhiều nhất ở compile-time** — runtime checks chỉ ở những chỗ thực sự không tránh được (ví dụ: array bounds).
3. **Performance** — zero-cost abstraction, không refcount runtime ở core language.
4. **AI-friendly** — ít concept, syntax explicit, error messages có fix suggestion. Compile-time errors > runtime errors cho AI debugging.

Trade-off chấp nhận: doubly-linked list / cycle phải break bằng `&-`, không có "tự nhiên" như Vale gen-refs. Self-ref struct phải đi qua capability gate. Đổi lại zero runtime overhead + 100% compile-time check.

### 1.3 — Decisions D1–D3 chốt trong ADR này

| ID | Decision | Lý do |
|---|---|---|
| **D1** | `&+` là **unique/exclusive owner** (không clone tự do trong core language) | Zero runtime overhead, compile-time exclusivity check khả thi |
| **D2** | Default **read-only mọi nơi** (variable, parameter, struct field). Explicit keyword `mutable` để cho phép mutate | Brand fit "stability over speed", giống Rust 2018+ default |
| **D3** | Self-ref struct **default cấm**, mở khóa qua capability `dev::self_ref` (offset-based pattern) | Refuse-over-guess; tránh Pin/unsafe complexity |

D4–D7 chốt ở ADR-0025 và ADR-0026 (borrow checker + thread boundary send).

---

## §2 — Năm dạng reference (lock cú pháp)

**Lock:** Triết có đúng 5 dạng reference, đọc từ mạnh nhất tới yếu nhất:

| Cú pháp | Tên | Quyền sở hữu | Quyền ghi | Aliasing | Tương đương Rust |
|---|---|---|---|---|---|
| `&+ T` | Strong owner, frozen | Unique owner | Read-only | Không clone | `Box<T>` (frozen) |
| `&+ mutable T` | Strong owner, mutable | Unique owner | Mutable | Không clone | `Box<T>` |
| `&0 T` | Scope borrow, read-only | Borrow | Read-only | Nhiều handles OK | `&T` |
| `&0 mutable T` | Scope borrow, mutable exclusive | Borrow | Mutable | **Exclusive** (1 tại 1 thời điểm) | `&mut T` |
| `&- T` | Weak observer | Không sở hữu | Read-only sau upgrade | Nhiều handles OK | `Weak<T>` (compile-time) |

### 2.1 — Tại sao có cả `&+ T` (frozen) và `&+ mutable T`

Java analogy: `final User u = new User(...)` (frozen owner) vs `User u = new User(...)` (mutable owner). Cả 2 đều là owner duy nhất, khác nhau ở quyền mutate.

`&+ T` (frozen) tồn tại để **send qua thread boundary an toàn** (xem [ADR-0026](0026-actor-boundary-send-rules.md) §3) — frozen ≡ immutable share-able. `&+ mutable T` thì không Send (mutable shared = race condition).

### 2.2 — Tại sao không có syntax cho "shared owner" (Arc/Rc equivalent)

Per **D1**, core language không có shared ownership. Lý do:

- **Performance:** Rc/Arc đều có refcount overhead. Arc atomic ops đặc biệt đắt.
- **Compile-time clarity:** Unique owner cho phép compile-time exclusivity check không phải runtime guard.
- **Brand fit:** Triết chấp nhận verbose hơn Rust ở một số pattern để đổi lấy zero-cost + compile-time rigor.

Khi thực sự cần share đối tượng immutable cross-thread, [ADR-0026](0026-actor-boundary-send-rules.md) sẽ cho phép refcount **tự động ngầm** ở thread boundary — nhưng không expose vào user-facing language.

### 2.3 — Tại sao không có `&+ mutable shared T` (Rc<RefCell> equivalent)

Mutable share là nguồn gốc của data race + iterator invalidation. Rust giải bằng `RefCell` (runtime borrow check, panic on violation). Triết refuse vì:

- Runtime panic vi phạm priority "compile-time error catching".
- Pattern này 95% thay được bằng message-passing pattern (gom mutable state vào 1 thread/context, query/update qua message).

Edge case 5% còn lại (single-threaded interior mutability): có thể dùng `Cell<T>` cho primitive copy types (planned post-v1.0), hoặc refactor sang message-passing pattern.

---

## §3 — Mutability rule (D2: read-only default)

**Lock:** Read-only là mặc định ở 3 vị trí: variable binding, function parameter, struct field. Keyword `mutable` để cho phép mutate.

### 3.1 — Variable binding

```triet
let x = 10              // immutable binding (rebind cấm)
let mutable y = 20      // mutable binding (y = 30 OK; nhưng rebind type không OK)
```

Đây không liên quan đến reference type — chỉ về `let` binding. `mutable` trên binding ≠ `mutable` trên reference.

### 3.2 — Function parameter

```triet
function greet(name: String)            // name: &0 String (read-only borrow, default)
function append(buf: &0 mutable Bytes)  // exclusive mutable borrow
function consume(owned: &+ String)      // take ownership (move semantics)
```

Default infer `&0` cho parameter (xem §5). Explicit `&+` để take ownership. Explicit `&0 mutable` để mutate qua borrow.

### 3.3 — Struct field

```triet
public struct Process {
    pid: Integer,                          // immutable field (set 1 lần lúc construct)
    mutable state: ProcessState,           // mutable field (sửa được sau construct)
    children: Vector<&+ Process>,          // immutable field, owned children
    mutable parent: &- Process             // mutable field, weak ref to parent
}
```

`mutable` ở struct field cho phép sửa field đó sau khi struct được construct. Không cho phép sửa = field set 1 lần ở constructor, immutable từ đó về sau.

### 3.4 — "Frozen forever" không thể promote thành mutable

Quy tắc nghiêm ngặt: **`&+ T` không bao giờ promote được thành `&+ mutable T`.** Frozen là frozen vĩnh viễn.

```triet
let owner: &+ User = create_user("Alice")
owner.name = "Bob"                      // E2410 CannotMutateFrozenOwner
let mutable_owner: &+ mutable User = owner  // E2411 CannotPromoteFrozenToMutable
```

Lý do: nếu cho phép promote, frozen mất ý nghĩa — author có thể bypass bằng cách promote tạm thời. Hệ quả: tác giả phải quyết định lúc construct, frozen hay mutable.

Error codes E2410–E2419 reserved cho mutability violations (chi tiết trong ADR-0025 §4).

---

## §4 — Aliasing rule (D1: linear/unique `&+`)

**Lock:** Mỗi heap allocation có **đúng 1 `&+`** tại bất kỳ thời điểm nào. Muốn share = dùng `&0` borrow.

### 4.1 — Move semantics khi pass `&+`

```triet
function take(owned: &+ User) { /* ... */ }

let alice: &+ User = create_user("Alice")
take(alice)            // ownership move vào take(); alice no longer usable
print(alice.name)      // E2420 UseAfterMove
```

Khác Rust: Rust dùng `Box<T>` cũng move, nhưng `&T` thì borrow. Triết explicit hơn ở cú pháp — đọc `&+` là biết move sẽ xảy ra.

### 4.2 — Borrow không move

```triet
function read(borrowed: &0 User) { print(borrowed.name) }

let alice: &+ User = create_user("Alice")
read(alice)                  // implicit borrow: alice → &0 alice
print(alice.name)            // OK, alice vẫn own
```

Compiler tự động borrow `&+` thành `&0` khi truyền vào function expects `&0`. Không cần explicit `&` operator như Rust.

### 4.3 — Exclusive mutable borrow

```triet
function mutate(borrowed: &0 mutable User) { borrowed.name = "Bob" }

let mutable alice: &+ mutable User = create_user("Alice")
mutate(alice)                // exclusive mutable borrow tạm thời
print(alice.name)            // OK, alice còn own, đã thấy "Bob"
```

Tại 1 thời điểm: 1 `&0 mutable` XOR N `&0`. Compiler enforce qua NLL (Non-Lexical Lifetime) per [ADR-0025 §2](0025-borrow-checker-rules.md). Cố tình trộn → E2400 series.

### 4.4 — Tại sao linear/unique thay vì refcount

Refcount (Rc/Arc) cho phép nhiều `&+` cùng tồn tại, mỗi clone tăng count. Triết từ chối vì:

1. **Runtime cost:** atomic op cho Arc, non-atomic op cho Rc — vi phạm priority "performance".
2. **Cycle problem:** refcount không thu hồi cycle → phải có cycle collector (runtime overhead + non-deterministic drop) hoặc force user dùng Weak break (như Rust).
3. **Compile-time predictability:** unique ownership cho phép compile-time exclusivity check; refcount thì không.

Cross-thread immutable share sẽ được handle ở [ADR-0026](0026-actor-boundary-send-rules.md) bằng refcount **ngầm**, không expose cú pháp ra user. Memory layout chi tiết (8-byte object header chứa refcount field) lock ở [ADR-0026 §7](0026-actor-boundary-send-rules.md) — Scenario A "header always present" thay vì lazy box-wrapping, vì lazy wrap phá invariant compile-time của `&-` weak refs.

---

## §5 — Default inference theo namespace

**Lock:** Trong `usr::` namespace, compiler infer dạng reference khi tác giả không gõ explicit. Trong `sys::` / `dev::`, không có infer — phải explicit.

### 5.1 — Quy tắc infer trong `usr::`

| Vị trí | Default ngầm | Explicit override |
|---|---|---|
| Struct field type `field: T` | `&+ T` (owned, immutable) | `&+ mutable T`, `&- T`, `T` (value type only) |
| Function param `param: T` | `&0 T` (borrow, read-only) | `&0 mutable T`, `&+ T`, `T` (value type only) |
| Function return `-> T` | Inferred từ body (owned hoặc borrow tie tới input) | Explicit `-> &0 T` |
| `let x = expr` | Inferred từ expr | Explicit `let x: &+ T = expr` |

Ví dụ:

```triet
// usr namespace — implicit refs
module usr.account

public struct Account {
    id: AccountId,             // value type, no ref
    owner: User,               // ngầm &+ User (struct field default)
    balance: Money             // value type, no ref
}

public function transfer(from: Account, to: Account, amount: Money) {
    // from, to ngầm &0 Account (param default)
    // ...
}
```

### 5.2 — Quy tắc cấm infer trong `sys::` / `dev::`

```triet
module sys.kernel.scheduler

public struct Process {
    pid: Integer,              // explicit OK (value type)
    state: &+ mutable ProcessState,    // BẮT BUỘC explicit
    parent: &- Process                 // BẮT BUỘC explicit
}

public function schedule(proc: ProcessHandle) {  // E2208 LayoutNotExplicit
    // sys namespace ép explicit
}

public function schedule(proc: &+ ProcessHandle) {  // OK
    // ...
}
```

Error code E2430 `ImplicitRefInSystemNamespace` cho violation này (reserve, define trong ADR-0025).

### 5.3 — Tại sao phân tầng

VISION §3.5 chốt 3 namespace có tầng quyền lực khác nhau. `usr::` ưu tiên developer ergonomics, `sys::` / `dev::` ép kỷ luật. Pattern này nhất quán với [ADR-0018 §1](0018-capability-loader-semantics.md) — capability cũng phải explicit declare trong `sys::` / `dev::`.

---

## §6 — Định lý vô-chu-trình của Linear Ownership

**Lock:** Triết **không cần** thuật toán dò chu trình, không cần cycle collector, không cần GC. Luật unique ownership (D1) **làm cho việc tạo ra chu trình `&+` lúc runtime là bất khả thi về mặt toán học**. Compiler không tốn 1 dòng code nào để check cycle.

### 6.1 — Định lý

> **Định lý vô-chu-trình:** Trong một chương trình Triết hợp lệ, không tồn tại đường đi đóng (closed path) nào trên đồ thị object lúc runtime mà toàn bộ cạnh là `&+`.

Đây là hệ quả trực tiếp của D1 (`&+` unique/linear) + move semantics (§4.1). Không cần compiler dò — bản thân luật cú pháp đã chặn ở compile time qua use-after-move detection.

### 6.2 — Chứng minh phác (move semantics chặn ngay lúc gán)

Giả sử tác giả muốn tạo chu trình 2-node `A ⇄ B` với cả 2 cạnh là `&+`:

```triet
let a: &+ A = create_a()       // bước 1: a sở hữu A. Owner chain: caller → a
let b: &+ B = create_b()       // bước 2: b sở hữu B. Owner chain: caller → b
a.b_field = b                  // bước 3: move b VÀO a.b_field
                               //         Owner chain giờ: caller → a → b
                               //         Sau bước này, biến `b` đã consumed.
b.a_field = a                  // bước 4: ERROR — `b` đã được move ở bước 3.
                               //         E2420 UseAfterMove.
```

Vẫn cố? Cố dùng nested path:

```triet
a.b_field.a_field = a          // bước 4': cố move `a` vào field-của-field-của-a
                               //          Vế phải là `a`. Vế trái thuộc về cây sở hữu của a.
                               //          Để move `a`, `a` phải đang là free owner —
                               //          nhưng chính vế trái đang đọc qua `a` để định địa chỉ.
                               //          E2421 SelfOwnershipParadox (defined trong ADR-0025).
```

Mở rộng cho chu trình n-node: cùng nguyên lý. Mỗi cạnh `&+` là 1 lần move, mỗi move "consume" biến nguồn. Để khép vòng phải move lại biến đã consumed → impossible.

**Kết luận:** Linear ownership không phải "thuật toán đẹp để chống cycle". Nó là **bất biến cấu trúc** (structural invariant) — cycle với toàn `&+` đơn giản là không phát biểu được trong ngôn ngữ.

### 6.3 — Hệ quả thực tiễn

Mọi cấu trúc dữ liệu **có bản chất 2 chiều** (doubly-linked list, tree với parent pointer, graph với back-edge) **bắt buộc** dùng `&-` trên cạnh ngược. Đây không phải gợi ý lint — là luật vật lý của ngôn ngữ.

```triet
public struct DListNode {
    value: Integer,
    next: (&+ DListNode)?,      // forward owns next (nullable cho tail terminator)
    prev: &- DListNode           // backward weak — luật vật lý
}

public struct TreeNode {
    children: Vector<&+ TreeNode>,
    parent: &- TreeNode          // luật vật lý
}
```

Không có cách nào "tránh" `&-` cho back-edge. Author thử dùng `&+` ở cả 2 chiều sẽ gặp E2420 hoặc E2421 — ngôn ngữ refuse-over-guess.

### 6.4 — Compiler duy nhất check 1 thứ: type có constructible không

Vì `&+ T` là indirection (pointer-sized), **type size luôn finite** kể cả với self-reference `struct Node { next: &+ Node }`. Không có infinite-size issue.

Cái compiler check là **constructibility termination**: nếu type T có field `&+ T` (trực tiếp hoặc qua chain), constructor phải có **base case** để dừng:

| Pattern | Base case | Constructible? |
|---|---|---|
| `struct Node { next: &+ Node }` | Không | ❌ E2422 NonTerminatingConstruction — buộc wrap nullable |
| `struct Node { next: (&+ Node)? }` | `~0` | ✅ chain kết thúc bằng `~0` |
| `struct Node { children: Vector<&+ Node> }` | `empty()` | ✅ Vector có thể rỗng |
| `struct Node { parent: &- Node }` | Có (weak null) | ✅ weak có null state tự nhiên |

Đây là **constructibility check** chứ không phải cycle check — local, đơn giản, không cần đồ thị toàn cục.

### 6.5 — Không có runtime collector, không có GC

Vì định lý §6.1 cấm cycle lúc runtime, mọi `&+` sẽ được giải phóng deterministic khi scope kết thúc. Không cần:

- ❌ Mark-sweep collector (như Java/Go)
- ❌ Reference counting với cycle detector (như Python)
- ❌ Generational GC (như V8, .NET)

Đây là điểm Triết **đi trước Swift** (ARC + có cycle leak vì lập trình viên không nhớ `weak`) và **đi cùng Rust** (zero-cost) — nhưng không cần lifetime annotation.

---

## §7 — Self-referential struct (D3: capability-gated)

**Lock:** Default cấm `&0 T` field trong struct (vì borrow scope không thể outlive struct). Mở khóa qua capability `dev::self_ref` — chỉ cho phép offset-based pattern, không phải pointer thực sự.

### 7.1 — Tại sao default cấm

```triet
public struct Foo {
    data: Vector<Tryte>,
    cursor: &0 Tryte     // E2402 BorrowInStructField
}
```

`&0` là scope-bound — không thể tồn tại quá scope. Lưu vào struct → struct outlive scope → dangling. Rust phải dùng `Pin` + `unsafe` (hoặc đặc biệt là ouroboros crate).

### 7.2 — Capability `dev::self_ref` mở khóa offset-based pattern

```triet
// dao.package
capabilities {
    dev::self_ref: grant
}

// trong module sys::network
public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer,     // OK — chỉ lưu vị trí, không phải pointer
    payload_offset: Integer
}

public function get_header(packet: &0 NetworkPacket) -> &0 Header {
    return slice_at(packet.buffer, packet.header_offset)
}
```

Pattern này không thực sự lưu `&0` vào struct — chỉ lưu `Integer` offset. Capability `dev::self_ref` chỉ chứng nhận "tôi biết mình đang dùng pattern này có chủ ý". Không thực sự mở khóa unsafe gì.

### 7.3 — Không có Pin equivalent

Rust `Pin<&mut T>` cho phép thực sự lưu reference vào self. Triết không cho phép pattern này — buộc tác giả dùng offset hoặc index. Trade-off: một số Future state machine pattern phức tạp hơn so với Rust async, nhưng v0.8 BYOS primitives thay thế phần lớn use case.

---

## §8 — Capability thay `unsafe` (philosophy)

**Lock:** Triết **không có keyword `unsafe`**. Mọi hành vi Rust cần `unsafe` được reframe thành capability declaration trong `dao.package` per [ADR-0018](0018-capability-loader-semantics.md).

### 8.1 — Bảng capability liên quan ownership

| Operation | Capability | Tác động |
|---|---|---|
| Self-ref struct (offset-based) | `dev::self_ref` | Cho phép pattern §7.2 |
| Custom collection (raw allocation) | `dev::raw_memory` | Bypass `&+` tracking, manual lifetime |
| Transmute / bit reinterpret | `dev::reinterpret` | Cast bytes giữa type khác layout |
| FFI sang C/extern | `dev::ffi` | Pass raw pointer to extern function |
| MMIO / physical address | `sys::io.memory` | Read/write địa chỉ vật lý |
| Custom destructor logic | `dev::custom_drop` | User-defined `drop fn`, ràng buộc order |

Tất cả capability phải declare trong `dao.package` của root project. Auditor đọc 1 file = biết toàn bộ "unsafe surface" của codebase.

### 8.2 — Tại sao capability tốt hơn `unsafe` block

| Khía cạnh | Rust `unsafe` | Triết capability |
|---|---|---|
| Audit surface | Grep `unsafe {` rải rác mọi crate | 1 file `dao.package` |
| Granularity | Tất cả unsafe ops thông qua 1 keyword | Mỗi loại op có capability riêng |
| Per-package opt-in | Cargo features khắc phục 1 phần | Capability là first-class declaration |
| Runtime audit | Compile-time only | Compile + link + runtime resolver per [ADR-0017](0017-trilean-policy-hook.md) |
| Brand fit | "Memory safety with escape hatch" | "Memory safety with explicit capability" |

`unsafe` của Rust là binary (có/không). Capability của Triết là 4-state per [ADR-0016 CapabilityLevel](0016-capability-type-system.md): `Grant`/`Ambient`/`Deny`/`Defer`. Tinh tế hơn cho deployment.

---

## §9 — Integration với Outcome (ADR-0020) — `&-` deref

**Lock:** Deref `&- T` trả `T?` (nullable), không phải `T~UseAfterFree` (Outcome). Compile-time tracked, không runtime gen check.

### 9.1 — Cú pháp upgrade

```triet
function notify_parent(child: &0 Process) {
    let parent: &- Process = child.parent
    match parent.upgrade() {
        ~+ p => p.on_child_event(child),
        ~0 => log("parent never set")
    }
}
```

Method `.upgrade()` trả `T?` (nullable, 3-state per ADR-0001 + ADR-0020). Force match — không có silent deref.

### 9.2 — Tại sao `T?` không phải `T~UseAfterFree` (Outcome)

Decision này khác S5 (sketch ban đầu của AI assistant 2026-05-25). Lý do chốt 2026-05-26:

- **Compile-time safety priority:** nếu deref trả Outcome `T~UseAfterFree`, có nghĩa là use-after-free là runtime concept. Triết S6 chốt: **`&-` không bao giờ "use after free"** vì compile-time check đảm bảo upgrade match trước khi deref. `T?` chỉ phản ánh "parent có thể chưa được set", không phải "parent đã chết".
- **Zero runtime overhead:** `T~UseAfterFree` đòi hỏi gen check runtime. Bỏ.
- **Nhất quán với ADR-0001:** `T?` đã có wire format + nullable semantic. Reuse thay vì thêm khái niệm mới.

### 9.3 — `&- T` không bao giờ là dangling pointer

Compile-time invariant: `&- T` chỉ tồn tại khi có ít nhất 1 `&+ T` còn sống ở scope cha. Borrow checker enforce. Khi `&+` drop, mọi `&-` chỉ tới object đó đều ngoài scope (compile-time guaranteed) → không có dangling reference observable từ user code.

Edge case: nếu tác giả lưu `&-` vào struct outlive `&+`, compile-time error E2403 `WeakRefOutlivesOwner`. Algorithm chi tiết ADR-0025.

### 9.4 — Khác Rust `Weak<T>`

Rust `Weak<T>` carries gen+ptr runtime; `.upgrade()` trả `Option<Rc<T>>` qua runtime check. Triết `&-` là compile-time concept thuần — không có gen, không có runtime check, không upgrade runtime.

Trade-off: Triết `&-` hạn chế hơn Rust `Weak` — không thể giữ `&-` qua thread boundary tự do (xem ADR-0026 §3). Đổi lại zero runtime overhead.

---

## §10 — Prior art & lý do chọn S6

| Ngôn ngữ | Approach | Strength | Weakness so với Triết priority |
|---|---|---|---|
| Rust | Static borrow + lifetime annotation | Zero-cost, compile-time | Viral `<'a>` annotations |
| Mojo | Borrow conventions (`borrowed`/`inout`/`owned`) | Đơn giản | Không giải doubly-linked, no thread/BYOS story |
| Pony | 6 reference capabilities (iso/trn/ref/val/box/tag) | Concurrency-safe | Curve học cực dốc |
| Hylo (Val) | Mutable value semantics, no references | Không lifetime | Phải restructure mọi data-oriented code |
| Vale | Generational references (runtime check) | Solve cycles tự nhiên | 1-2% overhead, runtime errors |
| Swift | ARC | Đơn giản | Implicit refcount overhead |

**Triết S6 = Rust static check core + cú pháp tam phân từ ADR-0022 original + capability từ ADR-0018 + Outcome integration từ ADR-0020.**

Điểm Triết-unique không có ở ngôn ngữ nào khác:
1. **Ternary syntax `&+/&0/&-`** map vào trit identity.
2. **Capability-as-unsafe** — không có `unsafe` keyword nào.
3. **No lifetime annotation syntax** — compile error với fix suggest khi elision fail (xem ADR-0025 §4).
4. **Frozen owner `&+ T`** distinct với mutable owner `&+ mutable T` — cho phép send cross-thread tự nhiên.

---

## §11 — Out of scope (defer cho ADRs khác)

ADR-0022 lock **conceptual model**. Các phần sau defer:

| Topic | ADR |
|---|---|
| Borrow checker algorithm (NLL, elision, use-after-move, constructibility termination) | [ADR-0025](0025-borrow-checker-rules.md) (TODO) |
| Drop order, custom destructor, capability `dev::custom_drop` | ADR-0025 §6 |
| Move semantics (use-after-move detection) | ADR-0025 §8 |
| Send rule, cross-thread refcount | [ADR-0026](0026-actor-boundary-send-rules.md) (TODO) |
| FFI memory model (raw pointer, alignment, ownership across C boundary) | Future ADR (post-v0.8) |
| Generic + reference interaction (`Vector<&+ T>` vs `Vector<&0 T>`) | ADR-0025 §9 |
| Closure capture semantics (`Fn` vs `FnMut` equivalent) | Future ADR khi closure lock |
| Trait object reference (`dyn Trait` equivalent) | Future ADR khi dynamic dispatch lock |

---

## §12 — Examples (4 patterns)

### 12.1 — Process tree (back-edge bắt buộc `&-` per §6.3)

```triet
public struct Process {
    pid: Integer,
    mutable state: ProcessState,
    children: Vector<&+ Process>,    // forward owns (Vector terminator natural)
    mutable parent: &- Process,      // back-edge — luật vật lý
    mutable next_in_queue: &- Process
}

public function add_child(parent: &0 mutable Process, child: &+ Process) {
    push(parent.children, child)
}

public function notify_parent(p: &0 Process) {
    match p.parent.upgrade() {
        ~+ parent => parent.on_child_event(p),
        ~0 => log("root or unset parent")
    }
}
```

### 12.2 — Doubly-linked list (cú pháp khởi tạo `&-` parallel với type position)

```triet
public struct DListNode {
    value: Integer,
    mutable next: (&+ DListNode)?,    // (&+ T)? — nullable tail terminator (§6.4)
    mutable prev: &- DListNode         // weak back-edge (§6.3)
}

// Cú pháp khởi tạo &- y hệt cú pháp type: dùng toán tử &-.
// Map 1:1 với declaration `prev: &- DListNode`.
public function example_link(current_tail: &0 DListNode, value: Integer) -> &+ DListNode {
    return DListNode {
        value: value,
        next: ~0,                      // chưa có successor — null terminator
        prev: &- current_tail           // toán tử &- tạo weak ref, nhất quán brand
    }
}
```

Lưu ý: `&- expr` ở vị trí giá trị (expression) là **constructor operator** cho weak reference, đối xứng với `&- T` ở vị trí kiểu. Không có function call `weak()` — bản thân ký hiệu `&-` đã là toán tử cấp ngôn ngữ, nhất quán với việc `&0 expr` (nếu cần explicit borrow) cũng dùng cùng họ ký hiệu `&`.

### 12.3 — Network packet (self-ref via capability)

```triet
// dao.package
capabilities {
    dev::self_ref: grant  // chứng nhận có chủ ý dùng offset pattern
}

// module sys::network
public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer,
    payload_offset: Integer
}

public function get_header(packet: &0 NetworkPacket) -> &0 Header {
    return slice_at(packet.buffer, packet.header_offset)
}
```

### 12.4 — MMIO blink LED (capability gates kernel access)

```triet
// dao.package
capabilities {
    sys::io.memory: grant
}

// module sys::driver::led
public function blink(reg: &0 mutable HardwareRegister) {
    // sys::io.memory capability gate runtime check
    sys.io.memory.write(reg.address, 0xFF)
}
```

Lưu ý: §12.4 thực ra là demo ADR-0018 chứ không phải ADR-0022 — ownership ở đây chỉ là `&0 mutable HardwareRegister` thông thường. Capability mới là điểm quan trọng. Test 3 của ADR-0022 cũ bị xóa khỏi chính tài liệu này vì conflation, nhưng giữ ví dụ ở §12.4 để chỉ rõ ownership + capability **kết hợp ra sao**.

---

## §13 — Tham chiếu

- [SPEC §10 — Memory model](../../SPEC.md) (sẽ rewrite đồng bộ với ADR-0025 + ADR-0026 land)
- [VISION §3.5 — Capability + namespace](../../VISION.md)
- [VISION §5 — Bản sắc Triết: ternary first-class](../../VISION.md)
- [VISION §6 — Refuse over guess](../../VISION.md)
- [ROADMAP §v0.8 — Concurrency Model](../../ROADMAP.md)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) (`T?` discriminator reuse cho `&- T` upgrade trong §9)
- [ADR-0016 — Capability type system](0016-capability-type-system.md) (4-state level dùng cho memory-related caps)
- [ADR-0017 — Trilean policy hook](0017-trilean-policy-hook.md) (capability resolver path cho `dev::self_ref` etc.)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (`dao.package` declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` đã chốt cú pháp dùng cho `&-` upgrade)
- [ADR-0021 — Trilean refinement](0021-trilean-refinement.md) (refuse-with-fix-suggest pattern reuse cho E2400 series)
- ADR-0025 — Borrow Checker Rules (TODO, sibling) — enforcement algorithm
- ADR-0026 — Concurrency Primitives & Send Rules (sibling) — concurrency interplay
