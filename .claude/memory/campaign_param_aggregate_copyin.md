---
name: campaign_param_aggregate_copyin
description: "✅ ĐÓNG 2026-07-28 — WO-Param-Aggregate-CopyIn: struct param không có struct_slots entry ⇒ 49 cổng ownership của JIT mù ⇒ 3 họ UB (134/139/132). Vá gốc bằng copy-in prologue (ADR-0066 §AMEND-1). 0f11ede+2469e2e+2c75b60, gate 0·clean·0·546·0. Lộ nợ P0: sret trả rác CÂM exit 0."
metadata: 
  node_type: memory
  type: project
  originSessionId: fbe58419-7612-4d3d-906c-3e89337bfdc6
  modified: 2026-07-28T14:55:26.017Z
---

## ✅ ĐÓNG — `0f11ede` (fix+teeth) + `2469e2e` (tooth 555) + `2c75b60` (P0 ruling). O✅/G✅ 2026-07-28

origin/main `2c75b60`, gate `0·clean·0·546·0`. Fixture 533 → **546** (+13).

## ⚔ O BÁC NHÃN CỦA CHÍNH CHIẾN DỊCH — "param alias là bug" là SAI

Nhãn bàn giao (chẩn đoán của D, G chép lại): *"prologue để Variable của param alias
bộ nhớ caller thay vì copy-in"*. **Phép đo bác vế "alias là bug":**
`function take(p: Leaf) -> Integer { return 7; }` → **exit 0, free đúng 1 lần**; fixture
`258`/`260`/`264` (counting `FREE==1`) đang xanh trên chính hình đó.

Truyền by-pointer là **ABI có chủ ý** (call-site `mir_lower.rs:3818` truyền `stack_addr`
slot caller; `copy_base_addr:1250` nhánh else đọc con trỏ — ADR-0066 KCN-1b). **CƠ CHẾ NÀY ĐÚNG.**

🔑 **Bệnh thật:** struct param bị loại khỏi `struct_slots` tại `:2597`
(`i < reserved_locals → continue`), mà `struct_slots` là **cổng canh của gần như mọi cơ
chế ownership trong JIT** ⇒ mọi cổng đó **mù hoặc rơi fallback sai** trước param.

## 📊 BÁN KÍNH — MỌI cách dùng struct param heap-bearing đều UB

| Hình | Đo |
|---|---|
| param **không đụng tới** · Copy-struct (kể cả lồng) · `&0` borrow param | ✅ 0 |
| `return length(p.s)` — **chỉ ĐỌC field** | 🔴 134 |
| `let s = p.s` · `return p` | 🔴 134 |
| `let q = p` · struct lồng · **`return inner(p)` forwarding** | 🔴 139 |
| hai param heap / heap+Copy **có phép cộng** | 🔴 132 |
| `Leaf?` param | ✅ fail-closed `"Struct? Drop without slot"` `:3476` |

**S9 forwarding đáng sợ nhất:** không cần cú pháp lạ — chính lowerer tự phát
`Deinit(_0); Drop(_0)` sau `Call inner(_0)` (ADR-0042 Q1).

## 🔬 HAI HỌ TRIỆU CHỨNG, MỘT GỐC (O đo, không suy loại)

- **M-α → 139.** `Deinit(param)` `:2917`/`:2921` gated `struct_slots` → miss → rơi
  fallback `:2944` `def_var(var,0)` **xoá chính con trỏ caller** → `Drop` load tại địa chỉ 0.
- **M-β → 134.** Tombstone field-move-out `:3239` gated `struct_slots.get(&source.local)`
  → **bỏ qua CÂM cả khối**, gồm sync `len@8`/`cap@16` `:3251`.
- **132 KHÔNG phải bug riêng.** Tách biến: cò súng là **phép cộng**, không phải số param.
  `_2 = move _0.s` → `_3 = move _2.len` đọc `len@8` **chưa bao giờ được ghi** = rác →
  `Add` vi phạm range-check ADR-0044 → SIGILL.

🩸 **Hai probe thép:** (1) vô hiệu hoá `:2944` → **139 → 134** (chứng minh con trỏ-bị-zero
là đầu vào SIGSEGV, và bên dưới có double-free thật). (2) ghi mốc `777` vào `len@8` →
**132 → 134** (chứng minh SIGILL đến từ ô len rác). Control `c1` (local, MIR **y hệt**)
không đổi ở cả hai.

## 🦷 VÙNG MÙ — ĐO ĐƯỢC TỪ CORPUS, KHÔNG SUY LOẠI

533 fixture. Ba fixture heap-struct-param (`258`/`260`/`264`) có thân callee **đúng một
dòng `return 0`** = ô duy nhất còn sạch. `14` có đụng param nhưng `Point` là **Copy**.
⇒ **ZERO ca dùng thật một struct param heap-bearing.** Lặp y nguyên vùng mù `Pt{x,y}`-Copy
của phiên 07-27(f) — luật HP.3.

## 📍 FIX — 53 dòng, thuần THÊM, một chỗ

Prologue param loop cấp `StackSlot` + copy-in `layout.total_size` byte, **mirror String
`:2691` / Enum `:2730` / Outcome `:2756` — Struct là aggregate ABI thứ TƯ còn thiếu.**
Vòng chỉ duyệt `signature.parameters` ⇒ **`Local(0)` sret không bao giờ bị chạm.**
Phạm vi khoá `MirType::Struct` thuần; `Nullable(Struct)` giữ refuse fail-closed.

🔑 **Luận cứ khoá lệnh vá gốc: vá lẻ đã thất bại 3 lần** — `WO-NullableEnumParamABI`
(`:2704`), `WO-StructParamABI` (`:1343`), và lần này.

🔑 **Bất biến chịu lực O tìm ra:** copy-in chỉ sound vì lowerer phát `Deinit(arg)`
**vô điều kiện** cho mọi đối số Move-type. Gỡ nó → canary đỏ **hai tầng** (test structural
FAILED + SIGABRT 6 thật).

## ⚖ D BÁC O — ĐÚNG

WO của O trỏ bất biến vào `triet-lower/src/lib.rs:4462-4465`. **Sai chỗ.** Có **HAI** site
giống hệt: `:4462` (trả `ret_local`) và `:4544` (trả `dest`); `take(p) -> Integer` đi qua
site **thứ hai**. D tự dump, sửa đúng, canary poison của O chứng minh D đúng.

## 🔴 NỢ P0 LỘ RA — sret trả rác CÂM (G chốt mặt trận kế)

```tri
function make() -> Leaf { let p = Leaf { s: "hi" }; return p; }   // 0 THAM SỐ
```
→ trả **`94060113734544`**, **exit 0**. Không crash, không chẩn đoán.
Worktree sạch tại `35f4f02` → cũng rác ⇒ **pre-existing, trực giao**.
⇒ `WO-SRet-Aggregate-StringField-Corruption`, **P0**, cấm mở chiến dịch tính năng trước nó.
⚠️ Cơ chế (`_0` sret không có slot ⇒ sync `len@+8`/`cap@+16` `:3156-3168` không chạy) mới
là **chẩn đoán của D — O CHƯA đo độc lập.**

## ⚔ VẾT TRONG PHIÊN

- **O đếm hụt cổng: nói "10", thật là 49** (`grep -c`). Tự đính chính trong WO, bắt D nộp
  bảng phân loại đủ 49 site.
- **O grep `^FAIL` hụt vì dòng thụt 2 space** → suýt kết luận "không có răng". Luật
  *"vắng output ≠ xanh"* cứu lần thứ hai trong hai phiên liền.
- **D tóm tắt lệch bảng của chính nó** (prose "13/6/30" vs bảng thật **15/5/29**). Bảng đúng.
- **D nộp gate có mục `test failures` bị TÓM TẮT** → **O REJECT thẳng**, không đọc file,
  không chạy gate hộ. Lần đầu O thực thi sắc lệnh đúng như G đã ban (lịch sử: O từng nhượng
  bộ và biến điều luật thành trò dọa). D nộp lại raw ngay vòng sau.
- **O từ chối lệnh của G "O thêm fixture"** — ma trận quyền khoá cứng *D độc quyền cầm bút
  fixture* (dựng sau vụ APP.2b-1). O trả D viết, tự verify. Kết quả G muốn không đổi, luồng đúng.

## 🟡 NỢ NHỎ GHI SỔ

1. `step_by(8)` ghi lố slot khi `total_size` không bội của 8 (`triet-mir:1590`, xảy ra khi
   mọi field là Trit/Trilean/Tryte). O ép 2 probe → **hiện KHÔNG quan sát được** (Cranelift
   đệm frame). **Mẫu kế thừa từ nhánh Enum `:2743`**, không do WO này đẻ.
2. `param_aggregate_copyin_counting.rs` **in-process**, không subprocess-isolated → dưới
   poison cả binary chết, 6 assertion không báo riêng lẻ. Răng thật nhưng thô ở chiều đỏ.

[[campaign_aggregate_move_tombstone]] [[campaign_drain_fifo_teeth]] [[campaign_truc_b_heap_in_aggregate]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]
