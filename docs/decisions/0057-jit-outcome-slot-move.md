# ADR-0057 — JIT Outcome-slot Assign-move: teach `Statement::Assign` to move a StackSlot Outcome

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-11. Khởi thảo Mentor O 2026-06-11, grounded từ JIT spike (scalar merge → 5).
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O (mổ JIT Assign + spike pre-alloc/slot-copy/tombstone, revert sha-identical).
- **Chữ ký:** O ✅ (grounded từ spike: scalar Outcome merge end-to-end, no regression) · G ✅ (ký duyệt + đóng dấu 2026-06-11).
- **Liên quan:** [ADR-0052](0052-outcome-abi-implementation.md) (Outcome 2-reg ABI), [ADR-0053](0053-heap-payload-outcome.md) (32-byte slot), [ADR-0056](0056-heap-value-merge.md) (lower types merge result — tiền đề). **Phong ấn → ADR-0058** (Heap Error Consume: bind `~- e` xong USE ra rác — projection offset sai). **Tách → `fix(lower)`** Bug A dead-block synthetic return (lowerer, KHÔNG thuộc ADR này).

---

## 1. Context — `Statement::Assign` mù hoàn toàn khi thấy một Outcome

Sau ADR-0056, `if`/`match` value-merge type kết quả từ giá trị nhánh. Với
Fat-Pointer (String/Vector) → chạy (JIT typed-Assign copy {ptr,len,cap}). Với
**Outcome** vẫn vỡ:

```triet
function f(c: Color) -> Integer ~ Integer = match c { Red => ~+ 5, Blue => ~- -1 }
// JIT error: "OutcomeDiscriminant access on non-Outcome local"
```

Merge result `_2` được lower type là Outcome (ADR-0056), nhưng JIT KHÔNG cấp
StackSlot cho nó và `Statement::Assign` chỉ copy 1-word — bỏ rơi cả 32-byte slot.

## 2. Root cause — ĐO TỪ CODE + SPIKE, KHÔNG ĐOÁN

MIR merge (`match c {Red=>~+5, Blue=>~--1}`):
```
bb2: { _3 = Outcome; _3.disc=1; _3.payload=5;  _2 = move _3;  Goto(bb1) }
bb1: { _13 = move _2.disc; _14 = move _2.payload; Return(_13,_14) }
```

**Hai khuyết trong `triet-jit/src/mir_lower.rs`:**
1. `outcome_slots` CHỈ populate cho `Statement::OutcomeAlloc { dest }` (line 758-767).
   Merge result `_2` (alloc + `_2 = move _3`) KHÔNG qua OutcomeAlloc → **không có slot**.
2. `Statement::Assign` handler (line ~1010): `load_place`+`store_place` = copy 1-word.
   Có nhánh slot-copy cho **String** (struct_slots), **KHÔNG có nhánh Outcome**.

→ `_2 = move _3` chỉ copy 1 word, `_2` không slot → bb1 `_2.disc`
(OutcomeDiscriminant) refuse tại `mir_lower.rs:332-336` ("non-Outcome local").

**SPIKE chốt (O đâm, revert sha-identical):** 3 điểm chạm bên dưới → scalar
Outcome merge `match c {Red=>~+5, Blue=>~--1}` consume qua match → **5** (trước:
JIT refuse). Driver 38 + jit tests không regress. → JIT CÓ THỂ học move slot;
fix lower-only-không-đủ, phải đụng JIT (đúng instinct G).

## 3. Decision (G chốt scope — KÝ DUYỆT 2026-06-11)

**Phạm vi KHOÁ: Scalar Outcome Merge.** Dạy `Statement::Assign` move một StackSlot
Outcome. **CHỈ scalar** (success+error đều scalar). Heap-payload Outcome merge phụ
thuộc ADR-0058 (heap-error-consume), KHÔNG thuộc đây.

**3 điểm chạm + lưới an toàn (`mir_lower.rs`):**

1. **Pre-alloc** (cạnh String 704-715): cấp + đăng ký `outcome_slots` cho MỌI
   Outcome-typed local (`outcome_slot_size`), KHÔNG chỉ OutcomeAlloc dest. Merge
   result được slot.
2. **Assign Outcome-branch** (handler ~1010): khi dest+source (projection rỗng)
   đều ∈ `outcome_slots` → copy slot-to-slot từng word `[0, outcome_slot_size)`.
3. **Double-free guard:** sau copy, **tombstone source disc=0** (stack_store 0 @ offset 0)
   → Drop source thành no-op (G: "đập nát source là đòn kết liễu Double-Free").
4. **Memory-leak guard (G thêm):** **`Deinit(dest)` trước copy** — đề phòng dest đã
   lỡ giữ Outcome cũ (SSA hiếm, nhưng giăng lưới). Drop-glue cũ của dest chạy trước
   khi đè.

**Lằn ranh:**
- CHỈ JIT Assign + slot pre-alloc. KHÔNG đụng lower (ADR-0056 đã type result).
- KHÔNG đụng heap-error-consume (ADR-0058). Teeth heap Outcome merge → defer.
- KHÔNG đụng dead-block (Bug A `fix(lower)` riêng).

## 4. Teeth (route-lower / .tri run — scalar Outcome merge)

| Ô | Form | Sau fix | Poison-revert |
|---|---|---|---|
| if Outcome ~+ | `= if c {~+5} else {~--1}` consume match | 5 | "non-Outcome local" 🔴 |
| if Outcome ~- | nhánh else lấy ~- | -1 | 🔴 |
| match Outcome ~+ | `match c {Red=>~+5, Blue=>~--1}` → consume | 5 | 🔴 |
| match Outcome ~- | Blue arm → ~- | -1 | 🔴 |
| **double-free** | merge Outcome, free-count source+dest | free đúng (tombstone) | tước tombstone → count↑ |
| **regression** | 110-129 Outcome fixtures + ADR-0055/0056 | xanh | — |

**KHÔNG có ô heap Outcome merge** (String/Vector payload) — phụ thuộc ADR-0058;
ai thêm = lệch scope, REJECT.

## 5. Thứ tự thi công
1. Teeth §4 (scalar Outcome merge) — ĐỎ trước.
2. 3 điểm chạm + 2 lưới (tombstone + Deinit-dest) theo §3.
3. Teeth đỏ→xanh; poison (tước slot-copy / tước tombstone) chứng minh đỏ.
4. Regression Outcome + ADR-0055/0056. Gate raw 4 mục.

## 6. Consequences
- **Tích cực:** Outcome value chảy qua merge (if/match) — mở error-handling biểu thức.
- **Phạm vi:** `mir_lower.rs` (pre-alloc + Assign branch), 0 lower, 0 ABI.
- **Rủi ro:** pre-alloc-cho-mọi-Outcome có thể đụng OutcomeAlloc (double-alloc) —
  spike đã xác nhận KHÔNG regress, nhưng implementer phải đảm bảo OutcomeAlloc dest
  dùng đúng slot (single source). Double-free/leak: tombstone-source + Deinit-dest.
- **Phong ấn ADR-0058 (Heap Error Consume):** bind `~- e` heap xong USE → rác (142
  HP.5 chỉ bind không xài nên ăn may). JIT projection offset nhánh `~-` nghi sai.
  Điều tra riêng SAU 0057.

## 7. Chỉ thị tác chiến cho người lãnh
- Slot-copy dùng `outcome_slot_size` (16 scalar / 32 heap) — nhưng teeth CHỈ scalar.
- Tombstone source disc=0 SAU copy; Deinit(dest) TRƯỚC copy — cả hai bắt buộc.
- CẤM đụng heap-error-consume / dead-block / lower.
- Route-lower hoặc .tri run; double-free phải đo free-count (không chỉ exit-code —
  bài học ADR-0055 death-cell). Poison phải đỏ. Gate raw 4 mục.
- O teeth tay code cuối: poison slot-copy→"non-Outcome"; poison tombstone→free-count↑;
  regression Outcome + 0055/0056 xanh.

## 8. Amendment 2026-06-11 — double-free teeth DEFER + latent leak-guard hazard (append-only)

**Bối cảnh:** implement xong, O teeth tay code cuối (poison + revert sha-identical):
- **slot-copy** poison→1-word: 158-161 garbage 🔴 (cơ chế sống).
- **refactor** `emit_outcome_drop_glue` (extract HP.2 drop-glue, shared Drop+leak-guard):
  byte-identical; poison double-free neg-arm → **138/141 SIGABRT 134** (helper LIVE+faithful).
- **tombstone** poison (xóa `stack_store zero src_slot 0`): 158-161 **VẪN XANH**.

**RULING O (chuẩn thuận đề xuất D):** **double-free free-count teeth (§4 dòng "double-free")
DEFER sang ADR-0058.** Grounded: scalar Outcome Drop = no-op (`emit_outcome_drop_glue`
trả Ok(true) trước khi emit free vì `!is_any_heap`). Tombstone bảo vệ một no-op → không
observable bằng free-count/behavior trong scope scalar. Ba ràng buộc mâu thuẫn (free-count
cần heap · heap merge bị §4 cấm · hand-build MirBuilder bị cấm) → teeth bất khả thi ở đây,
KHÔNG phải D né. Tombstone+leak-guard **read-verified đúng §3.3/§3.4**; drop-glue chúng
dùng chung đã teeth LIVE (138/141). **ADR-0058 BẮT BUỘC mang teeth double-free tombstone**
(heap merge `_2=move _3` payload thật free → poison tombstone → count↑).

**🔴 LATENT HAZARD cho ADR-0058 (O phát hiện khi đào leak-guard):** leak-guard
`emit_outcome_drop_glue(dest)` chạy trên merge-result `_2` — slot pre-alloc **KHÔNG
zero-init**, disc rác tới lần ghi đầu. Scalar: vô hại (bail tại `!is_any_heap` TRƯỚC khi
`stack_load(disc)`). **Heap (ADR-0058): leak-guard sẽ `stack_load` disc RÁC từ `_2` chưa
init → branch → free con trỏ hoang → UB/crash.** Thêm nữa trong SSA merge `_2` ghi-một-lần-
mỗi-path → leak-guard chống kịch bản không xảy ra (G: "SSA rarity"); với heap nó GÂY bug
thay vì chặn. **ADR-0058 PHẢI:** zero-init disc merge-result slot TRƯỚC leak-guard, HOẶC
bỏ leak-guard cho merge-result (fresh, không có Outcome cũ để drop). Ghi để không quên.

- **Chữ ký amendment:** O ✅ (teeth tay 3 mũi + latent hazard 2026-06-11) · G ✅ (ký duyệt
  2026-06-11 — defer double-free teeth → ADR-0058, latent leak-guard hazard ghi án lệ;
  G chốt ADR-0058 sẽ XÉ BỎ leak-guard cho merge-result `_2` (SSA fresh, không Outcome cũ).
  §3 decision KHÔNG đổi).
