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

---

## A1. AMENDMENT 2026-06-20 — Match Tryte/Long + Tryte range-check (Phụ lục §A1)

**Bối cảnh:** §8 quyết định #5 ghi nợ "Tryte/Long DEFER — Rule §2 chỉ nêu Integer/Trilean/Trit; lower chưa support match Tryte/Long". Campaign này đóng nợ đó: mở `match` value-keyed cho **Tryte** và **Long**, áp Rule vét cạn §2 đồng nhất, và bịt lỗ Tryte range chưa enforce.

**Số đo khắc đá (verify từ `triet_core::Tryte`, KHÔNG tin trí nhớ):** `Tryte::MAX = 9_841`, `Tryte::MIN = -9_841` → Tryte = **9 trit**, miền `[-9841, 9841]` (19_683 giá trị). *(Recon ban đầu nhầm 6 trit / ±364 — đã tự vạch mặt và đo lại bằng `tryte.rs:42`.)*

**KHÔNG đảo ngược §2/§3/§8** — chỉ mở rộng cơ chế đã khóa. Campaign **KHÔNG đụng** value-model i64 ABI, borrowck, drop-glue, JIT shim.

### A1.1 Cơ chế (mechanism)
- **Tryte/Long literal match → value-keyed SwitchInt, cùng key-extraction i64 như Integer** (cả ba đều map literal → `i64` key trực tiếp). Lower trích **helper dùng chung** gom Integer/Tryte/Long vào 1 lò hạ tầng (diệt 5-copy smell — §8 mở Integer/Trilean, nay thêm Tryte/Long = 3 copy value-keyed nếu không gom).
- **Trit/Trilean GIỮ RIÊNG** (key khác: Trit = −1/0/1 suffix; Trilean = True/False/Unknown discriminant). Surgical, không nhồi vào helper.

### A1.2 Exhaustiveness (áp Rule §2)
- **Tryte:** miền hữu hạn `[-9841, 9841]` nhưng **19_683 giá trị** → coi như **miền-lớn**, BẮT BUỘC catch-all `_` (hoặc `Variable`), giống Integer. Liệt kê 19_683 nhánh = phi thực tế; không cho phép "đủ-mặt-bỏ-wildcard" như Trit/Trilean.
- **Long:** miền ý niệm bignum (thực tế i64-capped, xem A1.4) → BẮT BUỘC catch-all `_`, giống Integer.
- **Enforce ở typecheck (compile-time), tái dùng E1026** `NonExhaustiveScalarMatch` — cùng khuôn §8 path Integer. Thiếu catch-all → E1026.

### A1.3 Tryte range-check (bịt lỗ giá-trị-lố)
- **Literal Tryte ngoài `[-9841, 9841]` → E1036** (generalize từ `IntegerLiteralOverflow`). Bắt **CẢ HAI vị trí:**
  - **Expression:** `let x: Tryte = 9999_tryte` → E1036.
  - **Pattern literal:** `match t { 9999_tryte => .. }` → E1036. *(Cửa lọt chính: `bind_pattern` hiện no-op trên literal → match-arm là đường giá-trị-lố chui lọt nếu chỉ check expression.)*
- **E1036 generalize:** thêm `type_name` để message phân biệt `Tryte` (±9_841) vs `Integer` (±3_812_798_742_493). Đúng tinh thần G "không để giá trị lố chui lọt — bít từ lúc lọt lòng Typecheck".

### A1.4 ★ NỢ trung thực — Long i64-cap (khắc đá, KHÔNG vùng tối)
- **Long range KHÔNG enforce ở lát này.** Long là phần **bignum đã defer** (value-model i64, ADR-0050 MirType). Tryte range bịt ở campaign này; Long range **vẫn treo**.
- **Hệ quả i64-cap:** Long match-arm với key literal `> i64::MAX` (hoặc `< i64::MIN`) → **lower error "out of range"**, kế thừa thẳng giới hạn i64 của value-model. KHÔNG silent-truncate (tinh thần ADR-0044). Đây là **nợ ghi sổ minh bạch**, không phải feature — sẽ đóng khi bignum value-model lên (tương lai JIT/wide-int).

### A1.5 Bảng quyết định

| # | Quyết định | Chốt |
|---|---|---|
| 1 | Cơ chế Tryte/Long match | Value-keyed SwitchInt, key i64 như Integer; helper dùng chung Integer/Tryte/Long. Trit/Trilean giữ riêng. |
| 2 | Exhaustiveness Tryte | Miền-lớn (19_683 giá trị) → BẮT BUỘC catch-all `_`, giống Integer (không cho đủ-mặt). E1026. |
| 3 | Exhaustiveness Long | BẮT BUỘC catch-all `_`, giống Integer. E1026. |
| 4 | Tryte range-check | E1036 generalize (`type_name`); bắt CẢ expression LẪN pattern literal. |
| 5 | Long range-check | **DEFER** (bignum). Long key > i64 → lower "out of range" (kế thừa i64-cap). Nợ ghi sổ A1.4. |

### A1.6 Chữ ký amendment §A1
- O: ✅ (số Tryte::MAX=9_841 verify từ tryte.rs:42, tự đính chính recon 6-trit→9-trit; cơ chế cùng key Integer; helper-extraction diệt 5-copy; range bắt CẢ expr+pattern; Long i64-cap khắc đá A1.4 minh bạch — không đụng value-model/borrowck/JIT)
- G: ✅ (ký duyệt 2026-06-20 — 3 điểm gác-cổng O đồng ý hoàn toàn: helper extraction bắt buộc, Tryte range bắt CẢ Expr+Pattern, Long i64-cap defer khắc đá phụ lục; trap GAP-2 §4 cấm gỡ; mirror khuôn §8, cấm đẻ pattern mới)
