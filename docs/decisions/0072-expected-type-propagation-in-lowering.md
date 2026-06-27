# ADR 0072 — Expected-Type Propagation in AST→MIR Lowering

**Trạng thái:** **✅ APPROVED** (Mentor O soạn + Mentor G ký 2026-06-27; chờ implement 3-slice → O verify máu byte-identical → SEALED). Áp dụng cho rewrite-era (Bậc C, `triet-lower`). Đóng **TODO Gap #2** (expected-type propagation cho `~0`/Outcome-constructor lồng trong block-final/if-arm/match-arm) + bug producer-side **hàm trả `T?`** phát hiện phiên 2026-06-27.

**Quyết định G (LOCKED 2026-06-27):** dùng **param tường minh `expected: Option<&MirType>`** truyền thẳng qua signature `lower_expr` — KHÔNG dùng context ẩn (`c.expected_stack`). Lý do G: *"Cái gì ẩn thì sớm muộn cũng sinh bug thối — `c.sig.return_type` là ví dụ hoàn hảo. Refactor một lần cho đàng hoàng, compiler tự-mô-tả."* Lộ trình 3-slice; điều kiện tiên quyết: **gate byte-identical ở Slice 1.**

**Issue:** Cú pháp `~+ v` / `~- e` / `~0` (`Expr::OutcomeConstructor`) và `~0` bare (`Expr::NullLiteral`) **không tự mang đủ thông tin** để biết phải hạ thành đại diện nào: 16-byte Outcome StackSlot (`OutcomeAlloc`), hay nullable repr (PA-3c — payload-plain / NULL_SENTINEL). Quyết định đó phụ thuộc **kiểu kỳ vọng tại vị trí** (`expected type` / context). Lowerer hiện **KHÔNG truyền** kiểu kỳ vọng; nó dùng `c.sig.return_type` (kiểu trả về của HÀM) làm proxy toàn cục, rồi vá thủ công các vị trí cục bộ khác bằng "redirect" đặc thù. Hệ quả: một lớp bug `OutcomeAlloc on non-Outcome type` + nợ chắp-vá-per-site.

---

## 1. Bối cảnh — bằng chứng máu (recon phiên 2026-06-27)

### 1.1 Case study: một chẩn đoán sai sống 1 phiên vì thiếu expected-type

Sổ bàn giao ghi blocker `match-arm bind heap payload move-out`: `match get(){~+ s => s}` → `lowerer does not support Identifier`. **Recon chứng minh đây là HAI lỗi chồng nhau, cả hai đều KHÔNG phải "match-arm move-out":**

1. **Sương mù — name collision.** Hàm test tên đúng chữ `get`, va `get` builtin free-function (Vector/HashMap) tại `lib.rs:2220`. Call `get()` (0 arg) → `arguments.len() != 2` → `unsupported_expr(callee)` (`lib.rs:2223-2227`) → in `Identifier { name: "get" }`. Đổi `get`→`fetch`: lỗi bốc hơi.
2. **Kẻ thù thật — expected-type.** Đổi tên xong lộ: `function fetch() -> Integer? = ~+ 5` (chỉ producer, không match, không move-out, không heap) → `MIR verification error: OutcomeAlloc on local _0 with non-Outcome type 'Integer?'`.

Probe matrix (mỗi dòng đổi đúng 1 biến, driver chạy độc lập):

| Probe | Hình | Kết quả |
|---|---|---|
| `match make_greeting() { ~+ x => x }` (`String~Integer`) | Outcome move-out | **OK, chạy, exit 0** |
| `fetch() -> Integer? = ~+ 5` + main rỗng | producer nullable | **FAIL OutcomeAlloc** |
| `-> Integer? = fetch()` | passthrough | FAIL OutcomeAlloc |
| `let r: Integer? = fetch()` | consumer | FAIL OutcomeAlloc |
| `let r: Integer? = ~+ 5; match r {…}` (local) | local nullable match | **OK** |

→ `match`-arm move-out trên Outcome **đã chạy**. Temp slot của call-return-aggregate hạ bình thường (fixture 113/139/142). "Match-arm move-out" KHÔNG phải feature mới — nó là **nạn nhân** của expected-type.

### 1.2 Cơ chế khuyết tật (file:line)

`Expr::OutcomeConstructor` (`lib.rs:1712`) quyết Outcome-vs-Nullable bằng cách đọc **`c.sig.return_type`** (kiểu trả về của hàm — context TOÀN CỤC):
- `lib.rs:1722` `~0`: nếu `c.sig.return_type` ≠ Outcome → coi như NullLiteral.
- `lib.rs:1762-1775` payload-type: `if let Outcome = c.sig.return_type` → lấy value/error type; ngược lại `Unknown`.
- `lib.rs:1784-1790` **LUÔN emit `OutcomeAlloc` với `outcome_ty = c.sig.return_type.clone()`**.

Khi `c.sig.return_type = Nullable(T)`: nhánh `if let Outcome` trượt → payload `Unknown`, **NHƯNG vẫn emit `OutcomeAlloc` trên slot kiểu `Nullable(T)`** → verifier bắt `OutcomeAlloc on non-Outcome type`. Đó là **"Bug B"** — tên đã được đặt sẵn trong code tại `lib.rs:1307-1313`.

### 1.3 Các "redirect" bolt-on hiện có — bằng chứng nợ chắp vá

Vì constructor đọc context toàn cục, mọi vị trí context CỤC BỘ phải vá riêng bằng cách **lột `~+` TRƯỚC khi tới constructor**:
- **let-annotation** `let x: T? = ~+ v` — `lib.rs:1314-1324`: nếu annotation nullable + init là `~+ inner`, hạ `inner` plain thay vì OutcomeConstructor.
- **struct-field** `Struct { f: ~+ v }` (field `f: T?`) — `lib.rs:2986-2998`: cùng mánh lột `~+`.
- **return-stmt `~0`** — `lib.rs:1446-1451`; **expr-body `~0`** — `lib.rs:884-895` (chỉ `is_null_expr`, KHÔNG bắt `~+ v`).

Mỗi vị trí value-context mới (block-final, if-arm, match-arm, call-argument, function-return-body cho `~+ v`) lại phải vá lại. Comment `lib.rs:882-883` tự thú: *"Block-final / if-arm `~0` is a SEPARATE expected-type-propagation gap"*.

### 1.4 Đính chính kỹ thuật framing của G (NullableAlloc)

WO nói "gọi đúng Constructor (`NullableAlloc` hay `OutcomeAlloc`)". **KHÔNG tồn tại `NullableAlloc`** (`grep -rn NullableAlloc crates/` = rỗng). Theo PA-3c, "xây" nullable KHÔNG phải một alloc:
- **present scalar** `~+ 5` (`Integer?`) = hạ `5` plain — **value IS the repr** (identity, không tag).
- **present aggregate** `~+ Struct{..}` (`Struct?`) = hạ payload plain + **widening Assign** (slot = size+8, tag@0, fields@+8 — JIT taxonomy case 2, `lib.rs:1339-1362`).
- **null** `~0` (`T?`) = `Const NULL_SENTINEL` (`lib.rs:892` / niche-cho-struct tại runtime).

Vậy nhiệm vụ của expected-type KHÔNG phải "chọn alloc khác", mà là **chọn ĐƯỜNG HẠ**: Outcome-StackSlot vs nullable-(identity/widen/sentinel). ADR giữ đúng cơ chế PA-3c đã có; chỉ thay nguồn-quyết-định từ `c.sig.return_type` (proxy sai) sang `expected_ty` (cục bộ đúng).

---

## 2. Quyết định

**Luồng `expected_ty: Option<&MirType>` được truyền tường minh xuyên cây lowering.** `Expr::OutcomeConstructor` và `Expr::NullLiteral` đọc `expected_ty` (KHÔNG đọc `c.sig.return_type`) để chọn đường hạ. Mọi "redirect" bolt-on (§1.3) bị xoá, thay bằng một quy tắc đồng nhất: **vị trí value-context truyền expected_ty xuống biểu thức của nó.**

### 2.1 Đổi signature `lower_expr`

```rust
// CŨ (lib.rs:1622):
fn lower_expr(expr_id: ExprId, arena: &Arena, c: &mut Ctx) -> Result<Local, LowerError>

// MỚI:
fn lower_expr(
    expr_id: ExprId,
    expected: Option<&MirType>,   // kiểu kỳ vọng tại vị trí; None = không ràng buộc
    arena: &Arena,
    c: &mut Ctx,
) -> Result<Local, LowerError>
```

61 call-site `lower_expr(` cập nhật cơ học. **Mặc định an toàn = `None`** (giữ y hệt hành vi hiện tại cho mọi vị trí KHÔNG phải value-context). Chỉ một nhúm vị trí truyền `Some(_)` (xem §2.3). Quy ước chống churn-vĩnh-viễn: vì 61 site, **không** thêm overload thứ hai — một hàm, một param, đa số `None`.

> **Quyết định G (LOCKED):** param tường minh. Hướng `c.expected_stack` (context ẩn) **bị bác** — cùng họ bệnh với `c.sig.return_type`. Churn 61 site là chi phí một lần, đổi lấy compiler tự-mô-tả.

### 2.2 Quy tắc truyền — TRANSPARENT vs OPAQUE

Phân loại từng vị trí con của mỗi `Expr`:

- **TRANSPARENT** (forward expected_ty xuống nguyên vẹn — giá trị của con CHÍNH LÀ giá trị vị trí cha):
  - `Block { .., tail }` → tail nhận expected của block.
  - `If { cond, then, else }` → cond nhận `Some(Trilean!)`; **then & else nhận expected của if** (cả hai arm cùng kiểu kết quả).
  - `Match { scrutinee, arms }` → scrutinee nhận `None` (kiểu của nó độc lập); **mỗi arm body nhận expected của match**.
  - `OutcomeConstructor`/`NullLiteral` = **LEAF tiêu thụ** (xem §2.4).
- **OPAQUE** (con nhận `None` — kiểu con không liên quan kiểu cha):
  - `BinaryOp`/`UnaryOp` operands, comparison, logic ops.
  - index/receiver/argument-trong-builtin, condition của while, scrutinee của match.
  - *(call-argument: tương lai có thể truyền param-type; ADR này để `None` — out of scope, ghi backlog.)*

### 2.3 Nguồn của expected_ty (nơi sinh `Some`)

| Vị trí | expected_ty đến từ | file:line hiện tại |
|---|---|---|
| Function body tail (block-final / expr-body) | `c.sig.return_type` | `lib.rs:878-898` |
| `Stmt::Return expr` | `c.sig.return_type` | `lib.rs:1446` vùng |
| `let x: T = init` | annotation `T` (qua `lower_type_simple`) | `lib.rs:1307-1366` |
| Struct-field init `Struct{ f: e }` | kiểu khai báo của field `f` | `lib.rs:2954-2998` |
| Match-arm body | expected của `match` (transparent) | nhánh match §2.2 |
| If-arm body | expected của `if` (transparent) | nhánh if §2.2 |

Lưu ý: `c.sig.return_type` KHÔNG biến mất — nó trở thành **nguồn ban đầu** của expected_ty tại đúng 2 vị trí (function-body-tail + return-stmt), thay vì bị constructor đọc lén ở tầng sâu.

### 2.4 LEAF tiêu thụ — `OutcomeConstructor` & `NullLiteral` tái cấu trúc

`Expr::OutcomeConstructor { arm, payload }` đọc `expected` (KHÔNG đọc `c.sig.return_type`):

```
match expected {
  Some(Outcome{value_type, error_type, ..}) =>
      // ĐƯỜNG OUTCOME (giữ nguyên lib.rs:1762-1810): OutcomeAlloc + disc + payload.
      // payload_ty = value_type/error_type theo arm.
  Some(Nullable(inner)) =>
      match arm {
        Positive(Some(p)) => lower_expr(p, Some(inner), ..)   // payload-plain; widen do vị trí cha Assign lo (PA-3c)
        Zero               => Const NULL_SENTINEL              // null repr
        Negative(_)        => Err  // ~- trên T? — typecheck E lẽ ra đã chặn
      }
  Some(other_non_wrapper) | None =>
      Err(null_literal_without_expected_type / outcome_without_expected_type)
}
```

`Expr::NullLiteral` (`lib.rs:1679`) đồng dạng: `Some(Nullable(_))` → sentinel; `Some(Outcome{..})` → Outcome-zero (disc=0); khác → `Err` (đây chính là `is_null_expr` special-case ở `lib.rs:884` được TỔNG QUÁT HOÁ — xoá nhánh ad-hoc).

**Hệ quả dọn nợ:** ba redirect bolt-on (`lib.rs:1314-1324`, `2986-2998`, `884-895`) **bị xoá**. Chúng từng phải lột `~+`/`~0` trước constructor vì constructor đọc sai context; nay constructor đọc đúng `expected` → để `~+`/`~0` đi thẳng qua `lower_expr(.., Some(field_or_annotation_ty))`. Ít code hơn, một đường duy nhất.

---

## 3. Phạm vi & blast radius

- **Signature churn:** 61 call-site `lower_expr(` (cơ học, đa số thêm `None`).
- **Đọc `c.sig.return_type` cho quyết-định-constructor:** 11 usage (`grep` 2026-06-27) — gom về §2.3.
- **Xoá:** 3 redirect bolt-on + nhánh `is_null_expr` ad-hoc tại function-body.
- **KHÔNG đụng:** JIT (`triet-jit`), MIR statement set (`OutcomeAlloc`/`StructAlloc`/`EnumAlloc` giữ nguyên — KHÔNG thêm `NullableAlloc`), borrowck. Đây thuần là tầng `triet-lower`.

**Bất biến phải giữ (regression gate):** mọi fixture Outcome (113/139/142, 107-135) + nullable local (225-230) + nested-nullable-aggregate (ADR-0065) **byte-identical**. Đường Outcome `Some(Outcome{..})` = copy nguyên logic cũ; đường nullable local đã đi qua redirect → nay đi qua expected_ty cùng kết quả.

---

## 4. Tiêu chí mở khoá (feature lộ ra sau khi ADR landed)

1. `function f() -> T? = ~+ v` / `= ~0` hạ đúng (scalar + aggregate + heap-aware theo ADR-0062 nếu đã mở).
2. `match call_returning_T_question() { ~+ s => … ~0 => … }` chạy.
3. `if c { ~+ v } else { ~0 }` với kiểu kết quả `T?` chạy (Gap #2 if-arm).
4. Block-final `{ …; ~0 }` trong vị trí kỳ vọng `T?`/Outcome chạy (Gap #2 block-final).
5. Toàn bộ corpus cũ XANH byte-identical.

---

## 5. Phương án bị bác

**Option A — chắp vá per-site:** thêm redirect "Bug B" tại function-body-tail (và sau đó if-arm, match-arm…). **Bác (G, 2026-06-27):** *"Hôm nay vá function-body, ngày mai vá if-arm, ngày mốt vá match-arm? Đéo bao giờ."* Chính TODO Gap #2 đã cảnh báo "KHÔNG chắp vá per-site". Option A nhân nợ; mỗi vị trí value-context mới = một redirect mới + một mặt bug mới.

---

## 6. Backlog sinh ra (KHÔNG quên — G chốt 2026-06-27)

- **BuiltinShadowing UX trap (E-code mới).** User đặt tên hàm trùng builtin (`get`/`append`/…) → hiện quăng `unsupported_expr` khó hiểu (case study §1.1 nướng 1 phiên). Sửa: error đàng hoàng `ReservedBuiltinName` / `BuiltinShadowing` (namespace `triet::lower::Exxxx` hoặc typecheck). Ưu tiên: gom vào một WO dọn dẹp sau, KHÔNG ưu tiên số 1, KHÔNG ĐƯỢC QUÊN.
- **call-argument expected_ty:** truyền param-type xuống argument (cho `~+`/`~0` làm tham số). Out of scope ADR này; ghi nhận khi cần.

---

## 7. Liên kết

- Đóng **TODO Gap #2** (`TODO.md` mục Heap-Nullable backlog #68).
- Liên quan: ADR-0020 (Outcome), ADR-0041 (PA-3c nullable sentinel), ADR-0065 (nullable aggregate — nguồn của redirect bolt-on), ADR-0062 (heap-nullable — hưởng lợi khi mở).
- Đính chính sổ bàn giao: `spec/plans/MENTOR_G_STATE.md:14/49` ("match-arm move-out blocked by does not support Identifier") = chẩn đoán sai; kẻ thù thật = expected-type (ADR này).
