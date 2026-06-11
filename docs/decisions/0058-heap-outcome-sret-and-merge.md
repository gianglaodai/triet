# ADR-0058 — Heap Outcome: sret return ABI + heap value-merge unlock

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-11. Khởi thảo Mentor O 2026-06-11, grounded từ MIR+JIT line-cite + tiền lệ String sret.
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O (mổ return ABI 2-register + bản đồ 6 điểm chạm).
- **Chữ ký:** O ✅ (root cause proven, Cranelift-feasibility retired bằng tiền lệ String) · G ✅ (ký duyệt + đóng dấu 2026-06-11).
- **Liên quan:** [ADR-0052](0052-outcome-abi-implementation.md) (BinaryOutcome 2-reg ABI — nguồn lỗ), [ADR-0053](0053-heap-payload-outcome.md) (32-byte slot), [ADR-0049](0049-fat-pointer-abi.md) (String sret — tiền lệ tái dùng), [ADR-0057 §8](0057-jit-outcome-slot-move.md) (slot-move scalar + latent leak-guard hazard — Lát 2 kế thừa + xé lưới).

---

## 1. Context — heap Outcome rơi {len,cap} qua boundary; merge còn khóa

Hai nợ heap-Outcome còn treo sau ADR-0057:
1. **Heap Outcome qua return boundary → rác.** `f() -> Integer~String = ~- "ab"`,
   consume `length(e)` → garbage. (Án lệ "ăn may" 142: bind không xài sâu nên thoát.)
2. **Heap Outcome merge bị khóa** (ADR-0057 §4 cấm) — `if`/`match` trả heap Outcome
   chưa chạy.

## 2. Root cause — ĐO TỪ CODE, KHÔNG ĐOÁN (KHÔNG phải load sai offset)

JIT projection load offset ĐÚNG TUYỆT ĐỐI (`mir_lower.rs:330-363`: disc@0/payload@8/
len@16/cap@24). **Lỗ ở ABI return 2-register:**

MIR `f() -> Integer~String = ~- "ab"`: viết đủ `_0.payload_len`@16 + `_0.payload_cap`@24
vào slot CỦA NÓ, nhưng `Return(_6, _7)` chỉ trích `{disc, payload-ptr}`. Caller reconstruct
(`mir_lower.rs:1478-1481`):
```rust
stack_store(disc, slot, 0);  stack_store(payload, slot, 8);  // KHÔNG @16, KHÔNG @24
```
→ `_0.payload_len`@16 / `_0.payload_cap`@24 slot caller **không bao giờ set → garbage**.
**`ReturnShape::BinaryOutcome` = 2-register, vật lý không tải nổi {len,cap} của 32-byte
heap Outcome.** Mấu chốt thiết kế: String dùng `ReturnShape::Struct` (sret) nhưng Outcome
LUÔN BinaryOutcome bất kể payload heap (lib.rs:149-162).

**Cranelift sret feasibility RETIRED bằng tiền lệ String** (cùng codebase, đang chạy):
`Signature SystemV` + `is_sret` → `param[0]=ptr, 0 returns` (mir_lower.rs:546-552); entry
`block_params[0]→Local(0)` (897-899); callee Return ghi slot→sret_ptr (1337-1350).
Cranelift là cỗ máy câm: 32-byte Outcome = stack-value cùng class struct/String → **không
có Cranelift-unknown**.

## 3. Decision (G chốt — KÝ DUYỆT 2026-06-11). HAI LÁT bổ trợ.

### Lát 1 — sret return ABI cho heap Outcome (bản đồ 6 điểm)

Heap-payload binary Outcome (`value_type.is_any_heap() || error_type.is_any_heap()`) trả
qua **sret**, tái dùng `ReturnShape::Struct` machinery của String:

| # | File:vùng | Đổi |
|---|---|---|
| 1 | `lib.rs:149-162` ReturnShape | heap binary Outcome → `Struct` thay BinaryOutcome |
| 2 | `lib.rs:838` `lower_outcome_return_values` | heap Outcome → `vec![slot_local]` (slot) thay [disc,payload] |
| 3 | `lib.rs:2083` call-site `is_outcome_ret` | heap Outcome → sret-style: insert dest arg[0], `dest=[]`, ReturnShape::Struct (mirror is_fat_ret 2040) |
| 4 | `mir_lower.rs:1337` Return-sret | nhánh Outcome: ghi 4 word slot → sret_ptr @0/8/16/24 |
| 5 | `mir_lower.rs:1448` arg-prep | Outcome sret-buffer arg → `stack_addr(outcome_slot)` thay use_var |
| 6 | signature is_sret (547) + entry (897) | **AUTO** kích hoạt do ReturnShape::Struct — KHÔNG sửa |

**Scope guard:** CHỈ heap binary Outcome. Scalar binary Outcome GIỮ 2-register (110-129
không regress). Ternary Outcome heap = nợ riêng (KHÔNG trong ADR này nếu chưa cần).

### Lát 2 — gỡ khóa heap Outcome merge (kế thừa ADR-0057 slot-move)

Heap Outcome qua `if`/`match` value-merge: tái dùng JIT Assign Outcome-slot-move của
ADR-0057 (`_2 = move _3` copy 32-byte slot + tombstone source). Lát 1 đảm bảo slot có đủ
{len,cap} (qua sret) để merge copy đúng.

> ### ⚰️ LỆNH TỬ HÌNH (G chốt — chạm khắc vào Decision):
> **CẤM TUYỆT ĐỐI dùng lưới `Deinit(dest)` / `emit_outcome_drop_glue(dest)` cho Assign
> kết quả merge của heap Outcome.** Merge-result `_2` là **SSA fresh local CHƯA init** —
> slot disc rác. Với heap, leak-guard sẽ `stack_load(disc)` rác → branch → **free con
> trỏ hoang → Undefined Behavior** (latent hazard O vạch ở ADR-0057 §8). Trong SSA, `_2`
> ghi-một-lần-mỗi-path, KHÔNG bao giờ giữ Outcome cũ để mà drop. **Lát 2 PHẢI xóa
> leak-guard khỏi đường merge-result** (tombstone source vẫn giữ — đó chống double-free
> hợp lệ).

## 4. Teeth (ranh giới sinh tử)

### Lát 1 (sret — 32-byte cập bến)
- `f() -> Integer~String = ~- "ab"`; consume `~- e => length(e)` → **2** (không rác).
  Poison: bỏ store @16 → rác.
- **cap đúng:** bind heap-error `e`, `append`/realloc dùng cap → trót lọt + free-đúng-1
  (KHÔNG SIGABRT). Phơi án-lệ-ăn-may 142 (free garbage-cap). Đo free-count, KHÔNG chỉ exit.
- success heap (`~+ "x"` qua boundary, consume) → đúng len.

### Lát 2 (heap merge)
- `if c {~+ "x"} else {~- "y"}` + `match` heap Outcome arms → consume đúng giá trị.
- **KHÔNG double-free** qua merge (free-count source+dest = đúng-1; poison tombstone → count↑).
- **leak-guard CẤM:** test/đọc-code xác nhận merge-result Assign KHÔNG emit Deinit(dest)
  (poison: thêm Deinit(dest) → wild-pointer free → SIGABRT/garbage).
- **Regression TUYỆT ĐỐI:** 110-129 Outcome + ADR-0055 (143-151) + ADR-0056 (152-155) +
  ADR-0057 (158-161) XANH hoàn toàn.

## 5. Thứ tự thi công
1. **Lát 1 trước** (ABI sret — móng): 6 điểm chạm → teeth length(e)+cap đỏ→xanh.
2. **Lát 2 sau** (merge — kế thừa, cần Lát 1 cấp slot đủ): slot-move heap + XÓA leak-guard.
3. Mỗi lát: gate raw 4 mục. Regression full.

## 6. Consequences
- **Tích cực:** heap Outcome chảy trọn — qua boundary (sret) + qua merge. Error-handling
  heap end-to-end. Tái dùng sret machinery (ít code mới), retire Cranelift-risk bằng tiền lệ.
- **Phạm vi:** lib.rs (3 điểm) + mir_lower.rs (2 điểm + Lát 2 merge). KHÔNG đụng borrowck.
- **Rủi ro:** (a) sret-arg-prep cho Outcome — nếu pass var thay stack_addr → vỡ; teeth
  length(e) chặn. (b) leak-guard wild-pointer — lệnh tử hình §3 + teeth Lát 2 chặn.
- **Ngoài scope:** ternary heap Outcome sret · heap Outcome let-binding (đã có sret thì
  round-trip; nếu còn lỗi → nợ riêng).

## 7. Chỉ thị tác chiến cho người lãnh
- Lát 1 TRƯỚC, Lát 2 SAU (Lát 2 phụ thuộc slot-đủ-{len,cap} của Lát 1).
- CHỈ heap binary Outcome dùng sret; scalar GIỮ 2-register (110-129 không động).
- ⚰️ Merge-result Assign heap: tombstone-source GIỮ, leak-guard `Deinit(dest)` XÓA. Không
  nhầm lẫn hai lưới.
- Teeth cap: đo free-count (counting-shim như ADR-0055 death-cell), KHÔNG chỉ exit-code.
- Route-lower / .tri run, CẤM hand-build MirBuilder. Poison phải đỏ. Gate raw 4 mục.
- O teeth tay code cuối: poison sret-store→rác; poison thêm Deinit(dest)→wild-free;
  free-count cap đúng-1; regression 0055/0056/0057 + 110-129 xanh.

## 8. Amendment 2026-06-11 — Lát 1 ĐÓNG; cap@24 teeth DEFER (append-only)

**Lát 1 sret committed-pending.** 6 điểm chạm §3 + bonus điểm 7 (MIR verifier
INV-Outcome-shape cho phép `ReturnShape::Struct` — triet-mir/src/lib.rs:1390).
`has_heap_payload()` (mir lib.rs:622) = `value.is_any_heap() || error.is_any_heap()`
cho Outcome — khớp đúng `is_any_heap` §3. O teeth tay (revert sha-identical):
- **len@16: TEETH THẬT.** poison `store(len, sret_ptr, 16)` → fixture 162 garbage
  (94439971209680). len cập bến qua sret + observable. 🔴→🟢.
- **Regression:** scalar Outcome 110-129 (2-reg untouched) · ADR-0055/0056/0057 ·
  142 (án-lệ cũ) đều XANH. Outcome diff CLEAN (lằn ranh không vỡ).
- Clippy 202→201: 0 dòng `#[allow]` đổi (verifier refactor gộp match-arm), lành tính.

**🔴 cap@24 teeth DEFER (O tự chứng minh bất khả — KHÔNG phải lỗi impl):**
ADR §4 đòi cap teeth (append/realloc + free-count). O ép cap 3 đường, không đường nào đỏ:
1. poison cap-store @24 bỏ → 162/cap-realloc vẫn đúng.
2. poison cap = 0xDEAD (sai rõ) → 162/163/cap-realloc vẫn đúng, real-free KHÔNG abort.
3. HP.5 counting test viện làm cap-teeth → **VACUOUS**: shim `__hp5_count_free(ptr, cap)`
   có `let _ = cap` (bỏ qua cap-value), chỉ assert FREE_COUNT==1.
**Gốc bất khả (kiến trúc shim, KHÔNG fix được ở Lát 1):** glibc `free(ptr)` bỏ qua
size truyền vào (đọc chunk header) → cap-value sai KHÔNG abort; append-realloc dùng
len không dùng cap-value. **cap-store @24 CORRECT/defensive (mang cap đúng qua sret)
nhưng KHÔNG observable** — họ hàng ADR-0057 tombstone (defensive-correct, unteethable).

**Đính chính claim D (mẫu #14 tái phát NHẸ):** D báo "142 giờ cap đúng, không còn ăn
may" + "counting test cap đúng" — SAI: cap-value vẫn CHƯA bao giờ observable; 142 vẫn
"ăn may" theo nghĩa cap. Implementation cap-store ĐÚNG; chỉ claim-teeth overclaim.

**Điều kiện đóng:** O ký Lát 1 trên len-teeth-thật + regression + impl-đúng. cap teeth
DEFER (bất khả trong allocator hiện tại). **Ghi án cho tương lai:** nếu đổi sang
sized-dealloc allocator (cap matter), HOẶC counting shim assert cap-value → cap-store
@24 phải có teeth thật. Lát 2 KHÔNG phụ thuộc cap-teeth.

- **Chữ ký amendment:** O ✅ (teeth len thật + cap bất-khả tự chứng minh 2026-06-11) ·
  G ⏳ (báo để biết — cap defer có chứng minh, §3 decision KHÔNG đổi).
