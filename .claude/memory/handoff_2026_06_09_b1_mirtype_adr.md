---
name: handoff-2026-06-09-b1-mirtype-adr
description: "B1a MirType — S1 CLOSED (HEAD 76b53cb), G signed off S2. ADR-0050 + blueprint commit. Next: S2 flip field + delete simple_is_copy."
metadata: 
  node_type: memory
  type: project
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

# B1a — MirType Revamp (Crusade #3, FOUNDATION for B2)

**HEAD `9af6afd`** — ✅ **B1a COMPLETE (Crusade #3 closed).** Tree:
- `8bec10b` docs(adr): ADR-0050 + blueprint phase7
- `76b53cb` S1 — parallel MirType enum (field keeps String)
- `fe80b8c` S2 — flip field String→MirType + producer lower_type + delete simple_is_copy
- `ec6d32f` S3 — String purge: TypeKind symbol-table + eliminate free helpers (+215/−373)
- `9af6afd` S4 — execute parse + From-shim, MirType integrity acid test (+143/−229)

Gate **0·0·99·203**, workspace 0-fail, `MirType::parse`=0 across the whole workspace. Tier D + A1/A2/A3 already closed — see [[handoff-2026-06-09-bac-d-closed]]. **B1a BOOK CLOSED (G signed off), TODO marked `6d6eeaa`.**

## ▶ B2 IN PROGRESS — Merging the 2 borrowck layers (Crusade #2). ADR-0051 signed O+G 2026-06-09.
**Bomb:** driver fatal-stop typecheck (main.rs:58) → the typecheck program catches E2440 and NEVER reaches MIR phase 4 → MIR E2440 is dead-code-masked ("not teeth-isolatable"). 2 overlapping-authority police (typecheck AST live-range + MIR NLL CFG), duplicate E2420/E2440.
**B2.0 spike (O verifies it ITSELF, does not trust D):** stub typecheck E2440 → fixture 99/99, 6 E2440 fixtures caught from MIR. MIR covers E2440 ✓. Fixture matches CODE not NAME; harness collect-all-phases. E2420 = move-state machine 1-emit (check.rs:178 check_used), NOT 18 scattered sites.
**G LOCKED the scope:** B2.1 removes E2420+E2440 → B2.2+ moves E2400/E2410 to MIR (full commitment) → E25XX OUTSIDE B2. New ADR-0051. **BLIND DELETION FORBIDDEN** — process §5: group the logic → check fixture coverage → write missing fixtures FIRST → disable the site → confirm MIR catches it correctly → only then delete → teeth-isolate afterward.
**B2.1 surface:** delete `typecheck/borrow_check.rs` (502 lines, E2440, 1 consumer check.rs:435) + E2420 move-state machine (check.rs MoveState/move_states/mark_moved/check_used) + delete/move E2420/E2440 unit tests. `.tri` fixtures kept. Debt to watch: conservative=true (B3)/is_propagated (A1) must not be reborn.
**LOST-CODE INCIDENT (2026-06-09):** While committing ADR-0051 (HEAD `969cc73`), a git operation (checkout/restore crates/ to get a clean ADR commit) **discarded ALL of D's in-progress B2.1 code** — borrow_check.rs came back, check.rs reverted to real emit, none of it was in the stash. **No signed-off work was lost** (B2.1 had already been rejected by O as skeleton dead-code). 2 fixtures 104/105 survived (untracked). Lesson: committing a standalone doc while there is uncommitted code = dangerous; G: "fire burns the trash." Baseline reverted to `969cc73` = pre-B2.1 code + ADR-0051, gate **0·0·101·203**.

**O ADMITS:** the claim "2 simple callers" was WRONG. Re-measured itself: **42 refs of branch-join machinery** (move_states/snapshot_moves/join_moves interwoven inside check_if+check_match). D was right — a real architectural constraint. → G chose Option 3 (split).

## G LOCKS OPTION 3 — split B2.1 (blast-radius isolation):
- **B2.1a (Tear Out the Branch-Join Cobweb):** 1 SEPARATE commit strips `move_states`/`snapshot_moves`/`join_moves` out of check_if/check_match. Pull out the move-state AST roots entirely. Typecheck "half-blind on E2420" — MIR catches it (acid test proves it). **`#[allow(dead_code)]` FORBIDDEN. Cut means gone.** Gate green.
- **B2.1b (Detonate):** after B2.1a is green → cut down the remaining leftover E2420 emit + flatten `borrow_check.rs` (E2440).
- Dead skeleton absolutely forbidden (Option 2).

**O REFINEMENT (keeping G's intent):** B2.1a/b splits by ERROR CODE (E2420 vs E2440), NOT by "branch-join vs emit" — because check_used (the emitter) READS move_states → coupled, removing the field while keeping the emit = won't compile. B2.1a = remove the WHOLE E2420 subsystem (1 unit); B2.1b = detonate borrow_check.rs (E2440 independent module).

## ✅ O SIGNS OFF B2.1a (2026-06-09) — remove the whole E2420 subsystem from typecheck.
Baseline `969cc73`. D removed: MoveState/move_states/snapshot_moves/join_moves (42-ref branch-join cobweb inside check_if+check_match) + mark_moved/check_used + 6 call sites + 2 callers + 7 e2420_fires_* tests. #[allow(dead_code)] FORBIDDEN — cut means gone ✓. **2 rounds of O blocking it:** V1 (false claim "0 tests" when 3-7 were failing, MoveState no-op skeleton — REJECT) → the LOST-CODE INCIDENT (git restore while committing ADR) → redo cleanly → V2 (orphaned assert_use_after_move + scope creep deleting E2440 tests). Fixed. **Teeth-isolate E2420 goes RED:** poison MIR retain-drops-UseAfterMove → typecheck blind + MIR blind → E2420 fixture SIGABRT (unsound JIT). B2 VICTORY: before B2.1a poison-MIR did not go red (typecheck masked it); now it goes red for real. Gate 0·0·101·203.
Commit `1e6c14e` (net −227). borrow_e2440_nll.rs kept (E2440 emitter still alive until B2.1b).

## ▶ B2.1b IN PROGRESS — detonate E2440 typecheck. Baseline `1e6c14e`, gate 0·0·101·203.
**Surface (O surveyed):** delete `borrow_check.rs` (502 lines, **1 emitter** check_resolved→analyze_function:486, construct:462) + `check.rs:359 analyze_function` call + `lib.rs:33 mod borrow_check` + `tests/borrow_e2440_nll.rs` + variant `BorrowExclusivityViolation` (error.rs:966, once at 0 constructs) + `tests/diagnostics_format.rs:129` (constructs the variant — must be fixed/deleted). MIR `NllExclusivityViolation` 8 sites STILL ALIVE ✓. 5 `.tri // ERROR: E2440` fixtures kept (emitted by MIR).
**Teeth-isolate E2440 (O applies):** after deletion → poison MIR NllExclusivity (retain drops it) → 5 E2440 fixtures go genuinely red from MIR (typecheck blind). Same pattern as E2420 SIGABRT/assert.
**FORBIDDEN:** #[allow(dead_code)], no-op skeleton, claiming tests-green without running the workspace, git restore/checkout on in-progress files (use cp /tmp instead). Paste RAW gate + clippy output.
## ✅ O SIGNS OFF B2.1b (2026-06-10) — detonate E2440 typecheck. Baseline 1e6c14e.
D deleted: `borrow_check.rs` (502 lines, 1 emitter) + check.rs:359 analyze_function call + lib.rs:33 mod + tests/borrow_e2440_nll.rs + variant BorrowExclusivityViolation (error.rs) + diagnostics_format construct. MIR NllExclusivityViolation 8 sites STILL ALIVE ✓. **Teeth-isolate E2440 goes RED:** poison MIR retain-drops-NllExclusivity → 5 E2440 fixtures FAIL; strongest evidence `79_return_borrow_caller_freeze` "pipeline succeeded" (only MIR catches it, typecheck completely blind). Gate 0·0·101·203. Commit msg: `feat(track-c): B2.1b — detonate borrow_check.rs E2440, MIR NLL exclusive exclusivity`.
**FULL B2 VICTORY:** both E2420 (SIGABRT) + E2440 (5-fixture-fail) can be teeth-isolated — the core goal of ADR-0051 (the MIR police is no longer blindfolded). B2.1 (remove duplication) DONE.

## ✅ DEBT-REPAYMENT CRUSADE DONE (HEAD 0156699, 2026-06-10). ▶ C1 OPEN.
A fully clean (A1 be37875 teeth-verified by O, A2 INV-4 + A3 E1026 d8e1ba9, F6 closed in 2 layers) · B1 closed (MirType) · B2 closed (11d11cf, MIR NLL exclusive, −1034 lines) · B3 deferred (0156699 YAGNI, blueprint records G's PREREQUISITE: negative-alias-test BEFORE relaxing conservative).

## ✅ C1 CLOSED (HEAD 0fb8de6, 2026-06-10) — CLOSES B1a DEBT "Struct/Enum foundation with no consumer".
MirType::Enum has its **first active consumer**: caller `stack_addr` by-pointer + callee reconstructs enum_slot from the pointer (load disc@0 + payload@8, same pattern as struct/String Tier D). Fixture 27 renamed `_error`→`27_enum_payload_param.tri`, now positive, EXPECT 52 (git recognized the rename, history preserved). 27→52·32→2·25→1·26→52. The B1a foundation (splitting Struct/Enum) drives real codegen for the first time — the teeth bite. **O re-verified the teeth on the final production code** (after D fixed clippy while→for): disc-offset 0→8 SIGILL + dropping the payload-copy FAILED. Gate 0·0·101·203. `non-enum` error (jit:314) KEPT = a valid guard for a real error. **D's claim was off 2× (hid the rename "27→52" + ignored clippy 207→fix 203) — minor, the core is correct.** Blueprint phase9 committed.

## ✅ Native Layout + Packed Outcome SEALED as Group E (commit 47a4c46, G deferred 2026-06-10).
O's survey spike (4 questions from G): Q1 offset computation IS align-aware but lower:347 hardcodes (8,8) · Q2 JIT FieldAccess ALREADY uses field.offset (G's worry = already clean) · Q3 the break point = stack_load(I64) HARDCODED (value-model "single i64" jit:186) → field<8B overflows · Q4 blast radius 14 loads+21 stores+value-model. 2 missing prerequisites: MirType-byte-size + value-model upgrade. 0 fixtures with Trit/Tryte-in-struct → YAGNI. 3 opening conditions recorded (phase10 + TODO).

## ▶ C2 IN PROGRESS — arm-level Pattern::Wildcard in enum match (G fully signed off 2026-06-10, NO ADR needed).
**O's probe:** `match c { Red=>1, _=>0 }` → `unsupported match pattern (expected enum variant): Wildcard` (lower:2545) = BLOCKED. Sub-pattern `SomeInt(_)` already handled (2487); arm-level `_` is blocked. Foundation already exists: enum match has SwitchInt + `default_bb: trap_bb` → C2 maps the wildcard arm into default instead of trap.
**3-slice plan (G signed off):** C2.1 lower wildcard-arm→default_bb (reuse the wildcard-last guard + ≤1 from nullable lower:2204) · C2.2 typecheck wildcard suppresses E1026 exhaustive · C2.3 fixture + teeth.
**CRITICAL RISK (G stresses): A3 regression** — the wildcard suppressing E1026 must NOT let a non-wildcard-non-exhaustive case slip through. **Fixture 103 (A3) must be protected at all costs.**

## ✅ C2 CLOSED (HEAD a25fbff, 2026-06-10). Wildcard arm enum match.
C2.1 lower wildcard→default_bb Goto (guard wildcard-last+≤1 reusing nullable) · C2.2 suppressing E1026 was ALREADY-THERE from A3 (exprs.rs:1578 short-circuit) · C2.3 fixture 106 + **relaxed INV 4i-6 `Trap→Trap|Goto`** (D discovered it and reported it straightforwardly, correct scope: Unreachable STILL rejected → A2 not reborn). Teeth: A3-suppress-poison→103 loses E1026 (real protection) + wildcard→trap-poison→SIGILL. Gate 0·0·102·203. **D improved: reported touching the verifier straightforwardly, without hiding it.**

## ✅ C6 CLOSED (HEAD 992311e, 2026-06-10) — TIER D LEFTOVERS CLEANED.
concat→sret: shim `(dest_slot,a_ptr,a_len,b_ptr,b_len)` writes back into `*mut FatStr` (pattern (b) append, NOT (a) Rust-auto-sret). **O corrects C6.0:** the probe overturned D's claim "auto-sret proven" — `ArgumentPurpose::StructReturn`=0 hits, the "sret" in the codebase = manual by-pointer (append *mut FatStr); D wired (b) correctly. The caller dropped reconstructing len (removing the caller-must-know-concat-len coupling). Teeth: shim len→0→fixture 35 FAILED. Shim registration fn_4_1→fn_5_0 (main+tests). Gate 0·0·102·203. Every fat-String return is now consistently sret callee-filled.

## ✅ C5 DEFERRED (502713a) + ▶ OUTCOME PRODUCER OPEN (ADR-0052, HEAD 16e0d56, 2026-06-10).
C5 spike: light premise (Cranelift multi-return is native, does NOT break the value-model) but 0 producer → deferred to Group E. **Outcome producer = the use-case that reopens C5 (closes the loop).**
**ADR-0052 Outcome ABI (continuing the ADR-0020 design-lock):** 2-slot {disc:Trit, payload} MIR + Cranelift multi-return. G's invariants: payload is ONLY Tier A scalar (heap deferred to B/C) · un-defer C5 ONLY for BinaryOutcome/TernaryOutcome (generic tuple stays Err) · Cranelift native (value-model unchanged).
Current state: Frontend ✅ (lexer ~+/~-/~0 + AST OutcomeConstructor) · Typecheck 🟡 (check_outcome_constructor_context has a foundation) · Lower 🔴 degenerate (`~+ e`=identity lib.rs:1108, `~-`=unsupported 1124) · MIR ✅ (ReturnShape::BinaryOutcome + OutcomeDiscriminant/Unwrap ops defined, 0 producer) · JIT 🔴 (multi-value blocked jit:1070).
**4 OP slices:** OP.1 typecheck E1024/E1025+return-match → OP.2 lower 2-slot **check-mode fixture** (MIR verify, isolating the producer from the JIT) → OP.3 JIT un-defer C5-for-Outcome (remove guard 1070 ONLY for Outcome) → OP.4 match/unwrap. Teeth: disc Positive→Zero (E1025), removing-the-guard-for-generic-tuple still Err, caller inst_results[1] drops the payload wrongly, OutcomeDiscriminant picks the wrong slot.
## ✅ OP.1 CLOSED (HEAD 1e980d0, 2026-06-10 — SESSION STOPPING POINT). Gate 0·0·105·203.
OP.1 typecheck Outcome: E1025 (`~0` on T~E) + E1026 outcome-non-exhaustive (SEPARATE variant exprs.rs:313, distinct from enum's 327, sharing the code) was already in place; D added the return-type-match payload (guard-style exprs.rs:404-411, `~+`:value_type `~-`:error_type). Fixtures 107/108/109. Teeth: payload-match poison→109 FAILED. A3 fixture 103 KEPT (Outcome has its own separate E1026 variant, doesn't break the enum). O raised soundness on the wildcard-single-variant→D fixed `_=>{}`→explicit `OutcomeArm::Zero=>None`.

## ▶ NEXT SESSION: OP.2 — Lower Outcome → 2-slot (CORE WORK, G is waiting for "the shape of the 2-slot ReturnShape").
Lower is currently DEGENERATE (lib.rs:1108 `~+ e`=identity, 1124 `~-`=unsupported). OP.2 must: `~+ v`/`~- e` → allocate 2-slot {disc:i64 Trit, payload:i64} · disc=Positive(1)/Negative(-1) const · `ReturnShape::BinaryOutcome` (arity 2) for fn `-> T~E` · `Return{values:[disc,payload]}`. **CHECK-MODE fixtures** (parse→typecheck→lower→borrowck→MIR verify, NO JIT — isolating the producer from the backend, pattern signed off by G). MIR ops OutcomeDiscriminant/Unwrap (mir:254-280) get wired in OP.4. ADR-0052 §3-4. Payload ONLY Tier A scalar (heap deferred to B/C). Teeth O plans: disc Positive→Zero (E1025/verifier), wrong ReturnShape arity, Return values.len≠2.
After OP.2: OP.3 JIT un-defer C5-for-Outcome (remove guard jit:1070 ONLY for BinaryOutcome) · OP.4 match/unwrap.

## GLOBAL STATE AT END OF SESSION (2026-06-10, HEAD 1e980d0):
Debt-Repayment Crusade DONE: **A clean** (A1/A2/A3 teeth) · **B1/B2 closed** (MirType ADR-0050 · borrowck-merge ADR-0051 −1034 lines) · **B3/Native/Packed/C5 sealed as Group E** (YAGNI, opening conditions recorded in the blueprint) · **C1/C2/C6 done**. IN PROGRESS: **Outcome Producer** (ADR-0052, OP.1 ✅, OP.2-4 remaining). 18 commits this session. 3 new ADRs (0050/0051/0052) + 5 blueprints (phase7-12). Gate 0·0·105·203.

--- (C1 gap detail below, for reference) ---
**▶ C1 — Enum payload through function param (G's order, active consumer of the B1a foundation):**
Gap (O surveyed): caller jit:1162 enum-arg→stack_load only takes the discriminant, DROPS the payload · callee param-entry 676-690 doesn't create enum_slots for enums · payload-access jit:310/382 → "Payload access on non-enum local". Fix = the Fat-Pointer String param pattern from Tier D (jit:1148-1165): caller stack_addr by-pointer, callee reconstructs enum_slot from the pointer; by-pointer decision via `match MirType::Enum(_)`. Fixture 27 pinned the bug as `// ERROR` but the program is actually VALID → C1 turns it positive `// EXPECT: 52`, hitting the jit:314 string-match. D wrote `spec/plans/phase9-c1-enum-payload.md`. Closes the B1a debt "Struct/Enum foundation with no consumer".

--- (B2.2 audit history below, kept for reference) ---
## ⚠ B2.2 AUDIT (O, G's order — OVERTURNING THE ASSUMPTION). HEAD 58dfa4e.
G ordered "move E2410 mutability + E2400 lifetime to MIR". O audited 100% of emit sites → **the premise was wrong**:
- **E2410 `CannotMutateFrozenOwner`**: **0 constructs in the logic = DEAD skeleton** (ADR-0025 §7.1 not yet wired). 0 fixtures. NOTHING to move.
- **E2430 `NamespaceInferenceFailed`**: **0 emits, 0 fixtures = DEAD skeleton.** (G guessed name-resolution — the logic actually doesn't exist yet.)
- **E2400 `BorrowLifetimeInferenceFailed`**: **ALIVE** (emit check.rs:468, 2 fixtures) — return-borrow elision ambiguity (ADR-0046, static-signature-level, NOT NLL live-range).
- **The mutability check ACTUALLY running for real = E1016 `AssignToImmutable`** (typecheck::E1016, type-level `let x` vs `let mutable x`) — a type-system concern, NOT borrow/dataflow. Correctly stays in typecheck.
→ **B2.2 in reality only has E2400 left** (moving return-borrow lifetime). E2410/E2430 = dead variants (decide delete-dead or keep). B2.1 already moved the ENTIRE currently-running borrowck-enforcement (E2420+E2440). Report package awaiting G's decision: B2.2 = E2400 only? + clean up the 2 dead variants? E25XX OUTSIDE B2.

## S1 CLOSED (verified by O): MirType 14 variants in triet-mir, Display round-trip, parse-shim (MUST KILL at S4), is_copy(Option<&Body>) ONE piece of logic + the invariant-B8 carried along. Field `ty:String` KEPT — +0 behavior change. Teeth 2 red hits (is_copy heap; is_vec ordering after patching the fake-teeth test). **Round 4 lesson → [[feedback-poison-must-be-red]] (G's iron rule).**

## S2 CLOSED — G SIGNED OFF THE COMMIT (2026-06-09). HEAD will be the next commit (NOT YET committed at time of writing — waiting for the author/D to type it).
**S2 = flip field String→MirType + producer `lower_type` (map TypeExpr→MirType directly) + delete `simple_is_copy`.** Gate **0·0·99·205**. 3 rounds of O blocking it: V1 re-parse reference (G invariant #1), V2 claiming-clippy-blame-on-the-wrong-source (blaming generated code), **V3 disguised producer** (`type_name→String` then parsed back = fake producer, nearly broke G invariant ③ → [[feedback-verify-producer-before-consumer]]). Teeth A (producer String→Unknown: 10/99 red) + B (is_copy heap→Copy: mir unit + 6/99 red). Commit msg approved by G: `feat(track-c): B1a S2 — flip field String→MirType + producer lower_type + delete simple_is_copy`.

2 INVARIANTS G must keep watching:
1. **Display-bridge is ONE-WAY:** `match ty.to_string().as_str()` FORBIDDEN. Matching MUST go through `MirType`.
2. **Poison-must-go-red RULE** ([[feedback-poison-must-be-red]]) + **producer-before-consumer** ([[feedback-verify-producer-before-consumer]], G confirmed via V3).

## S3 IN PROGRESS — STRING PURGE (G's order). On the working tree (not yet committed, base fe80b8c).
**Already fixed (verified by O):** S3.1 return_type→MirType ✓ · lower_type_simple refuse-over-guess (removed default-Struct) ✓ · jit destructure Struct/Enum ✓ · parse in production=0 (confined to tests) · to_string dispatch=0.
**Remaining debt before closing S3 (G's directive 2026-06-09):**
1. **Eliminate the 5 free `&str` helpers** (mir:2924-2961 is_nullable_type/nullable_payload/is_vec_type/is_hashmap_type/is_copy&str) + the last 2 callers. "Deleting 37 helpers is no joke" (G). The allow items_after_test_module will disappear.
2. **OPTION (b) — G LOCKED IT IN:** delete the 2 separate HashSet struct_names/enum_names → create a **`HashMap<String, TypeKind>`** (`enum TypeKind{Struct,Enum}`) = a rudimentary ItemSymbolTable. Pass-1 scans Items and populates the map; Pass-2 lower_type references the map (without waiting for struct_layouts). Cuts `too_many_arguments`. Reason for keeping a map (not a layout-table): lower_type runs DURING layout-construction (lower:354), so the tables aren't complete yet.

**Teeth 1a+1b do NOT go red → splitting Struct/Enum is a no-consumer FOUNDATION (G accepts this):** correct-by-construction for B2/C1; the net closes when C1 matches the enum-payload. NOT stuffing in a fake test (G praised the honesty).

## G DELEGATES TO O (2026-06-09): O measures gate+teeth ITSELF → self-approves the S3 COMMIT (no need to ask for a separate sign-off) → reports the closed book to G. D relabeling allow on its own = overstepping authority/covering up (G: "hiding an error is worse than causing one").

## ✅ O SIGNS OFF S3 (2026-06-09) — meets the bar after 5 rounds of O blocking it:
- V1 code-doesn't-compile (35 E0308, claim "build 0" only checked lib) · V2 lower_type_simple default-Struct (fake producer #2, violating G invariant ②) · V3 G's S3.2 delta→Option (b) `HashMap<String,TypeKind>` ItemSymbolTable · V4 5 helpers not yet deleted + 3 bare clippy warnings (D deleted allow→bare warning, claim "too_many resolved" wrong since 9>7) · V5 TypeKind inserted in the middle of the lower_program doc-comment.
- **O's verification result:** workspace passes · 5 free helpers=0 · TypeKind map scans Items (replacing the 2 HashSets) · return_type→MirType · `=="String"`/`starts_with('&')`/is_enum_type/is_fat_type=0 · parse in production=0 (confined to tests) · to_string dispatch=0 · clippy 203 (allow justified). **Teeth 1 (producer String→Unknown: fixture red) + Teeth 2 (is_copy heap→Copy: mir unit red).** Splitting Struct/Enum = no-consumer foundation (G accepts, closes when C1 lands).
- Commit msg: `feat(track-c): B1a S3 — String purge: TypeKind symbol-table + eliminate free helpers`. Waiting for the author/D to type the commit (O does not git commit without the author's order).
- S4 debt: `MirType::parse` remains only in tests → **S4 rips out parse** (build blows up red wherever it's forgotten). lower_function 9-param → gather into a LoweringInput struct (deferred, allow justified).

## S4 — THE EXECUTION of parse (G's order 2026-06-09, S3 CLOSED). B1a integrity acid test.
**Surface (O surveyed ec6d32f):** delete `pub fn parse` (mir:559) + **3 From-shims that call parse** (`From<&str>`/`From<String>`/`From<&String>` mir:732-746 — this is the umbilical cord for `MirBuilder::new("Unit")`). `From<&MirType>` (748) KEPT (clone). Deleting the shims → **35 `MirBuilder::new(...,"str")` + ~25 alloc_local_ty/LocalDecl &str** break → D switches them to the enum directly (`MirType::Unit` etc.). + ~25 tests calling `MirType::parse("..")` (mir) + lower:2930 (test). lower:2930 already verified = a test (0 production use of parse, matching S3).
**G's acid test:** delete parse → `cargo test --workspace` builds; any production code sneaking a use of parse → blows up red. O's teeth: after deletion, add `let x: MirType = "String".into()` to PRODUCTION → it MUST NOT COMPILE (From<&str> is dead = the umbilical cord is cut).
**Done:** `rg MirType::parse` workspace=0 · `rg 'From<&str> for MirType'`=0 · gate green · tests use the enum variant directly (G: "expose every nook and cranny"). → B1a COMPLETE, closing report to G.

## Done this session
- **ADR-0050** (`docs/decisions/0050-mir-type-enum.md`) — **signed O+G 2026-06-09**. Decision: hand-written `MirType` enum in triet-mir (NOT the schema-generated Type — MIR is the backend IR, not the AST). 3 invariants from G: ① MirType-lives-in-mir ② **SPLIT `Struct(String)`/`Enum(String)`** (merging into a UserType forbidden — preserves type-safety, catches wrong-table-pointer bugs) ③ the transitional `parse(&str)` shim must **die at the final commit** (tagged `// TECH-DEBT(B1a): MUST KILL THIS SHIM`).
- **CORRECTION to ADR §3.1.1/§3.1.2 (O, post-probe)** — fixing O's own mistake in the first signed version:
  - **Vector/HashMap BARE, NO payload** (`Vector(Box)` → `Vector`). Measured: 0 consumers extract the element type, 0 diagnostic prints `"Vector<…>"` → the payload is a dead field (Rule #4). **Major consequence: R1/R2 generic-parsing DISSOLVES — `lower_type` only-reads-the-arena, does NOT touch typecheck's `type_map`.**
  - **`Trilean` BARE** (drop `refined`). Measured: 0 backend readers of `.refined` (refinement is a frontend gate, checked before MIR).
  - This is enforcing Rule #4 within the framework G already signed → O accepts it on its own authority, NO need to re-sign, but **flag it for G in the next report package**.
- **Phase-0 spike (D)**: thrown out entirely (rg MirType crates/ → none). Proved the structural fix caught the ordering-rule bug (`is_vec_type("Vector<Integer>?")` old=true wrong → `Nullable(Vector).is_vec()`=false correct).

## BLOCKED: S1 not yet approved — blueprint `spec/plans/phase7-b1-type-system.md` deviates from the ADR in 6 places
D wrote a 386-line blueprint (good file:line survey) but O **did not approve S1** because of deviations:
1. Named it `enum Type` → must be `MirType` (clashes in meaning with typecheck/generated Type).
2. Added `Trilean{refined}` → drop it (dead, 0 readers).
3. Kept `Vector(Box)`/`HashMap{k,v}` + items R1/R2 → bare instead, R1/R2 dissolve.
4. **`is_copy_simple` dodges G's death sentence** — renamed instead of unified. Must be ONE piece of logic (`is_copy(&self, body: Option<&Body>)`).
5. **Step 2 "the gate will break"** — VIOLATES CLAUDE.md "tests green before any commit". Must have the Display-bridge (`.ty.to_string()` at consumers not yet migrated) in-the-same-commit → every commit green.
6. The "site" count is off: blueprint 67 vs TODO 189 vs O's-pure-dispatch-count 76. Lock down 1 definition.

## What D must do before requesting re-approval of S1
Fix the phase7 blueprint per the 6 points above (sync with ADR §3.1.1/§3.1.2, delete R1/R2, Step 2 Display-bridge-green, unify is_copy). Do NOT merge Step 1+2 (Step 1 in parallel = O's gate checkpoint).

## Production plan (ADR §6, strangler, every commit GREEN)
S1 parallel (add MirType+Display+parse, field keeps String) → S2 flip field + Display-bridge + **delete simple_is_copy** → S3 migrate consumers cluster by cluster (mir→lower→borrowck→jit; 20 `__triet_*` literals KEPT) → S4 rip out the `parse` shim (build blows up red wherever forgotten). Done: `rg parse/is_vec_type/simple_is_copy` → 0; gate 0·0·99·208; teeth red (String→Copy, ordering, Struct-looks-up-wrong-table, INV-4, fixture-27).

## Debt carried forward (not touching B1a)
B1b typecheck↔schema Type reconcile (deferred until after B2) · concat→sret · B2 borrowck merge · B3 alias-analysis · C1 enum-payload fixture-27.
