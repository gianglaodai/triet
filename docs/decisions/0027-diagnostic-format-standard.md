# ADR 0027 — Diagnostic Format Standard (AI-first)

**Status:** **Locked** (promoted via v0.8.x.review 2026-05-28). Language-wide canonical format for all compiler/runtime diagnostics — applied in v0.8.10 for E24XX + E25XX skeleton diagnostics. Retroactive scope: ADR-0020 (11 error blocks) + ADR-0025 (10 error blocks, already follows this standard) + all future ADRs with error codes.

**Issue:** Author 2026-05-26 finalized 4 priorities for Triết: strict + compile-time + performance + **AI-friendly**. The final priority means diagnostic messages must be parseable for LLM agents and quick-fix tools. Pre-2026-05-26 state:

| Source | Format |
|---|---|
| ADR-0020 (Outcome) | Name + description + inline hint (`Did you mean X?`). Lacks span block and structured fix. |
| ADR-0021 (Trilean refinement) | Reference only, no full blocks. |
| ADR-0025 (Borrow checker, recently landed) | Full format with span block + `[Fix N]` numbered fixes + "Change X to Y" imperative. |
| Other ADRs | Reference only. |

Inconsistency $\rightarrow$ LLM agents learn different patterns for each error code, making AST-modification fix extraction unreliable.

ADR-0025 §1.4 originally defined the format for the E24XX namespace; reviewer 2026-05-26 pointed out it should be promoted cross-cutting. The format currently in [ADR-0025 §1.4](0025-borrow-checker-rules.md) will be slimmed down to a pointer to this ADR.

ADR locks: canonical format for ALL diagnostic blocks in SPEC, ADRs, CLAUDE.md, and Rust source generating diagnostics.

---

## §1 — Goals

1. **Single canonical format** for compile-time, link-time, and runtime diagnostics.
2. **Machine-parseable** — LLM agents and `dao fmt --fix` can extract fix instructions via simple regex/grammar.
3. **Human-readable** — clear context and actionable advice.
4. **Compact** — no redundant boilerplate.
5. **Forward-compatible** — support LSP code actions, RAG indexing, and IDE quick-fixes.
6. **Pure ASCII** — no emojis, no Unicode arrows, and no box-drawing characters (ensuring compatibility with all terminals/tools).

---

## §2 — Format specification

**Lock:** Every diagnostic block must adhere to the skeleton below.

```text
EXXXX ErrorName
    [Description line 1 — what happened, 1 concise sentence.]
    [Context line 2-3 — types/values/positions involved.]
    
    --> path/to/file.tri:LINE:COL
       |
    LINE | <source line that triggered>
       |     ^^^^^^^^ <short pointer label>
    
    Suggested fixes:
    
    [Fix N] <Approach name — 1 verb phrase, ≤ 60 chars>:
    <Imperative instruction starting with `Change`, `Wrap`, `Use`, `Add`, `Replace`, `Remove`, `Move`, `Refactor`.>
```

### 2.1 — Header line

```
EXXXX ErrorName
```

- `EXXXX` — 4-digit code per CLAUDE.md namespace allocation (E0000 lex, E000X parse, E10XX typecheck, E20XX runtime, E21XX modules, E22XX capability, E23XX pack, E24XX borrow, E25XX+ reserved).
- `ErrorName` — PascalCase, descriptive. Ends without punctuation.
- Warnings: `WXXXX WarningName` — same shape, `W` prefix.

### 2.2 — Body block

1-3 lines, indented 4 spaces. Lines:

- **Line 1 (required):** What happened. Past tense or imperative. 1 sentence.
- **Line 2 (optional):** Context — relevant types, values, or positions.
/ - **Line 3 (optional):** Reference to a spec section if non-obvious (e.g., "Frozen owners are read-only (ADR-0022 §3.4)").

### 2.3 — Span block (optional but recommended)

Required if the compiler knows the specific source position. Format follows the Rust diagnostic style but is ASCII-only:

```text
    --> path/to/file.tri:LINE:COL
       |
    LINE | <source line>
       |     ^^^^^^^^ <pointer label>
```

- `-->` ASCII arrow.
- `path:LINE:COL` — relative path from project root + 1-based line + 1-based column.
- `|` separator aligned vertically.
- `LINE` left-aligned line number (aligned with the line content).
- Caret `^^^^^^` + label for the main span.
- `---` (dashes) for secondary spans (for borrow conflicts requiring 2-3 lines).
- Multi-line spans use the `|` separator between line markers; insert `...` for large gaps.

Omit when:
- Runtime error has no source position (corrupt wire data, internal invariant).
- Link-time error covers multiple files (use prose context instead).

### 2.4 — Suggested fixes block (optional)

```text
    Suggested fixes:
    
    [Fix 1] <Approach name>:
    <Imperative instruction>
    
    [Fix 2] <Approach name>:
    <Imperative instruction>
```

Rules:

1. **Always numbered** `[Fix 1]`, `[Fix 2]`, ... — the number serves as a machine-extractable key.
2. **Approach name** — 1 verb phrase, $\le$ 60 chars, ends with `:`. Examples: `Return owned value instead`, `Reorder the read before mutation`, `Wrap in a method on the owner struct`.
3. **Imperative instruction** — starts with allowed verbs:

| Verb | Use when |
|---|---|
| `Change X to Y` | Small textual replacement, X and Y are specific |
| `Replace X with Y` | Larger replacement, Y is prose |
| `Wrap X in Y` | Structural enclosure (`Wrap logic in impl block`) |
| `Use X` | Suggest using an existing feature/pattern |
| `Add X` | Insertion (e.g., `Add Send trait`) |
| `Remove X` | Deletion |
| `Move X to Y` | Reordering |
| `Refactor X to Y` | Multi-step structural change |
| `Verify X` | Conditional fix requiring a context check |

4. **No diff format** `-old/+new` — difficult to parse and easily confused with comments.
5. **No emoji**, no Unicode `→`; use "to" or "becomes" instead of an arrow.
6. **Code fragments** in backticks: `` `code` ``.
7. **Backtick discipline for substitution form** — see §2.4.1 below.
8. **References** in parentheses: `(ADR-0022 §3.4)`.

Omit the entire block when no actionable fix is available (e.g., wire data corruption — the user cannot fix this).

### 2.4.1 — Backtick discipline for substitution form (regex contract)

**Lock:** When an instruction uses the form `Change X to Y` where both X and Y are **literal source code**, it is MANDATORY to wrap both X and Y in backticks. This is a contract with the parser/extractor:

```
Regex contract:   Change `([^`]+)` to `([^`]+)`
Capture group 1:  X (exact source text being replaced)
Capture group 2:  Y (exact source text being inserted)
```

**Mandatory form for "direct textual substitution":**

```
Change `-> &0 String` to `-> &+ String`        OK
Change `null` to `~0`                          OK
Change `take(alice)` to `take(&0 alice)`        OK
```

**Incorrect pattern (parser will fail to extract):**

```
Change -> &0 String to -> &+ String            WRONG — missing backticks
Change "old" to "new"                          WRONG — uses quotes instead of backticks
```

**When X is not literal code but a noun phrase ("parameters", "the signature line", "field type"), DO NOT use `Change`** — choose another verb so the parser knows this is not a direct substitution:

```
Refactor parameters to a single collection borrow:
Change `(a: &0 String, b: &0 String)` to `(items: &0 Vector<String>)`

Replace the entire signature line with the owned-return form:
Change `function f(...) -> &0 T` to `function f(...) -> &+ T`

Move `print(r1.length)` to immediately before `v.push(4)`
```

The first 3 verbs (`Refactor`, `Replace`, `Move`) signal to the parser that this is a "structural change, not a simple substitution — fall back to prose handling." The body may contain an additional line `Change `X` to `Y`` to concretize the actual substitution.

**Practical rules for authors writing diagnostics:**

| Fix Type | Leading Verb | Backtick Rule |
|---|---|---|
| Direct textual substitution (regex-extractable) | `Change` | Both X and Y must be backticked |
| Structural / multi-step change | `Refactor`, `Wrap`, `Replace`, `Move` | Backtick only literal code parts in the body |
| Run external tool | `Use` | Backtick the command string |
| Add/Remove element | `Add`, `Remove` | Backtick the element being added/removed |

Parser logic: scan each `[Fix N]` block, regex-extract `Change `X` to `Y`` $\rightarrow$ direct substitution. Other verbs $\rightarrow$ prose handling (human review or more complex parser).

### 2.5 — Recommended number of Fixes

- **1 fix:** Still use the full `[Fix 1]` (for parser consistency).
- **2-3 fixes:** Optimal. Place the most recommended fix first.
- **4+ fixes:** Consider merging similar fixes. If more than 3 are truly necessary, sort them by relevance.

---

## §3 — Diagnostic categories

| Category | Span | Fix block |
|---|---|---|
| Compile-time error from user code | Required | Required if actionable |
| Compile-time error from impossible state (compiler bug) | Optional | Omit (file a bug report instead) |
| Link-time error (cross-file) | Optional (use prose) | Required if actionable |
| Runtime error from user logic | Required if trackable by interpreter | Required if actionable |
| Runtime error from corrupted state (wire data, FFI) | Omit | Omit |
| Warning (W prefix) | Required | Required if an auto-fix path exists |

---

## §4 — Examples

### 4.1 — Compile-time error with span + 3 fixes (E2400 from ADR-0025)

```text
E2400 BorrowLifetimeInferenceFailed
    Cannot infer which input the returned borrow ties to.
    Function has 2 input borrows: `a: &0 String`, `b: &0 String`.
    
    --> src/example.tri:1:62
       |
    1  | function pick_longer(a: &0 String, b: &0 String) -> &0 String {
       |                                                     ^^^^^^^^^ ambiguous return borrow
    
    Suggested fixes:
    
    [Fix 1] Return owned value instead (requires cloning inside body):
    Change `-> &0 String` to `-> &+ String`
    
    [Fix 2] Group inputs into a collection with a single borrow scope:
    Refactor parameter list: change `(a: &0 String, b: &0 String)` to `(items: &0 Vector<String>)`
    
    [Fix 3] Encapsulate inside a struct method (ties return to `self`):
    Wrap logic in `impl StringPair { function longer(self: &0 StringPair) -> &0 String { ... } }`
```

### 4.2 — Compile-time error WITHOUT span (declaration-level)

```text
E1024 NullableErrorInOutcomeType
    Outcome error type cannot itself be nullable.
    Type `T~E?` parses as `T~(E?)`, which is semantically meaningless:
    if the operation fails, an error must be present.
    
    Suggested fixes:
    
    [Fix 1] Use ternary-outcome syntax when null success is meaningful:
    Change `T~E?` to `T?~E`
    
    [Fix 2] Drop the nullable suffix on the error type:
    Change `T~E?` to `T~E`
```

(Span is optional when the reproduction location is the type expression itself — the parser will inject the span at the use site.)

### 4.3 — Runtime error with no fix (corruption)

```text
E2210 InvalidOutcomeState
    Outcome value of type `T~E` has discriminator Trit::Zero,
    which is reserved. This indicates corrupt wire data or a
    future-version pending state encountered by a pre-v0.8 reader.
```

No span (interpreter does not trace source). No fix (user code did not cause this).

### 4.4 — Warning with auto-fix path (W2001 from ADR-0020 §10.3)

```text
W2001 NullDeprecated
    `null` keyword is deprecated. Replace with `~0` (canonical Trit::Zero
    literal per ADR-0020 §10). This warning becomes error E2002 at v1.0.
    
    --> src/example.tri:5:23
       |
    5  |     let maybe_x: Integer? = null
       |                             ^^^^ deprecated keyword
    
    Suggested fixes:
    
    [Fix 1] Replace with canonical Trit::Zero literal:
    Change `null` to `~0`
    
    [Fix 2] Run the automated migration tool across the codebase:
    Use `dao fmt --fix --migrate-null` from project root
```

---

## §5 — Rationale & Alternatives Considered

### 5.1 — Why not use a rustfix JSON sidecar?

Rust emits machine-applicable suggestions via a JSON sidecar (`cargo fix`). Triết chose to **embed them in text**:

- **Pros of embedding:** Single source of truth (no sidecar drift), humans read the same content, simpler tooling pipeline, and works in any terminal, log, or issue tracker.
- **Cons:** The parser must understand the text format (vs. JSON parsing). Mitigation: The §2 format is rigid enough to be regex-extractable.

### 5.2 — Why not use LSP code actions exclusively?

LSP code actions operate within an IDE editor. Triết diagnostics must be usable in:
- Terminal output (CLI compilation)
- CI logs
- Issue trackers
- LLM agent context (Claude reading an error)
- RAG indexing
- `dao fmt --fix` batch mode

The embedded text format works everywhere; the LSP layer builds on top as needed.

### 5.3 — Why "Change X to Y" instead of a diff `-/+`?

- Diff formats are easily confused with code comments (`// -foo +bar`).
- They are hard to extract via regex (multi-line, ambiguous boundaries).
- `Change X to Y` is an imperative sentence — LLM agents can map it directly to an AST replace operation.
- Inspired by GitHub's "Did you mean?" suggestions and Clippy's "consider X" lints, refined for machine parsing.

### 5.4 — Why force `[Fix N]` numbering even for a single fix?

Consistency for parsers. The regex `\[Fix \d+\]` will always match. If a single fix used plain text, the parser would require two separate codepaths.

### 5.5 — Forward compatibility with LSP code actions

The LSP code action format requires a (title, edit ranges, replacement text). Mapping from the §2 format:

- `[Fix N] Title` $\rightarrow$ code action `title`.
- `Change X to Y` instruction $\rightarrow$ workspace edit applying a text range replacement.
- Span block $\rightarrow$ diagnostic range.

A future `triet-lsp` server can auto-generate code actions from diagnostic text using a parser of $\le$ 50 lines.

---

## §6 — Retroactive migration scope

The ADR locks the format. Retroactive updates are applied to:

| ADR | Error blocks | Status (2026-05-26) |
|---|---|---|
| ADR-0020 (Outcome) | 11 (E1024, E1025, E1026, E1027, E1028, E1029, E1030, E1031, E1032, E2002, E2210) + W2001 | Sub-task of this ADR — to be updated in the same commit as ADR-0027 |
| ADR-0025 (Borrow checker) | 10 (E2400, E2402, E2403, E2410, E2411, E2420, E2421, E2422, E2430, E2440) | Already follows the format (originated in §1.4). Slimming §1.4 to a pointer. |
| ADR-0021 (Trilean refinement) | None — only references | No update needed |
| ADR-0018 (Capability loader) | None — only references E2200-E2208 | No update needed unless future expansion |
| Other ADRs | None | No update needed |
| Rust source generating diagnostics | Audit deferred | v0.8+ sub-task — codegen layer follows §2 |

Future ADRs introducing diagnostics: **MUST** follow the §2 format or cite a §3 exemption.

---

## §7 — Out of scope

- **Multi-language error messages** — Triết defaults to English diagnostics. The i18n layer is deferred until post-v1.0.
- **Color codes / terminal escape sequences** — This is a concern for the output layer, not the format specification.
- **Error code aliases / deprecated mappings** — Handled by individual ADRs when sunsetting codes.
- **Stack trace format for runtime** — An orthogonal concern; requires a separate ADR if needed.
- **JSON output mode** (`dao --json`) — The wire format is already locked at the CLI level; it uses the same fields but encoded as an object. Mapping table deferred to the CLI ADR if the format changes.

---

## §8 — References

- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (Origin of §2 format; §1.4 will slim to a pointer here when this ADR lands)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (Retroactive update target — 11 error blocks + W2001)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (W-to-E migration window for W2001 $\rightarrow$ E2002)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (Updated to mention ADR-0027 as the canonical format spec)
- [VISION §6 — Refuse over guess](../../VISION.md) (Philosophical alignment — error messages must be actionable, not "warn-and-continue")
- `feedback_explicit_strictness.md` (User memory — verbose explicit pattern, applies to diagnostic clarity)
