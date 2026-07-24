---
name: campaign_front_b_panic_audit_group_a
description: "Front-B WO-1 — panic-audit mir_lower.rs Nhóm A (Internal invariants) + JitError::Internal taxonomy; Nhóm B bia mộ, Nhóm C defer"
metadata: 
  node_type: memory
  type: project
  originSessionId: 1f77ea83-6290-499f-8cde-98ba80a1d51b
  modified: 2026-07-24T16:40:11.886Z
---

# FRONT-B WO-1 — PANIC-AUDIT `mir_lower.rs` NHÓM A (đóng 2026-07-24(c))

origin/main **`e2db04d`** (synced sau timeout-rồi-retry). Gate `0·clean·0·452·0`.
7 commit push (`6ae6cc3` fmt-gate + 6 WO). D = Sonnet 5 subagent (Giang lệnh spawn),
O gác cổng verify máu, G ký hai lần (duyệt WO + đóng lát).

## Bối cảnh
Mở đầu Front-B (G ghim từ phiên Front-A). Task hạ tầng đầu phiên: **vã `cargo fmt
--all --check` vào `scripts/gate.sh`** — lỗ lưới an toàn Front-A (code chưa fmt lọt
máu đầu O, chỉ pre-commit hook bắt). Gate giờ mirror hook **verbatim** (cùng lệnh,
cùng stable channel, `2>/dev/null` im tiếng nightly-option `imports_granularity`/
`group_imports` mà stable skip ở CẢ hook LẪN gate → hai lưới không lệch). Commit `6ae6cc3`.

## Recon TRIAGE (O tự đọc file:line, KHÔNG đoán)
`crates/triet-jit/src/mir_lower.rs` 10062 dòng, `JitError{Unsupported,Module}`.
- **Ranh giới `#[cfg(test)]` = dòng 6782** (ảnh chụp: 6753 lúc recon, trôi sau edit).
  **42/51 site ≥ranh giới = test-infra → KHÔNG đụng.** Chỉ **9 site production**.
- **BẪY suýt dính (bộ nhớ cảnh báo tật bảng 7→8):** memory Front-A ghi "51 site" gộp
  cả test; O suýt map 12 production vì 6877/6912/6942 nằm NGAY sau ranh giới test.
  **Đọc mới thấy = 9, không đoán.** Đây đúng là "map bằng trí tưởng tượng" mà kỷ luật
  poison/verify cứu.
- **3 nhóm:**
  - **A. Internal-invariant (4):** 1066 (`Nullable(_)` sau strip — `T??` bất khả),
    1516 (`compile()` map rỗng), 2883 (String đã bắt if-let trên), 5031 (inner re-match `_`).
  - **B. Env-precondition (2):** 1466/1469 host ISA detect — nền tảng, không user-input.
  - **C. Runtime-shim OOM (3):** 5232/5535/6075 `Layout::from_size_align().unwrap()` —
    `string/vector/hashmap_layout`, chạy lúc THỰC THI, nổ chỉ khi `total>isize::MAX`.

## Phán quyết G (ADR-lite, chốt TRƯỚC WO)
1. **Nhóm A:** thêm `JitError::Internal(String)` mirror **triết lý E1190** (ADR-0086 "please
   report"), KHÔNG đẻ namespace mã `triet::jit::EXXXX` (scope creep, cần ADR riêng). 1066/1516
   → `Err(Internal)`. **2883/5031 TÁI CẤU TRÚC chứng minh exhaustiveness ở tầng type**, không
   để filler. → [[campaign_front_a_lower_error_codes]]
2. **Nhóm B:** GIỮ `expect` + comment `// RATIONALE: fatal environment error, abort intended`.
3. **Nhóm C:** DEFER, ghi sổ nợ TODO.md (`D-JIT-OOM`, nâng cấp null-return ở phase Sandboxing).

## Thực thi D (không bế tắc, không fallback, không vượt scope)
- **3c:** `if-let String / else{match}` → **một `match value{String|Integer|Trit|Unit}`** 4 arm;
  3 arm scalar chia helper mới `store_scalar_const` (mirror niche ~0/NULL_SENTINEL ADR-0062/0065).
- **3d:** inner `match op{... _=>unreachable!()}` xóa hẳn → 6 arm `BinOp::Eq/Ne/Lt/Le/Gt/Ge`
  cụ thể gọi nested fn `cmp` (icmp+select → Trilean! +1/-1).
- 6 commit: `1d6bc14`(enum) `db716c8`(3a/3b) `9afef87`(3c) `d48ed18`(3d) `3867408`(4) `e2db04d`(5).

## Teeth máu O (độc lập, verify-don't-trust)
- Tự chạy gate CLEAN (không tin raw D). Grep nghiệm thu: production <6782 chỉ còn Nhóm B(expect+
  RATIONALE)+C(unwrap), 0 `unreachable!`, test-infra bất động.
- **Tooth 3c THẬT (chuẩn sách giáo khoa):** cắm lại `_ => unreachable!("TOOTH PROBE")` vào
  `match value` → compiler kêu **`warning: unreachable pattern … collectively making this
  unreachable`** ⟹ 4 arm phủ kín ConstValue ở TẦNG TYPE. Khôi phục `cp` (md5 `25132ec…`), KHÔNG
  git checkout. → [[feedback_teeth_never_git_checkout]] [[feedback_poison_must_be_red]]
- **3a/3b:** provably-unreachable by construction → **KHÔNG bịa mock test giả poison** (G khen:
  defense-in-depth là áo giáp chìm, không lôi ra làm xiếc). → [[feedback_failure_mode_precision]]

## Bài học / vết
- **O nhận vết THỨ TỰ TRÌNH BÀY:** đẩy bản đồ triage lên trước làm báo cáo gate bị chìm; G mắng
  "nhảy cóc quy trình" DÙ gate đã chạy+commit đúng. Luật mới: **báo-gate-lên-đầu**, task hạ tầng
  luôn là Task 1 trong WO (kể cả khi đã xong, đánh dấu ✅+hash, KHÔNG bắt D làm lại = trung thực).
- **Push timeout 143 lần đầu (2 phút):** pre-push hook chạy clippy+test >2ph. Retry `timeout 300`
  → xong. ls-remote xác nhận trước+sau (đừng tin exit code push đơn lẻ). Remote `77fdbe3→e2db04d`.
- Tiered: mir_lower.rs thường Opus-only (ABI/IR) nhưng WO refactor-cục-bộ-hợp-đồng-chốt + O verify
  máu → Sonnet đủ; rủi ro 3c/3d chặn bằng "kẹt thì DỪNG-báo-O". D làm sạch, 0 lần dừng.

## Nợ còn treo (sổ đen G, chờ mở)
push_owned-vs-M3 isolation (defense-in-depth) · **D-JIT-OOM** (mới, Nhóm C) · container
Nullable(Struct-heap) refuse §15.6 · N1 widening · method-call Struct?/Enum? return over-refuse ·
Deep-Clone · drain · &0 Enum tiêu thụ · borrow-params &+ T · Front-B panic-audit CÒN Nhóm B/C
(đã xử: A đóng, B bia-mộ, C defer).
