---
name: campaign_typecheck_exhaustiveness
description: "Campaign Item 1 — Typecheck-Exhaustiveness compile-time (match missing Integer/Trilean/Trit arms → E1026, closes ADR-0064 §4 debt)"
metadata: 
  node_type: memory
  type: project
  originSessionId: c624c108-e9ed-41ee-bef7-4ac77a915998
---

# Campaign — Typecheck-Exhaustiveness (Item 1, closes ADR-0064 §4 debt)

**Opened 2026-06-19 after Item 4 (Latent Type-Inference) closed. Role: Mentor O. G signed off on 5 decisions + ADR-0064 §8.**

## Recon (O measured file:line)
- Gap: `triet-typecheck/src/check/exprs.rs:1728 check_match()` dispatches exhaustiveness for Outcome(:1784)/Nullable(:1789)/Enum(:1794), **MISSING the scalar branch** → :1797 swallows it. This is GAP-2 (ADR-0064 §4 runtime-trap instead of compile-error).
- Code E1026 already exists at `error.rs:399` — the "1 code, multiple variants" mold (NonExhaustiveOutcomeMatch/EnumMatch). Span-extract :928.
- Reusable wildcard-detect mold: check_nullable_exhaustiveness(:1861), check_enum_exhaustiveness(:1904).
- Pattern model (pattern.rs:13/79): catch-all = Wildcard|Variable(name); Integer arm = Literal(Integer{suffix:None}); Trit = Literal(Integer{suffix:Some(Trit)}) values -1/0/1; Trilean = Literal(Trilean(True/False/Unknown)); Or expands; Range does NOT satisfy Integer.
- Type: Type::Integer, Type::Trilean{refined:_} (both), Type::Trit (types.rs:13).
- **Blast-radius: ZERO fixtures break** — 215/218 already have Integer-with-`_`, 174/214 Trit-complete-3, 216 Trilean-complete-3, all already exhaustive. No fixture EXPECTS a scalar runtime-trap.

## G's ruling (5 decisions, ADR-0064 §8 amended with a visible trail)
1. Reuse E1026 + variant NonExhaustiveScalarMatch{missing,span}. NO new code.
2. Catch-all = Wildcard `_` OR Variable (bind `other =>`).
3. **The GAP-2 trap in lower STAYS AS-IS — removal FORBIDDEN (G: hands off). Defense-in-depth.**
4. Amend ADR-0064 §8 (no new ADR; 0065 is reserved for Struct?/Enum?).
5. Tryte/Long DEFERRED (recorded as debt; lower doesn't support match for them yet).

## Campaign = TYPECHECK-ONLY. Lower is NOT touched.
- Slice 1: commit ADR-0064 §8 (already drafted) separately as `docs(adr)`.
- Slice 2: error.rs variant NonExhaustiveScalarMatch + check_scalar_exhaustiveness (Integer→needs catch-all; Trilean/Trit→needs all 3 faces or a catch-all) called from check_match after the enum dispatch.
- Teeth: 3 fixtures RED E1026 (integer-no-wildcard, trilean-missing, trit-missing) red-before-green-after + green regression + 1 GREEN fixture Variable-catch-all.
- Iron rules: full raw gate (including all clippy lines), separate commit per slice with no stray files, clippy 0, do NOT remove the trap.

## Progress
- **Slice 1 CLOSED** `7bb54fa` (ADR §8, 1 clean file).
- **Slice 2 code DONE (uncommitted), O verified clean design:** error.rs NonExhaustiveScalarMatch{missing,span} E1026 + span-extract; exprs.rs dispatch after enum (Integer/Trilean/Trit) + has_scalar_catch_all(Wildcard|Variable) + collect_literal_patterns(Or-expand recursive) + 3 helpers. Lower NOT touched. Fixtures 219/220/221 ERROR E1026.

## 2 forks where D stopped-to-ask (Laws 4+5), O verified independently + ruled:
- **Regression in 3 tests of match_literal_t.rs** (old match-on-literal campaign): used INT_NO_WILDCARD (deliberately non-exhaustive to test the lower trap) → now E1026 → helper `lower_source:43 assert type_errors.is_empty()` blows up. **D's claim "the real driver keeps lowering past the type-error" = WRONG** (O verified main.rs:59 driver returns ExitCode(3) on type-error, does NOT lower — BLOCKING). **O's RULING = HYBRID (not D's global-relax Option A):** test #1 case-maps + #3 jit → change INT_NO_WILDCARD→INT_WITH_WILDCARD (keep strict); test #2 trap → scoped helper `lower_bypassing_typecheck` (keeps INT_NO_WILDCARD), `lower_source` stays strict as-is. Reason: global-relax would lose the type-clean teeth for every test. Fixing the tests is in-scope; lib.rs lower CODE is untouched, the trap is intact.
- **Fixture 222 Variable-catch-all → typecheck test:** O verified lib.rs:3224 lower REFUSES Variable in an Integer match → `// EXPECT` fixture is impossible. **APPROVED** replacing it with a typecheck unit test (decision #2 belongs to the typecheck layer). D probed-before-reporting correctly.

## ★ NEW DEBT (made transparent, surfaced from 222): typecheck-accept / lower-refuse for Variable-catch-all
- `match x {1=>10, other=>other}`: typecheck PASSES (Variable=catch-all per ADR-0064 §8 #2) but lower REFUSES (lib.rs:3224 only accepts Wildcard+literal). A typecheck-clean program → blows up at lower. **Loud-fail, not silent-wrong**, outside the typecheck-only scope. Deferred: lower should bind the scrutinee→variable in the default block. Ask G whether to record this in the ADR §8.

## ✅ ITEM 1 CLOSED — O VERIFIED IN BLOOD + SIGNED OFF 2026-06-20. Slice 1 `7bb54fa` (ADR §8) + Slice 2 `57021c0` (code). 2 local commits, NOT YET pushed (ahead of origin 8e41129 by 2).
- **O measured independently:** git show --stat 57021c0 = exactly 8 files (error.rs, exprs.rs, fixtures 219/220/221, match_scalar_exhaustiveness.rs, match_literal_t.rs, match_trit_t6.rs) — NO ADR/MENTOR_G_STATE/close-session. Full gate `0·0·216·0`. 3 binaries: match_scalar 5/5, match_literal 5/5, match_trit 3/3 (no ignore/filter). **Poisoning the dispatch neuter → 3 typecheck negative tests FAILED + fixtures 219/220/221 swallow E1026** → load-bearing; restored byte-identical via Edit-revert (NOT checkout — an old lesson).
- **O personally swept the full blast-radius (both D and O's earlier recon had missed spots):** scanned EVERY test source for scalar-match → candidate #5 trilean_refined_annotation consume_plain is EXHAUSTIVE (safe); counting/heap = Outcome/nullable (not scalar). Blast-radius = 4 tests (3 literal + 1 trit), the hybrid covers all of them.
- **Hybrid (O's ruling, NOT a global-relax):** case→INT_WITH_WILDCARD strict; trap→scoped lower_bypassing_typecheck (documented verbatim); jit→drop INT_NO_WILDCARD; lower_source stays strict. Lower CODE untouched, GAP-2 trap intact.

## D's lesson this session (the ledger of reckoning):
- **Regression argument was WRONG** (confused run_fixture with driver main.rs:59) — D admitted this after O verified.
- **The claim "ONLY 3 tests break" was INCOMPLETE** — missed a 4th test (match_trit_t6 non_exhaustive_trit trap) due to a truncated grep/cache. D self-disclosed after re-measuring with --no-fail-fast. **The grep-truncation pattern recurs** — O must personally sweep exhaustively, not trust "already enough N". PROGRESS: D self-disclosed both issues, didn't hide them; didn't delete/ignore a failing test to hide it.

## ★ NEW DEBT: typecheck-accept / lower-refuse Variable-catch-all — CLOSING IN PROGRESS (WO issued 2026-06-20, G signed off on DRY helper)
- `match x {1=>10, other=>other}`: typecheck PASSES (decision #2) but lower REFUSES (lib.rs:3224). G already recorded the debt into ADR-0064 §8:71 commit `d20b4b7`.
- **O's recon:** 3 symmetric scalar paths (Trit 2934/Trilean 3055/Integer 3171) — loop `Wildcard=>wildcard_arm`/`other=>refuse`; default_bb=wc.body|Trap. vars are NOT frame-scoped (push/pop_scope only tracks owned_locals). Scalar Copy→doesn't push_owned.
- **G's ruling: DRY helper** `bind_scalar_catch_all(c,arena,catch_all,scrut_local,&scrut_ty,&span)` (mirrors idiom 2734-2742: alloc+StorageLive+Assign from scrut_local+vars.insert). Wiring 3 paths: loop adds `Variable(_)=>wildcard_arm=Some(arm)` + default block calls the helper after push_scope before lowering the body. Lower-ONLY, no new ADR (closes the §8 debt).
- **Teeth:** fixture 222 Integer value-proof (`other => other*10`, EXPECT 110), 223 Trit + 224 Trilean routing-proof (EXPECT 21). Red-first (refuse "Variable") green-after. Slice 2 docs close the §8 debt.
- **Iron Rule reminder to D:** full raw gate (clippy), blast-radius --no-fail-fast with NO truncated grep (lesson from the missed 4th test).

## Variable-catch-all Slice 1 — O VERIFIED IN BLOOD + SIGNED OFF 2026-06-20. Commit `fa021b4` (4 files, ahead of origin by 1, not yet pushed).
- O measured independently: git show --stat = exactly 4 files (lib.rs + fixtures 222/223/224), NO stray files. lib.rs diff = the helper verbatim + wiring across 3 symmetric paths (Variable(_)=>wildcard_arm + bind_scalar_catch_all after push_scope). Gate `0·0·219·0`. RUN 222→110 (other*10 value-proof), 223→21, 224→21. **Poisoning the Integer Variable arm → 222 refuses "Variable(\"other\")" while 223/224 stay green** (3 paths are separate, not vacuous, load-bearing). Restored byte-identical via Edit-revert.
- **D's transparency flag:** 222/223/224 already existed untracked with different content (draft EXPECT 17/99/42 — NOT created by O, source unknown). D overwrote per the WO spec (correct — the WO is the signed spec), commit matches the WO. Resolved.
- **Slice 2 (pending):** D marks ADR-0064 §8 debt-line (0064-...md:71) as CLOSED + hash fa021b4, commits docs(adr) separately.

## ✅ Variable-catch-all DEBT FULLY CLOSED (code + docs). O signed off on both slices 2026-06-20.
- Slice 1 feat `fa021b4` (helper + 3 paths + fixtures 222/223/224). Slice 2 docs `5897aec` (closes §8 debt-line :71, 1 file 1+/1−). O verified: stat correct, poisoning Integer→222 refuses, 223/224 green, gate 0·0·219·0.
- **PUSHED 2026-06-20** (`d20b4b7..5897aec`, Gate B clean). origin/main = `5897aec` synced. Ledger clean.

## SUMMARY of the 2026-06-19→20 chain (role O throughout): Item 4 Latent-Type (pushed) → Item 1 Typecheck-Exhaustiveness (pushed) → Variable-catch-all (fa021b4+5897aec, not yet pushed). Backlog remaining: Struct?/Enum? heap-nullable (ADR-0065 needs ADR-first) + return happy-path (bottom of the stack).
[[campaign_latent_type_inference]] [[mentor_o_persona]] [[colleague_d_persona]]
