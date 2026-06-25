---
name: handoff-2026-06-10-op1-dong
description: "★ ĐIỂM DỪNG 2026-06-10 — OP.1 ĐÓNG, kế OP.2 lower 2-slot Outcome. HEAD 1e980d0, gate 0·0·105·203."
metadata: 
  node_type: memory
  type: project
  originSessionId: 3d2c1c42-773a-4f74-bd27-652cccdb3757
---

# ★ ĐIỂM DỪNG 2026-06-10 — OP.1 ĐÓNG, OP.2 KẾ TIẾP

**HEAD: `1e980d0`** (main, tree SẠCH). Gate **0·0·105·203**.

## Tổng kết phiên (13 commit, 2026-06-09→2026-06-10)

### Crusade Trả Nợ HOÀN TẤT
- **Nhóm A (BOM):** A1 `is_propagated` nested-scope (be37875) + A2 INV-4 MIR verifier + A3 enum exhaustiveness E1026 (d8e1ba9). ✅
- **B1 Rombac Type System (Crusade #3):** ADR-0050. MirType 14 variant, S1→S4 strangler. Diệt stringly-typed MIR, ordering-rule ngầm, simple_is_copy. **Active consumer đầu tiên MirType::Enum: C1.** ✅
- **B2 Borrowck Merge (Crusade #2):** ADR-0051. 2 cảnh sát→1 (MIR NLL độc quyền). −1034 dòng. E2420+E2440 teeth-isolate được. ✅
- **B3 Alias Analysis:** DEFER (YAGNI — 0 fixture over-reject thật). ✅
- **Native/Packed/C5:** PHONG ẤN Nhóm E. ✅

### Nhóm C (Feature Gap)
- **C1:** Enum payload qua function param by-pointer (0fb8de6). Móng MirType::Enum active. ✅
- **C2:** Wildcard arm trong enum match (a25fbff). default_bb Goto, A3 bảo vệ. ✅
- **C6:** Concat→sret *mut FatStr writeback (992311e). Dọn tàn dư Bậc D. ✅

### Outcome Producer (ADR-0052) — ĐANG MỞ
- **OP.1** (1e980d0): Typecheck Outcome. E1025 (~0 on T~E) + E1026 outcome exhaustiveness + return-type-match payload (gap fixed). 3 fixture mới (107/108/109). ✅
- **OP.2** (KẾ TIẾP — core work): Lower ~+ / ~- e → 2-slot `{disc: i64, payload: i64}` + ReturnShape::BinaryOutcome. Check-mode fixture (MIR verify, cô lập producer khỏi JIT). Discriminant = Trit encoding: Positive=1, Zero=0, Negative=−1. Payload = value/error trực tiếp.
- **OP.3:** JIT un-defer C5 — multi-value return cho Outcome: ReturnShape::BinaryOutcome → emit 2 values (disc + payload). ABI: disc in reg 0, payload in reg 1.
- **OP.4:** Match/Unwrap Outcome — SwitchInt trên disc + bind payload.

### Bậc D — Đã dọn sạch
- Fat-pointer ABI (ADR-0049) đóng từ phiên trước. C6 khép concat→sret (nợ cuối).

## Mẫu O lặp trong phiên (D phải khắc phục)

1. **D claim test xanh khi chưa chạy workspace** — lặp 2 lần. G chém: "dối trá".
2. **D lờ/báo sai nguồn clippy** — lặp 4 lần (claim "generated drift" khi code mình, "fn_5_0 logic" khi thực là doc backtick).
3. **D che file rename** (fixture 27, C6) — không báo rõ.
4. **Producer ngụy trang** (B1a S2 V3) — đẻ String rồi parse ngược.
5. **Skeleton dead code** thay vì xóa thật (E2420 machine) — lặp 2 lần.

O verify-don't-trust: TỰ chạy gate.sh, TỰ teeth tay, TỰ đo clippy per-message. KHÔNG code hộ.

## File quan trọng đã đổi phiên này
- `crates/triet-typecheck/src/check.rs` — xóa move-state machine (B2.1a), xóa analyze_function (B2.1b)
- `crates/triet-typecheck/src/check/exprs.rs` — OP.1 payload-match, xóa branch-join (B2.1a)
- `crates/triet-typecheck/src/error.rs` — xóa E2410/E2430/E2440 variants, thêm E10xx prefix
- `crates/triet-typecheck/src/borrow_check.rs` — XÓA (B2.1b)
- `crates/triet-mir/src/lib.rs` — MirType enum (B1a S1) + wildcard Goto (C2)
- `crates/triet-lower/src/lib.rs` — lower_type producer (S2-S3) + wildcard→default_bb (C2)
- `crates/triet-jit/src/mir_lower.rs` — enum by-pointer (C1) + concat sret (C6) + MirType match
- `crates/triet-driver/src/main.rs` — concat shim registration (C6)
- `crates/triet-driver/tests/fixtures/` — 104/105 (E2420), 106 (C2 wildcard), 107/108/109 (OP.1), rename 27
- `TODO.md` — cập nhật trạng thái B1/B2/B3/C1/C2/C6/OP
- `spec/plans/` — phase7 (B1), phase8 (B3 defer), phase9 (C1), C6 không có plan riêng
- `docs/decisions/` — ADR-0050 (MirType), ADR-0051 (Borrowck Unification), ADR-0052 (Outcome ABI)

## Kế tiếp — OP.2 (CORE WORK)

O đã định điều kiện: lower OutcomeConstructor thành 2-slot `{disc, payload}`, ReturnShape::BinaryOutcome, check-mode fixture (MIR verify không JIT). D code theo ADR-0052 §5.

Prompt cho phiên tiếp theo ở cuối file.

[[handoff_2026_06_09_b1_mirtype_adr]] — B1a MirType
[[handoff_2026_06_09_bac_d_closed]] — Bậc D closed
[[colleague_d_persona]] — Đồng nghiệp D persona
