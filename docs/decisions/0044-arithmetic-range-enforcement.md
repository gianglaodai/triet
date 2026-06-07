# ADR-0044: Integer Arithmetic Range Enforcement — Bậc C ưu tiên 1

**Status:** Draft — CHỌ KÝ Mentor O (semantics/soundness) + Mentor G (layout/ABI).
**Date:** 2026-06-07
**Author:** AI (khảo sát + đề xuất), quyết định cuối: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Trap-on-overflow cho mọi phép tính `Integer` (27 trit) tại tầng JIT
+ literal range check tại typecheck. Đóng D1, D1-literal, D2, D3.

---

## Tóm tắt

JIT arithmetic hiện tại là raw i64 — không enforce range ternary 27 trit.
`NULL_SENTINEL = i64::MIN` được bảo vệ bởi bất đẳng thức range chứ không bởi
cơ chế runtime. ADR này chốt: **mọi phép tính `Integer` vượt phạm vi
`[−(3²⁷−1)/2, +(3²⁷−1)/2]` → trap (panic)** — đúng nguyên văn SPEC §3.3
"mặc định **panic** — fail-fast, dễ phát hiện bug".

Trap rẻ hơn wrap: 1-2 chu kỳ so với 15-35. Và trap là ngữ nghĩa SPEC đã khóa
— không cần "tạm" hay "defense-in-depth", không cần lập luận lòng vòng.

---

## §0 — Dữ kiện

| # | Dữ kiện | Vị trí |
|---|---------|--------|
| F1 | `Integer` = 27 trit, range `M = (3²⁷−1)/2 ≈ ±3.81×10¹²` | SPEC §2.1 |
| F2 | JIT arithmetic raw i64: `iadd/isub/imul/sdiv` không range check | `mir_lower.rs:1142-1146` |
| F3 | SPEC §3.3: overflow mặc định **panic** | SPEC:502 |
| F4 | `Neg` đối xứng — `-x` của mọi giá trị in-range vẫn in-range | SPEC §3.2 |
| F5 | `|a±b| ≤ 2M ≈ 7.6×10¹² ≪ i64::MAX` — Add/Sub KHÔNG tràn carrier | số học |
| F6 | `|a*b| ≤ M² ≈ 1.45×10²⁵ ≫ i64::MAX` — Mul TRÀN carrier, cần smulhi | số học |
| F7 | `|a/b| ≤ |a| ≤ M`, `|a%b| < |b| ≤ M` — Div/Mod không cần check | số học (quy nạp từ input in-range) |
| F8 | Literal Integer không range-check — MIN qua typecheck sạch | probe O |
| F9 | `HashMap::insert` reject `v == MIN` (D2) | `mir_lower.rs:1714` |
| F10 | `Long` (81 trit) không tồn tại ở Bậc A | ADR-0041 F3 |

**Quy nạp:** nếu mọi nguồn sinh `Integer` đều enforce range (literal: E1036,
BinOp: trap, shim return: len ≤ memory ≪ M), thì mọi input của BinOp đã
in-range → chỉ cần check kết quả.

---

## §1 — Quyết định

### Q1: Trap hay wrap?

**Trap (panic).** Lý do:
1. **SPEC §3.3:** "mặc định **panic** — fail-fast". Trap là đúng ngữ nghĩa
   ngôn ngữ đã khóa, không phải "tạm" hay "defense-in-depth".
2. **Rẻ hơn wrap:** 1-2 chu kỳ (icmp + brif predicted-not-taken) vs 15-35.
3. **Wrap là việc của `add_and_truncate`** — method opt-in cho modular
   arithmetic tường minh. Khi method dispatch có mặt (Bậc C+), công thức
   balanced-modular trong §B sẽ được dùng cho method đó.

### Q2: Bảng per-op

| Op | Cần gì | Cơ chế |
|----|--------|--------|
| Add/Sub | Range check | `\|r\| > M` → trap. Carrier không tràn (F5). |
| Mul | Carrier overflow + range check | `smulhi` ≠ sign-extension của `smlo` → trap (F6). Sau đó `\|r\| > M` → trap. |
| Div/Mod | Không gì | Input in-range (quy nạp) + div-by-zero Cranelift native trap. |
| Neg | Miễn | Đối xứng (F4). |

Chỉ 3 op cần code (Add/Sub/Mul), không phải 5.

### Q3: smulhi — vá soundness cho Mul

`imul a, b` cho i64 có thể tràn carrier trước khi post-check thấy. Tích
128-bit của `a × b` có nửa cao trong `smulhi` và nửa thấp trong `smlo`. Nếu
`smulhi ≠ sign_extension(smlo)` → overflow đã xảy ra → trap. Sau khi qua
carrier check: `smlo` là giá trị 64-bit đúng, range-check `|smlo| > M` →
trap nếu vượt.

### Q4: D2 — giữ reject-MIN

Giữ `HashMap::insert` trap `v == MIN` làm defense-in-depth. Chi phí 1
compare/insert ≈ 0, tiền lệ Outcome-guard (guard cả đường provably-
unreachable), và là lưới cuối nếu quy nạp có lỗ chưa thấy.

### Q5: Literal range check (D1-literal)

Typecheck: literal Integer ngoài `±M` → E1036 `IntegerLiteralOverflow`.
Đây là việc riêng — code ở typecheck, không đụng JIT.

### Q6: 4 món nợ sau trap

| Nợ | Hành động |
|----|-----------|
| D1 (phantom null) | **ĐÓNG** — arithmetic không sinh ra giá trị ngoài range |
| D1-literal | **ĐÓNG** — E1036 ở typecheck |
| D2 (HashMap reject MIN) | **GIỮ** — defense-in-depth |
| D3 (shim MIN-input) | **ĐÓNG** — MIN không còn reachable |

---

## §2 — Implementation plan

1. **feat(track-c): Integer range constants** — `INTEGER_MAX`/`MIN`/`RANGE` trong
   `triet-core`, dùng cho cả typecheck và JIT.
2. **feat(track-c): JIT trap-on-overflow** — Add/Sub: range check; Mul: smulhi
   + range check. `lower_binop`.
3. **feat(track-c): Typecheck E1036 literal overflow** — range check literal
   Integer, reject ngoài ±M.
4. **feat(track-c): D2 update + fixtures** — giữ reject-MIN (update comment
   "bounded by D1" → "defense-in-depth"), fixtures: overflow trap,
   literal reject, Mul lớn trap.

---

## §3 — Đường migration

| Mốc | Việc |
|-----|------|
| Bậc C method dispatch | `add_and_truncate` dùng balanced-modular formula (§B) |
| Bậc C constant folding | Bỏ trap cho hằng số compile-time-known in-range |
| Bậc C Long (81 trit) | Carrier khác, cần smulhi tương tự với width 128+ |

---

## §A — Balanced modular formula (dành cho `add_and_truncate` tương lai)

```
M = (3²⁷−1)/2
R = 2M + 1 = 3²⁷

wrap(x) = ((x + M) % R + R) % R − M   // shift-positive → mod → shift-back
```

Công thức này KHÔNG dùng cho default `+` — chỉ cho method opt-in.

---

## §B — ADR / tài liệu liên quan

| Tài liệu | Quan hệ |
|----------|---------|
| SPEC §2.1 | Integer 27 trit range |
| SPEC §3.2 | Balanced ternary properties |
| SPEC §3.3 | Overflow semantics: default panic |
| ADR-0041 §6.2 | D1 — phantom null |
| ADR-0043 Q6 | D2 — HashMap reject-MIN |
| TODO.md | D1 + D1-literal + D2 + D3 |
