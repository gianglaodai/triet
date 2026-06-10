# ADR-0054 — Core-Borrowck-Patch: Drop kills liveness (use-after-Drop → E2421 UseAfterStorageEnd)

- **Status:** 🔒 LOCKED — G ký duyệt 2026-06-11. ⛔ BÁO ĐỘNG ĐỎ (critical foundation soundness hole).
- **Date:** 2026-06-11
- **Khởi thảo:** Mentor O. Phán quyết front + §7 chốt bởi G (2026-06-11).
- **Chữ ký:** O ✅ (grounded từ HP.0 spike) · G ✅ (ký duyệt 2026-06-11).
- **Liên quan:** [ADR-0053 §9](0053-heap-payload-outcome.md) (HP.0 spike phơi lỗ), ADR-0025 (E24XX borrowck).

---

## 1. Context — báo động đỏ từ HP.0

HP.0 spike (ADR-0053 §9.3) phơi: MIR sinh `Drop(x)` rồi ngay sau `move x` vào biến khác, borrowck
báo **"OK (no borrow errors)"**. Teeth độc lập (KHÔNG dính Outcome) xác nhận ở mức nền:

```rust
// hand-build MIR Body, s: String (Move type)
Drop(s)                      // free String
assign(other, s)             // move s AFTER drop
// check_body(&body).errors == []   ← MÙ. Phải là E2420.
```

Đây **KHÔNG phải bug Ergonomics/Outcome** — nó là lỗ soundness **cấp nền của NLL borrowck**, nát
**MỌI Move type** (String/Vector/HashMap), bất kể Outcome. Một compiler để lọt use-after-Drop là
compiler ăn mòn bộ nhớ. Phải vá TRƯỚC mọi heap-payload work (HP.4 dừng — ADR-0053 §9.4).

## 2. Root cause (đo từ code, KHÔNG đoán)

`VarState` (checker.rs:134-145) có 3 trạng thái:
- `Owned` — dùng được.
- `Moved` — đã move → **mọi use là E2420**.
- `Ended` — storage kết thúc bởi Drop/StorageDead. Doc (dòng 140-143): *"Return vẫn consume được,
  nhưng **any other use is E2420**. Tách khỏi `Moved` để check E2450 tại Return không kèm false-positive E2420."*

Handler `Drop(l)` (checker.rs:720-722) **set đúng** `var_states[l] = Ended` (nếu chưa `Moved`).
**Lỗ:** các **use-site** (Assign-source move-check, BinOp operands, …) **CHỈ** coi `Moved` là E2420,
**bỏ qua `Ended`**. ⟹ hợp đồng "`Ended` + use ⟹ E2420" được **ghi trong doc nhưng KHÔNG enforce**.
Drop set Ended, nhưng không ai chặn move-of-Ended. Đó là toàn bộ con quái vật.

## 3. Decision (G chốt — chờ ký)

1. **Enforce hợp đồng `Ended` với MÃ MỚI E2421 (G chốt):** mọi **đọc/move/borrow** một local
   **Move-type** đang `Ended` (sau Drop) → **E2421 UseAfterStorageEnd** (mã mới, KHÔNG gộp E2420). Drop =
   thao tác **kill liveness**. Hai mental-model tách bạch: E2420 = "đã move cho kẻ khác" (chủ động);
   **E2421 = "đã bị tiêu hủy hết vòng đời, không đào xác lên xài"** (vòng đời/tự động). Cần variant
   `BorrowError::UseAfterStorageEnd` + `#[diagnostic(code(triet::borrow::E2421))]`.
2. **Giữ NGOẠI LỆ Return:** terminator `Return` consume một local `Ended` vẫn HỢP LỆ (không E2421) —
   đây là lý do `Ended` tách khỏi `Moved` ngay từ đầu. KHÔNG được phá (nếu không false-positive
   Return-of-dropped). Fix phải phân biệt **use-thường (E2421)** vs **Return-consume (OK)**.
3. **Phạm vi: CHỈ Move type (G chốt).** Copy type (vô hướng, không heap): Drop = no-op ảo, dữ liệu an
   toàn trên stack, KHÔNG rủi ro UAF → **KHÔNG enforce** (siết Copy = false-positive rác, biến ngôn ngữ
   cứng nhắc ngu ngốc). E2421 nhắm thẳng Move-only (String/Vector/HashMap/Struct chứa chúng). Sửa ở
   borrowck core (checker.rs use-site checks), KHÔNG đụng lowerer/MIR.

   > **§3 amend (2026-06-11, D implement):** variant đặt tên `UseAfterStorageEnd` thay vì
   > `UseAfterStorageEnd` như bản G ký. Lý do: `VarState::Ended` được set bởi cả `Drop` lẫn
   > `StorageDead` — tên `UseAfterStorageEnd` phủ chính xác cả hai nguồn. G đồng ý.

## 4. Teeth (bắt buộc đỏ→xanh; ai lãnh task phải làm chuyển)

- **T1 (entry point, O đã dựng & chứng minh `got: []`):** hand-build `Body { Drop(s:String); assign(other, s) }`
  → `check_body` PHẢI emit **`E2421 UseAfterStorageEnd`**. Hiện mù → fail. (Test `drop_then_move_must_be_rejected`.)
- **T2 (ngoại lệ Return không vỡ):** `Body { ... Drop(s); Return([s]) }` hoặc move-rồi-Return hợp lệ
  hiện có — PHẢI VẪN XANH (no false-positive E2421). 20 test borrowck hiện hành xanh sau fix.
- **T2b (Copy không over-reject):** `Body { Drop(n:Integer); assign(other, n) }` — Copy type, Drop=no-op
  → PHẢI XANH (KHÔNG E2421). Chứng minh phạm vi chỉ Move-type.
- **T3 (regression hậu-fix, gắn ADR-0053):** sau khi F1 (desugar heap-aware) sửa, HP.0 map chain
  `~+> |v| v` (String) KHÔNG còn Drop-then-move → borrowck xanh ĐÚNG (không phải mù).

## 5. Thứ tự thi công (G chốt)
1. **ADR-0054 (front này) TRƯỚC** — vá borrowck core, T1 đỏ→xanh, T2 giữ xanh.
2. Rồi mới ADR-0053 HP.1→HP.4 (heap payload Outcome). HP.4 (map) cần F1 desugar + T3.
3. Người lãnh: D (hoặc ai nhận) — KHÔNG blueprint mớm; bar = T1 xanh + T2 không vỡ + gate raw.

## 6. Consequences
- **Tích cực:** bịt lỗ soundness nền; mọi Move type an toàn use-after-Drop; mở đường heap Outcome.
- **Rủi ro fix:** ranh giới `Ended`-use (E2420) vs `Ended`-Return (OK) tinh tế — dễ false-positive
  Return nếu enforce thô. Teeth T2 là lưới. Có thể lộ thêm site hiện đang dựa ngầm vào sự khoan dung
  của `Ended` (đếm khi fix).
- **Không ABI/lowerer:** thuần borrowck — không đụng JIT/MIR shape.

## 7. Phán quyết G (CHỐT 2026-06-11 — đóng §7)
1. **Mã lỗi: TÁCH MÃ MỚI E2421 (UseAfterStorageEnd).** KHÔNG gộp E2420. Hai mental-model khác nhau hoàn
   toàn: E2420 "đã chuyển nhượng cho kẻ khác, hết quyền" (chủ động) vs E2421 "đã bị tiêu hủy hết vòng
   đời, không đào xác lên xài" (vòng đời/tự động). Compiler xịn bắt đúng mạch tâm lý người dùng.
2. **Phạm vi: CHỈ Move type.** Copy type Drop=no-op ảo, dữ liệu an toàn trên stack, không UAF → siết
   Copy = false-positive rác, ngôn ngữ cứng nhắc ngu ngốc. E2421 nhắm yết hầu Move-only.

**§7 đóng. ADR-0054 G ký duyệt — Chiến dịch Vá Móng bắt đầu.**

## 8. Chỉ thị tác chiến cho người lãnh (G, KHÔNG blueprint)
"Vào `checker.rs`, khóa chặt lỗ `VarState::Ended`, ném **E2421** khi vi phạm, GIỮ nguyên ngoại lệ
`Return-leniency` để 20 test cũ không vỡ. Phải làm bài test MIR của Mentor O (`drop_then_move_must_be_rejected`)
ĐỎ QUẠCH vì E2421." Bar O gác cổng: T1 đỏ-vì-E2421 → xanh · T2/T2b không vỡ · gate RAW. Giao thức Thép áp.
