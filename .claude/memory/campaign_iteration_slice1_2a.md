---
name: campaign_iteration_slice1_2a
description: "✅ ĐÓNG — ADR-0084 verify (386 vacuous→sole-guard) + ADR-0089 Iteration Slice 1 (loop/break/continue+for-Range) + Slice 2a (for-item Vector copy sugar). Phiên 2026-07-26, 3 pháo đài."
metadata:
  node_type: memory
  type: project
  originSessionId: cc94f7c9-d642-45c2-aa5f-f17917fd833b
  modified: 2026-07-26T10:44:40.186Z
---

# Phiên 2026-07-26 — 3 pháo đài: ADR-0084 verify + ADR-0089 Slice 1 + Slice 2a

origin/main **`adfe8f9`** (synced), gate `0·clean·0·479·0`.

## 🏁 ADR-0084 verify (`9ff47c1`) — 386 vacuous → sole-guard
Nợ-verify treo từ phiên trước: code land+corpus xanh nhưng ADR DRAFT + tooth-386 VACUOUS.
**O bóc mẽ harness:** `integration_test` `run_fixture` gộp-đa-pha (không dừng typecheck-fatal) →
E2450 chỉ hiện qua harness; **CLI thật 386 → E2400 typecheck-fatal** (main.rs:58-64 dừng trước
borrowck), user KHÔNG BAO GIỜ thấy E2450. G lệnh "đâm nách typecheck": thêm `dummy: &0 String` param
→ tie return-borrow → né E2400 → E2450 nổ SẠCH ở borrowck (đo 2 lần D+O). Thay ruột 386 = răng thật.
**O poison chase (checker.rs:710-714 plain-strip)** → 386 compile SẠCH exit 0 (would-dangle→JIT=UAF) =
chase **sole-guard trên đường tới-runtime** (khác 386 cũ E2400 che). ADR §định-lý-phân-tầng: typecheck
E2400=guard UNBOUND return-escape; borrowck chase=SOLE-GUARD cho E2440 (move-while-borrowed 387) +
E2450 (BOUND return-escape param-tie 386).

## 🏁 ADR-0089 Slice 1 (`85371a6`) — loop/break/continue + for-Range (Scope B, amends ADR-0003)
G chốt Scope B (concrete CFG desugar, CẤM generic trait — ADR-0003 trait-Iterator defer vô hạn +
tombstone "AI-first"). **Loop-context stack** (break_bb/continue_bb/drop_snapshot). for-Range=while-shape
+ **step block** (continue→step KHÔNG hdr, tránh vô hạn). break/continue emit_scope_drops
`owned_locals[snapshot..]` emit-không-clear (mirror flush_all_for_return Case-D) → drop đúng 1 lần/đường.
**Borrowck KHÔNG chạm** (CFG-generic fixpoint back-edge). 3 guard: **E1052** non-Range typecheck (kill
silent-Unknown), **E0009** parse_break reject break-value (G tự soi `stmt.rs:169` nuốt câm), **E1143**
break/continue-outside-loop (D bác giả định ADR "parser ràng break"=SAI bằng data; đổi từ E1140-mượn).
Răng permanent **`break_drop_counting.rs`** (O poison→FREE 3→2 leak). SPEC §7.2 hết nói dối.

## 🏁 ADR-0089 Slice 2a (`adfe8f9`) — for-item Vector copy sugar (scalar + bare copy-Struct)
`for item in v` desugar index-loop, **infallible in-bounds get** (bind `item:T`, KHÔNG `!!`/nullable),
tái dùng shim `__triet_vector_get`/`_get_copy` raw (bỏ nullable-wrap). **KHÔNG move-out/tombstone** —
copy bytes, v nguyên vẹn.
**Bãi mìn G #1 (heap-element by-value = alias→double-free):** refuse tại typecheck **E1053**.
**Bãi mìn G #2 (handle-aliasing container double-free):** desugar tái-dùng local của v (KHÔNG alias
handle vào owned_local mới). Răng permanent `vector_iter_container_free_counting.rs` FREE=1 lvalue+rvalue.
**🚩 O đào 2 loose-end sau khi D nộp lần 1:** (a) **asymmetry bẫy câm MỚI** — typecheck
`is_copy_aggregate()` broad allow Vector<CopyEnum>/Nullable nhưng lower E1100 (O probe Vector<Color>→E1100
SỐNG). G lệnh thắt typecheck khớp CHÍNH XÁC lower: `is_scalar() || (UserStruct && is_copy_aggregate())`.
(b) **dead code** — `if !is_lvalue push_owned` REDUNDANT (O poison 2 hướng FREE không đổi → truy
`emit_shim_call:1783` push_owns arg → dòng D thừa; map-trace D sót). Cleanup xóa + sửa comment.
Poison guard broad→485 E1100 (trap tái mở)=load-bearing.

## Bài học phiên (Mentor O)
1. **Harness ≠ thế giới thực** (386): răng qua test-harness-gộp-pha có thể VACUOUS — user thật thấy mã khác.
2. **Luật #12 stale-binary cứu 2 lần:** probe 478/485 ra E1140/E1100 do `./target/release` chưa rebuild;
   rebuild-first mỗi lần chạy binary. Suýt cắm cờ giả (như P1 phiên trước).
3. **push_owned idempotent** (lib.rs:651) → poison "double-push cùng local" = no-op không đỏ; bãi mìn thật
   là "alloc fresh local + Assign". Chọn đúng shape poison.
4. **emit_shim_call push_owns arg không-consumed** (lib.rs:1783) — nguồn ownership ngầm của iter_local;
   ai thiết kế ownership quanh shim-call phải tính tầng này.
5. **D (Sonnet 5) MVP:** map-trace tự làm, tự poison handle-alias→SIGABRT trước khi nộp, khai thật asymmetry
   + dead-code cho O đào tiếp. Vết: treo-lượt-chờ-gate (luật 17 hạ tầng) — O verify bằng máu mình bất kể.

→ [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[campaign_typed_collections]] [[campaign_aggregate_nullable]]
