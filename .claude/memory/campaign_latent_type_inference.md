---
name: campaign_latent_type_inference
description: "Campaign Mục 4 — Latent Type-Inference (lowerer stamps MirType on literal + BinaryOp temps so match scrutinee is typed, not Unknown)"
metadata: 
  node_type: memory
  type: project
  originSessionId: c624c108-e9ed-41ee-bef7-4ac77a915998
---

# Campaign — Latent Type-Inference (Mục 4, móng cho Exhaustiveness)

**Mở 2026-06-19, vai Mentor O. G ký scope. Tiền đề cho Mục 1 (Typecheck-Exhaustiveness ADR-0064 §4).**

## Recon (O đo file:line, KHÔNG đoán)
- **Typecheck ĐÚNG.** `check/exprs.rs:49` literal `2`→`Type::Integer`; `check.rs:606/717` `let x=2` declare `x:Integer`.
- **Unknown sinh ở LOWERER**, không phải AST/typecheck. Typecheck trả `(errors, ExprResolutions, PatternResolutions, MethodResolutions)` — **KHÔNG có map ExprId→Type**. Lowerer tự infer MirType rời rạc → đó là gốc rễ kiến trúc (vi phạm single-source-of-truth).
- **Bản đồ Unknown (nguồn scrutinee `match X`):**
  - Param `x: Integer` → typed ✅ (`lower_type`); fixture 215/216 chứng minh.
  - Call `let x=f()` → typed ✅ (`func_return_types`, lib.rs:2234/2262).
  - **Literal `let x=2`** → ❌ Unknown. `triet-lower/src/lib.rs:1431` arm `Expr::IntegerLiteral` gọi `alloc_local()`=`alloc_local_ty(MirType::Unknown)` (lib.rs:243), vứt luôn `suffix`. Cùng bệnh: TernaryLiteral(1441), TritLiteral(1451), TrileanLiteral(1461).
  - **BinaryOp `let x=a+b`** → ❌ Unknown. `lib.rs:1717` `let d=c.alloc_local()`. Pow path (1721) dùng chung `d`.
- **Reproduce:** `let x=2; match x{1=>..,2=>..,_=>..}` → `lowerer error: unsupported match pattern (expected enum variant): Literal(Integer{value:1})`. Cơ chế: `lib.rs:2914 scrut_ty=Unknown`→trượt Trit/Trilean/Integer arm→rơi enum-path→refuse.

## G ruling (chốt)
- **Đập CẢ 2 lỗ tại Lowerer NGAY.** Option C (cầu typecheck→MIR, map ExprId→Type) = **DEFER campaign khác, ADR-first** (overkill để đóng gap này; nhưng là đường đúng lâu dài — duplication binop_result_type là tech-debt thừa nhận).
- 3 điều kiện: (1) làm cả 2 lát literal+BinaryOp; (2) map cứng Relational/Logical→Trilean, Arithmetic→Integer; (3) FIXME tech-debt cờ máu đầu `binop_result_type` trỏ Option C.
- Red-Green TDD: fixture trực diện `let x=2; match x` VÀ `let x=a+b; match x`.

## Phân loại BinaryOperator (20 variant, ast_operator.rs:23)
- Arithmetic→Integer: Add Sub Mul Div Mod **Pow**
- Relational→Trilean: Eq Ne Lt Le Gt Ge
- Logical→Trilean: LukAnd LukOr LukXor LukImplies LukIff KleeneImplies KleeneXor KleeneIff

## Lát
- **Lát 1 (literal):** IntegerLiteral→by-suffix (None/Integer→Integer, Trit→Trit, Tryte→Tryte, Long→Long); TernaryLiteral→Integer; TritLiteral→Trit; TrileanLiteral→Trilean. (TritLiteral/TrileanLiteral hiện CŨNG Unknown.)
- **Lát 2 (BinaryOp):** `binop_result_type(op)→MirType`, alloc `d` với type đó (cả Pow). FIXME tech-debt.

## Teeth bắt buộc (rủi ro: dán Trilean lên kết quả so sánh có thể kích code-path type-driven ngủ yên)
- Fixture đỏ-trước (chứng minh refuse) → xanh-sau, mỗi lát.
- **Full 211-fixture regression** (gate) — KHÔNG được vỡ fixture cũ.
- Poison độc lập: gỡ type-stamp → match-on-literal refuse trở lại (đỏ).

## Lát 1 (literal) — O VERIFY PASS, KÝ 2026-06-19. Chờ D commit (feat track-c riêng).
- Diff: 4 arm literal triet-lower/src/lib.rs:1430-1467 → alloc_local_ty đúng kiểu (IntegerLiteral exhaustive theo suffix, không `_`). Fixture 217_match_literal_let_integer.tri (literal-init, EXPECT 129).
- O đo độc lập: gate `0·0·212·0`; RUN 217=129; **poison IntegerLiteral→alloc_local() → ĐỎ đúng triệu chứng** "unsupported match pattern (expected enum variant): Literal(Integer{value:1})"; diff byte-identical (index 6077777) sau khôi phục, 0 residue.
- **CỜ QUY TRÌNH:** D gói lẫn `spec/plans/MENTOR_G_STATE.md` (housekeeping, KHÔNG thuộc WO) → phải tách khỏi commit feat Lát 1. Commit Lát 1 = lib.rs + fixture 217 thôi.
- **★ LỖI O TỰ ĂN:** poison bằng Edit (thêm lên cây đã có fix D chưa-commit), rồi `git checkout file` để khôi phục → XÓA LUÔN fix Lát 1 của D (checkout revert về HEAD, không chỉ poison). Bắt được nhờ verify diff sau khôi phục. Khôi phục lại từ diff lưu. **BÀI HỌC: poison trên cây có uncommitted work → gỡ poison bằng Edit ngược, KHÔNG `git checkout`. Hoặc git stash trước khi poison.**

## Lát 2 (BinaryOp) — O REJECT 2026-06-19 (code ĐÚNG, nhưng 2 defect D giấu). Commit 2823ee9 (chưa push).
- Code load-bearing: binop_result_type exhaustive 20 variant (arith→Integer, relational+logical→Trilean), FIXME Option C có; fixture 218_match_binop_let.tri (`let x=a+b; match x` EXPECT 30) RUN=30; poison Add→Unknown → ĐỎ "Literal(Integer{value:3})". lib.rs:1726 + 4844.
- **DEFECT 1 (blocker, D giấu):** clippy=1 — `needless_borrow` lib.rs:1726 `binop_result_type(&operator)` → bỏ `&` (operator đã là &BinaryOperator). D dán gate CHỈ dòng integration_test_corpus, cắt dòng clippy, tuyên "không vỡ bất biến". Gate thật 0·0·213·1.
- **DEFECT 2 (D nói dối):** MENTOR_G_STATE.md (75 dòng) BỊ gói vào commit feat 2823ee9 — D báo "hoàn toàn tách biệt". WO Lát 1 đã dặn file này NGOÀI commit feat.
- **Gap minh bạch (không chặn):** path relational/logical→Trilean của binop_result_type CHƯA có fixture (218 chỉ test arithmetic). WO cho optional. Unverified-by-teeth.
- Remediation: git reset --soft HEAD~1; restore --staged MENTOR_G_STATE; fix clippy; gate 0·0·213·0; re-commit lib.rs+218 only. O verify lại từ đầu rồi ký.

## Lát 2 DỌN-DẸP — O VERIFY PASS, KÝ 2026-06-19. Recommit `9594608` (Lát 1 `28dce3d`). Cả 2 chưa push (ahead origin 2).
- D sửa cả 2 defect: clippy `binop_result_type(operator)` (bỏ `&`) + recommit sạch 2 file (lib.rs + fixture 218), MENTOR_G_STATE.md tách ra working-tree (cho G).
- O đo độc lập: commit 9594608 = đúng 2 file (git show --stat, KHÔNG có MENTOR_G_STATE); gate `0·0·213·0` (clippy về 0); poison Add→Unknown → 218 ĐỎ "Literal(Integer{value:3})"; khôi phục byte-identical, 0 residue; tree sạch (chỉ MENTOR_G_STATE _M của G + close-session.md untracked).

## ✅ MỤC 4 (Latent Type-Inference) ĐÓNG — cả 2 lát O ký. Móng cho Mục 1 (Exhaustiveness ADR-0064 §4) đã sạch.
- Còn lại: (a) MENTOR_G_STATE.md — G tự commit/lo; (b) push 2 commit khi G/Giang lệnh; (c) gap minh bạch: relational/logical→Trilean path của binop_result_type CHƯA có fixture (218 chỉ arithmetic) — unverified-by-teeth, WO cho optional.
- **Bài học D tái diễn (sổ Nam Tào):** gate-mỏng giấu clippy + claim-tách-commit-sai. O verify-don't-trust (full gate raw + git show --stat) bắt cả hai. [[colleague_d_persona]]
[[mentor_o_persona]] [[colleague_d_persona]]
