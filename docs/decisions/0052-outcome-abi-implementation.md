# ADR-0052: Outcome ABI Implementation — 2-slot MIR + Cranelift multi-return

## 1. Status
**Approved (O + G, 2026-06-10).** Nối **ADR-0020** (Outcome design-locked: syntax `~+`/`~-`/`~0`, type `T~E`/`T?~E`, type-level semantics). ADR-0020 chốt **bề mặt**; ADR-0052 chốt **ABI cấp thấp** — cách hạ Outcome xuống MIR (2-slot) + đẩy qua JIT Cranelift (multi-value return). Đây là Low-level ABI Contract (đổi cách hàm Triết trả dữ liệu qua register/FFI).

**Ràng buộc G (bất biến — không vi phạm):**
1. **Payload CHỈ SCALAR (Bậc A): Integer/Trit/Trilean.** Heap payload (String/Vector trong Outcome) = Bậc B/C DEFER (ownership/drop/borrow qua multi-return ABI = bãi mìn riêng).
2. **Un-defer C5 CHỈ cho `BinaryOutcome`/`TernaryOutcome`.** Gỡ guard `values.len()>1` (jit:1070) DUY NHẤT cho Outcome — KHÔNG mở generic Tuple-return.
3. **Cranelift native multi-return** — mỗi value 1 i64, KHÔNG đụng value-model "single i64" (C5 spike proven premise nhẹ).

## 2. Context
Outcome (error-handling core, ADR-0020) Frontend+Typecheck đã sẵn, nhưng **Lower degenerate**: `~+ e` = identity (lower payload, không tạo 2-value), `~-` = unsupported (lower:1124). `ReturnShape::BinaryOutcome` (arity 2) + `OutcomeDiscriminant`/`OutcomeUnwrap` MIR ops định nghĩa SẴN nhưng **0 producer**. JIT chặn multi-value (jit:1070, = C5 phong ấn). Outcome producer = use-case mở C5 (khép vòng: C5 phong ấn vì thiếu use-case, Outcome mang chìa khóa).

## 3. Quyết định ABI

### 3.1. MIR value model: 2-slot `{disc, payload}`
`T~E` value = **2 i64 slots**:
- `disc: i64` — Trit discriminator: `Positive(1)` = success (payload là T), `Negative(-1)` = failure (payload là E). `Zero(0)` INVALID trên T~E (E1025 compile-time, ADR-0020 §1.1).
- `payload: i64` — scalar union (Bậc A: 1 i64 chứa T hoặc E tùy disc).
- `T?~E` (ternary): disc `Zero(0)` hợp lệ = null state (payload bỏ qua).

### 3.2. Constructor lowering
- `~+ value` → alloc 2-slot; `disc = const Positive(1)`; `payload = lower(value)`.
- `~- error` → `disc = const Negative(-1)`; `payload = lower(error)`.
- `~0` (T?~E only) → `disc = const Zero(0)`; payload undefined.

### 3.3. Return: `ReturnShape::BinaryOutcome` (arity 2)
Fn `-> T~E` → `ReturnShape::BinaryOutcome`, `Return { values: [disc_local, payload_local] }`. JIT: Cranelift `sig.returns` = 2× `AbiParam::new(I64)`, callee `return_(&[disc, payload])`, caller `inst_results[0]=disc, [1]=payload`.

### 3.4. Destructure: discriminant + unwrap (Từ bỏ Statement ops chuyên biệt)
`match o { ~+ x => .. ~- e => .. }`:
- Đọc disc: `Assign { dest, source: outcome.project(OutcomeDiscriminant) }` — stack_load slot@0.
- Branch trên Trit: `If { cond: disc, positive_bb: success, negative_bb: error, zero_bb: None }`.
- Đọc payload: `Assign { dest, source: outcome.project(OutcomePayload) }` — stack_load slot@8.
**Kiến trúc thống nhất projection-based:** tái dùng hạ tầng `Projection`/`Assign`/`StackSlot` giống Struct/Sret. Các `Statement` ops chuyên biệt `OutcomeDiscriminant`/`OutcomeUnwrap`/`OutcomeUnwrapError` (định nghĩa cũ mir:254-280) đã bị xóa — chúng giả định Outcome là 1 value đơn nhất (trước StackSlot refactor OP.3.5), không khớp biểu diễn 2-slot. Projection-based thống nhất toàn bộ đường đọc/ghi offset.

### 3.5. JIT un-defer C5 (CHỈ Outcome)
Gỡ guard jit:1070 `if values.len()>1 → Err` **CHỈ khi** `return_shape ∈ {BinaryOutcome, TernaryOutcome}`. Generic >1 values (tuple) vẫn Err (chưa có ngôn ngữ). Cranelift multi-return native — premise nhẹ (C5 spike phase11 proven).

## 4. Phân lát (OP.1-4, mỗi lát gate xanh)
- **OP.1 Typecheck:** verify+bổ sung `check_outcome_constructor_context` — return-type-match (`~+ v`:T, `~- e`:E khớp `-> T~E`) + **E1025** (`~0` on T~E) + **E1024 exhaustiveness** (match T~E cover ~+/~-). Fixtures negative (check-mode).
- **OP.2 Lower:** constructor → 2-slot + `ReturnShape::BinaryOutcome` + `Return[disc,payload]`. **Fixtures CHECK-MODE** (parse→typecheck→lower→borrowck→MIR verify) — chứng minh producer đúng tới MIR, KHÔNG cần JIT. Cô lập producer khỏi backend.
- **OP.3 JIT (un-defer C5-cho-Outcome):** gỡ guard 1070 cho Outcome, Cranelift 2-return, caller inst_results[0,1]. Fixtures RUN end-to-end (T~E ra giá trị).
- **OP.4 Match/unwrap:** OutcomeDiscriminant+branch+Unwrap. Fixtures run match.

## 5. Teeth dự kiến (O)
- OP.2: poison disc const (Positive→Zero) → MIR verifier / typecheck bắt (Zero invalid T~E).
- OP.3: poison gỡ-guard cho generic tuple → tuple-return phải VẪN Err (un-defer chỉ Outcome).
- OP.3: poison caller inst_results[1] (bỏ payload) → fixture run sai giá trị.
- OP.4: poison OutcomeDiscriminant (đọc nhầm slot) → match rẽ nhánh sai.

## 6. Consequences
- **Tích cực:** error-handling core sống (giá trị ngôn ngữ lớn); mở C5-cho-Outcome (gỡ 1 phong ấn Nhóm E); móng cho C4 Packed Outcome (tối ưu sau).
- **Defer (Bậc B/C):** heap payload Outcome (String/Vector) — ownership/drop/borrow qua multi-return. Generic tuple-return (C5 đầy đủ). C4 Packed (bit-pack disc+payload 1 register) — tối ưu, sau khi 2-slot chạy.
- **ABI đổi:** hàm `-> T~E` trả 2 register (SysV) thay 1 — FFI/caller phải biết. Ghi rõ đây.
