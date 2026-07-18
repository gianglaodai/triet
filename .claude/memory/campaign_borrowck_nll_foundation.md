---
name: campaign_borrowck_nll_foundation
description: "✅ ĐÓNG 2026-07-18 — ĐẠI PHẪU MÓNG BORROWCK: NLL liveness (giết Drop(reference)-as-read) + alias propagation qua Assign/CFG-merge + E2450 thay máu; kèm chuỗi &0 Enum (Lát A → P0 Enum-Return-sret → Lát B payload sub-borrow). origin/main 7cd387d, gate 0·0·407·0. HAI BỆNH CHE NHAU + fixture 102 làm chứng. MEMORY.md index chỉ trỏ vào đây."
metadata: 
  node_type: memory
  type: project
  originSessionId: ded168f4-7a9c-48af-9ed8-d64a9122041c
---

## ✅ ĐÓNG — 4 mặt trận, 4 commit PUSHED (O+G ký, 2026-07-18)

```
7cd387d  Lát B — enum payload sub-borrow (String/Struct/Vector/HashMap)   gate 0·0·407·0
45a431c  P0 MÓNG BORROWCK — NLL liveness + alias propagation + E2450      gate 0·0·400·0
ae20d75  P0 Enum Return sret + bịt twin Nullable(Enum) silent-miss        gate 0·0·398·0
d4486ad  Lát A — consume &0 Enum via match (ADR-0084 §AMEND, E1050)       gate 0·0·393·0
```
ADR-0046 **GIẢI ĐÔNG** (G rút lệnh "không đâm NLL" — chính cái wart nó bảo vệ là bug ta gỡ).

## 🔑 PHÁT HIỆN LỚN NHẤT — HAI BỆNH NGƯỢC CHIỀU CHE NHAU

| Bệnh | Cơ chế | Hậu quả |
|---|---|---|
| **Under-refuse** (alias loss) | `Statement::Assign` copy reference **không re-anchor loan** sang dest mới → loan neo local đã chết → NLL kết thúc đúng luật → giá trị reference sống tiếp **không khiên** | **UB câm lọt** ở MỌI CFG merge (match + if/else) |
| **Over-refuse** (lexical trá hình NLL) | `liveness.rs:191` đếm `Statement::Drop(l)` là **READ**. Lowerer sinh Drop cho mọi local kể cả reference — mà `&0` **không có drop obligation** → Drop là no-op ngữ nghĩa nhưng liveness coi là use thật → mọi loan sống tới cuối scope | chặn nhầm code đúng |

**Engine dataflow KHÔNG sai** — backward fixpoint đúng, `is_live_after` (`liveness.rs:107`) đã point-level chuẩn NLL. **Chỉ INPUT là rác.** (O+G đều đoán sai chỗ này trước khi đo.)

**⚰️ FIXTURE 102 = VIÊN NGỌC VƯƠNG MIỆN.** `102_nested_borrow_uaf_e2450` là UAF thật (reference tới block-local thoát block rồi bị đọc). Cái `Drop(reference)` giả cầy (bệnh 2) **vô tình che** nó. Sửa liveness ĐƠN LẺ → 102 **lọt câm (chạy ra 5)**. ⇒ **BẮT BUỘC hạ 4 trụ cùng một nhát**, không có đường đi từng bước an toàn. Máu hai chiều: chốt lắp → E2450; tháo **cả hai** site → chạy ra 5, exit 0.

## Bốn trụ cột (đã hạ)
1. **Liveness**: `Drop` chỉ là read khi local **không** reference-like (`is_reference_like` unwrap Nullable — `(&0 T)?` từ get_ref vẫn là reference). Luồn types vào `compute`.
2. **Alias**: `Assign` copy reference → re-anchor mọi loan có dest = local đó sang dest mới. **Tái dùng `PropagatedLoan` cross-call** (`checker.rs:1139`), không phát minh.
3. **E2450 thay máu**: tại mỗi điểm owner chết (Drop/StorageDead/move-out): `loan.source neo owner ∧ is_live_after(điểm, loan.dest)` ⇒ E2450. Bỏ short-circuit `!is_propagated` (gốc over-refuse).
4. **E2440 KHÔNG ĐỤNG** — O đo: đã đúng chuẩn, không ăn bám Drop.

**5 fixture "vỡ" chia 2 loại:** 94/95/21/24 = **over-refuse, vỡ là TÍNH NĂNG** (hình "tạo reference rồi không dùng lại" — NLL đúng phải cho qua) → viết lại thành vi phạm thật. 102 = **UAF thật** → phải cứu.

## ⚖ D BÁC O — 4 LẦN, ĐÚNG CẢ 4
1. §AMEND-3.2 predicate Copy · 2. failure-mode `&0` dangling · 3. **handle-repr vs inline-repr** · 4. **Trụ 3 (không phải Trụ 2) mới giữ 102**.

**#3 (Lát B):** WO của O viết luật chung `Struct hoặc is_any_heap() → Borrow`. **SAI cho Vector/HashMap** — chúng là **handle-repr** (value ĐÃ là con trỏ i64), `Borrow` trả địa-chỉ-ô-chứa-handle → shim đọc nhầm → **silent-MISS** (đo: `94891986642280` thay vì 3). Struct/Enum/**String** = **inline-repr** → `Borrow` đúng. **O trượt vì probe bằng String+Struct — hai mẫu CÙNG một lớp repr.**

## 🩸 BÀI HỌC O TỰ ĂN (khắc)
- **★ CƠ CHẾ PHÒNG THỦ CÓ ANH EM SINH ĐÔI — dính 3 LẦN.** Loan-ending có 2 site (`checker.rs:1091` statement-level + `:1347` terminator-level); E2450 sau khi sửa cũng có 2 site (Drop + StorageDead). **Poison từng cái = INERT** (cái kia gánh). Suýt kết luận "vô can" và đi sai hướng. ⇒ **Poison không đỏ ở một cơ chế phòng thủ → HỎI NGAY "có site song sinh không?"** Đây là biến thể (c) của poison-không-đỏ: không phải (a) bất-khả-observable, không phải (b) test yếu, mà **hai cơ chế che nhau**.
- **★ STALE BINARY — dính LẠI** (bài học #12 đã có trong sổ). Restore `checker.rs` + chạy gate nhưng **quên rebuild release** → báo cáo G rằng if/else "bắt được" (thực ra LỌT). Tự nghi vì thấy "bắt được mà không site nào bắn" → mới lộ. **`cargo build --release` TRƯỚC MỖI lần chạy binary.**
- **★ LỆNH GIT CHẠY KHÔNG LỖI ≠ LÀM ĐÚNG VIỆC.** Lúc viết lại commit message: `git cherry-pick` trả **empty** (không báo lỗi rõ) → `--amend` ngay sau **đè message Lát B lên commit móng nhà, xoá mất nó**. Bắt được vì có bước bắt buộc `git diff <gốc> HEAD` phải rỗng — nó KHÔNG rỗng. Khôi phục bằng branch `safety/` cắm trước khi động dao, dựng lại bằng **`git commit-tree`** (ghép tree chính xác vào parent chỉ định — an toàn hơn cherry-pick cho việc đổi message).
- **★ RÒ RỈ TOPOLOGY trong WO** (G bắt, không phải O): commit móng nhà dùng fixture 410/412 **viết bằng feature Lát B** → tách ra là vỡ. **Móng nhà TUYỆT ĐỐI không được mượn gạch của tính năng làm bài test chịu lực.** O đo bằng worktree cherry-pick lên `ae20d75` → FAIL 412 (E1050). Sửa: dời 410/412 sang Lát B, bù fixture **413 struct match-merge** (Alias Loss **độc lập Enum**).

## Dual Verification (G mandate — mỗi commit tự đứng)
| Mốc | Gate | Bằng chứng |
|---|---|---|
| Móng `45a431c` | 0·0·400·0 | over-refuse hết · merge leak đóng (`y_if`, struct-match-merge, **0 dòng Lát B**) · 102→E2450 · **`x_latB`→E1050** (Lát B vắng ⇒ móng tự đứng) |
| Lát B `7cd387d` | 0·0·407·0 | 4 payload chạy đúng (2/33/3/2) · P0 giữ nguyên · **`x_latB`→E2440** (feature có + khiên có) |

`x_latB`: **E1050 → E2440** qua hai mốc = bài thơ topology (G).

## Nợ chuyển tiếp
- **Caller/callee ReturnShape divergence → panic** (`mir_lower.rs` `inst_results[0]`): pre-existing, dùng chung với Struct, **không tới được từ user input** (cả hai đọc cùng `func_return_types`). Cần **cross-body ABI verify = ADR riêng**.
- **Full sret cho `Enum?`**: đụng disc-niche ADR-0065. Defer, **có refuse còi-to canh** (unit-only `Enum?` ở return position, predicate 3 tầng `Nullable ∧ Enum ∧ unit-only`).
- ~~**`Nullable(Enum)` trong `resolve_aggregate_size`** (`lower/lib.rs:503` tra nhầm `struct_map`): mìn hẹn giờ, bất-khả-observable hôm nay~~ → **✅ ĐÓNG 2026-07-18 (`186bd1c`, PA-A).** ⚠️ **Ghi chú này SAI HAI CHỖ:** (1) tọa độ thật là **`:567`**, không phải `:503` (số dòng trôi); (2) **KHÔNG "bất-khả-observable"** — là bug **SỐNG**, chạm được bằng 5 dòng Triết hợp lệ (`struct Mid{e:E?,m:Integer}` → `mid.m` đọc ra 42 thay 5, exit 0). Nó sống sót vì **ZERO COVERAGE**, không vì bị guard nhốt. **Bài học: "bất-khả-observable" là một CLAIM, phải đo trước khi ghi sổ — đừng suy ra từ "có vẻ bị refuse che".** Chi tiết → [[campaign_nullable_enum_aggregate_pa_a]].
- Reference trong struct field: lowerer refuse (không phải đường thoát).
- `&0 mutable` payload sub-borrow: ADR-0081 FROZEN.
- ⚠️ BOM FIX-2 zero-@8 · ⚰️ ADR-0068 Box CẤM CỬA.

[[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]] [[campaign_typed_collections]]
