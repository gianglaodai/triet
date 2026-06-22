# ADR 0067 — Trục B Lát 2: Nested-Flat & Enum-Payload Heap Drop-glue (No-Box)

> # ⚖️🩸 LUẬT THÉP SOUNDNESS (kế thừa ADR-0066 — vẫn hiệu lực)
> # `byte-copy` ⟶ `tombstone-source` PHẢI ATOMIC TRONG CÙNG MỘT BASIC BLOCK.
> Mọi move của aggregate chứa heap (nested hay enum-payload) vẫn tuân LUẬT THÉP:
> copy + tombstone liền kề, không panic/CFG-branch/call xen khe. (G khắc đá 2026-06-21.)

**Trạng thái:** Đề xuất (scaffold giấy trắng — recon-trước, CHƯA code; chờ G ký từng nhát).
Áp dụng Bậc C+. Mở rộng heap-in-aggregate từ **FLAT một tầng** (ADR-0066 Lát 1) lên
**nested non-recursive (bounded)** + **enum-payload heap** — **KHÔNG box, KHÔNG recursive**.

**Issue:** ADR-0066 Lát 1 mở khóa `struct{ f: String }` (heap leaf TRỰC TIẾP, FLAT một tầng).
Rào M-2 (`lib.rs:~3052`) vẫn **refuse transitive** — `struct{ inner: HasHeap }` (HasHeap chứa
String) bị chặn ở construction; KCN-1 drop-glue (`mir_lower.rs:1728`) chỉ walk MỘT tầng, filter
`is_any_heap()` (= String/Vector/HashMap, KHÔNG gồm Struct) nên bỏ qua field-kiểu-struct → leak.
Enum-payload heap chưa có drop-glue (Drop handler chỉ `MirType::Struct`; enum heap chỉ Outcome
2-arm). Đây là tầng vật lý kế thừa 100% móng Lát 1 — KHÔNG đụng allocator/box.

**Quan hệ ADR:** kế thừa ADR-0066 (KCN-1 inline drop-glue + KCN-2 copy-then-tombstone + M-1/M-2).
Tổng quát hóa `emit_outcome_drop_glue` (ADR-0057, 2-arm) → N-arm cho enum. Layout nested:
ADR-0060 (fixup size cho aggregate field). Box `&+` + true-recursive → **ADR-0068 (defer)**.

---

## Quyết định (scaffold — chi tiết khóa theo từng nhát recon)

Mở rộng B8 cho **bounded heap-in-aggregate no-box** bằng 2 nhát:

### Nhát 2a — Nested non-recursive heap-in-struct (bounded recursive drop-glue)
`struct Holder { inner: HasHeap }` (HasHeap chứa String/Vector/HashMap, KHÔNG self-ref):
- **M-2 nới:** cho phép field kiểu Struct/Enum chứa heap **transitive** (KHÔNG self-ref/recursive —
  cái đó typecheck đã chặn + defer ADR-0068). Vẫn refuse box/`&+`.
- **KCN-1 → drop-glue đệ quy TĨNH:** walk layout đệ quy compile-time, mỗi field kiểu struct-chứa-heap
  → đệ quy vào layout của nó, **accumulate offset**; mỗi heap LEAF → free tại absolute offset.
  Độ sâu = nesting TĨNH (compile-time, struct graph là DAG vì recursive bị chặn) → **KHÔNG đệ quy
  runtime, KHÔNG nổ stack**.

#### 🔒 Nhát 2a — ĐÃ THI CÔNG (chờ O verify máu + ký; D KHÔNG tự điền chữ ký)
- **2a-1 M-2 nới (lib.rs ~3061):** thêm `is_nested_struct` = bare `Struct` có layout resolve → ALLOW.
  **CHỈ Struct, KHÔNG Enum** (thu hẹp có chủ đích so với chữ "Struct/Enum" của WO): `collect_heap_leaves`
  chỉ đệ quy Struct; enum-payload heap = tag-dependent → 2b. Copy-enum vẫn pass qua `ctx_is_copy`;
  heap-enum field GIỮ refuse (ép 2b xử lý drop-glue enum). `Nullable(heap)` (ADR-0062) + box/`&+` GIỮ refuse.
- **2a-2 `collect_heap_leaves` (mir_lower.rs):** associated fn compile-time, đệ quy Struct, accumulate
  absolute offset, trả `Vec<(i32, MirType)>` phẳng. **Depth-limit 64 → JitError** (bùa chống nổ stack).
  **DÙNG CHUNG Drop + Deinit** (đối xứng Sinh-Tử G mandate): Drop `emit_heap_free_at(copy_base_addr(local,abs))`;
  Deinit `stack_store(0, slot, abs)`. FLAT một tầng (Lát 1) = trường hợp depth-0 → giữ xanh.
- **2a-3 Move nested:** tái dùng 1b/1c (aggregate byte-copy total_size>8 + Deinit đệ quy 2a-2). 0 dòng mới.
- **Teeth (O verify):** R-leak-nested (collect non-recursive → Drop Unsupported, refuse > leak) · R-double-free-nested
  (Deinit một-tầng → FREE_COUNT==2 double-free) · R-recursive-creep (tháo depth-limit → stack-overflow SIGABRT).
  Fixtures 263/264/265, counting `struct_nested_heap_counting`, unit `collect_heap_leaves_recursive`.
- **Fixture 257 FLIP** (LUẬT 3, O sign-off): 1d negative `Outer{inner:Inner}` refuse → 2a unlock → EXPECT.

### Nhát 2b — Enum-payload heap (tag-switch drop-glue N-variant)
`enum E { A(String), B(Integer), C }`:
- **Construction:** gỡ refuse enum-payload-heap (lib.rs:1890).
- **Drop-glue:** tổng quát hóa `emit_outcome_drop_glue` (2-arm) → **N-arm tag-switch**: read disc →
  switch → free heap payload của variant ACTIVE (không chạm rác variant khác). No-op cho unit/scalar
  variant.

#### 🔒 Nhát 2b — ĐÃ THI CÔNG (chờ O verify máu + ký; D KHÔNG tự điền chữ ký)
**Recon-trước bắt gap payload-layout (D, analog 1a STEP-4; O verify + rule IN-SCOPE):** enum payload size
hardcode 8 (lib.rs:603) → String-payload slot 16B; construction store CHỈ ptr@8 (STEP-4 fat-sync chỉ
`struct_slots`); drop-glue đọc cap@payload+16 = slot+24 = OOB slot 16B → UB. Vector/HashMap (thin 8B) KHÔNG gap.
→ thêm 2 mảnh phụ:
- **2b-0a (lib.rs:603):** enum payload size heap-aware — `String→24`, Vector/HashMap/scalar→8. String-payload
  `total_size = 8+24 = 32` → slot đủ `{disc@0, ptr@8, len@16, cap@24}`. (M-1 struct-fixup KHÔNG chạm enum — site duy nhất.)
- **2b-0b (mir_lower.rs Assign):** fat-store String payload vào `enum_slots` (analog STEP-4): copy len@payload+8/cap@payload+16 từ src String-slot, đọc src TRƯỚC M1-zeroing.
- **2b-1 (lib.rs gate EnumLiteral + EnumVariant-call):** `is_direct_heap_leaf || ctx_is_copy` → allow leaf/Copy;
  refuse struct-transitive-heap (collect-trong-arm = 2b+) + Nullable(heap). EnumLiteral path TRƯỚC chưa có gate.
- **2b-2 (`emit_enum_drop_glue`):** N-arm `brif` chain — read disc@0, mỗi variant heap-payload → arm `icmp disc==dv`
  → `emit_heap_free_at(stack_addr(slot, payload_off=8), variant.payload.ty)`; scalar/unit variant KHÔNG sinh arm.
- **2b-3 (Deinit):** tombstone zero CHỈ payload ptr@8, **KHÔNG đụng disc@0** (disc=0 là variant HỢP LỆ — khác Outcome).
- **Teeth (O verify):** R-enum-leak (str count 0) · R-enum-double-free-move (count 2) · **⚔ R-enum-wrong-variant**
  (`Pair{Text(String),Buf(Vector)}` — cross-wire → gọi nhầm shim → per-type count sai; Buf→vec=1/str=0, Text→str=1/vec=0) ·
  **R-enum-cap** (poison 2b-0b → cap rác ≠ 5). Fixtures 266/267/268, counting `enum_heap_payload_counting`.
- **Defer 2b+:** enum-in-struct-field, payload-struct-chứa-heap (collect đệ quy trong arm).

---

## ⛔ DEFER — tống sang ADR-0068 (Lát 3 — Đại chiến Box, campaign tiền đề riêng)
- **2c True-recursive type** (`struct Node { next: &+ Node }` / `(&+ Node)?`): cần `&+` heap-box
  backend (allocator cấp + box-drop) — CHƯA TỒN TẠI (chỉ MirType variant + S6 borrowck, phong-ấn
  YAGNI ADR-0059).
- **#0 Typecheck self-ref** (`resolve_type` check.rs:1020 → self-ref `Node` raise UnknownType): vá
  cùng 2c (self-ref chỉ hợp lệ KHI qua box/indirection).
- **Iterative drop chống nổ stack:** linked-list/tree sâu → drop đệ quy runtime nổ stack → cần
  iterative (follow-pointer loop) hoặc depth-bound. Quyết định ABI lớn, thuộc ADR-0068.

---

## Các phương án đã cân nhắc
(khóa theo từng nhát recon — scaffold)

## Hậu quả
### Tích cực
- Mở khóa record lồng nhau thực tế (`struct Person { name: String, address: Address }`).
- Enum sum-type chứa heap (`Result`-like, AST node) — nền cho mọi data-structure.
- Kế thừa 100% móng Lát 1, KHÔNG đụng allocator/value-model.

### Rủi ro cần mitigate
- **R-leak-nested:** drop-glue đệ quy bỏ sót 1 tầng → leak. Teeth: nested 2-tầng, poison đệ quy → FREE < N.
- **R-enum-wrong-variant:** tag-switch free nhầm variant → free rác/double-free. Teeth: poison switch.
- **R-recursive-creep:** self-ref lọt M-2 (typecheck miss) → drop-glue đệ quy compile-time vô tận. Teeth: self-ref PHẢI refuse.

## Ngày hiệu lực
- Bậc C Lát 2: 2a nested-flat + 2b enum-payload (no-box).
- Defer ADR-0068: 2c true-recursive + box + iterative-drop + #0 typecheck self-ref.

---

**Chữ ký ADR-0067:** (scaffold — chờ recon từng nhát + G ký)
