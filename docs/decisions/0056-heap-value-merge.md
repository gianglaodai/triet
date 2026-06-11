# ADR-0056 — Heap Value-Merge: type the if/match merge result so Fat-Pointers survive

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-11. Khởi thảo Mentor O 2026-06-11, grounded từ spike if-heap.
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O (mổ `Expr::If`/`Expr::Match` lower + spike type-result).
- **Chữ ký:** O ✅ (grounded từ probe B1/B2 + spike revert sha-identical) · G ✅ (ký duyệt + đóng dấu 2026-06-11).
- **Liên quan:** [ADR-0055 §8](0055-block-body-tail-expression.md) (descope nguồn — phơi 3 ô branch-merge), [ADR-0049](0049-fat-pointer-abi.md) (Fat-Pointer {ptr,len,cap}). **Phong ấn sang [ADR-0057]:** Outcome value-merge + Outcome-let-binding (KHÔNG thuộc ADR này).

---

## 1. Context — if/match bóp nghẹt tính biểu thức của dữ liệu Fat-Pointer

Triết expression-based: `if`/`match` PHẢI evaluate ra giá trị. Hôm nay với
**Fat-Pointer** (String/Vector — heap 24-byte `{ptr,len,cap}`), giá trị nhánh bị
**bóp về 1 word (chỉ `ptr`)** khi gộp về kết quả chung → len/cap rác:

```triet
function pick(n: Integer) -> String = if n == 0 { "xx" } else { "yyy" }
// length(pick(0)) → 0/garbage thay vì 2
```

Phơi ra khi viết teeth ADR-0055 (§8 descope). Lỗ độc lập ADR-0055 (tái hiện trong
`= if…/= match…` pre-fix).

## 2. Root cause — ĐO TỪ CODE + SPIKE, KHÔNG ĐOÁN

Hai construct cùng một tội (`triet-lower/src/lib.rs`):

| Construct | Result alloc | Ghi nhánh |
|---|---|---|
| `Expr::If` merge | `lib.rs:2201` `c.alloc_local()` **untyped** | `Assign{result, then_val}` (2205) · `{result, else_val}` (2221) |
| plain enum `Expr::Match` merge | `lib.rs:3082` `c.alloc_local()` **untyped** | `Assign{result, body_val}` (3179 EnumVariant · 3205 unit · 3243 wildcard) |

**Hai khuyết:** (1) result local cấp **UNTYPED** → mặc định scalar i64; (2) ghi
bằng `Statement::Assign` 1-local → JIT hạ `_5 = move _4` **1-word**. Heap mất len/cap.

**SPIKE chốt scope (O đâm rồi rút, revert sha-identical):**
- JIT `Assign` LÀ type-aware: `let y: String = x` → `_1 = move _0` copy đủ
  `{ptr,len,cap}` → `length=4` ✓. **JIT đã biết move Fat-Pointer KHI local typed.**
- Type result local của if-merge từ `then_val` → B1 if-heap **0/rác → 2** (đúng "xx").

→ **Fix LOWER-ONLY. JIT branch-codegen KHÔNG cần đụng.** Đây là phát hiện đắt giá:
JIT đủ thông minh sẵn; lỗ thuần ở việc Lowerer quên gắn type.

## 3. Decision (G chốt scope — KÝ DUYỆT 2026-06-11)

**Phạm vi KHOÁ:** chỉ Fat-Pointer (String/Vector) value-merge qua **if** + **plain
enum match**. Giải pháp: **bơm type chính xác cho `result` local từ giá trị nhánh.**

**Site 1 — `Expr::If` (lib.rs:2201):** result cấp SAU `then_val` → type tại alloc:
```rust
let result = c.alloc_local_ty(c.local_decls[then_val.0].ty.clone());
```

**Site 2 — plain enum `Expr::Match` (lib.rs:3082):** result cấp TRƯỚC vòng arm →
không type được tại alloc. **Patch `result` type tại write-site đầu** (idempotent —
typecheck đảm bảo mọi arm cùng type): tại mỗi `Assign{result, body_val}` (3179/3205/
3243) set `c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();` trước
khi push Assign. (Implementer chọn helper hoặc set-mỗi-write — miễn type cuối đúng.)

**Bốn lằn ranh bất khả xâm phạm:**
1. **CẤM tuyệt đối đụng JIT** (mệnh lệnh G). JIT typed-Assign đã đúng — spike chứng minh.
2. **KHÔNG đụng nullable-match** (2605/2618 đã `alloc_local_ty(payload_ty)` — typed sẵn).
3. **KHÔNG đụng outcome-match** (2862 `Unknown`) — phong ấn ADR-0057.
4. **Scalar merge KHÔNG ĐƯỢC regress** — type-từ-branch áp đồng đều, scalar vẫn i64.

## 4. Teeth (bắt buộc đỏ→xanh — route-lower, KHÔNG hand-build MirBuilder)

| Ô | Form | Sau fix | Poison-revert (result về untyped) |
|---|---|---|---|
| if-heap String | `= if n==0 {"xx"} else {"yyy"}` → `length` | 2 | 0/rác 🔴 |
| if-heap Vector | `= if … {vec a} else {vec b}` → `length` | len đúng | rác 🔴 |
| enum-match-heap String | `= match c { Red=>"xx", Blue=>"yyy" }` → `length` | đúng arm | rác 🔴 |
| enum-match-heap Vector | match → Vector theo arm | len đúng | rác 🔴 |
| **regression scalar** | if-scalar / match-scalar (fixtures 146/147) | vẫn đúng | — (không được vỡ) |
| **regression ADR-0055** | 9 fixture 143-151 | vẫn xanh | — |

**KHÔNG có ô Outcome** — Outcome-merge thuộc ADR-0057, sẽ vẫn đỏ sau ADR này; nếu
ai thêm ô Outcome vào teeth 0056 = lệch scope, REJECT.

## 5. Thứ tự thi công

1. Viết teeth §4 (fixtures heap if/match) — ĐỎ trước fix.
2. Site 1 (if) + Site 2 (match) type-result theo §3.
3. Teeth CHUYỂN đỏ→xanh; poison-revert (result→`alloc_local()`) chứng minh quay lại rác.
4. Regression: scalar fixtures + 9 fixture ADR-0055 vẫn xanh.
5. Gate đầy đủ (`bash scripts/gate.sh`) — raw 4 mục.

## 6. Consequences

- **Tích cực:** trả tính biểu thức cho if/match dữ liệu Fat-Pointer; ~2-4 dòng lower,
  0 JIT, 0 ABI.
- **Phạm vi:** 1 file `triet-lower/src/lib.rs`, 2 merge-site.
- **Phong ấn → ADR-0057 (Outcome Value-Flow & Let-Binding):** Outcome value KHÔNG
  round-trip sạch qua local thường — không chỉ merge (`match → ~+/~-` arity 2→1) mà
  cả `let r: T~E = ~+5; return r` (arity 2→0). Bệnh chung: hệ thống mù cách **move
  một StackSlot Outcome** giữa các Local. Cần mũi khoan tĩnh tâm điều tra JIT
  Outcome-slot move SAU khi ADR-0056 đóng. **KHÔNG đụng trong ADR này.**

## 7. Chỉ thị tác chiến cho người lãnh

- Type result **từ giá trị nhánh** (then_val / body_val), KHÔNG hardcode String —
  để Vector + scalar đều đúng qua cùng đường.
- CẤM đụng JIT, nullable-match, outcome-match.
- CẤM thêm ô Outcome vào teeth (lệch scope → ADR-0057).
- Counting/structural test route-lower (`lower_source`). Poison phải đỏ. Gate raw 4 mục.
- O teeth tay code cuối: poison result→untyped, verify if-heap + match-heap về rác,
  scalar + 9 fixture 0055 không regress.
