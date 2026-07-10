# Memory index

## Project context
- **★★★ 2026-07-10 — 🏁 SLICE C `HashMap<K,aggregate>` VALUE insert+drop ĐÓNG (ADR-0082 B-α, G ký, PUSHED).** origin/main `6d9e144`, gate `0·0·331·0`. F1–F4 (value-free-loop guard→`aggregate_needs_drop` · Enum-arm đệ quy defense-latent · marshal 2-đầu S3-gap · refuse tách key/kv). **🩸 O tự bắt lỗ G bỏ sót MÌN-3 ĐẦU-B** (8B value ôm handle→`use_var` LEAK CÂM, 331 fixture không thấy). **D "lệch lệnh" có tri thức** (get-family chết typecheck E1041/E1002/E1048, chỉ remove chạm JIT — O probe 5 `.tri` source verify). 4+1 poison→RED, md5 `62ab04…`. Mặt trận kế: value move-out aggregate HOẶC key-aggregate. Detail → [[campaign_typed_collections]]. [[feedback_poison_must_be_red]] [[colleague_d_persona]]
- **★★★ 2026-07-09 — 🏁 SLICE B `Vector<Enum>` push+drop ĐÓNG (ADR-0082 B-α, G ký, PUSHED).** origin/main `c22da0a`, gate `0·0·331·0`. push+drop SOUND (heap-payload variants), pop/move-out REFUSE (AM1). **🩸 O tự bắt 2 lỗ:** (1) pop fat-aggregate UB **pre-existing Slice A** (deferred-không-refuse=UB câm)→AM1 refuse bịt cả A+B; (2) push+drop UNSOUND **HAI bug che nhau** (BUG-1 `aggregate_needs_drop` thiếu Enum→leak · BUG-2 enum-local không tombstone→double-free; named-case=**"2 giả sound"**). **poison-must-be-red cứu mạng** (named-tooth đếm nhầm→10/10 xanh giả, poison S2 không đỏ mới đào ra). FIX-1+FIX-2. **⚠️ Bom hẹn giờ FIX-2 zero-@8** coupling frontend refuse enum-payload multi-heap-leaf. Detail → [[campaign_typed_collections]]. [[feedback_poison_must_be_red]] [[colleague_d_persona]]
- **★★★ 2026-07-08 — 🏁 SLICE A `Vector<UserStruct>` by-value element ĐÓNG (ADR-0082 B-α + §AMEND-1, G ký, PUSHED).** origin/main `1e49058`, gate `0·0·331·0`. **INV-B-α:** *một layout hai nhà byte-identical* (cell = StackSlot, 8B-granular), ĐẠP CHẾT B-β sub-8B, defer B-γ. Tái dùng `collect_heap_leaves` recursive drop-glue. §AMEND-1: M3 tombstone String-only→`Vector<User>` double-free 134→T7 vá; `vector_elem_size` rò HashMap→T8 refuse. **🎯 O tự bắt bug 331-fixture bỏ lọt = leak câm 8B-heap-struct** (`total_size==8`→push `use_var` đọc Variable→buffer 0→free 0)→C5 T9 vá `stack_load`. Detail → [[campaign_typed_collections]]. [[feedback_poison_must_be_red]] [[colleague_d_persona]]
- **★★★ 2026-07-04 — 🏁 READ-SIDE (CỤM A) get-borrow generic-V + P0 String-key SIGSEGV VÁ (ADR-0079 §AMEND-1, G ký, PUSHED).** origin/main `96f4241`, gate `0·0·331·0`. 6 overload `get` V∈{Vector,HashMap}→`(&0 V)?`; §AMEND-1 `get_ref` stride-conditional deref (thin→body_ptr, fat→cell) giữ `&0 V` bit-for-bit. **P0 báo động đỏ (pre-existing ADR-0080 latent):** read-path nhận `&0 HashMap` Reference-wrapped → key_stride default 8 → String-key marshal by-value → **SIGSEGV 139** (0 fixture test String-key read); VÁ unwrap `MirType::Reference` trước match HashMap. **❄️ A2 mutable FROZEN→Cụm D** (vacuous khi chưa deref-assign). **⚠️ D bẻ lệnh G** (giữ String-key overload không test)→G cảnh cáo thép "lần cuối dung túng ném-API-không-test". Detail → [[campaign_typed_collections]]. [[colleague_d_persona]]
- **★★★ 2026-07-03(b) — 🔑 key-typed `HashMap<String,V>` ĐÓNG (ADR-0080 + §AMEND-1, Author+O+G ký, PUSHED).** origin/main `381979e`. ADR-0080 BÁC amend ADR-0038 (`Ord ≠ Hash`) + BÁC `Hashable` trait. §AMEND-1: free-trong-thân-shim = counting mù → out-param ABI đẩy free ra JIT call-site registry-routed. Backend key_stride 24B + `__triet_string_hash` FNV-1a; source typecheck K∈{Int,String} + E1048. **⚔ O đính chính D:** value-loop state-check LÀ load-bearing đơn lẻ (single-poison→134); D under-analyze "2 lớp redundant" hạ chuẩn tooth. Detail → [[campaign_typed_collections]]. [[colleague_d_persona]]
- **★ 2026-06-30 — WO-NullableFieldMoveOut 🔒 ĐÓNG (G ký + PUSHED) — MẶT TRẬN OWNERSHIP FIELD-MOVE-OUT KHÉP, E2423 source-reachable CUỐI bay màu. origin/main = `4165c18`, gate `0·0·308·0`.** (ADR-0070 §AMEND Phase 4 + ADR-0076 §AMEND) `let s=b.s` với `String?`/`Vector?`/`HashMap?` → E2423→RUN sound. **💀 TIỀN ĐỀ "dynamic drop-flag" SỤP ĐỔ — O bác G lần 2 bằng bằng chứng thép: SLOT TỰ LÀ CỜ** (static tombstone, MIR join `Drop(base)` vô-điều-kiện, ptr@offset ∈ {ptr→free, 0/sentinel→no-op}, 0 `brif`; `collect_heap_leaves` jit:472 đã anticipate moved-out từ WO-0076). G rút lệnh "ĐÉO có drop-flag". **Recon-tách-tầng cũng lật chính O: ABI 2-reg ĐÃ XONG** (callee jit:849, caller jit:2413, fx113→42); guard `mir_lower.rs:2246` thuần phòng thủ (MIR không có Tuple); ABI còn lại = native-layout đeo mặt nạ → defer. **3 site:** ① borrowck `checker.rs:775` `Nullable(inner) if inner.is_any_heap()` tường minh · ② JIT `mir_lower.rs:1980` zero ptr@field_off + sync `is_string_repr()` (String? 24B) · ③ Lower `lib.rs:2997` propagate dest-type. **O verify 7 teeth ĐỘC LẬP (cp-snapshot, restore md5 khớp):** #1 count FREE==2 · #1b real SIGABRT 134 (đk G ký) · #2 is_copy→true LEAK · #4 ⚔CFG-divergent taken=134/not-taken=0 · #5 Site-3 LEAK · #S1 borrowck→E2423 · #6 E2420. **⚠️ O bắt D bốc phét: claim "Site-3→SIGSEGV" SAI — thực=LEAK câm** (xem [[feedback_failure_mode_precision]]). KHÓA defer: Index/Deref/Payload→E2423, E2424 reassign. [[campaign_path_separator_and_partial_move]] [[campaign_truc_b_heap_in_aggregate]] [[mentor_o_persona]] [[colleague_d_persona]]
- ★ 2026-06-29(e) — BÀN THIẾT KẾ MODULE SYSTEM (Facade). O dùng ADR-0005 bác CẢ Giang (lo 1-1 Java — §17/§96/§155 đã tách logical/physical) LẪN G (auto-discovery = A1/A3/A5 ĐÃ reject, phá hermetic) — **G nhận sai #2, co-sign**. NHẬN Facade `public use` re-export (lỗ defer §76, parser `item.rs:78`) + 🌱 capability-aware seed. Backlog: amend ADR-0005 §76, chờ std/PackageManager. **Bài học: verify-don't-trust áp cả Mentor G.** [[mentor_o_persona]]
- ★ 2026-06-29(d) — WO-0076 heap-`T?` aggregate field/payload 🔒 ĐÓNG — KỶ NGUYÊN NULLABLE KHÉP. `994afc8`, gate 306. Giao điểm B8 cuối; sentinel-no-op conditional-drop 0 `brif`; O vồ double-free CASE B match-present-bind→D đóng STATIC tag-niche-tombstone. O 3 teeth. **Bài học: gate-lift mở bề mặt → mọi thứ mới-compile phải sound-HOẶC-refused.** [[campaign_truc_b_heap_in_aggregate]] [[campaign_aggregate_nullable]]
- ★ 2026-06-29(c) — WO-0075 multi-level `let x=h.inner.x` 🔒 ĐÓNG (ĐẠI PHẪU borrowck core projection-path, ADR-0070 §AMEND P3). `bd614f3`, gate 303. `partial_moves` field-name→`Vec<String>` path; C1 `3826924` vá **fixpoint-hole CÓ SẴN** (UAM lọt back-edge loop, latent unsound) commit tách trước C2; E2424 mới khóa sub-path reassign. O 9 teeth. **Bài học: recon mổ tim lôi khối u ngủ đông — vá cùng ca, commit tách.** [[campaign_path_separator_and_partial_move]] [[campaign_capability_luk3]]
- ★ 2026-06-29(b) — WO-0074 enum-field move-out `let e=h.msg` 🔒 ĐÓNG (Phase 3 Nợ A). `e0b1ed7`, gate 303. 3 site đối xứng; Site-3 zero payload-ptr@field_off+8. O 5 teeth (T5 ⚔SIGSEGV in-suite signal 11). **Bài học: gate đếm chỉ corpus integration; counting/subprocess là binary riêng.** [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-29 — WO-0073 heap-nullable-RETURN drop-glue 🔒 ĐÓNG (cờ đỏ ADR-0072 §6). `3738eb5`, gate 303. expr-body=escape-by-omission (M4 INERT) vs named-local=flush emit Drop (M4 load-bearing). O 7-cell counting. **Bài học: verify cắt cả WO của chính O.** [[campaign_expected_type_propagation]]
- ★ 2026-06-27(b) — ADR-0072 EXPECTED-TYPE PROPAGATION 🔒 SEALED. `3d7618f`, gate 303. Giết `c.sig.return_type` proxy toàn cục → `lower_expr(expr, expected, …)` tường minh. 3 slice; 157 untyped vs annotated = MIR byte-identical. **Blocker "match-arm move-out" trong sổ = chẩn đoán SAI (name-collision).** [[campaign_expected_type_propagation]]
- ★ 2026-06-27 — HEAP-IN-AGGREGATE 2 pháo đài. `5e54233`, gate 297. `e2b5c36` ADR-0067 AMEND diệt live-UB double-free construct-into-field từ named-local; Phase 2 heap-STRUCT field move-out `let m=h.inner` (Site-3 D bắt: Unknown→SIGSEGV, vá propagate Struct type). O 4 teeth. ⛔ ADR-0068 Box/recursive CẤM CỬA. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-26 — ADR-0070 Partial-move (ZST/Cap per-Place) + ADR-0071 Path `::`/`use`/enum-variant SEALED. `d3aa4ce`/`4a7da96`+`c831274` (supersede ADR-0005). AST `::`=tĩnh `.`=động; giết 3 cơ chế variant ngầm + E1018 khai tử. O 5+5 teeth. [[campaign_path_separator_and_partial_move]]
- ★ 2026-06-25 — TRỤC B LÁT 2 NO-BOX (ADR-0067) ĐÓNG (2a+2b+2b+). `c928b42`, gate 265. Enum-in-struct field, death-line#2 enum-sizing. [[campaign_truc_b_heap_in_aggregate]]
- ★ 2026-06-23 — ADR-0067 2a Nested-Flat + 2b Enum-Payload. `2eae669`, gate 263. [[campaign_truc_b_heap_in_aggregate]]
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
- [Future — ternary placement syntax +T/T/-T](idea_ternary_placement_syntax.md) — Giang: heap/stack/static-pool. O bác 4 lỗ (chí mạng: placement không-cực → loãng coherence VISION §8). PARKED tới khi mở lại ADR-0068 Box.
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
- **[⚔ Failure-mode phải CHÍNH XÁC](feedback_failure_mode_precision.md)** — D bốc phét SIGSEGV khi thực = LEAK. 4 signal khác nhau (134 double-free / 139 bad-deref / FREE==0 leak / FREE==2). G ĐẠI KỴ Hollywood. aggregate field no-slot→SIGSEGV; heap-scalar/`T?` no-slot→leak câm.
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
