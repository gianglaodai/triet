# ADR-0047: Read-ops Extension — Bậc C lát 4

**Status:** DRAFT — chờ O + G ký
**Date:** 2026-06-08
**Author:** AI (đồng nghiệp D, implement)
**Reviewers:** Mentor O (semantics, soundness) · Mentor G (codegen, ABI)
**Scope:** Mở `contains` + `is_empty` cho String, Vector, HashMap — read-op thuần qua
`&0 T`, pass-by-ref, không struct-chứa-ref, không ABI mới, không fat-pointer.

---

## Tóm tắt

Lát 2 (ADR-0045) cho phép `&0 T` param — nhưng op duy nhất mở là `length`/`len`.
Lát 3 (ADR-0046) mở `-> &0 T` return-borrow. Giờ rút lãi từ móng: `contains` (tìm
key/element/substring) và `is_empty` (len == 0) — read-op thuần, không mutation,
không sở hữu mới, không struct-chứa-ref.

---

## §0 — Dữ kiện

| # | Dữ kiện | Vị trí |
|---|---------|--------|
| F1 | `length`/`len` đã mở cho String, Vector, HashMap (Lát 2 §8). Mẫu chuẩn: typecheck overload (owned + &0) + lower dispatch theo type-string + shim C. | `env.rs:204-349`, `lib.rs:1316-1349`, `mir_lower.rs:1509/1586/1774` |
| F2 | `contains` và `is_empty` CHƯA có: không shim, không typecheck overload, không lower dispatch. | grep xác nhận: 0 kết quả |
| F3 | `is_empty` có thể derive từ `len` (emit `len` shim → compare == 0) — không cần shim mới. HOẶC shim mỏng `__triet_*_is_empty(ptr)->i64`. | |
| F4 | `contains` cần shim mới cho mỗi type: String (substring search), Vector (linear scan), HashMap (key lookup). Trả về Trilean! (xác định, không Unknown). | |
| F5 | `slice` bị DỪNG: ref-view cần fat-pointer/struct-chứa-ref → vi phạm ADR-0046 Q3 (FieldPath::Field đã CẮT). slice-copy (owned output) là feature riêng — không phải read-op. | ADR-0046 Q3, Phase-0 probe |
| F6 | Cả 3 type đều có shim cơ bản (alloc/free/len/get/insert) trong `mir_lower.rs`. Shim mới viết cạnh các shim hiện có. | `mir_lower.rs:1368-1904` |

---

## §1 — `contains`: shim mới cho 3 type

**Quyết định:** Viết 3 shim `extern "C"` — String, Vector, HashMap. Mỗi shim nhận
handle i64 (và key/element), trả về i64 (1 = true, -1 = false), tuyệt đối không 0.

### String: `__triet_string_contains(haystack: i64, needle: i64) -> i64`

Substring search. Duyệt trong bytes của haystack, tìm needle. Trả về 1 (true) nếu tìm
thấy, -1 (false) nếu không. **TUYỆT ĐỐI không trả 0** — 0 = Unknown, vi phạm type
`Trilean!` refinement (statically ≠ Unknown).

Vị trí: `mir_lower.rs`, cạnh `__triet_string_len` (line 1509).

### Vector: `__triet_vector_contains(vec: i64, elem: i64) -> i64`

Linear scan. Duyệt mảng element, so sánh `==` (i64 equality). Trả về 1 (true) nếu
tìm thấy, -1 (false) nếu không. Không bao giờ 0.

Vị trí: `mir_lower.rs`, cạnh `__triet_vector_len` (line 1586).

### HashMap: `__triet_hashmap_contains(map: i64, key: i64) -> i64`

Key lookup. Duyệt bucket, so sánh key. Trả về 1 (true) nếu key tồn tại, -1 (false)
nếu không. Không bao giờ 0.

Vị trí: `mir_lower.rs`, cạnh `__triet_hashmap_len` (line 1774).

### Return type: Trilean! (không Trilean)

`contains` luôn trả về xác định (true/false), không có trạng thái Unknown như
`get(...)` (có thể null). → Return type `Trilean!` (refinement: statically ≠ Unknown).
Điều này cho phép `if contains(s, "needle")` không cần E1033 guard.

### Đăng ký shim

Shim mới phải đăng ký ở 2 nơi:
1. **Driver** (`main.rs`): danh sách `shims` trong `main()` — nếu không, JIT không tìm
   thấy symbol.
2. **Harness** (`integration_tests.rs`): danh sách `ShimSymbol` trong `run_fixture()` —
   nếu không, fixture test không chạy được (bài học §8 Lát 2).

---

## §2 — `is_empty`: derive từ `len` (không shim mới)

**Quyết định:** `is_empty(X)` → lower emit `len(X) == 0`. Không cần shim mới.

**Lý do:** `is_empty` thuần tuý là syntactic sugar cho `len(...) == 0`. Derive ở tầng
lower thay vì viết 3 shim × 2 ngôn ngữ (Rust shim + Triết wrapper) = 6 implementation
point.

**Triển khai:**
- Typecheck: `declare_overload("is_empty", fn(X) -> Trilean)` và
  `declare_overload("is_empty", fn(&0 X) -> Trilean)` — mỗi type 2 overload.
- Lower: khi gặp `is_empty`, emit `len(arg)` shim → compare với `ConstValue::Integer(0)`
  → trả về Integer (1/0 = true/false).

**Return type:** `Trilean!` (như contains — xác định, cho phép dùng trong `if`).

**Xác nhận encoding:** `is_empty(x)` = `len(x) == 0` qua JIT `BinOp::Eq`
(mir_lower.rs:1206-1209) trả `select(cmp, 1, -1)` — tự đúng Trilean: empty→1 (true),
non-empty→−1 (false). Không cần convert, không cần shim mới. Đây là lý do mạnh để
derive thay vì viết shim: thừa hưởng encoding đã đúng từ `Eq`.

---

## §3 — `slice`: TÁCH (không trong lát này)

**Quyết định:** DỪNG — không implement slice.

**Lý do (3 tầng):**

1. **Ref-view vi phạm ADR-0046 Q3:** Slice trả về reference vào sub-range của String
   cần fat-pointer `{ptr_offset, len}` → struct-chứa-ref → cần FieldPath::Field cho
   return-borrow. FieldPath::Field đã bị CẮT trong ADR-0046 Q3.

2. **Slice-copy (owned String mới):** Cấp phát + copy bytes → String mới. Đây là
   feature ngữ nghĩa riêng (substring-clone), không phải "read-op qua &0". Cần ADR
   riêng cho cơ chế copy/clone.

3. **Fat-pointer cần ABI mới:** Hiện tại mọi value là single i64. Fat-pointer = 2×i64
   → phá ABI. Đây là thay đổi kiến trúc, không phải "thêm op".

Quyết định tách thuộc về author (2026-06-08). Ghi vào ADR này để session sau không
tái đề xuất.

---

## §4 — Teeth (mỗi op một positive RUN)

Điều kiện sine-qua-non (bài học §8 Lát 2, happy-path Lát 3): mỗi op phải có ít nhất
MỘT fixture RUN ra số.

### contains

| Fixture | Directive | Nội dung |
|---------|-----------|----------|
| `85_contains_string_run.tri` | EXPECT: 1 | `contains("hello", "ell")` → true (Trilean true=1) |
| `86_contains_vector_run.tri` | EXPECT: 1 | `contains(v, 42)` → true |
| `87_contains_hashmap_run.tri` | EXPECT: 1 | `contains(m, key)` → true |
| `88_contains_miss_run.tri` | EXPECT: -1 | `contains("hello", "xyz")` → false (Trilean false=-1) |
| `89_contains_borrow_run.tri` | EXPECT: 1 | `contains(&0 s, "x")` qua borrow + reuse owner |

### is_empty

| Fixture | Directive | Nội dung |
|---------|-----------|----------|
| `90_is_empty_run.tri` | EXPECT: 1 | `is_empty("")` → true (empty → true=1) |
| `91_is_empty_nonempty.tri` | EXPECT: -1 | `is_empty("hello")` → false (non-empty → false=-1) |
| `92_is_empty_borrow.tri` | EXPECT: 1 | `is_empty(&0 s)` qua borrow + reuse owner |

---

## §5 — Kế hoạch triển khai

Theo mẫu 3 chỗ của `length` (F1):

| # | Việc | File chính | Mẫu |
|---|------|-----------|-----|
| 1 | ADR → commit | `docs/decisions/0047-read-ops-extension.md` | Chờ O+G ký |
| 2 | Shim: `__triet_string_contains` | `mir_lower.rs` (cạnh 1509) | `__triet_string_len` |
| 3 | Shim: `__triet_vector_contains` | `mir_lower.rs` (cạnh 1586) | `__triet_vector_len` |
| 4 | Shim: `__triet_hashmap_contains` | `mir_lower.rs` (cạnh 1774) | `__triet_hashmap_len` |
| 5 | Typecheck: `contains` + `is_empty` overloads | `env.rs` (cạnh 204-349) | `len`/`length` |
| 6 | Lower: `contains` dispatch | `lib.rs` (cạnh 1316) | `len`/`length` dispatch |
| 7 | Lower: `is_empty` derive len+compare | `lib.rs` (cạnh 1316) | Mới (không có shim) |
| 8 | Đăng ký shim: driver + harness | `main.rs` + `integration_tests.rs` | 3 shim × 2 nơi |
| 9 | Fixtures 85-92 | `fixtures/` | 8 fixture |
| 10 | Gate + commit | `scripts/gate.sh` | |

---

## Q&A

### O-Q1: Vì sao `is_empty` không viết shim riêng?

Derive từ `len` → ít code hơn, ít surface area cho bug. `len` đã có sẵn, được test
kỹ. `is_empty` = `len(...) == 0` là semantic đúng và đủ.

### O-Q2: Vì sao `contains` trả về Trilean! không phải Trilean?

Kết quả `contains` luôn xác định (found hoặc not-found). Không có trạng thái thứ ba.
Trả về `Trilean!` cho phép dùng trực tiếp trong `if` không cần E1033 guard.

### G-Q1: Shim ABI cho `contains`?

Handle i64 by-value, đồng nhất với tất cả shim hiện có. Không fat-pointer, không ABI
mới.

### G-Q2: `is_empty` có cần thêm shim không?

Không bắt buộc. Derive ở lower = 0 shim mới. Nếu G muốn tối ưu (tránh double-dispatch
len+compare), có thể thêm shim mỏng sau — nhưng không cần trong lát này.

### G-Q3: `slice` defer — bao giờ làm?

Khi có fat-pointer/string-view (Bậc D+) hoặc khi author chốt ngữ nghĩa substring-clone.
Tách khỏi lát read-ops.
