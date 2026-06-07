# ADR-0044: Arithmetic Wrap mod-3²⁷ — Bậc C ưu tiên 1

**Status:** Draft — CHỜ KÝ Mentor O (semantics/soundness) + Mentor G (layout/ABI).
**Date:** 2026-06-07
**Author:** AI (khảo sát + đề xuất), quyết định cuối: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Wrap mọi kết quả phép tính `Integer` (27 trit) về phạm vi cân bằng
`±(3²⁷−1)/2` tại tầng JIT. Đóng một lần 4 món nợ: D1, D1-literal, D2, D3.

---

## Tóm tắt

JIT arithmetic hiện tại là raw i64 — không enforce range ternary 27 trit.
`NULL_SENTINEL = i64::MIN` được bảo vệ bởi bất đẳng thức range chứ không bởi
cơ chế runtime. Khi arithmetic vượt phạm vi, MIN có thể được sinh ra hợp lệ
→ phantom null (D1), ambiguous sentinel trong HashMap (D2), và mọi shim read
mà chỉ guard 0 thì chấp nhận MIN-input (D3).

ADR này chốt: **mọi phép tính `Integer` (Add/Sub/Mul/Div/Mod/Neg) được wrap
về phạm vi balanced ternary `[−(3²⁷−1)/2, +(3²⁷−1)/2]`** ngay tại tầng JIT
`lower_binop`. Đây KHÔNG phải là i64 overflow wrap (power-of-2 mask) — đây
là balanced ternary modular arithmetic trong carrier i64, đúng với bản sắc
tam phân của ngôn ngữ.

---

## §0 — Dữ kiện đã verify

| # | Dữ kiện | Vị trí | Hệ quả |
|---|---------|--------|--------|
| F1 | `Integer` = 27 trit, range `±3_812_798_742_493` | SPEC §2.1 | Carrier i64 rộng hơn range hợp lệ ~2.4 triệu lần |
| F2 | JIT arithmetic là raw i64: `BinOp::Add → iadd`, `Mul → imul`, không wrap ternary | `mir_lower.rs:1142-1146` | D1: phantom null constructible qua arithmetic |
| F3 | SPEC §3.3: overflow mặc định **panic**; `add_and_truncate` là wrap, `add_and_saturate` là clamp, `try_add` là Option | SPEC:502-518 | Ngữ nghĩa ngôn ngữ ĐÃ ĐỊNH NGHĨA — ADR này chọn wrap làm chiến lược Bậc A/C, KHÔNG thay đổi default panic của SPEC |
| F4 | `Neg` (`-x`) không cần overflow guard — phạm vi balanced ternary đối xứng | SPEC:512 | Neg là no-op với wrap (identity trong phạm vi) |
| F5 | Literal Integer không bị range-check — `-9223372036854775808` qua typecheck sạch | probe O 2026-06-07 | D1-literal: cần range check riêng cho literal (typecheck, không phải JIT) |
| F6 | `HashMap::insert` reject `v == MIN` (D2) | `mir_lower.rs:1714` | Gỡ reject khi wrap chặn MIN từ arithmetic |
| F7 | 6 điểm chèn wrap trong `lower_binop`: Add, Sub, Mul, Div, Mod, Neg | `mir_lower.rs:1142-1149` | 6 vị trí, mỗi vị trí thêm 1 lần balance-mod |
| F8 | `Long` (81 trit) không tồn tại ở Bậc A | ADR-0041 F3 | Không cần xử lý Long wrapping hôm nay |

---

## §1 — Quyết định

### Q1: Wrap ở đâu?

**Tầng JIT `lower_binop`.** Wrap kết quả của Add, Sub, Mul, Div, Mod trước
khi trả về. Neg được miễn (F4: phạm vi đối xứng — negate của mọi giá trị
trong range vẫn nằm trong range).

Không wrap ở typecheck (Q2), không wrap ở MIR. Lý do: JIT là tầng duy nhất
nhìn thấy giá trị runtime thật; typecheck chỉ thấy literal, MIR chỉ thấy type
string.

### Q2: Wrap literal riêng (D1-literal)

Literal `Integer` vượt range 27 trit phải bị từ chối ở **typecheck**. Đây là
việc riêng của D1-literal, không liên quan đến JIT wrap. Typecheck parse
literal → kiểm tra `±(3²⁷−1)/2` → reject nếu ngoài range.

Lý do tách: dù JIT wrap mọi giá trị runtime, literal ngoài range là lỗi
lập trình — dev viết `let x = 9_223_372_036_854_775_808` không nên được
wrap âm thầm thành giá trị khác, mà nên bị từ chối.

### Q3: Wrap loại gì?

**Balanced ternary modular: `wrap(x) = x mod₃ (3²⁷)`** — không phải i64
overflow mask (2⁶⁴), không phải Euclidean modulo đơn thuần. Balanced ternary
modular arithmetic map x về phạm vi `[−(3²⁷−1)/2, +(3²⁷−1)/2]` bằng cách:

```
M = (3²⁷−1)/2  // half-range
x' = (x + M) % (2M + 1)  // shift to positive, mod, shift back
if x' > M { x' -= 2M + 1 }
return x'
```

Cách khác (đơn giản hơn cho implementation): `x % (2M+1)` rồi điều chỉnh
về phạm vi đối xứng.

### Q4: SPEC có bị thay đổi không?

**Không.** SPEC §3.3 nói default overflow = **panic**. ADR này không thay đổi
ngữ nghĩa ngôn ngữ — nó thêm cơ chế bảo vệ sentinel ở tầng runtime cho Bậc A
(i64 carrier). Khi Bậc C có overflow detection đầy đủ, default panic sẽ được
implement. Cho đến lúc đó, wrap ngăn sentinel bị xâm phạm — đây là defense-
in-depth, không phải thay đổi language semantics.

Nói cách khác: wrap là **hành vi tạm của Bậc A/C** để bảo vệ sentinel, không
phải ngữ nghĩa vĩnh viễn của `+`. Khi overflow detection hoàn chỉnh, dev sẽ
dùng `add_and_truncate` cho wrap tường minh và `+` sẽ panic — như SPEC đã nói.

### Q5: Chi phí?

Mỗi phép Add/Sub/Mul/Div/Mod thêm ~3-5 Cranelift instruction:
1. `iconst` — nạp hằng M
2. `iadd` — shift sang positive
3. `sdiv`/`srem` hoặc `urem` — modulo (đắt nhất, ~10-30 chu kỳ)
4. `isub` — shift về balanced

So với binary `iadd` (1 chu kỳ), balanced wrap thêm ~15-35 chu kỳ mỗi phép
tính. Đây là tradeoff thật: mọi `a + b` chậm hơn ~10-30× so với raw i64. Đổi
lại: sentinel kín tuyệt đối, 4 món nợ được xóa, và arithmetic fidelity với
balanced ternary identity.

**Mitigation:** Bậc C native layout + constant folding có thể loại bỏ wrap
cho hằng số tại compile time. Fast path cho phép tính đã biết nằm trong range
(loop induction variable với step nhỏ, v.v.) là optimization Bậc C sau.

### Q6: Điều gì thay đổi với 4 món nợ?

| Nợ | Trước wrap | Sau wrap | Hành động |
|----|-----------|----------|-----------|
| D1 (phantom null) | Arithmetic raw i64 sinh ra MIN | MIN không thể sinh ra từ arithmetic | **ĐÓNG** |
| D1-literal | Literal MIN qua typecheck sạch | Typecheck range-check literal (Q2) | **ĐÓNG** — cần code riêng |
| D2 (HashMap reject MIN) | `insert` trap MIN | MIN không còn reachable → trap không bao giờ bắn | **GỠ trap** — hoặc giữ làm defense-in-depth |
| D3 (shim MIN-input) | Shim read trap-on-0, nhận MIN | MIN không còn reachable | **ĐÓNG** |

---

## §2 — Implementation plan

1. **feat(track-c): Integer range constants trong triet-core** — `INTEGER_MAX`,
   `INTEGER_MIN`, `INTEGER_RANGE` (2M+1). Dùng cho cả typecheck range-check
   và JIT wrap.
2. **feat(track-c): JIT balanced wrap trong lower_binop** — sau mỗi Add/Sub/
   Mul/Div/Mod, wrap kết quả về `[−M, +M]`. 5 điểm chèn.
3. **feat(track-c): Typecheck literal range check** — integer literal ngoài
   ±M → E1036 (literal overflow). D1-literal đóng.
4. **feat(track-c): Gỡ D2 reject-MIN trong insert** — MIN không còn reachable
   từ arithmetic, trap chỉ còn defense-in-depth (giữ hoặc gỡ tùy quyết định).
5. **feat(track-c): Fixtures** — overflow wrap, literal reject, sentinel kín
   sau wrap.

---

## §3 — ADR / tài liệu liên quan

| Tài liệu | Quan hệ |
|----------|---------|
| SPEC §2.1 | Integer 27 trit, range `±3_812_798_742_493` |
| SPEC §3.3 | Overflow semantics: default panic, `add_and_truncate` wrap |
| SPEC §3.2 | Balanced ternary properties (F4: symmetric negate) |
| ADR-0041 §6.2 | D1 — phantom null qua arithmetic |
| ADR-0043 Q6 | D2 — HashMap reject-on-insert MIN |
| TODO.md | D1 + D1-literal + D2 + D3 |
