# ADR-0043: HashMap Representation & Runtime Shims — Bậc B

**Status:** ĐÃ ĐÓNG — Mentor O ĐÃ KÝ (semantics & soundness, 2026-06-07) + Mentor G ĐÃ KÝ (layout/ABI, 2026-06-07).
**Date:** 2026-06-07
**Author:** AI (khảo sát + đề xuất), quyết định cuối: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** `HashMap<K, V>` với `K=Integer, V=Integer` ở Bậc B. String keys defer Bậc C
(kéo theo hash-by-content + free đệ quy).

---

## Tóm tắt

HashMap là cổng cuối cùng trong lộ trình a→c→b. Tiền đề `get -> V?` đã có từ
ADR-0041; B7-lift (ADR-0042) đã mở boundary cho heap types. ADR này thiết kế
memory layout + runtime shims cho `HashMap<Integer, Integer>` — scope tối thiểu,
dùng template Vector shim hiện hành.

---

## §0 — Template (Vector shim hiện hành)

Vector layout (`__triet_vector_alloc` tại `mir_lower.rs:1465`):
```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: current length
       cap (8B)       i64: capacity
       data[]         i64 elements
```
Pattern: alloc với `Layout::from_size_align(total, 8)`, write header + body fields
qua `write_unaligned`, free qua `dealloc(header, layout)`. Trap-on-0 ở read shims.
Tất cả shim là `extern "C" fn(i64, ...) -> i64`.

---

## §1 — Quyết định (Q1-Q7)

### Q1: Memory layout

Dùng template Vector: open addressing, flat array.

```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: số entry đang sống
       cap (8B)       i64: capacity (số slot)
       entries[]      mảng cap slot, mỗi slot: key(8B) + value(8B) + state(1B)
```

Entry state byte: `0 = EMPTY, 1 = OCCUPIED`. Byte value `2` reserved cho
`TOMBSTONE` khi `remove` lands (Bậc C) — hiện tại không có producer.

Lý do chọn byte riêng thay vì sentinel key: Q6 — `i64::MIN` là giá trị hợp lệ
với `V=Integer`. Không thể dùng sentinel key để đánh dấu empty. State byte cho
phép key nhận mọi giá trị i64 kể cả MIN.

Tổng kích thước slot: 8 + 8 + 1 = 17 byte. Padding lên 24 byte (align 8) để
key/value thẳng hàng 8-byte → `write_unaligned` không cần.

```
Layout: HEADER(8) + len(8) + cap(8) + cap × 24
```

### Q2: Hash function

`K=Integer`: `hash(k) = (k % cap + cap) % cap` — Euclidean modulo, luôn không
âm kể cả với `k = i64::MIN` (Rust `%` là truncating remainder — cho kết quả âm
với toán hạng âm; double-mod chuẩn hóa về `[0, cap)`). Identity hash — không
cần hàm băm phức tạp cho Integer key.

Khi String key đến (Bậc C), thay bằng hash function tổng quát (FNV-1a hoặc
SipHash).

Lưu ý: key `i64::MIN` là hợp lệ — không bị reject. State byte (Q1) đánh dấu
occupancy độc lập với giá trị key, nên không cần key sentinel để phân biệt
empty. Chỉ VALUE bị reject (Q6).

### Q3: Collision resolution — open addressing, linear probing

Open addressing với linear probing: `idx = (hash + i) % cap` cho i = 0, 1, 2, …

- Insert: probe đến EMPTY → ghi entry (chuyển state thành OCCUPIED).
- Get: probe đến OCCUPIED với key khớp → trả value; đến EMPTY → không có (trả
  NULL_SENTINEL).
- Remove: KHÔNG có trong scope Bậc B (defer). TOMBSTONE sẽ dùng cho remove
  (Bậc C) — lúc đó insert thêm nhánh "probe đến TOMBSTONE → ghi entry".

So với chaining: open addressing đơn giản hơn cho shim C-ABI — không cần alloc
riêng cho node chain, không cần free linked list.

**Invariant (termination):** load factor < 1 đảm bảo luôn tồn tại ít nhất 1
EMPTY slot → linear probing dừng trong mọi trường hợp. Realloc tại 0.75 duy
trì invariant này (tối đa ¾ slot OCCUPIED → tối thiểu ¼ EMPTY).

### Q4: Load factor + realloc

Load factor mặc định: 0.75 (75%). Khi `len >= cap * 3 / 4`, realloc gấp đôi:
`new_cap = cap * 2`.

Realloc mirror cơ chế `push` (fixture 37 đã test pattern):
1. Alloc mảng mới với new_cap
2. Rehash tất cả entry OCCUPIED từ mảng cũ sang mảng mới
3. Free mảng cũ
4. Trả về body ptr mới

Insert trả về HashMap mới (functional style — consume-and-return như push):
`m = insert(m, k, v)`.

### Q5: Drop/free — scope cut

Chỉ `K=Integer, V=Integer`: không có heap value nào trong entry → free chỉ giải
phóng 1 allocation duy nhất (mảng flat). Không deep-free.

`__triet_hashmap_free` guard cả `0` lẫn `MIN` → no-op — đồng bộ contract free
của ADR-0041 (`mir_lower.rs:1490` Vector free, `:1337` String free).

String keys + String values defer Bậc C: cần hash-by-content thay identity,
free từng String trong entry trước khi free mảng.

### Q6: MIN sentinel collision (D2)

**Vấn đề:** `get(map, k)` trả `V? = Integer?`. Key không tồn tại → trả
`NULL_SENTINEL` (`i64::MIN`). Nhưng `i64::MIN` cũng là một giá trị `Integer`
hợp lệ — nếu user `insert(m, k, i64::MIN)`, `get(m, k)` trả `i64::MIN` không
phân biệt được "có, giá trị = MIN" với "không có".

**Quyết định: REJECT-ON-INSERT — trap (SIGABRT).** `insert` gặp value
`i64::MIN` → `std::process::abort()`. Đây là họ D1 (ADR-0041 §6.2): món nợ
arithmetic không-wrap tạo phantom null; với container, phantom null hiện hình
thành ambiguous lookup.

Lý do chọn reject thay vì debt:
- D1 là bounded debt — sentinel nằm ngoài range hợp lệ. Với container, giá
  trị `i64::MIN` CÓ THỂ được insert hợp lệ → ambiguous là bug, không phải
  edge case lý thuyết.
- Khi arithmetic wrap mod-3²⁷ (Bậc B), `Integer` chỉ dùng 27 trit → `i64::MIN`
  vẫn là sentinel, không phải giá trị Integer hợp lệ → D2 tự đóng. Nhưng
  hôm nay Bậc A arithmetic chưa wrap → MIN là giá trị reachable.

Lưu ý: rejection là runtime-only — không có range check compile-time cho
literal Integer (cơ chế đó không tồn tại; thêm nó là debt riêng họ D1).
TODO.md: ghi D2 + điều kiện gỡ reject (arithmetic wrap mod-3²⁷).

### Q7: API mặt ngôn ngữ

| Builtin | Signature | Shim | Ghi chú |
|---------|-----------|------|---------|
| `hashmap_new()` | `-> HashMap` | `__triet_hashmap_alloc(0, 4)` | `alloc(len=0, cap=4)` — cap khởi tạo = 4 |
| `insert(map, k, v)` | `(HashMap, K, V) -> HashMap` | `__triet_hashmap_insert` | consume-and-return; trap nếu v == MIN (Q6) |
| `get(map, k)` | `(HashMap, K) -> V?` | `__triet_hashmap_get` | key không có → MIN; total function |
| `len(map)` | `(HashMap) -> Integer` | `__triet_hashmap_len` | trap-on-0 |

**BuiltinShimMeta (arg_consumes) — đồng bộ borrowck M3:**

| Shim | arg_consumes | Lý do |
|------|-------------|-------|
| `__triet_hashmap_alloc` | `[false, false]` | len, cap là Copy |
| `__triet_hashmap_insert` | `[true, false, false]` | consume map; k, v là Copy |
| `__triet_hashmap_get` | `[false, false]` | tiền lệ fixture 47: get không consume |
| `__triet_hashmap_len` | `[false]` | tiền lệ fixture 47 |

Tất cả dùng functional style: `insert` trả HashMap mới, consume map cũ. Pattern
`m = insert(m, k, v)` đã được chứng minh sống qua fixture 65 (Vector push idiom).

---

## §2 — Acceptance criteria

| # | Tiêu chí | Cách verify |
|---|----------|-------------|
| C1 | `hashmap_new()` → HashMap rỗng, len = 0 | Fixture |
| C2 | `insert(m, k, v)` → HashMap mới, len tăng | Fixture |
| C3 | `get(m, k)` sau insert → v | Fixture |
| C4 | `get(m, k)` với key không có → NULL_SENTINEL (Elvis → default) | Fixture |
| C5 | `insert` với v = i64::MIN → SIGABRT (reject) | Unit test kiểu N7 (subprocess — spawn child + env var + check signal; driver không bắt được SIGABRT trực tiếp) |
| C6 | Realloc khi vượt load factor → không mất dữ liệu | Fixture (insert nhiều) |
| C7 | `insert` consume-and-return — dùng lại map cũ → E2420 | Fixture |
| C8 | `m = insert(m, k, v)` idiom → functional update hoạt động | Fixture |

---

## §3 — Phạm vi (IN / OUT)

| IN | OUT (defer) |
|----|-------------|
| `HashMap<Integer, Integer>` | `HashMap<K, V>` generic |
| insert/get/len/hashmap_new | remove/contains/keys/values |
| Open addressing, linear probing | String keys (hash-by-content) |
| Reject MIN value on insert | Deep-free cho V=String |
| Functional insert (consume-and-return) | Mutable update in-place |

---

## §4 — Implementation plan

1. **feat(track-b): HashMap shims** — `__triet_hashmap_alloc/insert/get/len/free`
   + đăng ký vào driver/harness
2. **feat(track-b): HashMap typecheck + lowering** — overload `hashmap_new`,
   `insert`, `get`, `len`; type-string `"HashMap<Integer,Integer>"`. Lưu ý thứ
   tự classifier: `is_nullable_type` hỏi TRƯỚC `is_hashmap_type` (bài học
   `is_vec_type` nuốt `"Vector<Integer>?"` từ ADR-0041 §5.1)
3. **feat(track-b): HashMap fixtures 66-73** — acceptance C1-C8

---

## §5 — ADR / tài liệu liên quan

| Tài liệu | Quan hệ |
|----------|---------|
| ADR-0040 | Vector shim template, M1-M4 zeroing, arg_consumes |
| ADR-0041 | PA-3c NULL_SENTINEL, get → V?, D1 debt |
| ADR-0042 | B7-lift, Deinit, borrowck M3+ |
| ADR-0037 | Enum StackSlot (không dùng — HashMap là heap type thuần) |
