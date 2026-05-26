# ADR 0027 — Diagnostic Format Standard (AI-first)

**Trạng thái:** **Draft**. Language-wide canonical format cho mọi compiler/runtime diagnostic. Retroactive scope: ADR-0020 (11 error blocks) + ADR-0025 (10 error blocks, đã follow chuẩn này) + tất cả ADRs tương lai có error code.

**Issue:** Author 2026-05-26 chốt 4 priority cho Triết: strict + compile-time + performance + **AI-friendly**. Priority cuối nghĩa là diagnostic message phải parseable cho LLM agents và quick-fix tools. State pre-2026-05-26:

| Source | Format |
|---|---|
| ADR-0020 (Outcome) | Name + description + inline hint (`Did you mean X?`). Không có span block, không có structured fix. |
| ADR-0021 (Trilean refinement) | Reference only, no full blocks. |
| ADR-0025 (Borrow checker, vừa land) | Full format với span block + `[Fix N]` numbered fixes + "Change X to Y" imperative. |
| ADRs khác | Reference only. |

Inconsistency → LLM agents học pattern khác nhau cho mỗi error code, AST-modification fix extraction không reliable.

ADR-0025 §1.4 originally defined format cho E24XX namespace; reviewer 2026-05-26 chỉ ra nên promote cross-cutting. Format hiện nằm ở [ADR-0025 §1.4](0025-borrow-checker-rules.md) sẽ slim xuống thành pointer tới ADR này.

ADR locks: canonical format cho ALL diagnostic blocks ở SPEC, ADRs, CLAUDE.md, và Rust source generating diagnostics.

---

## §1 — Goals

1. **Single canonical format** cho compile-time, link-time, runtime diagnostics.
2. **Machine-parseable** — LLM agents và `dao fmt --fix` extract fix instructions qua regex/grammar đơn giản.
3. **Human-readable** — context rõ ràng, actionable advice.
4. **Compact** — không boilerplate thừa.
5. **Forward-compatible** — support LSP code actions, RAG indexing, IDE quick-fix.
6. **Pure ASCII** — không emoji, không Unicode arrows, không box-drawing chars (đảm bảo pipe qua mọi terminal/tool).

---

## §2 — Format specification

**Lock:** Mọi diagnostic block tuân thủ skeleton dưới đây.

```text
EXXXX ErrorName
    [Description line 1 — what happened, 1 câu súc tích.]
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
- **Line 2 (optional):** Context — relevant types, values, positions.
- **Line 3 (optional):** Reference to spec section if non-obvious (e.g., "Frozen owners are read-only (ADR-0022 §3.4)").

### 2.3 — Span block (optional but recommended)

Required nếu compiler biết source position cụ thể. Format theo Rust diagnostic style nhưng ASCII-only:

```text
    --> path/to/file.tri:LINE:COL
       |
    LINE | <source line>
       |     ^^^^^^^^ <pointer label>
```

- `-->` ASCII arrow.
- `path:LINE:COL` — relative path from project root + 1-based line + 1-based column.
- `|` separator chạy thẳng cột.
- `LINE` left-aligned line number (giữ thẳng cột với line content).
- Caret `^^^^^^` + label cho main span.
- `---` (dashes) cho secondary spans (cho borrow conflict cần show 2-3 lines).
- Multi-line spans dùng `|` separator giữa các line markers; chèn `...` cho khoảng cách lớn.

Omit khi:
- Runtime error không có source position (corrupt wire data, internal invariant).
- Link-time error covering multiple files (use prose context instead).

### 2.4 — Suggested fixes block (optional)

```text
    Suggested fixes:
    
    [Fix 1] <Approach name>:
    <Imperative instruction>
    
    [Fix 2] <Approach name>:
    <Imperative instruction>
```

Rules:

1. **Always numbered** `[Fix 1]`, `[Fix 2]`, ... — number is machine-extractable key.
2. **Approach name** — 1 verb phrase, ≤ 60 chars, ends with `:`. Examples: `Return owned value instead`, `Reorder the read before mutation`, `Wrap in a method on the owner struct`.
3. **Imperative instruction** — starts with allowed verbs:

| Verb | Use when |
|---|---|
| `Change X to Y` | Small textual replacement, X and Y specific |
| `Replace X with Y` | Larger replacement, prose Y |
| `Wrap X in Y` | Structural enclosure (`Wrap logic in impl block`) |
| `Use X` | Suggest using existing feature/pattern |
| `Add X` | Insertion (e.g., `Add Send trait`) |
| `Remove X` | Deletion |
| `Move X to Y` | Reorder |
| `Refactor X to Y` | Multi-step structural change |
| `Verify X` | Conditional fix requiring context check |

4. **No diff format** `-old/+new` — khó parse, dễ nhầm với comments.
5. **No emoji**, không Unicode `→`, dùng "to" / `becomes` thay arrow.
6. **Code fragments** trong backticks: \`code\`.
7. **Backtick discipline cho substitution form** — xem §2.4.1 dưới.
8. **References** trong parens: `(ADR-0022 §3.4)`.

Omit toàn block khi không có fix actionable (e.g., wire data corruption — user không sửa được).

### 2.4.1 — Backtick discipline cho substitution form (regex contract)

**Lock:** Khi instruction dùng form `Change X to Y` với X và Y đều là **literal source code**, BẮT BUỘC bọc cả X và Y trong backtick. Đây là contract với parser/extractor:

```
Regex contract:   Change `([^`]+)` to `([^`]+)`
Capture group 1:  X (exact source text bị thay)
Capture group 2:  Y (exact source text thay vào)
```

**Bắt buộc form khi muốn báo "direct textual substitution":**

```
Change `-> &0 String` to `-> &+ String`        OK
Change `null` to `~0`                          OK
Change `take(alice)` to `take(&0 alice)`        OK
```

**Sai pattern (parser sẽ không extract được):**

```
Change -> &0 String to -> &+ String            SAI — không có backtick
Change "old" to "new"                          SAI — quote thay backtick
```

**Khi X không phải literal code mà là noun phrase ("parameters", "the signature line", "field type"), KHÔNG dùng `Change`** — chọn verb khác để parser biết đây không phải direct substitution:

```
Refactor parameters to a single collection borrow:
Change `(a: &0 String, b: &0 String)` to `(items: &0 Vector<String>)`

Replace the entire signature line with the owned-return form:
Change `function f(...) -> &0 T` to `function f(...) -> &+ T`

Move `print(r1.length)` to immediately before `v.push(4)`
```

3 verb đầu (`Refactor`, `Replace`, `Move`) signal cho parser "structural change, không phải simple substitution — fall back to prose handling". Body có thể chứa thêm 1 dòng `Change \`X\` to \`Y\`` để concretize phần substitution thực tế.

**Quy tắc thực dụng cho author viết diagnostic:**

| Loại fix | Verb đầu | Backtick rule |
|---|---|---|
| Direct textual substitution (regex-extractable) | `Change` | Cả X và Y phải backtick |
| Structural / multi-step change | `Refactor`, `Wrap`, `Replace`, `Move` | Backtick chỉ literal code parts trong body |
| Run external tool | `Use` | Backtick command string |
| Add/Remove element | `Add`, `Remove` | Backtick element being added/removed |

Parser logic: scan từng `[Fix N]` block, regex-extract `Change \`X\` to \`Y\`` → direct substitution. Các verb khác → prose handling (human review hoặc more complex parser).

### 2.5 — Số lượng Fix khuyến nghị

- **1 fix:** Vẫn dùng `[Fix 1]` đầy đủ (consistency cho parsers).
- **2-3 fix:** Optimal. Most-recommended first.
- **4+ fix:** Cân nhắc gộp similar fixes. Nếu thực sự cần > 3, sort theo độ phù hợp.

---

## §3 — Diagnostic categories

| Category | Span | Fix block |
|---|---|---|
| Compile-time error from user code | Required | Required nếu có fix |
| Compile-time error from impossible state (compiler bug) | Optional | Omit (file bug report instead) |
| Link-time error (cross-file) | Optional (use prose) | Required if actionable |
| Runtime error from user logic | Required nếu interpreter trackable | Required if actionable |
| Runtime error from corrupted state (wire data, FFI) | Omit | Omit |
| Warning (W prefix) | Required | Required nếu có auto-fix path |

---

## §4 — Examples

### 4.1 — Compile-time error with span + 3 fixes (E2400 từ ADR-0025)

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

(Span optional khi reproduction location is the type expression itself — parser sẽ inject span at use site.)

### 4.3 — Runtime error with no fix (corruption)

```text
E2210 InvalidOutcomeState
    Outcome value of type `T~E` has discriminator Trit::Zero,
    which is reserved. This indicates corrupt wire data or a
    future-version pending state encountered by a pre-v0.8 reader.
```

No span (interpreter doesn't trace source). No fix (user code didn't cause this).

### 4.4 — Warning với auto-fix path (W2001 từ ADR-0020 §10.3)

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

## §5 — Rationale & alternatives considered

### 5.1 — Tại sao không dùng rustfix JSON sidecar?

Rust emits machine-applicable suggestions via JSON sidecar (`cargo fix`). Triết chọn **embed in text**:

- **Pros của embedded:** 1 source of truth (no sidecar drift), human reads same content, simpler tooling pipeline, works in any terminal/log/issue tracker.
- **Cons:** Parser phải hiểu format text (vs JSON parse). Mitigation: format §2 đủ rigid để regex-extractable.

### 5.2 — Tại sao không dùng LSP code actions exclusively?

LSP code actions hoạt động trong IDE editor. Triết diagnostic phải usable trong:
- Terminal output (CLI compile)
- CI logs
- Issue trackers
- LLM agent context (Claude reading error)
- RAG indexing
- `dao fmt --fix` batch mode

Embedded text format works everywhere; LSP layer build on top khi cần.

### 5.3 — Tại sao "Change X to Y" thay vì diff `-/+`?

- Diff format dễ nhầm với code comments (`// -foo +bar`).
- Hard to extract via regex (multi-line, ambiguous boundary).
- `Change X to Y` là 1 imperative sentence — LLM agents map thẳng vào AST replace operation.
- Inspired bởi GitHub's "Did you mean?" suggestions + Clippy's "consider X" lints, refined for machine parsing.

### 5.4 — Tại sao force `[Fix N]` numbered ngay cả khi chỉ 1 fix?

Consistency cho parsers. Regex `\[Fix \d+\]` luôn match. Nếu single-fix dùng plain text, parser cần 2 codepaths.

### 5.5 — Forward compat với LSP code actions

LSP code action format yêu cầu (title, edit ranges, replacement text). Mapping từ §2 format:

- `[Fix N] Title` → code action `title`.
- `Change X to Y` instruction → workspace edit applying text range replacement.
- Span block → diagnostic range.

Future `triet-lsp` server có thể auto-generate code actions từ diagnostic text bằng parser ≤ 50 lines.

---

## §6 — Retroactive migration scope

ADR locks format. Retroactive update applied to:

| ADR | Error blocks | Status (2026-05-26) |
|---|---|---|
| ADR-0020 (Outcome) | 11 (E1024, E1025, E1026, E1027, E1028, E1029, E1030, E1031, E1032, E2002, E2210) + W2001 | Sub-task of this ADR — update trong commit cùng land ADR-0027 |
| ADR-0025 (Borrow checker) | 10 (E2400, E2402, E2403, E2410, E2411, E2420, E2421, E2422, E2430, E2440) | Đã follow format (originated §1.4 here). Slim §1.4 to pointer. |
| ADR-0021 (Trilean refinement) | None — only references | No update needed |
| ADR-0018 (Capability loader) | None — only references E2200-E2208 | No update needed unless future expansion |
| Other ADRs | None | No update needed |
| Rust source generating diagnostics | Audit deferred | v0.8+ sub-task — codegen layer follows §2 |

ADRs tương lai introducing diagnostics: **MUST** follow §2 format hoặc cite §3 exemption.

---

## §7 — Out of scope

- **Multi-language error messages** — Triết default English diagnostic. i18n layer defer post-v1.0.
- **Color codes / terminal escape sequences** — output layer's concern, not format spec.
- **Error code aliases / deprecated mappings** — handled by individual ADRs khi sunset codes.
- **Stack trace format** for runtime — orthogonal concern, separate ADR if needed.
- **JSON output mode** (`dao --json`) — wire format already locked at CLI level; uses same fields but encoded as object. Mapping table defer to CLI ADR if format changes.

---

## §8 — Tham chiếu

- [ADR-0025 — Borrow Checker Rules](0025-borrow-checker-rules.md) (origin of §2 format; §1.4 will slim to pointer here when this ADR lands)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (retroactive update target — 11 error blocks + W2001)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (W-to-E migration window cho W2001 → E2002)
- [CLAUDE.md — Error code namespace](../../CLAUDE.md) (cập nhật mention ADR-0027 là canonical format spec)
- [VISION §6 — Refuse over guess](../../VISION.md) (philosophical alignment — error message phải actionable, không "warn-and-continue")
- `feedback_explicit_strictness.md` (user memory — verbose explicit pattern, applies to diagnostic clarity)
