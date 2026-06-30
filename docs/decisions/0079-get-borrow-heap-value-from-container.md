# ADR 0079 — Get-Borrow Heap Value from Container

> # 🩸 NGUYÊN LÝ CỐT LÕI (G khắc đá 2026-07-01)
> # "Đọc giá trị trong hộp mà KHÔNG đập hộp, và KHÔNG cắn RAM sau lưng người dùng."
> # `get` TUYỆT ĐỐI không clone ngầm (hidden heap allocation = rác của dev lười).
> # Mượn (zero-copy `&0 V`) là con đường ĐÚNG. Ai cần copy thì gõ tường minh
> # `m.get(k).clone()` — tự chịu trách nhiệm performance. Phần "máu" = Borrow Checker:
> # mượn heap-value từ container = chơi với dangling pointer (drop / realloc / rehash).

**Trạng thái:** ✅ **QUYẾT ĐỊNH — G ký 2026-07-01.** Áp dụng cho Bậc C+. Mở `get(&0 container, key) → (&0 V)?`
cho V heap (String / Vector / HashMap / Nullable), **zero-copy borrow**, thay cho E1047 refuse ở vị trí mượn.
**G chốt:** loan = whole-container (an toàn tuyệt đối, không fine-grained per-key) · not-found = `(&0 V)?`
nullable-borrow (tái dùng PA-3c, không trap không E-code) · giữ tên `get` (overload theo form Borrow vs Value).

**Sibling/kế thừa:**
- **ADR-0046** — PropagatedLoan (return-borrow ở call-site, bounded by dest liveness) = cỗ máy TÁI DÙNG.
- **ADR-0059** — stack-borrow `&0` heap Vector/HashMap; scaffold `&0 get`/`len`/`contains`/`is_empty` (scalar) ĐÃ CÓ.
- **ADR-0077 / 0078** — Typed Vector / HashMap P1 (value-typed storage sound). `get` heap → **E1047 refuse** (lỗ read-side này lấp).
- **ADR-0025** — borrow checker error codes E24XX. **ADR-0022 / SPEC §10** — 5 reference forms.

**KHÔNG đụng:** get-borrow **mutable** (`&0 mutable V` vào slot — defer), key-typed `HashMap<String,V>` (Tầng 2),
clone heap value (chỉ là method tường minh tương lai, KHÔNG ngầm trong `get`).

---

## Issue — vì sao cần quyết NGAY

Sau ADR-0077/0078, container heap-valued chỉ **ghi rồi hủy**: `insert`/`push` được, `remove`/`pop` (move-out) được,
nhưng **đọc một `String` trong map mà GIỮ nó lại** thì `get` → **E1047 refuse** (`check/exprs.rs:1147-1160`).
Mọi lookup-table thực tế cần ĐỌC lặp giá trị. Đây là lỗ chặn dùng thực tế. Lựa chọn:

- **Clone** — `get` trả copy sâu (cấp phát mới). ❌ **G phủ quyết**: giấu allocation sau lưng người dùng, ngược triết lý explicit/zero-cost.
- **Borrow** — `get` trả `&0 V` (mượn đọc, zero-copy). ✅ Tái dùng ADR-0046 + ADR-0059. **Quyết định này.**

Mượn heap-value = **chơi với dangling pointer**: nếu mượn xong container bị drop, hoặc ai đó `insert`/`remove`
key đó gây rehash/realloc (tầng C), `&0 V` cũ thành con trỏ rác. Borrow checker PHẢI thòng lọng.

## Quyết định

### 1. Chữ ký mới (typecheck/env)

```
get(&0 Vector<T>, Integer) -> (&0 T)?           // T heap; not-found → ~0
get(&0 HashMap<Integer, V>, Integer) -> (&0 V)?  // V heap (key=Integer cứng P1); key vắng → ~0
```

`(&0 V)?` = nullable-borrow (PA-3c sentinel pointer): present → con trỏ slot; vắng → `~0`. Người dùng PHẢI
check tường minh (`?:` / `match ~+/~0`) — compiler chửi nếu không. Đối xứng `remove → V?`.

- Vị trí **value** (`get(map, k)` owned/copy-out heap) GIỮ **E1047 refuse** — chỉ mở vị trí **borrow** (`get(&0 map, k)`).
- Scalar `get(&0 container, k) -> Integer` (copy-out) của ADR-0059 GIỮ NGUYÊN (Integer là Copy, không sinh loan).

### 2. Mô hình loan — **mượn VALUE = mượn cả CONTAINER** (bảo thủ, refuse-over-guess)

Borrow checker **không thể đặt tên** `map[k]` như một `Place` — slot tới qua hash-shim opaque (`places_conflict`
`checker.rs:66` đã xử Index = không-chứng-minh-được-rời-nhau → conservative). Do đó:

> **Loan của `&0 V` lấy `source` = CẢ container** (Place của arg 0), `dest` = return-temp `&0 V`,
> `is_propagated = true`, bounded by liveness của `&0 V` (y ADR-0046 PropagatedLoan).

Hệ quả: mượn MỘT value khoá CẢ map chống drop + mutate cho tới khi `&0 V` chết. Sound; người dùng cần truy cập
song song thì `.clone()` tường minh.

### 3. Ba điều kiện G — quy về luật enforcement

| # | G yêu cầu | Luật | Mã | Cơ chế |
|---|---|---|---|---|
| 1 | **Lifetime obligation** — `&0 V` không sống thọ hơn container | drop/return container khi `&0 V` còn sống → lỗi | **E2450** DropWhileBorrowed | PropagatedLoan + E2450 (`checker.rs:1013/1091`) ĐÃ CÓ — chỉ cần loan source = container |
| 2 | **Mutate-while-borrowed** — đang mượn, cấm `insert`/`remove`/mutate | consume/move container khi có active loan → lỗi | **E2440** (hoặc mã mới) | ⚠️ **NÂNG CẤP** — xem U3 |
| 3 | **ReferenceForm interaction** | `&0 V` = `BorrowReadOnly`: compose với `&0` khác (vd `length(&0 String)`, in ra); move / `&0 mutable` → conflict | E2440 | `conflicts_with` (`checker.rs:115`) ĐÃ phủ |

## Nghiên cứu NLL hiện tại → ĐIỂM CẦN NÂNG CẤP

Cỗ máy borrowck (`crates/triet-borrowck/src/checker.rs`) hiện có: `Loan{source:Place, dest, form, is_propagated}`
(`:95`), `places_conflict` field-level (`:66`), `conflicts_with` (`:115`), E2440 (`:328`), E2450 (`:350`),
PropagatedLoan cross-call (`:1100-1143`), M3 builtin consume-marking (`:1148-1161`).

| U# | Điểm | Hiện trạng | Cần nâng cấp |
|---|---|---|---|
| **U1** | Typecheck/env (`env.rs:441-477`) | `&0 get` overload chỉ **monomorphic scalar** (trả Integer copy). Heap → E1047. | Thêm heap-value `get(&0 container,k) → &0 V` overload (generic V / per-element). Phân biệt vị trí value (E1047 giữ) vs borrow (mở). |
| **U2** | Borrowck **builtin return-borrow** | PropagatedLoan (`:1105-1111`) CHỈ chạy cho **user sigs** (`callee_sigs` + `return_borrow_map`). Builtins đi `builtin_shim_meta` (`:1151`) — **KHÔNG có return-borrow**. | **Khai báo builtin return-borrow**: `get`(heap) → return mượn arg 0. Cơ chế: thêm field `returns_borrow_of: Option<usize>` vào `BuiltinShimMeta`, hoặc tổng hợp một sig. Loan source = cả container. |
| **U3** | Borrowck **move/mutate-while-borrowed** | M3 consume-marking (`:1153-1160`) mark arg Moved **KHÔNG kiểm active loan trước**. E2450 chỉ bắn ở Drop/Return — KHÔNG ở consume-via-builtin. | **Chèn kiểm**: trước khi M3 mark một consumed-arg Moved, nếu Place arg đó có active loan (`places_conflict(loan.source, arg)`) → **E2440** (mutate-while-borrowed). Đây là luật G #2. |
| **U4** | Lower/JIT | `get` heap chưa lower (E1047). | `get(&0 map,k)` → shim trả **con trỏ slot** (`&0 V` = địa chỉ value trong container), **zero-copy** (KHÔNG memcpy, KHÔNG alloc). Routing JIT mới cho get-heap-borrow. Not-found → ? (xem Rủi ro). |

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Get-borrow, loan = cả container** (chọn) | Zero-copy; tái dùng ADR-0046/0059; sound bảo thủ | Mượn 1 value khoá cả map | ✅ **CHỌN** (G mandate borrow-only) |
| 2 | Get-clone (`get` trả copy sâu) | Né lifetime; đơn giản borrowck | Giấu allocation ngầm; máy clone-shim mới | ❌ **G phủ quyết** |
| 3 | Loan per-slot (`source = map[k]`) | Mượn 1 value không khoá value khác | Borrowck **không đặt tên được** slot (hash-shim opaque); cần region-analysis nặng | ❌ vượt phạm vi, không sound được với shim opaque |
| 4 | `&0 mutable` get (mượn ghi vào slot) | Cho sửa value tại chỗ | Phức tạp gấp đôi (exclusive); chưa có use-case | ⏸️ **Defer** |

## Hậu quả

### Tích cực
- Container heap-valued **đọc được** end-to-end, zero-copy — khép vòng usability sau ADR-0077/0078.
- Tái dùng cỗ máy NLL (PropagatedLoan + E2440/E2450) — phẫu thuật khu trú 4 điểm (U1-U4), 0 đại phẫu.
- Coherence triết lý: explicit, zero-cost, no-hidden-alloc. Clone = method tường minh tương lai.

### Tiêu cực
- Mượn 1 value **đóng băng cả container** (không insert/remove khi đang mượn). Người dùng phải `.clone()` nếu cần đọc-rồi-sửa song song. (Bảo thủ có chủ đích — đổi an toàn lấy granularity.)

### Rủi ro cần mitigate
- **Dangling do realloc/rehash:** U3 (mutate-while-borrowed → E2440) là tử huyệt. **Răng cưa O:** đang mượn `&0 V` rồi `insert`/`remove` → PHẢI E2440 đỏ; gỡ U3 → poison phải đỏ (nếu không = lỗ UB câm).
- **Not-found semantics:** `get(&0 map, k)` khi key vắng — trả gì? `&0 V` không có "null borrow". Đề xuất: giữ kiểu trả `&0 V` **chỉ khi** đã `contains`, HOẶC trả `(&0 V)?` (nullable borrow = sentinel pointer). **Cần G chốt** (mục mở dưới).
- **Loan source = whole-base:** verify E2450 bắn đúng khi loan source là container-local (không phải local thường).

## Mục đã chốt (G ký 2026-07-01)

1. **Not-found → `(&0 V)?` nullable-borrow** (phương án a). G: trap (b) phí 2 lookup + ngu UX; E-code (c) là rác. Nullable-borrow an toàn + đối xứng `remove→V?` + tái dùng PA-3c (điểm cộng kiến trúc).
2. **Giữ tên `get`** — overload theo form (Borrow vs Value). Value-position heap đã E1047 → người dùng heap CƠ BẢN chỉ có 1 đường `get(&0 map,k)`. Không đẻ keyword rác.

## Ngày hiệu lực

- Bậc C+ — get-borrow heap value kích hoạt sau khi G ký + D implement + O verify máu (E2440 mutate-while-borrowed · E2450 drop-while-borrowed · zero-copy đọc đúng nội dung).
- Không áp dụng hồi tố. Scalar `&0 get` (ADR-0059) + value-position E1047 GIỮ NGUYÊN.
