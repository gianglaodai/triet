# ADR 0076 — Heap-Nullable trong aggregate field/payload (giao điểm B8 — khép kỷ nguyên Nullable)

> # 🩸 NGUYÊN LÝ CỐT LÕI (G khắc đá 2026-06-29)
> # Một ngôn ngữ mà `let x: String? = …` chạy mượt nhưng `struct S { x: String? }`
> # bị compiler nhổ toẹt là ngôn ngữ **cụt lủn, thiếu đồng nhất**. Móng struct/enum
> # layout phải GÁNH TRỌN kiểu dữ liệu của ngôn ngữ trước khi mở bất kỳ frontier mới.
> # Không bỏ lại một lỗ rò.

**Trạng thái:** 🔒 **QUYẾT ĐỊNH (SEALED) — O verify máu + G FINAL sign-off 2026-06-29** (commit `6327890`, gate `0·0·306·0`).
Áp dụng Bậc C+. Đây là **mũi cuối khép kỷ nguyên Nullable**: heap-`T?` (`String?`/`Vector?`/
`HashMap?`) ở vị trí **struct-field / enum-payload** — giao điểm B8 duy nhất còn refused.
**Triển khai = MỘT LÁT ATOMIC** (5 mũi liên động: gate+layout+drop+construct+borrowck khóa nhau —
nới gate thiếu layout = SIGSEGV, có layout thiếu drop = leak; ráp một phát động cơ nổ, G chốt).

> **Lát đơn `6327890` — O verify máu độc lập 3 tooth lõi (cp-snapshot, restore byte-identical):**
> #1 CASE B tombstone (`lower:3700` gỡ Deinit-after-present-bind → double-free SIGABRT 134 ×3 biến thể) ·
> #2 **sinh-tử** `is_copy(Nullable(heap))==false` (`mir:691` poison `→true` → heap-`T?` thành Copy → no-drop → **7 counting test LEAK đỏ**) ·
> #3 drop-arm mũi-3 (`jit:472` gỡ collect arm → 7 counting leak FREE==0).
> **Lỗ O vồ ở vòng 1:** match-present-bind-move heap-`T?`-aggregate **compile-thành-double-free, borrowck im** (MỚI do gate-lift — pre-WO exit 4, vòng-1 = 134). D đóng STATIC (tombstone outer-Nullable = tag-niche drop-flag, KHÔNG dynamic-flag). Run-witness 180/230/236/255/311/312 + 310→E2423.

**Issue — incoherence một-giao-điểm:** heap-nullable đã CHẠY gần trọn (23 fixture RUN: top-level
`String?`/`Vector?`/`HashMap?` null/present/use-after-move/match `~+~0`/Elvis/`?+>`-map/method-
return; `Enum?`/`Struct?` aggregate; nested). Chỉ còn **4 fixture refused**, cùng một hình:
heap-`T?` làm field của struct hoặc payload của enum (`struct S{x:String?}`, `enum Bag{Has(String?)}`).
Compiler chấp nhận top-level nhưng nhổ ra ở aggregate → ngôn ngữ thiếu đồng nhất. ADR này khép lỗ.

---

## 0. Current Reality (recon O đo file:line, 2026-06-29 — khắc đá, không đoán)

| # | Phát hiện | Chứng cứ (file:line) | Hệ quả thiết kế |
|---|---|---|---|
| R1 | Gate vị-trí-phụ-thuộc, 2 predicate | `triet-mir/src/lib.rs:1431` (`is_lowerable_nullable_payload`), `:1482` (`is_field_payload_lowerable`) | Return/local/param: heap-`T?` ĐƯỢC LOWER. Field/payload: heap-`T?` REFUSED |
| R2 | Chokepoint refuse | `triet-mir/src/lib.rs:1573-1596` (`Body::verify` quét `struct_layouts`+`enum_layouts` qua `find_refused_nullable_field`) | Lift = nới `is_field_payload_lowerable` cho heap |
| R3 | ptr-sentinel ĐÃ có | `triet_mir::NULL_SENTINEL` | Null heap = ptr@slot == NULL_SENTINEL (`i64::MIN`) |
| R4 | Drop-shim ĐÃ no-op trên sentinel | `triet-jit/src/mir_lower.rs:2942, 3214, 3437` (`if ptr == 0 \|\| ptr == NULL_SENTINEL { return }`) | **Conditional-drop KHÔNG cần rẽ nhánh JIT** — shim nuốt điều kiện |
| R5 | `collect_heap_leaves` bỏ qua Nullable | `triet-jit/src/mir_lower.rs:450-466` (arm `_ => {}` scalar-skip) | Field heap-`T?` hiện **leak** (không emit drop) |
| R6 | Field sizing heap-aware | `triet-lower/src/lib.rs:549` (`String → 24`, else 8) | `String?` field tái dùng layout `String` (24B fat); `Vector?`/`HashMap?` = 8B handle |
| R7 | LeafKind | `triet-jit/src/mir_lower.rs:214` (`{Heap(MirType), Enum(String)}`) | Tái dùng `Heap(inner)` khả thi (xem PA #1) |
| R8 | Tombstone + free-dispatch site | `mir_lower.rs:1541` (tombstone zero@abs), `~2041` (free-shim) | Cả hai cần nhận diện Nullable(heap) |

**Biên giới chốt cứng:** 4 fixture refused — `180_heap_nullable_struct_field_refused`,
`230_enum_nullable_heap_payload_refused`, `236_struct_nullable_heap_field_refused`,
`255_field_string_nullable_b8_refuse` — đều là heap-`T?` ở field/payload. KHÔNG có gì khác refused.

---

## Quyết định

Nới gate field/payload cho heap-`T?`, đặt slot heap (sentinel-bearing) tại **field-offset**,
tái dùng drop-shim sentinel-safe. **5 mũi dao phẫu thuật**, nối thẳng cỗ máy heap-aggregate
(WO-0073/74/75) — KHÔNG đại phẫu value-model.

### Hình thức cụ thể

**Mũi 1 — Gate Lifting** (`triet-mir/src/lib.rs:1482`):
`is_field_payload_lowerable(inner, body)` nới: `scalar || (Struct/Enum Copy) || inner.is_any_heap()`.
Heap-`T?` field/payload qua gate. (Heap-bearing aggregate vẫn lọc qua `is_copy` ở mũi 5.)

**Mũi 2 — JIT Layout tại field-offset** (`triet-jit/src/mir_lower.rs`, struct/enum slot sizing):
field `String?` cấp 24B fat `{ptr,len,cap}` @offset (= layout `String`, R6); `Vector?`/`HashMap?`
cấp 8B handle @offset. **Null = đổ NULL_SENTINEL vào ptr@field-offset** (không phải 0 — phân biệt
moved-out=0 vs null=sentinel; shim no-op cả hai nên drop sound, nhưng giữ sentinel cho match `~0`).

**Mũi 3 — Drop-Glue** (`mir_lower.rs:450-466` collect + `:1541`/`:2041` dispatch):
`collect_heap_leaves` thêm arm `Nullable(inner) if inner.is_any_heap() => push heap-leaf @abs`.
Drop/tombstone emit **vô điều kiện** trên ptr@abs → shim sentinel-safe (R4) tự no-op khi null.
**KHÔNG `brif` ở Cranelift** — đây là cổ tức kiến trúc PA-3c (§Conditional-drop).

**Mũi 4 — Construct/Widen vào field** (fixture 255 `Bad{s: ~+ "hi"}`):
`~+ <heap>` materialize fat-pointer thật → store @field-offset; `~0`/null → store NULL_SENTINEL
@field-offset. Tái dùng đường widen top-level (`NullableStructCopy`-analog), dời sang field-offset.

**Mũi 5 — Borrowck Verify** (G mandate — không lọt UAM):
Đảm bảo struct/enum mang heap-`T?` field phân loại **Move (non-Copy)** → có drop-glue + move-tracking;
ADR-0070 `partial_moves` projection-path bao phủ trạng thái field. **KHÔNG mở partial-heap-field-
move-out** (`let s = b.s` — vẫn là Nợ defer ADR-0070, đòi dynamic drop-flag) — scope ADR này chỉ
construct + whole-struct-move + drop, KHÔNG move field heap ra riêng. **G chốt 2026-06-29: gặp
`let s = b.s` (b.s heap) → quăng thẳng E2423 vào mặt user** (giữ refuse hiện hành, KHÔNG scope creep).

### §Conditional-drop = sentinel-no-op (cổ tức kiến trúc — câu trả lời cho G)

Câu hỏi: *"drop-glue chạm field heap có-thể-null, rẽ nhánh thế nào cho sound?"*
**Trả lời: KHÔNG rẽ nhánh.** Ba trạng thái field-ptr@offset đều an toàn dưới cùng một lệnh Drop:
| Trạng thái field | ptr@offset | Drop-shim (R4) |
|---|---|---|
| present | ptr thật | free → đúng |
| null (`~0`) | NULL_SENTINEL | no-op → sound |
| moved-out (tombstone) | 0 | no-op → sound |
Điều-kiện-tính bị nuốt vào shim. Emit Drop idempotent. PA-3c ptr-sentinel (ADR-0041) trả cổ tức.

---

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **LeafKind mới `NullableHeap` vs tái dùng `Heap(inner)`** | NullableHeap: tường minh ý đồ | Heap: 0 variant mới, vì offset+shim trùng plain-heap | **Tái dùng `Heap(inner)`** (implementer-choice D) — invariant: slot luôn giữ {ptr thật, 0, sentinel}. Nếu D thấy cần phân biệt semantics → NullableHeap, ghi lý do |
| 2 | Gate ở **typecheck** vs **MIR-verify** | typecheck: lỗi sớm | MIR: stdlib DECLARE heap-nullable API được; chỉ refuse compile | **Giữ MIR-verify** (ruling β, ADR-0062) — nhất quán toàn saga |
| 3 | Mở **partial-heap-field-move-out** (`let s=b.s`) trong ADR này | đối xứng | đòi JIT dynamic drop-flag (Nợ defer ADR-0070); scope creep mổ-tim | **KHÔNG** — giữ Nợ defer; ADR này chỉ construct+whole-move+drop |
| 4 | Null = store **0** vs **NULL_SENTINEL** @field-offset | 0: đơn giản | mất phân biệt null vs moved-out cho match `~0` | **NULL_SENTINEL** — match `~+/~0` cần discriminate; drop sound cả hai |

---

## Hậu quả

### Tích cực
- Khép kỷ nguyên Nullable: heap-`T?` đồng nhất mọi vị trí (local/return/field/payload).
- Móng struct/enum layout gánh trọn kiểu dữ liệu → mở đường frontier mới (Outcome ABI/native layout).
- 4 fixture negative FLIP → positive run-witness (LUẬT THÉP #3, như 298/302 WO-0075).
- 0 dòng value-model; thuần mở rộng heap-aggregate machinery đã có.

### Tiêu cực
- Field sizing thêm nhánh Nullable(heap) → tăng bề mặt layout-code (surgical, bounded).

### Rủi ro cần mitigate
- **`is_copy(Nullable(String))` PHẢI = false** — nếu true → struct phân loại Copy → no drop → LEAK. Teeth bắt buộc (mũi 5).
- **Null-store sai offset/giá-trị** → shim đọc rác → SIGSEGV. Teeth counting + poison (mũi 2/4).
- **Tombstone vs sentinel lẫn lộn** → double-free. Teeth move-then-drop FREE==1 (mũi 3).

---

## Teeth (O verify máu độc lập — poison phải đỏ, restore cp KHÔNG git checkout)

| Mũi | Tooth | Poison → kỳ vọng đỏ |
|---|---|---|
| 1 | gate-lift load-bearing | re-add refuse heap-field → 180/230/236/255 quay lại refused (control: scalar-field vẫn qua) |
| 2/4 | layout + null-store | counting: field `String?` present → FREE==1, null → FREE==0; poison store-0-thay-sentinel → match `~0` sai / SIGSEGV |
| 3 | drop arm load-bearing | gỡ arm `Nullable(heap)` trong `collect_heap_leaves` → field present LEAK (FREE==0 đỏ) |
| 3 | no-double-free | construct→move struct→drop: FREE==1; poison bỏ tombstone → FREE==2 |
| 5 | is_copy non-Copy | poison `is_copy(Nullable(heap))→true` → struct Copy → no drop → LEAK đỏ |
| 5 | UAM | use-after-move struct mang heap-`T?` field → E2420 |

Mỗi mũi đập một biến thể `String`/`Vector`/`HashMap` (TEETH quét cả không-gian-biến-thể — bài học HP.3).

## Quan hệ ADR

- **Kế thừa:** ADR-0041 (PA-3c sentinel — cổ tức conditional-drop), ADR-0062 (heap-nullable top-level
  + ruling β gate-at-MIR), ADR-0065 (nullable aggregate `Enum?`/`Struct?` niche/tag-prepend),
  ADR-0066/0067 (No-Box heap-in-aggregate, `collect_heap_leaves`/drop-glue), ADR-0070 (partial-move
  projection-path move-state).
- **KHÔNG đụng:** ADR-0068 (Box/recursive — CẤM CỬA). Mở Nợ defer: partial-heap-field-move-out.

## Ngày hiệu lực

- Bậc C+ — lift gate + field-layout + drop-arm khi từng lát landed (O verify máu, G ký từng lát).
- Không hồi tố. Top-level heap-nullable (ADR-0062/0065) không đổi hành vi.
