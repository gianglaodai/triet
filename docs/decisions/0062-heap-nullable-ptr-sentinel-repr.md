# ADR-0062 — Heap-Nullable repr: ptr-sentinel (`T?` cho T ∈ {String, Vector, HashMap})

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-18. Khởi thảo Mentor O 2026-06-18, grounded từ MIR/JIT line-cite + tiền lệ móng runtime no-op.
- **Date:** 2026-06-18
- **Khởi thảo:** Mentor O (mổ slot layout String/Vector/HashMap + đối chiếu móng free-shim no-op).
- **Chữ ký:** O ✅ (repr grounded, móng runtime đã hé sẵn) · G ✅ (ký duyệt 2026-06-18 — scope khóa hẹp String/Vector/HashMap, defer Struct?/Enum?; tombstone bắt buộc mọi lát Drop; bất biến `ptr==NULL_SENTINEL` khắc đá, cấm `ptr==0`).
- **Liên quan:** [ADR-0041](0041-nullable-pa3c.md) (scalar `T?` PA-3c `i64::MIN` sentinel — repr này MỞ RỘNG sang heap) · [ADR-0049](0049-fat-pointer-abi.md) (String 24-byte slot `{ptr@0,len@8,cap@16}`, "slot is sole truth") · [ADR-0042](0042-ownership-across-boundary.md) (Deinit tombstone — Lát 5 tránh double-free) · [ADR-0044](0044-arithmetic-range.md) (NULL_SENTINEL canary nằm ngoài mọi range).

---

## 1. Context — stdlib kêu gào, compiler không dịch nổi

`T?` cho `T` **scalar** (Integer/Trit/Tryte/Long/Trilean/Unit) đã chạy từ ADR-0041:
single-i64 sentinel `NULL_SENTINEL = i64::MIN` (`triet-mir/src/lib.rs:2334`), canary N1 chứng
minh nằm ngoài mọi range scalar.

`T?` cho `T` **heap** (String/Vector/HashMap) hiện **bị chặn cứng** ở
`Body::verify()` (`triet-mir/src/lib.rs:1440-1464`, `MirError::HeapNullableNotLowered`):
`find_heap_nullable` (1380) tìm `Nullable(inner)` với `inner` ngoài whitelist scalar
(`is_scalar_nullable_payload` 1362) → refuse. Lý do gate (ruling β, G ký 2026-06-18): stdlib
**khai** heap-nullable làm API (`env.get`/`path.parent`/`fs.read -> String?`); declaration vô
hại (stub `= ~0`), nhưng **compilation** thì miscompile — single-i64 sentinel không chứa nổi
fat-pointer 24-byte. Gate ở LOWER (không typecheck) để declaration lọt, compilation chặn.

**Hệ quả:** một hàm `function read() -> String? = ...` typecheck OK nhưng JIT bất khả. Đây là
feature-gap năng lực thật, chặn toàn bộ stdlib I/O optional-return.

## 2. Decision — repr (a) ptr-sentinel, KHÓA

**`T?` cho heap dùng CÙNG slot với `T`, không thêm byte nào.** Trạng-thái-null mã hóa
bằng **ô `ptr` mang giá trị `NULL_SENTINEL`** (`i64::MIN`). Không cờ boolean, không
discriminant word, không boxing.

- Null-check = **MỘT phép so i64** trên ô `ptr`, KHÔNG memcmp cả slot.
- Widening `T → T?` = **NO-OP ở tầng repr** (cùng slot; non-null nghĩa là `ptr` trỏ allocation thật).
- `~0` (null) = ghi `NULL_SENTINEL` vào ô `ptr`.
- Drop null = an toàn **miễn phí** nhờ móng runtime đã no-op trên `NULL_SENTINEL` (§4).

## 3. Memory layout — mổ MIR (G yêu cầu chỉ rõ offset)

Ba heap type có HAI hình dạng slot khác nhau — nhưng ptr-sentinel áp **đồng nhất** vì cả ba
đều có một ô mang con trỏ:

### 3.1 String — 24-byte stack slot (fat-pointer)
```
offset:  0        8        16
        +--------+--------+--------+
slot:   |  ptr   |  len   |  cap   |     (mir_lower.rs:2301 "Must match StackSlot: ptr@0,len@8,cap@16")
        +--------+--------+--------+
        ↑
   null-check soi ĐÚNG ô này: stack_load(I64, slot, 0) == NULL_SENTINEL ?
```
- `String?` null  → `ptr@0 = NULL_SENTINEL`; `len@8`/`cap@16` = don't-care.
- `String?` non-null → y hệt String thường (ptr trỏ buffer; len/cap trong slot — ADR-0049 "slot is sole truth").
- Null-check = `stack_load(I64, slot, 0)` rồi `icmp eq NULL_SENTINEL` — **1 load + 1 cmp**, không đụng len/cap.

### 3.2 Vector / HashMap — single i64 handle
```
handle (i64): ptr tới [header | len | cap | data...]    (__triet_vector_alloc/__triet_hashmap_alloc -> i64)
              ↑
        handle == NULL_SENTINEL ? = null
```
- `Vector?`/`HashMap?` = chính i64 handle. Null → handle = `NULL_SENTINEL`.
- len/cap/data nằm trong heap header (không ở slot) → null-check = so handle, **0 dereference**.
- Đây là trường hợp ĐƠN GIẢN NHẤT: handle i64 đã là "ô ptr", so trực tiếp.

### 3.3 Vì sao ptr-sentinel áp được đồng nhất
Mọi heap type đều quy về "có một ô i64 mang con trỏ" (String: `slot[0]`; Vector/HashMap:
handle). Null = ô đó == `NULL_SENTINEL`. Không type nào cần thêm storage cho null-state →
**0 byte overhead**, đúng tinh thần G ("không đẻ flag boolean 8-byte rác").

## 4. Khớp HOÀN HẢO với móng runtime no-op (đã tồn tại — KHÔNG xây mới)

Toàn bộ free-shim ĐÃ coi `ptr == NULL_SENTINEL` (và `ptr == 0`) là no-op — móng cho Lát 3
(conditional Drop) **đã có sẵn**, đo được:

| Shim | Vị trí | Hành vi trên NULL_SENTINEL |
|---|---|---|
| `__triet_string_free` | mir_lower.rs:4024 + test 4786 | no-op (test khẳng định) |
| `__triet_vector_free` | mir_lower.rs:2469-2470 | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| `__triet_hashmap_free` | mir_lower.rs:2692-2693 | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| string ops (append…) | mir_lower.rs:2198 | guard NULL_SENTINEL |
| vector get OOB / hashmap key-miss | mir_lower.rs:2575/2848 | TRẢ NULL_SENTINEL (đã là producer null) |

**Hệ quả thiết kế:** JIT có thể gọi `free(ptr)` **vô điều kiện** trên một heap-nullable đang
null mà KHÔNG crash — shim tự nuốt. Drop của `String?`/`Vector?`/`HashMap?` null = free shim =
no-op. Lát 3 (conditional Drop) chủ yếu là **xác nhận + teeth**, không phải xây cơ chế mới.
(Vẫn cần conditional ở borrowck/lowerer cho semantics move-out, không chỉ dựa shim — xem §8.)

## 5. Phương án bị loại

- **(b) Cờ boolean tách rời** (`{is_null: i64, ptr, len, cap}` = 32-byte): +8 byte/giá trị, phình
  bộ nhớ, thêm một ô phải sync. G bác thẳng ("flag boolean 8-byte rác rưởi vô học"). Loại.
- **(c) Discriminant word** (kiểu Outcome `{disc@0, payload}`): biến `String?` thành 32-byte như
  heap-Outcome. Lãng phí — `String?` KHÔNG cần phân biệt 3 trạng thái như `T?~E`, chỉ cần null/non-null,
  mà `ptr` đã mã hóa được. Loại.
- **(d) Boxing/Option-tag trên heap:** thêm một lớp indirection + allocation cho null. Vô lý khi
  sentinel miễn phí. Loại.
- **(a) ptr-sentinel:** 0 byte overhead, 1-cmp null-check, khớp móng no-op. **CHỌN.**

## 6. Scope — KHÓA cứng, chống scope-creep

**TRONG scope (`is_any_heap()` = triet-mir:602):** `String?`, `Vector?`, `HashMap?`. Đây là cái
stdlib cần (`fs.read -> String?` v.v.).

**NGOÀI scope — DEFER (cần ADR riêng):**
- **`Struct?` / `Enum?`** — aggregate đa-word, KHÔNG có một ô `ptr` tự nhiên để cắm sentinel.
  Cần quyết định riêng (discriminant word, hoặc niche-fill vào field đầu, hoặc box). KHÔNG nhét
  vào campaign này. `find_heap_nullable` vẫn refuse chúng — đúng.
- **`T?~E` ternary heap** (Outcome có null-state + heap payload) — đã có hướng riêng ở chuỗi
  ADR-0053/0058 (32-byte slot disc). KHÔNG đụng.
- **Gap #2** (`~0`/constructor lồng trong block-final/if-arm không nhận expected-type) — type-
  propagation, là khách hàng của repr này nhưng là lát ĐỘC LẬP về cơ chế lowering.

## 7. Kế hoạch campaign (5 lát — sau khi ADR có 2 chữ ký)

1. **Lát 1 — repr nền:** `MirType::Nullable(heap)` được lowerer/JIT chấp nhận; slot = slot của
   inner; `is_copy` đã delegate (654: `Nullable(inner) → inner.is_copy`). Mở whitelist
   `is_scalar_nullable_payload`/`find_heap_nullable` cho heap-nullable (gate chuyển từ "refuse"
   sang "cho qua, có repr"). Teeth: `String?` compile + RUN.
2. **Lát 2 — widening + `~0`:** `String → String?` (no-op repr); `~0` materialize `ptr=NULL_SENTINEL`.
3. **Lát 3 — conditional Drop:** xác nhận free-no-op-on-null (móng §4) + teeth double-free.
4. **Lát 4 — Elvis `?:` + match `~+/~0`:** null-check project ô `ptr` (String slot[0] / handle),
   move payload ở non-null arm.
5. **Lát 5 — `?+>` map/flatMap heap:** unwrap move + Deinit/tombstone (ADR-0042) tránh double-free.
6. **Gỡ gate** `HeapNullableNotLowered` + dọn `find_heap_nullable`/`is_scalar_nullable_payload`.

## 8. Rủi ro + teeth bắt buộc (gỡ mìn ABI)

- **Double-free trên non-null Drop:** một `String?` non-null bị move-out rồi Drop lại → double-free.
  Lát 4/5 PHẢI tombstone (ghi `ptr=NULL_SENTINEL` sau move) — họ hàng hazard ADR-0057/0058 (dirty-slot
  → SIGABRT 134). **Teeth bắt buộc:** poison tombstone → re-drop → SIGABRT, đúng arm (success vs null).
- **Sentinel-vs-zero:** slot fresh init `ptr=0` (ADR-0049 Lát 3); cả `0` và `NULL_SENTINEL` đều
  free-no-op. PHẢI phân biệt "uninit (0)" vs "explicit null (SENTINEL)" ở null-check semantics
  (uninit không nên đọc như null hợp lệ). Teeth: probe cả hai.
- **Borrowck:** move-out một heap-nullable phải kill liveness như heap thường (ADR-0051). Teeth:
  use-after-move → E2420.
- **Mọi lát:** teeth phải quét CẢ String (24-byte slot) VÀ Vector/HashMap (single handle) —
  hai hình dạng slot khác nhau là hai mặt trận (blind-spot rule).

### 8.1 Amendment (2026-06-18, Lát 4) — double-free reachable Ở LÁT 4, KHÔNG defer

> **Lịch sử ruling (sửa-có-dấu-vết):** trong nghiệm thu Lát 4, Mentor O ban đầu
> phán "double-free vacuous ở Lát 4, defer sang Lát 5" dựa trên probe dùng
> `match f() {…}` (scrutinee là **temp** → MIR chỉ `Drop(arm_local)` → một free).
> O tự lật phán quyết khi đối chiếu fixtures thực: chúng dùng `let x = f(); match x`
> (scrutinee **named**). Với scrutinee named, drop-elaboration phát ra `Drop`
> cho CẢ arm-local `s` LẪN scrutinee `x` ở merge block — hai `Drop` trên cùng
> con trỏ. **Hazard double-free CÓ THẬT và reachable NGAY ở bề mặt Lát 4** (match/
> Elvis), không phải đợi Lát 5.

- **Cơ chế cứu mạng = M1 zeroing-on-move tombstone** (`triet-jit/src/mir_lower.rs`
  non-aggregate Assign path, `stack_store(zero, slot, 0)`): khi `s = move x`,
  ptr@0 của scrutinee bị ghi `0` → `Drop(x)` đọc ptr@0 == 0 → free-shim no-op →
  còn đúng MỘT free sống. `String?` đi non-aggregate path vì `ty_total_size`
  của `MirType::String` = 8 (`is_aggregate` false), nên M1 áp được.
- **Tombstone ghi `0`, KHÔNG `NULL_SENTINEL`** (ruling (b), G+O 2026-06-18, GIỮ
  NGUYÊN): an toàn vì (1) free-shim no-op trên cả `0` lẫn `NULL_SENTINEL`; (2)
  slot moved-out là **dead/unreachable** — borrowck E2420 chặn mọi use-after-move
  (fixture 191), nên bất biến §2 "ptr==SENTINEL ⟺ null, cấm ptr==0" áp cho giá trị
  **SỐNG**; slot chết miễn nhiễm. M1 dùng chung `layout.name=="String"` với String
  non-nullable → KHÔNG đổi (đụng move-semantics non-nullable, ngoài scope + rủi ro).
- **★ COUPLING ghi sổ:** soundness của tombstone-`0` **phụ thuộc borrowck-soundness**
  (use-after-move bị chặn cho mọi Move type). Nếu borrowck từng cho lọt một
  use-after-move trên heap-nullable, tombstone-`0` sẽ để lộ slot chết như "uninit
  (0)" thay vì "null (SENTINEL)" — nhưng đó là lỗ hổng borrowck, không phải repr.
- **Tooth bắt buộc (KHÔNG incidental crash):** free-count tường minh —
  `present_arm_move_out_freed_once` (`crates/triet-driver/tests/string_nullable_match_move_counting.rs`):
  non-null present-arm → count == 1; poison M1 (slot@0→slot@8) → count == 2 → RED.
  Dùng counting shim, KHÔNG dựa SIGABRT (allocator khoan dung có thể không abort →
  double-free thành leak câm). 192/196 (value fixtures) bắt M1-hỏng chỉ tình cờ
  (crash ≠ EXPECT) — không đủ tư cách memory-safety tooth; counting test mới là lưới.
- **Defer Lát 5 = đường double-free KHÁC:** Lát 5 (`?+>` map/flatMap) payload move
  vào map-fn trong khi scrutinee có thể bị drop riêng = double-drop trên con trỏ
  SỐNG (tombstone-trên-move không cứu được nếu move-target escape) — mặt trận
  riêng, teeth riêng.

## 9. Consequences

- **Tích cực:** stdlib optional-return (`fs.read`/`env.get -> String?`) compile được; 0 byte
  overhead; tái dùng móng free-shim no-op (ít code mới); nhất quán với scalar PA-3c (ADR-0041).
- **Chi phí:** 5 lát + gỡ gate; mỗi lát teeth double-free có máu.
- **Đóng băng:** `Struct?`/`Enum?` defer minh bạch — không hứa hẹn, không skeleton dead-code.
- **Bất biến mới:** "ô `ptr` == NULL_SENTINEL ⟺ heap-nullable null" — khóa cứng, mọi consumer
  (lowerer/JIT/borrowck) so ô ptr, KHÔNG so cả slot.
