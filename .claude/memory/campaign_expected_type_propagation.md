---
name: campaign_expected_type_propagation
description: "ADR-0072 Expected-Type Propagation in AST→MIR lowering — 🔒 SEALED and pushed to origin/main 3d7618f. It killed the cancer seed of using c.sig.return_type as a global proxy, replacing it with an explicit `expected: Option<&MirType>`. 3 slices. It unlocked functions returning T?. READ THIS if you touch the lower_expr signature / OutcomeConstructor / NullLiteral / the ~+/~0/~- constructors / nullable returns / if-match-block forwarding."
metadata: 
  node_type: memory
  type: project
  originSessionId: bb3e8b29-d3f8-402a-908d-36cd844a8e9a
---

**ADR-0072 — Expected-Type Propagation (🔒 SEALED 2026-06-27, pushed to origin/main `3d7618f`, gate `0·0·303·0`).**

## The origin was a WRONG diagnosis in the handover ledger
The ledger recorded the blocker as "a match arm binding a heap payload move-out → `lowerer does not support Identifier`". O's recon (a probe matrix) proved it wrong on two levels:
1. **A name collision:** the test function was called `get`, colliding with the Vector/HashMap builtin free function (`lib.rs:2220`); a 0-argument call → `unsupported_expr(callee)` printing `Identifier{name:"get"}`. Renaming `get`→`fetch` made the error evaporate. **`match` move-out on an Outcome `T~E` ALREADY WORKED** (fixtures 113/139/142).
2. **The real enemy:** functions returning `T?` (nullable) could not be lowered → `OutcomeAlloc on non-Outcome type`. **LESSON: verify-don't-trust applies to the recon written in the LEDGER, not just to other people's recon.**

## The root defect (the cancer seed, as G named it)
`OutcomeConstructor`/`NullLiteral` decided which lowering path to take (an Outcome StackSlot versus a PA-3c nullable) by reading `c.sig.return_type` — **a GLOBAL variable used as a proxy**. It is wrong whenever the local context differs from the return type (a nullable let, a nullable field, a function returning `T?`). The 3 old bolt-on redirects (let `:1314`, struct field `:2986`, `~0` is_null `:884`) were only local patches that STRIPPED the `~+` before the constructor. **There is NO `NullableAlloc`** — a present nullable is identity (scalar) or widening (aggregate), and null is NULL_SENTINEL (correcting G's framing).

## The solution (G chose an explicit parameter and REJECTED an implicit context)
`lower_expr(expr, expected: Option<&MirType>, arena, c)`. 3 slices, each verified INDEPENDENTLY with blood by O (byte-identical output + a red poison + a structural grep), and co-signed by G one slice at a time:
- **Slice 1** `c9a46e6` — add the parameter, 61 call sites pass `None`, byte-identical (an empty MIR diff across the whole corpus, against a worktree baseline).
- **Slice 2** `2c900fb` — leaf consumers read `expected` (with the §2.5 transitional fallback `unwrap_or(sig.return_type)`); wire 4 sources (body tail / return / let init / struct field); smash the 3 redirects (KEEPING the widening block). This unlocked scalar `T?` returns (303/305). 2 poisons of `OutcomeAlloc on non-Outcome` went red. **Defence in depth: 2 guards (the Nullable arm + the non-wrapper check) — removing one still leaves it red.**
- **Slice 3** `3d7618f` — transparent forwarding of `expected` down into block tails, if/then/else, and 13 match-arm bodies (but NOT scrutinees or conditions); **the §2.5 fallback removed entirely**; **`c.sig.return_type` ripped out of the constructor's input** (leaving only the 4 legitimate return-position sources + 1 reference-form check); `emit_outcome_zero` extracted. 306/307/308 unlocked (context ≠ signature), **309 is a negative fixture locking the rule "an untyped `let r=~+5` is REFUSED"**, and 157 was annotated (a semantic fix; the intent of ADR-0055 Bug A is preserved). The diagnostics became general (no more saying "~0 null" for `~+`/`~-`). 3 R-forwarding poisons went red.

## The closing evidence (a masterpiece)
157 UNTYPED (running through the cancerous fallback) versus 157 ANNOTATED (running through the explicit source) → **byte-identical MIR, byte for byte**. The heart was replaced without the patient noticing. D's scope extension (8→13 arms, 2→4 sources) was validated by 299/299 staying byte-identical (any error would have broken more than one fixture).

## The debt carried forward (a red flag) — ✅ CLOSED (WO-0073, `3738eb5`, signed by G 2026-06-29)
~~🔴 heap-nullable-return drop glue~~ → **UPROOTED.** `heap_nullable_return_present_counting.rs`, 7 cells. 2 shapes of a `~+ <heap>` present return: **an expression body** (A/B/C/D) + **a named local with an explicit return** (E/F/G). O verified independently with blood: the leak tooth → 7/7 RED with FREE→0; the double-free tooth (removing M4 at 1982) → E/F/G RED with FREE→2, while A/B/C/D stayed INERT at 1. **The architectural truth:** an expression body is the lowerer **escaping by omission** (the callee emits NO Drop → a double free is impossible → the M4 tooth is INERT); a named local goes through `flush_all_for_return`, which emits the Drop(s) → **M4 is load-bearing**. **Lesson: verify-don't-trust cuts into O's own WO** — the original double-free tooth spec was wrong (it assumed M4 guarded expression bodies), D caught it, and the scope was extended by 3 cells (G approved); G then made the doc comment fixed over 2 rounds (the text must equal the architectural truth exactly). [[campaign_truc_b_heap_in_aggregate]]

## The ground is now clean for
`match call_returning_T?(){~+ s=>… ~0=>…}`, `if c {~+v} else {~0}`, and a block-final `{~+v}` in every value context. Capability Ł3 (ADR-0069, [[campaign_capability_luk3]]) is still outstanding.
[[feedback_verify_producer_before_consumer]] [[mentor_o_persona]] [[colleague_d_persona]]
