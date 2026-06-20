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

---

## 8. AMENDMENT 2026-06-19 — Typecheck Exhaustiveness (đóng nợ §4) — sửa-có-dấu-vết

**Bối cảnh:** §4 ghi nợ "Typecheck Exhaustiveness = campaign riêng". Campaign Latent Type-Inference (Mục 4) đã đổ móng (scrutinee scalar có kiểu tĩnh). Campaign này (Mục 1) đóng nợ §4: **dời enforcement Rule §2 từ runtime-trap (lower) lên compile-time (typecheck).**

**KHÔNG đảo ngược §2/§3** — chỉ thêm tầng enforce. Decisions (G ký 2026-06-19):

| # | Quyết định | Chốt |
|---|---|---|
| 1 | Mã lỗi | **Tái dùng E1026** + variant mới `NonExhaustiveScalarMatch { type_name, missing }`. KHÔNG đẻ mã mới (khuôn "1 mã, nhiều variant" như Outcome/Enum). |
| 2 | Catch-all | `Pattern::Wildcard` (`_`) **HOẶC** `Pattern::Variable(name)` (bind `other =>`) — cả hai short-circuit. |
| 3 | Trap GAP-2 ở lower | **GIỮ NGUYÊN, cấm gỡ.** Typecheck = tường thành; trap lower = defense-in-depth bất-khả-đạt cho code well-typed. |
| 4 | ADR | Amend 0064 §8 (đây). ADR-0065 dành Struct?/Enum? heap-nullable. |
| 5 | Tryte/Long | **DEFER, ghi nợ.** Rule §2 chỉ nêu Integer/Trilean/Trit; lower chưa support match Tryte/Long (refuse, không silent). Áp rule khi lower mở. |

**Enforcement (typecheck `check_match`, exprs.rs:1728, thêm nhánh sau dispatch enum/nullable/outcome):**
- **`Type::Integer`** (miền vô hạn): KHÔNG catch-all → E1026 "Integer match requires `_` wildcard". (`Range`/`Or` literal KHÔNG thỏa — vẫn cần catch-all.)
- **`Type::Trilean { .. }`** (cả refined): không catch-all & thiếu mặt nào của {true, false, unknown} → E1026 liệt kê mặt thiếu. `Or` expand sub-pattern.
- **`Type::Trit`** (−1/0/1): không catch-all & thiếu mặt → E1026 liệt kê. `Or` expand.

**Blast-radius (O quét toàn corpus 2026-06-19):** ZERO fixture vỡ — mọi scalar match hiện có đã exhaustive (215/218 Integer-có-`_`; 174/214 Trit-đủ-3; 216 Trilean-đủ-3).

**Chữ ký amendment:**
- O: ✅ (recon file:line — gap tại exprs.rs:1797; E1026 error.rs:399 khuôn sẵn; blast-radius ZERO đo bằng quét; trap lower giữ)
- G: ✅ (duyệt 2026-06-19 — 5 quyết định chốt; trap GAP-2 cấm gỡ; Tryte/Long defer ghi nợ)

**Nợ mới (2026-06-20) — ✅ ĐÃ ĐÓNG (`fa021b4`):** `Pattern::Variable` (catch-all bind name `other =>`) đã được typecheck chấp nhận, nhưng **lowerer (lib.rs:3224) đang refuse** đối với scalar-match — gap giữa typecheck-accept và lower-refuse. Đóng: lowerer nay bind Variable catch-all vào giá trị scrutinee (`bind_scalar_catch_all`, wiring cả 3 path Trit/Trilean/Integer; scalar Copy nên không push_owned/Drop). Trap GAP-2 giữ nguyên cho path không-catch-all. Teeth: fixtures 222 (Integer value-proof) / 223 (Trit) / 224 (Trilean) đỏ-trước-xanh-sau; poison gỡ Variable arm → refuse trở lại.
