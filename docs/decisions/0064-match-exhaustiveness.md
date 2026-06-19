# ADR-0064 — Match Exhaustiveness (scalar-literal match)

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-19. Khởi thảo Mentor O 2026-06-19 theo phán quyết G.
- **Date:** 2026-06-19
- **Khởi thảo:** Mentor O (campaign Match-on-Literal — mở value-keyed match cho Integer/Trilean, nối Trit-path T6).
- **Liên quan:** ADR-0061 T6 (Trit-match value-keyed SwitchInt, lib.rs:2924 — tiền lệ khuôn) · ADR-0021 (Trilean Ł3) · SPEC §match.

---

## 1. Context
Triết cho `match` trên `Trit` (value-keyed SwitchInt, T6) + enum (GetDiscriminant) + nullable (`~+/~0`) + Outcome. NHƯNG `match` trên **Integer/Trilean literal** bị refuse thô ở LOWER (`Expr::Match` enum-path fallthrough, lib.rs:3797-3800): `match x { 1 => .. }` / `match t { true => .. }` → "unsupported match pattern". Typecheck **nhận** (lỗi chỉ ở lower). UI rác: cho match Trit mà cấm match Integer/Trilean.

## 2. Decision — Rule vét cạn (G chốt 2026-06-19)
**MỌI `match` trong Triết PHẢI exhaustive (vét cạn).**
1. **Integer (domain vô hạn):** BẮT BUỘC nhánh wildcard `_` (hoặc bind `other =>`). Thiếu wildcard = lỗi (vét cạn bất khả không có catch-all).
2. **Trilean / Trit (domain hữu hạn 3 giá trị):** được bỏ wildcard NẾU liệt kê đủ 3 mặt (Trilean: `true`/`false`/`unknown`; Trit: `-1_trit`/`0_trit`/`1_trit`). Thiếu một mặt mà không wildcard = lỗi.

## 3. Encoding (đo, không đoán)
- **Trilean** literal → `ConstValue::Trit` i64: `True=1, False=-1, Unknown=0` (lower:1464-1466). `MirType::Trilean` (khác `MirType::Trit`).
- **Trit** literal → `-1/0/1` (suffix `_trit`).
- **Integer** literal → value i64 trực tiếp (`LiteralPattern::Integer{value, suffix:None}`).

## 4. Implementation — TẦNG nào enforce (G phán: tạm trap ở lower)
**Cổng exhaustiveness ĐÚNG = typecheck (compile-time).** Hiện typecheck KHÔNG enforce (nuốt match thiếu nhánh, đẩy xuống lower). G chốt cho campaign này:
- **Lower:** value-keyed SwitchInt cho Integer/Trilean (mirror Trit-path 2924). `cases: Vec<(i64,BasicBlock)>` + wildcard-last + SwitchInt + **default → wildcard body NẾU có, else `Terminator::Trap`/Unreachable** (GAP-2, y Trit-path).
- **Trap GAP-2 = BIỆN PHÁP TẠM.** Match thiếu nhánh + không wildcard → runtime trap, KHÔNG compile-error. Đây là nợ.
- **★ NỢ ghi sổ (campaign riêng, KHÔNG nhét vào Lát này):** Typecheck Exhaustiveness — bắt thiếu-nhánh ở compile-time (đúng Rule §2) thay vì văng trap runtime. Áp cho Integer/Trilean/Trit/enum/nullable đồng nhất.

## 5. Teeth (campaign Match-on-Literal lowering)
- Integer match đúng nhánh → giá trị đúng; Trilean match đủ 3 mặt → đúng.
- **Trap khi thiếu nhánh:** Integer match không-wildcard, scrutinee rơi giá trị không liệt kê → runtime trap (SIGILL). Trilean thiếu mặt + không wildcard → trap.
- Wildcard catch-all → đúng giá trị.
- Regression: Trit-path (174) + 209 corpus + workspace giữ xanh.

## 6. Consequences
- **Tích cực:** `match` đồng nhất cho mọi scalar; UI hết "match Trit OK, Integer refuse".
- **Tạm:** exhaustiveness enforce ở runtime-trap (lower), chưa compile-time. Ghi nợ §4 minh bạch — KHÔNG vùng tối.
- **Đóng băng:** typecheck-exhaustiveness = campaign riêng.

## 7. Chữ ký
- O: ✅ (encoding đo từ lower:1464; khuôn Trit-path 2924 tiền lệ; nợ typecheck tách minh bạch)
- G: ✅ (ký duyệt 2026-06-19 — Rule vét cạn khóa; trap GAP-2 ở lower = BIỆN PHÁP TẠM; nợ Typecheck-Exhaustiveness là campaign RIÊNG, cấm nhồi vào Lát lowering; mirror Trit-path, cấm đẻ pattern rẽ nhánh mới)
