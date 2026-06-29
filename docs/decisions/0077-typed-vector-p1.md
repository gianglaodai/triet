# ADR 0077 — Typed Vector P1 (element-type qua type-erasure, built-in element only)

> # 🩸 NGUYÊN LÝ CỐT LÕI (G khắc đá 2026-06-30)
> # Một ngôn ngữ KHÔNG cho bỏ một `String` vào một `Vector` là ngôn ngữ **vứt đi**.
> # Ownership đã khép; mũi giáo kế đâm thủng phòng tuyến **Type-Erasure** của
> # collection. Element-SIZE của MỌI built-in là HẰNG SỐ compile-time → KHÔNG cần
> # native-layout. UserStruct/Enum element bị KHÓA MÕM (E-code) ở biên P1 — đó là
> # cây cầu sang native-layout phase sau, KHÔNG dây dưa vào đây.

**Trạng thái:** 📝 **DRAFT — chờ implement + O verify máu + G sign-off.** Áp dụng Bậc C+.
Mở `Vector<T>` với T = built-in (scalar / String / Vector / HashMap / Nullable tương ứng);
**REFUSE `Vector<UserStruct>` / `Vector<Enum>`** by-value (→ P2, đòi native-layout ADR sau).
Continuation hữu cơ của heap-aggregate (ADR-0066/0067/0076) — tái dùng cỗ máy tombstone/free.

**Sibling/kế thừa:** ADR-0066/0067 (No-Box heap-in-aggregate, `collect_heap_leaves`/drop-glue),
ADR-0076 (heap-`T?` field — sentinel-no-op free, R4), ADR-0060 (P1/P2 tách-tầng pattern).
**KHÔNG đụng:** ADR-0068 (Box/recursive — CẤM CỬA), native multi-field layout (Option D — defer).
HashMap<K,V> = **campaign RIÊNG sau** (2 element-type K+V, slot 24B, chưa có dedicated
`Type::HashMap(K,V)` typecheck) — G chốt cắt khỏi đợt này.

---

## Issue

`Vector` ở backend là **bare / Integer-only**. Typecheck CÓ `Type::Vector(Box<Self>)`
(`types.rs:40`) nhưng lower **erase** element-type về `MirType::Vector` bare
(`lib.rs:975/1018/1082/1119` — `"Vector"`/`starts_with("Vector<")` → bare). Hệ quả:
`push(vector_new(), "hi")` → typecheck REFUSE (`expected Integer, found String`). Ngôn ngữ
không có collection chứa String/heap. Ba điểm coupling khóa cứng ở Integer (đo file:line):

1. **STRIDE hardcode 8:** `vector_layout` `HEADER+8+8+cap*8` (`mir_lower.rs:3259`); push
   `old_len*8` + `(new_data as *mut i64).add(old_len)` (`3375/3377`); get `(data as
   *const i64).add(idx)` (`3416`).
2. **DROP-GLUE element-blind:** `__triet_vector_free` (`3305`) chỉ `dealloc(block)` —
   KHÔNG loop free element. Vector<String> = **máy bơm leak** (mỗi element String rò).
3. **ELEMENT-ABI 1 i64:** `push(vec, elem: i64)` (`3347`), `get → i64` (`3401`). String fat
   24B không lọt 1 register.

---

## Quyết định

Mở **Typed Vector P1** = `Vector<T>` cho T **built-in known-size**, qua 4 mũi liên động.

### Ranh giới P1/P2 (crux — cắt sạch khỏi native-layout)
- **P1 (built-in, element-size HẰNG):** T ∈ {Integer, Trit, Tryte, Long, Trilean, String,
  Vector\<_\>, HashMap\<_\>, Nullable\<những cái đó\>}. **`Vector<Vector<String>>` nested
  CŨNG P1** — element = handle 8B, inner-size vô can.
- **P2 (đòi native-layout):** `Vector<UserStruct>` / `Vector<Enum>` by-value (element-size =
  struct layout tùy ý). **REFUSE bằng E-code mới ở P1** — không silent, không panic. Đây là
  biên chặn dây dưa sang Option D.

Tách-tầng đứng vững vì P1 chỉ cần element-size cho **built-in (hằng 8/24)** + memcpy size-biết
+ free-shim per-kind. KHÔNG walk struct field, KHÔNG pack register. **Vector P1 ⊥ native-layout.**

### Mũi 1 — Element-type vào MIR: `MirType::Vector` → `Vector(Box<MirType>)`
Mirror typecheck `Type::Vector(Box<Self>)`. **Blast ~25 site** match bare (mir/lib.rs 14 ·
lower/lib.rs 10 · jit/mir_lower.rs 1 · borrowck 0) — bounded, mechanical (như `Nullable(inner)`
ADR-0062). Erasure point sửa ở lower (`975/1018/1082/1119`): `Vector<E>` → `Vector(Box::new(
lower(E)))`; bare `"Vector"` (no arg) → giữ tương thích = `Vector(Box::new(Integer))` (Bậc A
default) HOẶC E-code thiếu-annotation (implementer-choice D, ghi lý do).

### Mũi 2 — `elem_size(MirType) -> usize` (hằng compile-time)
scalar/handle/Nullable(scalar) = 8 · String/Nullable(String) = 24 · Vector/HashMap handle = 8 ·
**Struct/Enum → REFUSE (E-code P1, không trả size).** ⚠️ KHÔNG tái dùng `ty_total_size`
(jit:483) — nó trả 8 cho String (sai cho stride 24). Helper RIÊNG.
Shim đổi `*8` → `*stride`, `.add(idx)` → byte-offset `idx*stride` trên `*mut u8`.

### Mũi 3 — Typed drop-glue: `__triet_vector_free_typed(ptr, elem_kind, stride)`
Loop `len` element @stride; mỗi element heap → gọi free-shim theo `elem_kind`
(0=scalar/no-drop · 1=String · 2=Vector · 3=HashMap; Nullable(heap) cùng kind, sentinel-no-op).
**Tái dùng NGUYÊN free-shim + sentinel-no-op (R4 ADR-0076)** — ptr element ∈ {ptr→free,
0/NULL_SENTINEL→no-op}. Drop-glue site JIT (`mir_lower.rs` Drop arm cho Vector) đổi
`__triet_vector_free` → typed variant + truyền elem_kind/stride từ `Vector(inner)`.

### Mũi 4 — By-pointer ABI cho fat element
`push`/`get` với element fat (String 24B): pass **by-pointer** (push nhận `*const elem`,
memcpy `stride` byte; get trả qua sret/out-ptr). Scalar/handle (8B) giữ by-value i64 (fast
path, backward-compat Vector<Integer>). By-pointer ⊥ native-layout (size là const biết trước).

---

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Inline element by stride** (chọn) | 1 alloc/vector, cache-local, đối xứng struct-field | shim cần stride-param + by-ptr ABI cho fat | **CHỌN** — element-size built-in là hằng → tách native-layout |
| 2 | Box mọi element (uniform 8B ptr) | stride luôn 8, ABI luôn i64 | +1 alloc/element, +1 indirection, drop = free box rồi free inner | Bác — phí allocation, nghịch value-semantics |
| 3 | Gộp HashMap cùng ADR | 1 lần xong | K+V 2 type, slot 24B, chưa có typecheck variant | Bác (G chốt) — campaign riêng sau |
| 4 | Mở luôn Vector<UserStruct> | tổng quát | = native-layout (Option D đại phẫu) | Bác — REFUSE ở biên P1, cầu sang P2 |
| 5 | `Vector` bare giữ, side-map element-type | 0 đụng MIR variant | side-channel = mầm ung thư (bài học ADR-0072) | Bác — element-type vào MIR tường minh |

---

## Hậu quả

### Tích cực
- Collection chứa String/heap/nested → data-structure thật, thủng Type-Erasure.
- Element-type tường minh trong MIR (không side-channel) — nền cho HashMap<K,V> + iteration sau.
- Tái dùng tombstone/free machinery (continuation, 0 cỗ máy mới ở drop).
- ⊥ native-layout — không mở Option D.

### Tiêu cực
- `MirType::Vector(Box)` chạm ~25 site (mechanical).
- By-ptr ABI cho fat element thêm path (bounded — scalar giữ fast i64).

### Rủi ro cần mitigate
- **Drop-glue element-blind P1** → leak khổng lồ nếu free không loop. Teeth bắt buộc (Vector<String> drop → FREE==n).
- **Stride sai** → đọc/ghi lệch element → SIGSEGV / corruption. Teeth: push nhiều String, get đọc lại đúng.
- **REFUSE UserStruct lọt** → element-size struct tùy ý → dây sang native-layout. Negative tooth khóa E-code.
- **moved-out element / sentinel** → double-free. Teeth pop-then-drop FREE đúng số.

---

## Teeth (O verify máu độc lập — poison phải đỏ, restore cp KHÔNG git checkout)

| # | Tooth | Scenario | Poison → RED |
|---|---|---|---|
| 1 💀 leak | Vector<String> push 3 → drop cả mảng | gỡ typed-free loop → FREE==0 (leak), không phải 3 |
| 2 💀 double-free | push String, **pop**, drop mảng (G mandate) | tombstone pop sai → FREE==2 / SIGABRT 134 |
| 3 stride | push 3 String, get[0/1/2] đọc lại đúng nội dung | stride giữ 8 → đọc lệch → sai/SIGSEGV |
| 4 negative | `Vector<MyStruct>` (UserStruct element) | bỏ E-code → element-size struct → lọt P2/native-layout |
| 5 backward-compat | Vector<Integer> cũ (72 fixture corpus) | regression nếu fast-path i64 vỡ |
| 6 nested P1 | `Vector<Vector<String>>` (element handle 8B) | inner-drop sai → leak inner String |

Mỗi tooth quét biến thể element (String/Vector?/Nullable(String)) — bài học HP.3.
G mandate teeth #2: **mảng String → pop → drop, vỡ bộ nhớ = vặn cổ.**

## Quan hệ ADR
Kế thừa: ADR-0060 (P1/P2 tách-tầng), 0066/0067 (heap-in-aggregate drop-glue), 0076 (sentinel-no-op
free R4). KHÔNG đụng: 0068 (Box CẤM), native-layout (defer). Mở đường: HashMap<K,V> typed
(campaign sau), collection iteration / Index-move (Collection-Semantics).

## Ngày hiệu lực
Bậc C+ — element-type-MIR + elem_size + typed-free + by-ptr-ABI khi landed (O verify máu, G ký).
Không hồi tố Vector<Integer> (fast-path i64 bảo tồn byte-compat).
