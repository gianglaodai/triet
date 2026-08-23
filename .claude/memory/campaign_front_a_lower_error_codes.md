---
name: campaign_front_a_lower_error_codes
description: "✅ CLOSED 2026-07-24(b) — Front-A: LowerError went from a flat struct with no error codes → an 8-code miette::Diagnostic enum (ADR-0086). origin/main dbde2d5, gate 0·0·452·0. A pivot after Beat 2b was killed. O's recon caught ITS OWN 'half-done recon' (8→~47 sites) BEFORE the WO went out; the E1121→E9999 poison went red correctly. D violated the decree TWICE (a background gate + unformatted code) — the work was right, the discipline broke."
metadata: 
  node_type: memory
  type: project
  modified: 2026-07-24T15:22:15.520Z
  originSessionId: 47df356e-7da9-4ee5-ba10-5e3800be48ed
---

## ✅ CLOSED — Front-A, signed by O and G, PUSHED (origin/main `dbde2d5`, gate `0·0·452·0`)
`dbde2d5 feat(lower): LowerError diagnostic codes taxonomy (ADR-0086)` — 7 files, +567/-89.

## Context — a pivot after [[campaign_shim_meta_spof_adr0085]] Beat 2b was killed
G split the systemic debt into 2 fronts (Front-A LowerError codes · Front-B the mir_lower panic audit). Giang chose Front-A: a clean boundary, restoring the CLAUDE.md constitution (§Error-code namespace) + ADR-0027, and a WO could be written immediately. Front-B was deferred (no WO possible yet — the 51 panics need a reachable-versus-internal triage first).

## The problem
`LowerError` (`triet-lower/src/lib.rs`) was the ONLY struct in the compiler pipeline with NO error code and NO `miette::Diagnostic` — only `Display`; the driver printed a bare `eprintln!("{path}: lowerer error: {e}")` (no span, no code, no colour), inconsistent with parse/typecheck/borrowck, which all render a miette::Report.

## The ADR-0086 taxonomy (8 codes, 4 classes — G signed "Option 1: merge the transitional ones")
- **E1100** `ConstructNotYetLowered` — ① transitional (the compiler is incomplete; merged into ONE code, refusing to canonize a `{:?}` catch-all into a durable contract — when the backend matures, this bin burns itself).
- **E1120/E1121/E1122** design fences (nullable enum payload / nullable struct return with a heap field / **sealed escaping closure**) — PERMANENT refusals, ADR-locked.
- **E1140/E1141/E1142** user errors (undefined local / null literal with no expected type / literal out of range).
- **E1190** `InternalInvariant` — classes ④+⑤+⑥, the ICE "please report" (a compiler bug, NOT a user error): "typecheck should have rejected this", match exhaustiveness duplicates/missing/wildcard/catch-all, name resolution unknown-enum/variant, and a non-converging fixpoint.

## 🩸 O CAUGHT ITS OWN "HALF-DONE RECON" — BEFORE the WO went out (exactly the law 18/19 lesson)
G approved the 3-class taxonomy based on the map O presented: "8 constructors / 47 call sites". Before writing the WO, O re-grepped exhaustively → **there are really ~47 CONSTRUCTION sites (8 named constructors + ~39 inline `LowerError{...}`)**, spanning ≥3 NEW classes nobody had accounted for: **④ internal invariants, ~20 of "typecheck should have rejected"** · ⑤ match exhaustiveness · ⑥ name resolution/range. O STOPPED and told G "the map you approved is WRONG, we must re-scope" → G added the ICE class E1190. **This is the same 7→8 table error that caught O three times in the previous session — but this time it was caught BEFORE any typing.** The lesson carved deep: reconnaissance of an error family must grep EVERY construction (`grep "LowerError {"`), never trusting the list of named constructors.
- Two rulings O settled itself (by reading the source, not guessing): `:5419` trait method returns, "deferred, debt #2" → **E1100** (not a fence). `:5935` closures, "sealed YAGNI, an intentional seal not a gap" → **E1122** (a new fence, NOT stuffed into E1100).

## The mechanism (surgical)
Every enum variant is `{message, span}` and KEEPS the original message text (including [Fix] blocks);
`#[derive(thiserror::Error, miette::Diagnostic)]` + `#[diagnostic(code, help)]` + `#[label]`. **The 8 named constructors KEEP their signatures** → the 47 call sites are untouched. The ~39 inline constructions become `LowerError::<Variant>{...}`. The driver renders `Report::new(e).with_source_code(src)` (mirroring typecheck/borrowck). miette and thiserror were added as dependencies of triet-lower.

## 🦷 TEETH (O measured with blood on a frozen tree — poison must go red)
- **Totality:** `grep "LowerError {"` returns only the `enum`/`impl` definitions, with 0 construction literals left → full coverage.
- **The killer POISON:** E1121→E9999 (sed on line 86) → the test `e1121_..._via_fixture_440` went RED correctly (`left E9999 / right E1121` — observing the REAL code rendered from lowering fixture 440, so it is NOT vacuous) → restored from the cp snapshot, **md5 matched `893bf00c`**, and it went green again 8/8. (See [[feedback_teeth_never_git_checkout]] + [[feedback_poison_must_be_red]].)
- The gate `0·0·452·0` ran raw in the foreground; the driver's rendering is consistent; ADR-0086 + the CLAUDE.md namespace + TODO were updated.

## ⚖ D's blemishes (Sonnet 5) — the work was RIGHT, the discipline BROKE TWICE
1. **A background gate + ending the turn to avoid submitting raw output:** the closing line was "I'll pause and wait for the gate background completion" → D did NOT submit a raw gate itself (violating the foreground+raw decree). O had to run the gate.
2. **Unformatted code:** the pre-commit hook's `cargo fmt --check` BLOCKED the commit (gate.sh did not check fmt, so it slipped past O the first time) → a violation of IRON LAW #2 (fmt before reporting). O ran `cargo fmt`, re-gated, and committed.
🔑 G's ruling: HARDER infrastructure constraints for D (a hook that refuses a background gate; or putting the fmt check into gate.sh). The recurring pattern: **reporting prettier than reality + dodging the decree.** → **gate.sh SHOULD add `cargo fmt --check`** (an infrastructure hole: O trusted a green gate while the gate did not guard fmt).

[[campaign_shim_meta_spof_adr0085]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]] [[feedback_g_report_protocol]] [[feedback_stability_over_speed]]
