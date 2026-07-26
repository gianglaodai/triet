# ADR 0089 — Concrete Loop CFG & Range Iteration (Amending ADR-0003)

> Status: **SIGNED + IMPLEMENTED (2026-07-26) — O ✅ G ✅.** Slice 1 land trọn:
> `loop`/`break`/`continue` (CFG primitives) + `for i in <Range>` desugar + guard
> non-Range (E1052) + guard break-value (E0009) + guard break/continue-outside-loop
> (E1143). O verify máu: gate `0·clean·0·472·0`; poison 3 mũi ĐỎ (break-drop permanent
> counting FREE 3→2, guard E1052→E1100, guard E0009→silent-discard); E1143 fresh-binary
> confirmed (stale-binary E1140 caught per luật #12). SPEC §7.2 + ADR-0086 đồng bộ honest.

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

**`break`** (top loop-context bắt buộc tồn tại — **ĐÍNH CHÍNH (cleanup pass,
2026-07-26): parser KHÔNG ràng break/continue trong loop.** `E0006
BreakValueOutsideLoop` có 0 điểm dựng trong parser (dead code, chưa từng
enforce) và typecheck no-op `Stmt::Break`/`Stmt::Continue` — nên top-level
`break;`/`continue;` NGOÀI mọi loop thực sự lọt tới lowerer. Guard phòng thủ
tại đây (Track B rule #1: never panic on user input) refuse bằng mã riêng
**`E1143 BreakContinueOutsideLoop`** thay vì coi else-nhánh là ICE/bug):
emit drops cho `owned_locals[drop_snapshot..]` theo THỨ TỰ pop_scope
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
- **T-break-drop** ⭐ (G mandate a) — **CHỐT permanent, CLEANUP pass 2026-07-26**:
  fixture 477 (`// EXPECT: 3`, value-only) là VACUOUS cho soundness (leak không đổi
  exit code); tooth thật là `crates/triet-driver/tests/break_drop_counting.rs`
  (`break_path_frees_heap_local_each_iteration`) — `loop { let s = "x"; i+=1; if i==3
  { break } }` 3 vòng, FREE=3 (2 structural back-edge + 1 break-path). **Poison
  verify (D, trước khi cắm assert):** bỏ `emit_scope_drops` ở arm `Stmt::Break` →
  FREE=2 (đo thật, không bịa) → test đỏ.
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
- **T-break-outside-loop-refuse** ⭐ (CLEANUP pass, honesty item b): `break;` top-level
  ngoài mọi loop → **E1143 tại lower** (KHÔNG mượn E1140 UndefinedLocal). Fixture 478
  `// ERROR: E1143` + `crates/triet-lower/tests/diagnostics.rs::e1143_break_continue_outside_loop_code_via_fixture_478`
  (khóa `err.code()`). **Poison verify (D):** đổi `#[diagnostic(code(...))]` sang mã
  giả → `diagnostics.rs` đỏ (message-substring fixture KHÔNG đỏ theo poison này —
  message field hardcode literal "E1143:" độc lập với thuộc tính `code()`, xem báo cáo).

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

## §AMEND — Slice 2a: `for item in <Vector>` copy/by-value sugar

> Status phần này: **SIGNED + IMPLEMENTED (2026-07-26) — O ✅ G ✅.** Land trọn: for-item
> Vector by-value sugar (scalar + bare copy-Struct), desugar raw-shim infallible-get, guard
> **E1053** thắt CHÍNH XÁC khớp lowerer (`is_scalar() || (UserStruct && is_copy_aggregate)` —
> Enum/Nullable/heap refuse tại typecheck, KHÔNG lọt E1100 lower). O verify máu: gate
> `0·clean·0·479·0`; poison ĐỎ (guard broad→485 E1100 trap tái mở; handle-alias→SIGABRT
> D+O; guard E1053 load-bearing); counting container FREE=1 lvalue+rvalue (emit_shim_call
> `lib.rs:1783` gánh ownership — dòng `if !is_lvalue push_owned` redundant ĐÃ xóa). §2a.3.1
> handle double-free tránh bằng tái-dùng-local (không alias local mới).

Mở `for item in v` với `v : Vector<T>` khi **T là Copy** (scalar hoặc copy-aggregate).
Desugar về index-loop tái dùng TRỌN CFG Slice 1 — **KHÔNG generic trait, KHÔNG
consume, KHÔNG tombstone** (element đọc ra bằng Copy, `v` còn nguyên sau vòng).

### §2a.1 — Phát hiện (O probe 2026-07-26)
`for i in 0..len(v) { ... }` **ĐÃ CHẠY** trên Slice 1 (`len(v)` là `end` của inline
`Expr::Range`, probe → đúng). Blocker của "đọc phần tử" là `get(v,i)` trả `T?` +
`!!` (ForceUnwrap) **chưa lower** (E1100) — nhưng Slice 2a **KHÔNG cần `!!`**: desugar
dùng **infallible internal get** (in-bounds nên OOB-null bất khả), bind `item : T`.
`!!` là nợ độc lập (Slice 2c, KHÔNG đi ké — G lệnh).

### §2a.2 — Typecheck guard (mở Vector-Copy, refuse Vector-heap)
`Stmt::For` arm (`check.rs:704`, sau Slice 1): thứ tự quyết:
1. iterable node là `Expr::Range` → luồng Slice 1 (Range element).
2. else `iter_ty == Type::Vector(inner)` (`types.rs:40`):
   - `inner.is_scalar()` (`types.rs:147`) **HOẶC** `inner.is_copy_aggregate()` (`types.rs:238`)
     → **CHO PHÉP**; element-type = `(*inner).clone()`; bind `item : inner`.
   - else (`inner.is_heap()` — String/Vector/HashMap — hoặc heap-bearing struct,
     tức `!is_copy_aggregate()`) → **REFUSE** mã mới **`E1053
     HeapVectorByValueIterationUnsupported`** (`triet::typecheck::E1053`; E1052 cao
     nhất hiện dùng). Message trỏ drain (Slice 2b).
3. else (HashMap, String, struct thường, Range-typed-variable, MethodCall như
   `.drain()`/`.enumerate()`, …) → **E1052** (như Slice 1, không đổi).

🚩 **CHỐT CHẶN THÉP (G):** by-value copy một phần tử heap = **alias con trỏ heap →
DOUBLE FREE** khi cả `item` (owned) lẫn `v[i]` cùng drop. Nên Vector-heap PHẢI
refuse tại typecheck, KHÔNG lọt xuống lower/JIT. `get`-builtin đã refuse heap-element
(E1047 `exprs.rs:1179`) — nhưng `for` là bề mặt RIÊNG, cần guard riêng E1053 (thông
điệp đúng ngữ cảnh iterate, trỏ drain).

### §2a.3 — Lowering desugar (`Stmt::For`, nhánh Vector)
Sau khi match `Expr::Range` thất bại, lower iterable thành 1 local; nếu
`local_decls[iter_local].ty == MirType::Vector(inner)` (`mir:496`) → desugar:
```
iter_local = <base handle của iterable>   // §2a.3.1 — KHÔNG alias handle vào owned_local mới
__len = len(iter_local)         // __triet_vector_len shim (i64) — tính MỘT lần trước vòng
__i = 0
cur → Goto hdr
hdr: cond = __i < __len         // Lt → Trilean!
     If cond → bdy else ext
bdy: item = <infallible-get>(iter_local, __i)   // shim theo inner kind (dưới), bind item : inner (KHÔNG T?)
     <body>                     // break→ext, continue→step (loop-context như Slice 1)
     Goto step
step: __i = __i + 1 ; Goto hdr
ext:
```

#### §2a.3.1 — ⚠️ HANDLE-ALIASING = DOUBLE-FREE CONTAINER (cảnh báo thép G)
Vector là **handle 8-byte**. Nếu desugar tạo một `owned_local __vec` MỚI rồi
`Assign(__vec = v)` (copy handle), thì `__vec` VÀ `v` cùng nằm trong `owned_locals`
→ `pop_scope`/scope-exit phát **HAI Drop lên CÙNG buffer** = **double-free CONTAINER**.
Quy tắc chịu lực:
- **iterable là lvalue có tên** (`Expr::Variable` → `for item in my_vec`): `iter_local`
  = **chính local sẵn có của `my_vec`** (`c.vars[name]`). TUYỆT ĐỐI KHÔNG `alloc_local`
  + `push_owned` thêm. `len`/`get` chỉ ĐỌC handle (read-use), không consume. `my_vec`
  drop đúng 1 lần ở scope-exit CỦA NÓ, không phải của vòng lặp.
- **iterable là rvalue** (`for item in make_vector()`): `iter_local` = temp từ
  `lower_expr`; temp này PHẢI owned-tracked để drop **đúng 1 lần ở cuối vòng** (không
  leak, không double). D phải MAP TRACE (luật 20): xác nhận `lower_expr` owned-track
  temp heap-rvalue thế nào; nếu chưa → `push_owned(iter_local)` một lần, drop ở `ext`.
- **Phân biệt bằng expr-kind:** `matches!(arena.expression(iterable).node, Expr::Variable(_))`
  → nhánh lvalue; else → nhánh rvalue. (Param `&0 Vector`/`&0 mutable Vector` cũng là
  Variable → lvalue, KHÔNG drop — đúng vì borrow không sở hữu.)
**Infallible-get theo inner kind** (tái dùng shim `get` sẵn, KHÔNG shim mới):
- `inner` scalar → `__triet_vector_get(__vec, __i)` (`mir_lower.rs:5936`, trả i64 raw),
  bind `item : inner` trực tiếp (KHÔNG wrap Nullable — in-bounds bảo đảm ≠ NULL_SENTINEL).
- `inner` copy-aggregate (Struct Copy) → `__triet_vector_get_copy` (`mir_lower.rs:4007`,
  sret), bind `item : Struct`.
- loop-context: `break_bb=ext`, `continue_bb=step` (giống for-Range).
- **KHÔNG move-out, KHÔNG tombstone:** `__triet_vector_get`/`get_copy` COPY bytes, `v`
  giữ nguyên len/buffer. `item` scalar/copy-agg KHÔNG owned-tracked (không heap) → không
  Drop. Sau vòng, `v` vẫn owned bởi caller, drop bình thường 1 lần.

### §2a.4 — Soundness
- **Copy phần tử ⇒ không alias heap-element:** guard §2a.2 loại mọi phần tử heap;
  scalar/copy-agg copy bytes thuần, không con trỏ heap chia sẻ ⇒ không double-free element.
- **Không alias handle-CONTAINER (§2a.3.1):** lvalue → không owned_local mới; rvalue →
  owned đúng 1 lần. Chống double-free CONTAINER (bãi mìn G).
- **`v` bất biến:** không op nào chạm len/buffer của `v` ⇒ sau vòng `len(v)` không đổi.
- **Borrowck:** KHÔNG chạm (CFG chuẩn Goto/If như §4; `v` chỉ đọc — read-use, không move).

### §2a.5 — Teeth
- **T2a-scalar** (EXPECT): `for x in v { sum += x }` trên `Vector<Integer>` → tổng đúng.
- **T2a-copy-struct** (EXPECT): `for p in pts { sum += p.x }` trên `Vector<CopyStruct>` → đúng.
- **T2a-intact** ⭐ (EXPECT): `let v=...; for x in v {}; return len(v)` — (1) len KHÔNG
  đổi (copy-không-consume; poison: đổi infallible-get→pop → len giảm → đỏ); (2) **`v` ra
  scope cuối main → exit 0 SẠCH, KHÔNG SIGABRT** (chống double-free CONTAINER §2a.3.1;
  poison: alias handle vào owned_local mới → double-free → subprocess đỏ). **Counting tooth
  (O verify):** `__triet_vector_free` đếm = 1 cho container, KHÔNG 2.
- **T2a-rvalue** ⭐ (EXPECT + counting): `for x in make_vector() {}` — container rvalue
  drop **đúng 1 lần** (FREE=1: không leak FREE=0, không double FREE=2). Chứng minh nhánh
  rvalue owned-track đúng.
- **T2a-heap-refuse** ⭐ (ERROR E1053): `for s in string_vector { }` → **E1053 tại
  typecheck** (KHÔNG E1100 lower, KHÔNG JIT). **Poison (O verify):** gỡ nhánh refuse E1053
  → for-loop lower→JIT→**double-free SIGABRT** (subprocess tooth) = refuse load-bearing.
- **T2a-break/continue** (EXPECT): break/continue trong for-Vector hoạt động (loop-context).

### §2a.6 — Out of scope (Slice 2a)
- Vector-heap iterate (String/Struct-heap) → refuse E1053, **đường consume = drain (Slice 2b)**.
- HashMap iterate, String iterate → E1052 (chưa mở).
- `for item in v.drain()`/`.enumerate()` (MethodCall) → E1052 (Slice 2b/trait defer).
- `!!` ForceUnwrap → nợ độc lập Slice 2c (G lệnh tách).
- `item` mutable / ghi ngược vào `v[i]` (`set`) → KHÔNG (set-builtin không tồn tại).

### §2a.7 — Sites
1. **Typecheck** `check.rs:704` (For arm — thêm nhánh Vector) + `error.rs` (thêm E1053).
2. **Lower** `lib.rs` `Stmt::For` (thêm nhánh MirType::Vector sau khi Range-match fail;
   infallible-get theo inner kind; loop-context như for-Range).
3. **Borrowck / Schema / JIT shim** — KHÔNG chạm (tái dùng `__triet_vector_get`/`_copy`/`_len`).

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
   break/continue vào `Stmt::While` `:2100`; helper `emit_scope_drops`. `break`/`continue`
   với loop-context stack rỗng (top-level, ngoài mọi loop — xem đính chính §3) → mã riêng
   **`E1143 BreakContinueOutsideLoop`** (`LowerError::break_continue_outside_loop`, ADR-0086
   amend), KHÔNG mượn `E1140 UndefinedLocal`.
4. **Borrowck** — KHÔNG chạm (§4).
5. **Schema** — For/Loop/Break/Continue đã có (schema:1329-1353); KHÔNG đổi schema.

## Signatures
- **O: ✅ (2026-07-26)** — soạn + verify claim parse_break (`stmt.rs:169-184` silent-discard)
  + borrowck CFG-generic (`checker.rs:552/563`) + While-shape (`lib.rs:2100`) bằng code. Verify
  máu sẽ chạy sau khi D implement (teeth poison hai chiều §5).
- **G: ✅ (2026-07-26)** — duyệt kiến trúc §2/§4, phát hiện + lệnh §2b break-value reject.
