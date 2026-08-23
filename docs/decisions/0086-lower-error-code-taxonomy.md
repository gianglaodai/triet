# ADR 0086 — `LowerError` Error Code Taxonomy (`triet::lower::E11XX`)

**Status:** Decision (WO-Front-A, signed by O + G). Applicable from Tier C.
Transforms `LowerError` (`crates/triet-lower/src/lib.rs`) from a flat struct
`{message, span}` WITHOUT error codes into an `enum` with 8 variants, each bearing a
`triet::lower::E11XX` code + `#[derive(miette::Diagnostic)]`, mirroring
`CapabilityError` (`crates/triet-typecheck/src/capability_check.rs`).

> **AMENDMENT (ADR-0089 §3 cleanup pass, 2026-07-26):** adds a 9th variant,
> `E1143 BreakContinueOutsideLoop`, in the User error category — see row E1143
> in the table below. The remainder of the ADR body describes the ORIGINAL decision
> (8 codes); preserving history without retroactive rewriting, recording the amendment here and in the table row.

**Issue:** `LowerError` was the only struct in the compiler pipeline lacking error
codes and lacking a `miette::Diagnostic` implementation — violating CLAUDE.md §Error code
namespace (all other compiler layers have namespaces: lexer E0000, parser E000X, typecheck E10XX,
modules E21XX, capability E22XX, borrowck E24XX, actor E25XX) and ADR-0027
(diagnostic format standard). The driver printed errors via `eprintln!("{path}: lowerer
error: {e}")` — bare unformatted text, lacking spans, lacking codes, inconsistent with
parse/typecheck/borrowck diagnostics (which render via `miette::Report` +
`GraphicalReportHandler`). There were ~47 `LowerError` construction sites in the lowerer (8 named
helper constructors + ~39 inline construction sites) — with no lookup codes, and no way to
distinguish "invalid user programs" from "unsupported compiler features" from
"broken internal invariants (ICE)" without parsing the error message string.

## Decision

**8 codes across 4 semantic categories**, derived from a complete reconnaissance of all ~47 existing
error construction sites in `crates/triet-lower/src/lib.rs`:

| Code | Variant | Category | Meaning |
|---|---------|-----|---------|
| `E1100` | `ConstructNotYetLowered` | Compiler-completeness gap | AST construct is semantically valid but current backend has not yet lowered it — NOT a user program error. |
| `E1120` | `NullableEnumPayloadUnsupported` | Design fence | Payload-bearing `Enum?` inside aggregates — discriminant niche nullable repr (ADR-0065 §12.7) is sound only for unit-only enums. Architectural fence, not an "unimplemented feature". |
| `E1121` | `NullableStructReturnHeapField` | Design fence | `Struct?` return containing heap-bearing fields — sret buffer tag-prepend lacks drop-glue (ADR-0065 §4 B8). Architectural fence. |
| `E1122` | `EscapingClosureSealed` | Design fence | Escaping/first-class closures (`Expr::Lambda`) are INTENTIONALLY sealed (YAGNI, ADR-0039 recon) — not a compiler gap. |
| `E1140` | `UndefinedLocal` | User error | Local variable does not exist in scope. |
| `E1141` | `NullLiteralWithoutExpectedType` | User error | Constructor `~+`/`~0`/`~-` lacks expected type from context to infer destination type. |
| `E1142` | `LiteralOutOfRange` | User error | Literal value (Trit/Tryte/Long/Integer in pattern match) exceeds representable range of the type. |
| `E1143` | `BreakContinueOutsideLoop` | User error | **Added post-facto (ADR-0089 §3 cleanup pass, 2026-07-26), OUTSIDE original 8 codes** — `break`/`continue` outside `loop`/`while`/`for`. Previously helper `break_continue_outside_loop` mistakenly borrowed `E1140 UndefinedLocal` (semantic mismatch: users seeing "Undefined Local" for `break;` suspect a variable resolution bug). Parser DOES NOT block this case (`E0006 BreakValueOutsideLoop` is dead code with 0 sites) and typecheck no-ops `Stmt::Break`/`Stmt::Continue`, making the lowerer the SOLE line of defense — dedicated code for clarity. |
| `E1190` | `InternalInvariant` | ICE (Internal Compiler Error) | An internal invariant relied upon by lowerer (resolved name resolutions, exhaustiveness scan, converging fixpoints, …) was violated. This is a **compiler bug**, not a user error — help text requests issue report with minimal reproduction. |

### Why E1190 Consolidates ALL Remaining 35/47 Sites into a SINGLE ICE Code

After isolating 4 "design fence" codes (E1120/E1121/E1122 — architectural fences with dedicated
ADRs) + 3 "user error" codes (E1140/E1141/E1142 — actual user code triggers) + 1 "completeness gap"
code (E1100 — valid constructs not yet lowered), all remaining (35/47 inline construction sites)
fall into branches where **"typecheck should have rejected this prior to reaching lowerer"**:

- Duplicate match arms (`duplicate ~+ arm`, `duplicate ~0 arm`, `duplicate catch-all`)
  — exhaustiveness/uniqueness is validated by typecheck (SPEC §A1.2); if lowerer
  encounters duplicates, lowerer is executing on an AST that typechecker should have rejected.
- Malformed patterns (`unsupported sub-pattern in ~+ arm`, `~- arm on
  nullable type`, …) — same rationale: typechecker gates pattern structure based on
  scrutinee type before lowerer inspects it.
- Unresolved name resolutions (`unresolved enum variant`, `unknown enum`,
  `unknown variant`) — `pattern_resolutions`/`method_resolutions` are outputs of
  typecheck; a missing entry indicates lowerer misreading resolution tables, not invalid user input.
- Non-converging fixpoints (`struct/enum layout sizing did not converge`) —
  ADR-0068 forbids recursive types/Box, ensuring type graph is always a finite DAG; non-convergence
  indicates the "finite DAG" invariant was violated upstream.
- Broken elision invariants (`return-borrow elision expects exactly 1 ref-param`) —
  in-place comment explicitly states "typecheck E2400 should have rejected this".

Consolidated into one code rather than 35 individual codes because: (a) no dedicated ADR/spec section
exists for each case — these are not design choices, they are "this should never happen"; (b) user
remediation action is IDENTICAL across all 35 sites (do not fix user code — report compiler bug);
(c) finer granularity creates the false illusion that each branch represents a distinct "user error type",
when in reality all 35 belong to a single class: "an upstream pipeline phase should have rejected this".
Message text is preserved at each site (unchanged), so detailed diagnostic context
(specific variable/variant/pattern names) is fully retained — consolidated solely under a single lookup code.

### 2 Boundary Rulings (Sites not naturally fitting the 3 categories)

- **`:5419` (original codebase, line drifting with edits) → `E1100`**, not `E1190`.
  This site refuses returning `Vector`/`HashMap`/`Enum`/`Reference` from trait
  methods (`callee_ret` is non-scalar) — in-place comment notes "debt #2
  scope": backend DOES NOT YET HAVE ABI support for these return cases (multi-value
  return, Outcome 2-register ABI, …), not an invariant violation.
  The user program is entirely valid (trait methods returning `Vector` is valid semantics) —
  merely unsupported in current backend. This is a textbook compiler-completeness gap → `E1100`.

- **`:5935` (original codebase) → `E1122`**, not `E1100` or `E1190`. This site
  refuses `Expr::Lambda` (escaping/first-class closures). In-place comment explicitly
  notes this is an INTENTIONAL seal (YAGNI per ADR-0039 recon Phase 14.0) —
  nullable/Outcome operator families (`~+>`, `~->`) lower via dedicated AST nodes,
  with no consumers requiring true escaping closures. This is NOT "unfinished work"
  (E1100) — completion will not occur because design intentionally provides no pathway.
  Nor is it an ICE (E1190) — no invariant was violated; this is an intentional refusal on
  syntactically/semantically valid input. Hence, requires dedicated code `E1122` rather than
  falling into other categories.

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | Retain flat struct, adding only `code: &'static str` field | Minimal work | Lacks `#[diagnostic]`/miette rendering, lacks type-safety (code is string disconnected from message), driver must assemble `Report` manually | Rejected — fails parity with typecheck/borrowck/capability. |
| 2 | Enum with 47 variants (1 variant per site) | Maximum granularity | 35/47 sites lack distinct ADR/semantic rationale — creates illusion of 35 distinct "error types" when all share "broken invariant" class; heavy maintenance burden (each lowerer refactor requires renaming variants) | Rejected — violates "Simplicity First" (CLAUDE.md §2). |
| 3 | Enum with 8 variants across 4 semantic categories (chosen) | Balanced: distinguishes user errors / design fences / completeness gaps / ICE, preserving exact message strings without losing context | ICE class (E1190) bundles 35 sites under 1 code — requires this ADR to document boundary rationale | **CHOSEN.** |

## Consequences

### Positive
- `LowerError` achieves parity with `TypeError`/`BorrowError`/`CapabilityError`/
  `ConcurrencyError` — all implementing `miette::Diagnostic` with `triet::<area>::EXXXX` codes.
  CLAUDE.md §Error code namespace updated with `triet::lower::E11XX`.
- Driver renders lowerer errors using uniform `miette::Report` +
  `NamedSource` formatting matching parse/typecheck/borrowck (span highlighting instead of bare text).
- 8 named constructors preserve signatures — 47/47 call sites retain invocation
  signatures, with only 39 inline construction sites updating `LowerError { .. }` to
  `LowerError::Variant { .. }`.
- Test `tests/diagnostics.rs` (new) locks all 8 codes via direct assertions
  on `miette::Diagnostic::code()`; 2 of the 8 (`E1120`, `E1121`) trigger via
  real fixtures (414, 440) rather than hand-built instances, proving codes emit naturally
  from the pipeline.

### Negative
- Code `E1190` trades lookup granularity: 35 distinct root causes share a single
  lookup code. Log readers must inspect `message` (preserved with full detail) to determine
  the exact broken invariant — code signifies "this is an ICE".
- `crates/triet-lower/Cargo.toml` adds 2 dependencies (`thiserror`, `miette`)
  — minor crate build-time increase (negligible compared to `triet-typecheck` which
  already depends on both).

### Risks to Mitigate
- If a future site categorized under E1190 turns out to BE triggerable by valid
  user code (meaning typechecker failed to reject as assumed) — that is an upstream
  typecheck bug to fix, not a reason to reclassify the site as a user error. Misclassifications
  should only be adjusted AFTER proving (Rule #7 refuse-over-guess) that typechecker genuinely cannot block it.

## Effective Date

- Tier C+ — applicable immediately upon merging WO-Front-A.
- Not retroactive for legacy logs/reports using bare text (git history preserves
  old struct representations without reverse migration).
