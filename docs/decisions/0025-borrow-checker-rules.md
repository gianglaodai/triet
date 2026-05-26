# ADR 0025 — Borrow Checker Rules (Luật Kiểm tra Mượn-Sở-Hữu)

**Trạng thái:** **Draft** (sibling của [ADR-0022](0022-trit-balanced-ownership.md) + [ADR-0026](0026-actor-boundary-send-rules.md) (TODO)). Targets v0.8 parser tokens → v0.9 simple enforcement → v0.10-v1.0 full algorithm. Locks compile-time enforcement algorithm cho 5 reference forms từ ADR-0022 §2. Định nghĩa namespace error code mới **E24XX** cho borrow-related diagnostics.

**Issue:** [ADR-0022](0022-trit-balanced-ownership.md) lock conceptual model — 5 reference forms, mutability default, linear `&+`, capability-as-unsafe, định lý vô-chu-trình. Còn lại các quyết định *thuật toán*:

- **D4** — Borrow exclusivity dùng NLL (Non-Lexical Lifetime) hay basic lexical?
- **D5** — Khi lifetime inference fail, cho phép explicit annotation `<'a>` không?
- Use-after-move detect thế nào (E2420, E2421).
- Constructibility termination check ra sao (E2422 từ ADR-0022 §6.4).
- Mutability violation enforcement (E2410, E2411 từ ADR-0022 §3.4).
- Drop order + custom destructor.
- Default inference algorithm trong `usr::` vs `sys::/dev::`.

Author 2026-05-26 chốt D4 = NLL (smart enough to accept code rejected by Rust 2015), D5 = **không có syntax `<'a>`** (compile error với refactor suggest, worst case dùng capability post-v1.0).

ADR này lock algorithm cho từng error category + implementation phasing. Mỗi sub-decision có error code dedicated trong namespace E24XX (E2400-E2499 reserved for borrow checker).

---

## §1 — Goals & non-goals

### 1.1 — Goals

1. **Compile-time only** — borrow checker không emit runtime check nào.
2. **Zero-cost** — không thay đổi memory layout, không insert guard code.
3. **90% common patterns work** không cần explicit annotation.
4. **10% edge cases** → compile error với 2-3 concrete refactor suggestions.
5. **Error messages** dùng pattern E2400 với "Suggested fix" block (style đã chốt qua [ADR-0021 §10](0021-trilean-refinement.md)).

### 1.2 — Non-goals (defer post-v1.0)

- **Polonius-level permissive analysis** — Rust's next-gen borrow checker. Triết v1.0 stop ở NLL.
- **Two-phase borrows** — Rust feature cho phép certain mutable+immutable trùng. Defer cho đến khi có evidence cần.
- **Generic lifetime variance** — vì không có annotation syntax, không có variance.
- **Self-referential async future** — defer cho đến khi async/await design lock (post-v0.8).

### 1.3 — Error code namespace E24XX

Reserved range **E2400–E2499** cho borrow checker. Phân bổ:

| Range | Category |
|---|---|
| E2400–E2409 | Lifetime inference & elision |
| E2410–E2419 | Mutability violations |
| E2420–E2429 | Move semantics & use-after-move |
| E2430–E2439 | Namespace inference violations |
| E2440–E2449 | Borrow exclusivity (NLL) |
| E2450–E2459 | Reserved (drop order, custom drop) |
| E2460–E2499 | Reserved future expansion |

Module path: `triet::borrow::E24XX`. CLAUDE.md cập nhật khi ADR land.

### 1.4 — Error message format

Tất cả diagnostic trong E24XX namespace (§2-§10) follow canonical format đã chốt ở [ADR-0027 — Diagnostic Format Standard](0027-diagnostic-format-standard.md). Format này áp dụng language-wide, không riêng E24XX.

Tóm tắt: header `EXXXX ErrorName` + body 1-3 câu + optional span block (`--> file:line:col` + caret) + optional `[Fix N]` numbered fix blocks với imperative `Change/Wrap/Use/Add/Replace/Move X to Y`. Pure ASCII, không diff `-/+`. Chi tiết spec + rationale: ADR-0027 §2.

---

## §2 — Borrow Exclusivity với NLL (D4)

**Lock:** Tại bất kỳ program point nào, với cùng 1 đối tượng (place), borrow phải thỏa **1 trong 2 trạng thái**:

- **Trạng thái A:** Có đúng 1 `&0 mutable T` active. Không có `&0 T` nào active đồng thời.
- **Trạng thái B:** Có N ≥ 0 `&0 T` active. Không có `&0 mutable T` nào.

Cấm: 2 `&0 mutable` cùng đối tượng; hoặc `&0 mutable` + `&0` cùng đối tượng.

### 2.1 — "Active" theo NLL không phải lexical

NLL: borrow chỉ "active" từ điểm tạo đến **lần cuối được dùng**, không phải đến cuối block. Cho phép nhiều pattern Rust 2015 từ chối:

```triet
let mutable v = Vector { 1, 2, 3 }
let r1: &0 Vector = &0 v        // borrow start
print(r1.length)                  // last use of r1
v.push(4)                         // OK với NLL — r1 đã hết active từ dòng trên
```

Rust 2015 lexical sẽ reject vì `r1` "still in scope". NLL accept vì last-use đã qua.

### 2.2 — E2440 BorrowExclusivityViolation

```text
E2440 BorrowExclusivityViolation
    Cannot create `&0 mutable` while `&0` is active on the same place `v`.
    Borrow `r1` is still live at the mutation point because of a later read.
    
    --> src/example.tri:11:5
       |
    10 |     let r1: &0 Vector = &0 v
       |                         ----- &0 borrow created here
    11 |     v.push(4)
       |     ^^^^^^^^^ &0 mutable creation conflicts
    12 |     print(r1.length)
       |           -------- &0 still used here (extends live range above)
    
    Suggested fixes:
    
    [Fix 1] Reorder the read before the mutation (shrinks live range of r1):
    Move `print(r1.length)` to immediately before `v.push(4)`
    
    [Fix 2] Copy the borrowed value out before mutating:
    Replace line 10 with `let len = v.length` and remove the `r1` binding
    
    [Fix 3] Restructure to avoid simultaneous read+write:
    Wrap the mutation behind a method on the owner struct that controls borrow scope internally
```

### 2.3 — Algorithm: live-range analysis trên CFG

Compiler build control-flow graph (CFG), gán cho mỗi borrow 1 "live range" từ creation point đến last-use point. Hai borrow conflict khi live range giao nhau **và** vi phạm exclusivity. Algorithm có complexity O(N·M) với N = borrow count, M = CFG size — practical cho codebase realistic.

Implementation phase v0.10 (chi tiết §12).

---

## §3 — Lifetime Elision (3 quy tắc)

**Lock:** Compiler suy ra borrow scope tự động qua 3 quy tắc tuần tự. Nếu cả 3 không apply → E2400.

### 3.1 — Quy tắc 1: 1 input borrow → output ties to input

```triet
function first_word(s: &0 String) -> &0 String {
    // Compiler tự suy: return scope = `s` scope
}
```

Function có **đúng 1** borrow input (`&0`, `&0 mutable`, hoặc `&-`) và trả về borrow. Output borrow scope = input borrow scope.

### 3.2 — Quy tắc 2: Method với self → output ties to self

```triet
public struct Cache {
    public function get(self: &0 Cache, key: String) -> &0 Entry {
        // Compiler tự suy: return scope = self scope
    }
}
```

Method với receiver `&0 self` hoặc `&0 mutable self` (bất kể có thêm borrow inputs khác hay không). Output borrow scope = self scope.

### 3.3 — Quy tắc 3: Owned return (no inference needed)

```triet
function parse(s: &0 String) -> &+ ParsedDoc {
    // Return owned — không có lifetime relationship cần suy ra
}
```

Output là `&+ T` (owned). Không có lifetime relationship — function transfer ownership ra. Không cần elision.

### 3.4 — Khi cả 3 quy tắc fail → E2400

```triet
function pick_longer(a: &0 String, b: &0 String) -> &0 String {
    if a.length > b.length { return a } else { return b }
}
```

Function có 2 borrow inputs, không phải method, trả về borrow. Compiler không biết output tie tới `a` hay `b`.

```text
E2400 BorrowLifetimeInferenceFailed
    Cannot infer which input the returned borrow ties to.
    Function has 2 input borrows: `a: &0 String`, `b: &0 String`.
    
    --> src/example.tri:1:62
       |
    1  | function pick_longer(a: &0 String, b: &0 String) -> &0 String {
       |                                                     ^^^^^^^^^ ambiguous return borrow
    
    Suggested fixes:
    
    [Fix 1] Return owned value instead (requires cloning inside body):
    Change `-> &0 String` to `-> &+ String`
    
    [Fix 2] Group inputs into a collection with a single borrow scope:
    Refactor parameter list: change `(a: &0 String, b: &0 String)` to `(items: &0 Vector<String>)`
    
    [Fix 3] Encapsulate inside a struct method (ties return to `self`):
    Wrap logic in `impl StringPair { function longer(self: &0 StringPair) -> &0 String { ... } }`
```

---

## §4 — No Annotation Policy (D5)

**Lock:** Triết **không có** cú pháp `<'a>` hay tương đương. Khi elision fail → compiler dứt khoát refuse. Tác giả refactor theo 1 trong 3 suggestion ở E2400.

### 4.1 — Lý do refuse annotation

1. **Brand fit:** annotation viral là lý do số 1 dev rời Rust theo các survey. Triết tham vọng "Rust rigor + ergonomics tốt hơn".
2. **AI-friendly:** annotation cần global reasoning về lifetimes — khó cho LLM generate đúng. Refuse-with-refactor cho LLM 1 mục tiêu rõ ràng hơn.
3. **Long-term simplicity:** mỗi feature compiler không có là 1 feature ngôn ngữ không cần học, document, version.

### 4.2 — Trường hợp 5% không refactor được

Một số API design thực sự cần multi-input borrow tied to different lifetimes (rare in practice). Triết policy:

- **v0.8–v1.0:** Hoàn toàn refuse — author refactor.
- **post-v1.0:** Đánh giá lại. Nếu có concrete evidence từ self-hosting compiler hoặc kernel work, mở capability `dev::explicit_region` cho pattern này. Capability gate giữ audit-friendly.

Author 2026-05-26 chốt: ưu tiên **clean brand** ngay v1.0. Worst case: 1-2% codebase phải refactor — chấp nhận được.

---

## §5 — Use-After-Move (E2420, E2421)

**Lock:** Move semantics theo ADR-0022 §4.1. Compiler track mỗi `&+` binding qua dataflow analysis. Truy cập sau move → E2420. Cố tạo self-ownership → E2421.

### 5.1 — E2420 UseAfterMove

```triet
let alice: &+ User = create_user("Alice")
take(alice)                  // ownership moves into take()
print(alice.name)            // E2420
```

```text
E2420 UseAfterMove
    Cannot access `alice` after ownership was moved.
    Binding `alice` was consumed by `take()` on line 4.
    
    --> src/example.tri:5:11
       |
    4  |     take(alice)
       |          ----- ownership moved here
    5  |     print(alice.name)
       |           ^^^^^ used after move
    
    Suggested fixes:
    
    [Fix 1] Borrow instead of move (keeps `alice` usable after the call):
    Change `take(alice)` to `take(&0 alice)` if `take` accepts `&0 User`
    
    [Fix 2] Restructure so the value is only consumed once:
    Move `print(alice.name)` to before `take(alice)`
    
    [Fix 3] Clone before move (only if `User` opts into clone semantics):
    Change `take(alice)` to `take(alice.clone())`
```

### 5.2 — E2421 SelfOwnershipParadox

Đặc biệt cho trường hợp cố move biến vào field của chính nó (xem chứng minh ADR-0022 §6.2):

```triet
a.b_field.a_field = a       // E2421
```

```text
E2421 SelfOwnershipParadox
    Cannot move `a` into a field reachable from `a` itself.
    Linear ownership requires the source and destination to be distinct.
    This is one of the patterns prevented by the no-cycle theorem (ADR-0022 §6).
    
    --> src/example.tri:7:5
       |
    7  |     a.b_field.a_field = a
       |     ^^^^^^^^^^^^^^^^^^^^^ ownership cycle attempt
    
    Suggested fixes:
    
    [Fix 1] Use a weak back-edge (recommended for parent-child or graph back-edges):
    Refactor `a_field` to a weak observer: change `a_field: &+ A` to `a_field: &- A`, then write the assignment as `a.b_field.a_field = &- a`
    
    [Fix 2] Restructure so the back-edge is not owning:
    Replace the back-link with an index or ID that resolves through a registry, removing the reference entirely
```

### 5.3 — Algorithm: move-state tracking

Compiler maintain mỗi `&+` binding 1 trạng thái 3-state:

- **Owned** — binding hợp lệ, có quyền truy cập.
- **Moved** — đã chuyển ownership đi, truy cập → E2420.
- **Conditionally moved** — moved trên 1 nhánh, owned trên nhánh khác (sau if/match) → merge logic.

Conditional move thường được resolve bằng cách compiler insert "drop flags" hoặc force re-assignment trước use point. Chi tiết defer implementation phase.

---

## §6 — Constructibility Termination (E2422)

**Lock:** Theo ADR-0022 §6.4, mỗi struct có recursive `&+ T` reference (T = Self trực tiếp hoặc qua chain) phải có **base case** để constructor terminate. Compiler check local, không cần SCC global.

### 6.1 — Algorithm

Cho struct `S` có field `f: F`:

1. Compute *reaches-self* set: tập kiểu T mà từ F có thể "reach" tới S thông qua `&+` ownership chain.
2. Nếu S ∈ reaches-self(F) → S là self-recursive qua F.
3. Self-recursive thì F phải là **terminable type**: `(&+ T)?`, hoặc `Vector<&+ T>` / `Map<K, &+ T>` / collection có empty state, hoặc `&- T` (weak, không count).
4. Nếu không → E2422.

### 6.2 — E2422 NonTerminatingConstruction

```triet
public struct Node {
    value: Integer,
    next: &+ Node               // E2422 — không terminable
}
```

```text
E2422 NonTerminatingConstruction
    Struct `Node` has recursive ownership through field `next: &+ Node`,
    but the field is not terminable. The constructor would require an
    infinite chain of pre-existing `Node` instances.
    
    --> src/example.tri:3:5
       |
    3  |     next: &+ Node
       |     ^^^^^^^^^^^^^ recursive field has no base case
    
    Suggested fixes:
    
    [Fix 1] Make the field nullable so the chain can terminate with `~0` (most common):
    Change `next: &+ Node` to `next: (&+ Node)?`
    
    [Fix 2] Use a collection that terminates naturally with the empty state:
    Change `next: &+ Node` to `children: Vector<&+ Node>`
    
    [Fix 3] Use a weak reference if this is not the owning chain:
    Change `next: &+ Node` to `next: &- Node`
```

### 6.3 — Không phải cycle check

E2422 là **local property check** (1 field, 1 struct definition tại 1 thời điểm). Không build type graph, không SCC. O(N) trên số field, không O(N²) như cycle detection.

Đây là điểm Triết khác Rust: Rust check size finiteness qua trait `Sized` + bounds. Triết check **constructibility** trực tiếp.

---

## §7 — Mutability Enforcement (E2410, E2411)

**Lock:** ADR-0022 §3.4 declares "frozen owner không promote được". Compiler enforce qua 2 error codes.

### 7.1 — E2410 CannotMutateFrozenOwner

```triet
let owner: &+ User = create_user("Alice")     // frozen owner
owner.name = "Bob"                              // E2410
```

```text
E2410 CannotMutateFrozenOwner
    Cannot mutate field `name` of frozen owner `owner: &+ User`.
    Frozen owners are read-only for their entire lifetime (ADR-0022 §3.4).
    
    --> src/example.tri:2:5
       |
    1  |     let owner: &+ User = create_user("Alice")
       |                ------- frozen owner declared here
    2  |     owner.name = "Bob"
       |     ^^^^^^^^^^^^^^^^^^ mutation through frozen reference
    
    Suggested fixes:
    
    [Fix 1] Declare the owner as mutable at construction site:
    Change `let owner: &+ User` to `let owner: &+ mutable User`
    
    [Fix 2] Construct a fresh owner with the new value (functional style):
    Replace `owner.name = "Bob"` with a new binding that constructs a fresh `User` with all fields copied explicitly and `name` set to `"Bob"`
```

### 7.2 — E2411 CannotPromoteFrozenToMutable

```triet
let frozen: &+ User = create_user("Alice")
let mutable_handle: &+ mutable User = frozen      // E2411
```

```text
E2411 CannotPromoteFrozenToMutable
    Cannot promote `&+ User` (frozen owner) to `&+ mutable User`.
    Frozen ownership is permanent — promotion would break the
    "safe to share across actor boundary" invariant from ADR-0026 §3.
    
    --> src/example.tri:2:46
       |
    1  |     let frozen: &+ User = create_user("Alice")
       |                 ------- declared frozen here
    2  |     let mutable_handle: &+ mutable User = frozen
       |                                           ^^^^^^ frozen-to-mutable promotion
    
    Suggested fixes:
    
    [Fix 1] Declare as mutable at construction, derive frozen view only when sharing:
    Replace line 1 with `let frozen: &+ mutable User = create_user("Alice")` and remove line 2
    
    [Fix 2] Keep frozen ownership and construct a fresh mutable owner with fields copied explicitly:
    Replace line 2 with a new binding that constructs `&+ mutable User` by reading each field from `frozen`
```

### 7.3 — Field-level mutability granularity

```triet
public struct User {
    id: UserId,                  // immutable field (set 1 lần)
    mutable display_name: String  // mutable field
}

let mutable u: &+ mutable User = User { id: ..., display_name: "Alice" }
u.display_name = "Bob"            // OK — field is mutable, owner is mutable
u.id = NewId                      // E2410 — field is immutable regardless of owner
```

Mutability của field **độc lập** với mutability của owner. Owner mutable cho phép tổng quát mutate; field cần thêm `mutable` keyword nếu muốn cho sửa.

---

## §8 — `&-` Upgrade & Scope Rules

**Lock:** ADR-0022 §9 declares `.upgrade()` trả `T?`. Compile-time invariant: `&-` chỉ tồn tại khi tracer back tới ≥ 1 `&+` còn live. Vi phạm → E2403.

### 8.1 — E2402 BorrowInStructField (từ ADR-0022 §7.1)

```triet
public struct BadIdea {
    cursor: &0 Tryte              // E2402
}
```

```text
E2402 BorrowInStructField
    Field `cursor: &0 Tryte` cannot be stored in struct `BadIdea`.
    Scope borrows (&0) are bound to the calling scope and cannot
    persist as struct fields (ADR-0022 §7.1).
    
    --> src/example.tri:2:5
       |
    1  | public struct BadIdea {
    2  |     cursor: &0 Tryte
       |     ^^^^^^^^^^^^^^^^ scope borrow stored as field
    3  | }
    
    Suggested fixes:
    
    [Fix 1] Use an owned reference (struct takes ownership of the byte):
    Change `cursor: &0 Tryte` to `cursor: &+ Tryte`
    
    [Fix 2] Use a weak reference (observer pattern, ownership stays elsewhere):
    Change `cursor: &0 Tryte` to `cursor: &- Tryte`
    
    [Fix 3] Use offset-based pattern under `dev::self_ref` capability (ADR-0022 §7.2):
    Change `cursor: &0 Tryte` to `cursor_offset: Integer` and declare `dev::self_ref: grant` in dao.package
```

### 8.2 — E2403 WeakRefOutlivesOwner

```triet
function escape() -> &- Process {
    let p: &+ Process = create_process()
    return &- p                   // E2403 — weak ref outlives the &+ owner
}
```

```text
E2403 WeakRefOutlivesOwner
    Weak reference `&- p` cannot escape the scope where owner `&+ p` lives.
    After this function returns, `p` drops and the weak ref dangles.
    
    --> src/example.tri:3:12
       |
    2  |     let p: &+ Process = create_process()
       |            ----------- owner created in local scope
    3  |     return &- p
       |            ^^^^ weak ref escapes the owner's scope
    
    Suggested fixes:
    
    [Fix 1] Return the owner instead so the caller decides lifetime:
    Change `return &- p` to `return p` and change return type to `&+ Process`
    
    [Fix 2] Accept a long-lived owner from the caller:
    Refactor function signature: add a parameter `owner: &0 Process` and derive `&- owner` inside the body
    
    [Fix 3] Restructure to store the weak ref in a long-lived struct owned by the caller:
    Pass a `&0 mutable Registry` parameter and insert the weak ref into it instead of returning
```

### 8.3 — Algorithm: trace weak to owner

Compiler maintain mỗi `&-` 1 "owner trail" — chain ngược tới gốc `&+`. Khi `&-` được assign, store, hoặc return, compiler check owner trail còn valid trong destination scope không. Implementation phase v0.10.

### 8.4 — Upgrade pattern

```triet
let weak: &- Process = ...
let result: Process? = weak.upgrade()

match result {
    ~+ proc => use_process(proc),
    ~0      => log("target dropped or never set")
}
```

`.upgrade()` là method built-in của type `&- T`. Trả `T?` per ADR-0022 §9. Force match — không có silent deref tới `T` trực tiếp.

---

## §9 — Default Inference per Namespace

**Lock:** ADR-0022 §5 declares quy tắc infer khác nhau giữa `usr::` và `sys::/dev::`. Algorithm chi tiết:

### 9.1 — Algorithm

Cho mỗi vị trí có thể ngầm reference type (struct field, function param, return), compiler:

1. Check namespace của module containing declaration.
2. Nếu `usr::` → apply default inference (xem 9.2).
3. Nếu `sys::*` hoặc `dev::*` → require explicit `&+`/`&0`/`&-` ngay. Không infer.
4. Nếu vi phạm namespace rule → E2430.

### 9.2 — Default inference table (usr::)

| Vị trí | Type declared | Inferred reference |
|---|---|---|
| Struct field | `field: T` (T là heap type) | `&+ T` (owned, immutable) |
| Struct field | `field: T` (T là value type — primitive, tuple) | `T` (no ref) |
| Function param | `param: T` (T là heap type) | `&0 T` (borrow, read-only) |
| Function param | `param: T` (T là value type) | `T` (no ref) |
| Function return | inferred từ body | Owned `&+ T` nếu body construct, borrow nếu body return input |
| `let x = expr` | inferred từ expr | Match expr type |

### 9.3 — E2430 ImplicitRefInSystemNamespace

```triet
module sys.kernel.scheduler

public struct Process {
    state: ProcessState,           // E2430 — ngầm &+ trong sys:: namespace
}
```

```text
E2430 ImplicitRefInSystemNamespace
    Field `state: ProcessState` requires explicit reference form in
    `sys::kernel::scheduler`. Implicit inference is disabled in
    `sys::*` and `dev::*` namespaces (ADR-0022 §5.2).
    
    --> src/sys/kernel/scheduler.tri:4:5
       |
    3  | public struct Process {
    4  |     state: ProcessState
       |     ^^^^^^^^^^^^^^^^^^^ missing explicit reference form
    5  | }
    
    Suggested fixes:
    
    [Fix 1] Declare owned immutable reference (most common for kernel state):
    Change `state: ProcessState` to `state: &+ ProcessState`
    
    [Fix 2] Declare owned mutable reference (when scheduler updates state):
    Change `state: ProcessState` to `state: &+ mutable ProcessState`
    
    [Fix 3] Keep as value type (only when `ProcessState` is primitive or tuple):
    Verify `ProcessState` is a primitive, tuple, or one of the stack-allocatable types from SPEC §10.3; if so, leave declaration as-is. If it is a heap struct, use [Fix 1] or [Fix 2] instead.
```

### 9.4 — Sys/dev namespace inference exception

Capability `dev::ergonomic_inference` (TBD post-v1.0) có thể restore inference cho `dev::` namespace nếu dev contributor cần. Default off.

---

## §10 — Drop Order & Custom Drop

**Lock:** Field drop theo **reverse declaration order**. Custom destructor logic require capability `dev::custom_drop`.

### 10.1 — Default drop order

```triet
public struct Connection {
    socket: &+ Socket,           // dropped THIRD (last declared first)
    buffer: &+ Buffer,           // dropped SECOND
    log_handle: &+ LogHandle     // dropped FIRST
}
```

Lý do reverse: thường resource dependent later trên resource earlier (socket depends on buffer setup, log_handle independent). Drop in dependency order minimizes "drop A while A's resource depends on B that already dropped".

### 10.2 — Capability `dev::custom_drop`

```triet
// dao.package
capabilities {
    dev::custom_drop: grant
}

// module dev::driver::pci
public struct PciDevice {
    handle: &+ DeviceHandle,
    
    public function on_drop(self: &+ mutable PciDevice) {
        // Custom destructor — sync flush, IRQ disable, etc.
        // Called automatically when self goes out of scope.
    }
}
```

Restrictions on custom drop:
- Cannot access fields đã được dropped by default order.
- Cannot move self.
- Cannot panic / return error (use Outcome ADR-0020 internal, log at boundary).

Chi tiết E2450 + E2451 (custom drop violations) defer implementation phase.

### 10.3 — Interaction với move

```triet
let conn: &+ Connection = open_connection()
take(conn)                       // moved; on_drop runs INSIDE take() when conn goes out of scope there
print(conn.socket)               // E2420 — not E2410 — chỉ ra "moved" không phải "frozen"
```

Move transfers drop responsibility với value. Custom drop chạy ở scope cuối cùng owns the value.

---

## §11 — Worked Examples

### 11.1 — 90% case: simple borrow (elision rule 1)

```triet
function uppercase_first(s: &0 String) -> &0 String {
    return s.to_uppercase().first_word()   // Compiler suy ra return tie to `s`
}
```

✅ Works without annotation. Elision rule 1 applies.

### 11.2 — 90% case: method (elision rule 2)

```triet
public struct Lexer {
    source: String,
    cursor: Integer,
    
    public function peek(self: &0 Lexer) -> &0 Token {
        // Return tie to self
    }
}
```

✅ Works. Elision rule 2.

### 11.3 — Edge case (5%): multi-input → E2400

```triet
function pick_longer(a: &0 String, b: &0 String) -> &0 String { /* ... */ }
// E2400 — refactor needed
```

Tác giả refactor theo suggestion (a) return owned, hoặc (b) wrap inputs.

### 11.4 — NLL accepts what Rust 2015 rejected

```triet
let mutable v = make_vector()
let r = &0 v
let len = r.length          // last use of r
v.push(x)                    // OK với NLL
```

✅ NLL accept. Lexical reject. Triết = NLL.

### 11.5 — Borrow exclusivity violation → E2440

```triet
let mutable v = make_vector()
let r1 = &0 v
let r2 = &0 mutable v        // E2440 — conflict với r1 still active
print(r1.length)
```

### 11.6 — Move semantics → E2420

```triet
let alice: &+ User = create_user("Alice")
take(alice)                  // moved
alice.name                   // E2420
```

### 11.7 — Cycle attempt → E2421

```triet
let a: &+ A = ...
let b: &+ B = ...
a.b_field = b                // OK — first move
b.a_field = a                // E2420 — b is moved
// Or: a.b_field.a_field = a // E2421 — self-ownership paradox
```

### 11.8 — Constructibility termination → E2422

```triet
public struct Node {
    next: &+ Node           // E2422 — refactor to (&+ Node)? or Vector
}
```

### 11.9 — Mutability violation → E2410

```triet
let owner: &+ User = create_user("Alice")
owner.name = "Bob"          // E2410 — frozen
```

### 11.10 — Self-ref capability case (5%)

```triet
// dao.package
capabilities { dev::self_ref: grant }

public struct NetworkPacket {
    buffer: &+ Vector<Tryte>,
    header_offset: Integer       // offset-based, not real &0
}
```

✅ Capability gate documents intent. Pattern is offset-based, no actual `&0` stored.

---

## §12 — Implementation Phasing

| Version | Scope |
|---|---|
| **v0.8** | Parser tokens `&+`, `&0`, `&-`, `mutable`. AST nodes. No enforcement. Examples typecheck với type system relaxed. |
| **v0.9** | Simple enforcement: E2420 use-after-move (linear `&+` tracking). E2422 constructibility termination. E2410/E2411 mutability frozen. E2430 namespace inference. |
| **v0.10** | NLL borrow exclusivity (E2440). Lifetime elision 3 rules (E2400). E2402 borrow in struct field. |
| **v0.11** | `&-` upgrade tracking (E2403). Default inference per namespace fully working. Drop order. |
| **v1.0** | Capability `dev::custom_drop` (E2450, E2451). All E2400–E2459 stable. Self-hosting compiler uses borrow check. |
| **post-v1.0** | Evaluate `dev::explicit_region` need. Evaluate Polonius adoption. |

Self-hosting compiler bootstrap chain (v0.7) hiện chưa expose references trong stdlib — so các sub-task v0.8+ port lexer/parser/typecheck sang Triết-in-Triết sẽ là **first real client** của borrow checker. Bootstrap loop là gate functional cho từng version.

---

## §13 — Out of scope

- **Polonius adoption** — post-v1.0, evaluate evidence-based.
- **Two-phase borrows** — defer until concrete pattern surfaces.
- **Generic lifetime variance** — không có annotation → không có variance.
- **Async/await self-borrow** — defer cho concurrency runtime ADR (post-v0.8).
- **Trait object lifetimes** — defer cho dynamic dispatch ADR.
- **Closure capture rules** — defer cho closure ADR (planned post-v0.8).
- **FFI memory ownership** — defer cho FFI ADR.
- **Reborrow patterns** (`&0 mutable T` → `&0 T` temporary downgrade) — implementation detail, decide khi build §2.3 live-range analysis.

---

## §14 — Tham chiếu

- [ADR-0022 — Trit-Balanced Ownership](0022-trit-balanced-ownership.md) (parent — locks 5 reference forms, this ADR enforces them)
- [ADR-0026 — Actor Boundary & Send Rules](0026-actor-boundary-send-rules.md) (sibling, TODO — Send derivation depends on §7 frozen invariant)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) (`T?` reuse cho `.upgrade()` return)
- [ADR-0018 — Capability loader semantics](0018-capability-loader-semantics.md) (capability declaration model)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (`T?` 3-state semantic)
- [ADR-0021 — Trilean refinement](0021-trilean-refinement.md) (error message + refactor suggest pattern reuse cho E2400 series)
- [SPEC §10 — Memory model](../../SPEC.md) (sẽ rewrite đồng bộ khi ADR-0022 + ADR-0025 + ADR-0026 cùng land)
- [ROADMAP §v0.8](../../ROADMAP.md) (concurrency phase — depends on this ADR land first)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (cập nhật E24XX `triet::borrow::*` khi ADR land)
