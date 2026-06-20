# ADR-0065 — Nullable Aggregate: `Enum?` & `Struct?` (nullable stack-slot)

- **Status:** 🔒 LOCKED — O+G ký duyệt 2026-06-20, Giang chốt hướng (Phương án A cho Struct? + 2 lát thi công + gửi WO Lát 1 cho D). Khởi thảo Mentor O, grounded từ MIR/JIT line-cite.
- **Date:** 2026-06-20
- **Khởi thảo:** Mentor O (mổ repr Enum/Struct ở `triet-mir`/`triet-jit` + đối chiếu niche disc + tách Enum?/Struct? theo gốc rễ repr).
- **Chữ ký:** O ✅ (repr grounded, bất đối xứng Enum/Struct đo trực tiếp file:line; loại box/niche-fill có lập luận) · G ✅ (ký duyệt ADR + WO Lát 1, 2026-06-20) · Giang ✅ (chốt Phương án A + 2 lát Enum?→Struct?).
- **Liên quan:** [ADR-0062](0062-heap-nullable-ptr-sentinel-repr.md) (ptr-sentinel String/Vector/HashMap — ADR NÀY mở rộng "sentinel trong một ô" sang ô disc/tag; §6 ADR-0062 defer Struct?/Enum? chính là nợ ADR này trả) · [ADR-0041](0041-nullable-representation-bac-a.md) (scalar `T?` PA-3c `i64::MIN`; `NULL_SENTINEL` hằng + canary N1) · [ADR-0040](0040-heap-aggregate-layout.md) §4 (**B8** — heap field/payload trong aggregate bị refuse; rào chắn allocator của ADR này) · [ADR-0037](0037-enum-layout.md) (EnumLayout disc@0/payload@8) · [ADR-0060](0060-nested-aggregate-layout.md) (multi-word copy + nested offset-walk — tiền lệ cho tag-prepend của Struct?) · [ADR-0057](0057-jit-outcome-slot-move.md) (slot `{disc, payload}` + slot-move word-by-word — tiền lệ tag-word).

---

## 1. Context — nợ ADR-0062 §6, repr aggregate cần hoàn tất

`T?` đã chạy cho scalar (ADR-0041) và heap String/Vector/HashMap (ADR-0062). Cả hai dựa **một bất biến**: null ⟺ một **ô i64 mang con trỏ** == `NULL_SENTINEL` (`i64::MIN`). Aggregate (`Struct`/`Enum`) bị **defer minh bạch** ở ADR-0062 §6 vì "không có một ô `ptr` tự nhiên để cắm sentinel" — và `Body::verify()` (`triet-mir:1478-1519`) refuse chúng (`HeapNullableNotLowered`), comment `triet-mir:1397-1398` ghi rõ "stay refused until a later ADR". **ADR-0065 là cái ADR đó.**

Recon (đo từ code, không đoán) lộ ra **bất đối xứng quyết định mọi thứ**:

| Loại | Repr hiện hành (file:line) | Có ô để cắm sentinel? |
|---|---|---|
| **Enum** | `EnumLayout` disc**@0** (i64, 8B) + payload@8; `discriminant_value: i64` ∈ {0,1,2,…} (`triet-mir:1097-1120`, `compute():1158-1166`) | **CÓ** — ô disc@0 là i64 full, chứa giá trị nhỏ → **niche khổng lồ** |
| **Struct** | `StructLayout` = N field inline {offset,size}, KHÔNG disc, KHÔNG ô ptr (`triet-mir:1064-1090`) | **KHÔNG** — dữ liệu thuần, không ô thừa |

→ **Enum? = quả ngọt** (niche trong ô đã có). **Struct? = phải đẻ ô tag.** Hai gốc rễ khác nhau → một ADR (bức tranh toàn cảnh), hai lát thi công.

## 2. Decision

**Bất biến hợp nhất toàn hệ nullable** (mở rộng ADR-0062 §2):

> **`tag_cell == NULL_SENTINEL` (`i64::MIN`) ⟺ null.** `tag_cell` là:
> - ô `ptr` (heap — ADR-0062): String slot[0] / Vector·HashMap handle;
> - ô **`disc@0`** (Enum? — niche, ADR NÀY);
> - ô **`tag@0`** (Struct? — disc-word prepend, ADR NÀY).
>
> Null-check = **MỘT load + MỘT `icmp eq i64::MIN`** trên `tag_cell`. KHÔNG memcmp cả slot. CẤM `tag_cell == 0` làm null (0 = uninit/dead — defense-in-depth ADR-0041 §6.1).

### 2.1 `Enum?` — disc-sentinel (niche, 0 byte overhead)
Tái dùng ô disc@0 đã tồn tại. Discriminant thật ∈ {0,1,2,…} không bao giờ chạm `i64::MIN`. `disc@0 == i64::MIN` ⟺ null; else == variant thật. **Widening `Enum → Enum?` = NO-OP** (disc đã là giá trị hợp lệ ≠ sentinel). Đây CHÍNH LÀ ptr-sentinel ADR-0062, đổi ô `ptr` → ô `disc`.

### 2.2 `Struct?` — disc-word prepend (Phương án A, +8B)
Struct không có ô tự nhiên → **đẻ một tag word i64 ở offset 0**, đẩy field xuống +8:
`Struct?` slot = `{ tag@0 : i64, fields@8… }`, `total_size = struct.total_size + 8`.
`tag@0 == i64::MIN` ⟺ null; `tag@0 == +1` (`Trit::Positive`, theo cực ADR-0020 §10.1 "value") ⟺ present. **Widening `Struct → Struct?` KHÔNG no-op** — store tag + multi-word-copy field (tái dùng ADR-0060 §Điểm-3). Tag-word `{tag, payload}` y hệt slot Outcome ADR-0057 đã chạy.

## 3. Memory layout (G yêu cầu offset cụ thể)

### 3.1 `Enum?` — không đổi layout
```
offset:  0          8
        +----------+------------------+
slot:   |  disc    |   payload union  |    total = 8 + max_payload (như Enum thường)
        +----------+------------------+
        ↑ disc@0 ∈ {0,1,2,…} = variant | i64::MIN = null
   null-check = stack_load(I64, slot, 0) == i64::MIN ?  (TRƯỚC GetDiscriminant)
```

### 3.2 `Struct?` — +8B tag prepend
```
        Struct (hiện tại)            Struct? (ADR này, +8B)
offset: 0      8                     0      8      16
       +------+------+              +------+------+------+
       |  x   |  y   |   16B   →    | tag  |  x   |  y   |   24B
       +------+------+              +------+------+------+
                                    ↑ tag@0: i64::MIN=null | +1=present
   null-check = stack_load(I64, slot, 0) == i64::MIN ?
   present arm bind: inner struct sống tại slot+8; field x = load(slot + 8 + field.offset)
   (offset-walk +8, y hệt enum payload_offset=8 / Outcome OutcomePayload offset 8)
```

## 4. ⛔ RÀO CHẮN B8 — KHẮC ĐÁ, KHÔNG ĐƯỢC VƯỢT ⛔

> # 🔴 **AGGREGATE NULLABLE CHỈ CHỨA COPY FIELD/PAYLOAD.** 🔴
> # 🔴 **KHÔNG DROP GLUE. KHÔNG ALLOC. KHÔNG FREE. KHÔNG ĐỤNG ALLOCATOR.** 🔴

`Enum?`/`Struct?` trong ADR-0065 **CHỈ** áp cho aggregate mà mọi field/payload là **Copy** (scalar: Integer/Trit/Tryte/Long/Trilean/Unit + nested aggregate Copy). Heap field/payload (String/Vector/HashMap) **đã bị refuse** bởi **B8** (ADR-0040 §4) và **GIỮ NGUYÊN refuse** — `Body::verify()` `triet-mir:1500/1513` (`is_scalar_nullable_payload` cho field/payload) KHÔNG được nới.

**Hệ quả khắc cốt:**
- Copy-only → Drop của `Enum?`/`Struct?` = **no-op** → **0 drop-glue, 0 free-shim mới**.
- Inline stack slot → **0 allocation, 0 con trỏ heap** → **0 đụng allocator/GC**.
- Widening = copy giá trị stack (multi-word), KHÔNG alloc.

Bất cứ ai (kể cả D) chế thêm `String`-trong-`Struct?`, drop-glue, hay free-shim cho aggregate-nullable trong campaign này = **VƯỢT RÀO**, review chặn thẳng. Heap-trong-aggregate là **campaign riêng** (ownership/drop), KHÔNG phải ADR-0065.

## 5. Phương án bị loại (viết hết để khỏi ai đề xuất lại)

- **`Struct?` (B) Box trên heap:** `Struct?` = i64 handle → heap struct; null = handle SENTINEL. Tái dùng ptr-sentinel — **nhưng** biến Struct? thành Move/heap type, cần struct alloc/free shim + Drop glue + free-no-op-on-sentinel, **phá rào B8 §4**, đụng allocator, phá mô hình inline-stack. **LOẠI** (G chốt 2026-06-20: "tách biệt khỏi allocator ở phase này là nước cờ khôn ngoan").
- **`Struct?` (C) Niche-fill field đầu:** mượn `i64::MIN` của field scalar đầu (0 byte, no-op widening) — **nhưng** lowerer phải dò type-dependent tree tìm field có niche; struct rỗng / field đầu nested-struct → không có niche rõ; offset null-check phụ thuộc shape. **LOẠI** (G chốt 2026-06-20: "niche-fill là trò tối ưu khốn nạn nhất compiler non trẻ dây vào… CHỐT A, trả 8 bytes"). Ghi lại làm tối ưu tương lai (kiểu Rust), KHÔNG cho cut đầu.
- **`Struct?` (A) Disc-word prepend:** +8B/value, uniform, type-independent, 0-allocator, dễ poison. **CHỌN.**

## 6. Safety & Dispatch (G câu 3)

- **Check chèn ở đâu:** lowerer chèn null-check tại **match `~+/~0`** và **Elvis `?:`** — load `tag_cell` (Enum?: disc@0; Struct?: tag@0) → `icmp eq i64::MIN` → rẽ null vs present. Mẫu y hệt Elvis ADR-0041 §5.3 (`triet-lower:2566-2581`). Với Enum?, null-check chạy **TRƯỚC** `GetDiscriminant`+`SwitchInt` (`triet-lower` đuôi match ~3050-3180; mẫu "must run BEFORE the enum GetDiscriminant fallthrough" đã có cho Trit/Integer scrutinee `triet-lower:2927-2932`).
- **★ KHÔNG có hazard sentinel-deref → KHÔNG segfault:** Phương án A + Enum? đều **inline, KHÔNG con trỏ để deref**. Sentinel chỉ là tag/disc inline; khi null, vùng field/payload là **don't-care, KHÔNG BAO GIỜ bị đọc** vì match ép vào nhánh `~0` (E1026 exhaustiveness — ADR-0064 — bắt buộc nhánh `~0`). Hazard deref CHỈ tồn tại ở phương án B (box) — thêm một lý do loại B. Đây là lý do an toàn cốt lõi khiến A thắng cả về soundness.
- **Poison test (giá-trị-sai, KHÔNG cần SIGABRT):** poison lệnh store tag/disc-sentinel (ghi nhầm offset hoặc giá trị) → match rẽ sai nhánh → trả **giá trị sai observable** (mẫu ADR-0062 §8 sentinel-vs-zero: poison `triet-jit:1189` → trả `-9…808`). Teeth bắt bằng EXPECT số đúng.

## 7. Tác động GC/Allocator (G câu 4) — **KHÔNG**

- **KHÔNG có GC** (move-only, refcount tắt Bậc A — ADR-0040 §1.2).
- Do rào B8 §4 (Copy-only) → Drop = no-op → **không có "thằng dọn rác" để dạy phân biệt sentinel vs con trỏ thật.** Không có con trỏ heap nào trong aggregate-nullable cut này.
- Phương án A + Enum? = inline stack → **0 đụng allocator.** (Đây là toàn bộ lý do gộp A vào ADR này thay vì B.)

## 8. Scope + 2 lát thi công

**TRONG scope:** `Enum?`/`Struct?` với aggregate **Copy-only** (B8 §4). Constructs (mirror ADR-0041 §7 + ADR-0062 §7): `~0` (null materialize sentinel vào tag_cell), widening `T → T?`, match `~+/~0`, Elvis `?:`. Return + local position.

**NGOÀI scope — defer:** heap-trong-aggregate (B8 giữ refuse); `?+>` map/flatMap trên aggregate-nullable; `T?~E` (Outcome aggregate); nested `Struct?`-field-trong-`Struct` (field-level nullable aggregate — nếu cần, lát riêng).

**Hai lát (G chốt 2026-06-20):**
- **Lát 1 — `Enum?` (quả ngọt, 0 byte):** niche disc@0. Gate `is_lowerable_nullable_payload` (`triet-mir:1399`) += `MirType::Enum(_)`. `~0`/match/Elvis null-check trên disc@0. Widening = no-op.
- **Lát 2 — `Struct?` (tag 8B, Phương án A):** gate += `MirType::Struct(_)`; layout +8B tag prepend; widening = tag-store + multi-word-copy (ADR-0060); match present-arm bind tại slot+8 (offset-walk).

## 9. Rủi ro + teeth bắt buộc

- **Sentinel-vs-disc collision (Enum?):** PHẢI chứng minh không discriminant thật nào == `i64::MIN`. Discriminant gán {0,1,2,…} (`VariantLayout:1119` "0, 1, 2, …") — an toàn. Teeth: enum nhiều variant + match present (mỗi variant) + match null.
- **Tag-offset (Struct?):** field bind sai offset (+8) → đọc đè tag / data-loss. Teeth: `Struct?` present-arm đọc field → giá trị đúng; poison bỏ +8 offset-walk → giá trị sai.
- **Widening Struct? (Lát 2):** multi-word-copy thiếu word → field sau mất. Teeth: struct ≥2 field, widen rồi đọc cả hai. Poison copy 1-word → field sau = rác.
- **`~0` materialize:** poison store sentinel (offset/giá trị) → match rẽ sai → giá trị sai (mẫu ADR-0062 §8).
- **Blind-spot rule:** teeth phải quét **cả Enum? LẪN Struct?** (Lát 2), và **cả nhánh present LẪN null** (mỗi nhánh một mặt trận — bài học HP.3).
- **B8 guard (regression):** teeth khẳng định `String`-trong-`Struct?` / `Vector`-trong-`Enum?` payload VẪN refuse `HeapNullableNotLowered` (fixture âm). Poison: nếu ai nới gate scalar→heap cho field/payload, fixture âm phải đỏ.

### 9.1 Amendment (Lát 1, 2026-06-20) — HAI cổng refuse B8 khác mã lỗi (O đo, sửa-có-dấu-vết)

Verify Lát 1 lộ ra B8 refuse qua **hai cổng phân biệt** — teeth phải nhắm đúng cổng của lát này:
- **`Enum?` với payload nullable-heap** `Has(String?)` → `Body::verify()` enum-payload gate (`triet-mir:1500/1513`, giữ `is_scalar_nullable_payload`) → **`MirError::HeapNullableNotLowered`** "at enum payload `Bag.Has`". **ĐÂY là guard của ADR-0065** (fixture âm 230). Nới gate scalar→heap cho field/payload → fixture 230 phải đỏ.
- **`Enum` với payload plain-heap** `Has(String)` (không nullable) → chặn sớm hơn ở is_copy construction gate (ADR-0040 **B8 pre-existing**, `triet-lower`) → `LowerError "heap types not supported"`. **Orthogonal** — không phải guard lát này; chỉ bắn khi CONSTRUCT variant.

Teeth B8 của Lát 1 dùng `String?` payload (cổng `HeapNullableNotLowered`), KHÔNG dùng plain `String` (cổng khác). §9 bullet "B8 guard" ở trên trỏ đúng cổng `HeapNullableNotLowered`.

## 10. Consequences

- (+) Hoàn tất hệ nullable cho mọi loại; bất biến `tag_cell == i64::MIN ⟺ null` hợp nhất scalar/heap/enum/struct.
- (+) Enum? = 0 byte, 0-allocator. Struct? = +8B, 0-allocator. Value-model i64 **KHÔNG đụng** (leaf load vẫn I64; chỉ mở rộng slot-layout, cùng họ Outcome/nested-aggregate).
- (−) Struct? +8B overhead/value (G chấp nhận: "RAM rẻ, não không rẻ để debug type-dependent offset").
- **Đóng băng:** heap-trong-aggregate + drop-glue defer minh bạch (B8 §4) — không hứa, không skeleton dead-code.

## 11. Migration

| Mốc | Việc | Repr đổi? |
|---|---|---|
| Tương lai: niche-fill (C) | Struct? bỏ tag-word cho struct có niche field → 0 byte | Có thể, cục bộ, ADR mới |
| Heap-trong-aggregate | Drop-glue + free-shim cho aggregate chứa Move field | Campaign riêng |
| Bậc C packed ABI | tag-word có thể hợp nhất với Outcome disc | Có thể, cục bộ |
