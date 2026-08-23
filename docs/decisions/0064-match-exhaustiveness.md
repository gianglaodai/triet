# ADR-0064 — Match Exhaustiveness (Scalar-Literal Match)

- **Status:** 🔒 LOCKED — G sign-off 2026-06-19. Drafted by Mentor O 2026-06-19 per G ruling.
- **Date:** 2026-06-19
- **Author:** Mentor O (Match-on-Literal campaign — enabling value-keyed matches for Integer/Trilean, following Trit-path T6 precedent).
- **Related:** ADR-0061 T6 (Trit-match value-keyed SwitchInt, `lib.rs:2924` — structural template) · ADR-0021 (Trilean Ł3) · SPEC §match.

---

## 1. Context
Triet supports `match` on `Trit` (value-keyed SwitchInt, T6) + enum (GetDiscriminant) + nullable (`~+/~0`) + Outcome. HOWEVER, `match` on **Integer/Trilean literals** was rejected outright in LOWERING (`Expr::Match` enum-path fallthrough, `lib.rs:3797-3800`): `match x { 1 => .. }` / `match t { true => .. }` → "unsupported match pattern". Typecheck **accepted** it (error was isolated to lowerer). Inconsistent developer experience: matching Trit was permitted, while matching Integer/Trilean was forbidden.

## 2. Decision — Exhaustiveness Rule (Locked by G 2026-06-19)
**EVERY `match` in Triet MUST be exhaustive.**
1. **Integer (infinite domain):** A wildcard arm `_` (or variable binding `other =>`) is MANDATORY. Missing a wildcard = error (exhaustiveness is impossible without a catch-all).
2. **Trilean / Trit (finite domain of 3 values):** Wildcard may be omitted IF all 3 possibilities are explicitly enumerated (Trilean: `true`/`false`/`unknown`; Trit: `-1_trit`/`0_trit`/`1_trit`). Missing any variant without a wildcard = error.

## 3. Encoding (Measured, Not Guessed)
- **Trilean** literal → `ConstValue::Trit` i64: `True=1, False=-1, Unknown=0` (`lower:1464-1466`). `MirType::Trilean` (distinct from `MirType::Trit`).
- **Trit** literal → `-1/0/1` (suffix `_trit`).
- **Integer** literal → direct i64 value (`LiteralPattern::Integer{value, suffix:None}`).

## 4. Implementation — Enforcement Layer (G Ruling: Temporary Trap in Lowerer)
**The CORRECT exhaustiveness gateway is Typecheck (compile-time).** Typecheck currently DOES NOT enforce this (accepts non-exhaustive matches and passes them to lowerer). G finalized for this campaign:
- **Lowerer:** value-keyed SwitchInt for Integer/Trilean (mirroring Trit-path 2924). `cases: Vec<(i64,BasicBlock)>` + wildcard-last + SwitchInt + **default → wildcard body IF present, else `Terminator::Trap`/Unreachable** (GAP-2, identical to Trit-path).
- **Trap GAP-2 = TEMPORARY MEASURE.** Missing match arms without a wildcard → runtime trap, NOT a compile error. This is documented debt.
- **★ RECORDED DEBT (Separate campaign, NOT bundled into this slice):** Typecheck Exhaustiveness — catch missing arms at compile-time (enforcing Rule §2) rather than panicking via runtime traps. Applied uniformly to Integer/Trilean/Trit/enum/nullable.

## 5. Teeth (Match-on-Literal Lowering Campaign)
- Integer match routes to correct branch → correct value; Trilean match covers all 3 branches → correct value.
- **Trap on Missing Arms:** Integer match without wildcard, scrutinee hits unlisted value → runtime trap (SIGILL). Trilean missing variants without wildcard → trap.
- Wildcard catch-all → correct value.
- Regression: Trit-path (174) + 209 corpus fixtures + workspace remain green.

## 6. Consequences
- **Positive:** Uniform `match` semantics across all scalars; resolves UI discrepancy where Trit was accepted while Integer was rejected.
- **Temporary State:** Exhaustiveness enforced via runtime traps (lowerer), not yet compile-time. Transparently recorded as debt in §4 — NO dark corners.
- **Frozen Scope:** Typecheck exhaustiveness is reserved for a dedicated campaign.

## 7. Signatures
- O: ✅ (encoding measured from `lower:1464`; template derived from Trit-path 2924 precedent; typecheck debt transparently isolated)
- G: ✅ (approved 2026-06-19 — Exhaustiveness Rule locked; GAP-2 trap in lowerer is a TEMPORARY MEASURE; Typecheck-Exhaustiveness is a SEPARATE campaign, forbidden to overload lowering slice; mirrors Trit-path, forbidden to introduce new branching patterns)

---

## 8. AMENDMENT 2026-06-19 — Typecheck Exhaustiveness (Closing §4 Debt) — Traceable Edit

**Context:** §4 recorded the debt "Typecheck Exhaustiveness = separate campaign". The Latent Type-Inference campaign (Item 4) established the foundation (scalar scrutinees have static types). This campaign (Item 1) closes the §4 debt: **migrating Rule §2 enforcement from runtime traps (lowerer) to compile-time (typecheck).**

**DOES NOT REVERSE §2/§3** — only adds an enforcement layer. Decisions (Signed by G 2026-06-19):

| # | Decision | Resolution |
|---|---|---|
| 1 | Error Code | **Reuse E1026** + new variant `NonExhaustiveScalarMatch { type_name, missing }`. DO NOT invent new error codes (following "1 code, multiple variants" precedent as in Outcome/Enum). |
| 2 | Catch-all | `Pattern::Wildcard` (`_`) **OR** `Pattern::Variable(name)` (binding `other =>`) — both short-circuit. |
| 3 | Lowerer GAP-2 Trap | **RETAINED AS-IS, forbidden to remove.** Typecheck is the fortress; lowerer trap is unreachable defense-in-depth for well-typed code. |
| 4 | ADR Scope | Amend 0064 §8 (here). ADR-0065 reserved for Struct?/Enum? heap-nullable. |
| 5 | Tryte/Long | **DEFERRED, recorded as debt.** Rule §2 specified Integer/Trilean/Trit; lowerer does not yet support Tryte/Long matching (rejects explicitly, not silently). Apply rules when lowerer support is introduced. |

**Enforcement (typecheck `check_match`, `exprs.rs:1728`, adding branches after enum/nullable/outcome dispatch):**
- **`Type::Integer`** (infinite domain): NO catch-all → E1026 "Integer match requires `_` wildcard". (`Range`/`Or` literals DO NOT satisfy this — catch-all is still required.)
- **`Type::Trilean { .. }`** (including refined): missing catch-all & missing any of {true, false, unknown} → E1026 listing missing variants. `Or` expands sub-patterns.
- **`Type::Trit`** (−1/0/1): missing catch-all & missing variants → E1026 listing missing items. `Or` expands.

**Blast Radius (O scanned entire corpus 2026-06-19):** ZERO broken fixtures — all existing scalar matches are already exhaustive (215/218 Integer matches have `_`; 174/214 Trit matches cover all 3; 216 Trilean matches cover all 3).

**Amendment Signatures:**
- O: ✅ (recon file:line — gap at `exprs.rs:1797`; E1026 template ready in `error.rs:399`; blast radius measured as ZERO; lowerer trap retained)
- G: ✅ (approved 2026-06-19 — 5 decisions finalized; GAP-2 trap forbidden to remove; Tryte/Long deferred with documented debt)

**New Debt (2026-06-20) — ✅ CLOSED (`fa021b4`):** `Pattern::Variable` (catch-all variable binding `other =>`) was accepted by typecheck, but the **lowerer (`lib.rs:3224`) rejected it** for scalar matches — a gap between typecheck acceptance and lowerer rejection. Closed: lowerer now binds Variable catch-alls to the scrutinee value (`bind_scalar_catch_all`, wired across all 3 Trit/Trilean/Integer paths; scalar Copy types require no `push_owned`/`Drop`). The GAP-2 trap remains for non-catch-all paths. Teeth: fixtures 222 (Integer value proof) / 223 (Trit) / 224 (Trilean) red-then-green; poison removing Variable arm → rejection returns.

---

## A1. AMENDMENT 2026-06-20 — Match Tryte/Long + Tryte Range-Check (Appendix §A1)

**Context:** §8 decision #5 recorded debt "Tryte/Long DEFERRED — Rule §2 specified Integer/Trilean/Trit; lowerer does not yet support Tryte/Long matching". This campaign resolves that debt: enables value-keyed `match` for **Tryte** and **Long**, uniformly applies Exhaustiveness Rule §2, and closes the un-enforced Tryte range vulnerability.

**Inscribed Measurements (verified from `triet_core::Tryte`, NOT relying on memory):** `Tryte::MAX = 9_841`, `Tryte::MIN = -9_841` → Tryte = **9 trits**, range `[-9841, 9841]` (19,683 values). *(Initial recon mistakenly assumed 6 trits / ±364 — self-corrected and verified against `tryte.rs:42`.)*

**DOES NOT REVERSE §2/§3/§8** — merely extends the locked mechanisms. Campaign **DOES NOT touch** i64 ABI value-models, borrowck, drop-glue, or JIT shims.

### A1.1 Mechanism
- **Tryte/Long literal match → value-keyed SwitchInt, identical i64 key-extraction as Integer** (all three map literals directly to `i64` keys). Lowerer extracts a **shared helper** unifying Integer/Tryte/Long under a single infrastructure pipeline (eliminating copy duplication — §8 introduced Integer/Trilean; adding Tryte/Long would yield 3 duplicate value-keyed paths without consolidation).
- **Trit/Trilean REMAIN SEPARATE** (different keys: Trit = −1/0/1 suffix; Trilean = True/False/Unknown discriminant). Surgical separation, not forced into the helper.

### A1.2 Exhaustiveness (Applying Rule §2)
- **Tryte:** finite domain `[-9841, 9841]` containing **19,683 values** → treated as a **large domain**, MANDATING a catch-all `_` (or `Variable`), identical to Integer. Enumerating 19,683 branches is impractical; omitting wildcards via full enumeration is not allowed (unlike Trit/Trilean).
- **Long:** conceptually bignum domain (practically i64-capped, see A1.4) → MANDATES a catch-all `_`, identical to Integer.
- **Enforced in Typecheck (compile-time), reusing E1026** `NonExhaustiveScalarMatch` — identical structure to §8 Integer path. Missing catch-all → E1026.

### A1.3 Tryte Range-Check (Closing Overflow Hole)
- **Literal Trytes outside `[-9841, 9841]` → E1036** (generalized from `IntegerLiteralOverflow`). Enforced at **BOTH locations:**
  - **Expressions:** `let x: Tryte = 9999_tryte` → E1036.
  - **Pattern Literals:** `match t { 9999_tryte => .. }` → E1036. *(Primary loophole: `bind_pattern` was previously a no-op on literals → match-arms allowed out-of-range values if only checked in expressions.)*
- **Generalized E1036:** adds `type_name` so error messages distinguish `Tryte` (±9,841) vs `Integer` (±3,812,798,742,493). In accordance with G's principle: "do not let out-of-range values slip through — seal them at Typecheck birth".

### A1.4 ★ Honest Debt — Long i64-Cap (Inscribed in Stone, NO Dark Corners)
- **Long range is NOT enforced in this slice.** Long is part of the **deferred bignum subsystem** (i64 value model, ADR-0050 MirType). Tryte range is sealed in this campaign; Long range **remains pending**.
- **Consequence of i64-cap:** Long match-arms with literal keys `> i64::MAX` (or `< i64::MIN`) → **lowerer error "out of range"**, directly inheriting the value-model's i64 limit. NO silent truncation (in accordance with ADR-0044). This is **transparently documented debt**, not a feature — to be closed when the bignum value model is implemented (future JIT/wide-int).

### A1.5 Decision Table

| # | Decision | Resolution |
|---|---|---|
| 1 | Tryte/Long Match Mechanism | Value-keyed SwitchInt, i64 key like Integer; shared helper for Integer/Tryte/Long. Trit/Trilean remain distinct. |
| 2 | Tryte Exhaustiveness | Large domain (19,683 values) → MANDATORY catch-all `_`, like Integer (no exhaustive listing allowed). E1026. |
| 3 | Long Exhaustiveness | MANDATORY catch-all `_`, like Integer. E1026. |
| 4 | Tryte Range-Check | Generalize E1036 (`type_name`); caught in BOTH expressions AND pattern literals. |
| 5 | Long Range-Check | **DEFERRED** (bignum). Long keys > i64 → lowerer "out of range" (inheriting i64-cap). Documented debt in A1.4. |

### A1.6 §A1 Amendment Signatures
- O: ✅ (Tryte::MAX=9,841 verified from `tryte.rs:42`, corrected 6-trit to 9-trit recon; mechanism shares Integer keying; helper extraction eliminates duplication; range check covers BOTH expr + pattern; Long i64-cap documented in A1.4 — untouched value-model/borrowck/JIT)
- G: ✅ (approved 2026-06-20 — 3 guard points fully agreed with O: mandatory helper extraction, Tryte range check on BOTH Expr + Pattern, Long i64-cap deferral recorded in appendix; GAP-2 trap in §4 forbidden to remove; mirrors §8 template, forbidden to invent new pattern variants)
