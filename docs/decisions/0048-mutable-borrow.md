# ADR-0048: Mutable Borrow — Bậc C lát 5

**Status:** ACCEPTED — O + G ký 2026-06-08
**Date:** 2026-06-08
**Author:** AI (đồng nghiệp D, implement)
**Reviewers:** Mentor O (semantics, soundness) · Mentor G (codegen, ABI)
**Scope:** Mở `&0 mutable String` param + op mutate `clear` (len=0 in-place).
Exclusivity E2440 reuse. Return-mut-borrow `-> &0 mutable T` CẮT.
Append/grow CẮT (realloc mìn).

---

## Tóm tắt

Lát 2 (ADR-0045) niêm phong `&0 mutable`, lát 3 (ADR-0046) mở `-> &0 T` return-borrow.
Lát 5 mở `&0 mutable` param cho String, với chính xác MỘT op mutate: `clear` (set
len=0, ptr bất biến, không realloc). Append/push bị CẮT vì realloc đổi ptr → caller
ôm handle cũ trỏ memory freed. Return-mut-borrow `-> &0 mutable T` CẮT (E1042) — né
nợ `is_propagated`×mutable.

---

## §0 — Dữ kiện

| # | Dữ kiện | Vị trí |
|---|---------|--------|
| F1 | `Loan.form: ReferenceForm` có sẵn, phân biệt BorrowReadOnly/BorrowExclusiveMutable/… | `checker.rs:93-104` |
| F2 | Exclusivity hai tầng: typecheck `borrow_check.rs` Pass-3 `forms_conflict` (ADR-0025 §2, fatal first-line) + MIR `checker.rs:113-124` `conflicts_with` (defense-in-depth). `BorrowExclusiveMutable` → conflict với BẤT KỲ loan nào. | `borrow_check.rs` (typecheck), `checker.rs:113-124` |
| F3 | E2440 fire site MIR: `places_conflict` + `loan.conflicts_with(*form)` → bắn `NllExclusivityViolation`. Fire site typecheck: `forms_conflict` trả E2440. | `checker.rs:507-515`, `borrow_check.rs` (typecheck) |
| F4 | Probe: `2× &0 mutable m` → E2440 nổ. Hạ tầng exclusivity đã hoạt động, không cần viết lại. | Phase-0 |
| F5 | Probe: `modify(&0 mutable m)` e2e compile+RUN — &0 mutable param qua typecheck+lower+jit không vấn đề. | Phase-0 |
| F6 | E1042 gate (check.rs:398-408) đang chặn TẤT CẢ `-> Ref T` trừ `BorrowReadOnly`. `&0 mutable` return đã bị chặn sẵn — không cần action thêm. | `check.rs:398-399` |
| F7 | `PropagatedLoan` copy `form: orig.form` — nếu sau này mở return-mut, form được truyền đúng. Nhưng hiện CẮT. | `checker.rs:804` |
| F8 | String layout: `{len: i64@0, cap: i64@8, bytes@16}`. `clear` ghi 0 vào len@0 — ptr bất biến, không đụng cap, không cấp phát. | `mir_lower.rs:1509-1516` (len read), `1531-1547` (contains layout) |

---

## §1 — Op: `clear(&0 mutable String)` — len=0 in-place

**Quyết định:** Shim `__triet_string_clear(ptr)` → ghi `0` vào `len@0`. Không
realloc, không đụng cap, không đụng bytes.

**Lý do:** `clear` là op mutate an toàn tuyệt đối: chỉ set len=0, ptr handle không
đổi. Không có data-race vì exclusivity E2440 đảm bảo chỉ MỘT `&0 mutable` tồn tại
tại mỗi thời điểm.

**Shim signature:** `fn(ptr: i64) -> i64` (nhận handle, trả 0 = Unit).

**Vị trí:** `mir_lower.rs`, cạnh `__triet_string_len` (line 1509).

**Append/grow CẮT — ghi rõ lý do (cho Bậc D):**
- `append(&0 mutable String, suffix)` cần realloc khi `len + suffix.len > cap`.
- Realloc = `std::alloc::alloc` mới → ptr mới → handle mới.
- Nhưng `&0 mutable String` = handle i64 by-value — caller giữ handle CŨ (i64 value
  trên stack).
- Callee realloc → ptr đổi → handle mới KHÔNG propagate về caller → caller ôm handle
  cũ trỏ memory freed → use-after-free.
- Giải pháp Bậc D: handle-indirection (fat-pointer {handle_ptr}) hoặc pointer-to-handle
  trong ABI. Cần redesign ABI — không phải scope lát 5.

---

## §2 — Exclusivity: REUSE E2440, không viết lại

**Quyết định:** Cơ chế exclusivity `conflicts_with` (checker.rs:113-124) +
`places_conflict` (checker.rs:507) đã có sẵn và đã được verify (Phase-0). Không
viết lại, không thêm rule mới.

**E2440 rule cho `&0 mutable` — hai tầng song song:**

1. **Typecheck (fatal first-line):** `borrow_check.rs` Pass-3 kiểm tra
   `forms_conflict` (ADR-0025 §2) — chặn sớm tại typecheck, không cần tới MIR.

2. **MIR borrowck (defense-in-depth):** `checker.rs:113-124` `conflicts_with` —
   `BorrowExclusiveMutable` → `true` cho MỌI form. Fire site `checker.rs:507-515`.

- Hệ quả: một local có `&0 mutable` đang active → không ai có thể tạo borrow mới
  trên cùng local (dù shared hay exclusive). Guard redundant tối thiểu 4 đường
  (typecheck + MIR + E2450 drop-while-borrowed), over-defended.

- Đây là đảm bảo exclusivity — aliasing XOR mutability.

**TECH-DEBT (O, 2026-06-08):** Hai tầng borrow-check song song (typecheck-era
  v0.10 ADR-0025 + MIR-borrowck mới) là nợ kiến trúc. Teeth-isolation trên E2440
  thất bại vì quá nhiều guard redundant — không sai (defense-in-depth) nhưng nên
  hợp nhất khi Bậc C khép.

---

## §3 — ABI: handle i64 by-value, mutate in-place

**Quyết định:** `&0 mutable String` truyền handle i64 by-value, đồng nhất ABI với
`&0 String` (shared) và `String` (owned). Callee nhận handle, mutate dữ liệu tại
`handle + offset`.

**Không thay đổi ABI.** JIT không cần biết Borrow vs MutableBorrow — cùng
`use_var` path.

**⚠ Bãi mìn realloc-dangling (ghi cho Bậc D):** Op grow (append/push) bị CẤM
trong Bậc C. Lý do: realloc đổi ptr → handle caller trỏ memory freed. Giải pháp
Bậc D = handle-indirection (fat-pointer chứa `*mut i64` trỏ handle thật, callee
ghi handle mới qua con trỏ). Đây là lý do G nói "giết 90% compiler sơ khai" —
fat-pointer là thay đổi ABI toàn diện, không phải thêm op.

---

## §4 — Return-mut-borrow: CẮT (E1042 giữ)

**Quyết định:** `-> &0 mutable T` tiếp tục bị E1042 chặn (check.rs:398-399 chỉ
whitelist `BorrowReadOnly`). Không cần action thêm.

**Lý do:**
1. `is_propagated` skip E2450 hiện dựa trên giả định no-nested-scope (ADR-0046
   TECH-DEBT). Kết hợp mutation + propagated loan chưa được audit.
2. Return-mut-borrow mở ra exclusive mutable alias qua function boundary → cần
   audit toàn bộ dataflow (ai mutate? ai read? order?). Quá lớn cho lát 5.
3. E1042 đã chặn sẵn — chỉ whitelist BorrowReadOnly, mọi form khác (gồm
   BorrowExclusiveMutable) bị từ chối.

---

## §5 — Teeth (3 fixtures)

### 93 — clear RUN (sine-qua-non)

| Fixture | Directive | Nội dung |
|---------|-----------|----------|
| `93_clear_run.tri` | EXPECT: 0 | `clear(&0 mutable m)` → `length(m)` = 0 |

Teeth: gỡ `write_unaligned(0)` trong shim (clear thành no-op) → length=5≠0 → đỏ.

### 94 — mut overlap (E2440)

| Fixture | Directive | Nội dung |
|---------|-----------|----------|
| `94_mut_overlap.tri` | ERROR: E2440 | 2× `&0 mutable m` → exclusivity violation |

Teeth: gỡ `BorrowExclusiveMutable => true` trong `conflicts_with` → lọt → đỏ.

### 95 — mut vs shared conflict (E2440)

| Fixture | Directive | Nội dung |
|---------|-----------|----------|
| `95_mut_shared_conflict.tri` | ERROR: E2440 | `&0 mutable m` + `&0 m` → conflict |

Teeth: gỡ check `conflicts_with` cho BorrowReadOnly vs BorrowExclusiveMutable → lọt → đỏ.

---

## Kế hoạch triển khai

| # | Việc | File chính | Mẫu |
|---|------|-----------|-----|
| 1 | ADR → commit | `docs/decisions/0048-mutable-borrow.md` | O+G ký |
| 2 | Shim `__triet_string_clear` | `mir_lower.rs` (cạnh 1509) | `__triet_string_len` |
| 3 | Typecheck overload `clear` | `env.rs` (cạnh 204-349) | `length` overloads |
| 4 | Lower dispatch `clear` | `lib.rs` (cạnh 1316) | `contains` dispatch |
| 5 | Đăng ký shim driver + harness | `main.rs` + `integration_tests.rs` | 1 shim × 2 |
| 6 | Fixtures 93-95 | `fixtures/` | 3 fixture |
| 7 | Gate + commit | `scripts/gate.sh` | |

---

## Q&A

### O-Q1: Vì sao chỉ clear, không append?

Append cần realloc → ptr đổi → handle caller trỏ memory freed. `clear` set len=0,
ptr bất biến, không realloc. (§1)

### O-Q2: Exclusivity có cần viết thêm rule không?

Không. `conflicts_with` (checker.rs:113) đã có `BorrowExclusiveMutable => true`
— conflict với MỌI form. Fire site (checker.rs:513) đã hoạt động. (§2)

### G-Q1: ABI cho &0 mutable?

Handle i64 by-value, đồng nhất &0/&0 mutable/String. JIT không phân biệt. (§3)

### G-Q2: Khi nào mở append/push?

Bậc D — sau khi có handle-indirection/fat-pointer. Cần redesign ABI toàn diện. (§3)

### G-Q3: Return-mut-borrow?

CẮT. E1042 giữ chặn. Mở sau khi re-audit is_propagated×mutable. (§4)
