# Phase 2 — Borrow Checker Design (dựa trên CFG + NLL)

**Status:** Partial — 2/5 error codes fire end-to-end; regression vs v0.10 borrowck (2026-06-04)
**See also:** `TODO.md` (live backlog + debt registry). REPORT-2026-06-04.md đã xóa — git history giữ.

**Dependency note:** Phase numbering ≠ build order. The borrow checker depends on
Phase 4 (AST→MIR lowerer) which produces the MIR bodies that borrowck analyzes.
Phase 4 must run before Phase 2 in any build pipeline.

**What actually works (verified against code):**

| Error code | Status |
|---|---|
| E2420 UseAfterMove | ✅ Fires end-to-end (CFG move-tracking + branch-aware join) |
| E2440 NllExclusivityViolation | ✅ Fires end-to-end (NLL dataflow, branch isolation, loop extension) |
| E2450 DropWhileBorrowed | ❌ Dead end-to-end — borrowck handles `Statement::Drop` but lowerer never emits it |
| E2400 LifetimeElision | ❌ Not implemented in new borrowck (v0.10 borrowck had 3-rule elision) |
| E2403 EscapingBorrow | ❌ Not implemented in new borrowck (v0.10 borrowck had escape analysis) |

**Regression:** The deleted v0.10 borrowck had E2400 (3-rule lifetime elision) and
E2403 (escaping borrow). The new borrowck has neither. This is a step backwards —
the new MIR-based borrowck covers fewer error codes than the old AST-based one.

**Known soundness debt:** `places_conflict(a, b, conservative=true)` treats any two
different base locals as conflicting for `&0`/`&-` — a conservative band-aid that
rejects valid shared-borrow programs. Proper alias analysis is future work.

**Implementation:** `crates/triet-borrowck/src/{lib.rs, checker.rs, liveness.rs}` (~980 dòng).
**Phụ thuộc:** `spec/schema/triet-schema.yaml` (Phase 1 — S6 model)
**Mentor yêu cầu:** "Không có bản vẽ CFG, cấm gõ một dòng code Rust nào!"

---

## 1. Tại sao AST không đủ cho NLL

AST là cây cú pháp — nó mô tả **cấu trúc** của chương trình, không phải **luồng thực thi**.
Ví dụ:

```
  let mutable x = 10;

  // Bắt đầu mượn (NLL: mượn chỉ sống đến lần dùng cuối)
  let r: &0 mutable Integer = &0 mutable x;
  write(r);       // ← lần dùng cuối của r
                   // ← NLL: r chết ở ĐÂY, không phải cuối block

  // Đoạn code sau không dùng r
  let y = x + 1;  // ← NLL: cái này OK vì r đã chết
  let z = x + 2;  // ← Lexical: cái này FAIL vì r vẫn "sống" trong scope
```

Trên AST, cấu trúc là:
```
Block
├── Let("x", 10)
├── Let("r", Borrow(&0 mutable, "x"))
├── Call("write", ["r"])
├── Let("y", Add("x", 1))
└── Let("z", Add("x", 2))
```

Không có cách nào để biết `r` được dùng lần cuối ở đâu nếu chỉ duyệt cây này
theo thứ tự từ trên xuống. Cần **phân tích liveness** trên một đồ thị biểu diễn
luồng điều khiển.

---

## 2. Thiết kế CFG (Control Flow Graph)

### 2.1 — Kiến trúc lowering: AST → MIR → CFG

Triết lowering pipeline:

```
AST (Expr/Stmt/Item)
    │
    ▼  lowering::ast_to_mir()
    │
MIR (Mid-level IR — simplified, flat, no nesting)
    │
    ▼  mir::build_cfg()
    │
CFG (BasicBlock graph + liveness)
    │
    ▼  borrowck::check()
    │
Borrow check result (pass / errors)
```

### 2.2 — MIR: cấu trúc phẳng, không đệ quy

MIR loại bỏ mọi nesting của AST. Mỗi statement MIR là một thao tác đơn giản:

```
MIR statement:
    Assign(dest: Local, src: Rvalue)
    Call(dest: Option<Local>, func: Local, args: Vec<Local>)
    MethodCall(dest: Option<Local>, receiver: Local, method: String, args: Vec<Local>)
    Borrow(dest: Local, form: ReferenceForm, source: Local)
    Drop(local: Local)
    StorageLive(local: Local)      // biến được tạo
    StorageDead(local: Local)      // biến bị hủy
```

`Local` là một index vào bảng `Locals` — flat, không có nesting. Mỗi biến,
temporary, và intermediate value đều có một `Local`.

MIR terminator (kết thúc mỗi basic block):
```
    Return(local: Option<Local>)
    Goto(target: BasicBlock)
    If(cond: Local, then_bb: BasicBlock, else_bb: BasicBlock)
    IfTernary(cond: Local, pos_bb: BasicBlock, zero_bb: BasicBlock, neg_bb: BasicBlock)
    Match(scrutinee: Local, arms: Vec<(Pattern, BasicBlock)>)
    CallDispatch(callee: Local, args: Vec<Local>, return_bb: BasicBlock, panic_bb: BasicBlock)
    Unreachable                        // sau loop {} hoặc return sớm
```

### 2.3 — Ví dụ lowering: AST → MIR → CFG

Code Triết:
```
function example(x: &+ mutable Integer) -> Integer {
    let r: &0 mutable Integer = &0 mutable x;
    let result = r + 1;          // deref r, add
    let y = 42;
    return result;
}
```

MIR:
```
bb0: {
    StorageLive(r)
    Borrow(r, &0 mutable, x)      // r = &0 mutable x
    StorageLive(result)
    Add(result, r, 1)             // result = r + 1  (r dùng lần cuối)
    StorageLive(y)
    Const(y, 42)                  // y = 42
    Return(result)
}
```

CFG:
```
    ┌──────────┐
    │  Entry   │
    └────┬─────┘
         │
         ▼
    ┌──────────┐
    │   bb0    │  Borrow(r, &0 mutable, x)
    │          │  Add(result, r, 1)
    │          │  Const(y, 42)
    │          │  Return(result)
    └────┬─────┘
         │
         ▼
    ┌──────────┐
    │   Exit   │
    └──────────┘
```

Ví dụ phức tạp hơn — có rẽ nhánh:
```
function abs_diff(a: Integer, b: Integer) -> Integer {
    if a > b {
        return a - b;
    } else {
        return b - a;
    };
}
```

MIR:
```
bb0: {
    Gt(cond, a, b)
    If(cond, bb1, bb2)
}

bb1: {
    Sub(tmp1, a, b)
    Return(tmp1)
}

bb2: {
    Sub(tmp2, b, a)
    Return(tmp2)
}
```

CFG:
```
         ┌──────────┐
         │  Entry   │
         └────┬─────┘
              │
              ▼
         ┌──────────┐
         │   bb0    │  Gt(cond, a, b)
         │          │─── If(cond) ───┐
         └──────────┘                │
              │ (false)              │ (true)
              ▼                      ▼
         ┌──────────┐          ┌──────────┐
         │   bb2    │          │   bb1    │
         │ Sub(b,a) │          │ Sub(a,b) │
         │ Return───┼──┐       │ Return───┼──┐
         └──────────┘  │       └──────────┘  │
                       │                     │
                       ▼                     ▼
                  ┌──────────┐          ┌──────────┐
                  │   Exit   │          │   Exit   │
                  └──────────┘          └──────────┘
```

### 2.4 — Xử lý Outcome propagation (`~?`)

`expr ~? |capture| early_return` chuyển thành branching trong CFG:

Code Triết:
```
let val = risky_call() ~? |e| return ~- e;
```

MIR (phân rã thành 3 block):
```
bb0: {
    CallDispatch(risky_call, [], bb_check, bb_panic)
}

bb_check: {              // kiểm tra Outcome discriminant
    OutcomeDiscriminant(disc, tmp)
    IfTernary(disc, bb_success, bb_null, bb_error)
}

bb_success: {            // ~+ : unwrap success
    OutcomeUnwrap(val, tmp)
    Goto(bb_continue)
}

bb_null: {               // ~0 : propagate null (T?~E only)
    ReturnNull
}

bb_error: {              // ~- : bind error, execute early_return
    OutcomeUnwrapError(capture, tmp)
    Return(~- capture)
}

bb_continue: {           // code sau ~?
    // ... sử dụng val
}
```

---

## 3. Thiết kế Borrow Checker

### 3.1 — Mô hình dữ liệu: Polonius-style

Mượn ý tưởng từ Rust's Polonius (NLL borrow checker), nhưng đơn giản hóa cho S6:

**Entities:**
- **Variable** — một `Local` trong MIR
- **Loan** — một lần mượn (`&0`, `&0 mutable`, `&-`). Loan có: source (biến bị mượn),
  form (loại reference), and region (khoảng thời gian loan tồn tại)
- **Region** — tập hợp các program point nơi loan còn hiệu lực

**Data structures chính:**

```
struct Loan {
    source: Local,           // biến bị mượn
    form: ReferenceForm,     // loại reference: BorrowReadOnly / BorrowExclusiveMutable / WeakObserver
    issued_at: ProgramPoint, // nơi tạo borrow
    last_use: ProgramPoint,  // lần dùng cuối (NLL: loan chết sau điểm này)
}

enum VariableState {
    Owned,                   // đang sở hữu — có thể move, borrow, hoặc drop
    Moved,                   // đã chuyển ownership — dead, không dùng được nữa
}

struct BorrowChecker {
    locals: Vec<LocalInfo>,  // thông tin mỗi biến
    loans: Vec<Loan>,        // tất cả loans đang active
}

struct LocalInfo {
    state: VariableState,    // trạng thái hiện tại
    var_type: Type,          // kiểu dữ liệu (từ type checker)
}
```

### 3.2 — Thuật toán: dataflow analysis trên CFG

Borrow checker chạy như một **forward dataflow analysis** trên CFG.
Tại mỗi program point, nó tính toán:

1. **Active loans** — những loan nào đang "sống"
2. **Variable states** — biến nào đang Owned, biến nào đã Moved

**Transfer function** — khi đi qua một MIR statement, borrow checker cập nhật state:

| Statement | Effect |
|---|---|
| `StorageLive(x)` | `x.state = Owned` |
| `StorageDead(x)` | Kết thúc mọi loan từ x (source.local); `x.state` removed |
| `Borrow(dest, form, source)` | **Conflict check**: nếu `places_conflict` + `conflicts_with(form)` → E2440. StrongFrozen/StrongMutable: conflict với MỌI loan (move invalidates references). Weak/borrow forms: tạo Loan. `dest.state = Owned` |
| `Move(dest, source)` | Conflict check với active loans. `source.state = Moved`; `dest.state = Owned` |
| `Call(dest, func, args)` | args có passing_mode Borrow → loan kết thúc sau call; Move → arg.state = Moved. Cross-call propagation qua `return_borrow_map` |
| `Drop(x)` | **Nếu active loans trên x → E2450 DropWhileBorrowed.** Sau đó: `x.state = Moved`, retain loans từ x |
| Sử dụng biến `x` | Cập nhật `loan.last_use = current_point` cho loan liên quan |

**Conflict check** — tại mỗi program point, kiểm tra:

| Hành động | Conflict condition | Error code |
|---|---|---|
| Tạo `&0 T` (shared borrow) | Conflict nếu có `&0 mutable` đang active trên cùng source | E2440 |
| Tạo `&0 mutable T` (exclusive) | Conflict nếu có BẤT KỲ loan nào đang active trên cùng source | E2440 |
| Tạo `&- T` (weak) | Conflict nếu có `&0 mutable` đang active | E2440 |
| Tạo `&+ T` / `&+ mutable T` (strong=move) | Conflict nếu có BẤT KỲ loan nào đang active — **move invalidates all references** | E2440 |
| Move `x` | Conflict nếu có BẤT KỲ loan nào đang active trên x | E2440 |
| Drop `x` | Conflict nếu có BẤT KỲ loan nào đang active trên x | **E2450** |
| Mutate qua `&0 T` | Conflict — &0 T là read-only | *(typecheck-level)* |
| Mutate qua `&+ T` (frozen) | Conflict — &+ T frozen không thể mutate | *(typecheck-level)* |

### 3.3 — NLL: khi nào một loan kết thúc?

**Lexical Lifetime (sai):** loan kết thúc ở `}` đóng block chứa borrow expression.

**Non-Lexical Lifetime (đúng):** loan kết thúc tại **program point sau lần dùng cuối cùng**
của reference được mượn. Điều này được tính bằng **liveness analysis**:

1. Với mỗi `Local`, tính tập hợp program point nơi nó "live" (có thể được đọc)
2. Một loan cho reference `r` (mượn từ `x`) kết thúc tại program point ngay sau
   lần dùng cuối của `r` (hoặc lần dùng cuối của bất kỳ reference nào derived từ `r`)

Ví dụ kinh điển:
```
bb0: {
    Borrow(r, &0 mutable, x)     // loan bắt đầu
    Write(r, 42)                  // ← lần dùng cuối của r
    // ← NLL: loan kết thúc ở ĐÂY
    Read(x)                       // ← OK! x không còn bị mượn
    Return(x)                     // ← OK!
}
```

Trong Lexical, `Read(x)` sẽ fail vì `r` chưa ra khỏi scope. Trong NLL, nó pass
vì loan đã kết thúc ngay sau `Write(r, 42)`.

### 3.4 — Phân biệt `&0 mutable` vs `&+ mutable` qua lời gọi hàm

Đây là câu hỏi 2 của mentor. Cơ chế:

**Hàm nhận borrow (`&0` hoặc `&0 mutable`):**
```
function write(buffer: &0 mutable VgaBuffer, text: String) -> Unit
//             ^^^^^^^^^^^^^^^^^^^^^^^^
//             Đây là BorrowExclusiveMutable — tham số được mượn
```

Khi gọi `write(vga, "hello")`:
1. Borrow checker tạo một **temporary loan**: `Loan { source: vga, form: BorrowExclusiveMutable, ... }`
2. Loan này tồn tại trong suốt thời gian hàm chạy
3. Khi hàm return, loan kết thúc — `vga` trở lại trạng thái cũ
4. **Caller giữ ownership** của `vga`

**Hàm nhận ownership (`&+` hoặc `owned`):**
```
function consume_buffer(buffer: &+ mutable VgaBuffer) -> Unit
//                         ^^^^^^^^^^^^^^^^^^^^^^^^
//                         Đây là StrongMutable — tham số được chuyển quyền
```

Khi gọi `consume_buffer(vga)`:
1. Borrow checker kiểm tra: `vga` có đang bị mượn không? Nếu có → conflict.
2. `vga.state = Moved` — caller mất quyền truy cập
3. **Callee sở hữu** `vga`

**Cách borrow checker biết:** từ type signature của hàm được gọi.
`ParameterPassing` trong schema định nghĩa 3 chế độ: `Borrow`, `Move`, `MutableBorrow`.
Khi typechecker resolve lời gọi hàm, nó gán `ParameterPassing` cho từng argument.
Borrow checker đọc thông tin này từ MIR:

```
// MIR cho lời gọi write(vga, "hello"):
// write có signature: (buffer: &0 mutable VgaBuffer, text: String) -> Unit
// ParameterPassing: buffer=MutableBorrow, text=Move

CallDispatch(write, [vga, "hello"], return_bb, panic_bb)
//               ^-- MutableBorrow: tạo temporary loan, kết thúc sau call
//                      ^-- Move: "hello".state = Moved sau call
```

---

## 4. SỬA LỖI SOUNDNESS: Loan Propagation qua giá trị trả về (mentor phát hiện)

### 4.1 — Lỗ hổng ban đầu

Thiết kế §3.2 ghi: "`Call(dest, func, args)` — args có passing_mode Borrow → loan kết thúc sau call."

**Điều này SAI** nếu hàm trả về một reference derived từ parameter:

```
function get_first_cell(buffer: &0 mutable VgaBuffer) -> &0 mutable VgaCell {
    return buffer.cells[0]; // Trả về reference tới phần tử bên trong buffer
}

function test(vga: &+ mutable VgaBuffer) {
    let cell: &0 mutable VgaCell = get_first_cell(&0 mutable vga);
    // Nếu loan kết thúc ở đây → consume_buffer được phép move vga
    consume_buffer(vga); // ← vga bị move
    write_cell(cell);    // ← USE-AFTER-FREE! cell trỏ vào vga đã bị move
}
```

Cell mượn từ vga. Khi get_first_cell return, cell vẫn còn sống → loan trên vga
không được kết thúc. Nó phải tồn tại đến khi cell chết.

### 4.2 — Cơ chế sửa: Lifetime Dependency + Loan Propagation

**Nguyên lý:** Khi một hàm trả về reference, lifetime của return value có thể bị
ràng buộc với lifetime của input parameters. Callee's signature xác định
dependency này qua **lifetime elision rules** (ADR-0025 §6, 3 rules). Caller's
borrow checker dùng dependency này để **kéo dài loan** của argument tới khi
returned reference chết.

**Bước 1 — Callee-side: Lifetime elision xác định dependency**

Với mỗi function signature, borrow checker (khi check thân hàm) hoặc type checker
(khi resolve signature) áp dụng 3 elision rules:

| Rule | Điều kiện | Kết quả |
|---|---|---|
| Rule 1 | Mỗi elided parameter reference có lifetime riêng | `fn foo(a: &0 T, b: &0 U)` — a và b có lifetime độc lập |
| Rule 2 | Chỉ có 1 input reference → output reference mượn từ nó | `fn foo(a: &0 T) -> &0 U` — output lifetime = a's lifetime |
| Rule 3 | Có `self` parameter → output mượn từ self | `fn method(&0 self) -> &0 T` — output lifetime = self's lifetime |

Với `get_first_cell(buffer: &0 mutable VgaBuffer) -> &0 mutable VgaCell`:
- Rule 2 khớp: 1 input reference (`buffer`) → output reference (`VgaCell`) mượn từ `buffer`.
- **Dependency: return → buffer (index 0)**

Thông tin dependency được lưu trong MIR function signature (sử dụng Field-level tracking để tránh over-restrictive borrows):
```rust
/// Maps each field path in the return value to the argument indices it borrows from.
/// - `FieldPath::Root` — the whole return value (direct reference returns)
/// - `FieldPath::Field("name")` — a named struct field
pub type ReturnBorrowMap = BTreeMap<FieldPath, BTreeSet<usize>>;

struct FunctionSignature {
    params: Vec<(Local, ParameterPassing)>,
    return_type: Type,
    return_borrow_map: ReturnBorrowMap,
}
```

Với `get_first_cell`: `return_borrow_map = { Root -> {0} }` (cả giá trị trả về phụ thuộc param index 0).
Với ví dụ `split_vga` trả về struct chứa reference: `return_borrow_map = { Field("left") -> {0}, Field("right") -> {1} }`.

Với hàm không trả về reference: `return_borrow_map` rỗng.

**Bước 2 — Caller-side: Loan propagation trong MIR**

Khi caller gọi hàm có `return_borrow_map` không rỗng, borrow checker (khi truy xuất trường tương ứng)
**không** kết thúc loan của các argument bị dependency. Thay vào đó, nó tạo
một **PropagatedLoan** từ argument tới return value:

```
// MIR cho: let cell = get_first_cell(&0 mutable vga);

CallDispatch(get_first_cell, [vga], return_bb, panic_bb)
//                              ^-- argument index 0
// get_first_cell có return_borrow_map = { Root -> {0} }
// → PropagatedLoan { source_arg: vga, dest_return: cell, field: Root... }
// Loan trên vga KHÔNG kết thúc. Nó tồn tại đến khi cell chết.
```

**Transfer function cập nhật (sửa §3.2):**

| Statement | Effect (đã sửa) |
|---|---|
| `CallDispatch(callee, args, return_bb, _)` | Với mỗi arg có passing_mode Borrow: nếu `return_borrow_map` chứa index của arg ở bất kỳ field nào → KHÔNG kết thúc loan ngay lập tức. Thay vào đó, tạo PropagatedLoan gắn với field path cụ thể của return_value. Nếu arg KHÔNG nằm trong map → loan kết thúc sau call. (Khi caller sử dụng field nào của return_value, chỉ các propagated loan của field đó mới được kéo dài `last_use`). |
| Sử dụng `x` | Nếu `x` là kết quả của một PropagatedLoan → cập nhật `last_use` cho loan gốc của source argument |

### 4.3 — End-to-end trace với Loan Propagation

```
function test(vga: &+ mutable VgaBuffer) {
    // vga: Owned

    let cell = get_first_cell(&0 mutable vga);
    // MIR: Borrow(tmp, &0 mutable, vga)    → Loan#1 { source: vga, form: ExclusiveBorrow }
    //      CallDispatch(get_first_cell, [tmp], ...)
    //      get_first_cell có return_borrow_map = { Root -> {0} }
    //      → PropagatedLoan { source_loan: Loan#1, dest: cell }
    //      Loan#1 KHÔNG kết thúc — nó được propagate qua cell

    consume_buffer(vga);
    // MIR: Move(tmp, vga)
    //      Check: vga có loan active không? CÓ — Loan#1 vẫn sống (qua cell)
    //      → E2420 UseAfterMove: cannot move vga while cell is still borrowing it
    //      COMPILE ERROR!
}

function test_fixed(vga: &+ mutable VgaBuffer) {
    let cell = get_first_cell(&0 mutable vga);
    // Loan#1 bắt đầu, propagate qua cell

    write_cell(cell);   // ← lần dùng cuối của cell
    // NLL: cell chết ở đây → PropagatedLoan kết thúc → Loan#1 kết thúc
    // → vga trở lại Owned

    consume_buffer(vga); // ← OK! vga không còn bị mượn
}
```

### 4.4 — Data structure cập nhật

```
struct Loan {
    source: Local,
    form: ReferenceForm,
    issued_at: ProgramPoint,
    last_use: ProgramPoint,
    propagated_to: Vec<Local>,  // NEW: return values this loan is propagated to
}

struct PropagatedLoan {
    source_loan: LoanId,        // loan gốc trên argument
    dest_local: Local,          // return value reference
    callee_signature: FunctionId, // để trace lỗi
}
```

---

## 5. Tổ chức code (Rust)

```
crates/triet-borrowck/
├── Cargo.toml
├── src/
│   ├── lib.rs              // entry point: pub fn check(mir: &Mir) -> Result<(), Vec<BorrowError>>
│   ├── mir.rs              // MIR data structures (Local, Statement, Terminator, BasicBlock, Body)
│   ├── lowering.rs         // AST → MIR lowering
│   ├── cfg.rs              // CFG construction từ MIR (build BasicBlock graph + predecessor/successor)
│   ├── liveness.rs         // Liveness analysis trên CFG (compute live-in/live-out sets per block)
│   ├── loans.rs            // Loan tracking (create, kill, active set per program point)
│   ├── checker.rs          // Borrow check logic (conflict detection + NLL region inference)
│   └── error.rs            // E24XX error types
└── tests/
    ├── nll_basic.rs        // NLL test cases
    ├── move_after_use.rs   // E2420 test cases
    ├── exclusive_borrow.rs // E2440 test cases
    └── function_calls.rs   // Borrow/move across function calls
```

---

## 5. Ví dụ end-to-end

Code Triết:
```
function write_twice(vga: &+ mutable VgaBuffer) -> Unit {
    // Mượn lần 1
    let b1: &0 mutable VgaBuffer = &0 mutable vga;
    write_cell(b1, 0, 0, 'H', 0x0F);
    // ← NLL: b1 kết thúc ở đây (lần dùng cuối là write_cell)

    // Mượn lần 2 — OK vì b1 đã chết
    let b2: &0 mutable VgaBuffer = &0 mutable vga;
    write_cell(b2, 0, 1, 'i', 0x0F);
    // ← NLL: b2 kết thúc

    // vga vẫn valid, có thể move tiếp
    consume(vga);
}
```

MIR + borrow check trace:
```
bb0: {
    // Initial state: vga=Owned
    Borrow(b1, &0 mutable, vga)    // → Loan#1 { source: vga, form: ExclusiveBorrow }
                                    //   Check: vga có loan nào không? Không → OK
    Call(write_cell, [b1, ...])     //   b1.last_use = here
    // Loan#1 kết thúc (NLL: b1 không dùng nữa)
    // → vga: Owned (trở lại)

    Borrow(b2, &0 mutable, vga)    // → Loan#2 { source: vga, form: ExclusiveBorrow }
                                    //   Check: vga có loan nào active không? Không (Loan#1 đã chết) → OK
    Call(write_cell, [b2, ...])     //   b2.last_use = here
    // Loan#2 kết thúc

    Move(tmp, vga)                  //   vga.state = Moved
    Call(consume, [tmp])            //   OK
    Return(())
}
```

---

## 6. Điểm mở (chưa giải quyết trong phase 2)

1. **Loop-carried borrows** — borrow tồn tại qua nhiều vòng lặp. Cần fixed-point
   iteration trên CFG cycle. Đây là phần khó nhất của NLL. Defer đến Phase 2.1.

2. **Inter-procedural borrow check** — kiểm tra borrow qua biên giới hàm.
   Phase 2 chỉ làm intra-procedural (trong 1 hàm). Defer đến Phase 2.2.

3. **Closure captures** — lambda đóng biến từ scope ngoài. Cần escape analysis.
   Defer đến Phase 2.3.

4. **`&- T` upgrade** — kiểm tra upgrade `&-` → `&0` có an toàn không (owner
   còn sống?). Cần lifetime analysis giữa owner và weak ref. Defer.

---

## 7. Câu hỏi cho mentor

1. **MIR granularity:** Có nên để MIR là một cấu trúc riêng biệt (separate crate)
   hay nhúng trực tiếp vào borrowck crate? Rust dùng MIR riêng — tôi đề xuất
   `triet-mir` crate riêng, dùng chung cho cả borrow checker và codegen.

2. **Liveness analysis:** Dùng thuật toán nào? Standard backward dataflow
   (live-in/live-out sets) hay sparse analysis (SSA-based)? Với quy mô hiện tại,
   standard backward dataflow đủ dùng — SSA-based là tối ưu hóa sớm.

3. **Polonius-style facts:** Có nên dùng Polonius (Datalog-style fact generation)
   không? Tôi đề xuất **không** — quá phức tạp cho phase 2. Standard dataflow
   analysis đủ để bắt 95% lỗi borrow. Polonius chỉ cần khi tối ưu hóa compile time.
