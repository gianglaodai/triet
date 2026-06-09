# ADR-0051: Sáp nhập Borrowck về một tầng NLL MIR (khai tử AST-based borrowck)

## 1. Status
**Approved (O + G, 2026-06-09)** — Crusade #2. B2.0 spike ĐÓNG (O tự verify).
Vào B2.1 implementation.

**Phán quyết G (bất biến — không vi phạm trong implementation):**
1. **NLL MIR borrowck độc quyền 100% kiểm soát vòng đời** (lifetime, exclusivity,
   use-after-move, drop, aliasing). Typecheck chỉ còn check-kiểu + resolve-name.
2. **E25XX (actor/Send/Sync/Scope, ADR-0026) Ở LẠI typecheck** — ngữ nghĩa đồng
   thời, KHÔNG phải NLL dataflow. Đá khỏi B2.
3. **CẤM xóa mù (blind delete).** 99 fixtures xanh chỉ chứng minh case HIỆN CÓ
   được MIR cover — KHÔNG chứng minh mọi điểm-chặn typecheck thừa. Mỗi điểm-chặn
   phải qua quy trình kiểm chứng §5 trước khi xóa.

## 2. Context & Motivation

### 2.1. Hai cảnh sát chồng quyền trên cùng đoạn đường
| | Cảnh sát #1 (Typecheck) | Cảnh sát #2 (MIR borrowck) |
|---|---|---|
| Cơ chế | AST walk + live-range (syntactic) | NLL dataflow trên CFG |
| Vị trí | `typecheck/borrow_check.rs` (502 dòng) + move-state machine trong check.rs | `borrowck/checker.rs` |
| Chạy | Phase 2 — **FATAL, chặn trước MIR** | Phase 4 — chỉ chạy nếu typecheck sạch |
| Phát | E2400/E2402/E2403/E2410/E2411/**E2420**/E2421/E2422/E2430/**E2440** + E25XX | **E2420**/E2423/**E2440**/E2450 |

- **Trùng:** E2420 (UseAfterMove), E2440 (Exclusivity).
- **Chỉ MIR (NLL thật, vượt typecheck):** E2423, E2450.
- **Chỉ typecheck (MIR chưa cover):** E2400 lifetime, E2410 mutability, E2430 namespace, E25XX actor.

### 2.2. Bom: driver fatal-stop bịt mắt cảnh sát MIR
`driver/main.rs:58` — typecheck phát lỗi → `return ExitCode(3)`, **DỪNG trước phase 4**.
Hệ quả: chương trình typecheck bắt E2440 **không bao giờ tới MIR borrowck**. MIR
E2440 (`checker.rs`) là **dead-code-bị-che** cho mọi case typecheck cũng bắt →
"E2440 không teeth-isolate được" (poison MIR → fixture vẫn đỏ từ typecheck). Đây
là động cơ B2: cảnh sát MIR bị bịt mắt, không kiểm chứng được, dễ rot.

### 2.3. Tại sao MIR là tầng đúng
MirType (ADR-0050) đã đổ móng correct-by-construction. NLL dataflow trên CFG là
mô hình chuẩn cho lifetime/exclusivity/aliasing (Polonius-style). AST live-range
của typecheck là xấp xỉ syntactic over-strict ("any-branch-moves => moved",
borrow_check.rs tự nhận §loop-conservatism). Giữ 2 tầng = thừa + dễ lệch
(over/under-reject khi không đồng bộ).

## 3. Quyết định Kiến trúc
**Khai tử AST-based borrowck. NLL MIR là cảnh sát DUY NHẤT của vòng đời.**
- Xóa module `typecheck/borrow_check.rs` (E2440 AST live-range, 502 dòng).
- Xóa move-state machine E2420 trong `check.rs` (`MoveState` enum + `move_states`
  map + `mark_moved`/`check_used` — **1 emit site** check.rs:178, KHÔNG phải 18
  site rời như khảo sát thô; "18" gồm comment+test+call-site).
- Dời E2400/E2410 (và E2430 nếu thuộc vòng đời) sang MIR ở B2.2+.
- `driver` flow giữ nguyên: typecheck (giờ không borrowck) → lower → MIR verify →
  borrowck. Typecheck vẫn fatal cho LỖI KIỂU; vòng đời rơi xuống phase 4.

## 4. Phạm vi & Phân pha (G CAM KẾT TRỌN GÓI)

### B2.1 — Gỡ trùng E2420 + E2440 (bước đệm lấy niềm tin)
- **E2440:** xóa `borrow_check.rs` (502 dòng) + 1 consumer (check.rs:435 `analyze_function`).
- **E2420:** xóa move-state machine (1 emit + `mark_moved`/`check_used` calls).
- Xóa/chuyển unit test typecheck E2420/E2440 (test emitter đang xóa).
- `.tri` fixture GIỮ (`// ERROR: E2440` match MÃ không TÊN — typecheck
  `BorrowExclusivityViolation` vs MIR `NllExclusivityViolation` cùng mã E2440).

### B2.2+ — Dời E2400 (lifetime) + E2410 (mutability) sang MIR
Mỗi mã = 1 lát: xây MIR cover + quy trình §5 + teeth + xóa typecheck. Thứ tự theo
độ khó (mutability rẻ, lifetime khó). E2430 namespace: đánh giá thuộc-vòng-đời-hay-không
khi tới (có thể là resolve-name → ở lại typecheck).

### NGOÀI B2
E25XX (actor/Send/Sync/Scope, ADR-0026) — ở lại typecheck.

## 5. Quy trình kiểm chứng KHÔNG-XÓA-MÙ (G mệnh lệnh, áp mọi điểm-chặn)
Trước khi xóa BẤT KỲ điểm-chặn typecheck nào:
1. **Gom nhóm logic** — điểm-chặn bắt edge-case gì (vd move-out-of-struct,
   move-from-immutable-ref, reassign-after-move, branch-join-move).
2. **Kiểm fixture coverage** từng nhóm — bộ fixture hiện tại có test chưa?
3. **Nhóm chưa có test → BẮT D VIẾT FIXTURE TRƯỚC** (negative `// ERROR: EXXXX`).
4. **Tắt điểm-chặn → chạy fixture → răng cưa MIR phải cắn ĐÚNG nhóm đó** (báo lỗi
   từ MIR). Chứng minh 100% độ phủ MỚI được xóa.
5. **Sau xóa: teeth-isolate** — poison MIR EXXXX → fixture đỏ THẬT (không còn
   typecheck che). Đây là thắng lợi B2: lần đầu E2440/E2420 teeth-isolate được.

## 6. B2.0 Spike findings (O tự verify, không tin số D)
- **O tự teeth E2440:** stub typecheck `detect_conflicts`→no-op → fixture corpus
  **99/99**, 6 fixture E2440 bắt đúng mã **từ MIR**. MIR cover E2440 ✓ (case khó
  nhất — khác tên cùng mã).
- **Fixture match MÃ không TÊN** (`integration_tests.rs:36`) → đổi emitter không vỡ.
- **Harness collect-all-phases** (chạy tới borrowck dù typecheck lỗi,
  integration_tests.rs:65) → fixture đã mô phỏng đúng post-B2.1.
- **Caveat:** O chỉ tự verify E2440. E2420 (move-state machine) — 5 fixture spike
  pass nhưng quy trình §5 BẮT BUỘC trước khi xóa (G: cấm xóa mù).

## 7. Consequences
- **Tích cực:** 1 cảnh sát vòng đời (NLL MIR), teeth-isolate được, xóa ~600 dòng
  AST-borrowck over-strict, hết nguy cơ 2-tầng-lệch. Frontend gọn (chỉ kiểu+name).
- **Tiêu cực:** B2.2+ phải xây MIR cover E2400/E2410 (MIR chưa có) — công thật,
  không chỉ xóa. Rủi ro under-reject nếu §5 lơ là.
- **Nợ liên quan (canh, không tái sinh):** `conservative=true` (B3) · `is_propagated`
  (A1) trong checker.rs — B2 đụng vào không được tái sinh bom.
- **Móng từ B1a:** MirType + Struct/Enum split sẵn sàng gánh NLL.
