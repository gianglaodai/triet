# ADR-0055 — CFG Tail-Expression: block-form function body evaluates to its tail expression

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-11. Khởi thảo Mentor O 2026-06-11, grounded từ 8 probe MIR.
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O (mổ xẻ `triet-lower`, dựng probe p1–p8).
- **Chữ ký:** O ✅ (grounded từ MIR thật + 8 probe) · G ✅ (ký duyệt + đóng dấu 2026-06-11).
- **Liên quan:** [ADR-0052](0052-outcome-abi-implementation.md) (`lower_outcome_return_values` 2-register), [ADR-0053](0053-heap-payload-outcome.md) (heap Outcome drop), [ADR-0054](0054-borrowck-drop-kills-liveness.md) (use-after-Drop nền). SPEC §các mục expression-based.

---

## 1. Context — tội ác ngữ nghĩa của ngôn ngữ expression-based

Triết tự xưng **expression-based**: một `Block` CHÍNH LÀ một `Expression`, và giá
trị của nó là **tail expression** (biểu thức cuối, không dấu `;`). Một lập trình
viên viết:

```triet
function classify(c: Color) -> Integer {
    match c {
        Red => 100,
        Blue => 200,
        Green => 300,
    }
}
```

kỳ vọng hàm trả về giá trị của `match`. **Compiler hiện tại ngầm vứt giá trị đó
và trả về `0`.** Đây là hành vi ngầm định khốn nạn — không diagnostic, không
cảnh báo, chỉ một con số sai lặng lẽ. Với ngôn ngữ expression-based, đây là lỗ
hổng ngữ nghĩa nền móng, phải khóa bằng ADR để không ai xé rào lại.

## 2. Root cause — ĐO TỪ CODE, KHÔNG ĐOÁN

Có **hai** đường hạ-bậc-block song song trong `triet-lower/src/lib.rs`, một đúng
một sai:

| Đường | Vị trí | Tail expression | Dùng cho |
|---|---|---|---|
| `lower_block()` | `lib.rs:866` | **VỨT** (line 879–881: `lower_expr(e)` rồi bỏ `Local`) | function-body Block + while-body |
| `Expr::Block` arm | `lib.rs:2131` | **ĐẨY ĐÚNG** (`result = lower_expr(e)`) | block-as-expression (`= {…}`, RHS let, match arm) |

`lower_function` (line 642–660) rẽ body theo dạng cú pháp:
- `FunctionBody::Block { block }` → `lower_block()` → vứt tail → rơi xuống
  synthetic `Return { values: vec![] }` (line 666–677) → **trả unit/0**.
- `FunctionBody::Expression { expr }` → `lower_expr()` + `lower_outcome_return_values()`
  + Return đúng (line 646–657) → **đúng**.

**Bằng chứng MIR (probe, không suy diễn).** Block-body, tail literal `7`, không `return`:

```
fn main(...) -> Integer {
  bb0: {
    StorageLive(_0)
    _0 = const 7          ← giá trị tính xong, nằm sẵn trong _0
    Return(())            ← Return mang vec![], vứt _0 → trả 0
  }
}
```

Ma trận probe (8 ca, đo trên `triet-driver run`):

| Probe | Dạng body | Tail | Kết quả | |
|---|---|---|---|---|
| p1 | `{ 7 }` | literal | **0** | BUG |
| p5 | `{ if…{a}else{b} }` | if/else | **0** | BUG |
| p7 | `{ match c {…} }` | enum match | **0** | BUG |
| p2 | `{ return 7; }` | — | 7 | ✓ (return tường minh) |
| p3 | `= 7` | — | 7 | ✓ (expr-body) |
| p8 | `= { match c {…} }` | enum match | **200** | ✓ (expr-body) |

**p7 vs p8 khác đúng MỘT ký tự `=`.** Gốc rễ tuyệt đối: sự tồn tại của hai
code-path trùng lặp. KHÔNG liên quan match/if (chúng chỉ là tail hay gặp), KHÔNG
liên quan JIT (MIR đã ra lệnh `Return(())`, JIT trả unit là làm đúng). Lỗi thuần
ở tầng **lowerer**.

## 3. Decision (G chốt phán quyết — KÝ DUYỆT 2026-06-11)

**Định lý khóa vào lịch sử:** *Một block-form function body LÀ một expression;
giá trị trả về của nó LÀ giá trị của tail expression — y hệt expr-body `= expr`.*
Sự phân biệt ngữ nghĩa giữa `FunctionBody::Block` và `FunctionBody::Expression`
là **giả tạo** và bị xóa sổ.

**HỢP NHẤT, không vá víu.** (G phán: cấm phương án hèn nhát bắt `lower_block` trả
`Option<Local>`.) `lower_function` coi cả hai dạng body như một `ExprId` và đẩy
qua CÙNG đường `lower_expr`:

```rust
let body_expr = match &func.body {
    FunctionBody::Block { block }     => Some(*block),
    FunctionBody::Expression { expr } => Some(*expr),
    FunctionBody::External { .. }     => None,
};
if let Some(e) = body_expr {
    let val = lower_expr(e, arena, &mut c)?;
    if c.is_open(c.cur) {                       // block kết thúc bằng `return` đã đóng cur
        let values = lower_outcome_return_values(val, &mut c);
        let span = arena.expression(e).span.clone();
        let cur = c.cur;
        c.term(cur, Terminator::Return { values, span });
    }
}
```

**Bốn điểm soundness bất khả xâm phạm:**

1. **Guard `c.is_open(c.cur)`** — bắt buộc. Block-body thường kết thúc bằng
   `return` tường minh (`{ return a+b; }`) đã tự đóng `cur`; không guard → ghi đè
   terminator / Return kép. Guard chỉ THÊM điều kiện, không đổi hành vi expr-body
   thuần (ở đó `cur` luôn mở → guard luôn pass).
2. **`lower_outcome_return_values`** — block-body Outcome-tail hiện chết kép
   (empty `vec![]` + không split 2-register ADR-0052). Route qua đây vá Scalar +
   Fat-Pointer + Outcome MỘT lần.
3. **Synthetic fall-through (line 666–677) GIỮ NGUYÊN** — lưới cho unit-fn
   rơi-khỏi-đuôi (không tail, không return); sau guarded Return nó thành no-op.
4. **`lower_block` GIAM LỎNG ở while-body** (line 1141) — ở đó VỨT tail là ĐÚNG
   (giá trị thân vòng lặp bị bỏ theo ngữ nghĩa). Không xóa hàm, không dead-code.

**Nguyên tắc parity — KHÔNG phát minh lại drop/escape.** Đường expr-body
tail-return HÔM NAY đã chạy đúng cả heap (fixture `100_endgame_string_roundtrip`
pass). Fix đạt parity bằng cách đi CHUNG đường, KHÔNG tự chế drop logic mới.

## 4. Teeth (bắt buộc đỏ→xanh — người lãnh task phải làm CHUYỂN)

Teeth HAI CHIỀU: mỗi ô có fixture **chạy đúng giá trị** SAU fix, và poison-revert
(gỡ fix, khôi phục `lower_block`-cho-function-body) khiến nó **trả 0/sai**.

| Tail kiểu | Scalar | Fat-Pointer (String/Vector) | Outcome (T~E) |
|---|---|---|---|
| literal trần | `{ 7 }`→7 | `{ "hi" }`→len đúng | `{ ~+ 5 }`→success |
| `if/else` | `{ if…{a}else{b} }` | String hai nhánh | — |
| enum `match` | `{ match…}`→arm đúng | String theo arm | error-arm |
| nested block | `{ { 7 } }`→7 | `{ { s } }` | — |
| **parity-return-heap** | — | `{ let s=…; s }` trả CHÍNH heap nó sở hữu | — |
| control: `return` sống | `{ return 7; }`→7 vẫn đúng (guard không phá đường cũ) | | |

**Ô `parity-return-heap` = RANH GIỚI SINH TỬ.** Mọi lỗi Ownership
(Use-After-Free, Double-Free, Drop sai chỗ) bị che giấu trong đường hạ-bậc-biểu-thức
sẽ bị LỘT TRẦN ở ô này. Fixture phải: chạy xanh + free-đúng-1 (không double-free
khi heap vừa được return vừa bị scope-pop Drop). Nếu ô này vỡ → đó là lỗ
expr-body có sẵn bị phơi ra, KHÔNG phải lỗi fix tạo ra — nhưng vẫn phải đóng
trước khi O ký.

## 5. Thứ tự thi công

1. Viết teeth ma trận §4 (fixtures `triet-driver/tests/fixtures/`) — ĐỎ trước fix.
2. Hợp nhất `lower_function` body-path theo §3.
3. Teeth CHUYỂN đỏ→xanh; poison-revert chứng minh quay lại 0.
4. Gate đầy đủ (`bash scripts/gate.sh`) — raw 4 mục.

## 6. Consequences

- **Tích cực:** xóa duplication hai body-path; khóa ngữ nghĩa expression-based;
  vá luôn Outcome-tail + Fat-Pointer-tail (chết kép trước đây).
- **Phạm vi:** 1 file (`triet-lower/src/lib.rs`), ~15 dòng ở `lower_function`.
  KHÔNG đụng MIR types, KHÔNG đụng JIT, KHÔNG đụng borrowck.
- **Rủi ro:** ô `parity-return-heap` có thể phơi lỗ drop-escape expr-body sẵn có
  → nếu lộ, mở task con đóng trước khi ký (không nuốt).
- **Ngoài scope (Luật 4 — KHÔNG tự mở rộng):** match-on-integer-literal hiện
  lowerer chưa support (`expected enum variant`); rác ở chiến dịch này, dẹp sang
  bên. Teeth match chỉ dùng enum.

## 7. Chỉ thị tác chiến cho người lãnh (KHÔNG blueprint thêm — §3 là khuôn)

- KHÔNG đụng `lower_block` ngoài việc để nó phục vụ while-body. KHÔNG xóa nó.
- KHÔNG bỏ guard `is_open` — nó là cạnh bén của `return`-giữa-thân.
- Counting/structural test ưu tiên route-lower (`lower_source` qua pipeline thật),
  KHÔNG hand-build `MirBuilder`.
- Ô `parity-return-heap` phải có máu (poison đỏ + free-đúng-1) — O sẽ teeth tay
  lại trên code cuối, không tin claim.

## 8. Amendment 2026-06-11 — DESCOPE 3 ô teeth (append-only, §3 nguyên vẹn)

**Bối cảnh:** trước khi viết teeth, probe từng ô §4 qua expr-body `= …` (chính
đường block-body sẽ đi sau fix). O tự verify (KHÔNG tin bảng người lãnh), MIR-grounded:

| Ô §4 | expr-body hôm nay | Verdict |
|---|---|---|
| if/else **heap** | `length` đọc về 0 (merge mất len/cap) | 🔴 BLOCKED |
| enum match **heap** | garbage (merge mất len/cap) | 🔴 BLOCKED |
| enum match **outcome** | MIR verify `arity expected 2 got 1` | 🔴 BLOCKED |

**Gốc rễ (độc lập ADR-0055):** branch value-merge sinh `_5 = move _4` — move 1-slot
(1 i64 = chỉ `ptr`). Heap 24-byte `{ptr,len,cap}` → mất len/cap; Outcome 2-slot
`{disc,payload}` → arity mismatch. Lỗ ở `Expr::If`/`Expr::Match` value-merge, KHÔNG
ở `lower_function` body-path. Tái hiện y hệt trong `= if…/= match…` (pre-fix), nên
KHÔNG liên quan ADR-0055.

**Phán quyết O (gác cổng định teeth-scope):** **(A) DESCOPE.** ADR-0055 tự khai
`~15 dòng 1 file, no JIT/borrowck` (§6); fix branch-merge multi-slot đụng branch-codegen
→ ngoài scope. ADR-0055 tiến hành với **9 ô sound** (literal/if/match/nested ×
scalar + heap-literal/heap-nested + parity-return-heap + outcome-literal).

**Ô `parity-return-heap` — O đã CHỨNG MINH sound, không chỉ claim:** MIR phơi
`Drop(_1); Return(_1)`; O đo (a) stress realloc sau return → `length=5`, status 0
trực tiếp, không SIGABRT; (b) explicit `return s` sinh MIR **identical** → pattern
heap-return chuẩn codebase (`100_endgame` PASS), do ADR-0054 Return-leniency cai
quản. ADR-0055 tail-form = parity thuần, 0 hành vi mới.

**Follow-up:** chiến dịch "if/match value-merge multi-slot" → **ADR-0056** (2 chữ ký
O+G trước implement; đụng `Expr::If`/`Expr::Match` lower + có thể JIT branch-codegen).

- **Chữ ký amendment:** O ✅ (grounded từ probe B1/B2/B3 + parity stress 2026-06-11) ·
  G ✅ (ký duyệt descope 2026-06-11 — thu hẹp teeth, §3 decision KHÔNG đổi).
