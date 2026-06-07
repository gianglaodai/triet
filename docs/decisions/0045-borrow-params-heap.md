# ADR-0045: Borrow Params Heap — Bậc C lát 2 (shared read-only)

**Status:** ACCEPTED — O + G ký 2026-06-08
**Date:** 2026-06-08
**Author:** AI (đồng nghiệp D, khảo sát + đề xuất)
**Reviewers:** Mentor O (semantics, soundness) — KÝ 2026-06-08 · Mentor G (layout, ABI, codegen) — KÝ 2026-06-08
**Scope:** `&0 T` (shared read-only) cho heap types (String, Vector, HashMap) qua user-fn boundary. Mở read-op tối thiểu qua ref.

---

## Tóm tắt

B7-lift (ADR-0042) cho phép heap types qua function boundary với **Move-only** —
không thể đọc String mà không mất ownership. Bậc C lát 2 cho phép `&0 String` /
`&0 Vector<T>` / `&0 HashMap<K,V>` làm parameter: callee đọc qua shared reference,
caller giữ ownership, tiếp tục dùng sau call. JIT không thay đổi — reference truyền
handle i64 by-value, đồng nhất ABI với owned. Khác biệt thuần ngữ nghĩa, quản lý
bởi borrowck + lower.

---

## §0 — Dữ kiện

| # | Dữ kiện | Vị trí |
|---|---------|--------|
| F1 | B7-lift (ADR-0042) đã cho phép heap types ở param, nhưng chỉ Move. Move semantics: callee sở hữu + drop, caller zero handle sau call + borrowck M3+ đánh dấu Moved. | `lower/lib.rs:468-478`, `checker.rs:828-853` |
| F2 | `type_name` (lower) render reference type `&0 T` thành `"?"` (fallback `_ => "?"`). `"?"` được `is_copy` (cả lower `simple_is_copy` lẫn MIR `is_copy`) phân loại Copy. | `lower/lib.rs:522` (type_name), `lower/lib.rs:549` (simple_is_copy), `mir/lib.rs:2221` (is_copy) |
| F3 | Hệ quả F2: callee VẪN emit `Drop(_0)` cho ref param — nhưng Drop vô hại vì JIT skip free cho Copy (handle không bị free). Borrow "đúng" hiện nay đứng trên tai nạn chuỗi, không trên thiết kế. | MIR dump xác nhận: `fn process(a: &0 String) → Drop(_0)` |
| F4 | `ReturnBorrowMap` + `PropagatedLoan` engine tồn tại trong checker (`checker.rs:754-796`), có unit test bảo vệ (`returned_reference_extends_source_lifetime`). Engine hoạt động trong test nhưng chưa từng chạy trong production — hai mắt xích đứt: (a) `lower/lib.rs:168` luôn `ReturnBorrowMap::new()` rỗng; (b) `driver/main.rs:96` gọi `check_body` (không sigs). | `checker.rs:754-796`, `checker.rs:1372-1419` |
| F5 | `-> &0 T` làm return type hiện được typecheck chấp nhận. `fn id(s: &0 String) -> &0 String { return s }` compile không lỗi, checker không bắt. Tạo use-after-free latent: move owner sau khi padding 1 statement → checker OK; thành lỗi thật ngay khi mở op đọc qua ref. | MIR dump xác nhận: `fn id(...) -> ? { Drop(_0) Return(_0) }` |
| F6 | JIT `CallDispatch` (`mir_lower.rs:944-1004`) truyền args đồng nhất: struct → `stack_addr`, enum → `stack_load`, scalar/ref → `use_var`. Không phân biệt Borrow/Move. Handle heap = i64, reference truyền handle đó by-value. | `mir_lower.rs:974-989` |
| F7 | `checker.rs:828-853` (M3+) đánh dấu TẤT CẢ non-Copy args của `CallTarget::Jit` là Moved. Không phân biệt Borrow vs Move. | `checker.rs:834-852` |

---

## §1 — ABI: handle i64 by-value, không double-pointer

**Quyết định:** Heap reference (`&0 String`, `&0 Vector<T>`, `&0 HashMap<K,V>`) =
handle i64 by-value, ABI đồng nhất với owned. Không có pointer-to-handle.

**Lý do:** Handle heap đã là pointer (i64). Reference tới heap value dùng chính
handle đó — callee đọc qua cùng con trỏ, không cần thêm tầng gián tiếp. F6 xác
nhận JIT không cần thay đổi.

**Khẳng định:** G-Q1 "truyền thẳng handle, dẹp pointer-to-handle" — đồng thuận
tuyệt đối với O.

---

## §2 — Codegen rule: callee không Drop, caller không zero

### Callee

**Phương án A (chọn):** Lower không `push_owned` cho borrow param → MIR không
emit `Drop` cho param đó ngay từ đầu.

Bác phương án "giữ Drop, dựa vào is_copy" — mỏng manh: khi §3 landed (type thật
cho reference), nếu ai đó vô tình đổi `is_copy(&0 String) → false` thì Drop ở
callee free handle đang được caller sở hữu → double-free runtime.

**Triển khai:**
- `lower/lib.rs:472`: `push_owned(l)` → chỉ gọi khi `passing_mode == Move`
- Không `StorageDead` cho borrow param (đã bị `push_owned` quản)

### Caller

**Triển khai:**
- `lower/lib.rs:1399-1405` (sret path): `to_zero` filter → skip arg nếu callee param là Borrow
- `lower/lib.rs:1433-1439` (scalar path): tương tự
- `checker.rs:828-853` (M3+ move-mark): skip arg nếu callee param là Borrow
- **Lower side:** thêm `func_param_modes: HashMap<String, Vec<ParameterPassing>>`
  vào `LowerCtx`, build tại `lib.rs:389` cùng chỗ với `func_return_types`.
  Tra cứu `func_param_modes[callee_name][i]` để quyết định Deinit/Moved.
  Không dùng `FunctionSignature` (MIR type) — chưa có MIR ở thời điểm build
  registry, và return-borrow đã CẮT (§5) nên không cần `return_borrow_map`.
- **Borrowck side:** không cần registry mới. `check_body_with(body, callee_sigs)`
  đã có tham số — bước 4 wire driver gọi `check_body_with` với sigs từ
  `lower_program` (mỗi Body đã mang `.signature` đầy đủ).

---

## §3 — Type thật cho reference (MÓNG)

**Quyết định:** `type_name` phải render reference type thật (`&0 String`,
`&0 Vector<Integer>`, etc.) thay vì fallback `"?"`. `is_copy(reference_type)` =
`true` **bằng thiết kế** — reference là Copy: sao chép handle reference hợp lệ,
không double-free vì callee không Drop (§2).

**Xóa lệ thuộc tai nạn `"?"` (F2-F3).** Đây là điều kiện tiên quyết — mọi bước
khác phụ thuộc vào type thật có mặt trong MIR.

**Phân biệt quan trọng:** `TypeExpr::Reference { form, inner }` là AST type
expression (`triet-syntax/src/type_ast.rs:121`, struct variant — **không** phải
tuple). `Type::Reference` trong schema (`generated/types.rs:204`) là spec-only,
chưa được wire. `is_copy`/`simple_is_copy` match trên **chuỗi** (string prefix
do `type_name` render ra: `"&0 String"`), không match trên enum variant.

**TECH-DEBT (G-mandate, KÝ 2026-06-08):** Dùng `s.starts_with("&0 ")` làm
type-tag ở tầng MIR là **acceptable evil** — MIR hiện lưu type dưới dạng
`String`, đã có tiền lệ `is_vec_type`/`is_hashmap_type` dùng `starts_with`. Chấp
nhận để không phá schema MIR giữa Bậc C. NỢ: một ngày phải chuyển MIR Type từ
`String` sang AST Node / enum tường minh. Mọi prefix-match thêm vào trong lát
này PHẢI mang comment `// TECH-DEBT(ADR-0045): MIR-type-as-string, xem §3`.

**Triển khai:**
- `lower/lib.rs:type_name` (522): thêm nhánh `TypeExpr::Reference { form, inner }` → format `"&0 {inner}"` / `"&+ {inner}"` / `"&- {inner}"` dựa trên `form`
- `mir/lib.rs:is_copy` (2221): thêm prefix match `s if s.starts_with("&0 ") || s.starts_with("&+ ") || s.starts_with("&- ")` → `true`
- `lower/lib.rs:simple_is_copy` (549): thêm prefix match tương tự → `true`

---

## §4 — PropagatedLoan engine: GIỮ + TODO, không xóa

**Quyết định:** GIỮ engine PropagatedLoan (`checker.rs:754-796`) + GIỮ unit test
`returned_reference_extends_source_lifetime` + cắm TODO trỏ lát return-borrow
tương lai.

**Lý do bác "xóa":** Unit test đã đỏ-khi-gỡ (teeth-verified bởi O) — engine
logic đúng, được test bảo vệ. Xóa = mất tài sản đã chứng minh.

**TODO:**
- `checker.rs:754`: `// TODO(lát return-borrow): wire return_borrow_map from lowerer into driver → check_body_with`
- `lower/lib.rs:168`: `// TODO(lát return-borrow): populate return_borrow_map when callee returns &0 T`

**Ghi chú cho G:** Engine DEAD-in-production (hai mắt xích đứt: lower không
populate + driver không truyền sigs — G chỉ thấy mắt xích thứ nhất), nhưng
LIVE-in-test. Không xóa.

---

## §5 — Return-of-borrow: CẮT (typecheck refuse `-> &0 T`)

**Quyết định:** Typecheck refuse `-> &0 T` (và `-> &+ T`, `-> &- T`) trong lát
này. Mã lỗi **E1042 BorrowReturnNotYetSupported**.

**Lý do:** Lỗ hổng accepted-wrong (F5): `fn id(s: &0 String) -> &0 String {
return s }` compile không lỗi, borrowck không bắt. Hiện vô hại vì chưa có op đọc
qua ref; thành use-after-free thật ngay khi mở op (§8). Chặn cửa trước khi mở op.

**Mở lại:** Lát return-borrow tương lai (sau khi PropagatedLoan được wire trong
production — §4).

**Ghi chú mã lỗi:** E1041 đã có xung đột kép: (a) code tiền-tồn `error.rs:46`
`NoMatchingOverload` — đang emit, có test `lib.rs:1410`; (b) ADR-0039 reserve
`NullableHasNoErrorState` cho `?->`. E1042 tránh cả hai hoàn toàn. Tên
`BorrowReturnNotYetSupported` phản ánh "chưa hỗ trợ" (sẽ mở lại) — không phải
cấm vĩnh viễn.

---

## §6 — Mutability: shared read-only only

**Quyết định:** Lát này chỉ `&0 T` (shared read-only). `&0 mutable T` (exclusive
mutable borrow) và `&+ T` (strong owning reference) defer lát sau.

**Lý do:** `&0 mutable` đòi hỏi exclusivity guarantee (E2440 cho mượn overlapping)
— lớn hơn phạm vi lát này. `&+` liên quan đến refcount / ObjectHeader — defer.

---

## §7 — Call surface: explicit `&0` tại call site

**Quyết định:** Caller phải viết `callee(&0 s)` — explicit borrow tại call site.
Parser đã hỗ trợ syntax này (`Expr::Borrow`). Không auto-borrow.

**Lý do:** Khớp explicit-strictness convention của Triết. Auto-borrow che giấu
ownership transfer, làm mờ semantic boundary giữa Move và Borrow.

---

## §8 — Ops cho ref: mở read-op tối thiểu

**Quyết định:** Mở tối thiểu vài read-op qua `&0 T` — nếu không, borrow param
chỉ là trang trí (compile được nhưng không làm gì được với nó). Phạm vi chốt
cứng trong ADR này; mở rộng = incremental (thêm fixture, không đổi ABI).

**Shim ABI không đổi:** Cùng handle i64. Chỉ typecheck chấp nhận ref type ở vị
trí param của builtin shim.

**Phạm vi chốt cứng:**
- `length(s: &0 String) -> Integer` / `length(v: &0 Vector<T>) -> Integer`
  — luôn an toàn: trả về Integer (Copy), không lộ handle heap.
- `get(s: &0 String, index: Integer) -> Integer` — đọc char code: an toàn (Integer Copy).
- `get(v: &0 Vector<T>, index: Integer) -> T` — **chỉ khi T là Copy**
  (Vector<Integer>.get OK; Vector<String>.get defer).
- HashMap: tương tự Vector — value Copy mới mở.

**Ràng buộc soundness cho `get` với element non-Copy:** Pattern Δ3
(cannot_copy_move_type_out_of_field, E2423). `get(v: &0 Vector<String>) -> String`
sẽ copy handle String ra khỏi borrowed Vector → callee + caller cùng giữ một
handle → double-free. Giải pháp đúng cho non-Copy element là return-borrow
(`-> &0 T`, đã CẮT tại §5) hoặc clone (chưa có). Defer đến lát return-borrow.

**Mở rộng sau:** `contains`/`is_empty` cho HashMap, iterator, slice window —
incremental, thêm fixture không đổi ABI.

---

## Kế hoạch triển khai (sau hai chữ ký)

Theo thứ tự ép buộc (§3 là móng, phải làm trước):

| # | Việc | File chính | Teeth (phải đỏ khi gỡ) |
|---|------|-----------|------------------------|
| 1 | Type thật cho reference (§3) | `lower/lib.rs:type_name` (522), `mir/lib.rs:is_copy` (2221), `lower/lib.rs:simple_is_copy` (549) | Fixture: ref param → MIR mang type `&0 String`, không `"?"` |
| 2 | Lower không push_owned cho borrow param (§2 callee) | `lower/lib.rs:466-478` | Fixture: callee MIR không có `Drop(_0)` cho ref param; gỡ guard → Drop tái xuất |
| 3 | Caller không zero borrow arg (§2 caller) | `lower/lib.rs:1399,1433` + thêm `func_param_modes` | Fixture: caller dùng owner sau `peek(&0 s)` → chạy ra đúng, không E2420 |
| 4 | Driver collect sigs + `check_body_with` (mắt xích b của F4) | `driver/main.rs:96` | Unit test `returned_reference_extends_source_lifetime` vẫn xanh |
| 5 | Typecheck refuse `-> &0 T` (§5) | typecheck | Fixture: `-> &0 String` → E1042; đóng accepted-wrong |
| 6 | Mở read-op qua `&0` (§8) — `length` + `get` (Copy-only) | typecheck + lower | RUN fixture: `length(&0 s)` → ra số đúng; `get(&0 v, 0)` cho Vector<Integer> → đúng |
| 7 | TODO + giữ engine (§4) | `checker.rs:754`, `lower/lib.rs:168` | Unit test `returned_reference_extends_source_lifetime` vẫn đỏ-khi-gỡ |

Gate `scripts/gate.sh` raw sau mỗi bước. Mỗi fixture mới vào corpus.

---

## Q&A

### G-Q1: ABI cho borrow param heap?

Handle i64 by-value. Không double-pointer. (§1)

### G-Q2: Callee làm sao biết không Drop ref param?

Không phải cơ chế runtime — lower không emit Drop ngay từ đầu. (§2, Phương án A)

### G-Q3: Engine PropagatedLoan — giữ hay xóa?

GIỮ + TODO. Engine đúng, test đã đỏ-khi-gỡ. (§4)

### O-Q1: Explicit borrow tại call site?

Có. `callee(&0 s)`. (§7)

### O-Q3: `-> &0 T` return type?

CẮT. Typecheck refuse E1042 trong lát này. (§5)

### O-Q4: `&+ T` / `&0 mutable T`?

Defer. Lát này chỉ `&0 T` shared read-only. (§6)

### O-Q5: Op nào mở cho ref?

`length` + `get` (Copy-only). Phạm vi chốt cứng trong §8. Mở rộng incremental,
không đổi ABI.
