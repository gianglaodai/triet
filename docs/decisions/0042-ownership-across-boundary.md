# ADR-0042: Ownership Across Function Boundary — B7-lift (Move-only)

**Status:** Draft — CHỜ KÝ Mentor O (semantics/soundness) + Mentor G (layout/ABI).
**Date:** 2026-06-07
**Author:** AI (triển khai), quyết định cuối: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Move semantics cho heap types (String, Vector) qua user-defined function
boundary. KHÔNG đụng borrow params (`&+ T`, `&0 T`, `&- T`) — defer Bậc C.

---

## Tóm tắt

B7-lift gỡ hai refusal ở `triet-lower/src/lib.rs:492` (param heap type) và
`:1360` (arg heap type), thêm caller zeroing tại `CallDispatch::Jit` + borrowck
move-marking cho user function (keyed `CallTarget::Jit`, không phải
`builtin_shim_meta`). Move-only scope — borrow params cắt theo đồng thuận 2
mentor. Return path đã wired sẵn (M4 return-escape áp dụng cho user fn).

---

## §0 — Dữ kiện đã verify (Phase 0 probe, 2026-06-07)

5 probe chạy trên driver thật, tree tạm gỡ 2 refusal (đã khôi phục).

| # | Probe | Kết quả | Phát hiện |
|---|-------|---------|-----------|
| P1 | `consume(my_string)` — caller zeroing? | **SIGABRT** `double free` | Caller KHÔNG zero slot — M1-M3 không vươn qua call boundary |
| P2 | Giống P1 | Giống P1 | Xác nhận kép |
| P3 | `len(s)` sau `consume(s)` | **SIGABRT** (double-free trước `len`) | Borrowck KHÔNG bắt use-after-move qua call — không E2420 |
| P4 | `let t = make()` → `len(t)` | **5** (thành công) | Return path hoạt động: M4 skip Drop callee, caller Drop 1 lần |
| P5 | `Integer?` qua boundary + Elvis | **7** (thành công) | PA-3c MIN sentinel bảo toàn qua boundary |

O tái lập độc lập 5/5 probe, kết quả khớp 100%.

---

## §1 — Quyết định (6 câu hỏi)

### Q1: Caller zeroing tại CallDispatch::Jit

Sau `CallDispatch` với `CallTarget::Jit`, caller phải zero các arg Move-type
đã truyền vào callee. Cơ chế: loop args, `!is_copy(arg_ty)` → emit
`Statement::Const { value: 0 }` + `Statement::Assign { dest: arg, source: 0 }`
trong block return của caller. Giữ nguyên cơ chế M1 hiện hành — chỉ mở rộng
phạm vi từ builtin sang user fn.

### Q2: Callee drop giữ nguyên

Callee đã Drop param khi scope exit (cơ chế `owned_locals` + `pop_scope`). Không
cần thay đổi. Phối hợp Q1 để tránh double-free: caller zero → callee nhận giá
trị gốc → callee drop 1 lần → caller đã zero nên drop no-op.

### Q3: Borrow params CẮT — move-only

`&+ T`, `&0 T`, `&- T` param qua user fn boundary DEFER Bậc C. Hai mentor đồng
thuận: scope B7-lift này chỉ move semantics. Refusal hiện hành cho heap-type
param không phân biệt Move vs Borrow → gỡ refusal CHỈ cho Move-passing,
borrow-passing vẫn giữ Err (hiện tại tất cả heap param dùng Move passing mặc
định, nên thực tế gỡ toàn bộ).

### Q4: E2420 keyed CallTarget::Jit, check-then-mark

Borrowck M3 hiện tại (`checker.rs:790-805`) chỉ mark Moved cho `CallDispatch`
có `builtin_shim_meta`. Vá: thêm nhánh `CallTarget::Jit` — loop args,
`!is_copy(arg_ty)` → mark `VarState::Moved`.

**Check-then-mark:** Trước khi mark, kiểm tra arg đã `Moved` chưa → nếu rồi,
bắn E2420 (aliased double-move: `foo(s, s)` → callee nhận 2 param cùng pointer
→ drop cả hai → double-free TRONG callee). Pattern: `matches!(state.var_states.get(arg), Some(VarState::Moved))` → error, sau đó mới mark.

### Q5: T? nullable qua boundary

- `Integer?` (Copy): đã hoạt động, MIN sentinel bảo toàn (P5). Không cần code mới.
- `String?` (Move): repr đã định nghĩa (null=MIN) nhưng chưa có producer. Khi
  có producer, Q1+Q2+Q4 áp dụng nguyên xi vì `is_copy("String?") → Move`.

### Q6: trap-on-0 retrofit

Hỏi trực tiếp Mentor G: *"trap-on-0 retrofit đổi hành vi 5 shim cũ — ông có ý
kiến gì không khi nó nằm giữa vùng move semantics B7-lift?"*

Phản hồi của G (nguyên văn): "Double-free không phải trap-on-0 gap; M1-M3 chưa
vươn tới CallDispatch." — G xác nhận cơ chế trap-on-0 không liên quan đến lỗ
double-free hiện tại; double-free do thiếu caller zeroing, không phải do
trap-on-0 sai. Đã lưu vào `MENTOR_G_STATE.md`.

---

## §2 — Acceptance criteria (checklist của G, đối chiếu O)

| # | Tiêu chí | Cách verify |
|---|----------|-------------|
| C1 | Caller zero-out pointer sau call (M1-M3 mở rộng qua boundary) | P1: `consume(s)` → không SIGABRT |
| C2 | Callee `free()` đúng 1 lần khi scope exit | P1+P2: drop trace qua MIR |
| C3 | Return heap value không double-free (M4 return-escape) | P4: `make() → String` → `len(t)` → 5 |
| C4 | E2420 khi dùng lại biến đã move vào user fn | P3: `len(s)` sau `consume(s)` → E2420 |
| C5 | E2420 khi aliased move: `foo(s, s)` | P6: double-move → E2420 |
| C6 | `Integer?` qua boundary nguyên vẹn | P5: `maybe_value(0) ?: 7` → 7 |

---

## §3 — Phạm vi (IN / OUT)

| IN | OUT (defer) |
|----|-------------|
| Move heap param qua user fn | Borrow param (`&+`, `&0`, `&-`) — Bậc C |
| Caller zeroing sau CallDispatch::Jit | `String?`/`Vector?` param (chờ producer) |
| Borrowck move-marking cho CallTarget::Jit | Struct/enum heap payload qua boundary |
| E2420 use-after-move + aliased-move | |
| Return heap value từ user fn (đã wired) | |

---

## §4 — Implementation plan (4 commit, mỗi commit full gate)

1. **docs(adr): 0042** — ADR này
2. **feat(track-b): borrowck move-marking cho CallTarget::Jit** — checker.rs:
   thêm nhánh `CallTarget::Jit` cạnh M3, check-then-mark. Unit test hand-built
   MIR: heap arg → dùng lại → E2420; gỡ marking → test đỏ.
3. **feat(track-b): caller zeroing sau CallDispatch::Jit** — lowerer: sau
   `CallDispatch::Jit`, zero Move-type args. Unit test MIR-level.
4. **feat(track-b): B7-lift — gỡ refusal + fixtures 58-63** — xóa 2 guard
   `:492`/`:1360`. Fixtures: 58=P1, 59=P3 (E2420), 60=P4 (→5), 61=P5 (→7),
   62=P6 (E2420 aliased), 63=temp-arg.

---

## §5 — ADR / tài liệu liên quan

| Tài liệu | Quan hệ |
|----------|---------|
| ADR-0040 | M1-M4 zeroing-on-move, builtin shim ABI, arg_consumes |
| ADR-0041 | PA-3c uniform sentinel, nullable repr, `is_copy` delegation |
| SPEC §10 | S6 ownership, 5 reference forms, move semantics |
| `spec/plans/MENTOR_G_STATE.md` | Lưu Q6 trap-on-0 response của G |
