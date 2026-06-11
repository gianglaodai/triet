# ADR-0059 — Stack-borrow (`&0`) cho heap Vector/HashMap + vá nợ Generic return-bind

- **Status:** 🔓 APPROVED (scope) — chờ thi công C.1→C.2. Khởi thảo Mentor O 2026-06-11, grounded từ probe MIR+typecheck+JIT line-cite (driver run thật, exit-code đo trực tiếp).
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O (probe gốc rễ: đo cái gì đã chạy `&0 String`, vạch 3 lỗ vỡ Vector/HashMap, bóc tách `&+` ra khỏi scope).
- **Chữ ký:** O ✅ (root cause proven bằng driver-run + line-cite; ranh giới `&0` vs `&+` chứng minh) · G ✅ (duyệt scope 2026-06-11 — chốt đường (b), phong ấn `&+` theo YAGNI, ưng độc-dược-có-máu SIGABRT).
- **Liên quan:** [ADR-0045](0045-borrow-params-heap.md) (`&0 String` borrow param — tiền lệ tái dùng), [ADR-0046](0046-return-borrow-elision.md) (return-borrow + reference-drops-before-owner sort), [ADR-0042](0042-ownership-across-boundary.md) (B7-lift heap param Move + Deinit tombstone), [ADR-0050](0050-mir-type-enum.md) (MirType — Vector/HashMap bare), [ADR-0022](0022-trit-balanced-ownership.md) (S6 5-form — nguồn `&+`/`&-` BỊ LOẠI khỏi scope).

---

## 1. Context — `&0 String` đã sống; Vector/HashMap còn vỡ 3 chỗ

Probe 2026-06-11 (Mentor O) đo bằng `triet-driver run` trên file thật:

**ĐÃ chạy end-to-end** (trong corpus 160-fixture, line-cite từng điểm):
- `&0 String` / `&0 mutable String` borrow param: fixture `77_borrow_read_len`,
  `100_endgame_string_roundtrip` (append/realloc trong callee), `84/101` return-borrow.
- Cơ chế wire: call-site đẩy heap arg bằng `stack_addr(slot)` (pointer-to-caller-slot,
  `triet-jit/src/mir_lower.rs:1463-1483`); callee **borrow** KHÔNG copy + KHÔNG Drop
  (`triet-lower/src/lib.rs:621-626` skip `push_owned` cho ref-type; test
  `borrow_param_no_drop_in_callee:4346`). JIT bind thẳng pointer cho Reference-local.

→ **Wire backend cho stack-borrow heap ĐÃ ĐÓNG.** Mũi C không phải from-scratch; nó là
ba lỗ typecheck/lower còn hở cho Vector/HashMap.

## 2. Root cause — ĐO TỪ CODE, ba lỗ vỡ

| # | Triệu chứng (driver-run) | Gốc rễ (file:line) | Lớp |
|---|---|---|---|
| **B1** | `make()->Vector<Integer>; let ys=make(); len(ys)` → `lowerer error: len() on type ?` | `lower_type`/`lower_type_simple` (`triet-lower/src/lib.rs:740-789`, `802-848`) **không có nhánh `TypeExpr::Generic`**. `Vector<Integer>` (parser đẻ `Generic`, `triet-parser/src/type_expr.rs:202`) rơi `_ => MirType::Unknown`. User-fn return-type Vector → result-local Unknown → `len()` E. | Lower |
| **B2** | `peek(v: &0 Vector<Integer>)->Integer{return len(v)}` → `E1041 no overload of len` | `triet-typecheck/src/env.rs`: `len` chỉ có owned `(String)`/`(Vector<Integer>)`/`(HashMap)` (273-349) — **KHÔNG `&0` variant nào**. `get` (292-339) tương tự owned-only. (`contains`/`is_empty` ĐÃ có `&0` cho cả ba: 389-456.) | Typecheck |
| **B3** | `peek(s: &+ String)` → `E1041`, help chỉ `(String)`/`(&0 String)` | `&+` StrongFrozen không stdlib-fn nào nhận | **LOẠI khỏi scope** (§5) |

**Asymmetry chính xác (đo):** lỗ `&0` read-op còn hở đúng ở **`len` + `get`**. `contains`/
`is_empty` đã có `&0` cho String/Vector/HashMap. `length` có `&0 String` nhưng là hàm
riêng (không phải `len`).

## 3. Decision (G chốt đường (b) — duyệt scope 2026-06-11). HAI LÁT.

Mũi C định nghĩa lại: **Triệt nợ Type/Generic + mở rộng Stack-Borrow (`&0`) cho heap
Vector/HashMap.** `&+`/`&-` LOẠI (§5).

### Lát C.1 — vá nợ Generic return-bind (B1)

Thêm nhánh `TypeExpr::Generic { name, arguments }` vào **cả hai** converter
`lower_type` (`lib.rs:740`) **và** `lower_type_simple` (`lib.rs:802`):
- `name == "Vector"` → `MirType::Vector` (strip element type — Bậc A bare, theo ADR-0050).
- `name == "HashMap"` → `MirType::HashMap`.
- Khác → giữ `_ => MirType::Unknown` (refuse-over-guess).

Hệ quả: user-fn `-> Vector<Integer>` / `-> HashMap<...>` cho result-local đúng type →
`len`/`get`/`contains` trên kết quả call chạy.

### Lát C.2 — `&0` read-overload cho Vector/HashMap (B2)

`env.rs`: thêm `declare_overload` cho `len` và `get` các biến thể `&0`:
- `len(&0 Vector<Integer>) -> Integer`, `len(&0 HashMap) -> Integer`.
- `get(&0 Vector<Integer>, Integer) -> Integer?`, `get(&0 HashMap, Integer) -> Integer?`.
- (Tùy chọn đối xứng: `len(&0 String)` — quyết trong review nếu fixture cần.)

**RA NGOÀI scope C.2:** `push`/`insert` (mutate → consume+return owned hiện tại; đổi sang
`&0 mutable` in-place là thay đổi ngữ nghĩa, để ADR riêng nếu cần). `contains`/`is_empty`
đã có `&0` — không chạm.

Backend: `len` đã strip `&0` prefix (`lib.rs:1733-1737`), JIT bind pointer + shim nhận
pointer-to-slot — `len(&0 Vector)` chạy ngay khi overload tồn tại. **NHƯNG `get`
(`lib.rs:1939-1948`) dispatch bằng `arg0_ty.is_vec()/is_hashmap()` TRỰC TIẾP, KHÔNG strip
reference** (`is_vec` = `matches!(self, Vector)`, không nhìn xuyên `Reference`). → `get(&0
Vector)` đậu typecheck nhưng CHẾT ở lower `get() on type &0 Vector`. **C.2 PHẢI gồm
lower-fix `get`**: strip reference như `len`. (Đính chính §8 — claim "backend không sửa"
ban đầu SAI cho `get`.)

## 4. Teeth (ranh giới sinh tử) — route-lower qua `lower_source`, CẤM hand-build MirBuilder

### C.1 (Generic return-bind)
- **Positive:** fixture `make()->Vector<Integer>{...} ; main(){ let ys=make(); return len(ys) }`
  → RUN, exit = len đúng.
- **Poison:** revert nhánh `TypeExpr::Generic` → fixture regress về
  `lowerer error: len() on type ?`. Test phải đỏ khi đảo nhánh; xanh lại khi khôi phục.

### C.2 (`&0` borrow Vector) — ⚰️ ĐỘC DƯỢC CÓ MÁU (G đòi SIGABRT)
- **Positive:** `peek(v:&0 Vector<Integer>)->Integer{return len(v)}`; caller reuse `xs`
  sau borrow → KHÔNG E2420; RUN exit đúng len.
- **🩸 LỆNH TỬ HÌNH (đính chính §8 — chỗ poison ĐÚNG đã verify bằng máu):** poison
  `triet-lower/src/lib.rs:608` — strip Reference→owned inner type
  (`if let MirType::Reference{inner,..} = ty { ty = *inner; }`) → borrow param lower thành
  OWNED Vector → callee copy {ptr,len,cap} + Drop/free buffer của caller → caller reuse +
  scope-Drop lần hai → **DOUBLE-FREE → SIGABRT (134)**, `free(): double free detected`.
  - ⚠️ **KHÔNG poison `lib.rs:624`** (push_owned guard): O đã thử — lower emit `Drop(_0)`
    nhưng JIT Drop handler **type-gated** (không free local kiểu `Reference`) → vô hiệu hóa
    → exit 0, KHÔNG chảy máu. Bảo vệ borrow-param là HAI lớp độc lập; chỉ poison
    type-classification (608) mới defeat cả hai cùng lúc.
  - **Đã chứng minh máu trên đường `&0 String`** (fixture 77 dưới type-poison → exit 134,
    `free(): double free detected in tcache 2`, 2026-06-11). Cơ chế Vector ĐỒNG NHẤT
    (chung `lower_function:608`) — chạy được khi C.2 có overload.
  - Đo `exit == 134` trực tiếp KHÔNG qua pipe; **chụp lại cho G**. Đây là hazard THẬT
    (observable: glibc double-free abort) — khác defer-vô-nghĩa cap@24 (ADR-0058).
  - Khôi phục bằng `cp` snapshot /tmp (KHÔNG `git checkout` — luật teeth-never-checkout).

## 5. RA NGOÀI scope — `&+` StrongFrozen / `&-` Weak → backlog (YAGNI)

`&+` (StrongFrozen) / `&+ mutable` (StrongMutable) / `&-` (WeakObserver) theo S6/ADR-0022
là **shared-ownership refcount**, đòi `ObjectHeader` 8-byte + retain/release runtime shim
+ drop-decrement. Heap shim hiện tại (String/Vector/HashMap) là bare `{ptr,len,cap}` —
KHÔNG có ObjectHeader, KHÔNG refcount. Đây là một subsystem riêng, không phải tinh chỉnh
param-passing.

**Quyết:** phong ấn theo YAGNI (cùng logic đã đóng C3/C4 ở chiến dịch trước). **Điều kiện
mở khóa:** khi có use-case tiêu thụ shared-ownership thật (vd 2 owner sống đồng thời cùng
một heap object) → mở ADR riêng cho ObjectHeader refcount runtime, 2 chữ ký.

## 6. Consequences

- (+) `Vector<Integer>`/`HashMap` trở thành first-class qua user-fn boundary (return-bind
  + borrow read), khép nợ "only bare local holds heap" ở vế Vector mà String đã có.
- (+) Không thêm runtime, không thêm shim — thuần lower-converter + typecheck-overload.
- (−) `len`/`get` overload set phình thêm — chấp nhận (đối xứng với `contains`/`is_empty`).
- (−) `&+`/`&-` vẫn E1041 — có chủ đích, có điều kiện mở (§5).

## 7. Chỉ thị tác chiến

1. **D thi công từng lát**: C.1 trước (độc lập), gửi O review + raw gate → O teeth tay
   (poison Generic-arm, đo đỏ) → G ký → commit. Rồi C.2.
2. C.2 review: O tự ép **SIGABRT 134** bằng poison `lib.rs:608` (type-strip, §4 đính chính)
   trên code CUỐI, đo exit trực tiếp, **chụp lại** cho G. Không có SIGABRT đó → C.2 đồ bỏ.
3. Mỗi lát: gate dòng đầu raw (auto-reject nếu không raw), cập nhật TODO.md + handoff.

## 8. Amendment 2026-06-11 — đính chính teeth + scope `get` (append-only)

Trong lúc soạn work-order C.2, O probe sâu hơn + pre-verify teeth trên code thật, phát hiện
HAI sai trong bản gốc (§3/§4/§7):

1. **Teeth poison sai chỗ.** Bản gốc ghi poison `lib.rs:624` (push_owned guard). O ép thử:
   lower CÓ emit `Drop` cho borrow param, nhưng **JIT Drop handler type-gated** — bỏ qua
   local kiểu `Reference` → KHÔNG free → exit 0, KHÔNG SIGABRT. Bảo vệ là HAI lớp độc lập.
   **Chỗ poison ĐÚNG = `lib.rs:608`** (strip Reference→owned). Verify trên `&0 String`
   (fixture 77) → **exit 134 `free(): double free detected in tcache 2`**. Bài học: pre-verify
   teeth TRƯỚC khi giao, đừng hứa máu chưa thấy chảy (suýt lặp mẫu cap@24).

2. **`get` cần lower-fix.** Claim gốc "backend không sửa" đúng cho `len` (đã strip ref
   `lib.rs:1733-1737`) nhưng SAI cho `get` (`lib.rs:1939-1948` dùng `arg0_ty.is_vec()`
   trực tiếp, `is_vec`=`matches!(Vector)` không xuyên Reference). C.2 phải thêm strip-ref
   cho `get`.

- **Chữ ký amendment §8:** O ✅ (teeth-đúng-chỗ verify bằng máu + `get` gap đo từ code
  2026-06-11) · G ✅ (Đã đọc báo cáo. Poison phải nhắm đúng tử huyệt type-strip (608) mới xuyên thủng được 2 lớp phòng ngự. Tốt.)
