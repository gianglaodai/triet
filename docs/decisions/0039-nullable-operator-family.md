# ADR 0039 — Họ toán tử Nullable `?`-family (`?+>`, số phận `?0>`, cấm `?->`)

**Trạng thái:** **Đã duyệt — DESIGN LOCK** (Author, 2026-06-05; Mentor O verified
ADR-0020 §3.1 flatten + lexer precedent).
**Implementation:** **DEFERRED** — chờ nullable/Outcome lowering (Bậc B/C).
Backend hiện chưa lower được cả `?.`.

## Bối cảnh

Câu hỏi gốc của author: cú pháp nào thay map/flatMap (Monad) cho `T?` thuần?
Họ Outcome (`~+>`/`~0>`/`~->`, ADR-0020) đã phủ `T~E`/`T?~E`; `T?` thuần mới có
`?.` (safe call) + `?:` (Elvis) — thiếu transformer tổng quát (map giá trị không
qua member access, ví dụ `opt ?+> |s| parse(s)`).

Nguyên tắc dẫn đường: **đối xứng hình thái** — họ Outcome dùng tiền tố `~`, họ
Nullable dùng tiền tố `?`. Học một họ biết họ kia, hợp AI-first. Nhưng đối xứng
là phương tiện cho tính dễ đoán, không phải mục tiêu tự thân — điều khoản 2
dưới đây giết một token nhân danh chính nguyên tắc này.

## Điều khoản 1 — `expr ?+> |bind| body` (map + flatMap hợp nhất)

**Semantics** (đối xứng `~+>` ADR-0020 §3.1):
- `expr` có giá trị thực → bind vào `bind`, evaluate `body`, kết quả thay giá trị.
- `expr` là null (`~0`) → **pass through unchanged** (null đi tiếp, body không chạy).

**Body return** (kế thừa trực tiếp ADR-0020 §3.1 "Body return", dòng 378-380):
- Plain `U` → auto-wrap thành `U?`. Đây là **map**.
- Nullable `U?` → **auto-flatten**; không bao giờ sinh `U??`. Đây là **flatMap**.
  Quy tắc flatten này KHÔNG mới — `~+>` đã flatten nested outcome từ ADR-0020
  (§3.1: "Outcome `T'~E` hoặc `T'?~E` (same error type) → flatten; nested
  outcome unfolded"). Bản nullable còn sạch hơn: không cần ràng buộc "same
  error type" vì null chỉ có một dạng.
- Early-return form (`return ...`, `panic(...)`) → exit enclosing function,
  y như Outcome.

```triet
// map: body trả String (plain) → kết quả String?
let name: String? = get_name() ?+> |n| n.to_uppercase()

// flatMap: parse trả Integer? → auto-flatten, kết quả Integer? (KHÔNG Integer??)
let n: Integer? = get_input() ?+> |s| parse(s)
```

**Hệ quả:** `T??` (nested nullable) tiếp tục KHÔNG được định nghĩa trong Triết —
auto-flatten đảm bảo không đường nào sinh ra nó qua `?+>`.

## Điều khoản 2 — `?:` RHS là Expression; KHÔNG sinh `?0>`

**Chốt cứng trong SPEC:** vế phải của Elvis `?:` là một **Expression** — bao gồm
Block expression và `Return` (Return là Expr, `ast_expr.rs:131`):

```triet
let val = opt ?: {
    log("fallback")
    default_val()
}
let user = find_user(id) ?: return ~- AppError.NotFound   // guard pattern
```

Vì `?:` đã phủ cả fallback ngắn lẫn block/guard, **`?0>` là cú pháp thừa — bị
giết từ trong trứng** (Simplicity First, "one way to do it"). `~0>` của Outcome
tồn tại vì Outcome KHÔNG có Elvis (`~:` đã bị giết ở ADR-0020 §3.7); Nullable
có `?:` rồi thì không nhân bản.

> Lưu ý phạm vi: Triết **không có `throw`** (kernel language, không exception —
> mọi early-exit khác return là `panic(...)`, ADR-0020 §3.1). `break`/`continue`
> là loop-control (SPEC §quanh 899-915), tính hợp lệ trong RHS Elvis theo luật
> chung của Expression context, ADR này không cam kết riêng.

## Điều khoản 3 — Cấm tuyệt đối `?->`: **E1041 NullableHasNoErrorState**

`T?` chỉ có 2 cực: giá trị thực (`+`) và null (`0`). KHÔNG có cực âm (error).
Dev gõ `opt ?-> |e| ...` → compile error tức thì:

```
E1041 NullableHasNoErrorState
Kiểu `T?` không có trạng thái lỗi.
[Fix 1] Use `T~E` (Outcome) if you need error handling, then `~-> |e| body`.
[Fix 2] Use `?:` to provide a default when the value is null.
```

(Format per ADR-0027. Mã E1041 = mã typecheck tự do kế tiếp sau E1040.)

**Implementation note:** reserve token `?->` trong lexer (lex được nhưng bị
từ chối có chủ đích) để bắn E1041 với diagnostic đẹp, thay vì để nó vỡ thành
`?` + `->` và chết bằng parse error mơ hồ — cùng kỹ thuật lexer-refuses đã dùng
cho `~?`/`~:` deprecated (ADR-0020 §3.7).

## Lexer

`?+>` (và token reserved `?->`) là 3-char compound token, **không whitespace
bên trong**, longest-match đứng TRƯỚC `?~`/`?.`/`?:`/`?` — cùng kỹ thuật
`?~` phải đứng trước `Question` hiện tại (`token.rs:267-269`). Không va chạm:
Triết không có ternary `a ? b : c`, không có operator `+>` độc lập.
Precedence: cùng tầng với họ `~+>`/`~->` (postfix transformer, SPEC §4.6).

## Không làm

- **`?0>`** — thừa, xem điều khoản 2.
- **`?->`** — cấm vĩnh viễn, E1041 (điều khoản 3). Không phải "defer".
- **`T??` nested nullable** — vẫn không định nghĩa; auto-flatten né nó.
- **`~+>` áp trực tiếp lên `T?` thuần** — không; `T?` dùng họ `?`, Outcome dùng
  họ `~`. Hai họ tách bạch theo tiền tố, đó là điểm đối xứng.

## Bảng tổng kết hai họ

| Hành động | Nullable `T?` | Outcome `T~E` / `T?~E` |
|---|---|---|
| Safe member access | `?.` | — (unwrap explicit qua `~+>`/`~->`) |
| Fallback (ngắn + block + guard) | `?:` (RHS = mọi Expression) | — (`~:` đã giết, ADR-0020 §3.7) |
| Map + flatMap cực dương | `?+> \|v\| body` | `~+> \|v\| body` |
| Xử lý cực không (null) | `?:` | `~0> body` (chỉ `T?~E`) |
| Xử lý cực âm (error) | ❌ **E1041** | `~-> \|e\| body` |

## Tham chiếu

- [ADR-0020](0020-outcome-error-handling.md) §3.1 (semantics + Body return +
  flatten — nguồn kế thừa trực tiếp), §3.7 (`~:`/`~?` deprecated, lexer refuses).
- [ADR-0027](0027-diagnostic-format-standard.md) — format diagnostic E1041.
- SPEC.md §quanh 339-342 (`?.`/`?:`), §quanh 1345 (Elvis precedence — cần cập
  nhật thêm câu "RHS là Expression" per điều khoản 2), §quanh 899-915
  (`break`/`continue`).
- `crates/triet-lexer/src/token.rs:267-304` — họ `?` hiện có + longest-match
  precedent.
- [ADR-0038](0038-comparable-trait-deferred.md) — cùng pattern "design lock,
  implementation deferred".
