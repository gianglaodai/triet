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

## §AMEND — Slice 1b landed (DRAFT, chờ O verify + G ký)

> Status phần này: **DRAFT** — KHÔNG tự ký.

Slice 1b thi công đúng phần DEFERRED của §CỐT LÕI (aggregate/heap field qua `&0`
→ `&0 F` sub-borrow zero-copy). KHÔNG semantic mới. 4 tầng:

1. **Typecheck** `check_field_access` (exprs.rs ~1620): nhánh auto-deref mở rộng —
   sau khi khớp `Reference(BorrowReadOnly|BorrowExclusiveMutable, UserStruct)` và
   field tồn tại: `is_scalar()` → value (1a giữ nguyên); `UserStruct{..}` HOẶC
   `is_heap()` (String/Vector/HashMap) → trả
   `Type::Reference(BorrowReadOnly, field_ty)` (sub-borrow LUÔN read-only, kể cả
   base `&0 mutable`). Field kind khác (Enum, Nullable-aggregate) vẫn rơi
   `UnknownMember` (§OUT).
2. **Lowerer** `Expr::FieldAccess` rvalue (lib.rs ~3313): nếu `source` (do
   `lower_place` sinh) chứa `Projection::Deref` VÀ `place_result_type` là
   `Struct`/heap → emit `Statement::Borrow{form:BorrowReadOnly, source:[Deref,
   Field]}` (dùng dest kiểu `Reference`), thay vì `Assign` value-copy. Scalar
   terminal (1a) và owned-base move-out (WO-0074/0075, KHÔNG có Deref) đều rơi
   xuống `Assign` cũ — không đổi.
3. **JIT** `Statement::Borrow` codegen (mir_lower.rs ~3127): `source` nay có thể
   projected. Nhánh mới — `walk_projections` (Deref cộng 0 offset, chỉ unwrap
   type; Field cộng field offset) trả `total_offset`, rồi cộng vào CÙNG base
   (slot address hoặc pointer-value) mà nhánh bare-local đã dùng. Số học địa chỉ
   thuần, KHÔNG load/copy bytes.
4. **Borrowck** `Statement::Borrow` (checker.rs ~646): **WHOLE-OBJECT FALLBACK**
   (refuse-over-guess, G lệnh) — nếu `source.projection` chứa `Deref` thì loan
   KHÔNG fine-grain field; anchor lên whole object. **REBORROW CHASE:** combo
   form `(&0 h).name` lower thành HAI Borrow (`tmp = &0 h` rồi sub-borrow
   `tmp.name`); `tmp` là temp chết ngay → nếu chỉ strip về
   `Place::local(source.local)` thì loan anchor lên `tmp`, `Drop(h)` không thấy
   loan → dangling lọt câm. Fix: nếu `source.local` LÀ `dest` của một active loan
   (vừa được borrow ra) → kế thừa `source` của loan đó (chase về `h`); nếu là
   PARAM `&0 T` (không có loan tạo nó) → dùng `Place::local(source.local)`.

### Teeth (Slice 1b) — 383–387, gate `0·0·381·0`

- **383** (`383_ref_field_heap_leaf_sub_borrow.tri`, EXPECT 5) — heap-leaf
  `h.name` (`String`) qua `&0 Holder` PARAM → sub-borrow `&0 String` → `length`.
  Đường E1049 hứa. **Leading `tag: Integer`** đẩy `name` sang offset≠0 (né mask
  offset-0-coincidence). Poison JIT (revert projected-addr) → silent-wrong (đọc
  `tag` bit-pattern làm `{ptr,len,cap}` → 383 trả 140155117966008).
- **384** (`384_ref_field_nested_scalar_sub_borrow.tri`, EXPECT 7) — chain
  `o.trong.x`: `.trong` sub-borrow `&0 Trong`, `.x` scalar terminal. Leading
  `pad` trên CẢ HAI struct → cả hai offset≠0. (Đi qua Assign scalar-read, KHÔNG
  qua Statement::Borrow → poison-JIT không đụng — đây là tooth chain-typecheck +
  offset-accumulation.)
- **385** (`385_ref_field_nested_heap_sub_borrow.tri`, EXPECT 4) — chain 2 tầng
  tới heap: `o.trong.name` → `&0 String`. Poison JIT → 385 trả 2 (sai).
- **386** (`386_ref_field_sub_borrow_dangling.tri`, ERROR E2450) — POISON dangling:
  `(&0 h).name` return escape → E2450. Poison borrowck (bỏ reborrow-chase, plain
  strip) → E2450 BIẾN MẤT (chỉ còn E2400 pre-existing return-tie ambiguity, một
  chẩn đoán ĐỘC LẬP — xem Nghi ngờ §b).
- **387** (`387_ref_field_sub_borrow_move_while_borrowed.tri`, ERROR E2440) —
  POISON move-while-borrowed: `let s=(&0 h).name; let h2=h; length(s)` → move `h`
  khi `s` sub-borrow → E2440 (dùng move thay mutate vì `h.x=` không parse E0007).
  Poison borrowck (bỏ chase) → E2440 biến mất, compile pass → dangling `s`.

### Nghi ngờ / giả định (D báo trung thực)

- **(a) Whole-object false-conflict:** hai sub-borrow field KHÁC nhau qua CÙNG
  reference (`h.name` và `h.other`) sẽ false-conflict (whole-object loan). Đây là
  GIÁ refuse-over-guess G chấp nhận. KHÔNG có fixture hợp lệ nào trong corpus
  hiện tại đụng ca này (chưa có surface đọc-2-field-đồng-thời qua `&0`).
- **(b) Poison-386 KHÔNG "compile pass" hoàn toàn:** khi gỡ reborrow-chase, E2450
  biến mất ĐÚNG (chứng minh chase load-bearing), NHƯNG một lỗi ĐỘC LẬP
  E2400 "cannot infer which input the returned borrow ties to" vẫn chặn compile
  (return-borrow tie-to-input ambiguity, pre-existing, không phải Slice 1b). Nên
  poison-386 chứng minh chase-là-load-bearing-cho-E2450, KHÔNG chứng minh
  "dangling chạy tới JIT". Ghi rõ để O không nhầm.
- **(c) Vector/HashMap-field stride:** đã xác nhận cả String-field (fat 24B inline
  → addr của `{ptr,len,cap}` tại field-offset) LẪN Vector/HashMap-field (thin 8B
  handle → addr của handle tại field-offset) đều trả đúng địa chỉ: JIT dùng CHUNG
  `walk_projections` offset + base-addr, không phân biệt stride (chỉ trả ADDRESS,
  không load). Fixture 383/385 test String-field; Vector/HashMap-field cùng đường
  addr nhưng KHÔNG có fixture riêng (thu hẹp: chỉ String field được test end-to-end
  với `length`; Vector/HashMap-field-sub-borrow chưa có builtin đọc để exercise —
  báo O nếu cần fixture bổ sung).
