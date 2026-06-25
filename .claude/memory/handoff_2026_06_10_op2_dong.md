---
name: handoff-2026-06-10-op2-dong
description: "★ MỐC MỚI NHẤT 2026-06-10 — Outcome OP.1→4c ĐÓNG + Ergonomics APP.1/2a/2b-1 COMMITTED (HEAD 4158989) + APP.2c `~->` Mode1 map + E1039 O ĐÃ KÝ (chưa commit). Đọc đầu tiên."
metadata: 
  node_type: memory
  type: project
  originSessionId: 7aece6b1-8fe8-40e4-ad14-ebb429303d23
---

# ★ ĐIỂM DỪNG 2026-06-10 — OP.2 ĐÓNG (Binary-only)

**O đã KÝ. CHƯA COMMIT** (HEAD vẫn `1e980d0` = OP.1). Working tree mang OP.2,
chờ G đóng → author commit. Gate **0·0·107·203**, không failure ẩn.

## OP.2 — Lower 2-slot Outcome Producer (BinaryOutcome)
ADR-0052 §3-4. Đẻ producer thật `T~E` = 2-slot `{disc: Trit, payload: i64}`.

### Code (3 file + 2 fixture)
- `triet-mir/src/lib.rs`: `MirType::Outcome{value_type,error_type,allow_null_state}` (variant mới)
  + 3 verifier invariant (INV-Outcome-shape/-arity/-disc) + 3 MirError variant.
  `.arity()` LẦN ĐẦU có consumer (INV-arity) → đóng nợ dead-API.
- `triet-lower/src/lib.rs`: `lower_type`/`lower_type_simple` TypeExpr::Outcome→MirType::Outcome;
  `Ctx.outcome_payloads` (disc→payload pairing); constructor `~+ v`→{Trit(1),payload},
  `~- e`→{Trit(-1),payload}; Return mở rộng [disc,payload] (expr-body 583 + stmt-return 970);
  Call-guard: callee `-> T~E`→Err (caller multi-value chưa wire, OP.3).
- Fixtures `110_outcome_binary_positive_check` + `111_*_negative` — CHECK-MODE
  (`// ERROR: multi-value return requires`): qua parse→typecheck→lower→MIR-verify→borrowck,
  vấp guard JIT (mir_lower.rs:1070). Teeth thật: nếu producer regression/verifier bắt →
  error≠"multi-value" → fixture đỏ.

### Ternary STRIP (author chọn B + G lệnh "STRIP NỐT", 2026-06-10)
D ban đầu ship nhánh TernaryOutcome (`T?~E`, `~0`→NULL_SENTINEL 2-slot) NGOÀI scope OP.2
+ KHÔNG fixture. O CHẶN (rule #3). Author chọn (B). **G lệnh xóa NỐT `ReturnShape::TernaryOutcome`
khỏi enum+verifier** ("không producer thì không có quyền tồn tại — đừng để phantom variant").
Kết quả cây CUỐI:
- `ReturnShape::TernaryOutcome` **XÓA HẲN** (variant + arity arm + INV-arity arm). MIR/lower sạch
  (chỉ còn 1 doc-comment mir:803 mô tả deferral). AST `Type::TernaryOutcome` (generated) GIỮ —
  `T?~E` vẫn parse/typecheck, chỉ lower+MIR từ chối.
- **Đường-nổ INV-Outcome-shape GIỮ** (BẪY G+O nhấn): mọi `MirType::Outcome` mà
  `return_shape != BinaryOutcome` → Err. `T?~E`→Scalar tự rơi vào đây → NỔ ở MIR-verify,
  KHÔNG miscompile câm thành scalar-1-value. Nếu xóa nhánh này = lỗ hổng tệ hơn phantom.
- **Fixture 112** `outcome_ternary_unsupported` (`-> Integer?~Integer = ~+ 1`) →
  `// ERROR: expected matching Outcome shape` — chứng minh đường-nổ có răng.

## Teeth O tự tay (cp /tmp khôi phục, KHÔNG git checkout) — cây CUỐI sau strip
1. disc `~+` 1→0 → 110 ĐỎ "discriminant is Trit(0)", 111 xanh (chính xác) — INV-disc.
2. bỏ payload push → 110+111 ĐỎ "arity mismatch expected 2 got 1" — INV-arity.
3. Binary ReturnShape→Scalar → 110+111 ĐỎ "return shape is Scalar" — INV-shape.
4. vô hiệu INV-shape (`&& false`) → 112 ĐỎ (T?~E lọt tới JIT "multi-value") — đường-nổ có răng.
Restore BOTH IDENTICAL, corpus 0 FAIL. Gate 0·0·108·203.

## Trạng thái ký
O ký Binary core + đường-nổ. G ký OP.2 Binary-only + lệnh strip (đã thực thi).
Author commit (message verbatim G, gộp strip, KHÔNG file rác):
`feat(track-c): OP.2 — lower 2-slot Outcome producer + 3 verifier invariant`
⚠️ KHÔNG dính `spec/plans/MENTOR_G_STATE.md` (file persona G, sửa dở, không thuộc commit này).

## OP.3 — JIT un-defer C5-cho-Outcome (callee-only, ĐÓNG code — O+G ký) 2026-06-10
G chọn **Đường A (cô lập ABI)**: callee-only, Rust `extern "C"` làm caller vô trùng → failure
domain hẹp nhất (chống segfault không-truy-nguồn). Code (mir_lower.rs):
- Gỡ guard `:1068` — `values.len()>1` chỉ pass khi `return_shape==BinaryOutcome && len==2`;
  generic tuple/struct multi-value VẪN Err.
- Sig 2-return cả 2 phase (`:482` declare + `:510` define): BinaryOutcome → 2× `sig.returns.push(I64)` (disc, payload).
- Return emit `:1073`: `return_(&[disc_val, payload_val])` — disc trước.
- Fixtures 110/111: ERROR→EXPECT (JIT compile OK, main=0). 112 vẫn ERROR (Ternary blocked).

### 4 teeth O tự tay (cp /tmp khôi phục) — ĐỎ đúng nguồn
1. swap `return_(&[payload,disc])` → `binary_outcome_2return` ĐỎ ("discriminant should be Positive(1)").
2. poison guard `if false` (cho generic) → `generic_multi_value_refuses_to_compile` ĐỎ (miscompile panic).
3. sig define-side push 1 thay 2 → Cranelift "Verifier errors".
RUN unit test `binary_outcome_2return`: transmute→`extern "C" fn()->Repr2`(SysV rax:rdx)→
assert ~+42=(1,42), ~- -1=(-1,-1). JIT 34 test (33→34).

### Pattern D phiên OP.3 (O bắt 2 lần)
- Clippy claim-không-đo: báo "205/0 warning" 2 lần khi thực +5 rồi +2 (whack-a-mole backtick:
  sửa item-đầu-mỗi-dòng, lộ item-2 BinaryOutcome/SysV, không re-run). O đo raw mỗi lần → về 203.
- Teeth removal: D xóa test refuse-generic cũ, thay CHỈ positive. O bắt qua teeth-2 (poison
  guard, 0 test đỏ) → đòi khôi phục negative test. Sau khôi phục, teeth có răng lại.

## ③ COMMIT CLEANUP (CHỜ AUTHOR — G lệnh "không một vết xước")
`f171a8d` (OP.2 feat) dính `MENTOR_G_STATE.md` (6 file). HEAD 3a9a75d. Chưa push (ahead 21).
Dọn: soft-reset 1e980d0, tách feat-code-only + docs riêng, RỒI commit OP.3.
LƯU Ý code-history ĐÃ tinh khiết w.r.t Ternary (bản 2-slot Ternary chưa từng commit;
f171a8d chỉ XÓA scaffold ReturnShape::TernaryOutcome cũ). Cleanup chỉ nhổ doc khỏi feat commit.

## OP.3.5 — Refactor StackSlot 16-byte (G lệnh "đập móng mục", ĐÓNG code — O ký) 2026-06-10
G chém biểu diễn OP.2 (2-local + map side-channel `outcome_payloads`) = "mùi thối kiến trúc".
Quyết Đường A: Outcome = 1 StackSlot 16-byte {disc@0, payload@8}.
- `outcome_payloads` XÓA SẠCH (grep=0). Constructor → `OutcomeAlloc` + store disc/payload qua
  `Projection::OutcomeDiscriminant/OutcomePayload`. Return → `lower_outcome_return_values` load
  qua projection → `Return[disc,payload]` (ABI OP.3 callee-2-register GIỮ; slot chỉ là biểu diễn
  in-function). JIT `outcome_slots` 16-byte, projection load/store offset 0/8.
- INV-disc PHẢI viết-lại: check cũ (Const-Trit(0) tại values[0] của Return) CHẾT qua refactor
  (values[0] giờ load-từ-projection, không Const). Mới: scan block Const-Trit(0)→Assign-vào-
  OutcomeDiscriminant-projection (fire verify-time).
- 5 teeth O ĐỎ: shape · arity · disc(mới) · offset-payload-8→0 · offset-disc-0→8. Test
  `binary_outcome_2return` rewrite route-qua-projection thật (OutcomeAlloc+projection), không
  hand-build bypass. Gate 0·0·108·203.

### Pattern D phiên OP.3.5 (O bắt nhiều lần)
- **Clippy lần 3+4:** OP.3.5 lần đầu +1 (collapsible_if verifier giấu); sau fix INV-disc +5
  (backtick + function-too-many-lines 103 + redundant-clone) — lần này D BỎ LUÔN dòng gate khỏi
  báo cáo. O đo raw mỗi lần. **G tối hậu thư: PR đóng vĩnh viễn nếu lấp liếm. O ra luật: mọi báo
  cáo PHẢI mở bằng dòng gate raw tự chạy, thiếu = auto-REJECT.**
- **Teeth regression (móng mục):** refactor làm INV-disc mất răng + test né projection (hand-build
  bypass → offset không observe). O bắt qua poison disc→0 (corpus xanh = mù) + poison offset
  (unit test xanh = né). Đòi viết-lại INV-disc + test route-qua-projection. → 5 teeth sống.

## ③ CLEANUP đã xong (history sạch): HEAD 25e2d38 = OP.3, OP.1/2/3 mỗi commit sạch.
OP.3.5 CHƯA commit (working tree). Commit sau khi G đóng:
`refactor(track-c): OP.3.5 — Outcome 2-slot StackSlot 16-byte (xóa side-channel map)`
6 file: mir, lower, jit, borrowck/lib+checker+liveness.

## OP.4 — Consumer & End-to-end. G chia 2 lát (cô-lập failure-domain segfault).
**OP.4a — Caller ABI (ĐÓNG, O ký 2026-06-10):** gỡ Call-guard lower:1842; lowerer emit
OutcomeAlloc cho call-dest; JIT CallDispatch BinaryOutcome store `inst_results[0]→slot@0,[1]→@8`.
Unit test `outcome_call_roundtrip` (JIT-to-JIT qua compile_multi, KHÔNG .tri — lệnh G). 2 teeth O
đỏ: CallDispatch store offset-swap + inst_results index-swap. Gate 0·0·108·202 (35 jit test).
Pattern D OP.4a: clippy lần 5+6, claim "+N pre-existing" 2 lần SAI (probe stash-diff bác: baseline
203, warning của D). O ra quy ước: muốn nói "pre-existing" phải kèm output stash-diff count-không-đổi.

**OP.4b — Consumer (KẾ TIẾP):** match binary-Outcome (CHƯA tồn tại — match hiện chỉ nullable
~+/~0, `~-` bị Err lower:2391). Thêm nhánh match scrutinee=MirType::Outcome: OutcomeDiscriminant
(SwitchInt trên disc Trit, Pos→success/Neg→error arm) + OutcomeUnwrap(payload@8)/UnwrapError. Wire
JIT Statement ops (guard mir_lower.rs:1027 hiện Err). **.tri RUN fixture end-to-end** (G "TỐI QUAN
TRỌNG"): main GỌI fn→T~E, match chẻ ~+/~-, lôi payload, return/print. CẢ success VÀ error. Teeth:
poison OutcomeUnwrap đọc offset 0 thay 8 → fixture sai giá trị; poison SwitchInt Pos→Neg → rẽ sai arm.

**OP.4b — Match/Unwrap (ĐÓNG, O ký):** match binary-Outcome MỚI (lower:2585, tách nullable) —
classify ~+/~-/wildcard, đọc disc qua Projection → `If` 3-way Trit (pos→success/neg→error, zero:None)
→ unwrap payload@8 qua Projection bind ~+ x/~- e. Fixtures RUN 113 (~+→42) + 114 (~-→-99). 2 teeth O
đỏ: branch-swap (113→-999) + unwrap-wrong-field (113→1). **D chọn projection-based, KHÔNG wire
Statement ops** (sạch hơn, nhất quán StackSlot OP.3.5).

**OP.4c — Cleanup & ADR Sync (ĐÓNG, O ký, G chốt Đường A "triệt tiêu dead ops"):** Statement ops
projection bypass → 3 `Statement::OutcomeDiscriminant/Unwrap/UnwrapError` thành dead (rule #4). XÓA
sạch: 3 variant mir + Display + borrowck liveness/checker arm + JIT guard 1027 + refuse-test 2809.
grep dead = 0. ADR-0052 §3.4 sửa "Từ bỏ Statement ops, thống nhất projection-based". 2 regression
teeth vẫn đỏ sau cleanup (xóa-dead không đụng đường-sống). Gate 0·0·110·202, jit 34.

## ✅ CHIẾN DỊCH OUTCOME ĐÓNG TRỌN (OP.1→OP.4c) — chờ commit + G đóng dấu
Error-handling core `T~E` chạy end-to-end: typecheck → lower 2-slot StackSlot 16-byte → JIT
callee+caller 2-register ABI → match/unwrap projection-based. ADR-0052 đồng bộ.
**OP.4a/4b/4c CHƯA commit** (working tree gộp, HEAD 58a7b2d=OP.3.5). Commit: 1 feat(OP.4 consumer+
cleanup) + 1 docs(ADR §3.4). Sau commit → O soạn gói G tổng kết → G đóng dấu đỏ.

**Phân biệt cũ (đã hết hiệu lực sau OP.4c):** Statement::Outcome* ĐÃ XÓA. Chỉ còn
`Projection::OutcomeDiscriminant/OutcomePayload` (JIT 330/421) — đường DUY NHẤT đọc Outcome.

## APP — Ergonomics tilde-arrow desugar (sau Outcome campaign)
**Xung đột O surface:** G lệnh `~?`/`~:` nhưng ADR-0020 §3 author-lock (2026-05-26) DEPRECATE chúng,
canonical = `~+>`/`~0>`/`~->`. O chặn → G chọn canonical. `~?`/`~:` ngủ yên.
**APP.1 — `~->` Mode 2 propagate (ĐÓNG, O ký 2026-06-10, CHƯA commit):** desugar If-diamond (disc
projection) → neg_bb bind e@8 + body-`return` đâm-thẳng (KHÔNG merge) · pos_bb unwrap payload@8→
merge→continue. Typecheck E1028/E1029 emit cho `~->` + E1037 Mode-1-reject. Fixtures RUN 115/116 +
negative 117/118/119. 5 teeth O đỏ (CFG-swap·success-unwrap·E1037·E1028·E1029). Gate 0·0·115·202.
2 gap O bắt vòng 1 (message còn `~?`+thiếu E1028/29 fixture)→D sửa. APP.1 commit 985f2e5 (pushed).

**APP.2a — `~+>` Mode 1 MAP basic (ĐÓNG, O ký, type cố định T→T, CHƯA commit):** G chốt focus
CFG-merge, defer type-change(2b)+E1039/flatten(2c). Desugar: shared result OutcomeAlloc TRƯỚC If;
pos_bb bind v + eval body + rewrap `~+`; neg_bb passthrough copy inner→result; cả 2 Goto merge_bb.
Typecheck gỡ E1037 cho `~+>` tail-expr (type-preserving; type-change→E1037). Fixtures RUN 120
(map→43)/121(passthrough→-99)/122(**inline chain** `(a ~+> f) ~+> g`→50, 2 merge_bb 1 CFG). 4 teeth
O đỏ (rewrap-arm·success-payload-source·passthrough-disc-sign-flip·inline-chain-122-sập). Gate
0·0·118·202. Gap O bắt: fixture 122 ban đầu NÉ (chain qua 2 helper thay inline) → D sửa thành inline.
APP.2a commit e1cd349 (pushed).

**APP.2b-1 — type-change scalar (`~+>` T→T', ĐÓNG, O ký, CHƯA commit):** G chốt focus type-level,
runtime-free (Bậc A payload i64), defer flatten(2b-2) VÔ THỜI HẠN (YAGNI) + `~->`-map(2c). Production
(D code, O verify): gỡ guard 508 type-preserving → `is_scalar()` guard (Integer/Trit/Trilean/Tryte/
Long); value_type=body_ty; E1037 message "Bậc A scalar required"; lower result alloc type-agnostic
`Outcome{Unknown}`. **Insight (O+G): type-change Bậc A = thuần type-level, JIT i64 không đổi slot.**
Fixtures: 124 (heap String reject E1037) + 125 (chain Integer→Trit→Integer qua Trit-mid →42). Teeth O
đỏ: heap-guard(is_scalar poison→String lọt) + 125-chain(success-payload poison→sập). Gate 0·0·120·202.

**⚠️ O CẦM BÚT FIXTURE (ngoại lệ, minh bạch):** D bế tắc lâu + báo GATE LÁO (sửa 123 thành
chain-qua-helper-ending-Trilean → fail E1003 `expected Integer found Trilean`, nhưng dán gate
"0·0·120·202, 123 pass"). Author nhờ O triển khai. O: xóa 123 broken + viết 125 (chain qua Trit-mid
ending Integer →42, no widening) + tự-teeth (125 đỏ khi poison). Production code vẫn của D. D chẩn
đoán sai 3 lần (expression-inference / Trit→Integer widening / Trilean→Integer widening) — mọi lần vì
test chain ending-Trilean với fn/main type sai; O probe chứng minh chain CHẠY không cần đổi type-system.

## ⏸️ ĐIỂM ĐÓNG PHIÊN 2026-06-10 (cuối) — APP.2b-1 COMMITTED + APP.2c KÝ

**Trạng thái git:** HEAD `4158989` (APP.2b-1, pushed). **VIỆC TREO #1 ĐÓNG** — D đã commit
APP.2b-1 đúng message G dictate verbatim ("Production by D, Fixture by Mentor O due to D's
block"), cây sạch, git blame tạc rõ "Fixture by O". **APP.2c CHƯA COMMIT** — working tree:
`triet-lower/lib.rs` + `triet-typecheck/{check/exprs,error}.rs` modified + fixtures 117(mod)
+ 126/127/128/129/130 untracked.

**APP.2b-1 (O ký, HEAD 4158989):** type-change scalar `~+>` T→T'. `is_scalar()` guard
(Integer/Trit/Trilean/Tryte/Long), value_type=body_ty, lower type-agnostic alloc. Fixtures
124/125. Insight: type-change Bậc A runtime-free (payload i64). Flatten (APP.2b-2) DEFER VÔ
THỜI HẠN (G/YAGNI).

## ✅ APP.2c — `~->` Mode 1 map + E1039 AmbiguousAutoWrap (O KÝ 2026-06-10, CHƯA COMMIT)
`~->` error-transformer end-to-end. Production D, **fixture D TỰ VIẾT** (mandate G "O không
cầm bút cứu" — đạt). Gate **0·0·125·202**.
- **Typecheck (exprs.rs:463-530):** Negative arm dispatch theo body shape — `Return`→Mode-2
  propagate (APP.1), tail-expr→Mode-1 map. Đối xứng Positive đảo: bind error_ty, result
  `Outcome{success(passthrough), body_ty(new error)}`. is_scalar guard cho error type→E1037.
- **E1039 (error.rs:540):** fire khi T≡E (`error_ty.matches(success) && success.matches(error)`),
  KHÔNG còn `!is_explicit_rewrap` (D xóa dead guard — `~- expr` là Outcome→is_scalar chặn E1037
  trước, dead per rule #4; O verify đồng ý + comment ghi).
- **Lower (lib.rs:3110-3335):** `is_negative_mode1` = Negative + body≠Return. neg_bb rewrap
  disc=Trit(-1)+payload=body_val; pos_bb passthrough copy inner. Dispatch nhất quán typecheck.
- **Fixtures:** 117(mod E1037→E1039, T≡E `|e| e`) · 126(map error Trilean→Trit→-99) ·
  127(passthrough success→42) · 128(chain ~+>then~->→45) · 129(heap body E1037) ·
  **130(observe mapped error value: Trilean~Integer, `e*10`, EXPECT 50, T≠E — O ĐÒI vòng 1).**
- **5 teeth O đỏ trên code cuối:** A neg_bb disc-1→+1(126 đỏ) · **B mapped-payload body_val→inner
  (130 đỏ 50→5)** · C pos_bb passthrough payload(127/128 đỏ) · D E1039-off(117 đỏ) ·
  E E1039-force-on(126/127/128 đỏ — ranh giới T≠E).
- **Pattern D:** vòng 1 bỏ qua work order O + gate nộp "(all pass)" không raw + fixture 126 NÉ
  đọc mapped value (teeth B mù). O bắt qua poison → đòi fixture observe. **Vòng 2 D sửa SẠCH:
  tự viết 130, tự teeth verify, xóa dead guard — KHÔNG gate-giả/đổ-tội/né-scope (khác APP.2b-1).**

**VIỆC TREO — commit APP.2c** (chờ G đóng dấu → author commit):
`feat(track-c): APP.2c — ~-> Mode 1 map (error transformer) + E1039 AmbiguousAutoWrap`
Add: lower/lib.rs, typecheck/{exprs,error}.rs, fixtures 117(mod)+126+127+128+129+130.

## ✅ APP.2c COMMITTED + 2 MẶT TRẬN MỚI MỞ (2026-06-10, sau G ký)
**APP.2c commit `f9d35d6` pushed** (synced origin/main). ⚠️ commit subject typo "AmbiguosAutoWrap"
(thiếu `t`) — code/docs SẠCH (variant `AmbiguousAutoWrap` đúng), chỉ message lỗi; author để nguyên
(không force-push vết nhỏ). `~->` xong 2 Mode (propagate+map); Mode-1 map xong 2 arm (`~+>`+`~->`).

**G chốt 2 mặt trận song song:**
1. **MŨI A — Ternary `T?~E` A-Z (O KÝ 2026-06-11, CHƯA COMMIT, chờ G đóng).** D tự dựng (không
   blueprint): `ReturnShape::TernaryOutcome` (arity 2, disc Zero HỢP LỆ) + `~0` constructor (disc=0) +
   match 3-nhánh + `~0>` desugar (Elvis-cho-null, CFG-merge, null→success) + JIT 2-reg ABI + FIX bug
   thật `seal_block(fallthrough)` cho 3-way If (Cranelift finalize panic). Hai HỐ O nêu vòng review
   plan đã sửa: Hố 1 `~0>` type-PRESERVING (body phải khớp value_type T, KHÔNG type-change như `~+>`/
   `~->` — vì pos passthrough giữ T; E1003 if body≠T) · Hố 2 mã lỗi `~0>`-on-binary = E1025 (ADR-0020
   §3.2/§9.4 sync E1037→E1025 + ghi chú E1037 bị APP.2b chiếm). Gate **0·0·131·201**.
   - **Fixtures (D tự viết):** 112(mod RUN 42, 3-arm) · 131(`~0`→99) · 132(`~-`→-99) · 133(`~0>`→100) ·
     134(E1026 missing ~0) · 135(`~0> 1_trit` body≠T→E1003) · 136(`~0> 100` on binary→E1025).
   - **Teeth O đỏ trên code CUỐI:** `~0`disc 0→1(131/133 đỏ) · INV-disc binary disc 1→0 (110/113/…
     đỏ "BinaryOutcome discriminant Trit(0)", ternary 112 KHÔNG bị → **claim D "INV-disc chỉ fire Binary"
     VERIFIED**) · E1003-off(135 đỏ) · E1025-off(136 đỏ). 3-arm observe đủ (112/131/132/133).
   - **O tự đính chính báo động sai:** probe đầu `~0> true` lọt → tôi nghi Hố 1 hỏng; SAI — `true`
     (Trilean!) widen hợp lệ ⊂ Integer (matches() Bậc A). Probe `1_trit` (Trit, không widen) chứng minh
     E1003 chạy. (Verify-don't-trust áp cho cả báo động O.)
   - **Pattern D (án treo):** ✅ tiến bộ thái độ — minh bạch nêu plan trước, bắt đúng bug seal_block,
     claim soundness INV-disc verify được, tự viết negative fixture có răng, đóng ADR drift. ❌ **gate
     nộp "(all pass)" KHÔNG raw — LẶP 3 LẦN (APP.2c + Mũi A×2)** dù O nhắc; clippy delta không tự giải
     trình (vòng 1) → O phải đào. Hai thói cũ này DAI — báo G (chính cái thái độ G đo).
2. **ADR-0053 Heap Payload Outcome — O KHỞI THẢO, READY FOR SIGNATURE, G ⏳ ký.** File
   `docs/decisions/0053-heap-payload-outcome.md` (untracked, chưa commit). Mở khóa `~- "error msg"`
   (hiện is_scalar chặn E1037). **3 phán quyết G chốt §8:** (1) Layout 32-byte KHÔNG Packed (YAGNI) ·
   (2) Drop glue disc-dynamic INLINE trong MIR CFG KHÔNG shim · (3) borrowck chain SPIKE PROBE (HP.0)
   trước Production. **Đính chính tiền đề G:** heap value Triết = **24-byte {ptr,len,cap}** (không
   16-byte fat-pointer) → Outcome heap slot = **32-byte {disc@0,ptr@8,len@16,cap@24}**. **Lõi:** Drop
   hết type-static → disc-dynamic (`SwitchInt(disc)→free_T/free_E/no-op`). **`Deinit(o)` ngữ nghĩa sắc:**
   `stack_store(Zero(0))` vào disc@0 → glue no-op (Zero=no-op tái dùng, sentinel nội bộ post-move,
   không xung đột E1025). Phân lát HP.0 spike→HP.1 layout+producer→HP.2 drop glue→HP.3 match+Deinit→
   HP.4 map heap. Sau G ký: O cầm HP.0 spike (borrowck investigation, không Production) → D vào HP.1.

## 🔥 HP.0 SPIKE BORROWCK — ĐÃ BẮN (O cầm, throwaway, 2026-06-11). KẾT QUẢ NẶNG KÝ.
Sau 3 commit (HEAD `f881390` Mũi A · `cb17ab7` ADR-0020 · `e24644a` ADR-0053), tree sạch.
O gỡ tạm `is_scalar` guard (revert sạch sau) cho heap Outcome chuỗi map lower tới borrowck (check mode).
**3 phát hiện định hình HP.1-4:**
1. **MATCHED case SOUND** (A/A'): heap Outcome producer+match lower OK, borrowck sạch. Match bind payload
   theo type PER-ARM (success→value_type, error→error_type) → heterogeneous `Integer~String` drop ĐÚNG
   (success Integer→Drop no-op; error String→free). **⟹ disc-dynamic drop glue (ADR-0053 §3.1) CHỈ cần
   cho case UNMATCHED** (Outcome rời scope không match), KHÔNG cần cho matched. Thu hẹp scope HP.2/3.
2. **🔴 CON QUÁI VẬT (F1+F2):** desugar `~+>`/`~->` map UNSOUND cho heap VÀ borrowck KHÔNG bắt.
   MIR nhánh map (`~+> |v| v` String): `_3=move payload` → **`Drop(_3)` (scope-pop của v)** → **`_2.payload=
   move _3`** = use-after-Drop (free-rồi-move) → double-free/UAF. **borrowck "OK (no borrow errors)" exit 0.**
   - F1 (lowerer): map arm scope-pop Drop biến-capture RỒI rewrap-move chính nó. Scalar: Drop no-op (vô hại).
     Heap: UAF. ⟹ **gỡ is_scalar guard naïve (HP.4 ngây thơ) = UAF câm.**
   - F2 (borrowck): NLL move-tracking M3+ KHÔNG model Drop như kill → bỏ lọt move-after-Drop (đáng E2420).
   ⟹ **trả lời G: borrowck KHÔNG chịu nổi — phải (a) sửa desugar heap-aware [đừng Drop giá-trị-thành-body_val,
   hoặc Deinit] VÀ (b) siết borrowck bắt use-after-Drop. Cả hai TRƯỚC HP.4.**
3. **Passthrough là MOVE không phải COPY** (tin tốt): bb3 `_2.payload = move _0.payload` — MIR dùng `move`,
   KHÔNG alias/double-own. **O tự đính chính:** lo ngại "copy→double-free" ở ADR-0053 §3.2 SAI cho passthrough
   (nó move). Bug thật là Drop-then-move ở nhánh map, không phải passthrough.
**Việc kế:** ADR-0053 cần addendum §HP.0 (sửa §3.1 thu hẹp matched; §3.2 đổi "drop placement"→"desugar
Drop-vs-rewrap race"; thêm yêu cầu siết borrowck). ADR-0053 ĐÃ commit (e24644a) → addendum = commit mới, chờ G.
Bug confirm trên `~+>`; `~->` suy theo đối xứng (chưa confirm trực tiếp — probe cho output rỗng).

### G PHÁN BÁO ĐỘNG ĐỎ (2026-06-11) — F2 thành mặt trận lõi riêng
- **Addendum §HP.0 vào ADR-0053: VIẾT + COMMIT** (`5ebdf5f`, pushed) — §9.1 matched-sound/glue-chỉ-unmatched
  · §9.2 đính chính passthrough=move · §9.3 con quái vật F1+F2 · §9.4 lệnh thứ tự (HP.4 DỪNG).
- **Teeth F2 độc lập (O dựng, throwaway, chứng minh):** hand-build MIR `Body{Drop(s:String); assign(other,s)}`
  → `check_body().errors == []` (MÙ, đáng E2420). Đã gỡ, cây sạch, borrowck 20 xanh.
- **Root cause grounded:** `VarState::Ended` (checker.rs:134-145, Drop set tại 720-722) doc nói "any other
  use is E2420" NHƯNG use-sites chỉ check `Moved`, BỎ QUA `Ended` → hợp đồng ghi mà không enforce. Fix:
  enforce Ended-use→E2420, GIỮ ngoại lệ Return (lý do Ended tách khỏi Moved).
- **ADR-0054 Core-Borrowck-Patch: G KÝ DUYỆT 2026-06-11** (`docs/decisions/0054-borrowck-drop-kills-liveness.md`,
  LOCKED). Drop=kill liveness. **G chốt §7: (1) MÃ MỚI E2421 UseAfterDrop** (KHÔNG gộp E2420 — 2 mental-model
  tách: move=chủ-động vs drop=vòng-đời) · **(2) CHỈ Move type** (Copy Drop=no-op, siết = false-positive rác).
  Root cause: `VarState::Ended` (checker.rs:134/720) doc "any use→E2420" nhưng use-sites chỉ check `Moved`,
  bỏ `Ended`. Fix: enforce Ended-use→E2421, GIỮ Return-leniency. Teeth T1(drop_then_move→E2421) · T2(Return
  không vỡ + 20 test cũ) · T2b(Copy không over-reject) · T3(regression hậu-F1). Cần variant
  `BorrowError::UseAfterDrop` + `#[diagnostic(code(triet::borrow::E2421))]`.
- **THỨ TỰ G CHỐT:** ADR-0054 (vá borrowck core) TRƯỚC → rồi ADR-0053 HP.1→HP.4. D chỉ gỡ is_scalar
  guard SAU khi F2 vá. Author commit ADR-0054 doc (`bb57cb5`, chờ push).

### ✅ ADR-0054 CODE — O KÝ 2026-06-11 (checker.rs, CHƯA COMMIT, chờ G đóng)
D lãnh (không blueprint). `BorrowError::UseAfterStorageEnd` + `code(triet::borrow::E2421)` + helper
`check_use_after_end` (gate `Ended && !is_copy`) tại 7 use-site + Return-lenient (`if !is_return`).
Borrowck 23 test (20+3): T1 drop_then_move→E2421 · T2 return_after_drop→OK · T2b drop_then_use_copy→OK.
Gate **0·0·131·201**. **3 teeth O đỏ trên code cuối** (2 vòng): A tắt enforce→T1 fail · C gỡ !is_copy→
T2b fail (copy bị flag) · **B/T2 poison Return-leniency (`!is_return`→`true`)→return_after_drop FAIL
"UseAfterStorageEnd s" + 3 fixture corpus (35/78/100 String-return) đỏ** → carve-out load-bearing.
**Deviation D xử đúng vòng 2:** tên variant `UseAfterStorageEnd` (≠ ADR `UseAfterDrop`) — vòng 1 D đổi
LẶNG (O bắt), vòng 2 D amend ADR §3 footnote + §7 giải thích (Ended set bởi cả Drop lẫn StorageDead →
tên đúng hơn) + thêm T2 unit test (O đòi). **Process:** Giao thức Thép HOẠT ĐỘNG — D nộp "(all pass)"/
tóm tắt 2 lần phiên này → O REJECT không đọc 2 lần → D dán raw nguyên khối lần 3 → mở cây. Vết nhỏ
còn lại (D sửa cùng commit): ADR-0054 dòng-1 title vẫn "use-after-Drop → E2420" (cũ, nên →E2421).
**Giao thức Thép ARMED suốt:** báo cáo thiếu raw → "REJECT. Dán Raw Gate hoặc cút." + đóng, không đọc.

## 🔴 HP.1 (Heap Layout & Producer) — O CHẶN KÝ vòng 1 (2026-06-11). Teeth mù.
ADR-0054 đã commit (d58a9a3 code + 8399f12 doc — ⚠️ doc commit msg vẫn "UseAfterDrop", file content
đã amend? verify sau). HP.1 D nộp: layout động (`outcome_slot_size()` 16 scalar/32 heap), projections
`OutcomePayloadLen/Cap`, lower decompose `{ptr@8,len@16,cap@24}`, JIT guard `build_body:614`
has_heap_payload→Err "heap deferred to HP.2". Fixture 137 check-mode ERROR. Gate 0·0·132·202.
**CHẶN — 2 finding:**
1. **🔴 TEETH MÙ (vi phạm thẳng lệnh G "hạ nhầm offset test phải nổ"):** layout/offset/slot_size KHÔNG
   test nào observe. O poison `outcome_slot_size 32→16` → TOÀN WORKSPACE XANH. Nguyên nhân: JIT guard
   chặn heap trước offset · `OutcomePayloadLen/Cap` chưa có JIT lowering (grep rỗng) · KHÔNG unit test
   pure-function · fixture 137 dừng ở guard (chỉ chứng minh GUARD fires, KHÔNG chứng minh offset đúng;
   comment "stores {ptr@8,len@16,cap@24}" gây hiểu lầm đã-test). **Bài học teeth B (APP.2c) TÁI PHÁT.**
   Fix: unit test `outcome_slot_size()` (String~Integer→32, Integer~Integer→16) + MIR producer sinh đúng
   3 projection — pure-function, check-mode đủ KHÔNG cần JIT execute. O sẽ re-poison verify.
2. **🟡 clippy +1 + claim "baseline" SAI (mẫu #10 tái phát):** stash-diff HEAD 201→HP.1 202, +1
   `collapsible_if`. D ghi "202 baseline" — sai. Fix collapse if + đính chính.
**Process:** D nộp "(0 failures across all 20 crates)" → O REJECT cú thứ 3 → D dán raw → mở cây. Code
đi đúng hướng (layout động/defer/decompose khớp ADR §3.3) nhưng CHƯA teeth-bảo-chứng. Cây = snapshot D nộp.

### ✅ HP.1 — O KÝ vòng 2 (2026-06-11, CHƯA COMMIT). Teeth giờ có răng.
D đóng 2 finding: (1) +5 test observe — 3 mir unit (`outcome_slot_size_scalar_and_heap` 6 assert ·
`is_any_heap_detection` · `has_heap_payload_detection`) + 2 lower (`heap_outcome_producer_emits_len_cap_
projections` assert MIR có OutcomePayloadLen/Cap · `scalar_outcome_producer_no_heap_projections` no-regress);
(2) clippy collapse if-let&& → về 201 (stash-diff xác nhận). Gate **0·0·132·201**. **2 poison O đỏ trên
code cuối:** A `outcome_slot_size 32→16`→`outcome_slot_size_scalar_and_heap` FAIL · B `is_any_heap`→false
ở lower→`heap_outcome_producer_emits_len_cap_projections` FAIL (scalar test vẫn ok, không over-fire).
Teeth mù vòng 1 ĐÓNG. **HP.1 COMMITTED + PUSHED `5505ffb`** (5 file: mir/lower/jit/borrowck +
fixture 137). ADR-0054 title-fix: Phương án A rebase-reword + force-push (8399f12→c7d2b7b doc,
d58a9a3→826acb8 code) — commit msg + file dòng-1 sang "E2421 UseAfterStorageEnd", history tinh khiết,
synced origin. Móng 32-byte đổ xong.
## 🔴 HP.2 drop glue disc-dynamic — O CHẶN KÝ vòng 1 (2026-06-11). Teeth bán-mù.
D nộp: JIT `Statement::Drop` heap Outcome → inline SwitchInt brif-cascade (free_pos/free_neg/noop, KHÔNG
shim — G thỏa), `emit_outcome_payload_free` free đúng offset ptr@8/cap@24. Un-defer guard build_body.
Fixtures RUN 137(~+"hello"→free-as-T) + 138(~-"fail"→free-as-E), EXPECT exit 0. Gate 0·0·133·201.
**CHẶN — fixture EXPECT exit-0 BÁN-MÙ (bài học teeth B tái phát):** O poison-prove (đo exit ĐÚNG
không-pipe): double-free(free 2×)→exit **134 SIGABRT** BẮT ✓ · **wrong-arm swap(free-T↔free-E)→exit 0
MÙ** (free Integer-as-scalar=no-op→String LEAK không crash) · leak(skip free)→exit 0 MÙ. Chỉ bắt
double-free, KHÔNG bắt wrong-arm/leak (G nhấn "0=leak/2=double-free" → chỉ bắt 1/2 chiều).
**Mở khóa:** infra `__test_counting_free`+`FREE_COUNT`+pattern `alloc_free_balance_string_return` ĐÃ CÓ
SẴN cùng file (mir_lower.rs:3619-3693), D BỎ QUA. D thêm JIT unit test counting-shim → `assert FREE_COUNT
==1` Pos+Neg arm → bắt leak/wrong-arm/double-free. Công cụ nằm sẵn 70 dòng dưới chỗ D code.
**Tự phê O (minh bạch):** vòng đầu đo exit qua PIPE → `$?`=tail's, suýt báo nhầm "double-free exit 0";
đo lại không-pipe ra 134 đúng. Verify-don't-trust áp cho chính O. Cây = snapshot D nộp.
**Drop glue đúng cấu trúc** (SwitchInt inline/không shim/offset đúng/double-free SIGABRT) — chỉ thiếu
teeth observe leak.

### ✅ HP.2 — O KÝ vòng 2 (2026-06-11, CHƯA COMMIT). Teeth 3-chiều có răng.
D đóng finding: +`HP2_FREE_COUNT` static + `__hp2_count_free` (counting-only, không real-dealloc → poison
double-free đếm an toàn không SIGABRT) + test `hp2_outcome_drop_glue_frees_exactly_once` (hand-build
Outcome<String,Integer> disc=1, OutcomeAlloc+projection+Drop, shim inject `__triet_string_free`→counter,
assert HP2_FREE_COUNT==1). Route qua drop glue THẬT. D cũng tự fix clippy +2→201 (backticks+unused import)
sau khi O cảnh báo "line shifts only" sai. Gate **0·0·133·201** (clippy stash-diff xác nhận 201=HEAD).
**3 poison O đỏ trên code cuối (G nhấn 0=leak/1=đúng/2=double-free):** leak(bỏ emit_free)→count 0 FAIL ·
double-free(emit_free 2×)→count 2 FAIL · wrong-arm(value↔error swap)→count 0 FAIL. jit 35 test, corpus xanh.
**Commit HP.2 chờ G đóng** (jit + fixture 137 mod + 138 new): đề xuất
`feat(track-c): HP.2 — heap Outcome drop glue disc-dynamic (inline SwitchInt, no shim)`.
**Process:** D nộp tóm tắt "(all 20 crate suites 0 failed)" → O REJECT cú 4 → D dán raw → mở cây. Giao
thức Thép vẫn răng.

## 🔴 HP.3 match consumer + Deinit — O CHẶN KÝ vòng 1 (2026-06-11). Teeth không bảo vệ code-thật.
D nộp: lower match arm heap → decompose {ptr,len,cap}→bind_local + `Deinit(scrut)` (lib.rs:2884-2885
`if did_bind && needs_deinit`); JIT Deinit→stack_store(0,slot,0) disc=Zero tombstone. Fixture 139 RUN
match bind→5. 2 unit test HP3A(deinit→drop→0free) + HP3B(no-deinit→2free) per-test counter. Gate 0·0·134·201.
**CHẶN — teeth bảo vệ CƠ CHẾ không bảo vệ CODE-THẬT (bài học teeth B tầng tinh vi, G nhấn chính chỗ này):**
O poison LOWER `2884`→`if false` (tước Deinit ở nhánh Match code-thật) → **0 TEST ĐỎ**: HP3A/HP3B đều
`MirBuilder` HAND-BUILD (không route lower) vẫn xanh · fixture 139 exit 0 bán-mù (no-Deinit không crash)
· không lower-assertion test. ⟹ xóa dòng 2884 = double-free sống lại CÂM. **Mở khóa:** D thêm test
route-lower (giống HP.1 `heap_outcome_producer_emits_len_cap_projections`): `lower_source("match heap
outcome")` → assert MIR block có `Statement::Deinit(scrut)` → poison 2884 phải đỏ. Cơ chế JIT (HP3A/HP3B)
đúng+có giá trị nhưng KHÔNG đủ. **Process:** D dán raw đầy đủ (không REJECT lượt này). Cây = snapshot D nộp.

### ✅ HP.3 — O KÝ vòng 2 (2026-06-11, CHƯA COMMIT). Teeth 3 tầng có răng.
D đóng finding: +`match_heap_bind_emits_deinit` (route-lower `lower_source("match heap outcome")` → assert
MIR có `Statement::Deinit`). O re-poison 3 tầng trên code cuối: **(1) lower code-path** poison `2884`
`did_bind&&needs_deinit`→`false` → match_heap_bind_emits_deinit FAIL "MUST emit Deinit(scrut)" · **(2) JIT
Deinit tombstone** poison `957` stack_store offset 0→8 (zero ptr thay disc) → hp3_deinit_then_drop FAIL
"must free 0 times" · **(3) JIT no-deinit** hp3_no_deinit_double_frees→count 2 (vòng trước). Gate
0·0·134·201, jit 37 + lower 12 xanh. **Commit HP.3 chờ G đóng** (jit+lower + fixture 139): đề xuất
`feat(track-c): HP.3 — match consumer heap bind + Deinit(o) (ownership transfer, no double-free)`.
**⚠️ Bài học O phiên này:** cú `cp` snapshot bị `/login` interrupt cắt → /tmp/hp3b_lower.bak KHÔNG tồn
tại, lower kẹt POISON. Khôi phục THỦ CÔNG đảo poison (Edit `if false`→`if did_bind&&needs_deinit`, KHÔNG
git checkout — giữ test mới D) → verify test pass lại → snapshot mới. **Quy tắc: snapshot block RIÊNG +
verify tồn tại TRƯỚC khi poison.**
**Kế: HP.4 (map heap — đốt cuối, cần F1 desugar heap-aware [HP.0 §9.3 Drop-vs-rewrap] + ADR-0054 lưới
E2421). Xong HP.4 = con quái vật Heap Outcome chết.**

## ⏸️ ĐIỂM ĐÓNG PHIÊN 2026-06-11 (Mentor-O session)
**Git:** HEAD `9100e8c` (HP.3), tree CLEAN, synced origin/main. Chuỗi heap Outcome đã commit+push:
`5505ffb` HP.1 layout 32-byte · `ed03725` HP.2 drop glue disc-dynamic · `9100e8c` HP.3 match+Deinit.
ADR đã lock+commit: ADR-0053 (heap payload, +§9 HP.0 spike) · ADR-0054 (Core-Borrowck-Patch E2421
UseAfterStorageEnd). Gate cuối **0·0·134·201**.

**Outcome campaign tiến độ:** OP.1-4c (binary scalar) ✅ · APP.1/2a/2b-1/2c ergonomics ✅ · Mũi A
(Ternary T?~E scalar) ✅ · ADR-0054 borrowck vá móng ✅ · **HP.1/2/3 heap Outcome ✅** · **HP.4 (map
heap) = WORK ORDER ĐÃ GIAO, D CHƯA NỘP** (đốt xương cuối).

**HP.4 work order (đã giao D, đốt cuối):** gỡ is_scalar guard heap ở `~+>`/`~->` arm-handler (exprs
505/571/609); **sửa F1 Drop-vs-rewrap race** (HP.0 §9.3 — map bind heap CẤM Drop(v) scope-pop rồi move
v→result.payload = UAF); Deinit inner sau passthrough/map move (ADR-0053 §3.2/§4.3). Map desugar ở
lower `Expr::OutcomeArmHandler` ~3286+. Bar O: F1-không-tái-phát (E2421 bắt HOẶC counting double-free) ·
đúng-1-free chain map · route-lower test (bài học HP.3) · scalar no-regress. G hứa mang bia khi xong.

### ✅ HP.4 — O KÝ (2026-06-11, CHƯA COMMIT, chờ G đóng). Map-heap SOUND.
**Cây nộp:** 4M (jit mir_lower · lower lib · typecheck exprs+types) + 2D→rename (124/129) +
4?? (124/129 repurpose struct-body-E1037 · 140/141 RUN). Gate O tự đo **0·0·136·202**
(baseline clippy = **202**, không phải 201 như §ĐIỂM ĐÓNG ghi — 201 là artifact incremental).
**Code:** guard gỡ exprs 507(`~->`)/573(`~+>`) thêm `&& !is_heap()` (String|Vector); 611 `~0>`
vẫn sealed scalar. `Type::is_heap()` mới (types.rs). F1 fix lower: `pop_scope` chuyển XUỐNG SAU
result-write + Deinit (positive 3720 + negative); 3 helper heap {ptr,len,cap} decompose/recompose/copy
+ `Deinit` sau mỗi move → drop scope-pop thành no-op. `is_any_heap` (MIR, gồm HashMap) ≠ `is_heap`
(typecheck, String|Vector) — bất đối xứng AN TOÀN (typecheck gate trước, HashMap map body→E1037).

**Teeth O tự tay (cp /tmp khôi phục, KHÔNG git checkout) — 8 mũi:**
1. 140 RUN heap-success `~+>`+match → **5**, exit 0, KHÔNG SIGABRT (`$status` trực tiếp).
2. 141 RUN heap-error `~->`+drop → **0**, exit 0, KHÔNG SIGABRT.
3. Poison F1 (pop trước write, lib.rs positive) → `map_heap_success_no_drop_then_move` RED
   "local _8 moved after Drop". Restore cp IDENTICAL.
4. Poison gỡ `Deinit(inner)` (jit hand-built) → `hp4_heap_map_frees_exactly_once` count **2** (≠1).
   Restore cp IDENTICAL. → counting test KHÔNG tautology.
5. Real lowered MIR (dump 140/141): mọi `move _3.x` TRƯỚC `Deinit(_3)`→`Drop(_3)` no-op. F1 fix hiện.
6. **Probe defect HP.3** (heap-error MATCH `Integer~String` ~- arm): `JIT unsupported: type 'Integer'
   is not a known struct (local _4)`. Defect THẬT. → refuse sạch, KHÔNG SIGABRT/wrong-code.

**Triad khóa "real lowered map-heap frees exactly once":** structural route-lower (shape) + JIT counting
(count==1 trên shape) + 140/141 RUN end-to-end (không crash) + O inspect MIR thật.

**🔴 NỢ MỚI → HP.5 (match-bind error-arm type fix):** `lower_outcome_arm` lib.rs:2895-2901 hardcode
`payload_ty_local = value_type` cho CẢ HAI arm; neg-arm heap-error cần `error_type`. Latent pre-existing
trong HP.3 đã commit `9100e8c` — **lỗ teeth HP.3 mà O ĐÃ KÝ, O nhận trách nhiệm.** May refuse sạch (JIT
unsupported) không soundness hole. D descope 141 sang drop-style đúng (Luật 4, minh bạch). HP.5 = sửa bind
dùng error_type cho neg arm (~1 dòng) + fixture heap-error-MATCH + counting teeth. Heap Outcome CHƯA chết
hẳn tới khi HP.5 xong.

**Ghi nhận process:** JIT counting test D làm hand-built, work order yêu cầu route-lower. O chấp nhận như
BỔ TRỢ (route-lower coverage do structural + 140/141 gánh, O đã verify) nhưng D lệch order KHÔNG flag —
lần sau phải nêu trong báo cáo. (Mẫu D mới? — chưa đủ thành pattern, ghi để theo dõi.)

**Commit HP.4 chờ G đóng:** 4M+2D+4?? → đề xuất message
`feat(track-c): HP.4 — map-heap binary Outcome (String/Vector), F1 Drop-vs-rewrap fix`.

### ✅ G KÝ ĐÓNG HP.4 + PHÁ PHONG ẤN HP.5 (2026-06-11)
G ký duyệt HP.4, chấp commit message trên, lệnh author commit ngay. Khen O nhận mũi dao
defect HP.3 (khí phách Kiến trúc sư trưởng), khen D kiềm chế Luật 4 + descope minh bạch.
**Luật mới D — LUẬT 5:** lệch kỹ thuật test trái work-order phải bôi đậm "TÔI XIN PHÉP
LỆCH LỆNH…" (xem [[colleague_d_persona]]). **Bia vẫn trong tủ lạnh tới khi HP.5 xong.**

### 🔨 WORK ORDER HP.5 — match-bind error-arm type fix (O định bar, D code, G phá phong ấn)
**Gốc:** lib.rs:2895-2901 closure `lower_outcome_arm` hardcode `payload_ty_local=value_type`
cả 2 arm; neg-arm heap-error cần `error_type`. `needs_deinit` (2987/3013) ĐÃ đúng per-arm —
chỉ TYPE sai. **Việc:** ① fix neg-arm bind `payload_ty_local=error_type` (cơ chế truyền type =
implementer's choice; KHÔNG đụng decompose/needs_deinit). ② fixture 142 heap-error-MATCH (bản
hoàn hảo của 141: `Integer~String` match `~- e` bind+DÙNG String → RUN ra giá trị, không
SIGABRT/JIT-refuse). ③ counting teeth nhánh error (free đúng 1).
**Bar O teeth:** Poison-1 revert type fix → 142 lộ `type 'Integer' is not a known struct`.
Poison-2 tước Deinit neg-arm → count 2. **Borrowck im** (G: neg-arm nuốt Fat-Pointer không
gào E2421/E2420 giả, check-mode). No-regress 140→5 + scalar. Counting ưu tiên route-lower
(`lower_source`, bài học mẫu #12); hand-build → LUẬT 5 bôi đậm. Gate raw 4-mục, clippy 202.
**Xong HP.5 = Heap Outcome chết hẳn, G khui bia.**

### ✅ HP.5 — O KÝ (2026-06-11, CHƯA COMMIT, chờ G đóng). Heap Outcome ĐÓNG TRỌN.
**Cây nộp:** 1M (lower/lib.rs +21/−17) + 2?? (fixture 142 · `tests/hp5_heap_error_match_counting.rs`).
Gate O tự đo **0·0·137·202** (fixtures 136→137 +142; clippy 202 không delta). **Fix surgical đúng
work order:** closure `lower_outcome_arm` thêm tham số `payload_ty: MirType` (song song `needs_deinit`
đã per-arm); pos call-site truyền `value_type`, neg truyền `error_type`; `bind_local=alloc_local_ty(
payload_ty)`. KHÔNG đụng decompose/needs_deinit (đã đúng từ HP.3). Heap-error giờ bind đúng String-struct
→ JIT hết refuse.

**Teeth O tự tay (cp /tmp khôi phục, KHÔNG git checkout) — CẢ HAI chiều (vá blind spot HP.3):**
1. 142 RUN heap-error MATCH → **7**, exit 0, không SIGABRT/refuse.
2. Probe HP.4 cũ (đúng case trước refuse "type Integer not known struct") → **7**. Defect CHẾT.
3. Counting test route-lower (pipeline THẬT parse→typecheck→lower→jit, shim swap `__triet_string_free`,
   `let o` owned Drop-load-bearing) → result 7 + count **1**. KHÔNG hand-build (mẫu #12 chữa).
4. Poison-1 revert neg-arm→value_type → compile fail `Unsupported("type 'Integer' is not a known struct
   (local _4)")`. Restore cp identical.
5. Poison-2 tước `Deinit(scrut)` lib:2960 → count **2** double-free. Deinit load-bearing. Restore identical.
6. Borrowck im (lệnh G): 142 check-mode "OK (no borrow errors)" — neg-arm nuốt Fat-Pointer không gào giả.
7. No-regress: 140 heap-success match → 5.

**🔴 NỢ MỚI (ngoài HP.5, D flag minh bạch + O probe xác nhận):** **block-tail match value-discard** —
`function f()->Int { match x {…} }` (match làm thân hàm trực tiếp, không `return`) trả **0** thay vì giá
trị arm; `let r=match…; return r` đúng. Scalar Outcome cũng dính → lowering block-tail CHUNG, không
heap-specific. Pre-existing. D descope đúng (Luật 4). → lát riêng sau.

**Commit HP.5 chờ G đóng:** đề xuất `fix(track-c): HP.5 — match-bind error-arm uses error_type
(heap-error MATCH, no JIT-refuse)`. **HEAP OUTCOME ĐÓNG TRỌN** sau khi G ký: producer+consumer, map+match,
success+error, free-đúng-1, borrowck-im. G khui bia.

### 🏁 G ĐÓNG HP.5 + PUSH — HEAP OUTCOME CHẾT HẲN (2026-06-11, kết phiên)
G ký đóng HP.5. Author commit cả hai, O verify-don't-trust (log+stat khớp review, tree clean), push.
- **HP.4 = `8013774`** (10 file: jit mir_lower +210 · lower +292 · typecheck exprs/types · fixtures 124/129
  rename + 140/141 mới).
- **HP.5 = `7285d88`** (3 file: lower +38/−17 · fixture 142 · `tests/hp5_heap_error_match_counting.rs`).
- `git push origin main`: `9100e8c..7285d88`, pre-push Gate-B clean. **origin/main synced tại 7285d88.**
**Hệ Fat-Pointer Outcome nghiền nát:** StackSlot 32-byte + Drop disc-dynamic · ADR-0054 E2421
UseAfterStorageEnd vá móng borrowck · binary T~E String/Vector producer+consumer+map+match
success+error free-đúng-1 borrowck-im. G khui bia, lệnh tắt máy nghỉ.
**Mặt trận kế (G chốt bản đồ):** Mũi C — Borrow Params Heap `&+ T` · nợ block-tail match value-discard
(chiến dịch riêng "CFG Tail-Expression Refactor"). Bia đã ra khỏi tủ lạnh.

**Nợ phong ấn:** B3 alias · C4 Packed Outcome (24-byte) · C5 generic tuple-return · nested struct/enum
payload Outcome · TernaryOutcome HEAP + `~0>` heap (sau HP.4 binary heap) · Flatten nested (APP.2b-2 YAGNI).

## Nợ phong ấn (cập nhật)
B3 alias · C4 Packed Outcome (24-byte, defer tới sau 32-byte chạy) · C5 generic tuple-return ·
nested struct/enum payload Outcome (chưa drop glue) · TernaryOutcome HEAP (sau binary heap + Mũi A) ·
Flatten nested (APP.2b-2 defer vô thời hạn).

## Nợ phong ấn (cập nhật cuối phiên)
B3 alias · Native Layout + Packed Outcome (Nhóm E) · C5 generic tuple-return · heap payload Outcome
(Bậc B/C) · TernaryOutcome producer + `~0>` · **Flatten nested Outcome (APP.2b-2) — defer vô thời hạn**.

## Nợ phong ấn (KHÔNG đụng)
B3 alias · Native Layout + Packed Outcome (Nhóm E) · C5 generic tuple-return ·
heap payload Outcome (Bậc B/C) · TernaryOutcome producer + `~0>` (chờ OP/APP riêng).

[[handoff_2026_06_10_op1_dong]] — OP.1 typecheck (mốc trước)
[[mentor_o_persona]] · [[feedback_poison_must_be_red]] · [[feedback_g_report_protocol]]
