# Outcome Producer — Blueprint (O soạn, G lệnh 2026-06-10)

**HEAD `502713a`. Gate 0·0·102·203.** Error-handling core của Triết (ADR-0020).
Móng để sau lôi C4 (Packed Outcome) + C5 (Multi-value) khỏi Nhóm E.

## 0. Hiện trạng (O khảo sát — đo, không đoán)
| Tầng | Trạng thái |
|------|-----------|
| **Lexer** | ✅ `~+`/`~-`/`~0` + `~+>`/`~->` tokens (token.rs:258-272) |
| **AST** | ✅ `OutcomeConstructor`/`OutcomeArm`/`OutcomePropagate`/`OutcomeDefault`/`OutcomeArmHandler` (generated) + `BinaryOutcome`/`TernaryOutcome` types |
| **Parser** | ✅ parse `~+ value`/`~- error` (constructor + match-arm pattern, pattern.rs:96-342) |
| **Typecheck** | ✅ `check_outcome_constructor_context` + arm_handler/propagate/default (exprs.rs:314+) — **cần verify đủ E1024/E1025** |
| **Lower** | 🔴 **DEGENERATE**: `~+ e`=identity (lower payload, KHÔNG tạo 2-value); `~-`=unsupported (1124); bare `~+`=unsupported |
| **MIR** | ✅ `ReturnShape::BinaryOutcome` (arity 2) + `Return{Vec<Local>}` sẵn — **0 producer** |
| **JIT** | 🔴 multi-value chặn (jit:1070, = C5 defer) |

→ Outcome Producer thật (T~E = {disc: Trit, payload}) **chưa có ở Lower+JIT**. Frontend đã sẵn.

## 1. Q1 (G): Parse `~+`/`~-` ở Frontend?
**ĐÃ XONG.** Lexer + Parser + AST đầy đủ. `~+ value` → `OutcomeConstructor{arm:Positive, payload}`; `~- error` → `{arm:Negative, payload}`. Không cần làm thêm Frontend. **Verify only.**

## 2. Q2 (G): Typecheck check Return Type khớp Outcome?
**CÓ MÓNG, cần verify+bổ sung.** `check_outcome_constructor_context` (exprs.rs:317) đã có. Phải đảm bảo (ADR-0020):
- `~+ value` trong fn `-> T~E` → value khớp `T`. `~- error` → khớp `E`.
- **E1025**: `~0` trên `T~E` (binary, không null state) = compile error.
- **E1024**: Outcome exhaustiveness — match `T~E` phải cover `~+` + `~-` (không `~0`); `T?~E` cover cả 3.
- Return type inference: fn body `~+ v`/`~- e` → infer `T~E`.
**Lát Typecheck:** đo `check_outcome_constructor_context` cover đủ chưa; bổ sung return-type-match + E1024/E1025 nếu thiếu.

## 3. Q3 (G): Lower → `ReturnShape::BinaryOutcome`?
**ĐÂY LÀ CORE WORK** (lower hiện degenerate). Thiết kế:
- **Outcome value model:** `T~E` = 2 slots `{disc: i64 (Trit), payload: i64}`. (Bậc A: payload 1 i64; heap payload = Bậc B/C.)
- **`~+ value`** → MIR: alloc 2-slot, `disc = Trit::Positive (1)`, `payload = lower(value)`. KHÔNG identity nữa.
- **`~- error`** → `disc = Trit::Negative (-1)`, `payload = lower(error)`.
- **Fn `-> T~E`** → `ReturnShape::BinaryOutcome` (arity 2), `Return{values: [disc_local, payload_local]}`.
- **Match `~+ x / ~- e`** → `OutcomeDiscriminant` (đọc disc Trit) → branch → `OutcomeUnwrap`/`OutcomeUnwrapError` (đọc payload). MIR ops `OutcomeDiscriminant`/`OutcomeUnwrap` ĐÃ định nghĩa (mir:252-277, guarded) — wire chúng.

## 4. Q4 (G): Fixtures — JIT thiếu C5/C4. (a) Err-at-JIT hay (b) giả-lập-C5?
**ĐIỂM CHẶN CỐT TỬ:** Outcome 2-value return → JIT đụng C5 (multi-value chặn jit:1070) = vừa defer.

**Đề xuất O — (a) Pipeline tới MIR, JIT Err tạm + (b-lite) giả-lập-C5 CHỈ-cho-Outcome:**
- **Lát 1-3 (Frontend→Typecheck→Lower→MIR):** Outcome producer sinh `ReturnShape::BinaryOutcome` + MIR ops. **Fixture `check` mode** (parse→typecheck→lower→borrowck→**MIR verify**) chứng minh producer đúng tới MIR — KHÔNG run JIT. Đây test được toàn bộ producer mà không cần C5.
- **Lát 4 (JIT, sau):** giả-lập-C5 **đặc thù Outcome** — vì Outcome 2-value là use-case CỤ THỂ (disc+payload, không generic tuple). Có thể: sret-style 2-slot (mẫu Bậc D) HOẶC gỡ guard jit:1070 cho riêng BinaryOutcome (Cranelift native 2-return, premise nhẹ như C5 spike đã chứng minh). **Đây mở C5-cho-Outcome — premise nhẹ (C5 spike: Cranelift native, không vỡ value-model).**

**O nghiêng:** Lát 1-3 trước (producer + MIR-level fixture, check mode), chứng minh ĐÚNG. Lát 4 JIT = mở C5-cho-Outcome (gỡ guard 1070, Cranelift 2-return) — đây chính là điều kiện-mở-C5 (Outcome producer) tự thỏa. **C5+Outcome khép vòng:** Outcome producer = điều kiện mở C5; C5-JIT = backend cho Outcome. Làm cùng.

## 5. Phân lát đề xuất
- **OP.1 Typecheck:** verify+bổ sung return-type-match + E1024 exhaustiveness + E1025 (`~0` on T~E). Fixtures negative (E1024/E1025).
- **OP.2 Lower:** `~+ v`/`~- e` → 2-slot Outcome + `ReturnShape::BinaryOutcome` + `Return[disc,payload]`. Wire `OutcomeDiscriminant`/`OutcomeUnwrap`. Fixtures `check` mode (MIR verify producer đúng).
- **OP.3 JIT (= mở C5-cho-Outcome):** gỡ guard jit:1070 cho BinaryOutcome, Cranelift 2-return, caller `inst_results[0,1]`. Fixtures `run` (T~E end-to-end). Đây gỡ C5 khỏi Nhóm E (điều kiện mở: Outcome producer — tự thỏa).
- **OP.4 Match/unwrap:** `match o { ~+ x => .. ~- e => .. }` lower OutcomeDiscriminant+branch+unwrap. Fixtures run.

## 6. Câu hỏi cho G
1. **Scope OP:** OP.1-2 (producer tới MIR, check-mode fixture) trước, rồi OP.3 (mở C5-cho-Outcome JIT)? Hay G muốn dừng ở MIR (Err-at-JIT) tới khi quyết C5 riêng?
2. **C5 khép vòng:** Outcome producer = điều kiện-mở-C5; OP.3 chính là mở C5-cho-Outcome (premise nhẹ, Cranelift native). G đồng ý gộp OP.3 = un-defer C5-cho-Outcome (không generic tuple)?
3. **ADR?** Outcome producer là feature lớn + đụng ReturnShape ABI. O nghiêng **cần ADR** (nối ADR-0020 design-locked + ghi value-model 2-slot Outcome + JIT 2-return). G xác nhận?
4. **Bậc:** payload heap (String trong Outcome) = Bậc B/C. OP scope Bậc A (payload scalar i64) trước?
