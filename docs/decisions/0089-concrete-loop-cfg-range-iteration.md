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

## §AMEND — Slice 2b: `for item in <Vector>.drain()` consuming iteration (move-out)

> Status phần này: **🚧 O ĐỀ (2026-07-26) — chờ G + Giang ký ban hành.** CHƯA một dòng
> code. Scope Giang/G chốt 2026-07-26: **Vector<T>.drain() ONLY** (HashMap.drain() BÁC —
> "từng pháo đài một"). Kiến trúc G duyệt: **desugar về vòng `pop_front`** (0 shim JIT mới,
> 100% mảnh proven), chấp nhận O(N²) correctness-first.

Mở `for item in v.drain()` — **tiêu thụ** `v` phần tử một, move-out **by-value** từng
`item : T` cho **MỌI T** (kể cả heap: `Vector<String>`, `Vector<User{String}>`). Đây là
đường consume mà Slice 2a REFUSE (E1053 copy=alias): drain **chuyển quyền sở hữu** → hết
alias → heap-element hợp lệ. Continuation move-out ADR-0082 §AMEND-2, **KHÔNG ADR-nền mới**.

### §2b.1 — Phát hiện (O recon 2026-07-26, file:line)

`drain` = **100% mảnh ĐÃ PROVEN**, không cần shim JIT/borrowck/schema mới:

| Mảnh | Trạng thái | Bằng chứng |
|---|---|---|
| loop/break/continue CFG | ✅ Slice 1 | `lib.rs:2325` for-arm, `loop_stack` |
| `pop_front(v)` move-out + **len-- tombstone** | ✅ ADR-0082 §AMEND-2 | shim `mir_lower.rs:4491`; `mutates_arg:Some(0)`, `arg_consumes:[false]` (`triet-mir/lib.rs:1194`) |
| `pop_front → T?` + `match ~+/~0` trên **String** | ✅ end-to-end | fixture **347** `vector_string_pop_front_run`; **351** shift nhiều phần tử |
| pop trên `Vector<UserStruct-heap-bearing>` (String bên trong) | ✅ allocator THẬT | fixture **338** `vector_userstruct_pop_run` |
| `v.drain()` parse | ✅ `Expr::MethodCall{receiver,method,args}` | `expr.rs:965` |

### §2b.2 — `.drain()` là FOR-GUARD ĐẶC QUYỀN (điều kiện thép G #1)

`drain` **KHÔNG đăng ký thành method chung** trong symbol table. Nó chỉ có nghĩa ở vị trí
for-iterable. `Stmt::For` arm (`check.rs:692`) kiểm **expr-kind TRƯỚC** khi infer generic:
- iterable là `Expr::MethodCall { receiver, method == "drain", arguments == [] }` →
  infer **CHỈ `receiver`** (né E1041 no-matching-overload), rồi guard §2b.3.
- `v.drain()` đứng độc lập (`let x = v.drain();` / `v.drain();`) → đi đường MethodCall
  thường → **E1041** (method not found). CẤM lọt.

### §2b.3 — Typecheck guards (fail-closed, refuse-over-guess)

Trong nhánh drain, sau khi infer `receiver`:
1. `receiver_ty == Type::Vector(inner)` **và `receiver` KHÔNG là reference** →
   - **`inner == Type::Nullable(_)`** → **REFUSE E1053** (điều kiện thép G #4): `Vector<T?>`
     drain đẻ `pop_front : (T?)? = Nullable(Nullable(_))` = **double-nullable** — vùng cấm
     ADR-0088 (get-family V=Nullable đã refuse E1051). Message trỏ ADR-0088 defer, KHÔNG thả
     rông. **SOUNDNESS-TRƯỚC-SYNTAX**: chưa có bằng chứng an toàn ⇒ refuse.
   - else → **CHO PHÉP** MỌI `inner` (scalar / copy-agg / **heap** String/Vector/HashMap /
     heap-bearing struct/enum); element-type = `(*inner).clone()`; bind `item : inner`.
2. `receiver_ty` là **reference** (`&0 Vector` / `&0 mutable Vector` / `&mutable Vector`) →
   **REFUSE E1053** (điều kiện thép G #2): drain = consuming mutation, KHÔNG được qua
   mượn-chia-sẻ. Slice 2b chỉ nhận **owned local hoặc rvalue** Vector. Borrow-receiver drain
   = mở rộng sạch tương lai (`&mutable` có thể mở sau; refuse cả hai lúc này = fail-closed).
3. receiver là **HashMap / String / kiểu khác**, hoặc method **≠ "drain"** (`v.other()`) →
   **E1052** (như Slice 1/2a — non-Range/non-drain iteration defer).

### §2b.4 — Lowering desugar (`Stmt::For`, nhánh drain — TRƯỚC nhánh Range/Vector)

Match `Expr::MethodCall{method=="drain"}` ở đầu `Stmt::For`. Lower `receiver` thành
`iter_local` (lvalue → own local sẵn; rvalue → owned temp — **owned-track đúng 1 lần**, y
hệt kỷ luật §2a.3.1 handle-container). Emit CFG:
```
cur → Goto hdr
hdr: __opt = pop_front(iter_local)   // Nullable(inner); len-- (tombstone) mỗi vòng
     <present-test>                  // reuse Nullable match tag-test (scalar sentinel PA-3c
     If present → bdy else ext       //   vs tag-prepend struct/String) — D map-trace routine
bdy: item = <present-unwrap __opt>   // reuse match ~+ present-arm bind (proven 319/347/338)
     <body>                          // break→ext, continue→hdr (KHÔNG step block —
     Goto hdr                        //   pop_front TỰ advance; né vô hạn khác for-Range)
ext:                                 // iter_local (rỗng, len==0) drop ở scope-exit: buffer-only
```
- loop-context: `break_bb = ext`, `continue_bb = hdr`.
- **Né "match-arm diverges"**: emit `Terminator::If` trên present-tag TRỰC TIẾP (không dùng
  match-expr với arm `~0 => break`). Present-test + unwrap = TÁI DÙNG routine lowering của
  `match nullable { ~+ x => .., ~0 => .. }` — D **map-trace** (luật 20) chỉ ra chính xác điểm
  reuse; refuse-nếu-không-rõ (luật 4), KHÔNG tái phát minh tag-test.

### §2b.5 — Soundness (hợp đồng AMEND-2.1 thoả MIỄN PHÍ)

- **Tombstone per-element (🔩 DOUBLY LOAD-BEARING — O đo 2026-07-26)**: `pop_front` `len--`
  mỗi vòng → tại MỌI điểm break/return/fall-through, `v.len` = **đúng số phần tử CHƯA drain**.
  `Drop(v)` free đúng survivors `0..len` + buffer. Phần tử đã drain owned bởi `item` (drop
  trong body). ⇒ **mỗi leaf free đúng 1 lần** — 0 leak, 0 double-free, kể cả break giữa chừng.
  **PHÁT HIỆN VÀNG (O poison độc lập):** tháo dòng `len--` khỏi `__triet_vector_pop_front`
  gây HAI failure-mode phân biệt — (a) **full-drain HANG VÔ HẠN** vì `pop_front` không bao giờ
  báo empty (len đứng nguyên) → present-test không bao giờ dừng vòng; (b) **break-giữa-chừng
  FAILED** survivor re-free mismatch (Drop re-walk slot đã move-out). Nên `len--` mang **tải
  trọng KÉP**: vừa là **điều kiện DỪNG** của CFG loop, vừa là **chốt chống double-free** cho
  survivor. Teeth `drain_iter_counting.rs` canh cả hai (full-drain hang + break-mid count).
- **Heap-element mở an toàn**: move-out chuyển sở hữu (khác Slice 2a copy=alias) ⇒ không hai
  chủ một allocation.
- **Rvalue temp**: `for x in make_vec().drain()` — container rỗng sau vòng, buffer drop đúng
  **1 lần** (không leak buffer FREE=0, không double FREE=2).
- **Borrowck KHÔNG chạm**: `pop_front.mutates_arg=Some(0)` → E2440 tự bắt nếu `v` có loan
  sống; CFG chuẩn Goto/If.
- **O(N²)**: `pop_front` shift → drain N = O(N²). **Chấp nhận correctness-first** (tái dùng
  100% hạ tầng proven >> shim cursor O(N) mới mang nguy cơ off-by-one/dangling). O(N)
  cursor-drain = **nợ kỹ thuật, ADR performance tương lai**.

### §2b.6 — Teeth (O verify máu — cp-snapshot, KHÔNG git checkout; 6 điều kiện thép G)

Positive:
- **T2b-scalar** (EXPECT): `for x in v.drain()` `Vector<Integer>` → tổng đúng.
- **T2b-heap-string** ⭐ (điều kiện G #3, EXPECT): `Vector<String>` drain — string đọc được
  trong body, exit 0 sạch (allocator THẬT), free sạch sau vòng.
- **T2b-heap-struct** ⭐ (điều kiện G #3, EXPECT): `Vector<User{name:String}>` drain — field
  String đọc được, drop sạch.
- **T2b-empty** (EXPECT): `v.drain()` vector rỗng → 0 vòng, v drop sạch.
- **T2b-break/continue** (EXPECT): break/continue trong drain hoạt động (loop-context).

Soundness (counting/subprocess — FREE dedup con-trỏ):
- **T2b-tombstone** ⭐ (điều kiện G #5): drain N heap → FREE = N (element) + 1 (buffer).
  **Poison** (O): phá lệ thuộc len-- (giả lập tombstone hỏng) → popped cell double-free →
  **SIGABRT tcache**. Đo THẬT, không bịa.
- **T2b-break-mid** ⭐ (điều kiện G #5): drain 5, `break` sau 2 → FREE = 2 (item) + 3
  (survivor qua `Drop(v)`) + buffer; KHÔNG double (134), KHÔNG leak (FREE thiếu).
- **T2b-rvalue** ⭐ (điều kiện G #6): `for x in make_vec().drain()` — buffer FREE=1 (không
  leak FREE=0, không double FREE=2).

Negative (guard — fail-closed):
- **T2b-standalone-refuse** ⭐ (điều kiện G #1, ERROR E1041): `let x = v.drain();` → **E1041**
  tại typecheck (drain KHÔNG là method chung). Poison: đăng ký drain thành method → mất E1041.
- **T2b-borrow-refuse** ⭐ (điều kiện G #2, ERROR E1053): drain trên `&0 Vector` param →
  **E1053** tại typecheck (KHÔNG compile ngầm → KHÔNG UB/crash). Poison: gỡ guard reference →
  lọt lower/JIT.
- **T2b-nullable-refuse** ⭐ (điều kiện G #4, ERROR E1053): `Vector<String?>` / `Vector<Integer?>`
  drain → **E1053** (double-nullable ADR-0088 defer). Fail-closed, KHÔNG đoán.
- **T2b-nondrain-method-refuse** (ERROR E1052): `for x in v.enumerate()` → **E1052**.

### §2b.7 — Sites
1. **Typecheck** `check.rs:692` (`Stmt::For` arm — thêm nhánh drain MethodCall TRƯỚC
   inline-Range check; guards §2b.3). `error.rs` — tái dùng E1041/E1052/E1053 (KHÔNG mã mới;
   E1053 message drain-context-aware cho reference vs nullable).
2. **Lower** `lib.rs` `Stmt::For` (thêm nhánh drain MethodCall TRƯỚC nhánh Range `:2337` &
   Vector `:2484`; desugar pop_front-loop §2b.4; reuse Nullable present-test/unwrap; loop-context).
3. **Borrowck / Schema / JIT shim** — KHÔNG chạm (tái dùng `__triet_vector_pop_front`/present-test).

### §2b.8 — Out of scope (Slice 2b)
- **HashMap.drain()** — BÁC (G): đụng `emit_hashmap_value_free_loop` + state-gate bucket riêng →
  pháo đài RIÊNG. Refuse E1052.
- String iterate, `.enumerate()`/`.iter()` adapter — E1052 (trait defer §1).
- `Vector<T?>` drain (double-nullable) → E1053, đợi ADR-0088.
- Borrow-receiver drain (`&mutable Vector`) → E1053, mở rộng sạch tương lai.
- O(N) cursor-drain shim → nợ perf ADR tương lai.

### §2b — Signatures
- **O: ✅ VERIFY MÁU XONG (2026-07-26)** — recon 5 mảnh proven (fixture 347/351/338) + verify
  độc lập: gate sạch `0·clean·0·488·0`; poison tombstone `len--` ĐỎ hai chiều (full-drain HANG
  vô hạn + break-mid FAILED count) → doubly-load-bearing (§2b.5); present-test fat-Nullable đúng
  (487/488 total=5 allocator thật); guard 491-494 đúng mã (E1041/E1053/E1053/E1052); Deinit=zero
  không free; sentinel-collision bất khả (PA-3c ngoài dải Integer). **D bị cắt ngang mid-verify:**
  O restore code-poison D để lại (`mir_lower:6164` về HEAD `da3a0d80`, KHÔNG vào commit) + sửa
  docstring giả-thuyết-sai của D (`STR_FREES==6` → thực tế hang vô hạn) về đúng đo thật; code
  logic của D (check.rs/lib.rs/error.rs) NGUYÊN VẸN.
- **G: ✅ BAN HÀNH + CO-SIGN (2026-07-26)** — duyệt scope Vector-only (BÁC HashMap.drain() —
  "từng pháo đài một") + kiến trúc pop_front-desugar zero-shim + 6 điều kiện thép. Co-sign sau
  verify: chấp nhận Option-1 (O ký+commit, không recall D — "bằng chứng là vua, không thờ cúng
  thủ tục"); lệnh khắc "Tombstone DOUBLY LOAD-BEARING" vào §2b.5.
- **Giang: ✅ BAN HÀNH (2026-07-26)** — chốt scope Vector-only, lệnh xuất quân.

---

## §AMEND — Slice 2d: `for item in <&0 mutable Vector>.drain()` borrow-receiver drain

Mở §2b.8 dòng "Borrow-receiver drain → mở rộng sạch tương lai". Slice 2b tiêu thụ owner
by-value; **Slice 2d drain QUA mượn-độc-quyền-mutable — caller GIỮ container**.

### §2d.1 — Scope (G chốt 2026-07-27)
- **CHỈ `&0 mutable Vector<T>`** (`ReferenceForm::BorrowExclusiveMutable`). Mọi form khác —
  `&0` read-only (`BorrowReadOnly`), `&+`/`&+ mutable` (`StrongFrozen`/`StrongMutable`),
  `&-` (`WeakObserver`) — **TIẾP TỤC refuse E1053** (DrainBorrowedReceiverUnsupported).
- **T non-nullable.** `&0 mutable Vector<T?>` → E1051/E1053 (double-nullable, đợi ADR-0088).
- **KHÔNG ADR-nền mới.** Mirror desugar Slice 2b (`pop_front` loop) TRỪ buffer-drop cuối vòng.

### §2d.2 — Container-Survives semantics (khác BẢN CHẤT Slice 2b)
Runtime repr của `Vector<T>` = **buffer-pointer handle** single-i64 (`{len@0, cap@8, data@16}`).
`&0 mutable Vector` reference-value = **cùng buffer-pointer** (đo: `__triet_vector_get(vec)` =
`vec as *const u8`, fixture 168 `&0 xs`→get→15 ✅). Do đó:
- `pop_front(handle)` `len--` mutate **buffer header CHUNG** → caller quan sát được drain
  (khác `String` — `len` ở stack fat-slot nên `clear` cần slot-ptr; Vector `len` ở heap buffer).
- Receiver là `MirType::Reference{..}` → `is_reference()==true`/`is_copy()==true` (mir/lib.rs:736)
  → **KHÔNG `push_owned`, KHÔNG `Statement::Drop`** → buffer **BẢO TOÀN** cho caller. Sau vòng
  caller thấy `Vector` **rỗng-hợp-lệ** (`len==0`, cap giữ) — re-push/len/drop bình thường.

### §2d.3 — Break-Mid Caller-Drop soundness (câu hỏi (B), O verify cơ học)
Break-mid: `buffer.len` đã giảm đúng số item đã pop (tombstone `len--` mỗi `pop_front`). Survivors
nằm `0..len`. Caller drop `v` SAU vòng → `emit_vector_element_free_loop` đọc `len=load(ptr,0)`
buffer-header + loop `i<len` (mir_lower.rs:1873/1880) → free **CHỈ survivors**, KHÔNG đụng item
đã pop (đã move-out + consume trong body). `__triet_vector_free` dealloc buffer block theo `cap`
(mir_lower.rs:5828). **Tombstone `len--` trong buffer chung = chốt chống double-free — nay gánh
CẢ caller-later-drop** (Slice 2b gánh owner-consumed-drop; cùng cơ chế, tương tác MỚI). Teeth
break-mid trên `Vector<String>` bắt buộc chứng minh FREE khớp, no-leak, no-double-free.

### §2d.4 — Điểm chạm (2, contained)
1. **typecheck `check.rs:754`** — refuse mù `matches!(Type::Reference(..))` → **form-aware**:
   `Type::Reference(ReferenceForm::BorrowExclusiveMutable, inner)` nơi `inner=Type::Vector(T)`,
   `T` không `Nullable` → ALLOW, element=`T`. Mọi form/`T?` khác → E1053/E1051 (giữ nguyên).
   (Typecheck `Type::Reference` = **tuple** `(ReferenceForm, Box<Self>)`, types.rs:117.)
2. **lower `lib.rs:2361`** — hiện `let MirType::Vector(inner) = ty else Err`. Mở rộng nhận
   `MirType::Reference { form: BorrowExclusiveMutable, inner }` nơi `*inner = MirType::Vector(elem)`;
   unwrap lấy elem, iter_local = reference-value (buffer handle); phần desugar pop_front loop
   GIỮ NGUYÊN Slice 2b (is_reference tự bỏ drop). (MIR `MirType::Reference` = **struct**
   `{ form, inner }`, mir/lib.rs:507 — KHÔNG phải tuple.)
3. **borrowck** — `&0 mutable` exclusive loan span cả loop (NLL E2440 sẵn có, không sửa).

### §2d.5 — Out of scope (Slice 2d)
`&+ mutable`/BYOS drain · HashMap.drain() (pháo đài riêng) · `Vector<T?>` (ADR-0088) ·
O(N) cursor-drain perf. Đều giữ refuse hiện hành.

### §2d — Signatures
- **O: ✅ VERIFY MÁU XONG (2026-07-27)** — recon file:line + verify 7/7 sự thật load-bearing
  (ReferenceForm variants ✅ · Type::Reference tuple ✅ · MirType::Reference struct — **bắt lệch G
  viết tuple** ✅ · reference=buffer-handle ✅ · pop_front len-- shared buffer ✅ · element-free-loop
  quét 0..len ✅ · is_reference→no-drop ✅). **3 điểm chạm** (D `014442e`+`2dcc9b6`): typecheck
  form-aware `check.rs:759` + lower Reference-unwrap `lib.rs:2373` + **JIT fat-detect Reference-unwrap
  `mir_lower.rs:3909`** (lỗ D tự bắt ngoài scope phase-1 → phase-2 mở điểm chạm #3, mirror idiom
  `_get_copy:3967`). Gate độc lập `0·clean·0·501·0`. **Poison máu:** (2) tháo form-guard → 492/507 hết
  E1053 ĐỎ; (3) tháo JIT Reference-unwrap → heap drain 506 `unexpected String return` ĐỎ (scalar 505
  vẫn OK — bán kính đúng); (1) push_owned KHÔNG đỏ → **phát hiện no-drop 2-lớp** (lowerer is_copy +
  JIT Drop:3397 cùng qua `is_copy(Reference)==true` mir:736), escalate poison chokepoint → 506 `Drop
  not supported` fail-closed + counting ĐỎ = container-survives load-bearing (fail-closed, KHÔNG silent
  double-free). Fixtures 505-509 + counting teeth (full=3, break-mid=5) xanh, restore md5 4 file khớp.
- **G: ✅ NGHIỆM THU CHIẾN DỊCH (2026-07-27)** — verify độc lập trên commit `2dcc9b6`: gate
  `0·clean·0·501·0`, counting teeth (full=3, break-mid=5) sạch, canaries E1053 / break-mid survivor
  drop chuẩn xác, 2 lớp no-drop (`is_copy(Reference)` lowerer + JIT Drop:3397) bảo vệ borrow an toàn
  fail-closed.
- **G: ✅ CHẤP THUẬN KIẾN TRÚC (2026-07-27)** — duyệt scope `&0 mutable`-only + T-non-nullable,
  bắt buộc ADR-first, tự đo (E) Type::Reference/ReferenceForm cho O, khắc Container-Survives +
  Break-Mid Caller-Drop soundness.
- **Giang: ✅ CHỐT HƯỚNG (2026-07-27)** — chọn #6 trong 7 ứng viên ("đóng hòm cái nhanh gọn").

## §AMEND — HashMap.drain() Deferral (hai-bức-tường, fail-closed E1054)

> ⚠️ **SUPERSEDED bởi §AMEND-2 (2026-07-27, `816a729`)** — `HashMap.drain()`
> nay ĐÃ LAND qua PA-2 destructuring-only desugar. Bức tường "cần Tuple
> lowering" dưới đây **được đi vòng, không phải bị phá** (`MirType::Tuple`
> vẫn = 0). E1054 GIỮ nhưng đổi vai: chỉ còn refuse các hình ngoài fence
> lát 1. Đọc §AMEND-2 để biết trạng thái hiện hành; phần dưới giữ nguyên
> làm hồ sơ lý do defer tại thời điểm đó.

Formalize dòng §2d.5 out-of-scope "HashMap.drain() (pháo đài riêng)". Giang mở
campaign HashMap.drain() (2026-07-27); O recon-trước **BÁC nhãn backlog "mirror
Vector.drain / bucket state-gate riêng"** — nhãn bỏ sót bức tường lớn hơn (Tuple).
Quyết định: **KHÔNG land feature; refuse fail-closed bằng E-code RIÊNG.**

### §HM-drain.1 — Hai bức tường kỹ thuật (O verify file:line, 2026-07-27)

**🧱 Bức tường 1 — YIELD SHAPE cần Tuple `(K,V)`, mà Tuple CHƯA lower.**
Ngữ nghĩa đúng của `for (k, v) in m.drain()` yield `(K, V)`. Tuple tồn tại ở
AST + typecheck + parser (`Type::Tuple` `types.rs:49`; `Pattern::Tuple`
`parser/pattern.rs:173`; test `parses_for_with_tuple_destructuring`
`parser/stmt.rs:450`) — **nhưng grep `Tuple` trên `triet-lower` / `triet-mir` /
`triet-jit` = 0 hit CẢ BA CRATE**. Tuple chưa có MIR-repr, chưa lower, chưa JIT
layout. ⇒ yield `(K,V)` đòi **xây tuple-lowering từ đầu** (MIR + JIT) = campaign
prerequisite RIÊNG, nặng hơn drain, mở khóa nhiều thứ ngoài drain (multi-value
return, destructuring). HashMap.drain **bị gate SAU** campaign đó.

**🧱 Bức tường 2 — không có primitive enumerate-entry key-less.**
HashMap layout (`mir_lower.rs:6444`): open-addressing, slot = `key_stride +
value_stride + 1 state-byte`, body = `[len@0, cap@8, slots@16…]`, state==0 =
empty (enumerate-được về nguyên tắc: walk 0..cap skip empty). Shim inventory:
`alloc/free/len/insert/get/get_ref/get_ref_agg/get_copy/remove/contains` — grep
`hashmap_keys/values/iter/next/pop/drain/entries` = **0 hit**. Vector.drain tái
dùng `pop_front` (shim proven); HashMap **không có analog** — `remove(key)` đòi
biết key trước. ⇒ desugar-loop kiểu Slice 2b BẤT KHẢ; cần **shim mới**
(`__triet_hashmap_drain_next` cursor / bucket-walker) hoặc phơi bucket internals
cho lowerer. Đây là "bucket state-gate" nhãn nhắc — nhưng nhãn bỏ sót Bức tường 1.

### §HM-drain.2 — Quyết định: DEFER, refuse fail-closed (KHÔNG lossy)

PA-B (values-only) / PA-C (keys-only) **BỊ CẤM** (Giang phán 2026-07-27): drain
mà vứt câm key/value = lossy, phi đối xứng, phản trực giác — thuốc độc semantic,
vi phạm Bài học #6 ([[mentor_o_persona]] luật 18: "shape có ĐƯỢC PHÉP tồn tại
không, không đắp cơ chế vào chỗ thiếu"). Khi chưa có Tuple `(K,V)` lowering,
`HashMap.drain()` **KHÔNG ĐƯỢC PHÉP TỒN TẠI**. Refuse sạch, fail-closed, KHÔNG
silent error, KHÔNG panic vô hướng.

### §HM-drain.3 — E-code RIÊNG: E1054 (KHÔNG rơi E1052 chung chung)

Hiện `for x in m.drain()` (receiver HashMap) rơi vào `else` `check.rs:795-803`
→ **E1052** `NonRangeIterationUnsupported` (generic "trait Iterator chưa impl").
Che mất câu chuyện thật (2 cliff). Formalize code riêng:

- **E1054 `DrainHashMapUnsupported`** (next free — E1050..E1053 đã dùng).
- Header: `E1054: `for` iteration over `HashMap<{key}, {value}>.drain()` is unsupported`.
- Body/help nêu ĐÍCH DANH 2 bức tường: (1) yield `(K,V)` cần Tuple lowering
  (chưa có ở MIR/JIT) — trỏ prerequisite; (2) chưa có enumerate-entry shim.
- `[Fix]` gợi ý: dùng `remove(m, k)` theo từng key đã biết, hoặc chờ Tuple
  lowering + `HashMap.drain()` (deferred, ADR-0089 §AMEND HashMap.drain).
- **Scope chốt: CHỈ `.drain()` receiver = `Type::HashMap(..)`.** Plain
  `for x in m` (non-drain HashMap iterate) GIỮ E1052 — đó là deferral khác
  (Iterator trait), không phải drain. (Điểm quyết-scope này O nêu cho G; refuse-
  over-guess: không tự nới rộng E1054 sang plain-iterate.)

### §HM-drain.4 — Điểm chạm (contained, 1 site typecheck)

`check.rs` drain-branch: TRƯỚC `else` `:795`, thêm arm
`if let Type::HashMap(k, v) = &receiver_ty` → push `DrainHashMapUnsupported`.
String/other GIỮ NGUYÊN `NonRangeIterationUnsupported`. Không đụng lower/mir/jit
(refuse ở typecheck ⇒ không bao giờ tới lowerer). Zero shim mới.

### §HM-drain.5 — Teeth (fixture refuse + poison provable, D cắm)

- Fixture: `for (k,v) in m.drain()` (hoặc `for x in m.drain()`) trên
  `HashMap<Integer,Integer>` → EXPECT-ERROR **E1054** (không E1052, không panic,
  không SIGILL).
- **Poison chứng minh răng ở tầng harness** (luật 15): gỡ arm HashMap-detection
  ở `check.rs` → fixture PHẢI đỏ (rơi lại E1052 `got E1052, expected E1054`).
  Khôi phục byte-identical. (Đây là teeth tối thiểu cho một defer — chứng minh
  code path fail-closed vào ĐÚNG E-code, không im lặng crash.)

### §HM-drain.6 — Prerequisite / Out of scope

- **Prerequisite thật của feature:** campaign "Tuple lowering (MIR + JIT)" PHẢI
  land TRƯỚC khi HashMap.drain() có thể tồn tại đúng-chuẩn. Ghi vào backlog như
  một pháo đài độc lập, KHÔNG đi ké amendment này.
- Vẫn defer: enumerate-entry shim · O(N) cursor-drain · `HashMap<K, V?>`
  (double-nullable value, ADR-0088) · plain `for x in m` HashMap iteration (E1052).

### §HM-drain — Signatures
- **O: ✅ RECON + DESIGN + VERIFY MÁU (2026-07-27)** — verify 2 bức tường
  file:line (Tuple 0-hit lower/mir/jit; shim inventory không có enumerate
  key-less); đề xuất E1054 + scope drain-only; soạn WO cho D (KHÔNG tự code, pen
  D → `c001075`). **Verify độc lập:** đọc diff (E1054 variant + span arm `:1326`
  + HashMap arm trước else; plain-iterate + Vector E1053 nguyên vẹn); gate độc
  lập `0·clean·0·502·0`; **poison tự tay** (tắt HashMap arm `check.rs:795` →
  fixture 510 `FAIL: expected E1054, got E1052` = răng thật tầng harness) →
  restore byte-identical (md5 `bd8c08c4…`); scope-check 471 plain-iterate giữ
  E1052 dưới poison.
- **G: ✅ KÝ DUYỆT KIẾN TRÚC (2026-07-27)** — APPROVED PA-D (refuse-over-guess,
  no lossy semantics); E1054 `DrainHashMapUnsupported` **strictly scoped to
  `.drain()`** (KHÔNG gộp plain-iterate — "một E-code, một hợp đồng ngữ nghĩa";
  gộp = diagnostic laziness); teeth poison E1052-vs-E1054 bắt buộc; gate target
  `0·clean·0·502·0`. [G — RUTHLESS COMPILER GATEKEEPER].
- **Giang: ✅ KÝ DUYỆT PA-D (2026-07-27)** — defer sạch, cấm PA-B/C lossy, đòi
  ADR + E-code riêng + teeth fail-closed.

---

## §AMEND-2 — HashMap.drain() LANDED (PA-2 destructuring-only desugar)

**Trạng thái:** ĐÓNG (O✅/G✅/Giang✅ 2026-07-27, `816a729`).
**SUPERSEDE §AMEND HashMap.drain() Deferral** ở trên: bức tường "cần Tuple
lowering" **KHÔNG còn chặn** — nó được đi vòng, không phải bị phá. E1054 vẫn
sống nhưng đổi vai: từ *refuse toàn bộ* `.drain()` trên HashMap → chỉ còn
refuse các **hình ngoài fence lát 1** (§HM2.5).

### §HM2.1 — Vì sao KHÔNG làm Tuple hạng nhất (PA-1 bị BÁC)

Nhãn defer cũ ghi bức tường #1 là *"yield `(K,V)` cần Tuple lowering mà Tuple
0-hit lower/mir/jit"*. Recon đo lại giá thật của việc gỡ bức tường đó:

- `MirType::` bị match tại **729 site** trong `triet-lower`/`triet-mir`/
  `triet-jit` (riêng `mir_lower.rs` có 29 match exhaustive).
- Thêm một variant `MirType::Tuple` = gieo lại đúng họ bug **"match exact,
  QUÊN variant"** mà dự án vừa tốn trọn một chiến dịch để quét (§họ "quên
  `Nullable`": 6 thành viên, **2 nằm bên trong chính lưới an toàn**).
- Chạm **B-γ multi-value return** (defer vô thời hạn) và kề **B-β sub-8B
  packing** (đã đạp chết).

**Câu hỏi kiến trúc quyết định:** `for (k,v) in m.drain()` cần **hai biến
trong thân vòng lặp**, KHÔNG cần một **giá trị tuple**. PA-1 xây một kiểu
hạng nhất chỉ để lập tức phá nó ra làm hai — trả 729 site cho một trung gian
không ai giữ lại. **G BÁC PA-1, chuẩn thuận PA-2.**

### §HM2.2 — 🔒 BẤT BIẾN TỐI CAO: Zero-Tuple-ở-MIR

> **Tuple SỐNG ở front, CHẾT tại lower.** `MirType` giữ nguyên 11 variant.

Kiểm chứng thường trực (O verify 2026-07-27): `MirType::Tuple` = **0** trên
toàn `triet-lower` + `triet-mir` + `triet-jit`.

⚠️ **Cách kiểm SAI (đã ăn đòn):** `grep -c Tuple` trần **KHÔNG** phải tiêu chí
— lowerer BẮT BUỘC phải match `triet_syntax::Pattern::Tuple` để destructure
(`triet-lower/src/lib.rs:2036`), đó chính là thiết kế PA-2 chứ không phải vi
phạm. O đặt tiêu chí proxy thô này vào WO và **D bác bằng số đo — D đúng**.
Tiêu chí đúng duy nhất là **`MirType::Tuple` = 0**.

### §HM2.3 — Ba điểm chạm

1. **typecheck** `check.rs:829-845` — pattern là `Pattern::Tuple` **đúng 2**
   children ∧ `key_ok` ∧ `value_ok` ⇒ trả `Type::Tuple([K,V])`, bind qua
   `bind_pattern` (`check.rs:1097`, cơ chế có sẵn). Ngược lại ⇒ E1054.
   `Type::Tuple` ở typecheck là HỢP LỆ — nó chết ở bước sau.
2. **lower** `triet-lower/src/lib.rs:2031+` — destructure `Pattern::Tuple(2)`
   thành **hai local riêng** `_key`/`_val`; CFG mirror drain-arm Slice 2b
   (cursor local, `break`→ext, `continue`→hdr). Không giá trị tuple nào sinh ra.
3. **JIT** `mir_lower.rs:7005+` — shim mới `__triet_hashmap_drain_next`.

### §HM2.4 — Shim `__triet_hashmap_drain_next`: chuỗi 4 bước move-out

Thân mirror `__triet_hashmap_remove` (`:6824`) từ nhánh `state == 1`. Mỗi
entry drain PHẢI làm đủ, đúng thứ tự:

1. surface KEY → `key_out_ptr`, VALUE → `val_out_ptr` (`copy_nonoverlapping`)
2. **zero key-cell** (`write_bytes(key_ptr, 0, key_stride)`)
3. **`state → 2`** (tombstone)
4. **`len--`**

rồi trả `idx + 1` làm cursor kế.

**Vì sao chuỗi này đóng cả 3 tử huyệt bộ nhớ (G mandate) bằng MỘT cờ:**
drop-glue **chỉ walk `state == 1`** (`mir_lower.rs:1940` free KEY, `:2038`
free VALUE) ⇒ ① move-out sound (tombstone miễn nhiễm double-free) · ②
break-mid: phần đã drain `state 2` (bỏ qua) + phần còn lại `state 1`
(drop-glue dọn nốt) ⇒ không rỉ, không free hai lần · ③ container-survives:
`len--` mỗi entry ⇒ drain trọn `len == 0`, re-insert hợp lệ.

**Cursor O(N), KHÔNG rescan O(N²)** (D giải trình bằng số, G nghiêng cùng
hướng): ca `cap=1000, len=10` → cursor chạm mỗi slot đúng 1 lần = **1000**
lượt đọc state cho toàn bộ drain; rescan-từ-0 = **10 × 1000 = 10.000**. Tổng
quát cursor `O(cap)` vs rescan `O(len × cap)`.

**Sound-stop:** `while idx < cap` kiểm điều kiện TRƯỚC khi đọc byte nào ⇒
`cap == 0` / cursor đã ≥ `cap` → trả sentinel ngay, không đọc ngoài header.
Fixture 525 (map rỗng) là răng canh ca này.

### §HM2.5 — 🔑 QUY ƯỚC SENTINEL: cursor dùng `-1`, KHÔNG phải `NULL_SENTINEL`

**G bắt buộc ghi rõ để thế hệ sau không nhầm.** Hai sentinel cùng tồn tại
trong codebase, **khác miền, khác khái niệm**:

| Sentinel | Giá trị | Nghĩa | Dùng ở |
|---|---|---|---|
| `triet_mir::NULL_SENTINEL` | `i64::MIN` | **giá trị vắng mặt** (nullable PA-3c) | `T?`, pop/remove/get |
| cursor-stop (mới) | `-1` | **hết slot để quét** | `__triet_hashmap_drain_next` |

Miền hợp lệ của cursor luôn `>= 0` ⇒ `-1` không đụng dải hợp lệ. **KHÔNG tái
dùng `NULL_SENTINEL` cho cursor** — trộn hai khái niệm là mời một lớp bug câm.

### §HM2.6 — Fence lát 1 + ranh giới E1054

**MỞ:** `K` ∈ {scalar, String} · `V` ∈ {scalar, String, Vector, HashMap}.
**REFUSE (E1054):** pattern không phải `Pattern::Tuple` đúng 2 · tuple-3 ·
aggregate key/value · `V = Nullable`.
**`m.drain()` ngoài hàng rào `for`** → **E1015** (`no field or method named
drain`) — giữ bất biến for-guard-ONLY, y hệt tiền lệ Vector (fixture 491).

⚠️ **Nợ chẩn đoán ghi sổ (G tạm chấp nhận cho lát 1):** E1054 nay mang **4
nghĩa**; với ca pattern-shape (527/528) message vẫn in `key`/`value` dù
nguyên nhân là *hình pattern*, không phải kiểu. Một lát dọn sau có thể tách
mã nếu muốn siết "một E-code, một hợp đồng".

### §HM2.7 — Răng + giao thức 3 mũi poison (O verify độc lập, số đo thật)

**11 fixture 520-530** · **`hashmap_drain_counting.rs`** (7 test, **dedup CON
TRỎ**: assert cả `count == N` VÀ `dup == 0` — FREE-count đơn thuần mù trước
double-free, vì 3 lần free có thể là 3 object HOẶC 2 object + 1 trùng).

| Mũi | Poison | Đo được (O tự cắm) |
|---|---|---|
| **P1** | `state → 2` thành `1u8` | `drain_full` **9 vs 6** · `break_mid` **10 vs 8**, con trỏ lặp ⇒ **double-free thật** |
| **P2** | bỏ `len--` | `drain_full_leaves_len_exactly_zero` **3 vs 0** |
| **P3** | guard typecheck fail-**open** (`if true`) | 527·528·529·530 đỏ **+ fixture cũ 510 đỏ lây**; 520-525 **không** đỏ |

⚔ **Bài học P2 — "không đỏ" phải phân định (a)/(b) bằng đường-chạm-được:**
P2 **KHÔNG** làm đỏ corpus, vì vòng drain dừng theo `state` (qua cursor),
KHÔNG theo `len`; re-insert vào `cap=4` cũng chưa chạm ngưỡng resize. Đây là
**(b) test chưa đủ mạnh**, không phải (a) bất-khả-observable. **D báo trung
thực rồi tự cắm thêm răng** `drain_full_leaves_len_exactly_zero` đọc thẳng
`len(m)` — KHÔNG bịa mũi giả cho nổ để qua cửa.

⚔ **Bài học P3 — "tháo guard" nghĩa là fail-OPEN, không phải fail-closed.**
Mũi đầu D làm `if false &&` (siết chặt hơn) = sai hướng, không chứng minh gì;
D tự phát hiện, sửa thành `if true ||` (chấp nhận bừa) rồi đo lại. Dưới
poison đúng hướng, các hình refuse **không compile lọt** mà bị lowerer chặn
bằng `LowerError` khác ⇒ **có defense-in-depth 2 lớp** (typecheck = mã đúng,
lower = fail-closed cuối), cùng kiến trúc với ADR-0088 Lane A.

### Ngày hiệu lực §AMEND-2

- Hiệu lực từ `816a729` (2026-07-27). Gate `0 · clean · 0 · **522** · 0 ·
  CLEAN` (511 → 522 file fixture; **522 là TỔNG SỐ FILE**, không phải số hiệu
  cao nhất — số hiệu cao nhất là 530, corpus có gap lịch sử).
- **Nợ mở:** aggregate key/value drain (move-out key aggregate = đường ABI
  mới) · `V = Nullable` (chờ số đo; HashMap drain qua out-param KHÔNG bọc
  `Nullable` nên về lý thuyết an toàn hơn `Vector<T?>`, nhưng **chưa đo** ⇒
  giữ refuse) · tách mã E1054 4-nghĩa · `Tuple` hạng nhất (PA-1) vẫn **BỊ
  BÁC**, chỉ mở lại khi có use-case multi-return thật.

### Chữ ký §AMEND-2

- **O: ✅** — recon lật khung (PA-1 729-site vs PA-2 zero-hit); verify độc lập
  3 mũi poison + gate + `MirType::Tuple`=0; **tự nhận 2 tiêu chí sai bị D bác
  bằng số đo** (`grep -c Tuple` proxy thô · "530 fixtures" nhầm số-hiệu với
  tổng-số-file, trong khi chính O đã đo `ls | wc -l` = 511 cùng phiên).
- **G: ✅** — BÁC PA-1 ("tự châm lửa đốt nhà mình"), duyệt PA-2; ra 3 tử huyệt
  bộ nhớ + mandate teeth heap-key×heap-value dedup con trỏ; bắt ghi quy ước
  sentinel `-1`; tạm chấp nhận E1054 4-nghĩa cho lát 1.
- **Giang: ✅** — chốt hướng Tuple/HashMap.drain, ký phát lệnh thi công.

## §AMEND-3 — Split E1054 4-nghĩa → E1056 (pattern) / E1054 (key) / E1057 (value)

Trả nợ chẩn đoán ghi ở §HM2.6/§AMEND-2 ("tách mã E1054 4-nghĩa"). `E1054
DrainHashMapUnsupported` nhồi 3 trục độc lập vào MỘT nhánh `if…else` — pattern
không phải `(k,v)`, key aggregate, value nullable/aggregate — và LUÔN in
`HashMap<{key}, {value}>` làm nguyên nhân kể cả khi thủ phạm là cú pháp
pattern (fixture 527/528 sai lệch: message nói kiểu trong khi lỗi là hình
pattern). Vi phạm ADR-0086 "một E-code, một hợp đồng".

**Tách theo 3 trục, thứ tự cascade pattern→key→value:**

| Mã | Variant | Trục | Fixture |
|---|---|---|---|
| **E1056** | `DrainHashMapPatternUnsupported` | loop pattern ≠ `(k, v)` 2-tuple — message KHÔNG in `key`/`value` | 527, 528 |
| **E1054** | `DrainHashMapKeyUnsupported` (thu hẹp) | `K` aggregate — đích danh `key` | 529 |
| **E1057** | `DrainHashMapValueUnsupported` | `V` nullable/aggregate — đích danh `value` | 530 |

Cascade kiểm pattern trước (không phụ thuộc K/V), rồi key, rồi value — mỗi
refuse chỉ nêu đúng trục đã fail, không còn noise từ 2 trục kia. `526` (drain
ngoài `for`-guard) không đổi, vẫn E1015. `TypeError::error_span` cập nhật 3
arm thay 1. Không đổi hành vi ACCEPT (fence lát 1 giữ nguyên); chỉ đổi
diagnostic surface.

### Chữ ký §AMEND-3

- **O: ✅** — soạn WO 5-điểm-chạm, chốt bảng 3 mã + cascade order.
- **G: ✅** — duyệt tách trục, xác nhận không mở rộng scope ACCEPT.

## §AMEND-4 — FIFO contract cứng hoá + hoãn VÔ THỜI HẠN O(N) cursor-drain

### §4.1 — Hợp đồng FIFO (ngữ nghĩa quan sát được, ràng buộc)

`for x in v.drain()` trên `Vector<T>` duyệt phần tử theo thứ tự **index
`0 -> len-1` (FIFO)** — đúng thứ tự đã `push`. Đây KHÔNG phải chi tiết cài
đặt tình cờ của desugar `pop_front`-loop (Slice 2b §2b.4): nó là **ngữ nghĩa
quan sát được** mà mọi test/người dùng có quyền dựa vào (`break`-mid giữ
đúng survivor theo thứ tự, §2d.3; `return`-mid cũng vậy, §4.2 dưới). **Mọi
thay đổi thứ tự duyệt là breaking change**, phải qua ADR mới, không được vá
âm thầm bằng đổi shim bên dưới.

Corpus TRƯỚC WO này **mù thứ tự**: O tự poison `__triet_vector_pop_front` ->
`__triet_vector_pop` (đổi FIFO thành LIFO) và chạy toàn bộ 522 fixture khi
đó — chỉ **1/522 đỏ** (`490_drain_break_continue.tri`), và đỏ đó là **tai
nạn hằng số** (`if x == 100 { break }` tình cờ đúng ngay ở phần tử đầu dưới
LIFO), không phải vì thiết kế test khóa thứ tự. 6 fixture liên quan
(486/487/488/505/506/509) XANH 100% dưới cú lật — `509` mù vì mọi String
trong nó có `length == 1` nên hoán vị thứ tự không đổi giá trị quan sát
được. Fixture 531-534 (§4.3) là hàng rào vá lỗ mù này bằng oracle
position-weighted (`acc = acc*10 + x`), KHÔNG dùng tổng cộng dồn.

### §4.2 — O(N) cursor-drain: HOÃN VÔ THỜI HẠN (không phải "chưa làm", là BÁC)

Phương án tối ưu hoá `Vector.drain()` từ O(N²) (`pop_front`-loop hiện hành)
xuống O(N) (con trỏ cursor + epilogue dọn buffer một lần tại cuối vòng,
mirror ý tưởng cursor `state`-flag của `HashMap.drain()` PA-2 §AMEND-2) đã
được G **BÁC**, dựa trên 4 lý do ĐO ĐƯỢC (không phải suy đoán):

1. **`return` giữa thân drain-loop là một exit edge RIÊNG, KHÔNG đi qua
   block `ext`** (điểm hội tụ bình thường cuối vòng lặp). Quan sát được
   trực tiếp trong `fn drain_it` của `534_drain_order_return_mid_survivors.tri`
   (dump MIR đo lại 2026-07-27(f)):
   ```
   bb2: { ... Drop(_1) Drop(_0) Drop(_5) Return(_1) }   // return-mid
   bb3: { Drop(_1) Drop(_0)          Return(_1) }        // exit ext bình thường
   bb4: { If(_4) → +:bb3, -:bb2 }
   ```
   `bb2` (return-mid, nhánh `-` của `If` null-check trên `pop_front`) và
   `bb3` (nhánh `+`, tức "buffer cạn" — exit `ext` thật của vòng) là **HAI
   BLOCK KHÁC NHAU với TẬP DROP KHÁC NHAU**: `bb2` có `Drop(_5)` (drop biến
   `item` vừa move-out trong thân vòng trước khi return), `bb3` KHÔNG có —
   vì tại `bb3` không có `item` nào đang sống để drop. Đây là bằng chứng
   **tự-kiểm-chứng-được ngay trong fixture của corpus** (không phải một
   probe `/tmp` đã biến mất) rằng return-mid và exit-thường là hai đường
   TÁCH BẠCH, không hội tụ. (Ghi chú nguồn gốc: bản nháp trước của mục này
   trỏ vào `bb9`, sao chép nhầm số block từ probe recon `/tmp/o-recon/p1_return_mid_drain.tri`
   của Mentor O — probe đó có hình KHÁC: receiver **owned**, vòng lặp nằm
   thẳng trong `main`, không phải `&0 mutable` + hàm phụ `drain_it` như
   533/534. Số `bb9` đó không tồn tại trong `fn drain_it` — đã sửa để trỏ
   đúng nguồn kiểm chứng được, 2026-07-27(f).) Bất kỳ thiết kế
   cursor+epilogue nào đặt logic dọn dẹp/tombstone tại `ext` (tức `bb3`) sẽ
   bị `return`-mid (`bb2`) **BỎ QUA HOÀN TOÀN** — buffer của caller sẽ ở
   trạng thái nửa-vời (cursor đã tiến nhưng `len`/tombstone chưa cập nhật),
   dẫn tới **double-free** khi caller sau đó drop hoặc tiếp tục thao tác
   trên buffer.
2. **Buffer của `Vector<T>` không có state-byte per-slot** (`{len@0, cap@8,
   data@16}` — chỉ 1 con trỏ `len`, không như `HashMap` có mảng slot với
   trường `state` riêng cho từng ô, §AMEND-2). Cơ chế cursor của
   `HashMap.drain()` (một cờ `state` per-slot đóng cả 3 tử huyệt: move-out
   sound / break-mid / container-survives — xem `AMEND-2` PA-2) **không có
   gì để mirror lên `Vector`**: không có ô nào để đánh dấu "đã move-out"
   độc lập với `len`.
3. Bất biến hiện hành `buffer[0..len)` = **tập sống tại MỌI thời điểm**
   (không chỉ tại `ext`) đang mua soundness **MIỄN PHÍ** cho MỌI exit edge
   — kể cả những cạnh chưa từng được liệt kê tường minh (break, continue,
   return, panic-tương-lai). Một thiết kế cursor-epilogue phải liệt kê và
   xử lý ĐÚNG từng cạnh thoát — gánh nặng chứng minh cao hơn hẳn lợi ích.
4. O(N²) của `pop_front`-loop là **nợ hiệu năng** (thời gian chạy), **KHÔNG
   phải lỗ soundness** — không có áp lực correctness nào buộc phải sửa
   ngay; đánh đổi với rủi ro double-free ở mục 1 là không xứng đáng tại thời
   điểm này.

**Phương án LIFO** (đổi `pop_front` thành `pop`, duyệt từ cuối buffer) và
**phương án đảo-buffer trước khi duyệt** cũng đều bị **BÁC** cùng lý do gốc:
cả hai đều phá vỡ hợp đồng FIFO §4.1 (breaking change không có ADR), và
phương án đảo-buffer còn tốn thêm một lượt O(N) di chuyển dữ liệu chỉ để đổi
thứ tự quan sát — không giải quyết được độ phức tạp O(N²) mà lại đổi ngữ
nghĩa.

Kết luận: `pop_front`-loop O(N²) (Slice 2b §2b.4) là cách lower DUY NHẤT
được duyệt cho `Vector.drain()` cho tới khi có ADR mới đủ mạnh để chứng
minh soundness trên MỌI exit edge (kể cả `return`/`break`/`continue`/tương
lai) của một thiết kế cursor.

### §4.3 — Lính gác (WO-Drain-FIFO-Teeth, O✅/G✅/Giang✅ 2026-07-27(f))

- **531/532/533/534** — lính gác hợp đồng FIFO §4.1, oracle position-weighted
  (`acc = acc*10 + <giá trị>`, không phải tổng cộng dồn):
  - `531_drain_order_scalar.tri` — owned `Vector<Integer>`, EXPECT 123.
  - `532_drain_order_string.tri` — owned `Vector<String>` (heap move-out),
    lengths 1/2/4 phân biệt (509 dùng toàn length-1 nên mù), EXPECT 124.
  - `533_drain_order_break_mid_survivors.tri` — `&0 mutable` borrow-receiver
    drain + `break`-mid + đọc survivor theo thứ tự qua `pop_front`, EXPECT
    1024.
  - `534_drain_order_return_mid_survivors.tri` — y hệt 533 nhưng `break` ->
    `return acc;` giữa thân vòng. **Fixture ĐẦU TIÊN của toàn corpus** chạm
    cạnh return-mid `bb2` vs exit-thường `bb3` trong `fn drain_it` (§4.2
    mục 1) — chính cạnh đã bác phương án O(N). EXPECT 1024.
- **535/536** — lính gác THỨ TỰ CASCADE pattern->key->value (§AMEND-3), đa
  trục (một fixture sai đồng thời ≥2 trục, khác họ 510/527/528/529/530 mỗi
  file một trục):
  - `535_hashmap_drain_multiaxis_pattern_wins.tri` — sai cả 3 trục cùng lúc
    -> khóa cạnh pattern thắng trước (E1056).
  - `536_hashmap_drain_multiaxis_key_wins.tri` — pattern đúng, key+value sai
    -> khóa cạnh key thắng trước value (E1054).

### Chữ ký §AMEND-4

- **O: ✅ 2026-07-27(f)** — soạn WO 6-fixture + đo live 4 giá trị EXPECT
  (123/124/1024/1024) + 2 mã lỗi cascade (E1056/E1054), chốt 4 lý do hoãn
  O(N) cursor-drain vô thời hạn dựa trên probe MIR `bb9`/poison FIFO->LIFO.
- **G: ✅ 2026-07-27(f)** — duyệt hoãn O(N) vô thời hạn (không mở lại tới khi
  có ADR mới), duyệt hợp đồng FIFO cứng hoá thành ngữ nghĩa ràng buộc.
- **Giang: ✅ 2026-07-27(f)** — chốt hướng.
