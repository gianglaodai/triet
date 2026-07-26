---
name: campaign_iteration_slice2c_force_unwrap
description: "🏁 ĐÓNG — ADR-0089 Slice 2c: `!!` ForceUnwrap lowering đồng cấu Elvis (trap-on-null SIGILL + PA-3c identity move-out heap-scalar + fence Aggregate E1100). Phiên 2026-07-27, pháo đài #5. SHA aa500ab (code) + c77d674 (ADR)."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-26T18:04:53.752Z
  originSessionId: 1687df81-24d3-40a4-91cb-f1662466d6f7
---

# Phiên 2026-07-27 — 🏁 Pháo đài #5: ADR-0089 Slice 2c (`!!` ForceUnwrap)

origin/main **`c77d674`** (synced), gate **`0·clean·0·496·0`** (+8 fixtures 495/497/499-504).
Code `aa500ab` (D), ADR-0041 §AMEND-Slice2c `c77d674` (O). Giang chốt hướng (chọn Slice 2c
trong 5 ứng viên), G duyệt Scope A (Mirror-Elvis) + 4 điều kiện thép, O recon+WO+verify máu, D implement.

## 🏁 Cái gì đóng
`expr!!` (force-unwrap `T?` — historical primitive ADR-0020, operator family ADR-0039) nay LOWER
được (front-half lexer/parser/typecheck ĐÃ wire từ trước; lower RỖNG → E1100 = gap kinh điển).
**Đồng cấu tuyệt đối Elvis `?:`** (`triet-lower/lib.rs:4415`), khác đúng null-arm:
- **Trap-on-null:** operand `== NULL_SENTINEL` → `Terminator::Trap` (SIGILL), **KHÔNG merge** từ null-side.
- **Present = PA-3c identity:** `result = obj_val` (Assign). source=named-local non-Copy heap-scalar
  (String?/Vector?/HashMap?) ⇒ borrowck mark Moved (`checker.rs:975`) ⇒ đọc lại → **E2420** (chữ ký
  move-out, giết alias double-free). Copy scalar (Integer?/Trit?/Trilean?) non-consuming.
- **Fence:** payload `matches!(MirType::Struct(_)|Enum(_))` → **E1100** (defer ownership projection).
- **1 điểm chạm:** arm `Expr::ForceUnwrap` trong `lib.rs:4508`. KHÔNG đụng jit/mir/typecheck/schema/borrowck.

## 🔑 Recon-first bắt gap + verify claim G (verify-don't-trust cắt CẢ HAI chiều)
- Front-half `!!` đã wire (lexer BangBang token.rs:323 · parser expr.rs:786 · typecheck check_force_unwrap
  exprs.rs:1795, test dùng `String?` → cam kết heap, KHÔNG cho phép lùi scalar-only). Lower thiếu arm.
- **Bác claim G E2403** → đo thực **E2420** (`checker.rs:296` UseAfterMove). G nhận, confirm.
- **Bác claim G "String đôi khi mang `Struct("String")`"** → đo: MirType::String/Vector/HashMap là
  **variant RIÊNG** (mir:490/530/532), KHÔNG phải Struct. Fence `matches!(Struct|Enum)` tự loại String;
  belt `!is_string_repr()` G gợi ý = thắt-lưng-thừa-vô-hại. D chọn bỏ (simplicity). Đúng.
- Verify mắt xích cuối: `lower_expr(Expr::Identifier x)` trả local x TRỰC TIẾP (`lib.rs:3306` `return Ok(local)`)
  → present Assign source=x → move thật. Canary khả thi KHÔNG cần thêm borrowck (scope không phình).

## 🩸 Poison 2 mũi (O độc lập, G lệnh đích danh) — ĐỎ dứt khoát
Snapshot `cp` /tmp → poison → rebuild → đo → restore md5 (`78a3478`) + `git diff` RỖNG (KHÔNG git checkout).
1. **Tháo `Terminator::Trap`→Goto present:** 2 trap test `force_unwrap_null_trap.rs` FAILED
   (`expected signal 4, got None, success=true`). Trap teeth có răng.
2. **Tháo fence (`if false &&`):** corpus `FAIL 501/502: pipeline succeeded with 3/7` (E1100 mất);
   BONUS D đo trước: heap-bearing struct local → **double-free exit 134**. Fence load-bearing thật.

## 🥅 Bẫy harness suýt sập (bài học tầng đo)
Lần chạy poison-2 ĐẦU: `cargo test -p triet-driver 2>&1 | grep ...` rồi cat `tail -25` → thấy TOÀN "ok",
NO FAILED → suýt kết luận "poison không đỏ = fixture vacuous". SAI: corpus FAILED line nằm NGOÀI cửa sổ
tail-25 (nhiều test binary khác). Chạy RIÊNG `--test integration_tests integration_test_corpus` → lộ FAIL
501/502 ngay. Bài học: **grep-rồi-tail cắt cụt bằng chứng — chạy đúng test-target cô lập, đọc full-output
của CHÍNH nó**, đừng tin "im lặng = xanh" khi harness gộp nhiều binary. (Họ hàng nghi thức #15.)

## ⚙️ Quy trình 5 pha đầy đủ + kỷ luật hạ tầng
Giang chốt (AskUserQuestion 5 ứng viên) → G duyệt Scope A + 4 điều kiện thép + bắt buộc ADR amendment →
O recon file:line + verify claim G → soạn WO (còng sắt: 1 lệnh gate foreground, timeout 600s, cấm
background/Monitor, raw 5 dòng) → G duyệt WO → O spawn D (Sonnet 5, background agent) → D nộp `aa500ab` +
raw gate + tự poison 2 chỗ → O verify máu độc lập (rebuild-first luật#12, poison 2 mũi, canary/trap/fence
qua driver, MIR dump) → O ký → G ký (tự verify độc lập) → O commit ADR + push.
**D bác O 0 lần** (code sound ngay), tuân còng sắt hạ tầng (raw nguyên khối, không tóm tắt). D subprocess-
isolate 496/498 trap teeth ĐÚNG convention (SIGILL trong corpus giết harness — bài học Slice 2b).

## Nợ còn (7 ứng viên, G phong tỏa — chờ Giang+O chốt mở)
🔴 ADR-0088 double-nullable T?? (cliff nặng, ADR-first) · HashMap.drain() · Deep-Clone · §15.6 Vector<Leaf?> ·
N1 widening (chờ ADR-0065) · &mutable Vector drain · O(N) cursor-drain perf. ⚰️ ADR-0068 Box/recursive CẤM CỬA.

→ [[campaign_iteration_slice2b_drain]] [[campaign_iteration_slice1_2a]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]
