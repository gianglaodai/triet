# ADR 0070 — Partial-move & Field-level Move-state (ZST/Capability)

**Trạng thái:** **🔒 SEALED** (Mentor G ký 2026-06-25; O verify máu 5 teeth đỏ độc lập + restore byte-identical). Áp dụng cho rewrite-era (Bậc C). Mở khóa partial-move field-level cho ZST/Capability field, hoàn tất canonical proof của schema §10 `HardwareToken` (destructure `let vga = hw.vga`). Amend [ADR-0025](0025-borrow-checker-rules.md) §5.3 + §9 schema BorrowChecker. Sibling của [ADR-0069](0069-zst-capability-token-luk3.md) (capability Ł3 ZST token).

**Issue:** Cỗ máy capability (ADR-0069) đã ship đầy đủ Ł3 — nhưng Lát 4 (A2) phải **né** tầm nhìn gốc schema §10: capability được truyền qua *param riêng* (`use_vga(vga: VgaBuffer)`) thay vì gói trong một struct `Hardware { vga, uart }` rồi destructure (`let vga = hw.vga`). Lý do né: borrow checker hiện track move-state **per-Local** (`var_states: BTreeMap<Local, VarState>` — `checker.rs:153`), không có cách biểu diễn "`hw.vga` đã moved nhưng `hw.uart` còn sống". Hệ quả: Δ3 (`checker.rs:615–624`) **cấm tuyệt đối** mọi việc trích Move-type ra khỏi field (`CannotCopyMoveTypeOut`). Không có per-Place move-state thì không có partial-move, và schema §10 — bằng chứng cốt lõi rằng capability = ownership thuần, zero-cost — không bao giờ compile được.

§5.3 ADR-0025 đã khóa mô hình 3-state (Owned/Moved/Conditionally-moved) nhưng *per-binding* và ghi "chi tiết defer implementation phase". ADR này lấp đúng phần defer đó cho chiều **field-level**.

---

## Quyết định

**Nâng move-state của borrow checker từ per-Local → per-Place (field-aware), đối xứng với loan-tracking đã có** (`places_conflict`, `checker.rs:46–89`, vốn đã field-aware).

Bốn khóa cụ thể:

1. **Per-field move-state.** Mỗi `Local` mang thêm tập field đã moved (single-level field projection: `hw.vga`). Đọc một Move-type field ra (`let v = hw.vga`) ghi field đó vào tập moved — KHÔNG còn refuse.

2. **Phạm vi có RĂNG CƯA — chỉ ZST/Capability.** Partial-move chỉ mở cho field kiểu **`Capability`** (ZST, drop = no-op runtime, không heap → không double-free khả dĩ). Field **heap** (String/Vector/HashMap) hoặc Move-type khác trích ra khỏi struct **VẪN BỊ TỪ CHỐI** bằng `CannotCopyMoveTypeOut` — đỏ chót, **KHÔNG panic**. (Field Copy đã được phép sẵn vì `is_copy`=true bỏ qua Δ3.) Partial-move field heap đẩy sang sổ đỏ No-Box (đòi JIT nhúng dynamic drop-flag/tombstone chặn double-free runtime).

3. **Tái dùng E2420 UseAfterMove** (KHÔNG đẻ mã mới). Dùng lại bất kỳ phần đã moved đều là UseAfterMove:
   - `let v = hw.vga; hw.vga` → E2420 (field đã moved).
   - `let v = hw.vga; let w = hw` → E2420 (động vào xác chết đã bị moi mất một phần).
   - Sibling chưa moved (`hw.uart`) → hợp lệ.
   - Diagnostic CÓ THỂ ghi rõ "partially moved", nhưng **mã chuẩn = E2420**.

4. **Merge per-Place tại join CFG = hợp (union), conservative, đơn điệu.** Field moved trên BẤT KỲ nhánh predecessor nào → moved tại join. Union là phép đơn điệu (lattice tăng dần) ⇒ fixpoint **hội tụ** (không treo vô hạn — điều kiện sống còn cho dataflow per-Place).

### Hình thức cụ thể

Canonical proof (schema §10, nay compile + chạy được):

```triet
capability VgaBuffer grant
capability UartPort  grant
struct Hardware { vga: VgaBuffer, uart: UartPort }   // all-ZST aggregate

function use_vga(v: VgaBuffer) -> Integer { return 10 }
function use_uart(u: UartPort) -> Integer { return 7 }

function main() -> Integer {
    let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
    let v = hw.vga;        // PARTIAL MOVE — vga ra khỏi hw, 0 byte runtime
    let a = use_vga(v);    // vga MOVED → 10
    let u = hw.uart;       // sibling — VẪN SỐNG (uart chưa moved)
    let b = use_uart(u);   // → 7
    return a + b;          // 17
    // hw.vga;             // ← nếu đụng vào: E2420 (vga đã moved)
    // let w = hw;         // ← nếu đụng vào: E2420 (hw đã partial-moved)
}
```

Răng cưa ranh giới (heap field — phải đỏ, không panic):

```triet
struct Box { name: String }
function main() -> Integer {
    let b = Box { name: "hi" };
    let n = b.name;        // E24xx CannotCopyMoveTypeOut — KHÔNG mở cho heap
    return 0
}
```

Răng dataflow CFG (field moved trong một nhánh, dùng tại join):

```triet
let hw = Hardware { vga: mint VgaBuffer, uart: mint UartPort };
if cond {
    let v = hw.vga;        // moved chỉ trên nhánh then
}
let again = hw.vga;        // JOIN — E2420 (union: moved trên ≥1 nhánh)
```

### Mô hình dữ liệu (định hướng — chi tiết ở Work Order)

`BlockState` thêm trường per-Local tập field đã moved, ví dụ:

```rust
partial_moves: BTreeMap<Local, BTreeSet<String>>   // local → {field names moved}
```

- Use của base `hw` (không projection) → moved nếu `var_states[hw]==Moved` **HOẶC** `partial_moves[hw]` khác rỗng.
- Use của `hw.f` → moved nếu base Moved **HOẶC** `f ∈ partial_moves[hw]`.
- `merge`: `partial_moves` lấy **union** field-set qua các predecessor.
- `StorageLive` / tái-gán fresh → xóa `partial_moves[local]`.

Single-level field depth (`hw.vga`) là phạm vi ADR này — `hw.a.b` lồng sâu → conservative whole-base move (hoặc defer), KHÔNG mở field-path đa cấp ở đây.

---

## Các phương án đã cân nhắc

| # | Phương án | Ưu | Nhược | Kết luận |
|---|-----------|---|-------|----------|
| 1 | **Per-Place move-state, scope ZST/Capability** (chọn) | Đối xứng loan-tracking đã có; sound zero-cost (ZST drop no-op); union monotone → fixpoint hội tụ; canonical §10 chạy | Đụng dataflow core, cần test CFG-branch kỹ | **CHỌN** |
| 2 | Per-Place move-state mở luôn cho **heap field** | Tổng quát hơn | Đòi JIT nhúng dynamic drop-flag runtime chặn double-free → tự sát phạm vi, đụng Cranelift | Defer → sổ đỏ No-Box |
| 3 | Giữ per-Local, "force whole-struct move" rồi cấm dùng lại base | Không đổi data structure | Giết sibling field (`hw.uart` chết oan) → KHÔNG phải partial-move, phản schema §10 | Bác |
| 4 | Mã lỗi mới E2421 cho partial-move-reuse | "Tường minh" | Ngữ nghĩa y hệt UseAfterMove → đẻ mã rác; E2421 đã là SelfOwnershipParadox | Bác — tái dùng E2420 |

---

## Hậu quả

### Tích cực
- Hoàn tất canonical proof schema §10 `HardwareToken`: capability = ownership thuần, destructure-move, zero-cost — *chạy thật* chứ không còn "design only".
- Borrow checker có per-Place move-state — nền cho mọi partial-move tương lai (kể cả heap khi No-Box mở).
- Đối xứng hóa: move-tracking nay cùng độ phân giải field như loan-tracking.

### Tiêu cực
- Tăng độ phức tạp `BlockState` + `merge` + use-check (ba điểm phải sửa đồng bộ).
- Single-level field depth — `hw.a.b` chưa hỗ trợ (conservative).

### Rủi ro cần mitigate
- **Fixpoint treo / vỡ soundness ở merge** — bắt buộc union (monotone), bắt buộc test CFG-branch (răng #4). Đây là rủi ro G đã chỉ đích danh.
- **Răng cưa heap rò rỉ** — phải có test thọc String ra khỏi struct → `CannotCopyMoveTypeOut` đỏ, không panic (răng #5).
- **JIT all-ZST struct** — `struct Hardware` toàn field ZST = 0 byte; StructAlloc/field-read 0-byte có thể chạm edge-case Cranelift StackSlot size-0. Bắt buộc **probe Step 0** trước khi fix, refuse-over-guess.

---

## Ngày hiệu lực

- Rewrite-era Bậc C, sau ADR-0069 — kích hoạt khi Work Order ADR-0070 đóng (O verify máu + G ký).
- Amend [ADR-0025](0025-borrow-checker-rules.md) §5.3 (mở rộng move-state per-Place) — KHÔNG hồi tố, KHÔNG revisionism mô hình per-Local cũ (per-Local là subset của per-Place khi projection rỗng).
- Amend schema §9 `BorrowChecker` (ghi nhận field-level move granularity).
- Heap-field partial-move: KHÔNG áp dụng — defer sổ đỏ No-Box.
