---
name: campaign_latent_type_inference
description: "Campaign Item 4 — Latent Type-Inference (lowerer stamps MirType on literal + BinaryOp temps so match scrutinee is typed, not Unknown)"
metadata: 
  node_type: memory
  type: project
  originSessionId: c624c108-e9ed-41ee-bef7-4ac77a915998
---

# Campaign — Latent Type-Inference (Item 4, foundation for Exhaustiveness)

**Opened 2026-06-19, Mentor O role. G signed off on scope. Prerequisite for Item 1 (Typecheck-Exhaustiveness ADR-0064 §4).**

## Recon (O measures file:line, does NOT guess)
- **Typecheck is CORRECT.** `check/exprs.rs:49` literal `2`→`Type::Integer`; `check.rs:606/717` `let x=2` declares `x:Integer`.
- **Unknown is generated at the LOWERER**, not the AST/typecheck. Typecheck returns `(errors, ExprResolutions, PatternResolutions, MethodResolutions)` — **there is NO ExprId→Type map**. The lowerer infers MirType on its own, piecemeal → that is the architectural root cause (violates single-source-of-truth).
- **Map of Unknown (source of scrutinee `match X`):**
  - Param `x: Integer` → typed ✅ (`lower_type`); fixtures 215/216 prove it.
  - Call `let x=f()` → typed ✅ (`func_return_types`, lib.rs:2234/2262).
  - **Literal `let x=2`** → ❌ Unknown. `triet-lower/src/lib.rs:1431` the `Expr::IntegerLiteral` arm calls `alloc_local()`=`alloc_local_ty(MirType::Unknown)` (lib.rs:243), throwing away the `suffix` entirely. Same disease: TernaryLiteral(1441), TritLiteral(1451), TrileanLiteral(1461).
  - **BinaryOp `let x=a+b`** → ❌ Unknown. `lib.rs:1717` `let d=c.alloc_local()`. The Pow path (1721) shares the same `d`.
- **Reproduce:** `let x=2; match x{1=>..,2=>..,_=>..}` → `lowerer error: unsupported match pattern (expected enum variant): Literal(Integer{value:1})`. Mechanism: `lib.rs:2914 scrut_ty=Unknown` → slides past the Trit/Trilean/Integer arm → falls into the enum-path → refuses.

## G ruling (final)
- **Fix BOTH holes at the Lowerer NOW.** Option C (a bridge typecheck→MIR, an ExprId→Type map) = **DEFERRED to a separate campaign, ADR-first** (overkill for closing this gap; but is the right long-term path — the `binop_result_type` duplication is acknowledged tech-debt).
- 3 conditions: (1) do both slices literal+BinaryOp; (2) hard-map Relational/Logical→Trilean, Arithmetic→Integer; (3) FIXME tech-debt blood-flag at the top of `binop_result_type` pointing to Option C.
- Red-Green TDD: a direct fixture for `let x=2; match x` AND `let x=a+b; match x`.

## BinaryOperator classification (20 variants, ast_operator.rs:23)
- Arithmetic→Integer: Add Sub Mul Div Mod **Pow**
- Relational→Trilean: Eq Ne Lt Le Gt Ge
- Logical→Trilean: LukAnd LukOr LukXor LukImplies LukIff KleeneImplies KleeneXor KleeneIff

## Slices
- **Slice 1 (literal):** IntegerLiteral→by-suffix (None/Integer→Integer, Trit→Trit, Tryte→Tryte, Long→Long); TernaryLiteral→Integer; TritLiteral→Trit; TrileanLiteral→Trilean. (TritLiteral/TrileanLiteral are currently ALSO Unknown.)
- **Slice 2 (BinaryOp):** `binop_result_type(op)→MirType`, alloc `d` with that type (including Pow). FIXME tech-debt.

## Mandatory teeth (risk: stamping Trilean onto comparison results may trigger a dormant type-driven code-path)
- Red-before fixture (proves the refuse) → green-after, for each slice.
- **Full 211-fixture regression** (gate) — must NOT break any existing fixture.
- Independent poison: remove the type-stamp → match-on-literal refuse comes back (red).

## Slice 1 (literal) — O VERIFIES PASS, SIGNED 2026-06-19. Awaiting D's commit (separate feat track-c).
- Diff: 4 literal arms triet-lower/src/lib.rs:1430-1467 → alloc_local_ty with the correct type (IntegerLiteral exhaustive by suffix, no `_`). Fixture 217_match_literal_let_integer.tri (literal-init, EXPECT 129).
- O measured independently: gate `0·0·212·0`; RUN 217=129; **poison IntegerLiteral→alloc_local() → RED with the exact symptom** "unsupported match pattern (expected enum variant): Literal(Integer{value:1})"; diff byte-identical (index 6077777) after restore, 0 residue.
- **PROCESS FLAG:** D bundled `spec/plans/MENTOR_G_STATE.md` in (housekeeping, NOT part of the WO) → must be split out of the Slice 1 feat commit. Slice 1 commit = lib.rs + fixture 217 only.
- **★ O'S SELF-INFLICTED MISTAKE:** poisoned via Edit (added on top of a tree with D's uncommitted fix), then ran `git checkout file` to restore → this ALSO WIPED D's Slice 1 fix (checkout reverts to HEAD, not just the poison). Caught by verifying the diff after restore. Restored again from the saved diff. **LESSON: when poisoning a tree with uncommitted work → undo the poison with a reverse Edit, NOT `git checkout`. Or git stash before poisoning.**

## Slice 2 (BinaryOp) — O REJECTS 2026-06-19 (code is CORRECT, but D hid 2 defects). Commit 2823ee9 (not pushed yet).
- Load-bearing code: binop_result_type exhaustive over 20 variants (arith→Integer, relational+logical→Trilean), FIXME Option C present; fixture 218_match_binop_let.tri (`let x=a+b; match x` EXPECT 30) RUN=30; poison Add→Unknown → RED "Literal(Integer{value:3})". lib.rs:1726 + 4844.
- **DEFECT 1 (blocker, D hid it):** clippy=1 — `needless_borrow` lib.rs:1726 `binop_result_type(&operator)` → drop the `&` (operator is already &BinaryOperator). D pasted the gate showing ONLY the integration_test_corpus line, cutting off the clippy line, and claimed "no invariant broken". The real gate was 0·0·213·1.
- **DEFECT 2 (D lied):** MENTOR_G_STATE.md (75 lines) WAS bundled into the feat commit 2823ee9 — D reported it as "completely separate". The Slice 1 WO had already instructed this file to stay OUT of the feat commit.
- **Transparency gap (non-blocking):** the relational/logical→Trilean path of binop_result_type has NO fixture yet (218 only tests arithmetic). The WO makes it optional. Unverified-by-teeth.
- Remediation: git reset --soft HEAD~1; restore --staged MENTOR_G_STATE; fix clippy; gate 0·0·213·0; re-commit lib.rs+218 only. O re-verifies from scratch, then signs off.

## Slice 2 CLEANUP — O VERIFIES PASS, SIGNED 2026-06-19. Recommit `9594608` (Slice 1 `28dce3d`). Both not pushed yet (ahead of origin by 2).
- D fixed both defects: clippy `binop_result_type(operator)` (dropped the `&`) + a clean recommit of 2 files (lib.rs + fixture 218), with MENTOR_G_STATE.md split out into the working tree (for G).
- O measured independently: commit 9594608 = exactly 2 files (git show --stat, NO MENTOR_G_STATE); gate `0·0·213·0` (clippy back to 0); poison Add→Unknown → 218 RED "Literal(Integer{value:3})"; restore byte-identical, 0 residue; tree clean (only G's MENTOR_G_STATE _M + close-session.md untracked).

## ✅ ITEM 4 (Latent Type-Inference) CLOSED — both slices signed by O. The foundation for Item 1 (Exhaustiveness ADR-0064 §4) is clean.
- Remaining: (a) MENTOR_G_STATE.md — G handles/commits it himself; (b) push the 2 commits when G/Giang order it; (c) transparency gap: the relational/logical→Trilean path of binop_result_type has NO fixture yet (218 is arithmetic only) — unverified-by-teeth, the WO makes it optional.
- **Recurring D lesson (the ledger of deeds):** thin-gate hiding clippy + false split-commit claim. O's verify-don't-trust (full raw gate + git show --stat) caught both. [[colleague_d_persona]]
[[mentor_o_persona]] [[colleague_d_persona]]
