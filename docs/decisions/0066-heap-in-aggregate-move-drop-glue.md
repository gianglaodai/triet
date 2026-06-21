# ADR 0066 — Heap-in-Aggregate: Move & Drop-glue (Flat, Lát 1)

> # ⚖️🩸 LUẬT THÉP SOUNDNESS — BẤT BIẾN ATOMIC (LỜI THỀ TRỤC B)
> # `byte-copy` ⟶ `tombstone-source` PHẢI ATOMIC TRONG CÙNG MỘT BASIC BLOCK.
> **TUYỆT ĐỐI KHÔNG** được chèn function-call / rẽ-nhánh-CFG / điểm-panic-hoặc-trap nào vào khe giữa
> nhịp copy mảng byte và nhịp ghi `0` vào con trỏ source. Nếu lọt khe → panic → drop-glue kích hoạt khi
> CẢ HAI biến trỏ chung 1 heap → **DOUBLE-FREE BANH XÁC**. Mọi WO/code Trục B vi phạm bất biến này =
> REJECT không cần đọc tiếp. (G khắc đá 2026-06-21.)

**Trạng thái:** ✅ **Quyết định — G KÝ DUYỆT bản vẽ 2026-06-21** (D được phép `rustc` từ đây). Áp dụng
cho Bậc C+. Cho phép **struct chứa heap field PHẲNG** (`struct Person { name: String }`) construct, move
qua function boundary, và drop **không leak, không double-free** — gỡ rào B8 cho trường hợp FLAT.

**Issue:** Toàn bộ chuỗi aggregate (struct/enum/nullable) tới ADR-0065 đều **Copy-only** — rào B8
(§4 ADR-0065) khóa mọi heap field/payload (`String`/`Vector`/`HashMap`) trong aggregate. Recon Phase 1
(Trục B, 2026-06-21) chứng minh: **value-model i64 SỐNG** (con trỏ nằm được ở field-offset trong
StackSlot — `mir_lower.rs:660-665`), giới hạn chỉ là 10 cổng construction-gate `!is_copy(None)`. Nhưng
gỡ rào để lộ **3 lỗ tử thần**: (1) whole-struct copy = nhân đôi con trỏ → double-free; (2) KHÔNG có
struct drop-glue (`Drop(struct)` chứa heap → `Unsupported`); (3) KHÔNG có partial-move. Đây là campaign
VISION (object-model/ownership/lifetime) — phải có ADR trắng trước khi gõ phím.

**Quan hệ ADR:** trả nợ defer ADR-0065 §4 (B8) + ADR-0062 §6 (heap-in-aggregate). Tổng quát hóa
tombstone của ADR-0042 (Deinit) + Outcome heap drop-glue ADR-0057 (`mir_lower.rs:1454-1457`). Value-model
nền: ADR-0040 (heap layout) + ADR-0049 (fat-pointer String StackSlot). Box-tam-phân `&+` ≈ owner:
ADR-0022 §2 (drop-glue gắn vào owner-scope, KHÔNG vào object-header).

---

## Quyết định

Gỡ rào B8 cho **FLAT heap-in-struct** (Lát 1) bằng **3 cơ chế**, theo 3 kim chỉ nam G ký (2026-06-21):

### KCN-1 — Inline per-struct static drop-glue (CẤM header/v-table)
JIT giữ trọn `MirType` + `StructLayout` lúc compile. Khi `Drop(struct_local)`, JIT **tự walk layout**:
mỗi field heap → emit **tĩnh** một lệnh `free(ptr@offset)`. **Zero runtime memory overhead** — KHÔNG
object-header, KHÔNG v-table, KHÔNG drop-flag động. Mã dọn được "nhét tĩnh vào đít scope".

### KCN-2 — Copy-then-Tombstone move semantics (giả lập Move trên value-model copy)
Move struct-chứa-heap = byte-copy nguyên khối StackSlot (copy luôn địa chỉ con trỏ) **+ ngay sau đó**
emit **TOMBSTONE** (zero con trỏ heap ở slot GỐC). Khi scope gốc đến End-of-Scope, inline drop-glue
thấy ptr==0 → bỏ qua → KHÔNG double-free. Tổng quát hóa cơ chế Outcome `mir_lower.rs:1454-1457` cho
TỪNG heap field của struct.

### KCN-3 — FLAT only (recursive type defer Lát 2)
Lát 1 chỉ ôm **struct chứa heap LEAF trực tiếp** (`String`/`Vector`/`HashMap` field). Struct chứa
struct-chứa-heap (transitive/recursive `Node{next: Node?}`) **GIỮ refuse** → Lát 2 (tránh bài toán
infinite-size recursion).

### Tiền đề móng (phải vá TRƯỚC khi 3 cơ chế chạy)
- **M-1 Layout-sizing:** `lower_program` (`lib.rs:489`) hardcode mọi field = 8B; fixup ADR-0060
  (508-555) KHÔNG chữa heap. **Bảng width đã VERIFY (O đo shim 2026-06-21, G dặn kiểm):**

  | Heap type | Width field | Repr trong slot | Free shim | Drop-glue per-field |
  |---|---|---|---|---|
  | `String` | **24B** (fat) | `{ptr@0, len@8, cap@16}` — slot cache len/cap | `__triet_string_free(ptr, cap)` **2-arg** | load ptr@off+0 + cap@off+16 → free |
  | `Vector` | **8B** (thin handle) | `{ptr@0}` — len/cap/data sống TRONG heap (header) | `__triet_vector_free(ptr)` **1-arg** | load ptr@off → free |
  | `HashMap` | **8B** (thin handle) | `{ptr@0}` — len/cap/slots sống TRONG heap | `__triet_hashmap_free(ptr)` **1-arg** | load ptr@off → free |

  ⚠ **String ≠ Vector/HashMap về drop-arity** — drop-glue PHẢI dispatch per-field-type (String 2-arg với
  cap@+16; Vector/HashMap 1-arg). Khai nhầm width hoặc nhầm arity = byte-copy đạp vùng nhớ kế / free sai
  con trỏ = SIGSEGV chết cạn. Mở rộng fixup ADR-0060: `String → 24`, `Vector → 8`, `HashMap → 8`.
- **M-2 `is_copy(None)` audit:** `mir:706` default `None → assume Copy` ("SOUND only while B8 blocks heap
  fields"). Construction-gate `2999` truyền `None` → với field kiểu **nested-Struct** sẽ assume Copy →
  RÒ heap transitive. Lát 1 gate phải thread `Some(body)` + phân biệt **direct-heap-leaf** (cho phép) vs
  **transitive-heap** (refuse → Lát 2).

### Hình thức cụ thể — luồng SỐNG/CHẾT của `Person { name: String }`

```
struct Person { name: String }                 // layout (M-1): name@0 (24B), total_size = 24
function take(p: Person) -> Integer = { return 0 }
function main() -> Integer = {
    let p = Person { name: "Giang" };           // (A) construct
    return take(p);                             // (B) MOVE p → take
}                                                // (D) main scope end
```

Bộ nhớ & lifetime (StackSlot 24B mỗi struct local; String free shim đã null-safe: no-op nếu ptr==0):

```
(A) CONSTRUCT  Person{name:"Giang"}
    __triet_string_alloc("Giang") → heap H (cap=C, len=5)
    p.slot (main):  [ ptr=H | len=5 | cap=C ]      @ name@0
                       │
                       └── owns heap H

(B) MOVE  take(p)   ── ABI THẬT: pass-by-pointer (ADR-0049), KHÔNG byte-copy 24B ở call ──
    B.1 PASS-BY-POINTER: call take(&p.slot)  → callee đọc p qua con trỏ (cùng vùng nhớ main)
        callee NHẬN QUYỀN sở hữu (Move param, lib.rs:758 push_owned cho non-ref type)
    B.2 (xảy ra TRONG call, tại C) callee Drop(p) ở End-of-Scope → free(H)
    B.3 TOMBSTONE sau return: caller nã `Statement::Deinit(p)` (ADR-0042 Q1, lib.rs:2409)
        → zero ptr@name+0 trong p.slot CỦA MAIN
        p.slot (main):  [ ptr=0 | len=5 | cap=C ]  ← vô hiệu, Drop(main.p) sau sẽ no-op

(C) take SCOPE END → Drop(take.p)  [inline drop-glue KCN-1, đọc qua con trỏ param]
        walk Person layout → field name:String @0
        load ptr@0 = H (≠0) → __triet_string_free(H, C)   → heap H giải phóng (1 lần)

(D) main SCOPE END → Drop(main.p)  [inline drop-glue KCN-1]
        walk Person layout → field name:String @0
        load ptr@0 = 0 (tombstoned B.3) → __triet_string_free(0, C) = NO-OP
        → KHÔNG double-free ✓
```

**Bất biến soundness (BI):** mỗi heap allocation có **đúng 1** owner-slot với ptr≠0 tại mọi điểm CFG.
Hai dạng move:
- **Arg-move (by-pointer, `take(p)`):** callee Drop-glue (TRONG call) + caller `Deinit`-tombstone NGAY
  sau return. LUẬT THÉP: KHÔNG panic/CFG-branch xen giữa call-return và `Deinit`.
- **Assign-move (`let q = p`):** byte-copy slot→slot (mir_lower.rs:1510) + tombstone-source ATOMIC cùng
  basic-block (KCN-2 literal). LUẬT THÉP: KHÔNG xen giữa copy và tombstone.

Drop-glue free ⟺ ptr≠0. Tombstone (ptr=0) + free-null-safe = no-op idempotent. (Diagram amend 2026-06-21
theo Recon-2: ABI param là by-pointer + `Deinit`-reuse ADR-0042, KHÔNG byte-copy ở call như bản vẽ gốc.)

### Phạm vi Lát 1 (khoanh CHẶT)
| Trong scope | Ngoài scope (defer) |
|---|---|
| Construct `struct{f: String/Vector/HashMap}` (heap leaf trực tiếp) | Nested/recursive struct chứa heap (Lát 2) |
| Whole-struct MOVE qua function boundary (param by-move + sret return) | **Partial move** `let s = p.name` (move field ra — Lát 1.x) |
| Inline drop-glue walk layout, free từng heap leaf | Field reassignment `p.name = "x"` (drop-old + move-new — Lát 1.x) |
| Tombstone source sau move | Enum payload heap (cùng cơ chế, lát kế) |
| String trước (Vector/HashMap cùng mẫu) | `&+ T` owner-drop (design ADR-0022, backend chưa có) |

---

## Các phương án đã cân nhắc

### Drop-glue: sinh mã thế nào
| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Inline per-struct static JIT emit** (KCN-1) | Zero runtime overhead; tận dụng MirType có sẵn lúc compile; không phình bộ nhớ | JIT phức tạp hơn (walk layout đệ quy ở Lát 2) | ✅ **G CHỌN** — bám triết lý "no managed runtime" (VISION §7) |
| 2 | Object-header + drop-glue-table (mỗi heap object mang fn-ptr dọn) | Drop đồng nhất, dễ recursive | +8B/object runtime; "rác OOP" (G); phá value-model thuần | ❌ G bác — "cấm tha rác OOP vào" |
| 3 | Drop-flag động (bitset runtime theo dõi moved) | Move linh hoạt | Runtime overhead + state; thừa vì compile-time biết hết | ❌ thừa — value-model tĩnh |

### Move semantics qua boundary
| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Copy-then-tombstone** (KCN-2) | Tái dùng byte-copy StackSlot sẵn có; tombstone = 1 store; tiền lệ Outcome | "Copy thừa" địa chỉ rồi vô hiệu (vô hại — chỉ 1 store) | ✅ **G CHỌN** — tổng quát hóa Outcome 1454-1457 |
| 2 | True move (chuyển ownership không copy byte) | Không copy thừa | Phá ABI sret/pointer hiện tại; đập value-model | ❌ đập móng — value-model phải SỐNG |

### Phạm vi
| # | Phương án | Kết luận |
|---|-----------|----------|
| 1 | **FLAT trước, recursive defer** (KCN-3) | ✅ **G CHỌN** — tránh infinite-size recursion, miếng nhỏ |
| 2 | Ôm luôn recursive `Node{next:Node?}` | ❌ G bác — "tham nhai miếng quá to" |

---

## Hậu quả

### Tích cực
- Mở khóa case dùng cốt lõi: `struct{name: String}` — bất kỳ record/DTO chứa text. Tiền đề cho mọi
  data-structure thực tế.
- Value-model i64 ABI nguyên vẹn — không đập móng (recon Phase 1 chứng minh).
- Zero runtime overhead (no header/v-table) — giữ lời hứa freestanding/no-managed-runtime (VISION §7).
- Drop-glue inline mở đường `&+ T` owner-drop (ADR-0022) cùng luật "own → compiler nhét free vào đít scope".

### Tiêu cực
- JIT drop-glue + tombstone tăng độ phức tạp lowering (walk layout, per-field free emit).
- Byte-copy-thừa địa chỉ con trỏ ở mỗi move (1 store tombstone bù lại — vô hại).
- Layout fixup (M-1) phải biết width fat-pointer từng heap type (hard-code 24/8/8 — chấp nhận Bậc C).

### Rủi ro cần mitigate
- **R1 Double-free** nếu tombstone bị bỏ sót ở 1 nhánh CFG (move trong if-branch). → teeth: poison
  tombstone → `FREE_COUNT==2` (mẫu HP.2 `mir_lower.rs:4385`).
- **R2 Leak** nếu drop-glue bỏ sót 1 heap field (struct nhiều field). → teeth: struct 2 heap field →
  poison walk bỏ field thứ 2 → `FREE_COUNT < 2`.
- **R3 RÒ transitive** nếu gate Lát 1 (M-2) nhầm nested-struct-chứa-heap thành FLAT-cho-phép → JIT
  `Drop Unsupported` hoặc leak. → teeth: `Outer{inner: Person}` PHẢI refuse ở construction.
- **R4 Move-then-use** (dùng `p` sau khi move vào `take`) — borrowck phải bắt (E2420 move). Lát 1 dựa
  borrowck move-tracking sẵn có cho whole-local move; partial-move defer nên không thủng.
- **R5 Use-after-tombstone** trong cùng scope (đọc `p.name` sau move) → ptr=0 → đọc rác. borrowck
  E2420 phải chặn TRƯỚC khi tới JIT.

---

## Ngày hiệu lực

- **Bậc C Lát 1+** — FLAT heap-in-struct: construct + whole-move + inline drop-glue + tombstone. Chỉ
  `String` field trước; `Vector`/`HashMap` cùng mẫu trong Lát 1.x.
- **Defer Lát 2** — recursive/nested heap-in-aggregate, enum-payload heap, partial-move, field-reassign.
- Không áp dụng hồi tố: aggregate Copy-only (ADR-0065 và trước) giữ nguyên; rào B8 vẫn khóa mọi case
  NGOÀI FLAT-struct-heap-leaf.

---

## Tiến độ thi công (Lát 1 — 4 nhát)

- **Nhát 1a (M-1+M-2+KCN-1+STEP 4):** heap-leaf field sizing (String=24/Vector=8/HashMap=8) · B8-relax
  gate (`ctx_is_copy` đệ quy trên `field_decl_ty`: direct heap leaf ALLOW, transitive + `Nullable(heap)`
  REFUSE) · inline per-struct static drop-glue (`emit_heap_free_at` walk layout) · fat-store (String field
  projected dest copy len/cap — hết pass-by-luck UB). Fixtures 256/257 + 3 unit teeth (R-cap/R-leak/R2/R3).
- **Nhát 1b (arg-move):** `take(p: Person)` whole-move qua boundary — callee by-pointer drop-glue
  (`copy_base_addr` unify slot-local + param; `emit_heap_free_at` address-based) + caller `Deinit`
  tombstone (`to_zero` 6 site → `ctx_is_copy`; Deinit struct-walk). LUẬT THÉP ATOMIC: `Deinit` đầu ret_bb.
  Fixture 258 + counting test (R-callee/R1-deinit/R1-arg, double-free FREE_COUNT==2).
- **Nhát 1c (assign-move):** `let q = p` true-move (giết pseudo-copy alias) — `is_move_binding` →
  `ctx_is_copy`; `Deinit(p)` LIỀN SAU move-Assign (ATOMIC). LOWER-ONLY (JIT 0 dòng). Fixture 259 +
  counting test (R1-assign).
- **Nhát 1d (LOCK & SEAL):** Vector/HashMap field + struct use-after-move E2420 — mechanism type-generic
  1a/1b/1c đã phủ, niêm phong bằng fixtures 260/261/262 + counting teeth (R-leak-vec/R-leak-hmap +
  **isolation scalpel:** poison riêng `is_vec` → Vector leak 0, String CÙNG struct sống 1 — chứng minh
  drop-glue dispatch per-field-type) + R-e2420. 0 dòng compiler mới.

**Bãi mìn Partial-move (`let s = p.name` móc field heap ra) — KIÊN QUYẾT DEFER Lát 1.x (G chốt 2026-06-22).**
Borrowck track whole-local move-state, KHÔNG field-level; thêm nữa bị chặn bởi read-side gap (String field
read → `Unknown` type, chưa lower — cùng gap §12.8 ADR-0065). Không reachable sạch → đụng vào lúc này =
chọc Typing Inference, nhiễu loạn Trục B. Lát 1 dừng ở ranh giới **whole-local move**.

**✅ Lát 1 (1a+1b+1c+1d) HOÀN TẤT** — heap-leaf field (`String`/`Vector`/`HashMap`) construct + whole-move
(arg + assign) + inline drop-glue + tombstone + use-after-move E2420: **sound + locked**. Rào B8 thủng cho
FLAT heap-in-struct; vẫn khóa nested/recursive/enum-payload (Lát 2).

---

**Chữ ký ADR-0066:** O: ✅ (recon Phase 1 + vẽ bản kiến trúc + verify width Vector/HashMap qua shim) ·
G: ✅ (ký duyệt bản vẽ 2026-06-21 — 3 KCN + cắt scope whole-move + gộp M-1/M-2 vào Lát 1 + khắc LUẬT THÉP
Atomic). D được phép `rustc` từ điểm này.
