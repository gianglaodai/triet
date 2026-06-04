# Phase 3 — Cranelift JIT/AOT Backend Design

**Status:** Draft — thiết kế kiến trúc, chưa code
**Phụ thuộc:** `triet-mir` (MIR data structures + CFG), Phase 2 borrow checker
**Nguyên tắc:** IR → machine code từ day 0. Borrow checker đảm bảo safety ở compile-time; runtime chỉ là raw addresses + arithmetic.

---

## 1. Tổng quan pipeline

```
Triết source
    │
    ▼ triet-lexer + triet-parser       (AST)
    ▼ triet-typecheck                  (typed AST)
    ▼ AST → MIR lowering               (triet-mir: flat, non-SSA)
    ▼ Borrow checker                   (triet-borrowck: NLL dataflow)
    ▼ MIR → Cranelift IR lowering      (TRIET-JIT: THIS PHASE)
    ▼ Cranelift codegen                (machine code)
    ▼ Execution / AOT cache
```

**Phạm vi Phase 3:** module `triet-jit` mới (hoặc viết lại từ bản nháp v0.11), nhận `triet_mir::Body`, sinh Cranelift IR, compile, chạy.

---

## 2. Câu hỏi 1: MIR `Local` → Cranelift SSA

### 2.1 — Vấn đề

```triet
let mutable x = 10;       // x = Local(0)
x = x + 1;               // x bị gán đè — NON-SSA
x = x * 2;               // x bị gán đè lần nữa
```

MIR có `Local` với `Statement::Assign` cho phép gán đè. Cranelift IR là **strict SSA** — mỗi `Value` được định nghĩa đúng một lần.

### 2.2 — Giải pháp: `cranelift_frontend::Variable`

**Không tự viết SSA pass.** Cranelift cung cấp sẵn `FunctionBuilder` + `Variable` để xử lý việc này:

```rust
use cranelift_frontend::{FunctionBuilder, Variable};

// Mỗi Local trong MIR → một Cranelift Variable
let mut builder = FunctionBuilder::new(...);
let var_x = Variable::new(0);  // Local(0) → Variable(0)

// Gán giá trị (có thể gọi nhiều lần — FunctionBuilder tự SSA-ify)
builder.def_var(var_x, value_v1);   // x = 10
builder.def_var(var_x, value_v2);   // x = x + 1  (tạo SSA Value mới)
builder.def_var(var_x, value_v3);   // x = x * 2

// Đọc giá trị hiện tại
let current_x = builder.use_var(var_x);  // trả về SSA Value mới nhất
```

**Cơ chế bên trong `FunctionBuilder`:**
- `declare_var(var, type)` — đăng ký variable
- `def_var(var, val)` — ghi một SSA Value mới vào variable. Builder lưu giá trị này vào side-table.
- `use_var(var)` — trả về SSA Value được gán gần nhất. Ở cuối block, nếu variable được định nghĩa trong block này, `use_var` trả về định nghĩa cuối cùng. Nếu không, nó đọc từ block parameters (SSA φ được tự động tạo khi seal block).
- Khi 2 predecessor block cùng định nghĩa 1 variable → Cranelift **tự động chèn block parameter** (tương đương φ-node) khi seal block.

**Đây chính là câu trả lời cho câu hỏi "tự viết SSA pass": KHÔNG. Dùng `Variable` của Cranelift.**

### 2.3 — Mapping

| MIR | Cranelift |
|---|---|
| `Local(0)` | `Variable::new(0)` |
| `Statement::Assign { dest, source }` | `builder.def_var(var_dest, val_source)` |
| `Statement::Const { dest, value }` | `builder.def_var(var_dest, builder.iconst(..., value))` |
| `Statement::BinaryOp { dest, op, left, right }` | `builder.def_var(var_dest, builder.ins().iadd(l, r))` |
| `Statement::Borrow { dest, form, source }` | `builder.def_var(var_dest, builder.use_var(var_source))` — reference = pointer, copy the address |
| Terminator `Return { value }` | `builder.ins().return_(&[val])` |

---

## 3. Câu hỏi 2: CFG Traversal & Block Sealing

### 3.1 — Vấn đề

Cranelift's `FunctionBuilder` có protocol nghiêm ngặt:

1. **`create_block()`** — khai báo block (có thể làm target của branch trước khi fill)
2. **`switch_to_block()`** — bắt đầu fill instructions vào block
3. **`seal_block()`** — đóng block. Sau khi seal, không thể thêm predecessor hoặc instruction.
4. **Quy tắc:** Block chỉ seal được khi TẤT CẢ predecessors của nó đã được fill (hoặc ít nhất đã khai báo). Nếu seal sớm → block parameter không đầy đủ → lỗi.

May mắn: `triet-mir` đã có CFG với đầy đủ predecessor/successor → ta biết chính xác khi nào 1 block có thể seal.

### 3.2 — Thuật toán duyệt: Reverse Post-Order (RPO) + Worklist

```
Algorithm: lower_cfg_to_cranelift(body)

1. PRE-DECLARE: với mỗi BasicBlock trong MIR CFG:
   - cranelift_block[i] = builder.create_block()

2. MAP entry: set entry block's block params cho các parameter variable

3. RPO traversal (cho acyclic CFG):
   order = reverse_post_order(cfg)
   for block in order:
       lower_block(block)
       seal_if_ready(block)

4. Xử lý loop back-edges (cho cyclic CFG):
   - Khi gặp back-edge (successor đã được fill trước predecessor):
     * Block target đã được seal từ trước → không sao, Jump vẫn hợp lệ.
     * Nhưng Variable ở loop header cần block param cho giá trị từ back-edge.
     * Cranelift tự xử lý: khi seal block, nó tạo block param cho mỗi variable
       được dùng mà chưa được def trong block đó.
     * Back-edge cung cấp giá trị qua Jump args.

5. CHECK: sau khi duyệt hết, tất cả block phải được seal.
   Nếu còn block chưa seal → unreachable code (không predecessor nào fill xong) → seal với empty state.
```

### 3.3 — Reverse Post-Order cho acyclic CFG

```
fn reverse_post_order(cfg: &ControlFlowGraph) -> Vec<BasicBlock> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    dfs_post(cfg, cfg.entry, &mut visited, &mut order);
    order.reverse();  // RPO
    order
}

fn dfs_post(cfg, block, visited, order) {
    if visited.contains(block) { return; }
    visited.insert(block);
    for succ in cfg.successors[block] {
        dfs_post(cfg, succ, visited, order);
    }
    order.push(block);
}
```

RPO đảm bảo: predecessor được visit trước successor (trừ back-edge của loop). Đây là thứ tự tối ưu để seal block ngay sau khi fill.

### 3.4 — Seal logic

```
fn seal_if_ready(block) {
    // Một block sẵn sàng để seal khi tất cả predecessors đã được fill
    let all_preds_filled = cfg.predecessors[block]
        .iter()
        .all(|p| filled_blocks.contains(p));

    if all_preds_filled {
        builder.seal_block(cranelift_block[block]);
        sealed_blocks.insert(block);
    }
}
```

Với RPO, các block acyclic sẽ được seal ngay. Loop header cần 2 pass: pass 1 fill body, pass 2 (back-edge đã fill) → seal header.

---

## 4. Câu hỏi 3: Type Lowering & ABI

### 4.1 — Type mapping: MIR → Cranelift

| Triết Type | Cranelift Type | Ghi chú |
|---|---|---|
| `Trit` | `i8` | 2-bit payload, sign-extended |
| `Tryte` | `i16` | 9-trit, sign-extended |
| `Integer` | `i64` | 27-trit, sign-extended |
| `Long` | `i128` | 81-trit |
| `Trilean` | `i8` | +1=True, 0=Unknown, -1=False |
| `Unit` | `()` hoặc không có | Zero-sized, không chiếm register |
| `&+ T` | `i64` | Con trỏ thô (64-bit address space) |
| `&0 T` | `i64` | Con trỏ thô — giống hệt `&+` ở runtime |
| `&0 mutable T` | `i64` | Con trỏ thô |
| `&- T` | `i64` | Con trỏ thô |
| `String` | `i64` | Con trỏ tới heap object (Rc<RuntimeValue>) |
| `Vector<T>` | `i64` | Con trỏ tới heap object |
| `Outcome T~E` | `{i8, union_payload}` | Trit discriminant + payload (max(sizeof(T), sizeof(E))) |

### 4.2 — S6 References ở Machine Code = CON TRỎ THÔ (Zero-Cost Abstraction)

**Đây là điểm quan trọng nhất.** Toàn bộ S6 ownership model (`&+`, `&0`, `&0 mutable`, `&-`, `owned`) là **compile-time concept**. Borrow checker đã chứng minh memory safety. Khi xuống machine code:

```
&+ mutable VgaBuffer   →  i64 (địa chỉ 0xB8000)
&0 mutable VgaBuffer   →  i64 (cùng địa chỉ 0xB8000)
&0 VgaBuffer           →  i64 (cùng địa chỉ 0xB8000)
```

**Tất cả đều là cùng một con trỏ.** Sự khác biệt chỉ tồn tại trong type checker và borrow checker. Runtime không hề biết đến S6 — nó chỉ thấy địa chỉ bộ nhớ.

**Hardware Token:**

```
kernel_main(hw: &+ mutable Hardware)
                         │
                         ▼
            hw.take_vga() — method call, receiver được move
                         │
                         ▼
            Trả về i64 = 0xB8000 (địa chỉ VGA buffer)
                         │
                         ▼
            write_string(vga, "Hello") — nhận i64, ghi vào địa chỉ đó
```

`Hardware` struct biến mất hoàn toàn ở runtime. `take_vga()` được inline hoặc compile thành hàm trả về constant `0xB8000`. **Zero-cost abstraction** — đúng như nguyên tắc của Rust.

### 4.3 — Hàm ABI

**Tham số đầu vào:**
- Scalars (Trit, Tryte, Integer, Long, Trilean): truyền qua register (theo calling convention của nền tảng — System V AMD64).
- References (mọi S6 form): truyền qua register dưới dạng `i64`.
- Composites (structs): truyền qua register nếu vừa (≤ 16 byte), qua stack nếu lớn hơn.

**Giá trị trả về:**
- Scalar đơn: 1 register.
- Outcome `T~E`: 2 registers — Trit discriminant trong `rax` (i8), payload trong `rdx` (i64). Hoặc 1 struct `{i8, i64}` nếu ABI yêu cầu.
- Unit: không có giá trị trả về (void).

**Outcome ABI chi tiết:**

```
// T~E (binary outcome):
//   Success → rax = 1 (Trit::Positive), rdx = payload T
//   Error   → rax = -1 (Trit::Negative), rdx = payload E
//   Null    → không thể xảy ra (kiểu BinaryOutcome)

// T?~E (ternary outcome):
//   Success → rax = 1, rdx = payload T
//   Null    → rax = 0, rdx = unused
//   Error   → rax = -1, rdx = payload E
```

Cranelift implementation:
```rust
// Outcome discriminant → i8
let disc = builder.use_var(var_discriminant);

// Payload — union type, cast to i64 for ABI
let payload = builder.use_var(var_payload);

// Return as two values
builder.ins().return_(&[disc, payload]);
```

### 4.4 — Memory layout của `T?`

Như đã phân tích trong phase review trước:

```
T? với T là pointer:     sizeof = sizeof(ptr)  — niche: null pointer
T? với T là scalar:      sizeof = sizeof(T)    — niche: 1 bit trong T
T?? với inner pointer:   sizeof = 1 + pad + ptr — outer tag + inner niche
T?? với inner scalar:    sizeof = 1 + pad + sizeof(T) — outer tag + inner niche

Trit tag byte layout:
  0x00 = Trit::Positive (Some)
  0x01 = Trit::Zero (Null)
  0x02 = Trit::Negative (reserved / unused for nullable)
```

---

## 5. Kiến trúc module

```
crates/triet-jit/
├── Cargo.toml
├── src/
│   ├── lib.rs              // JIT entry point: pub fn compile(body: &Body) -> CompiledFunction
│   ├── lower.rs            // MIR → Cranelift IR lowering (the core)
│   ├── types.rs            // Type mapping: MIR types → Cranelift types
│   ├── abi.rs              // ABI: parameter passing, return value layout
│   ├── builtins.rs         // Built-in function shims (print, assert, ...)
│   └── cache.rs            // AOT cache (Phase 3.2 — deferred)
└── tests/
    ├── integer_arithmetic.rs  // 1 + 2 = 3 via JIT
    ├── function_calls.rs      // call + return
    ├── borrow_erased.rs       // verify S6 references = raw pointers at runtime
    └── outcome_abi.rs         // T~E return value via 2 registers
```

---

## 6. Ví dụ lowering: `abs_diff`

### Triết source
```triet
function abs_diff(a: Integer, b: Integer) -> Integer {
    if a > b {
        return a - b;
    } else {
        return b - a;
    };
}
```

### MIR (từ Phase 2)
```
bb0: {
    cond = a > b
    If(cond) → bb1 (true), bb2 (false)
}
bb1: {
    tmp1 = a - b
    Return(tmp1)
}
bb2: {
    tmp2 = b - a
    Return(tmp2)
}
```

### Cranelift IR (pseudo)
```
function u0:0(i64, i64) -> i64 {
    var0 = i64      // a
    var1 = i64      // b

block0:
    v0 = iconst.i64 0
    v1 = icmp sgt var0, var1
    brnz v1, block1, block2

block1:
    v2 = isub var0, var1
    return v2

block2:
    v3 = isub var1, var0
    return v3
}
```

### Machine code (x86-64, đã compile)
```asm
abs_diff:
    cmp     rdi, rsi
    jle     .Lelse
    sub     rdi, rsi
    mov     rax, rdi
    ret
.Lelse:
    sub     rsi, rdi
    mov     rax, rsi
    ret
```

---

## 7. Những gì KHÔNG làm trong Phase 3

1. **Không tự viết SSA pass.** Cranelift `Variable` + `FunctionBuilder` xử lý toàn bộ.
2. **Không GC / refcount trong JIT.** Mọi composite type ban đầu dùng delegate-to-VM shim (pattern từ v0.11 bản nháp). Native aggregate codegen là Phase 5+.
3. **Không AOT cache.** Phase 3 chỉ JIT (compile + run in-memory). AOT persistence là Phase 3.2.
4. **Không multi-thread / concurrent JIT.** Single-thread, synchronous compilation.
5. **Không PIC / relocatable code.** Code được compile cho địa chỉ cố định trong session.

---

## 8. Kế hoạch thực thi

### Phase 3.1 — JIT Framework
- Tạo `triet-jit` crate với dependency `cranelift` + `cranelift-jit`.
- Implement `lower.rs`: map MIR body → Cranelift function.
- Test: compile + run `abs_diff(10, 3)` và assert kết quả = 7.

### Phase 3.2 — Arithmetic + Control Flow
- Lower tất cả `BinaryOp` variants.
- Lower `If`, `Goto`, `Return`.
- Test: `factorial(5) = 120`.

### Phase 3.3 — Function Calls
- Lower `CallDispatch`: cross-function JIT compilation.
- Thin ABI: parameters qua register, return value qua register.
- Test: gọi hàm từ hàm khác, assert kết quả.

### Phase 3.4 — S6 References + Outcome
- Verify references = raw pointers (zero-cost).
- Lower `Outcome` return ABI (discriminant + payload).
- Test: hàm trả về `T~E`, caller kiểm tra discriminant.

---

## 9. Quyết định kiến trúc (mentor đã duyệt)

### 9.1 — Composite lowering: native structs + shim cho heap types

**Quyết định:** Cranelift native codegen cho scalars và structs thuần túy (không có con trỏ heap). Chỉ gọi shim (`__triet_alloc_*`) cho các type cần heap allocation (String, Vector, HashMap).

- **Struct không có heap pointer:** dùng Cranelift `StackSlot` — allocate trên stack, truy cập qua offset. Không delegate to VM. MIR `Body::struct_layouts` mang thông tin field offsets (từ `triet_mir::StructLayout`) — codegen đọc bảng này để biết kích thước + offset, không cần AST type definition đầy đủ.
- **Struct có heap pointer / String / Vector / HashMap:** gọi shim delegate-to-VM. Pattern từ bản nháp v0.11 — shim gọi VM helper, divergence-free by construction.

Lý do: Triết là OS-capable — trong kernel không có VM. Native struct codegen là bước đầu cho native aggregate codegen (Bậc C). Nhưng heap allocation cần memory allocator — chưa có trong Phase 3. `StructLayout` trong MIR đảm bảo codegen backend có đủ thông tin để tính offset mà không cần type erasure ra Opaque.

### 9.2 — Outcome ABI: 2 registers (rax, rdx)

**Quyết định:** Dùng multi-value return trong Cranelift IR — 2 values map thẳng xuống `rax` (discriminant i8) và `rdx` (payload i64).

Lý do: System V AMD64 ABI trả về struct nhỏ (≤ 16 byte) trong `rax` + `rdx`. Multi-value return của Cranelift map thẳng xuống 2 thanh ghi này. Không tốn memory traffic. Chỉ dùng stack slot nếu payload > 8 byte.

### 9.3 — JIT trước, AOT sau

**Quyết định:** Phase 3 dùng `cranelift-jit` (in-memory compile + execute). AOT (`cranelift-object`) là Phase 3.2.

Lý do: Vòng lặp phát triển nhanh — compile, chạy, thấy kết quả ngay. Debug segmentation fault dễ hơn nhiều so với debug ELF file. Khi test suite JIT pass hết, chuyển sang AOT chỉ là đổi backend — mất dưới 1 giờ.

### 9.4 — Không delegate-to-VM cho struct thuần túy

**Quyết định bổ sung:** Struct không chứa heap pointer (vd: `struct Point { x: Integer, y: Integer }`) được native-codegen qua Cranelift `StackSlot`. Tuyệt đối không gọi VM shim cho những struct này.

Đây là khác biệt với bản nháp v0.11 (mọi aggregate đều delegate-to-VM). Bản làm lại native ngay từ đầu cho struct — đúng nguyên tắc "IR → machine code từ day 0".
