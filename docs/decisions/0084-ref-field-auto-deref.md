# ADR 0084 — Field projection through read-only reference (auto-deref member-access)

> Status: **DRAFT** — chờ O verify + G ký. KHÔNG tự ký.

> # 🩸 NGUYÊN LÝ CỐT LÕI (O author semantic)
> # Member-access `e.f` khi `e : &0 T` (read-only reference tới UserStruct `T`)
> # **auto-deref ĐÚNG 1 tầng** rồi project field `f` trên pointee. Kind của kết
> # quả do **kiểu tĩnh của field quyết định** — KHÔNG nhập nhằng:
> #  - `f` **scalar** (Trit/Tryte/Integer/Long/Trilean) → **giá trị scalar**
> #    (Copy, đọc qua borrow). Terminal.
> #  - `f` **aggregate** (Struct) HOẶC **heap-leaf** (String/Vector/HashMap) →
> #    **`&0 F` sub-borrow** (zero-copy place projection, CÙNG loan gốc).
> #    Chainable: `(&0 Ngoai).trong` : `&0 Trong` → `.x` auto-deref tiếp.
> #  - **KHÔNG BAO GIỜ copy/move** aggregate hay heap value — chỉ scalar đọc
> #    by-value. Copy heap qua borrow = alias con trỏ heap → double-free lúc drop
> #    (đúng thứ ADR-0082/§AMEND-3 cấm).
> #  - **1 tầng:** mỗi `.f` deref đúng 1 `&0` trước nó; chain hoạt động vì mỗi
> #    bước re-borrow 1 tầng.
> #  - **Read-only:** ghi qua `&0` (`e.f = v`, `*e = v`) GIỮ refuse. `&0 mutable`
> #    có thể ăn theo cùng deref cho READ, nhưng ghi vẫn refuse độc lập.

## Scope

- ✅ **IN (Slice 1a — bản này):** `e.f` với `e : &0 T` / `e : &0 mutable T`, `T`
  = UserStruct, và `f` là **SCALAR** field. Auto-deref 1 tầng → đọc field by
  value (Copy). Cả reference-PARAM (`function read(p: &0 Point)`) lẫn
  reference-tới-LOCAL (`let r = &0 v`). Chain-đầu-scalar tầm thường
  (`(&0 v).x`).
- 🔜 **Slice 1b (DEFERRED):** `f` là **aggregate/heap** field → `&0 F` sub-borrow
  zero-copy (chainable). Cần cơ chế place-projection giữ loan qua lowerer/JIT
  chưa dựng — mở riêng để tránh copy-lén heap. Semantic đã khóa ở §CỐT LÕI trên.
- ❌ **OUT:**
  - `&0 Enum` field-access (`(&0 e).field`) → giữ **E1015 UnknownMember**. Enum
    truy cập qua `match`, không `.field`.
  - Ghi qua reference (`e.f = v`) → refuse (§WART: hiện parser E0007 đã chặn mọi
    non-identifier assignment target, nên chưa có bề mặt ghi để test).
  - `&+`/`&-`/`WeakObserver` field-access → ngoài phạm vi (chưa khảo).

## Sites (đã wire — Slice 1a)

1. **Typecheck** `crates/triet-typecheck/src/check/exprs.rs` `check_field_access`
   (~1608): đầu hàm, nếu `object_ty = Type::Reference(BorrowReadOnly |
   BorrowExclusiveMutable, UserStruct{fields})` và field tồn tại và
   `field_ty.is_scalar()` → trả `field_ty.clone()`. Aggregate/heap field KHÔNG
   khớp `is_scalar()` → rơi về `UnknownMember` như cũ (Slice 1b sẽ mở).
2. **Lowerer** `crates/triet-lower/src/lib.rs`:
   - `place_result_type` (~1699): thêm arm `Projection::Deref` → unwrap
     `MirType::Reference{inner}`.
   - `lower_place` `Expr::FieldAccess` arm (~1682): nếu base resolve ra
     `MirType::Reference{..}` → chèn `Projection::Deref` TRƯỚC `Field`. Chain lồng
     chỉ sinh 1 Deref ở đầu (field struct thường không tự lưu dưới dạng pointer).
3. **JIT** `crates/triet-jit/src/mir_lower.rs`:
   - `walk_projections` (~349): thêm arm `Projection::Deref` → unwrap
     `Reference{inner}`, offset KHÔNG đổi (deref không dịch địa chỉ; nhánh
     "pointer-based fallback" trong `load_place`/`store_place` @~1295 vốn cộng
     `total_offset` vào giá trị pointer trong var của `place.local`).
   - **Blocker B (§WART-B)** `Statement::Borrow` (~3110): mở rộng — lấy
     `stack_addr` cho MỌI local slot-backed (`struct_slots`/`enum_slots`), không
     chỉ String. String cũng cư trú `struct_slots` (name == "String") nên nhánh
     mới SUBSUME special-case cũ.
4. **Borrowck** — KHÔNG chạm. Sub-borrow (Slice 1b) sẽ là projection của loan
   `&0` sẵn có, không sinh loan mới; đọc qua `&0` = read-use, không xung đột.

## ⚠️ WART — borrowck LEXICAL (không NLL) — G lệnh ghi rõ

Borrow checker hiện là **lexical**: loan chết ở cuối scope, KHÔNG có NLL
last-use narrowing. Hệ quả với auto-deref: một borrow LOCAL (`let r = &0 v`) còn
sống tới điểm **return của owner** sẽ dính **E2450 DropWhileBorrowed giả**
(ADR-0046 Case-D conservative — fixtures 21/24 khóa hành vi này). Đây **KHÔNG
phải unsound** — chỉ cồng kềnh. Để dùng auto-deref trên borrow local, ép borrow
chết TRƯỚC return bằng:

- **block-scope lồng:** `let n = { let r = &0 v; r.x + r.y }; return n;`
  (`pop_scope` sort reference-trước → borrow drop ở cuối block, trước return).
- **reference-PARAM:** `function read(p: &0 Point) -> Integer { return p.x + p.y }`
  gọi `read(&0 pt)` (borrow chỉ sống qua call).

**NLL = hố đen, defer VÔ THỜI HẠN.** KHÔNG đụng `flush_all_for_return` /
ADR-0046 để "chữa" wart này.

### §WART-B — `Statement::Borrow` slot-address (Blocker B, đã vá)

Trước ADR này, `Statement::Borrow` codegen chỉ special-case String để lấy slot
address; mọi struct/enum local khác dùng `use_var(var(source.local))`. Nhưng
local aggregate được dựng bằng field-level `stack_store` — Cranelift Variable
của nó KHÔNG bao giờ `def_var` → `use_var` trả giá trị **undefined** → borrow
trỏ vào rác → **SIGSEGV** khi field-read đầu tiên qua reference. Chưa lộ vì
KHÔNG fixture nào từng borrow một local struct trực tiếp (chỉ scalar / String /
container-handle / param). Vá: lấy `stack_addr` cho mọi slot-backed local.

## Rationale

- **Zero-copy = triết lý G:** đọc qua borrow không copy; aggregate/heap field
  (Slice 1b) là sub-borrow, không nhân bản.
- **Unambiguous static kind:** kind kết quả (value vs sub-borrow) do kiểu field
  quyết định tĩnh — không cần suy luận runtime.
- **Không copy heap → giữ soundness ADR-0082:** copy heap value qua borrow sẽ
  alias con trỏ → double-free. Slice 1a chỉ đọc scalar (Copy) nên an toàn tuyệt
  đối; heap/aggregate defer sang 1b với cơ chế sub-borrow.
- **Read-only → không dính Cụm D / ADR-0081 (mutable-borrow FROZEN):** Slice 1a
  chỉ READ; write-path chưa tồn tại (parser gate).

## Teeth (Slice 1a)

- **381** (`381_ref_field_scalar_param.tri`, EXPECT 30) — scalar field qua `&0
  Point` PARAM; caller `read(&0 pt)` borrow local struct → exercise Blocker-B
  fix. Poison-2-tầng: gỡ auto-deref typecheck → E1015; revert Blocker-B →
  SIGSEGV 139.
- **382** (`382_ref_field_scalar_block_local.tri`, EXPECT 30) — scalar field qua
  `&0` tới LOCAL struct, borrow confined trong block lồng (né §WART E2450).
- **T3 nested (xác nhận thuộc 1b, KHÔNG viết ở 1a):** mọi truy cập nested
  (`o.trong.x` với `trong : Trong` aggregate) cần sub-borrow ở bước `.trong` →
  Slice 1b. KHÔNG có tooth nested-scalar hợp lệ trong 1a.
- **Không teeth âm mutation:** parser E0007 (`InvalidAssignmentTarget`) đã chặn
  mọi non-identifier assignment target ở tầng parse — vacuous với auto-deref.
