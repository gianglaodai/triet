# ADR 0089 — Concrete Loop CFG & Range Iteration (Amending ADR-0003)

> Status: **SIGNED (2026-07-26) — O ✅ G ✅.** G duyệt kiến trúc (§2 Typecheck Guard
> + §4 Drop-Model anchor `flush_all_for_return`) + lệnh bổ sung §2b (cấm silent-drop
> break-value). O soạn + verify claim parse_break bằng code. Xuất WO 0089-SLICE-1 sau dấu này.

> Scope quyết bởi G (2026-07-26): **Scope B — Slice 1**. Pragmatic engineering:
> ship `loop`/`break`/`continue` + `for i in <Range>` bằng CFG desugar tường minh,
> **KHÔNG đụng generic trait dispatch**. Trait-based `Iterator<T>` (ADR-0003) defer
> vô thời hạn tới khi generics chín.

## Context — bẫy câm-một-nửa + ADR-0003 chưa bao giờ land

Hiện trạng (đo 2026-07-26, file:line):

| Construct | Parser | Typecheck | Lower |
|---|---|---|---|
| `while` | ✓ | ✓ `check.rs:709` | ✅ `lib.rs:2100` (3-block: hdr/bdy/ext) |
| `for x in e` | ✓ | ⚠️ `check.rs:692` — chỉ `Type::Range` cho element thật; iterable khác → `Type::Unknown` **CÂM** | ❌ E1100 `lib.rs:2144` |
| `loop` | ✓ | ✓ `check.rs:722` | ❌ E1100 |
| `break`/`continue` | ✓ (`break x` ngoài loop → E0006) | ✓ no-op `check.rs:687` | ❌ E1100 |
| `Expr::Range` (`a..b`) | ✓ `expr.rs:111` | ✓ `Type::Range(inner)` `exprs.rs:224` | ❌ E1100 (không có arm) |

Hệ quả: `for i in 0..10 { ... }` **typecheck ra `i: Integer` rồi CHẾT E1100 ở lower** —
bẫy câm-một-nửa. `for x in vector` còn tệ hơn: typecheck câm `x: Unknown` (không lỗi)
rồi cũng E1100. Người dùng không chạy nổi một vòng lặp `for`/`loop` cơ bản.

**ADR-0003 (LIVE per ARCHIVE:79)** đặc tả trait `Iterator<T>`/`Iterable<T>` với
`next() -> T?` + desugar `for → loop`, NHƯNG (ADR-0003 dòng 64) **"NOT LANDED"** —
trượt qua mọi phase v0.2→v0.8, phụ thuộc generics + trait dispatch chưa có.
Đây là con thú kiêu hạo cấm động vào ở giai đoạn này.

## Decision

### §1 — Tombstone/Defer ADR-0003 trait protocol + phế "AI-first"

- **DEFER vô thời hạn** giao thức generic `Iterator<T>`/`Iterable<T>` của ADR-0003
  (`.iter()`/`.next()`/adapters `map`/`filter`/`zip`/`enumerate`). Gated trên
  generic trait dispatch trưởng thành — ngoài phạm vi rewrite hiện tại.
- **🪦 PHẾ rationale "AI-first phù hợp"** (ADR-0003 dòng 48). VISION tombstone
  "AI-first" 2026-06-22 — không đo, không bán. Rationale của iteration neo lại vào
  **coherence + craft**: một mô hình vòng lặp CFG tường minh, đọc được, borrowck
  nuốt được, không phụ thuộc runtime iterator value.
- ADR-0003 vẫn LIVE như **bản thiết kế tương lai** cho trait-protocol; ADR này
  AMEND phần roadmap: Slice 1 concrete đi TRƯỚC, trait-protocol là kỷ nguyên sau.

### §2 — Typecheck Guard (BẮT BUỘC — chấm dứt bẫy câm)

`Stmt::For` arm (`check.rs:692`) sửa: **iterable PHẢI là node `Expr::Range`**
(kiểm expr-kind, không chỉ kiểu tĩnh — vì Range-typed variable cũng chưa lower được):

- Iterable là `Expr::Range { start, end, inclusive }` với `start`/`end` cùng kiểu
  scalar-số (Integer/Long/…) → element-type = kiểu của `start`; bind `variable`;
  lọt xuống lower.
- **Mọi iterable khác** (Vector, HashMap, String, struct, enum, Range-typed
  non-literal, …) → **REFUSE NGAY tại typecheck** bằng mã mới
  **`E1052 NonRangeIterationUnsupported`** (namespace `triet::typecheck::E1052`),
  message trỏ ADR này + ADR-0003 (trait-iteration defer). Trả `Type::Unknown` cho
  element (dập cascade) SAU khi đã push lỗi — **KHÔNG câm `Unknown` không lỗi**.
- **CẤM `Type::Unknown` trôi xuống lower cho For.** Chỉ inline-Range lọt qua.

Message E1052 (theo ADR-0027 format):
```
E1052 NonRangeIterationUnsupported
Iteration over non-Range types is deferred (trait Iterator<T> not yet implemented).

[Fix 1] Use a Range loop over an index:
  Change `for x in <collection>` to `for i in 0..length(<collection>) { ... get(i) ... }`
```

### §2b — CẤM silent-drop break-value tại PARSER (G phát hiện, O verify code)

🩸 **Bẫy câm THỨ HAI, xác nhận `stmt.rs:169-184`:** `parse_break` parse
`value = if <terminator> None else Some(parse_expression(...)?)` — tức **CÓ** đọc
expression khi `break x`/`break 42` — rồi chỉ dùng `value` cho span, **trả về
`Stmt::Break` (unit) VỨT value đi**. Hệ quả: `break 42;` bị **nuốt câm** thành `break;`
— compiler tự ý bỏ giá trị user, không một tiếng động.

**Lệnh thép (G, Slice 1):** KHÔNG defer mù mờ, KHÔNG chấp nhận silent-discard.
- Sửa `parse_break` (`stmt.rs:169`): nếu `value.is_some()` → **ném `ParseError` mã mới
  `E0009 BreakWithValueNotSupported`** ngay tại parser (break-with-value defer toàn bộ
  Slice 1, kể cả trong `loop{}`). CẤM tuyệt đối nuốt giá trị.
- Lưu ý `E0006 BreakValueOutsideLoop` đã tồn tại (`error.rs:86`) với help "chỉ hợp lệ
  trong loop{}" — Slice 1 defer break-value HOÀN TOÀN nên E0009 rộng hơn/thay thế đường
  đó; D kiểm E0006 còn reachable không, KHÔNG xóa pre-existing nếu còn (surgical).

### §3 — Lowering CFG (desugar tường minh, tái dùng shape của `while`)

Thêm **loop-context stack** vào lowerer state (HIỆN KHÔNG TỒN TẠI — xác nhận
grep `loop_stack`/`break_target`/`continue_target` = 0 hit):

```rust
struct LoopContext {
    break_bb: BasicBlock,     // đích của `break` (= ext)
    continue_bb: BasicBlock,  // đích của `continue` (= hdr cho loop/while; = step cho for)
    drop_snapshot: usize,     // owned_locals.len() TẠI thời điểm vào loop-body scope
}
// Vec<LoopContext> — push khi vào loop, pop khi ra. break/continue đọc top.
```

**`loop { body }`** — 2 block:
```
cur → Goto hdr
hdr: <body>            // break→ext, continue→hdr
     Goto hdr          // back-edge (nếu body fall-through)
ext:                   // c.cur sau loop
```
`continue_bb = hdr`, `break_bb = ext`.

**`for i in start..end`** — desugar về CFG kiểu while + induction var + step block:
```
i = start              // Assign vào induction local (Integer, scalar/Copy — KHÔNG owned)
cur → Goto hdr
hdr: cond = i < end    // exclusive `..`; inclusive `..=` → i <= end. So sánh → Trilean!
     If cond → bdy else ext
bdy: <body>            // break→ext, continue→step
     Goto step
step: i = i + 1        // increment (Add — range-enforced trap per ADR-0044)
     Goto hdr          // back-edge
ext:
```
`continue_bb = step` (continue phải chạy increment rồi mới re-test — KHÔNG nhảy thẳng
hdr, tránh vô hạn). `break_bb = ext`. `inclusive` đọc TRỰC TIẾP từ `Expr::Range.inclusive`
(KHÔNG mang trong `Type::Range`); iterable là inline `Expr::Range` nên start/end lấy
được từ AST — **không cần Range runtime value** (Range vẫn không lower như standalone Expr).

**`break`** (top loop-context bắt buộc tồn tại, else = ICE/bug — nhưng parser đã ràng
break trong loop): emit drops cho `owned_locals[drop_snapshot..]` theo THỨ TỰ pop_scope
(reference trước, rồi LIFO `.rev()`), rồi `Goto break_bb`; đặt `c.cur` = block chết mới.

**`continue`**: giống break nhưng `Goto continue_bb`.

**`while`** (đã lower): wire thêm break/continue bằng CÙNG loop-context (`continue_bb=hdr`,
`break_bb=ext`) — chi phí gần-0, tránh nghịch lý "break cấm trong while". Included trong Slice 1.

### §4 — Soundness (drop-trên-nhảy-phi-cấu-trúc — cảnh báo G)

Cơ chế drop hiện có 2 kiểu (`lib.rs`): (a) `pop_scope` `:552` drop theo biên scope
tĩnh (drain snapshot.., reference-trước, `.rev()`); (b) `flush_all_for_return` `:659`
drop MỌI owned local cho `return`, **emit-không-clear** (Case-D: local live trước một
split phải drop trên MỌI đường exit). **KHÔNG có** cơ chế drop cho nhảy phi-cấu-trúc.

break/continue **mô phỏng chính xác pattern `flush_all_for_return`** nhưng phạm vi hẹp
`owned_locals[drop_snapshot..]` (đúng các owned local sinh trong loop-body tính tới điểm
nhảy, xuyên mọi nested scope — vì `owned_locals` phẳng, chỉ lớn dần; scope con đã đóng
thì đã drain):

- **emit-không-clear:** break/continue KHÔNG drain `owned_locals`. Sau nhảy, `c.cur` =
  block chết → `pop_scope` cấu trúc ở cuối body emit drop vào block chết (unreachable,
  vô hại) VÀ drain (giữ kế toán owned_locals nhất quán cho scope ngoài).
- **Đường fall-through (không break):** chỉ `pop_scope` drop (1 lần) trước back-edge.
- **Đường break:** chỉ break drop (1 lần) rồi chết. ⇒ mỗi owned local drop **đúng 1 lần
  trên mỗi đường**. Không leak, không double-free.
- **Refactor:** tách phần "sắp xếp + emit Drop cho một slice locals" thành helper
  `emit_scope_drops(&[Local])` mà `pop_scope` (rồi drain) và break/continue (không drain)
  cùng gọi — một nguồn thứ-tự-drop, tránh lệch.

**Borrowck: KHÔNG cần sửa.** `check_body_with` `checker.rs:508` chạy trên `build_cfg()`
thuần (worklist + fixpoint monotone, hội tụ qua back-edge — `checker.rs:552`+`:563` đã
propagate partial-move ngược back-edge cho While). CFG loop/for chỉ dùng `Goto`/`If`
(giống hệt While) ⇒ borrowck tự nuốt. Drop đặt đúng ở tầng lower ⇒ borrowck TỰ kiểm
E2450/UAF ở lối break (nếu ta đặt sai, borrowck bắt).

### §5 — Teeth (danh sách fixture nghiệm thu)

Positive:
- **T-loop-basic** (EXPECT): `loop { ...; if cond { break; } }` đếm/tổng đúng.
- **T-for-range** (EXPECT): `for i in 0..5 { sum = sum + i }` → 10 (exclusive).
- **T-for-range-inc** (EXPECT): `for i in 0..=5` → 15 (inclusive).
- **T-continue** (EXPECT): `for i in 0..N { if skip { continue; } ... }` — continue chạy
  step, không vô hạn, không double-count.
- **T-nested-break** (EXPECT): loop lồng, `break` chỉ thoát loop trong (loop-context stack đúng).

Soundness (counting-harness, FREE dedup con-trỏ):
- **T-break-drop** ⭐ (G mandate a): `loop { let s = "x"; if cond { break; } }` — FREE=1
  (s drop đúng 1 lần trên đường break). **Poison:** bỏ emit-drops của break → leak
  (FREE=0) hoặc structural double → counting bắt. Poison hai chiều.
- **T-break-borrow** (G cảnh báo dangling): break ra khỏi loop có borrow local → borrowck
  thấy drop ở exit-edge, không lọt UAF, không false-E2450.

Negative (guard typecheck — G mandate b):
- **T-for-vector-refuse** ⭐: `for x in <vector>` → **E1052 tại typecheck** (KHÔNG E1100
  lower). Fixture `// ERROR: E1052`. Poison guard (bỏ nhánh refuse) → rơi E1100 lower
  = chứng minh guard chặn đúng tầng.
- **T-break-value-refuse** ⭐ (G mandate — chấm dứt silent-drop §2b): `break 42;` →
  **E0009 tại PARSER** (`// ERROR: E0009`), KHÔNG nuốt câm thành `break;`. **Poison:**
  revert `parse_break` về silent-discard (bỏ nhánh ném E0009) → `break 42;` parse lọt
  thành unit `Stmt::Break` = giá trị bị nuốt câm ⇒ fixture đỏ (expected E0009, got no-error).

### §6 — Out of scope (defer, ghi rõ để không câm)

- Trait `Iterator<T>`/`Iterable<T>`, `.iter()`/`.next()`, adapters — §1 defer.
- `for x in Vector/HashMap/String` — refuse E1052 (Slice 2+, cần index-loop hardcode
  hoặc trait).
- `break x` break-with-value — refuse rõ (§5 T-break-value-defer).
- `drain` (Vector/HashMap consume + tombstone mỗi phần tử) — WO RIÊNG sau, họ hàng
  move-out ADR-0082; KHÔNG trong Slice 1.
- Range-typed **variable** (`let r = 0..10; for i in r`) — refuse E1052 (chỉ inline-Range).
- Increment overflow ở `..=` cận trần range: `i = i + 1` sau vòng cuối có thể trap per
  ADR-0044 — chấp nhận (nhất quán range-enforcement), ghi chú.

## Open questions
1. ~~`break x` parse thành gì?~~ **ĐÓNG (O verify `stmt.rs:169-184`):** silent-discard →
   unit `Stmt::Break`. Quyết: parser ném E0009 (§2b). Không còn open.
2. `i` induction var: Slice 1 là local thường (user gán lại `i` ảnh hưởng vòng — giống
   while-desugar). Chấp nhận cho Slice 1; fresh-per-iteration binding defer nếu cần.

## Sites (khi implement — WO sẽ chốt)
1. **Parser** `stmt.rs:169` (`parse_break`) + `error.rs` (thêm `E0009 BreakWithValueNotSupported`
   variant) — §2b cấm silent-drop break-value.
2. **Typecheck** `check.rs:692` (`Stmt::For` arm) + `error.rs` (thêm E1052 variant).
3. **Lower** `crates/triet-lower/src/lib.rs`: state loop-context stack; arm `Stmt::Loop`,
   `Stmt::Break`, `Stmt::Continue`, `Stmt::For` (thay E1100 catch-all `:2144`); wire
   break/continue vào `Stmt::While` `:2100`; helper `emit_scope_drops`.
4. **Borrowck** — KHÔNG chạm (§4).
5. **Schema** — For/Loop/Break/Continue đã có (schema:1329-1353); KHÔNG đổi schema.

## Signatures
- **O: ✅ (2026-07-26)** — soạn + verify claim parse_break (`stmt.rs:169-184` silent-discard)
  + borrowck CFG-generic (`checker.rs:552/563`) + While-shape (`lib.rs:2100`) bằng code. Verify
  máu sẽ chạy sau khi D implement (teeth poison hai chiều §5).
- **G: ✅ (2026-07-26)** — duyệt kiến trúc §2/§4, phát hiện + lệnh §2b break-value reject.
