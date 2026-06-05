# ADR-0040: Heap Aggregate Value Model & Layout — Bậc A

**Status:** Draft v4 (vòng 4 review)
**Date:** 2026-06-05
**Author:** Giang Hoàng (value model, semantics), AI (layout, MIR shims,
verification)
**Reviewers:** Mentor G (layout, ABI, runtime codegen), Mentor O (semantics,
soundness)
**Changes v3→v4:** §1.3 thêm M4 (Return-escape). §3.2 thêm cơ chế
Return-escape. §3.5 sửa ví dụ loop leak. §3.7 sửa cite (754-757).
§4 thêm B7 refusal (heap qua user-fn boundary). §5 thêm B7 steps.
§7 sửa fixture 35/36 (return len).

---

## Tóm tắt

Quyết định value model và memory layout cho heap aggregates (String, Vector,
HashMap) ở Bậc A. Chốt move-only owned làm semantic mặc định cho slice 1
(String, Vector), với ObjectHeader reserved trong layout nhưng refcount chưa
bật. Cơ chế an toàn runtime: **Zeroing-on-Move** (JIT ghi null vào source tại
**mọi move-site**) + **Null-guard-free** (Drop check `ptr != 0` trước khi gọi
free shim) + **Return-escape** (JIT bỏ qua Drop cho locals nằm trong Return
values). Bốn loại move-site: `Assign` (M1), let-Move-type→Assign (M2),
CallDispatch consume-arg (M3), Return-values escape (M4). Tất cả operation
qua extern "C" shim theo precedent `__triet_pow`.

## Động lực

1. **Copy/Move type-aware borrowck đã đóng** (HEAD `6e2843c`) — borrowck giờ
   phân biệt Copy vs Move type, enforce được single-owner move semantic. Đây
   là hạ tầng an toàn cho heap types có destructor thật.
2. **F1 gap đã đóng** — sticky Moved qua Drop → Return bắt E2420. Heap local
   bị move vào payload rồi dùng lại sẽ bị phát hiện.
3. **String/Vector/HashMap hiện là lỗ hổng** — lowerer trả `Err` cho mọi
   aggregate type. Cần chốt layout trước khi implement lowering.

---

## §1 — Value Model (Author quyết định, hai mentor input)

### 1.1 — Move-only owned cho Bậc A

**Quyết định:** String và Vector là **move-only owned** — single-owner,
không implicit copy, không implicit clone.

Ba dữ kiện đã verify từ code (không phải phỏng đoán):

| Dữ kiện | Vị trí | Ý nghĩa |
|---------|--------|---------|
| Copy/Move borrowck enforce single-owner | `triet-borrowck/src/checker.rs:586-589` (Δ1), `683-688` (Δ2) | Move type bị mark Moved khi assign, không dùng lại được → an toàn single-owner |
| `ObjectHeader` LIVE, chưa có consumer | `triet-core/src/memory.rs:51-58` | `refcount: AtomicU32` + `reserved: AtomicU32`, `repr(C, align(8))` — đã định nghĩa, có test |
| `&+` strong forms chưa lowered | `triet-jit/src/mir_lower.rs` — không có code path cho borrow lower | Không ai inc/dec refcount hôm nay |

**Hệ quả:** Refcount là dead code nếu bật ngay — không có producer (strong
form lower) và không có consumer (Drop::decrement). Move-only tận dụng được
**toàn bộ** hạ tầng borrowck vừa xây mà không thêm máy móc.

### 1.2 — ObjectHeader reserved, refcount = 1 (không inc/dec)

Layout heap object dùng ObjectHeader đầy đủ, nhưng ở Bậc A:

- `refcount` được khởi tạo = 1 (tương thích `ObjectHeader::new()`)
- **Không increment** (chưa có `&+ T` lower)
- **Không decrement** (Drop gọi thẳng `free`, không qua refcount→0 check)
- `reserved` = 0 (dành cho drop flags / type tag ở Bậc B/C)

**Migration path:** Khi `&+ T` lowering land (Bậc B/C), cùng một layout,
chỉ cần:
1. Lower `&+ T` → call `ObjectHeader::increment()`
2. Drop với refcount > 1 → decrement; refcount = 1 → free thật
3. **Layout không đổi** — binary tương thích ngược.

### 1.3 — Drop semantics: Zeroing-on-Move + Null-guard-free + Return-escape

**Vấn đề:** Borrowck là phân tích tĩnh — nó không sửa MIR. Lowerer sinh
`Statement::Drop` cho **tất cả** owned local ở cuối scope, bất kể local
đó đã bị move trên một số control-flow path hay chưa. Drop-on-Moved được
phép by-design (F1 dạy: Return chấp nhận Ended, không reject Moved).

Với Copy type, Drop = no-op (stack primitive). Nhưng với Move type (heap),
nếu JIT emit `free(ptr)` vô điều kiện cho mọi Drop — thì double-free và
dangling pointer xuất hiện ở những ranh giới ownership mà null guard không
đủ.

Claim "sticky-Moved đảm bảo Drop không chạy" trong v1 là **sai** — sticky-
Moved chỉ ảnh hưởng Return check (E2420) và transition VarState, không hề
xóa `Statement::Drop` khỏi MIR.

**Quyết định — Bốn loại move-site, JIT zero hoặc skip tại mỗi loại:**

Có **bốn** ranh giới ownership ở runtime, JIT phải xử lý tất cả:

| # | Move-site | Cơ chế | Đặc tả |
|---|-----------|--------|--------|
| M1 | `Statement::Assign` plain-source Move-type | Sau khi copy giá trị sang dest, store 0 vào source variable | Dưới |
| M2 | `let b = a` với a: Move type | Lowerer emit Assign thay vì alias local (§3.7); JIT zero như M1 | §3.7 |
| M3 | `CallDispatch` arg ở vị trí consume | JIT zero variable sau call, dùng chung bảng BuiltinShimMeta (§3.6) | §3.6 |
| M4 | `Return(values)` — giá trị escape khỏi hàm | JIT bỏ qua Drop cho local ∈ values (§3.2) | §3.2 |

**M1 — Assign (đã có trong hạ tầng):**

1. JIT codegen cho `Statement::Assign` với Move-type source:
   - Copy giá trị i64 từ source sang dest (bình thường)
   - **Store 0 vào source variable** (null pointer)
   - JIT biết type từ `body.local_decls[source.local.0].ty` → `triet_mir::is_copy`

**JIT codegen cho `Statement::Drop` với Move-type local:**

- Nếu local nằm trong Return values của block hiện tại → **skip** (M4, §3.2)
- Ngược lại: gọi `call __triet_<type>_free(ptr)` — shim tự guard null.
  Ở Bậc A, null-guard nằm trong shim (`if ptr == 0 { return; }`), không
  nằm trong JIT codegen. JIT-side null-check branch là tối ưu Bậc B
  (tránh call overhead cho null pointer).

**`__triet_<type>_free` shim nhận `ptr: i64`:**

- `if ptr == 0 { return; }` — shim-level null guard (Bậc A)
- Tính `header_ptr = ptr - 8`, free toàn bộ allocation

**Tại sao không dùng reserved field làm drop flag:** reserved field nằm
trên heap, cần load từ bộ nhớ để check. Null-on-move dùng chính giá trị
stack (đã có sẵn trong register/Cranelift Variable) — rẻ hơn 1 memory
load, và không cần đụng tới ObjectHeader cho mục đích drop-tracking.
Reserved field giữ nguyên cho Bậc B/C.

**Borrowck KHÔNG thay đổi** — sticky-Moved + E2420 + E2450 giữ nguyên.
Zeroing-on-Move + Return-escape là **runtime mechanism** bổ sung cho
static analysis, không thay thế.

### 1.4 — Tại sao không refcount ngay

| | Refcount ngay (Bậc A) | Move-only (Bậc A) |
|---|---|---|
| Số shim cần viết | `alloc`, `increment`, `decrement`, `free` | `alloc`, `free` |
| Producer của increment | Không có (`&+` chưa lower) | Không cần |
| Consumer của decrement | Drop với refcount check | Drop = free thẳng (có null guard) |
| Số dòng dead code | ~100 (increment/decrement path) | 0 |
| Rủi ro soundness | Refcount sai → leak hoặc use-after-free âm thầm | Move-only + cơ chế M1-M4 → borrowck bắt static, runtime bắt động |

**Kết luận:** Refcount là thứ sẽ cần, nhưng chưa phải hôm nay. Move-only
là cái máy borrowck bảo vệ được ngay, cơ chế M1-M4 bảo vệ runtime.

---

## §2 — Memory Layout (Implementer — đất của G)

### 2.1 — Object header

Mọi heap allocation dùng `ObjectHeader` từ `triet-core/src/memory.rs:51`:

```text
Address:  HEADER_ADDR              BODY_ADDR = HEADER_ADDR + 8
          |                        |
          v                        v
          [ refcount: u32 | reserved: u32 ] [ user data ... ]
          |<--- 8 bytes (64-bit) ------->|
```

- `refcount`@offset 0: `AtomicU32`, init = 1
- `reserved`@offset 4: `AtomicU32`, init = 0
- `repr(C, align(8))` — tương thích Cranelift `i64` alignment
- Body pointer = `header_ptr + 8` — pattern giống Objective-C/Swift

### 2.2 — `String` layout

```text
Stack (i64)                         Heap
┌──────────────────┐                ┌──────────────────────────────────────┐
│ body_ptr: i64    │──────────────> │ ObjectHeader (8 bytes)               │
└──────────────────┘                │  refcount: u32 = 1 (reserved)        │
                                    │  reserved: u32 = 0                   │
                                    ├──────────────────────────────────────┤
                                    │ len: i64 (số byte đang dùng)         │
                                    │ cap: i64 (số byte đã allocate)       │
                                    ├──────────────────────────────────────┤
                                    │ data: [u8; cap] (UTF-8 bytes)        │
                                    └──────────────────────────────────────┘
```

### 2.3 — `Vector<T>` layout

```text
Stack (i64)                         Heap
┌──────────────────┐                ┌──────────────────────────────────────┐
│ body_ptr: i64    │──────────────> │ ObjectHeader (8 bytes)               │
└──────────────────┘                │  refcount: u32 = 1 (reserved)        │
                                    │  reserved: u32 = 0                   │
                                    ├──────────────────────────────────────┤
                                    │ len: i64 (số phần tử)                │
                                    │ cap: i64 (số phần tử đã allocate)    │
                                    ├──────────────────────────────────────┤
                                    │ data: [T; cap] (contiguous elements) │
                                    └──────────────────────────────────────┘
```

- Bậc A: `T = i64` cho mọi element (generic chưa lowered)

### 2.4 — `HashMap<K, V>` layout

**Defer Bậc B.**

### 2.5 — Fat pointer representation

Trên stack, String/Vector = **1 giá trị i64** (con trỏ tới body).
Không dùng fat pointer 3×i64 — giữ ABI Bậc A nhất quán.

Khi cần len/cap, JIT load từ heap: `len = load(ptr+0)`, `cap = load(ptr+8)`.

---

## §3 — MIR + Runtime Shims (Architecture)

### 3.1 — Shim signatures + Ownership contracts

Pattern theo precedent `__triet_pow` (`triet-jit/src/mir_lower.rs:1178-1207`,
`triet-driver/src/main.rs:123`):

| Shim | Signature | Per-arg ownership | Mô tả |
|------|-----------|-------------------|-------|
| `__triet_string_alloc` | `fn(len: i64, cap: i64) -> i64` | copy, copy → new | Cấp phát String, trả về body_ptr |
| `__triet_string_from_bytes` | `fn(ptr: i64, len: i64) -> i64` | borrow, copy → new | Copy bytes từ read-only memory sang heap mới |
| `__triet_string_free` | `fn(ptr: i64)` | **consume** → void | Giải phóng String. No-op nếu ptr=0. |
| `__triet_string_concat` | `fn(a: i64, b: i64) -> i64` | borrow, borrow → new | Ghép 2 String, trả về ptr mới. a và b **không** bị free. |
| `__triet_string_eq` | `fn(a: i64, b: i64) -> i64` | borrow, borrow → scalar | So sánh bằng nhau: trả về 1 (true) hoặc 0 (false). |
| `__triet_string_len` | `fn(ptr: i64) -> i64` | borrow → scalar | Trả về `len` của String |
| `__triet_vector_alloc` | `fn(len: i64, cap: i64) -> i64` | copy, copy → new | Cấp phát Vector, trả về body_ptr |
| `__triet_vector_free` | `fn(ptr: i64)` | **consume** → void | Giải phóng Vector. No-op nếu ptr=0. |
| `__triet_vector_push` | `fn(vec: i64, elem: i64) -> i64` | **consume**, copy → new | Thêm phần tử (có thể realloc). **Shim tự free vec cũ nếu realloc.** Trả về ptr mới. |
| `__triet_vector_len` | `fn(ptr: i64) -> i64` | borrow → scalar | Trả về `len` của Vector |

**Quy ước:** consume = caller mất ownership (JIT zero sau call, Drop no-op);
borrow = caller giữ ownership; copy = i64; new = shim cấp phát.

### 3.2 — Drop codegen: Null-guard-free + Return-escape

JIT codegen cho `Statement::Drop(local)` trong block `bb`:

```
if is_copy(type_of(local)) {
    // no-op
} else if local ∈ terminator_return_values(body.blocks[bb].terminator) {
    // M4: Return-escape — giá trị đang được trả về cho caller.
    // Bỏ qua free: ownership chuyển sang caller, caller sẽ Drop.
    // Cả hai cơ chế E2450 giữ nguyên vì Drop vẫn trong MIR:
    //   1. Drop-before-Return (lowerer Gate B) — borrowck thấy Drop
    //      đang có active loan → E2450.
    //   2. Return-terminator check (checker.rs:720) — borrowck thấy
    //      Return của local có active loan → E2450.
    no-op
} else {
    // Move type, not escaping: call free shim.
    // Bậc A: null-guard in shim (if ptr == 0 → return).
    // Bậc B: move null-check branch to JIT (avoid call overhead on null).
    call ___triet_<type>_free(ptr);
}
```

**Ví dụ:**
```
function f() -> String = {
    let s = "hi";     // s = ptr_to_heap
    return s;         // lowerer: flush_all_for_return → Drop(s); Return(s)
                      // JIT: Drop(s) → s ∈ Return values → skip (M4)
                      //      Return(s) → caller nhận ptr_to_heap ✓
                      // Caller: Drop(s_caller) → free ✓
}
```

### 3.3 — String literal: `__triet_string_from_bytes`

**Quyết định:** JIT in-process mechanism. String literal là UTF-8 bytes
trong `ConstValue::String` của MIR `Body`. JIT emit `iconst(&bytes, len)`
→ call `__triet_string_from_bytes(ptr, len)` → body_ptr i64.

**⚠️ Lifetime obligation:** Bytes trong `ConstValue::String` nằm trong
`Body`. Nếu `Body` bị drop trước khi JIT code thực thi → dangling pointer.
**Nghĩa vụ:** `JitContext::compile_multi` phải đảm bảo mọi `Body` sống ≥
đời JIT module. Driver giữ `Vec<Body>` alive trong suốt thời gian JIT
compiled code tồn tại.

**Hạn chế:** Chỉ hoạt động trong JIT. Khi AOT → `define_data`. Ghi chú
trong code JIT: `// AOT: replace with define_data`.

### 3.4 — MIR mới (tối thiểu)

Không cần thêm MIR statement. Operation qua `CallDispatch` tới builtin shim.

### 3.5 — Temp heap values: documented leak Bậc A

`push_owned` chỉ track let-binding + param (Gate B). Biểu thức trung gian
không được push → temp không bao giờ Drop → leak:

```
call(__triet_string_concat(s1, s2))  // temp từ concat → leak

while eq(concat(a,b), c) {           // mỗi iteration: concat tạo temp
    ...                              // không let → không Drop → leak
}
```

(Lưu ý: `let result = concat(a,b)` trong loop thì let-binding → push_owned
→ pop_scope cuối iteration → Drop → null-guard-free → **không** leak. Đây
là hệ quả trực tiếp của cơ chế ở §1.3.)

Ngoài temp không-let, còn một nguồn leak khác ở Bậc A:
```
s = concat(a, b);  // M1 zero temp concat đúng, nhưng giá trị cũ của s
                   // (heap ptr bị ghi đè) không ai free → leak
```
Mutable rebind của Move-type local: Assign ghi đè dest với giá trị mới
nhưng không free giá trị cũ — JIT không biết dest đang số hữu heap.
Cùng họ temp-leak, accept Bậc A.

**Quyết định:** Chấp nhận leak cho temp không-let ở Bậc A. JIT chạy
`main()` một lần, exit → OS thu hồi. Ghi nhận có chủ đích.

**Fix Bậc B:** Lowerer push_owned temp, emit Drop sau expression.

### 3.6 — BuiltinShimMeta: bảng metadata ở triet-mir, hai consumer

**Vấn đề:** `CallDispatch` args hiện là reads (`checker.rs:806`), không
ai mark Moved, không ai zero. Với shim consume arg:

1. **Borrowck không biết** → use-after-free (tĩnh)
2. **JIT không zero** → Drop gọi free(old_ptr) → double-free (động)

**Thiết kế:** `BuiltinShimMeta` ở `triet-mir` — một nguồn sự thật, hai
consumer:

```rust
// triet-mir/src/lib.rs
pub struct BuiltinShimMeta {
    pub name: &'static str,
    /// Per-arg: true = consume (caller loses ownership)
    pub arg_consumes: &'static [bool],
}
```

**Consumer 1 — Borrowck:** CallDispatch tới builtin name → với arg ở vị
trí consume, nếu Move type → mark Moved (ngăn use-after-move tĩnh).

**Consumer 2 — JIT:** Sau `call` instruction → với arg ở vị trí consume,
emit `store 0` vào Cranelift Variable (M3). Drop sau đó → null guard → no-op.

Bảng khởi tạo: như §3.1 — `push` consume vec, `free` consume ptr, còn lại borrow/copy.

### 3.7 — Lowerer: let-binding Move-type → Assign (M2)

**Vấn đề:** Lowerer (`triet-lower/src/lib.rs:535-538`) xử lý
`let b = a` bằng alias: `lower_expr(init)` trả Local hiện có
(`Expr::Identifier` arm tại `754-757`), `vars.insert(name, local)` —
b và a là **cùng một Local**. Với Copy type, tối ưu hợp lệ. Với Move
type: không Assign → không M1 → Zeroing-on-Move không kích hoạt qua
let → `let b = a; use(a)` compile sạch → semantic §1.1 thành trang trí.

**Quyết định — M2:** Lowerer phân biệt:

```
Nếu init là Expr::Identifier { name } và type của name là Move (is_copy = false):
    1. alloc_local_ty(type_name)  → Local mới
    2. emit Statement::Assign(dest=new, source=old)  → JIT zero old (M1)
    3. vars.insert(name, new)
    4. push_owned(new)
Ngược lại (Copy type hoặc init không phải Identifier):
    giữ nguyên behavior hiện tại (alias)
```

---

## §4 — Scope-Out (cố ý defer)

| Mục | Lý do | Defer đến |
|-----|-------|-----------|
| **HashMap** | Hash function + bucket table + collision | Bậc B |
| **Refcount thật** (increment/decrement) | Chưa có `&+` lower | Bậc B/C |
| **Implicit clone** | Vi phạm explicit-strictness | Không bao giờ |
| **Drop flags** (reserved field) | Zeroing-on-Move + Return-escape đủ cho Bậc A | Bậc B/C |
| **Generic Vector\<T\>** | Monomorphization chưa có | Bậc B |
| **Outcome return từ shim** | C ABI trả i64; Outcome cần 2 giá trị | Bậc B |
| **AOT string literal** (define_data) | JIT in-process đủ | Bậc B/C |
| **Temp heap leak** (không-let) | Gate B chỉ track let-binding + param | Bậc B |
| **Heap qua user-function boundary** | **B7:** User-fn với Move-type param hoặc CallDispatch tới user-fn với Move-type arg → `Err(LowerError)`. Không có ownership calling-convention để biết callee consume hay borrow — từ chối sạch thay vì đoán. Dùng shim cho mọi heap operation ở Bậc A. | Bậc B (cần calling-convention + metadata cho user-fn) |
| **Aggregate chứa heap payload/field** | **B8:** Enum-constructor / struct-literal với payload/field type Move → `Err(LowerError)`. Drop-glue cho aggregate-chứa-heap chưa có — enum/struct local sống trong StackSlot, không phải i64 Variable, cần máy móc riêng (đọc discriminant, load ptr từ slot, gọi free). Heap value chỉ sống ở bare local trong Bậc A slice 1. | Bậc B/C (4.3c: drop-glue cho aggregate) |

---

## §5 — Implementation Sequence (String trước, Vector sau)

### Phase 4.3a — String Bậc A

1. `triet-mir`: thêm `BuiltinShimMeta` struct + bảng `BUILTIN_SHIM_META`
2. `triet-lower`: `Stmt::Let` với Move-type Identifier init → emit Assign + local mới (M2, §3.7)
3. `triet-lower`: string literal → `ConstValue::String` + `alloc_local_ty("String")`
4. `triet-lower`: từ chối user-fn với param type Move → `Err` (B7, §4)
5. `triet-lower`: từ chối CallDispatch tới user-fn (không-shim) có Move-type arg → `Err` (B7)
6. `triet-jit`: implement String shims (§3.1)
7. `triet-jit`: codegen `Assign` Move-type source → Zeroing-on-Move (M1)
8. `triet-jit`: codegen `Drop` Move-type → null-guard-free + Return-escape check (M4, §3.2)
9. `triet-jit`: codegen `ConstValue::String` → `__triet_string_from_bytes` (kèm lifetime invariant §3.3)
10. `triet-jit`: codegen `CallDispatch` shim → zero consume-arg vars sau call (M3, dùng `BuiltinShimMeta`)
11. `triet-borrowck`: `CallDispatch` check `BuiltinShimMeta` → mark Moved consume args (M3)
12. `triet-driver`: đăng ký String shims + giữ `Vec<Body>` alive (§3.3)

### Phase 4.3b — Vector Bậc A

Tương tự String, thêm Vector entries vào `BUILTIN_SHIM_META`. M2, M3, M4
tự động hoạt động cho Vector qua cùng cơ chế.

### Obligation

- **Không `alloc_local_ty("?")` cho heap value** — temp phải có type thật.
- **Let-Move-type phải emit Assign (M2)** — nếu không, toàn bộ null guard vô hiệu qua let.
- **Mọi heap operation qua shim** — user-defined function với heap type bị từ chối (B7).

---

## §6 — ADR Dependencies

| ADR | Mối quan hệ |
|-----|-------------|
| **ADR-0022** (S6 Ownership) | Move semantic + 5 reference forms |
| **ADR-0025** (Borrow Checker Rules) | E2420/E2450/Drop order |
| **ADR-0026** (Actor/Send Rules) | ObjectHeader refcount + Send derivation |
| **ADR-0037** (Enum Layout) | Enum payload với heap type → Partial-Moved defer |
| **COPY/MOVE BORROWCK** (`6e2843c`) | is_copy + sticky Moved + E2423 — hạ tầng tĩnh |
| **ADR-0038** (Comparable trait defer) | `__triet_string_eq` trả 1/0 (equality). 3-way compare cần Trait → defer. |
| **ADR-0039** (?-family) | Nullable String: **chưa thiết kế representation.** Lưu ý xung đột sentinel-0 (moved-out ≡ null value). |

---

## §7 — Verification (Test Plan)

### 7.1 — Test axes

| Trục | Values cần quét | Fixture |
|------|-----------------|---------|
| String độ dài | empty (""), 1 char, multi-byte UTF-8, dài > cap mặc định | `33_string_empty`, `34_string_utf8` |
| Concat chain | 2 strings, 3 strings, concat rồi return len | `35_string_concat` |
| Move chain (M2) | `let a = "x"; let b = a; let c = b;` → return len(c) | `36_string_move_chain` |
| **F1 end-to-end (negative)** | Hand-built MIR: Move-type local → Assign vào enum Payload → Return local gốc → expect E2420. Test linh hồn F1 (M1 + sticky-Moved + Return check) với Move type, không cần enum runtime. | borrowck unit test `f1_enum_payload_move_type` |
| **E2420 use-after-move** | `let a = "x"; let b = a; use(a)` → compile error | borrowck unit test |
| **E2423 field-copy** | struct với String field, project → compile error | borrowck unit test (đã có) |
| **E2450 heap** | `&0 s` rồi Drop s → E2450 | borrowck unit test |
| **Use-after-push (M3)** | `let v2 = push(v, x); use(v)` → E2420 | borrowck unit test |
| Vector push chain | push 3 lần (realloc), push rồi read len | `38_vector_push` |
| Vector move (M2+M3) | `let v2 = v1; push(v2, x)` — v1 bị Moved | borrowck unit test |
| **B7 refusal** | user-fn với String param → `Err` từ lowerer | lowerer unit test |

### 7.2 — Invariants

| # | Invariant | Cách verify |
|---|-----------|-------------|
| 4i-1 | M1: sau Assign Move-type, source = 0 | JIT unit test |
| 4i-2 | Null-guard-free: Drop(local=0) không gọi free | JIT unit test |
| 4i-3 | String free giải phóng đúng size | Rust unit test (valgrind) |
| 4i-4 | M3: push consume arg không double-free | Instrumented allocator: alloc/free balance |
| 4i-5 | Borrowck từ chối use-after-push (E2420) | Unit test: BuiltinShimMeta + CallDispatch |
| 4i-6 | `__triet_string_eq` trả 1/0 | Rust unit test |
| 4i-7 | M2: `let b = a` với String → hai Local khác, Assign trong MIR | Lowerer unit test |
| 4i-8 | M4: Return-escape — Drop của local trong Return values không free | JIT unit test: body Return(String), check heap còn sống |
| 4i-9 | B7: user-fn String param → `Err(LowerError)` | Lowerer unit test |
