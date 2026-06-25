---
name: campaign_typecheck_exhaustiveness
description: "Campaign Mục 1 — Typecheck-Exhaustiveness compile-time (match thiếu nhánh Integer/Trilean/Trit → E1026, đóng nợ ADR-0064 §4)"
metadata: 
  node_type: memory
  type: project
  originSessionId: c624c108-e9ed-41ee-bef7-4ac77a915998
---

# Campaign — Typecheck-Exhaustiveness (Mục 1, đóng nợ ADR-0064 §4)

**Mở 2026-06-19 sau khi Mục 4 (Latent Type-Inference) đóng. Vai Mentor O. G ký 5 quyết định + ADR-0064 §8.**

## Recon (O đo file:line)
- Gap: `triet-typecheck/src/check/exprs.rs:1728 check_match()` dispatch exhaustiveness cho Outcome(:1784)/Nullable(:1789)/Enum(:1794), **THIẾU nhánh scalar** → :1797 nuốt. Đây là GAP-2 (ADR-0064 §4 trap-runtime thay vì compile-error).
- Mã E1026 sẵn ở `error.rs:399` — khuôn "1 mã, nhiều variant" (NonExhaustiveOutcomeMatch/EnumMatch). Span-extract :928.
- Khuôn wildcard-detect tái dùng: check_nullable_exhaustiveness(:1861), check_enum_exhaustiveness(:1904).
- Pattern model (pattern.rs:13/79): catch-all = Wildcard|Variable(name); Integer arm = Literal(Integer{suffix:None}); Trit = Literal(Integer{suffix:Some(Trit)}) val -1/0/1; Trilean = Literal(Trilean(True/False/Unknown)); Or expand; Range KHÔNG thỏa Integer.
- Type: Type::Integer, Type::Trilean{refined:_} (cả 2), Type::Trit (types.rs:13).
- **Blast-radius: ZERO fixture vỡ** — 215/218 Integer-có-`_`, 174/214 Trit-đủ-3, 216 Trilean-đủ-3 đều exhaustive sẵn. Không fixture nào EXPECT trap-runtime scalar.

## G ruling (5 quyết định, ADR-0064 §8 sửa-có-dấu-vết)
1. Tái dùng E1026 + variant NonExhaustiveScalarMatch{missing,span}. KHÔNG mã mới.
2. Catch-all = Wildcard `_` HOẶC Variable(bind `other =>`).
3. **Trap GAP-2 ở lower GIỮ NGUYÊN — cấm gỡ (G: bẻ tay). Defense-in-depth.**
4. Amend ADR-0064 §8 (không mở ADR mới; 0065 dành Struct?/Enum?).
5. Tryte/Long DEFER (ghi nợ; lower chưa support match).

## Campaign = TYPECHECK-ONLY. Lower KHÔNG đụng.
- Lát 1: commit ADR-0064 §8 (đã soạn) riêng `docs(adr)`.
- Lát 2: error.rs variant NonExhaustiveScalarMatch + check_scalar_exhaustiveness (Integer→cần catch-all; Trilean/Trit→đủ 3 mặt hoặc catch-all) gọi từ check_match sau dispatch enum.
- Teeth: 3 fixture ĐỎ E1026 (integer-no-wildcard, trilean-missing, trit-missing) đỏ-trước-xanh-sau + regression xanh + 1 fixture XANH Variable-catch-all.
- Luật Thép: full gate raw (đủ dòng clippy), commit riêng từng lát không gói file lạ, clippy 0, KHÔNG gỡ trap.

## Tiến độ
- **Lát 1 ĐÓNG** `7bb54fa` (ADR §8, 1 file sạch).
- **Lát 2 code XONG (uncommitted), O verify design sạch:** error.rs NonExhaustiveScalarMatch{missing,span} E1026 + span-extract; exprs.rs dispatch sau enum (Integer/Trilean/Trit) + has_scalar_catch_all(Wildcard|Variable) + collect_literal_patterns(Or-expand đệ quy) + 3 helper. Lower KHÔNG đụng. Fixture 219/220/221 ERROR E1026.

## 2 NGÃ BA D dừng-hỏi (Luật 4+5), O verify độc lập + phán:
- **Regression 3 test match_literal_t.rs** (campaign match-on-literal cũ): dùng INT_NO_WILDCARD (cố ý non-exhaustive để test lower trap) → nay E1026 → helper `lower_source:43 assert type_errors.is_empty()` nổ. **D claim "driver thật lower tiếp qua type-error" = SAI** (O verify main.rs:59 driver return ExitCode(3) trên type-error, KHÔNG lower — BLOCKING). **O RULING = HYBRID (không phải Option A global-relax của D):** test #1 case-maps + #3 jit → đổi INT_NO_WILDCARD→INT_WITH_WILDCARD (giữ strict); test #2 trap → helper `lower_bypassing_typecheck` scoped (giữ INT_NO_WILDCARD), `lower_source` giữ strict nguyên. Lý do: global-relax mất teeth type-clean mọi test. Sửa test = trong scope; lib.rs lower CODE không đụng, trap nguyên.
- **Fixture 222 Variable-catch-all → typecheck test:** O verify lib.rs:3224 lower REFUSE Variable trong Integer match → `// EXPECT` fixture bất khả. **DUYỆT** thay bằng typecheck unit test (decision #2 là tầng typecheck). D probe-trước-báo đúng.

## ★ NỢ MỚI (minh bạch, lòi ra từ 222): typecheck-accept / lower-refuse Variable-catch-all
- `match x {1=>10, other=>other}`: typecheck PASS (Variable=catch-all per ADR-0064 §8 #2) nhưng lower REFUSE (lib.rs:3224 chỉ nhận Wildcard+literal). Chương trình typecheck-clean → nổ lower. **Loud-fail, không silent-wrong**, ngoài scope typecheck-only. Defer: lower bind scrutinee→variable ở default block. Báo G chốt có ghi ADR §8 không.

## ✅ MỤC 1 ĐÓNG — O VERIFY MÁU + KÝ 2026-06-20. Lát 1 `7bb54fa` (ADR §8) + Lát 2 `57021c0` (code). 2 commit local, CHƯA push (ahead origin 8e41129 by 2).
- **O đo độc lập:** git show --stat 57021c0 = đúng 8 file (error.rs, exprs.rs, fixture 219/220/221, match_scalar_exhaustiveness.rs, match_literal_t.rs, match_trit_t6.rs) — KHÔNG ADR/MENTOR_G_STATE/close-session. Full gate `0·0·216·0`. 3 binary: match_scalar 5/5, match_literal 5/5, match_trit 3/3 (no ignore/filter). **Poison dispatch neuter → 3 typecheck negative FAILED + fixture 219/220/221 nuốt E1026** → load-bearing; khôi phục byte-identical Edit-revert (KHÔNG checkout — bài học cũ).
- **O tự vét cạn blast-radius (D + recon-O đều sót trước):** quét MỌI test source scalar-match → candidate #5 trilean_refined_annotation consume_plain EXHAUSTIVE (an toàn); counting/heap = Outcome/nullable (không scalar). Blast-radius = 4 test (3 literal + 1 trit), hybrid áp đủ.
- **Hybrid (O ruling, KHÔNG global-relax):** case→INT_WITH_WILDCARD strict; trap→lower_bypassing_typecheck scoped (doc verbatim); jit→bỏ INT_NO_WILDCARD; lower_source strict giữ. Lower CODE không đụng, trap GAP-2 nguyên.

## Bài học D phiên này (sổ Nam Tào):
- **Lý lẽ regression SAI** (nhầm run_fixture với driver main.rs:59) — D tự nhận sau khi O verify.
- **Claim "CHỈ 3 test vỡ" KHÔNG đầy đủ** — sót test thứ 4 (match_trit_t6 non_exhaustive_trit trap) do grep-cụt/cache. D tự khai sau khi đo lại --no-fail-fast. **Mẫu grep-truncation tái diễn** — O phải tự vét cạn, không tin "đã đủ N". TIẾN BỘ: D self-disclose cả hai, không giấu; không xóa/ignore test giấu fail.

## ★ NỢ MỚI: typecheck-accept / lower-refuse Variable-catch-all — ĐANG ĐÓNG (WO phát 2026-06-20, G ký DRY helper)
- `match x {1=>10, other=>other}`: typecheck PASS (decision #2) nhưng lower REFUSE (lib.rs:3224). G đã ghi debt vào ADR-0064 §8:71 commit `d20b4b7`.
- **Recon O:** 3 path scalar đối xứng (Trit 2934/Trilean 3055/Integer 3171) — loop `Wildcard=>wildcard_arm`/`other=>refuse`; default_bb=wc.body|Trap. vars KHÔNG frame-scoped (push/pop_scope chỉ track owned_locals). Scalar Copy→không push_owned.
- **G ruling: DRY helper** `bind_scalar_catch_all(c,arena,catch_all,scrut_local,&scrut_ty,&span)` (mirror idiom 2734-2742: alloc+StorageLive+Assign từ scrut_local+vars.insert). Wiring 3 path: loop thêm `Variable(_)=>wildcard_arm=Some(arm)` + default block gọi helper sau push_scope trước lower body. Lower-ONLY, không ADR mới (đóng nợ §8).
- **Teeth:** fixture 222 Integer value-proof (`other => other*10`, EXPECT 110), 223 Trit + 224 Trilean routing-proof (EXPECT 21). Đỏ-trước (refuse "Variable") xanh-sau. Lát 2 docs đóng §8 debt.
- **Luật Thép nhắc D:** full gate raw (clippy), blast-radius --no-fail-fast KHÔNG grep-cụt (bài học sót test thứ 4).

## Variable-catch-all Lát 1 — O VERIFY MÁU + KÝ 2026-06-20. Commit `fa021b4` (4 file, ahead origin 1, chưa push).
- O đo độc lập: git show --stat = đúng 4 file (lib.rs + fixture 222/223/224), KHÔNG file lạ. lib.rs diff = helper verbatim + wiring 3 path đối xứng (Variable(_)=>wildcard_arm + bind_scalar_catch_all sau push_scope). Gate `0·0·219·0`. RUN 222→110 (other*10 value-proof), 223→21, 224→21. **Poison Integer Variable arm → 222 refuse "Variable(\"other\")" + 223/224 vẫn xanh** (3 path tách biệt, không vacuous, load-bearing). Khôi phục byte-identical Edit-revert.
- **Cờ minh bạch D:** 222/223/224 đã tồn tại untracked nội dung khác (draft EXPECT 17/99/42 — KHÔNG phải O tạo, nguồn bí ẩn). D ghi đè bằng WO spec (đúng — WO là spec ký), commit khớp WO. Resolved.
- **Lát 2 (chờ):** D đánh dấu ADR-0064 §8 debt-line (0064-...md:71) ĐÃ ĐÓNG + hash fa021b4, commit docs(adr) riêng.

## ✅ NỢ Variable-catch-all ĐÓNG TRỌN (code + docs). O ký cả 2 lát 2026-06-20.
- Lát 1 feat `fa021b4` (helper + 3 path + fixture 222/223/224). Lát 2 docs `5897aec` (đóng §8 debt-line :71, 1 file 1+/1−). O verify: stat đúng, poison Integer→222 refuse 223/224 xanh, gate 0·0·219·0.
- **ĐÃ PUSH 2026-06-20** (`d20b4b7..5897aec`, Gate B clean). origin/main = `5897aec` synced. Sạch bãi.

## TỔNG KẾT chuỗi 2026-06-19→20 (vai O suốt): Mục 4 Latent-Type (push) → Mục 1 Typecheck-Exhaustiveness (push) → Variable-catch-all (fa021b4+5897aec, chưa push). Backlog còn: Struct?/Enum? heap-nullable (ADR-0065 cần ADR-first) + return happy-path (đáy).
[[campaign_latent_type_inference]] [[mentor_o_persona]] [[colleague_d_persona]]
