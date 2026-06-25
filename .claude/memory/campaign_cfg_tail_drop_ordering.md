---
name: campaign_cfg_tail_drop_ordering
description: ★ ĐANG LÀM 2026-06-19 — Campaign CFG-Tail Drop Ordering (G mở). Bug A = block-tail-expr trả let-bound heap local bị pop_scope drop SỚM → E2421. Recon O xong, chờ WO→D.
metadata:
  node_type: memory
  type: project
  originSessionId: cfg-tail-drop-ordering
---

**Campaign CFG-Tail Drop Ordering** (G mở 2026-06-19, ưu tiên trước Lát 5 `?+>`). Nảy từ deep-recon heap-nullable: phát hiện **Bug A block-init E2421** vỡ cả plain Vector non-nullable (orthogonal heap-nullable + map).

## Bug A — root cause (O đào, file:line, KHÔNG đoán)
**Triệu chứng:** `let v: Vector<Integer> = { let mutable t = vector_new(); t = push(t,7); t }; len(v)` → **E2421 "use after storage end"**. Vỡ cả `String` (`{ let x="hi"; x }`) + plain non-nullable.
**MIR:** bb2 `_2 = move _4` (reassign t) → `Drop(_2)` NGAY → `len(_2)` dùng sau Drop.
**Rễ (triet-lower/src/lib.rs):** `Expr::Block` arm (2401-2434): push_scope → lower statements → lower tail `final_expr`→`result` → **`pop_scope()` (261-276) drop MỌI owned-local trong scope GỒM `result`** nếu result là `let`-bound heap local. `let mutable t` push_owned(_2) (1236); tail `t`=variable-ref trả thẳng _2 (không copy); pop_scope drop _2; rồi consumer Let push_owned(_2) lần nữa ở outer. **KHÔNG có cơ chế escape/remove owned_locals** (chỉ push_owned thêm + pop_scope drain — grep xác nhận 0 remove/retain).
**Vì sao hàm `mk()=...{...;t}` THOÁT:** tail→`Return(t)` + M4 return-escape (JIT skip Drop local trong Return). Expression-block KHÔNG có Return → không escape → bug.

## Trigger CHÍNH XÁC (control sạch, full-output)
| Case | KQ |
|---|---|
| block-init `{ let mutable t=...; t }` Vector/String | E2421 |
| if-arm let-bound heap tail | E2421 |
| double block-init | E2421 |
| if-arm tail = call trực tiếp `{ vector_new() }` | OK (call-result KHÔNG push_owned) |
| block tail = call `{ vector_new() }` | OK |
| block-init scalar `{ let a=5; a }` | OK (scalar no Drop) |
| match-arm let-bound heap tail | **UNVERIFIED** (match Trilean-literal chưa support lowering — limitation riêng, không dựng được repro) |
**Trigger = tail là variable-ref tới `let`-bound HEAP-owned local trong scope.** Scope dùng chung push_scope/pop_scope: block 2403/2432, if-arm 2914/2951, match-arm 3102.

## Liên hệ SIGILL 132 (G hỏi)
Anh em (cùng họ block-tail-expr) NHƯNG khác cơ chế: SIGILL 132 = struct-by-value tail-RETURN routing qua sret (`emit_struct_sret_copy` 1090, function-body return path, ĐÃ vá). Bug A = owned-local drop-escape trong expression-block (KHÔNG Return, KHÔNG M4-escape). Hàm-body thoát nhờ M4; expression-block không → Bug A.

## ★ Khuôn fix CHỐT (O verify fix-point, ĐÍNH CHÍNH đề xuất ban đầu)
**Fix-point DUY NHẤT = `Expr::Block` (lib.rs:2401-2434, pop_scope ở 2432).** Verdict 4 construct: Block trả thẳng tail-local (`result=lower_expr(tail)`) → pop_scope drop nó = BUG. `Expr::If` (2435, KHÔNG scope, Assign-to-outer), Trit-match arm (2914/2951), Nullable-match arm (3102) **ĐÃ ĐÚNG** — đều `Assign(result, body_val)` TRƯỚC pop_scope (move→M1 tombstone→pop drop no-op). If-arm E2421 chỉ vì nhánh CHỨA Expr::Block → vá Block là if-arm tự khỏi.
**KHÔNG đẻ `pop_scope_escaping` (phát minh thừa).** Idiom đúng đã nằm sẵn 3 chỗ. Fix = mirror: trong Expr::Block, sau `lower_expr(tail)→tail_val`, alloc fresh `result` typed theo tail_val, `Assign(result, tail_val)` (move→M1 tombstone tail-local), RỒI pop_scope (drop tail-local tombstoned=no-op), return result. SSOT-consistent, 0 cơ chế mới.
**Teeth: N7 counting block-init heap freed-EXACTLY-once (leak vs double-free 2 chiều); poison Assign-move (bỏ→quay lại direct-return) → E2421/double-free ĐỎ. Phủ block + if-arm-chứa-block, Vector + String. match-arm enum UNVERIFIED (match-literal limitation).**
(đề xuất CŨ pop_scope_escaping = bỏ; G duyệt nó nhưng O verify fix-point thấy idiom Assign-to-outer đã có sẵn → SSOT mirror thay vì helper mới.)

## ✅ Lát 1 (Bug A) — O KÝ, chờ G chốt+push — commit `159fd68`
Fix `Expr::Block` (2437-2449): tail non-reference → `Assign(fresh_result, tail_val)` move (M1 tombstone) trước pop_scope; **tail reference → direct-return `Ok(tail_val)`** (guard 2438). 3 fixture 207/208/209 + N7 `block_tail_drop_counting` (Vector+String).
**O verify máu (cây committed):** baseline 207→1/208→5/209→1/102→E2450; N7 2/2 xanh. Poison else→direct-return → 207/208/209 E2421 + N7 cả Vector lẫn String count2 (left:2 right:1). **LỆCH LỆNH `is_reference` guard — O VERIFY ĐÚNG bằng máu:** gỡ guard (unconditional) → 102 regress E2450→**E2440** (làm yếu A1 live-bomb guard). D flag đúng, data chống lưng. Reference = Copy, pop_scope không drop → không có Bug A → direct-path đúng (byte-identical code cũ). build/clippy 0, tree revert sạch.

## ⚠️ HỐ MỚI O ĐÀO RA (pre-existing, NGOÀI scope Lát 1) — Expr::If/match reference-arm MISS E2450
**Bằng máu:** `let r = if true { let inner="hello"; id(&0 inner) } else {...}; length(r)` → trả **5, KHÔNG E2450** = UAF chạy (length đọc memory freed). Fixture 102 (plain block, cùng pattern) bắt E2450; bản if-wrapped MISS. **Tệ hơn D flag** ("về lý thuyết E2450→E2440" — thực ra MISS hẳn diagnostic, exploitable). **Pre-existing (diff chứng minh):** D chỉ đụng Block; nhánh reference trả `Ok(tail_val)` byte-identical code cũ; Expr::If `Assign(result, then_val)` (2476) D KHÔNG đụng. Thủ phạm = `Assign(result, reference)` unconditional ở If(2476)/match phá loan-propagation → mất E2450 — đúng cái D's guard chặn ở Block. **→ Lát 2 CFG-tail (hoặc borrowck debt): áp guard is_reference cho Assign-to-result ở If/match.** match-arm vẫn UNVERIFIED (literal limitation).

## ★ Lát 2 RECON (O đào, LẬT outline G "bê khiên is_reference từ Block")
**Outline G KHÔNG dịch được:** Block direct-return MỘT tail (skip Assign OK). If/match **MERGE 2 nhánh vào 1 `result` qua Assign BẮT BUỘC** — skip Assign thì result không được ghi → vỡ merge. Không có lowerer-fix.
**Rễ thật (borrowck, MIR-confirmed):** `Call id(_2)→[_3]` tạo PropagatedLoan source=_1 dest=_3. Merge: `Drop(_1)` rồi `_4 = move _3`. `Drop` check (checker.rs:780-784) dùng **block-level `live_out`**; _3 bị `_4=move _3` tiêu thụ NGAY trong block → không ∈ live_out → check trượt → E2450 miss. **`Statement::Assign` handler (609-695) KHÔNG follow loan**: chỉ set dest→Owned (694), loan vẫn trỏ dest=_3 chết, không sang _4.
## ★★ Lát 2 ADR-0063 DRAFT (O recon EMPIRICAL — cây thử + revert, cây sạch `159fd68`)
**G chốt ADR-first + deep-recon-regression. O implement-thử-đo-revert 3 phương án, BÁC framing G "loan-follow Duplicate vs Retarget":**
- **(a) Duplicate loan @ Assign handler** → FAIL: headline vẫn 5. Timing — `Drop(_1)` xử TRƯỚC `_4=move _3` trong dataflow; lúc Drop, duplicate chưa xảy ra. Retarget cùng bệnh.
- **(b) point-level naive (dest dùng-sau, TÍNH Drop)** → E2450 fix nhưng **2 FALSE-POS 84/101** (return-borrow; `Drop(msg)` trước `Drop(r)` → r tính "dùng sau").
- **(c) CHỌN — point-level READ-after-Drop (loại Drop khỏi scan)** → headline E2450 + **204/204 integration + workspace 0 FAILED**.
**Fix thật = BORROWCK Drop-check (KHÔNG lowerer, KHÔNG loan-follow):** checker.rs:780-784 thêm `dest_used_after` = loan-dest được ĐỌC (Assign/Borrow/BinaryOp/GetDiscriminant source, KHÔNG Drop) ở statement SAU trong CÙNG block → `has_active_loans |= dest_used_after`. Bất biến: đọc ref sau source-drop cùng frame = UAF luôn E2450; Drop ref đó = an toàn → **0 false-pos by construction**. Construct-agnostic → phủ If+match+mọi merge bằng 1 điểm (G outline "guard 3 chỗ lowering" = vỡ merge, BÁC). Đính chính ADR-0046 (block live_out = xấp xỉ, thêm same-block read-after).
**ADR-0063 LOCKED (O+G ký, commit `fed21fc` local).** ADR mới đè ADR-0046 (không amendment — G ruling lịch sử không xóa). match-arm UNVERIFIED giữ biển báo.

## ✅ Lát 2 (UAF qua merge) — O KÝ, chờ G chốt+push — commit `51e401b`
Fix checker.rs Drop-check (806): `|| dest_used_after(loan.dest)` — clause point-level READ-after-Drop đúng ADR-0063 §3 (Drop loại khỏi scan). 1 điểm borrowck, KHÔNG lowerer. Fixture 210 (If-ref-arm UAF).
**O verify máu (cây committed):** baseline 210→E2450, 84/101→5, 205/205 integration + workspace 0 FAILED + borrowck 23/23. **Poison gỡ clause 806 → 210→5 (UAF về) RED + 84/101 GIỮ 5** (chứng minh clause chỉ THÊM catch merge-UAF, không động return-borrow → 0 false-pos by construction). Tree revert sạch, build/clippy 0. Khớp recon 204/204 (nay 205/205 +fixture 210). UAF class bịt cho mọi merge bằng 1 điểm construct-agnostic.

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_heap_nullable]] [[campaign_cfg_tail_expression_kickoff]]

## ✅ match-arm UNVERIFIED XÓA SỔ (push `cef6b4c`) + KẾ match-on-literal
**O sniff lật: UNVERIFIED = QUÁ BẢO THỦ** (O bỏ cuộc sớm, không thử Trit-param). Fixture `214_match_arm_uaf_e2450.tri` (`match t:Trit { -1_trit => {let a; id(&0 a)} ...}` ref ESCAPE arm → dùng sau merge) → **E2450**; poison `dest_used_after` (checker.rs:806) → 214 trả 2 (UAF về) RED → ADR-0063 phủ If + match-arm, máu chứng. ADR-0063 §5 xé cờ + §7 amendment sync chữ ký G (giữ lời gốc). 0 production code. **Bài học O (G mắng): vét cạn hướng trước khi cắm UNVERIFIED — refuse-fabricate tốt nhưng đầu hàng sớm là kém.**
**KẾ — campaign match-on-literal (FEATURE thuần, G chốt):** match refuse ở LOWER (Expr::Match enum-path fallthrough lib.rs:3797-3800); dispatch 4 nhánh Trit(2924 value-SwitchInt)/nullable(3040)/Outcome(3288)/else-enum. Integer/Trilean literal KHÔNG có value-path → mirror Trit-path 2924 (value-keyed SwitchInt). Exhaustiveness: Trilean=3 giá trị (true/false/unknown) có thể exhaustive; Integer=cần wildcard. Lý do "unblock teeth" moot — pure feature theo vision Giang.

## ★ ĐANG LÀM — Campaign Match-on-Literal (ADR-0064 LOCKED `1c26010` local, WO giao D)
ADR-0064 (O+G ký): Rule vét cạn — Integer cần wildcard, Trilean/Trit đủ-3-mặt-hoặc-wildcard. Encoding Trilean True=1/False=-1/Unknown=0 (lower:1464). **Tạm trap GAP-2 ở lower; nợ Typecheck-Exhaustiveness (compile-time) = campaign RIÊNG (G cấm nhồi chung).** WO: Expr::Match lowering thêm 2 nhánh value-keyed (scrut_ty==Trilean, ==Integer) TRƯỚC enum-path 3792, **mirror Trit-path 2924** (cấm pattern rẽ nhánh mới): cases Vec<(i64,bb)> + wildcard-last + SwitchInt + default→wildcard-body-else-Trap. Teeth: đúng-nhánh giá trị đúng · trap thiếu-nhánh SIGILL · poison default-Trap→goto-merge→giá trị rác RED · Trit 174+209 corpus no-regress.

## ✅ Match-on-Literal ĐÓNG — O KÝ, chờ G chốt+push — commit `d85b794` (ADR-0064 `1c26010`)
3 nhánh lower: Trit(2924)/Trilean(3045 key True=1/False=-1/Unknown=0)/Integer(3161 key=value), mirror khuôn, TRƯỚC enum-fallthrough. GAP-2 default→wildcard-else-Trap(3253). Fixtures 215→129/216→123. **O verify máu:** Integer/Trilean đúng nhánh; classify(9) no-wildcard → SIGILL 132; poison Integer Trap(3253)→Goto → exit 0 trả 0 (rác no-trap) RED; revert→SIGILL. Trit 174→111, 211 corpus, workspace 0 FAILED, build/clippy 0. **Limitation (D flag, O verify = refuse SẠCH không silent-wrong):** bare `let x=2`→Unknown→enum-fallthrough refuse; cần Integer/Trilean-typed scrutinee (type-inference literal-default-Unknown gap, pre-existing, ngoài scope). Nợ Typecheck-Exhaustiveness (compile-time) vẫn treo = campaign riêng (ADR-0064 §4).
