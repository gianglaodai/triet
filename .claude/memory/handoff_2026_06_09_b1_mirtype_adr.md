---
name: handoff-2026-06-09-b1-mirtype-adr
description: "B1a MirType — S1 ĐÓNG (HEAD 76b53cb), G ký S2. ADR-0050 + blueprint commit. Kế: S2 flip field + xoá simple_is_copy."
metadata: 
  node_type: memory
  type: project
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

# B1a — Rombac MirType (Crusade #3, MÓNG cho B2)

**HEAD `9af6afd`** — ✅ **B1a HOÀN THÀNH (Crusade #3 đóng).** Cây:
- `8bec10b` docs(adr): ADR-0050 + blueprint phase7
- `76b53cb` S1 — MirType enum song song (field giữ String)
- `fe80b8c` S2 — flip field String→MirType + producer lower_type + xóa simple_is_copy
- `ec6d32f` S3 — thanh trừng String: TypeKind symbol-table + triệt free helpers (+215/−373)
- `9af6afd` S4 — trảm parse + From-shim, acid test toàn vẹn MirType (+143/−229)

Gate **0·0·99·203**, workspace 0-fail, `MirType::parse`=0 toàn workspace. Bậc D + A1/A2/A3 đã đóng — xem [[handoff-2026-06-09-bac-d-closed]]. **B1a ĐÓNG SỔ (G ký), TODO mark `6d6eeaa`.**

## ▶ B2 ĐANG MỞ — Sáp nhập 2 tầng borrowck (Crusade #2). ADR-0051 ký O+G 2026-06-09.
**Bom:** driver fatal-stop typecheck (main.rs:58) → chương trình typecheck bắt E2440 KHÔNG tới MIR phase 4 → MIR E2440 dead-code-bị-che ("không teeth-isolate"). 2 cảnh sát chồng quyền (typecheck AST live-range + MIR NLL CFG), trùng E2420/E2440.
**B2.0 spike (O TỰ verify, không tin D):** stub typecheck E2440 → fixture 99/99, 6 fixture E2440 bắt từ MIR. MIR cover E2440 ✓. Fixture match MÃ không TÊN; harness collect-all-phases. E2420 = move-state machine 1-emit (check.rs:178 check_used), KHÔNG 18 site rời.
**G CHỐT phạm vi:** B2.1 gỡ E2420+E2440 → B2.2+ dời E2400/E2410 sang MIR (cam kết trọn) → E25XX NGOÀI B2. ADR-0051 mới. **CẤM XÓA MÙ** — quy trình §5: gom nhóm logic → kiểm fixture coverage → viết fixture thiếu TRƯỚC → tắt site → MIR cắn đúng → mới xóa → teeth-isolate sau.
**B2.1 bề mặt:** xóa `typecheck/borrow_check.rs` (502 dòng, E2440, 1 consumer check.rs:435) + move-state machine E2420 (check.rs MoveState/move_states/mark_moved/check_used) + xóa/chuyển unit test E2420/E2440. `.tri` fixture giữ. Nợ canh: conservative=true (B3)/is_propagated (A1) không tái sinh.
**SỰ CỐ MẤT CODE (2026-06-09):** Khi commit ADR-0051 (HEAD `969cc73`), thao tác git (checkout/restore crates/ để commit ADR sạch) **discard toàn bộ B2.1 code dở của D** — borrow_check.rs trở lại, check.rs về emit thật, không trong stash. **KHÔNG mất việc đã ký** (B2.1 vốn bị O reject vì skeleton dead-code). 2 fixture 104/105 sống (untracked). Bài học: commit doc-lẻ giữa lúc có code uncommitted = nguy hiểm; G: "lửa thiêu rác". Baseline về `969cc73` = pre-B2.1 code + ADR-0051, gate **0·0·101·203**.

**O ADMIT:** claim "2 caller đơn giản" SAI. Tự đo lại: **42 ref branch-join machinery** (move_states/snapshot_moves/join_moves đan trong check_if+check_match). D đúng — constraint kiến trúc thật. → G chọn Phương án 3 (tách).

## G CHỐT PHƯƠNG ÁN 3 — tách B2.1 (blast-radius isolation):
- **B2.1a (Gỡ Mạng Nhện Branch-Join):** 1 commit RIÊNG bóc `move_states`/`snapshot_moves`/`join_moves` khỏi check_if/check_match. Nhổ sạch rễ move-state AST. Typecheck "mù nửa E2420" — MIR chụp (acid test chứng minh). **CẤM `#[allow(dead_code)]`. Cắt là vứt.** Gate xanh.
- **B2.1b (Nổ mìn):** sau B2.1a xanh → chém nốt E2420 emit còn vương + san phẳng `borrow_check.rs` (E2440).
- Cấm tuyệt đối skeleton chết (Phương án 2).

**O REFINEMENT (giữ ý G):** B2.1a/b cắt theo MÃ (E2420 vs E2440), KHÔNG theo "branch-join vs emit" — vì check_used (emitter) ĐỌC move_states → coupled, gỡ field giữ emit = không compile. B2.1a = gỡ TRỌN subsystem E2420 (1 đơn vị); B2.1b = nổ borrow_check.rs (E2440 module độc lập).

## ✅ O KÝ B2.1a (2026-06-09) — gỡ trọn subsystem E2420 khỏi typecheck.
Baseline `969cc73`. D gỡ: MoveState/move_states/snapshot_moves/join_moves (42-ref mạng nhện branch-join trong check_if+check_match) + mark_moved/check_used + 6 call-site + 2 caller + 7 e2420_fires_* test. CẤM #[allow(dead_code)] — cắt là vứt ✓. **2 vòng O chặn:** V1 (claim dối "test 0" khi 3-7 fail, MoveState no-op skeleton — REJECT) → SỰ CỐ mất code (git restore lúc commit ADR) → làm lại sạch → V2 (orphan assert_use_after_move + lấn scope xóa e2440 test). Sửa. **Teeth-isolate E2420 ĐỎ:** poison MIR retain-bỏ-UseAfterMove → typecheck mù + MIR mù → fixture E2420 SIGABRT (unsound JIT). THẮNG LỢI B2: trước B2.1a poison MIR không đỏ (typecheck che); giờ đỏ thật. Gate 0·0·101·203.
Commit `1e6c14e` (net −227). borrow_e2440_nll.rs giữ (E2440 emitter còn sống tới B2.1b).

## ▶ B2.1b ĐANG MỞ — nổ E2440 typecheck. Baseline `1e6c14e`, gate 0·0·101·203.
**Bề mặt (O khảo sát):** xóa `borrow_check.rs` (502 dòng, **1 emitter** check_resolved→analyze_function:486, construct:462) + `check.rs:359 analyze_function` call + `lib.rs:33 mod borrow_check` + `tests/borrow_e2440_nll.rs` + variant `BorrowExclusivityViolation` (error.rs:966, sau khi 0 construct) + `tests/diagnostics_format.rs:129` (construct variant — phải sửa/xóa). MIR `NllExclusivityViolation` 8 site CÒN SỐNG ✓. 5 fixture `.tri // ERROR: E2440` giữ (MIR phát).
**Teeth-isolate E2440 (O áp):** sau xóa → poison MIR NllExclusivity (retain bỏ) → 5 fixture E2440 đỏ thật từ MIR (typecheck mù). Mẫu như E2420 SIGABRT/assert.
**CẤM:** #[allow(dead_code)], no-op skeleton, claim test-xanh chưa chạy workspace, git restore/checkout file dở (dùng cp /tmp). Dán RAW gate + clippy.
## ✅ O KÝ B2.1b (2026-06-10) — nổ E2440 typecheck. Baseline 1e6c14e.
D xóa: `borrow_check.rs` (502 dòng, 1 emitter) + check.rs:359 analyze_function call + lib.rs:33 mod + tests/borrow_e2440_nll.rs + variant BorrowExclusivityViolation (error.rs) + diagnostics_format construct. MIR NllExclusivityViolation 8 site CÒN SỐNG ✓. **Teeth-isolate E2440 ĐỎ:** poison MIR retain-bỏ-NllExclusivity → 5 fixture E2440 FAIL; bằng chứng mạnh nhất `79_return_borrow_caller_freeze` "pipeline succeeded" (chỉ MIR bắt, typecheck mù hoàn toàn). Gate 0·0·101·203. Commit msg: `feat(track-c): B2.1b — nổ borrow_check.rs E2440, MIR NLL độc quyền exclusivity`.
**THẮNG LỢI B2 trọn:** cả E2420 (SIGABRT) + E2440 (5-fixture-fail) teeth-isolate được — mục tiêu cốt lõi ADR-0051 (cảnh sát MIR hết bị bịt mắt). B2.1 (gỡ trùng) XONG.

## ✅ CRUSADE TRẢ NỢ XONG (HEAD 0156699, 2026-06-10). ▶ C1 MỞ.
A sạch bóng (A1 be37875 teeth-verify O, A2 INV-4 + A3 E1026 d8e1ba9, F6 đóng 2 lớp) · B1 đóng (MirType) · B2 đóng (11d11cf, MIR NLL độc quyền, −1034 dòng) · B3 defer (0156699 YAGNI, blueprint ghi ĐIỀU KIỆN TIÊN QUYẾT G: negative-alias-test TRƯỚC khi nới conservative).

## ✅ C1 ĐÓNG (HEAD 0fb8de6, 2026-06-10) — KHÉP NỢ B1a "móng Struct/Enum no-consumer".
`MirType::Enum` có **active consumer đầu tiên**: caller `stack_addr` by-pointer + callee tái dựng enum_slot từ pointer (load disc@0 + payload@8, mẫu struct/String Bậc D). Fixture 27 rename `_error`→`27_enum_payload_param.tri` positive EXPECT 52 (git nhận rename, history giữ). 27→52·32→2·25→1·26→52. Móng B1a (tách Struct/Enum) lần đầu lái codegen thật — răng cưa cắn. **Teeth O re-verify trên code production cuối** (sau D fix clippy while→for): disc-offset 0→8 SIGILL + payload-copy bỏ FAILED. Gate 0·0·101·203. `non-enum` error (jit:314) GIỮ = guard hợp lệ cho lỗi thật. **D claim lệch 2× (che rename "27→52" + lờ clippy 207→fix 203) — minor, lõi đúng.** Blueprint phase9 committed.
## ✅ Native Layout + Packed Outcome PHONG ẤN Nhóm E (commit 47a4c46, G defer 2026-06-10).
Spike O khảo sát (4 câu G): Q1 compute offset align-aware THẬT nhưng lower:347 hardcode (8,8) · Q2 JIT FieldAccess ĐÃ dùng field.offset (G lo lắng = đã sạch) · Q3 điểm gãy = stack_load(I64) CỨNG (value-model "single i64" jit:186) → field<8B tràn · Q4 blast 14load+21store+value-model. 2 tiền đề thiếu: MirType-byte-size + value-model nâng cấp. 0 fixture Trit/Tryte-in-struct → YAGNI. 3 điều kiện mở (phase10 + TODO).

## ▶ C2 ĐANG MỞ — Pattern::Wildcard arm-level trong enum match (G ký trọn 2026-06-10, KHÔNG cần ADR).
**Probe O:** `match c { Red=>1, _=>0 }` → `unsupported match pattern (expected enum variant): Wildcard` (lower:2545) = CHẶN. Sub-pattern `SomeInt(_)` đã handle (2487); arm-level `_` chặn. Móng sẵn: enum match có SwitchInt + `default_bb: trap_bb` → C2 map wildcard arm vào default thay trap.
**Kế 3 lát (G ký):** C2.1 lower wildcard-arm→default_bb (reuse guard wildcard-last + ≤1 từ nullable lower:2204) · C2.2 typecheck wildcard suppress E1026 exhaustive · C2.3 fixture + teeth.
**RỦI RO CỐT TỬ (G nhấn): A3 regression** — wildcard suppress E1026 KHÔNG được làm non-wildcard-non-exhaustive lọt. **Fixture 103 (A3) bảo vệ bằng mọi giá.**

## ✅ C2 ĐÓNG (HEAD a25fbff, 2026-06-10). Wildcard arm enum match.
C2.1 lower wildcard→default_bb Goto (guard wildcard-last+≤1 reuse nullable) · C2.2 suppress E1026 ĐÃ-CÓ-SẴN từ A3 (exprs.rs:1578 short-circuit) · C2.3 fixture 106 + **nới INV 4i-6 `Trap→Trap|Goto`** (D phát hiện+báo thẳng, đúng phạm vi: Unreachable VẪN reject → A2 không tái sinh). Teeth: A3-suppress-poison→103 mất E1026 (bảo vệ thật) + wildcard→trap-poison→SIGILL. Gate 0·0·102·203. **D tiến bộ: báo thẳng việc đụng verifier, không giấu.**

## ✅ C6 ĐÓNG (HEAD 992311e, 2026-06-10) — TÀN DƯ BẬC D SẠCH.
concat→sret: shim `(dest_slot,a_ptr,a_len,b_ptr,b_len)` writeback `*mut FatStr` (mẫu (b) append, KHÔNG (a) Rust-auto-sret). **O đính chính C6.0:** probe lật claim D "auto-sret proven" — `ArgumentPurpose::StructReturn`=0 hit, "sret" codebase = manual by-pointer (append *mut FatStr); D wire (b) đúng. Caller bỏ reconstruct len (coupling caller-biết-concat-len bỏ). Teeth shim len→0→fixture 35 FAILED. Shim registration fn_4_1→fn_5_0 (main+tests). Gate 0·0·102·203. Mọi return-fat String nhất quán sret callee-fill.

## ✅ C5 DEFER (502713a) + ▶ OUTCOME PRODUCER MỞ (ADR-0052, HEAD 16e0d56, 2026-06-10).
C5 spike: premise nhẹ (Cranelift multi-return native, KHÔNG vỡ value-model) nhưng 0 producer → defer Nhóm E. **Outcome producer = use-case mở C5 (khép vòng).**
**ADR-0052 Outcome ABI (nối ADR-0020 design-locked):** 2-slot {disc:Trit, payload} MIR + Cranelift multi-return. Bất biến G: payload CHỈ scalar Bậc A (heap defer B/C) · un-defer C5 CHỈ BinaryOutcome/TernaryOutcome (tuple generic vẫn Err) · Cranelift native (value-model không đổi).
Hiện trạng: Frontend ✅ (lexer ~+/~-/~0 + AST OutcomeConstructor) · Typecheck 🟡 (check_outcome_constructor_context có móng) · Lower 🔴 degenerate (`~+ e`=identity lib.rs:1108, `~-`=unsupported 1124) · MIR ✅ (ReturnShape::BinaryOutcome + OutcomeDiscriminant/Unwrap ops định nghĩa, 0 producer) · JIT 🔴 (multi-value chặn jit:1070).
**4 lát OP:** OP.1 typecheck E1024/E1025+return-match → OP.2 lower 2-slot **check-mode fixture** (MIR verify, cô lập producer khỏi JIT) → OP.3 JIT un-defer C5-cho-Outcome (gỡ guard 1070 CHỈ Outcome) → OP.4 match/unwrap. Teeth: disc Positive→Zero (E1025), gỡ-guard-generic-tuple vẫn Err, caller inst_results[1] bỏ payload sai, OutcomeDiscriminant nhầm slot.
## ✅ OP.1 ĐÓNG (HEAD 1e980d0, 2026-06-10 — ĐIỂM DỪNG PHIÊN). Gate 0·0·105·203.
OP.1 typecheck Outcome: E1025 (`~0` on T~E) + E1026 outcome-non-exhaustive (variant RIÊNG exprs.rs:313, khác enum 327, chung mã) đã-có-sẵn; return-type-match payload D bổ sung (guard-style exprs.rs:404-411, `~+`:value_type `~-`:error_type). Fixtures 107/108/109. Teeth payload-match poison→109 FAILED. A3 fixture 103 GIỮ (Outcome E1026-variant-riêng, không vỡ enum). wildcard-single-variant O nâng soundness→D fix `_=>{}`→`OutcomeArm::Zero=>None` tường minh.

## ▶ KẾ TIẾP PHIÊN SAU: OP.2 — Lower Outcome → 2-slot (CORE WORK, G chờ "hình hài ReturnShape 2-slot").
Lower hiện DEGENERATE (lib.rs:1108 `~+ e`=identity, 1124 `~-`=unsupported). OP.2 phải: `~+ v`/`~- e` → alloc 2-slot {disc:i64 Trit, payload:i64} · disc=Positive(1)/Negative(-1) const · `ReturnShape::BinaryOutcome` (arity 2) cho fn `-> T~E` · `Return{values:[disc,payload]}`. **Fixtures CHECK-MODE** (parse→typecheck→lower→borrowck→MIR verify, KHÔNG JIT — cô lập producer khỏi backend, mẫu G ký). MIR ops OutcomeDiscriminant/Unwrap (mir:254-280) wire ở OP.4. ADR-0052 §3-4. Payload CHỈ scalar Bậc A (heap defer B/C). Teeth O dự: disc Positive→Zero (E1025/verifier), ReturnShape arity sai, Return values.len≠2.
Sau OP.2: OP.3 JIT un-defer C5-cho-Outcome (gỡ guard jit:1070 CHỈ BinaryOutcome) · OP.4 match/unwrap.

## TRẠNG THÁI TOÀN CỤC CUỐI PHIÊN (2026-06-10, HEAD 1e980d0):
Crusade Trả Nợ XONG: **A sạch** (A1/A2/A3 teeth) · **B1/B2 đóng** (MirType ADR-0050 · borrowck-merge ADR-0051 −1034 dòng) · **B3/Native/Packed/C5 phong ấn Nhóm E** (YAGNI, điều kiện mở ghi blueprint) · **C1/C2/C6 done**. ĐANG: **Outcome Producer** (ADR-0052, OP.1 ✅, OP.2-4 còn). 18 commit phiên. 3 ADR mới (0050/0051/0052) + 5 blueprint (phase7-12). Gate 0·0·105·203.

--- (chi tiết C1 gap bên dưới, tham khảo) ---
**▶ C1 — Enum payload qua function param (G lệnh, active consumer móng B1a):**
Gap (O khảo sát): caller jit:1162 enum-arg→stack_load chỉ discriminant VỨT payload · callee param-entry 676-690 enum không tạo enum_slots · payload-access jit:310/382 → "Payload access on non-enum local". Fix = mẫu Fat-Pointer String param Bậc D (jit:1148-1165): caller stack_addr by-pointer, callee tái dựng enum_slot từ pointer; by-pointer decision `match MirType::Enum(_)`. Fixture 27 ghim bug như `// ERROR` nhưng chương trình HỢP LỆ → C1 biến positive `// EXPECT: 52`, đập string-match jit:314. D viết `spec/plans/phase9-c1-enum-payload.md`. Khép nợ B1a "móng Struct/Enum no-consumer".

--- (lịch sử B2.2 kiểm toán bên dưới, giữ tham khảo) ---
## ⚠ KIỂM TOÁN B2.2 (O, G mệnh lệnh — ĐẢO GIẢ ĐỊNH). HEAD 58dfa4e.
G ra lệnh "dời E2410 mutability + E2400 lifetime sang MIR". O rà 100% emit site → **premise sai**:
- **E2410 `CannotMutateFrozenOwner`**: **0 construct trong logic = DEAD skeleton** (ADR-0025 §7.1 chưa wire). 0 fixture. KHÔNG có gì để dời.
- **E2430 `NamespaceInferenceFailed`**: **0 emit, 0 fixture = DEAD skeleton.** (G đoán name-resolution — thực ra chưa tồn tại logic.)
- **E2400 `BorrowLifetimeInferenceFailed`**: **SỐNG** (emit check.rs:468, 2 fixture) — return-borrow elision ambiguity (ADR-0046, tĩnh-signature-level, KHÔNG phải NLL live-range).
- **Mutability ĐANG chạy thật = E1016 `AssignToImmutable`** (typecheck::E1016, type-level `let x` vs `let mutable x`) — type-system, KHÔNG phải borrow/dataflow. Ở lại typecheck đúng chỗ.
→ **B2.2 thực chất chỉ còn E2400** (dời return-borrow lifetime). E2410/E2430 = dead variant (quyết xóa-dead hay giữ). B2.1 đã dời TOÀN BỘ borrowck-enforcement-đang-chạy (E2420+E2440). Gói báo G chờ quyết: B2.2 = chỉ E2400? + dọn 2 dead variant? E25XX NGOÀI B2.

## S1 ĐÓNG (verify O): MirType 14 variant trong triet-mir, Display round-trip, parse-shim (MUST KILL at S4), is_copy(Option<&Body>) MỘT logic + invariant-B8 mang theo. Field `ty:String` GIỮ — +0 hành vi. Teeth 2 cú đỏ (is_copy heap; ordering is_vec sau khi vá test fake-teeth). **Bài học Vòng 4 → [[feedback-poison-must-be-red]] (G luật thép).**

## S2 ĐÓNG — G KÝ COMMIT (2026-06-09). HEAD sẽ là commit kế (CHƯA commit lúc viết — chờ author/D gõ).
**S2 = flip field String→MirType + producer `lower_type` (map TypeExpr→MirType trực tiếp) + xóa `simple_is_copy`.** Gate **0·0·99·205**. 3 vòng O chặn: V1 re-parse reference (G inv #1), V2 claim-clippy-sai-nguồn (đổ thừa generated), **V3 producer ngụy trang** (`type_name→String` rồi `parse` ngược = fake producer, suýt vỡ bất biến G ③ → [[feedback-verify-producer-before-consumer]]). Teeth A (producer String→Unknown: 10/99 đỏ) + B (is_copy heap→Copy: mir unit + 6/99 đỏ). Commit msg G duyệt: `feat(track-c): B1a S2 — flip field String→MirType + producer lower_type + xóa simple_is_copy`.

2 INVARIANT G phải canh tiếp:
1. **Display-bridge MỘT CHIỀU:** CẤM `match ty.to_string().as_str()`. Matching PHẢI qua `MirType`.
2. **Poison-phải-đỏ LUẬT** ([[feedback-poison-must-be-red]]) + **producer-trước-consumer** ([[feedback-verify-producer-before-consumer]], G chuẩn thuận V3).

## S3 ĐANG CHẠY — THANH TRỪNG STRING (mệnh lệnh G). Trên working tree (chưa commit, base fe80b8c).
**Đã sửa (O verify):** S3.1 return_type→MirType ✓ · lower_type_simple refuse-over-guess (bỏ default-Struct) ✓ · jit destructure Struct/Enum ✓ · parse production=0 (confined test) · to_string dispatch=0.
**Còn nợ trước khi đóng S3 (G chỉ thị 2026-06-09):**
1. **Triệt 5 free helper `&str`** (mir:2924-2961 is_nullable_type/nullable_payload/is_vec_type/is_hashmap_type/is_copy&str) + 2 caller cuối. "Xóa 37 helper không phải nói đùa" (G). allow items_after_test_module sẽ tan.
2. **PHƯƠNG ÁN (b) — G CHỐT:** xóa 2 HashSet struct_names/enum_names rời → tạo **`HashMap<String, TypeKind>`** (`enum TypeKind{Struct,Enum}`) = ItemSymbolTable sơ khai. Pass-1 scan Item nhét map; Pass-2 lower_type tham chiếu map (không chờ struct_layouts). Cắt `too_many_arguments`. Lý do giữ map (không layout-table): lower_type chạy DURING layout-construction (lower:354), tables chưa hoàn chỉnh.

**Teeth 1a+1b KHÔNG đỏ → tách Struct/Enum là MÓNG no-consumer (G chấp nhận):** correct-by-construction cho B2/C1; lưới khép khi C1 match enum-payload. KHÔNG nhét test giả (G khen trung thực).

## G ỦY QUYỀN O (2026-06-09): O TỰ ĐO gate+teeth → TỰ DUYỆT COMMIT S3 (không xin chữ ký lẻ) → báo cáo chốt sổ G. D tự đổi nhãn allow = vượt quyền/lấp liếm (G: "giấu lỗi nặng hơn gây lỗi").

## ✅ O KÝ DUYỆT S3 (2026-06-09) — đạt chuẩn sau 5 vòng O chặn:
- V1 code-không-compile (35 E0308, claim "build 0" chỉ lib) · V2 lower_type_simple default-Struct (fake producer #2, vi phạm G②) · V3 G S3.2 delta→phương án (b) `HashMap<String,TypeKind>` ItemSymbolTable · V4 5 helper chưa xóa + 3 clippy trần (D xóa allow→warning trần, claim "too_many tan" sai vì 9>7) · V5 TypeKind chèn giữa doc-comment lower_program.
- **Kết quả verify O:** workspace pass · 5 free helper=0 · TypeKind map scan Item (thay 2 HashSet) · return_type→MirType · `=="String"`/`starts_with('&')`/is_enum_type/is_fat_type=0 · parse production=0 (confined test) · to_string dispatch=0 · clippy 203 (allow justify). **Teeth 1 (producer String→Unknown: fixture đỏ) + Teeth 2 (is_copy heap→Copy: mir unit đỏ).** Tách Struct/Enum = móng no-consumer (G chấp nhận, khép khi C1).
- Commit msg: `feat(track-c): B1a S3 — thanh trừng String: TypeKind symbol-table + triệt free helpers`. Chờ author/D gõ commit (O không git commit không lệnh author).
- Nợ-S4: `MirType::parse` chỉ còn test → **S4 nhổ parse** (build nổ đỏ chỗ quên). lower_function 9-param → gom LoweringInput struct (defer, allow justify).

## S4 — LỄ TRẢM QUYẾT parse (G phát lệnh 2026-06-09, S3 KHÉP). Acid test toàn vẹn B1a.
**Bề mặt (O khảo sát ec6d32f):** xóa `pub fn parse` (mir:559) + **3 From-shim gọi parse** (`From<&str>`/`From<String>`/`From<&String>` mir:732-746 — đây là dây rốn cho `MirBuilder::new("Unit")`). `From<&MirType>` (748) GIỮ (clone). Xóa shim → **35 `MirBuilder::new(...,"str")` + ~25 alloc_local_ty/LocalDecl &str** gãy → D đổi enum trực tiếp (`MirType::Unit` v.v.). + ~25 test call `MirType::parse("..")` (mir) + lower:2930 (test). lower:2930 đã verify = test (0 production dùng parse, khớp S3).
**Acid test G:** xóa parse → `cargo test --workspace` build; production nào lén dùng parse → nổ đỏ. Teeth O: sau xóa, thêm `let x: MirType = "String".into()` vào PRODUCTION → phải KHÔNG COMPILE (From<&str> chết = dây rốn cắt).
**Done:** `rg MirType::parse` workspace=0 · `rg 'From<&str> for MirType'`=0 · gate xanh · test dùng enum variant trực tiếp (G: "vạch trần mọi ngõ ngách"). → B1a HOÀN THÀNH, báo cáo chốt G.

## Đã xong phiên này
- **ADR-0050** (`docs/decisions/0050-mir-type-enum.md`) — **ký O+G 2026-06-09**. Quyết: enum `MirType` viết-tay-trong-triet-mir (KHÔNG schema generated Type — MIR là IR backend, không AST). 3 bất biến G: ① MirType-trong-mir ② **TÁCH `Struct(String)`/`Enum(String)`** (cấm gộp UserType — bảo toàn type-safety, bắt lỗi trỏ-sai-bảng) ③ `parse(&str)` shim transitional phải **chết ở commit cuối** (gắn `// TECH-DEBT(B1a): MUST KILL THIS SHIM`).
- **CORRECTION ADR §3.1.1/§3.1.2 (O, post-probe)** — sửa lỗi của chính O trong bản ký đầu:
  - **Vector/HashMap TRẦN, KHÔNG payload** (`Vector(Box)` → `Vector`). Đo: 0 consumer trích element type, 0 diagnostic in `"Vector<…>"` → payload là dead field (Rule #4). **Hệ quả lớn: R1/R2 generic-parsing TAN — `lower_type` chỉ-đọc-arena, KHÔNG đụng typecheck `type_map`.**
  - **`Trilean` TRẦN** (bỏ `refined`). Đo: 0 backend đọc `.refined` (refinement là gate frontend, kiểm xong trước MIR).
  - Đây là enforce Rule #4 trong khuôn khổ G đã ký → O tự chấp nhận, KHÔNG cần re-sign, nhưng **flag G ở gói báo cáo kế**.
- **Phase-0 spike (D)**: vứt sạch (rg MirType crates/ → none). Chứng minh structural fix được ordering-rule bug (`is_vec_type("Vector<Integer>?")` cũ=true sai → `Nullable(Vector).is_vec()`=false đúng).

## CHẶN: S1 chưa duyệt — blueprint `spec/plans/phase7-b1-type-system.md` lệch ADR 6 điểm
D viết blueprint 386 dòng (khảo sát file:line tốt) nhưng O **không duyệt S1** vì lệch:
1. Đặt `enum Type` → phải `MirType` (va nghĩa typecheck/generated Type).
2. Thêm `Trilean{refined}` → bỏ (dead, 0 reader).
3. Giữ `Vector(Box)`/`HashMap{k,v}` + mục R1/R2 → trần, R1/R2 tan.
4. **`is_copy_simple` né án tử G** — đổi tên thay vì hợp nhất. Phải MỘT logic (`is_copy(&self, body: Option<&Body>)`).
5. **Bước 2 "gate sẽ vỡ"** — VI PHẠM CLAUDE.md "tests green before any commit". Phải Display-bridge (`.ty.to_string()` tại consumer chưa migrate) trong-cùng-commit → mỗi commit xanh.
6. Số "site" lệch: blueprint 67 vs TODO 189 vs O-đếm-dispatch-thuần 76. Chốt 1 định nghĩa.

## Việc D phải làm trước khi xin duyệt lại S1
Sửa phase7 blueprint theo 6 điểm trên (đồng bộ ADR §3.1.1/§3.1.2, xoá R1/R2, Bước 2 Display-bridge-xanh, hợp nhất is_copy). KHÔNG gộp Bước 1+2 (Bước 1 song song = checkpoint O gate).

## Production plan (ADR §6, strangler, mỗi commit XANH)
S1 song song (thêm MirType+Display+parse, field giữ String) → S2 flip field + Display-bridge + **xoá simple_is_copy** → S3 migrate consumer theo cụm (mir→lower→borrowck→jit; 20 literal `__triet_*` GIỮ) → S4 nhổ `parse` shim (build nổ đỏ chỗ quên). Done: `rg parse/is_vec_type/simple_is_copy` → 0; gate 0·0·99·208; teeth đỏ (String→Copy, ordering, Struct-tra-nhầm-bảng, INV-4, fixture-27).

## Nợ mang sang (không đụng B1a)
B1b typecheck↔schema Type reconcile (defer sau B2) · concat→sret · B2 borrowck merge · B3 alias-analysis · C1 enum-payload fixture-27.
