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

### 9.2 Amendment (Lát 2, 2026-06-20) — widening `Struct → Struct?` PHẢI sinh Assign (KHÔNG retype in-place)

Verify Lát 2 lộ ra một giả định sai trong WO gốc (recon-miss của O, vá in-scope, β/B8 không đổi):

- **Cơ chế lowerer thật:** `let x: T? = y` ở `triet-lower/src/lib.rs:1207` mặc định **retype local của `y` tại-chỗ** (`local_decls[v].ty = ann_ty`) + alias — KHÔNG sinh `Assign`. Đây CHÍNH là lý do widening Lát 1 (`Enum → Enum?`) là **no-op**: niche disc@0 dùng chung slot, relabel là đủ.
- **Tại sao Struct? phá:** `Struct?` phình +8B (tag prepend). Retype in-place giữ slot `StructAlloc` 16B (x@0,y@8) nhưng dán nhãn `Nullable(Struct)` → `walk_projections +8` đọc OOB → giá trị rác (fixture 231 trả 6 thay 7). TODO sẵn tại `1200-1206` đã tiên tri đúng ca này ("emit an Assign to a new typed local (M2 pattern) instead of mutating").
- **Delta 0 (sửa):** khi `init_ty == Struct(_)` **và** `ann_ty == Nullable(Struct(_))` → alloc local `Nullable(Struct)` MỚI + `Assign{new ← v}` (M2 pattern), KÍCH nhánh JIT widening (Delta 4a). **Khoanh chặt** chỉ `Struct→Struct?`; Enum?/scalar/String? giữ in-place (fixture 229 xanh nguyên).
- **Teeth tag-store (P3):** store `tag=present(1)` trong 4a load-bearing nhưng fixture slot-tươi KHÔNG bắt (uninit tình cờ ≠ MIN). Cần fixture **reassign-widen-over-null** (237: `let mutable n: Pt? = ~0; n = p;` trên slot từng giữ `~0`=MIN): bỏ store → tag cũ MIN sống → match nhầm `~0` → đỏ. §9 bullet "`~0` materialize" mở rộng: teeth widening-tag PHẢI dùng slot tái-dùng-null, không dùng slot tươi.

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

---

## 12. AMENDMENT (Lát 3, 2026-06-20) — Nested Nullable Aggregate of Copy (Trục A)

**Bối cảnh:** §8 ghi "NGOÀI scope — defer: nested `Struct?`-field-trong-`Struct`". §4/§9 đã khóa
B8 (Copy-only). Lát 1 (`Enum?`) + Lát 2 (`Struct?`) chỉ chạy ở vị trí **top-level** (return/local).
Khi `Struct?`/`Enum?` nằm **làm field/payload của một aggregate khác** (`struct Holder { p: Point? }`),
ba tầng chặn: gate field/payload (`is_scalar_nullable_payload`, scalar-only), sizing fixup (chỉ map
`Struct/Enum`, không `Nullable(Struct/Enum)` → field `Point?` rơi default 8B = SAI), offset-walk
(`walk_projections` không unwrap `Nullable` mid-walk → `h.p.x` fail "field access on non-aggregate").
**Lát 3 (Trục A)** mở Ca 1 — `struct Holder { p: Point? }`, Point all-scalar — thuần **layout math**,
KHÔNG allocator, KHÔNG drop-glue.

### 12.1 Cơ chế (kế thừa Lát 1/Lát 2, áp ở vị trí field/payload)
- **`Nullable(Struct)` field** = `inner.total_size + 8` (tag-word prepend @field-offset, y hệt top-level
  Lát 2 §2.2). Tag@field-offset == `i64::MIN` ⟺ null; == `+1` ⟺ present. Field thật của Point sống tại
  `field-offset + 8 + Point.field.offset`.
- **`Nullable(Enum)` field** = `inner.total_size` (disc-niche, **0 byte** overhead, y hệt Lát 1 §2.1).
  disc@field-offset == `i64::MIN` ⟺ null.
- **Offset-walk**: khi `walk_projections` đi VÀO một field kiểu `Nullable(Struct)` → cộng tag-shift +8
  rồi unwrap về `Struct`; `Nullable(Enum)` → +0 rồi unwrap về `Enum`. Tái dùng tinh thần
  `nullable_struct_base_offset` (struct→8/enum→0) nhưng áp **mid-walk** thay vì chỉ ở base.

### 12.2 Điều kiện Copy — KHẮC ĐÁ (nếp gấp soundness)
Chỉ áp khi `inner.is_copy(Some(body))` (`triet-mir:666` — đã đệ quy + body-aware: chui field/variant).
- Heap-trong-nested-nullable (`struct Bad { s: String }`, `Holder2 { p: Bad? }`) → `Bad.is_copy = false`
  → gate field/payload **GIỮ refuse** `HeapNullableNotLowered`. **B8 §4 NGUYÊN.**
- `Nullable(String/Vector/HashMap)` field → inner KHÔNG phải Struct/Enum → refuse (B8 nguyên).
- **Cảnh báo (G khắc đá):** gate field/payload PHẢI **body-aware**. Nới gate nhận `Nullable(Struct/Enum)`
  **thuần cấu trúc** (không check `is_copy`) = chỗ B8 lọt → `Bad?` field copy String bytes như Copy =
  double-free/leak tiềm ẩn. `find_refused_nullable` hiện cầm `allow: fn(&MirType)->bool` KHÔNG thấy body
  → cơ chế nới phải tải body vào nhánh field/payload (đổi sig hoặc path riêng).

### 12.3 Layout math — offset đệ quy lồng
Tag-word lớp ngoài (nếu aggregate ngoài cũng nullable) + tag-word lớp trong cộng dồn; padding 8-align.
Ca 1 (aggregate ngoài KHÔNG nullable):

```
struct Point  { x: Integer, y: Integer }      → Point.total  = 16  (x@0, y@8)
                Point?                          → Point?.total = 24  (tag@0, x@8, y@16)  [+8 tag]
struct Holder { p: Point? }                    → Holder.total = 24  (p@0)

  Holder slot:  offset 0      8      16
                +------+------+------+
                | tag  |  x   |  y   |     p@0 → tag@0, Point.x@8, Point.y@16  (tuyệt đối)
                +------+------+------+
  đọc h.p.x = load(slot + p.offset(0) + tag-shift(8) + Point.x.offset(0)) = load(slot+8)
  đọc h.p.y = load(slot + 0 + 8 + 8)                                       = load(slot+16)
```

`Nullable(Enum)` field: 0-byte tag → field-offset không dịch (disc@field-offset chính là niche).

### 12.4 KHÔNG drop-glue, KHÔNG allocator
Copy-only (§12.2) → Drop = no-op → 0 drop-glue, 0 free-shim, 0 đụng allocator (kế thừa §4 + §7 + §9).
Widening `Holder{ p: ~+ Point{...} }` = store tag + multi-word-copy Point fields (tái dùng Lát 2 §2.2
+ ADR-0060 §Điểm-3), KHÔNG alloc.

### 12.5 ⚰️ Phán quyết Trục B — SỔ TỬ THẦN (campaign VISION riêng)
**Heap-trong-aggregate + recursive drop-glue = Trục B = campaign VISION RIÊNG, KHÔNG phải ADR-0065.**
ADR cho Trục B còn **trắng** — chưa viết một dòng. **B8 (§4) khóa chặt mọi heap-in-aggregate field-offset
bất kể nullable.** ADR-0065 (kể cả §12 này) KHÔNG có một chữ nào ngụ ý Trục B được chạm tới trong campaign
này. Ai chế heap-in-nested-nullable, drop-glue, hay free-shim cho aggregate-nullable = VƯỢT RÀO, review
chặn thẳng.

### 12.6 Teeth bắt buộc (O verify máu từng lát)
- **Gate body-aware (Lát 2 WO):** poison nới gate thành thuần-cấu-trúc (bỏ `is_copy`) → fixture `Bad?`-heap-field
  KHÔNG còn refuse → ĐỎ (chứng minh Copy-check load-bearing chống B8-lọt). Control: `Nullable(String)` field vẫn refuse.
- **Sizing (Lát 3 WO):** poison bỏ `+8` cho Nullable(Struct) field → Holder.total sai → walk OOB → đọc rác/SIGSEGV ĐỎ.
- **Offset-walk (Lát 4 WO):** poison tag-shift nested 8→0 (hoặc 8→16) → `h.p.x` đọc lệch byte → giá trị sai/SIGSEGV ĐỎ.
- **Construction + read-back (Lát 5 WO):** read-back `h.p.x` BẮT BUỘC (construct-only không tính); fixture
  **field-kế-cận** (struct 2 field + Point? sau, đọc field-sau-nested) chứng minh +8 KHÔNG đạp field sau.
- **B8 regression:** fixture âm `Bad?` (heap-in-struct) + `Nullable(String)` field VẪN refuse `HeapNullableNotLowered`.

### 12.7 Construction Taxonomy (re-scope 2026-06-20 — sửa-có-dấu-vết, G ký Option a)

**Recon-miss của WO gốc (O tự nhận):** §12.4 ghi "tái dùng widening Lát 2 §2.2 (Delta 4a)". SAI. Delta 4a/4b
JIT (`mir_lower.rs:1375/1418`) **gate `projection.is_empty()` CẢ HAI bên** → chỉ chạy cho top-level
`let x: Struct? = y`. Construction (`_0.p = move v` — dest projected) + read-back (`_2 = move h.p` — source
projected) **KHÔNG BAO GIỜ** chạm 4a/4b → rơi general-copy. **Field-position construction chưa từng được
implement.** Đây là lỗ hổng lõi, không phải bug vặt.

**Ba bug (O trace tới MIR):**
- **Bug A — JIT base-downcast nuốt tag** (`walk_projections:297`): `nullable_struct_base_offset` bake `+8`
  cho mọi `Nullable(Struct)` base. `load_place`/`store_place` empty-proj đọc thẳng `slot@0` (KHÔNG gọi walk
  → top-level match 231-237 ĐÚNG), NHƯNG Assign-copy gọi walk cả src+dest → whole-slot move bị +8 → **bỏ qua
  tag@0** → null trả rác. Blast-radius khi gỡ = **HẸP, chỉ Assign-copy**.
- **Bug B — Lowerer `~+ aggregate` rẽ Outcome** (`lib.rs:1557`): `~+ Point` → `OutcomeAlloc` với
  `outcome_ty = return_type` (Integer của main) → `OutcomeAlloc non-Outcome Integer`. `~+` thuần Outcome,
  không có nhánh nullable-present.
- **Bug C — Lowerer implicit field-widen không set tag** (`lib.rs:2920`): `Point{..}` → plain Struct →
  `_0.p = move _1` plain Assign, KHÔNG widen, KHÔNG SetTag → present **pass-by-luck** (tag uninit tình cờ ≠ MIN).

**Giải pháp (Option a — faithful walk + Taxonomy 4-case):** bỏ base-downcast khỏi `walk_projections` (làm nó
**faithful** — offset thật, type `Nullable(Struct)` nguyên). Đưa quyết định downcast/widen/whole-copy vào
**chốt Assign-copy**, phân xử theo `(src_ty, dest_ty)` SAU faithful-walk:

| dest \ src | plain `Struct` | `Nullable(Struct)` |
|---|---|---|
| **plain `Struct`** | general copy (cũ, ADR-0060) | **case 3 DOWNCAST**: copy fields `src+8 → dest+0` (= match-bind `pt = scrut`) |
| **`Nullable(Struct)`** | **case 2 WIDEN**: set `tag=1@dest+0`, copy fields `src+0 → dest+8` (= 4a cũ + field implicit) | **case 1 WHOLE-COPY**: `N+8` bytes, **tag@0 FIRST**, `src_off → dest_off` (= 4b cũ + construction + readback) |

**5 điểm khắc đá:**
1. **Faithful-walk:** `walk_projections` trả offset thật (base bare-`Nullable(Struct)` KHÔNG +8); giữ Lát-4
   `nested_nullable_shift` cho field-INTO-nullable mid-walk.
2. **Subsume:** Taxonomy gộp Delta 4a (→ case 2) + 4b (→ case 1). **XÓA 4a/4b cũ**, không giữ song song.
   Downcast +8 (cũ bake mù trong walk) nay là hành vi **tường minh** của case 3.
3. **Tag bất biến NGUYÊN:** `{tag@0, fields@8}`, `tag@0==MIN ⟺ null`. Case 1 copy **tag-first** → preserve verbatim.
4. **Enum? field analog:** +0 (niche), tag = `disc@0 == MIN`.
5. **Copy-only:** KHÔNG drop-glue/allocator. Heap (Trục B) = sổ tử thần, B8 §4 khóa, CẤM chạm.

**Lowerer (Lát 3'):** `~+ inner` ở struct-field khi `field_ty == Nullable(Struct/Enum)` → lower `inner` plain
(KHÔNG đi `OutcomeConstructor`); field Assign tự widen qua **case 2**. Bug C implicit KHÔNG cần sửa lowerer —
case 2 JIT tự widen plain Assign. `~+` top-level (`let x: Struct? = ~+ y`) nếu vỡ → **ghi nợ tech-debt, NGOÀI
scope** (G chốt tách).

**Teeth (re-scope):**
- **LOCKED 231-237 XANH NGUYÊN** (lưới regression — case 1/2/3 subsume đúng).
- Poison case 1 (whole-copy → cố +8 downcast) → readback-null lệch → null-fixture ĐỎ.
- Poison case 2 (bỏ set-tag=1) → present mất tag → present-fixture ĐỎ. **PHẢI fixture quan-sát-được, KHÔNG
  pass-by-luck** (bài học Lát 2 P3 + reject Lát 5).
- Poison case 3 (bỏ +8) → match-bind `pt.x` đọc tag thay field → 231-237 + present ĐỎ.
- Poison `~+`-special-case → `Holder{p:~+ Point{...}}` lại `OutcomeAlloc` ĐỎ.
- **⚔ field-kế-cận** (`struct H2{a, p:Point?, z}`): construct rồi đọc `z` → tag-8B + nội dung lồng KHÔNG
  đạp địa chỉ `z` phía sau (offset verify bằng sizing-fixup + walk, KHÔNG tin số gợi ý).

**Chữ ký §12:** (chờ O verify máu + ký; G ký đóng — D KHÔNG tự điền)

### 12.8 Amendment (WO-~+-NULLABLE-UNIFY, 2026-06-21) — `~+ nullable-present` triệt để: top-level `let` + field scalar

**Bối cảnh:** §12.7 đóng field-position construction cho aggregate (`Struct?`/`Enum?`) nhưng ghi nợ
**hai đường sống** còn lại của Bug B (`OutcomeAlloc on non-Outcome type 'T?'`):
- **Ổ 1 — top-level `let x: T? = ~+ v`:** `~+` lower thẳng `OutcomeConstructor` → `outcome_ty =
  return_type` (Integer của `main`, non-Outcome) → `OutcomeAlloc` rác. Chết cả scalar / Struct / Enum
  (`Integer?`/`Point?`/`Color?` — O probe 3 cùng lỗi).
- **Ổ 2 — field scalar `Holder{f: ~+ 5}` với `f: T?` scalar:** gate §12.7 chỉ nhận `Nullable(Struct|Enum)`
  → scalar rơi else → cùng `OutcomeAlloc`. (Field `Struct?`/`Enum?` đã chạy §12.7 — fixture 247/249, KHÔNG đụng.)

**Quyết định — LOWERER-ONLY, tái dùng 100% đường widening, KHÔNG ADR mới:**

- **Fix 1 (top-level let, `lib.rs` đầu nhánh else ~1210):** trước `lower_expr(*init)`, nếu `init` =
  `OutcomeConstructor{ arm: Positive, payload: Some(inner) }` VÀ annotation lower ra `MirType::Nullable(_)`
  → lower `*inner` (plain payload) THAY `*init`. Khối widening sẵn có (Lát 2 Delta 0) gánh tiếp:
  - `Nullable(Struct)` → `is_struct_widening` → Assign fresh → JIT taxonomy **case 2 WIDEN** (chứng minh: 252).
  - `Nullable(Enum)` → retype in-place → **niche disc@0** (253, mirror 229/225).
  - `Nullable(scalar)` → retype in-place → **PA-3c no-op** (251).

  KHÔNG nhánh-hóa theo type — cả 3 chảy qua widening đã có răng Trục A. Mirror đối xứng redirect
  field-position §12.7 (StructLiteral).

- **Fix 2 (field gate, `lib.rs` StructLiteral ~2940):** nới `field_is_nullable_agg`
  (`Nullable(Struct|Enum)`) → `field_is_nullable = matches!(field_decl_ty, Some(Nullable(_)))`. Scalar
  `~+ 5` → lower `inner=5` plain → field Assign store i64 (scalar nullable: **value IS repr**, present
  5 ≠ MIN). **B8 NGUYÊN:** `is_copy` check chạy SAU mọi nhánh — `f: String?` set `~+ "hi"` → inner String
  → `is_copy` false → refuse (fixture 255).

**Teeth (O verify máu — 3 răng đỏ độc lập, mỗi ngã rẽ một răng):**
- **P1** tắt redirect Fix 1 → **251+252+253 cùng `OutcomeAlloc on non-Outcome 'Integer?'/'Point?'/'Color?'`**
  → chứng minh redirect load-bearing CẢ 3 type.
- **P2** revert gate Fix 2 về `_agg` → **254 `OutcomeAlloc on non-Outcome 'Integer'`** (cô lập field-scalar).
- **P3** nới `is_copy` cho String → **255 đỏ** (refuse "heap types…" biến mất). B8 có **defense-in-depth 2 lớp**:
  `is_copy` (lowerer, message fixture 255 pin) lớp 1 + verifier `heap-nullable T? not yet lowered` lớp 2.
- Widening per-type (case 2 / niche / PA-3c no-op) đã có răng Trục A (231/229/249) — KHÔNG poison lại.

**⛔ NGOÀI SCOPE — defer (G chốt tách, "Separation of Concerns"):** direct `match h.f` trên **scalar-nullable
FIELD** chết ở `unsupported match pattern (expected enum variant)` — đây là gap **luồng ĐỌC** (field-read temp
Unknown-typed, `lib.rs:2904-2911`, cố ý giữ scalar-leaf-as-i64 cho số học), KHÁC HẲN Bug B (luồng GHI). Fix
nó = nới field-read typing 2904, blast-radius CHƯA đo trên 245+ fixture → **Sổ Nợ Kỹ Thuật, KHÔNG mở WO-2 lúc
này**. Fixture 254 đọc qua `let y: Integer? = h.f` (typed-let widen Unknown→Nullable) làm cầu nối nghiệm thu
luồng GHI.

**Hệ quả:** sau Fix 1+2, KHÔNG còn site nào `~+` đẻ `OutcomeAlloc-on-non-Outcome`. Chuỗi Nullable Aggregate
construction đóng trọn (top-level + field, scalar + aggregate). `~+` thuần Outcome (`T~E`/`T?~E`) KHÔNG đổi
hành vi (annotation non-Nullable → redirect không kích → lower `OutcomeConstructor` bình thường).

**Chữ ký §12.8:** O: ✅ (verify máu — P1/P2/P3 đỏ độc lập, mỗi ngã rẽ một răng; gate `0·0·250·0`; diff lowerer-only sạch; B8 2-lớp nguyên) · G: ✅ (ký đóng 2026-06-21 — WO-~+-NULLABLE-UNIFY).

## 13. HOTFIX (2026-07-15, O recon on `9a1799c`) — payload-bearing `Enum?` REFUSED, disc-niche is unit-only-only

**Máu O:** disc-niche §2.1/§12.7 was validated on **unit-only** enums (`Color{Red,Green,Blue}`, 8B — disc@0
IS the whole value). It was never proven for an enum with a **payload-bearing variant** (>8B: disc@0 +
payload@8…). When `E?` is used as a **function's own return type**, the single-i64 return ABI truncates the
aggregate crossing the call boundary: the caller receives a corrupted discriminant, the enum drop-glue
`SwitchInt` on that garbage value falls to `default` → `Trap` → **SIGILL (exit 132)**. Reproduced for BOTH an
aggregate payload (`enum E{V(Big),N}`, `Big{p,q}` two `Integer` fields) and a scalar payload
(`enum E{V(Integer),N}`) — not aggregate-specific.

**Fix (surgical, lowerer-only, one chokepoint):** `Expr::OutcomeConstructor`'s `Nullable` branch
(`crates/triet-lower/src/lib.rs`, guards both the `~+` and `~0` arms) now refuses, at construction time,
any `E?` where `E`'s `EnumLayout` has at least one variant with `payload.is_some()`. This is a **structural**
refuse — it fires at every `E?`-value construction site (top-level `let`, function `return`, struct field),
not only the position that was proven to crash (function-return). Per refuse-over-guess: the disc-niche
machinery for payload-bearing enums is unproven outside the return-ABI hazard, so the whole surface is
closed rather than special-cased to "only refuse in return position."

**NOT fixed here (front deferred):** a proper `Enum?` repr for payload-bearing enums (e.g. a real disc-niche
marshal across the return ABI, or falling back to the `Struct?` +8B tag-word scheme) — tracked as new debt
"nullable-enum-payload niche marshal" pending a future slice. `Struct?` (§2.2/§3.2, +8B tag prepend) is
UNCHANGED and out of scope for this hotfix.

**Regression:** unit-only `Enum?` (§12.7 taxonomy, fixtures 249/250) is untouched — the refuse predicate only
fires when `payload.is_some()` for some variant; a unit-only enum's `EnumLayout` has `payload: None` on every
variant, so the guard never trips for it.

**Teeth:** fixtures 374 (aggregate payload, function-return shape, proven poison-red exit 132) / 375 (scalar
payload, same shape, proven exit 132) / 376 (struct-field construction path — refuse proven, crash NOT
independently reproduced for this exact shape; refused structurally regardless) / 377 (unit-only, local `let`
— non-vacuous negative control, still compiles+runs).

**Chữ ký §13:** D (Sonnet 5): implemented + poison-red verified (374 only, per WO) · chờ O verify + G ký.
