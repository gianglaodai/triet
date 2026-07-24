---
name: campaign_spof_container_nullable
description: "✅ ĐÓNG 2026-07-25 — WO-SPOF-1: bịt SPOF refuse container-Nullable(Struct-heap) ở struct-field + enum-payload (Phương án B, ADR-0065 §15). origin/main 0285bf2, gate 0·clean·0·455·0. O bật G bằng số đo (framing 'UB thò ra' SAI — không live-UB; hố THẬT là SPOF một-lớp)."
metadata:
  node_type: memory
  type: project
  originSessionId: 1cdd999a-6278-40aa-b41e-b788b111d906
  modified: 2026-07-24T17:34:17.136Z
---

# WO-SPOF-1 — bịt SPOF refuse container-`Nullable(Struct-heap)` (đóng 2026-07-25)

origin/main **`0285bf2`** (synced sạch). Gate `0·clean·0·455·0`. Fixture 452→455.
Mặt trận G chọn phiên này = "container Nullable(Struct-heap) refuse §15.6" trong sổ nợ.
D = Sonnet 5 subagent (Giang lệnh spawn); O gác cổng verify máu; G duyệt recon + ký đóng.

## 🔑 O BẬT G BẰNG SỐ ĐO — framing "UB thò ra" đo được là SAI
G phán *"đuôi UB free(1) vẫn thò ra ở refuse-gap, dung túng UB tái sinh"*. O **verify-don't-trust
áp cả lên G** (nghi thức #10): probe P1/P3/P4/P5 → **KHÔNG có UB observable reachable**. WO-5
(`f432987`) đã bịt site nổ; construction-side scan `find_refused_nullable_container` chạy trên
**mọi** local_decl (`triet-mir/src/lib.rs:1976`) + return (`:1957`) tóm sạch mọi materialization
`Vector<Leaf?>` — kể cả inline `vector_new()` (P4 đẻ temp `_3` bị bắt) + params (P5 `_1`).

## Hố THẬT (khác loại): refuse là SPOF một-lớp, không phải live-UB
Predicate struct-field/enum-payload `find_refused_nullable_field` (`:1884`) có nhánh
`Nullable/Reference/Outcome/_=>None` — **KHÔNG có nhánh `Vector`/`HashMap`**. Doc-comment tự biện
"field là một position, không phải container" — **lập luận sai**: field kiểu `Vector<Leaf?>` vừa là
position vừa là container có element nullable-heap. Hai vòng verify struct-field (`:1990`) +
enum-payload (`:2002`) CHỈ gọi predicate thiếu-nhánh đó, KHÔNG gọi container-scan (khác return/local
chạy CẢ HAI). **P6/P7 chứng minh:** `function consume(b: Bag)` với `struct Bag{v: Vector<Leaf?>}` /
`enum Box{Full(Vector<Leaf?>)}` **QUA verify+JIT sạch, exit 0**, emit drop-glue `free(1)` latent.
An toàn treo trên MỘT gate (construction-scan) = **SPOF** đúng mẫu ADR-0085 `builtin_shim_meta`.
Phân định (a)/(b): (a) hiện bất-khả-observable (không caller dựng nổi non-empty vector) + (b) gap
predicate THẬT reachable ở verify → xếp **latent-UB SPOF**, không live-fire, không cosmetic.

## Fix — Phương án B (G chốt): defense-in-depth, KHÔNG nới feature
Thêm `find_refused_nullable_container(&field.ty/&payload.ty, self)` vào struct-field loop (`:1988`)
+ enum-payload loop (`:1999`), mirror return/local. **KHÔNG đụng** `find_refused_nullable_field`
(giữ check direct position — G lệnh), `_container`, `is_field_payload_lowerable`. `+35/-8` một file.
Position mới: `struct field \`Bag.v\` (container element)` / `enum payload \`Box.Full\` (container element)`.

## Teeth máu O (harness-level, luật 15/21)
- **Poison load-bearing:** swap PRE-D lib.rs (gỡ 2 check) giữ 3 fixture → corpus `FAIL 459/460:
  pipeline succeeded with 0` — hố mở, hai fixture ĐỎ. Gộp luôn harness-genuine (459/460 flip khi gỡ fix).
- **Poison 461 non-vacuous:** bịa `EXPECT 0→999` → `FAIL 461: expected 999, got 0`.
- Khôi phục `cp` snapshot (PRE_D/POST_D md5 `ff3d1fd1…`, 461.orig), KHÔNG git checkout. Gate cuối
  độc lập `0·clean·0·455·0`. Diff so baseline pre-D = đúng scope (chỉ lib.rs 2 loop + 3 fixture).

## Vết D + kỷ luật
- **D nộp gate TÓM TẮT lần đầu** (dòng `=== test failures ===` → "(xem log đầy đủ ở trên)"). O
  **REJECT thẳng** theo GIAO THỨC THÉP ("Dán Raw Gate hoặc cút") — KHÔNG verify hộ, KHÔNG nhượng.
  D ói raw + nhận lỗi. Mẫu reporting-discipline = giới hạn hạ tầng (đã kết luận); constraint cứng
  trong WO chỉ chặn khi O thực thi reject, không cứu hộ. G khen "đủ lạnh để cầm trịch".
- Push timeout dùng `timeout 300` (pre-push hook clippy+test); ls-remote xác nhận `6f546b6→0285bf2`.

## Nợ chuyển tiếp
- **§15.6-support** (gỡ refuse cho `Vector<Leaf?>` CHẠY qua `struct_drop` arm) — feature, **defer**.
- **MẶT TRẬN SAU G ĐÃ CHỐT (recon, chưa WO): push_owned-vs-M3 isolation** — SPOF `arg_consumes`
  (lowerer `push_owned` + JIT M3 đọc chung bảng, 0 giáp cross-check). G lệnh recon 3 việc: (1) chỉ
  file:line lowerer-dựa vs JIT-dựa/tin-mù; (2) chiến thuật isolation (JIT tự guard độc lập, không
  tin mù bảng); (3) poison đầu độc `arg_consumes` trả sai → ép compile lọt → JIT phải trap/panic
  chặn sau khi cắm giáp. **KHÔNG viết WO trước khi G duyệt recon.**

[[campaign_forgot_nullable_sweep]] [[campaign_shim_meta_spof_adr0085]] [[campaign_nullable_position_and_temp_ownership]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]
