# Memory index

## Project context
- **★ 2026-06-27(b) — ADR-0072 EXPECTED-TYPE PROPAGATION 🔒 SEALED + PUSHED. origin/main = `3d7618f`, gate `0·0·303·0`.** ([campaign_expected_type_propagation.md](campaign_expected_type_propagation.md)) Giết **mầm ung thư `c.sig.return_type` proxy toàn cục**, thay bằng `lower_expr(expr, expected: Option<&MirType>, …)` tường minh (G bác context-ẩn). 3 slice O-verify-máu-độc-lập + G-co-sign: S1 `c9a46e6` plumbing byte-identical · S2 `2c900fb` leaf-consumer+wire-4-nguồn+đập-3-redirect (mở `T?`-return scalar) · S3 `3d7618f` forwarding if/match/block + gỡ fallback §2.5 + nhổ-cancer + 309-negative-lock-untyped-ctor (SEAL). **Bài học: blocker "match-arm move-out" trong SỔ là CHẨN ĐOÁN SAI** (name-collision `get`=builtin + nullable-return) — match-move-out Outcome vốn đã chạy. Kiệt tác đóng: 157 untyped(fallback) vs annotated(tường minh) = MIR byte-identical. **Nợ cờ đỏ: heap-nullable-return drop-glue (304 xoá, cần WO poison FREE==1).** [[campaign_expected_type_propagation]] [[mentor_o_persona]] [[colleague_d_persona]]
- **★ 2026-06-27 — origin/main(cũ trước ADR-0072) = `5e54233` (synced, sạch). Gate `0·0·297·0`. HEAP-IN-AGGREGATE: 2 pháo đài từ 1 móng nứt.** ([campaign_truc_b_heap_in_aggregate.md](campaign_truc_b_heap_in_aggregate.md)) **(1) `e2b5c36` ADR-0067 AMEND — diệt live UB double-free** construction-into-field từ named-local (`let i=Inner{..}; let h=Holder{inner:i}` struct+enum → exit134, lọt vì 263/264 chỉ test inline-temp). Fix Option A lower-side: emit `Deinit(field_val)` sau field-Assign khi `is_nested_struct||is_nested_enum`, atomic cùng BB, tái dùng JIT recursive tombstone. O 4 teeth (poison→count==2; R-atomic structural MIR). **(2) `5e54233` Phase 2 — heap-STRUCT field move-out `let m=h.inner` MỞ** (lật nắp quan tài fixture 300). 3 site: borrowck allow-arm +`Struct` (UAM kế thừa: reuse→E2420, sibling OK, enum/multi-level→E2423); JIT `collect_heap_leaves(name, field_off,..)` đệ quy tombstone leaf ở absolute offset slot cha; **Site 3 (D bắt — recon O THỦNG "dest KHÔNG cần thêm gì")**: Lower `FieldAccess` gán Unknown cho Struct field → JIT không cấp slot → SIGSEGV; vá = propagate type thật `Struct(_)` (G bless type-system; vá luôn latent truncation 8B Copy-struct). O verify máu: revert-site3→139, FREE==1 poison→2. ADR-0070 AMEND ghi 3-site. **BÀI HỌC O: verify-don't-trust cắt cả recon của chính O.** **NỢ Phase 3+: enum-field move-out · multi-level `h.inner.x` · ⛔ ADR-0068 Box/true-recursive CẤM CỬA (HOÃN tới lệnh mới).** [[campaign_truc_b_heap_in_aggregate]] [[colleague_d_persona]] [[mentor_o_persona]]
- **★ 2026-06-26 — ĐÓNG PHIÊN (CHUYỂN MÁY). 🏁 ADR-0070 Partial-move + ADR-0071 Path `::`/`use`/enum-variant SEALED. origin/main pushed; ⚠️ infra Kỷ-Luật-Gate committed `9263501` [PENDING O-VERIFY] — D nộp, O CHƯA verify máu, G CHƯA ký (commit để chuyển máy; phiên sau VERIFY TRƯỚC khi seal).** ([campaign_path_separator_and_partial_move.md](campaign_path_separator_and_partial_move.md)) **ADR-0070** (`d3aa4ce`): borrow-checker per-Place move-state (`partial_moves` union-merge), ZST/Capability `let v=hw.vga` sound; 0B true-ZST; heap-field-move defer. 6 fixtures, O 5 teeth. **ADR-0071** (`4a7da96`+`c831274`, supersede ADR-0005): AST pha lê `::`=tĩnh `.`=động. `use`+`Item::Use` schema-first; `Color::Red`→EnumLiteral; **giết 3 cơ chế variant ngầm** + **E1018 khai tử** + bare→E1002 + §2.A Variable=catch-all + dọn dead expr_resolutions (rule#4, 21 caller). O 5 teeth (⚔ bóc tooth-vacuous P-pattern-guess INERT→relabel P-catch-all + sharpen 293; verify HARNESS không grep thô). **infra Kỷ-Luật-Gate committed `9263501` [PENDING O-VERIFY]** (gate.sh exit-1-giả→exit0⟺sạch + counting Mutex 6 file; sanity gate exit=0 nhưng CHƯA verify máu teeth A-real-red + B-no-flake-10×). **Kế (G chốt): BƯỚC 1 O VERIFY infra committed → ký → BƯỚC 2 read-side heap `let s=p.name`. ADR-0068 HOÃN.** [[campaign_path_separator_and_partial_move]] [[mentor_o_persona]] [[colleague_d_persona]]
- ★ 2026-06-25 — TRỤC B LÁT 2 NO-BOX (ADR-0067) ĐÓNG TRỌN (2a+2b+2b+). `c928b42`, gate 265. Enum-in-struct field, death-line#2 enum-sizing, O 4 teeth. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-23 — ADR-0067 2a Nested-Flat + 2b Enum-Payload. `2eae669`, gate 263. collect_heap_leaves đệ quy + emit_enum_drop_glue tag-switch. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-22(b) — ĐỊNH VỊ: TERNARY-FIRST, gỡ "AI-first". `8ab55b8`. Coherence VISION §8 = đại số Ł3 xuyên null/logic/capability. [[doc_highlights_and_ternary_seeds]] [[project_vision_os_capable]]
- ★ 2026-06-22 — TRỤC B LÁT 1 (1a+1b+1c heap-struct construct/move/drop). `24daf3f`, gate 254. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-21(b) — ADR-0066 Heap-in-Aggregate 1a: rào B8 thủng lần đầu, value-model i64 sống. `ab2cae8`. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-21 — ADR-0065 §12.8 `~+` nullable-present unify. `badf50d`, gate 250. Nợ: read-side `match h.f` scalar-nullable. [[campaign_aggregate_nullable]]
- ★ 2026-06-20→21 — Match Tryte/Long (ADR-0064 §A1) + Nested Nullable Aggregate Trục A (ADR-0065 §12.7). `04beac8`, gate 245. [[campaign_aggregate_nullable]]
- ★ 2026-06-20 — ADR-0065 Nullable Aggregate Enum?/Struct? LOCKED + Lát 1/2. `e71f396`→`f83a8f7`. Phân-quyền flow O-recon→WO→D→O-verify→G-ký. [[campaign_aggregate_nullable]]
- ★ 2026-06-20 — Latent-Type-Inference + Typecheck-Exhaustiveness E1026 + Variable-catch-all. `deb61c4`, gate 219. [[campaign_typecheck_exhaustiveness]] [[campaign_latent_type_inference]]
- ★ 2026-06-19 — 5 campaign: Heap-Nullable (ADR-0062) · CFG-Tail (ADR-0063) · Match-on-Literal (ADR-0064). `d85b794`, gate 211. [[campaign_heap_nullable]] [[campaign_cfg_tail_drop_ordering]]
- ★ 2026-06-17→18 — CLEANUP "Đại Hốt Xà Bần": LoweringInput + fat-return sret + heap-nullable `?+>` shell gate-LOWER. `667ea24`. [[lang_return_keyword_survives]]
- ★ 2026-06-17(c) — QUYẾT ĐỊNH: GIỮ keyword `return` (G định trảm, O phản biện `~->`/ADR-0020). [[lang_return_keyword_survives]]
- ★ 2026-06-17 — PHASE 14 Nullable `?+>` (ADR-0039) map/flatMap + E1046. `73532b4`, gate 170.
- ★ 2026-06-15→16 — TRAIT SYSTEM Tier 1 (ADR-0061) static dispatch. `594abd9`, gate 169. E1043/1044/1045, `implement`/`self`.
- [★★ 2026-06-12 — ADR-0060 Nested Aggregate `a.b.c` (P1 sub-8B khóa)](handoff_2026_06_12_adr0060_nested_aggregate.md) — `99ffedf`, value-model i64 nguyên.
- [★★ 2026-06-11 — ADR-0059 stack-borrow `&0` heap Vector/HashMap; `&+` YAGNI](handoff_2026_06_11_muiC_adr0059.md) — `8be0263`.
- [★★ CFG/Outcome 0055→0058 ĐÓNG](handoff_2026_06_11_adr0055_tail_expr.md) — block=tail-expr · heap value-merge · Outcome sret. `bf672b6`.
- [★★ HEAP OUTCOME + Fat-Pointer 32B Drop disc-dynamic (ADR-0054)](handoff_2026_06_10_op2_dong.md) — `7285d88`. Chuỗi Outcome `T~E` (ADR-0052) 2-slot+2-reg ABI.
- [★★ Trả Nợ: B1 MirType (0050) · B2 borrowck MIR NLL (0051)](handoff_2026_06_10_op1_dong.md) — `1e980d0`.
- [★★ BẬC D Fat-Pointer ABI + A1/A2/A3 ba bom câm](handoff_2026_06_09_bac_d_closed.md) — `58a8519`.
- [★ THỰC TẠI REWRITE — ĐỌC THỨ HAI](project_rewrite_reality_2026_06_04.md) — backend v0.2-v0.10 đã XÓA, 13 crate, bắt đầu lại.
- [docs/HIGHLIGHTS.md + backlog tam phân](doc_highlights_and_ternary_seeds.md) — điểm sáng I✅/II🎯/III🌱. #2 rounding + Outcome-disc-là-Trit đáng ADR.
- [Future — Comparable trait + ?-family](future_comparable_trait_and_monad_gap.md) — compare()->Trit (ADR-0038); monad-map (ADR-0039) đóng trọn.
- [Future — sized ternary int types](future_sized_ternary_ints.md) — defer.
- [Triết — tầm nhìn OS-capable](project_vision_os_capable.md) — 5 trụ cột (vision không đổi).
- [Triết — tổng quan dự án](project_triet_overview.md) — workspace (crate list một phần stale).
- [Parser → schema-AST migration](project_parser_schema_migration.md) — frontend reuse vẫn đúng.

## Personas (HAI persona, vai khác nhau — đừng lẫn)
- **[★ Mentor O — ACTIVE khi gọi "Mentor O"/"Mentor 0"](mentor_o_persona.md)** — Opus, **gác cổng / review owner**. Verify-don't-trust (tự chạy gate), teeth-phải-đỏ-khi-gỡ-guard, refuse-over-guess, không sửa hộ, admit khi báo động sai, tree đóng băng khi chấm.
- **[★ Đồng nghiệp D — Strict Colleague](colleague_d_persona.md)** — **implement-side, FILE GỐC DUY NHẤT**. 6 rule + Rule #7 refuse-over-guess + **4 LUẬT THÉP G** (① gate dòng đầu + khớp cây nộp · ①b pre-existing kèm stash-diff · ② fmt+clippy+test trước báo · ③ không xóa negative test · ④ bế tắc→hỏi O). Mẫu: báo đẹp hơn thực + đổ lỗi hạ tầng.

## User
- [Webapp dev, vision-driven, defers technical decisions](user_role_webapp_dev_visionary.md)
- [Tiếng Việt + tone non-engineer](user_communication_vietnamese.md) — reply tiếng Việt, analogy webapp/Java.

## Feedback
- **[⚔ Verify PRODUCER trước CONSUMER](feedback_verify_producer_before_consumer.md)** — flip field type mà producer còn đẻ repr cũ = CHƯA migrate. REJECT.
- **[★ CHU TRÌNH 7 bước + 4 vai](feedback_collaboration_loop.md)** — O=chốt verify KHÔNG code; nhận "tiến hành"→định teeth, chờ D nộp.
- **[⚔ poison-phải-đỏ](feedback_poison_must_be_red.md)** — fix cấu trúc: poison logic → test không đỏ → REJECT. Đừng tin tên test.
- **[★ Giao thức báo cáo G (5 mục)](feedback_g_report_protocol.md)** — O soạn gói 5 mục; đối chiếu thư-G-về trước khi ghi sổ.
- **[⚠ TEETH không bao giờ git checkout](feedback_teeth_never_git_checkout.md)** — `cp` snapshot /tmp TRƯỚC, khôi phục cp/Edit, KHÔNG git checkout/restore/stash.
- [Verify semantics before asserting](feedback_verify_semantics_before_asserting.md) — probe trước khi mã hóa ngữ nghĩa vào test.
- [Stability over speed](feedback_stability_over_speed.md) — quyết định kiến trúc có ADR.
- [Syntax — verbose + ~~dot paths~~](feedback_syntax_verbose_dot_paths.md) — `module` not `mod`. ⚠️ dot-path SUPERSEDED bởi ADR-0071 (`::` cho path).
- [Proactive tech-debt audit](feedback_proactive_audit.md) — suggest audit window gần version freeze.
- [No abbreviations — Java naming](feedback_no_abbreviations.md) — Vector not Vec, length not len, function not fn.
- [Explicit strictness](feedback_explicit_strictness.md) — panic-possible ops MUST be verbose methods with message.
- [Phương án A — defer cleanly](feedback_cham_ma_chac_pattern.md) — design cliff: ship diagnostic + ADR backlog, NOT skeleton.
- [Implementer's choice](feedback_implementer_choice.md) — author delegate implementation-internal; AI picks.
- [Quality over speed](feedback_quality_over_speed_v0_10.md) — AI=technical-quality owner, pre-commit self-audit.
- [Tiered Opus/DeepSeek workflow](feedback_tiered_opus_deepseek_workflow.md) — unsafe/ABI/IR = Opus-only.

## Reference
- [Source-of-truth docs](reference_spec.md) — SPEC/VISION/ROADMAP/TODO/ADR/CLAUDE.md đường dẫn.

## Triết language gotcha
- [Trit literals — suffix form](triet_trit_literals.md) — `1_trit/0_trit/-1_trit`, không `0t+` (cái đó là balanced-ternary Integer). SPEC §1.5.1.
