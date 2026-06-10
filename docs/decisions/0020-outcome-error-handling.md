# ADR 0020 — Outcome error handling (`T~E` / `T?~E` — trit-encoded fallibility)

**Trạng thái:** Quyết định. Áp dụng cho v0.7.4.3-error + tất cả new code từ v0.7.4.3 trở đi. Closes [ADR-0019 Addendum §A7](0019-self-hosting-compiler-bootstrap.md#a7--deferred-items-log-technical-debt-surfaced-by-v073) deferred item *"Error handling primitive — recovery / try-catch / supervisor"*. Foundational design — affects SPEC §2.5 (nullable + new fallible primitive), ADR-0003 (Iterator), std.result existing enum, capability resolver return types, and the entire self-host compiler error paths.

**Issue:** Triết v0.6 ships với 4 cơ chế xử lý "absence/failure":

| Cơ chế | Style | Ví dụ |
|---|---|---|
| `T?` nullable primitive | Imperative-friendly (`?.`/`?:`/`!!`) | `find_user(id) -> User?` |
| `Result<T, E>` enum (std.result) | Functional (Rust-style match) | `parse_config(s) -> Result<Config, ParseError>` |
| `Trilean::Unknown` | Ł3 semantic uncertainty | `capability_resolve(req) -> Trilean` |
| Runtime panic (`VmError` E22XX) | Non-recoverable bug-tier | `divide(a, 0)` → E2204 |

Author 2026-05-17 surfaced philosophical concern: `Result<T, E>` borrowed from Rust **ép** functional programming style (`.map()`/`.and_then()`/`match` ceremony). Authors who prefer imperative style (Java-sensibility) chỉ có pattern match — verbose ở các call site đơn giản. Discussion explored Go-style `(value, error)` (rejected — `if err != nil` boilerplate + zero-value pitfall + ambiguous invariant) và considered Zig error union, Swift throws, effect systems (Koka).

Final design picks **trit-encoded outcome type** parallel với existing `T?` primitive: single-trit discriminator over 2 or 3 states, syntax-level imperative ergonomics, type-system safety, balanced ternary identity anchored.

This ADR locks the design fully — type system, wire format, syntax, operators, methods, pattern match, and coexistence with existing `Result<T, E>` — so all downstream sub-tasks v0.7.4.3+ can adopt idiomatic patterns without further redesign.

## §1 — Type syntax: `T~E` (2-state) and `T?~E` (3-state)

**Lock:** Triết gains two new primitive type forms — `T~E` (binary outcome) and `T?~E` (ternary outcome with null state). Both encoded by a **single trit discriminator** + payload union, mirroring existing `T?` nullable structure.

### 1.1 — `T~E` semantics

```text
T~E  ::=  Trit discriminator  +  payload union of { T, E }

    Trit::Positive  → payload is T (success)
    Trit::Zero      → INVALID (typecheck error / runtime panic)
    Trit::Negative  → payload is E (failure)
```

Mathematical alignment: balanced ternary `{-1, 0, +1}` directly encodes the outcome — positive is success, negative is failure, zero is the **reserved state** (covered §6).

### 1.2 — `T?~E` semantics

```text
T?~E ::=  Trit discriminator  +  payload union of { T, ∅, E }

    Trit::Positive  → payload is T   (success with value)
    Trit::Zero      → null marker    (success path, no value)
    Trit::Negative  → payload is E   (failure)
```

The `?` modifier here is **outcome-level**, not type-level. It promotes the Zero state from reserved-invalid to a meaningful null marker. The success arm carries bare `T`, NOT `T?` — there is no nested nullable.

### 1.3 — Lexer compound token `?~` (CRITICAL)

The marker for ternary outcome is a **lexer-level compound token** `?~` — emitted as a single token when `?` and `~` appear adjacent without whitespace. This is the same compound-token discipline as `~+`, `~-`, `~0` (§2) và `~+>`, `~0>`, `~->` (§3): no whitespace-within is allowed.

**Lexer rule cho ternary map operators:** mỗi `~+` / `~0` / `~-` (constructor tokens từ §2) look ahead 1 char. Nếu `>` adjacent (no whitespace), emit 3-char compound `~+>` / `~0>` / `~->` — đây là arm-specific map operator (postfix). Nếu không có `>` hoặc có whitespace trước, emit constructor token (prefix). Position context cũng đảm bảo không ambiguity: constructor đứng trước expression payload (prefix), map operator đứng sau expression (postfix). `~->` không xung đột với function return arrow `->` vì `->` đứng sau `)` của param list, còn `~->` đứng sau expression — lexer phân biệt qua preceding token.

```text
LexerToken    ::=  ...
                |  '?~'                   # ternary-outcome marker (compound, in type position)
                |  '~+>'                  # success-arm transformer (compound, postfix operator)
                |  '~0>'                  # null-arm transformer (compound, postfix operator)
                |  '~->'                  # error-arm handler (compound, postfix operator)
                |  '~'                    # binary-outcome marker (in type position)
                |  '?'                    # nullable marker (postfix in type position)

TypeExpr      ::=  BaseType '?~' ErrorType       # T?~E ternary outcome
                |  BaseType '~'  ErrorType       # T~E binary outcome
                |  BaseType ('?')+               # T? chained nullable
                |  ...
```

The lexer emits exactly one of:
- `?~` compound (when `?~` adjacent, no whitespace): used for `T?~E`.
- `?` and `~` as separate tokens (when whitespace between them): the parser will NOT recombine. In type position, `T ? ~ E` (with whitespace between `?` and `~`) is a **syntax error** because dangling `~ E` is not a valid type-position suffix.

This rules out three classes of bug:

1. **Parser ambiguity** with future operators that happen to start with `?` or `~`.
2. **Whitespace-sensitive parse divergence** (some compilers parse `T ? ~ E` differently from `T?~E` — Triết explicitly does not).
3. **Three-token-lookahead in the parser** — `?~` is single-token compound, parser stays predictive LL(1).

Style guide permits both `T?~E` (no spaces, terse) and `T ?~ E` (space outside compound, readable). The lexer treats both as identical. Author 2026-05-17 directive: "compound token, dính liền không khoảng trắng" — locked.

If a caller genuinely wants "outcome of nullable success" (vanishingly rare in practice — null can flow through outcome's Zero state instead), they use `std.result::Result<T?, E>` (the v0.4 legacy enum, see §8 coexistence).

### 1.4 — `T~E?` rejected

`T~E?` parses as `T~(E?)` — "fallible operation with nullable error". Semantically meaningless: if the operation fails, an error must be present. Compile-time refused:

```text
E1024 NullableErrorInOutcomeType
    Outcome error type cannot itself be nullable.
    Type `T~E?` parses as `T~(E?)`, which is semantically meaningless:
    if the operation fails, an error must be present.
    
    --> src/example.tri:3:30
       |
    3  | function read_file() -> String~IoError? {
       |                              ^^^^^^^^^ nullable error type
    
    Suggested fixes:
    
    [Fix 1] Use ternary-outcome syntax when null success is meaningful:
    Change `T~E?` to `T?~E`
    
    [Fix 2] Drop the nullable suffix on the error type:
    Change `T~E?` to `T~E`
```

(Format follows [ADR-0027](0027-diagnostic-format-standard.md) — canonical diagnostic format.)

Refuse-over-guess per VISION §6.

### 1.5 — Examples

```triet
// Binary outcome — operation either succeeds or fails:
function read_file(path: String) -> String~IoError {
    // ...
}

// Ternary outcome — operation succeeds-with-value, succeeds-with-null, or fails:
function lookup_symbol(name: String) -> Symbol?~IoError {
    // Cache lookup. I/O can fail. Symbol may not exist (success path).
}

// Nested outcomes (rare, but valid):
function complex(input: String) -> (Integer~ParseError)~IoError {
    // I/O can fail; on I/O success, parse can fail.
}
```

## §2 — Constructor syntax: `~+ expr` / `~0` / `~- expr`

**Lock:** Outcome values are constructed by three prefix forms aligned with balanced ternary `{+1, 0, -1}`. The `~` prefix links the constructor syntactically to the `T~E` type family.

### 2.1 — The three constructor forms

| Constructor | Trit state | Payload | Valid for |
|---|---|---|---|
| `~+ expr` | Trit::Positive | `expr` (must match T) | `T~E` and `T?~E` |
| `~0` | Trit::Zero | none | `T?~E` only |
| `~- expr` | Trit::Negative | `expr` (must match E) | `T~E` and `T?~E` |

### 2.2 — Spacing requirement (style guide MANDATORY)

The `~+`, `~-`, `~0` constructors **MUST be written with a space** between the marker and the expression in all source code:

```triet
return ~+ value          // GOOD
return ~+value           // PARSER OK, STYLE-GUIDE VIOLATION (rejected by `dao fmt`)

return ~- IoError::Invalid(path)  // GOOD

return ~+ -1             // GOOD — `~+` constructs outcome, `-1` is the negative integer payload
return ~+-1              // CONFUSING (parser accepts but rejected by `dao fmt`)
```

Lexer treats `~+`, `~-`, `~0` as compound prefix tokens — whitespace inside compound is forbidden (`~ +` is not the same as `~+`). Style guide enforces space *after* the compound prefix.

Rationale: the example `return ~+ -1` would be hard to read without the space. Mandatory space costs nothing and dramatically improves readability when the success payload is itself signed or includes operators.

### 2.3 — Examples

```triet
function read_file(path: String) -> String~IoError {
    if path.is_empty() {
        return ~- IoError::InvalidArgument("empty path")
    }
    let contents = std.io.fs.read(path)
    return ~+ contents
}

function lookup_symbol(name: String) -> Symbol?~IoError {
    let cache = std.io.fs.read("symbols.cache") ~-> |io_err| return ~- io_err
    if not_in_cache(cache, name) {
        return ~0
    }
    return ~+ parse_symbol(cache, name)
}
```

### 2.4 — Type inference for constructors

The compiler infers the payload type from the constructor argument and matches it against the function's declared return type:

- `~+ value` — `typeof(value)` must be compatible with `T` in declared `T~E` / `T?~E`.
- `~- error` — `typeof(error)` must be compatible with `E`.
- `~0` — only valid when the declared return type allows null state (i.e., `T?~E`). Otherwise → E1025.

```text
E1025 NullStateInBinaryOutcome
    `~0` constructor requires outcome type with null state (`T?~E`).
    Declared return type is `T~E` (binary), which has no null state.
    
    --> src/example.tri:7:12
       |
    7  |     return ~0
       |            ^^ null constructor in binary-outcome context
    
    Suggested fixes:
    
    [Fix 1] Use an explicit failure value instead of null:
    Change `return ~0` to `return ~- <default_error>`
    
    [Fix 2] Change function return type to allow the null state:
    Change `T~E` to `T?~E` in the function return-type annotation
```

## §3 — Ternary operator family: `~+>` / `~0>` / `~->`

**Lock (author 2026-05-26):** Three postfix operators, mỗi cái target **đúng 1 trit state** của outcome discriminator. Đối xứng hoàn hảo với constructor family `~+` / `~0` / `~-` từ §2. **No operator for force-unwrap** — dangerous extraction is method-only (§4) per `feedback_explicit_strictness`. **No legacy `~?` or `~:` operators** — design session 2026-05-26 chốt full migration sang ternary family vì brand-clean, AI-friendly, không redundant.

| Operator | Target trit state | Default auto-wrap | Use case |
|---|---|---|---|
| `expr ~+> \|val\| body` | Positive | `~+` (success) | Transform success payload (Functor map) |
| `expr ~0> body` | Zero (null) | `~+` (recover null → success) | Substitute default cho null; chỉ valid cho `T?~E` |
| `expr ~-> \|err\| body` | Negative | `~-` (error) | Propagate, recover, hoặc transform error |

Mỗi operator chỉ "fire" trên arm tương ứng; arm khác **pass through** unchanged. Chain nhiều operator để xử lý nhiều arm.

### 3.0 — Auto-wrap rule (operator-context constructor inference)

**Lock (author 2026-05-26):** Inside body của arm-specific operator, value returned (qua `return` statement hoặc tail expression) **tự động wrap** với constructor tương ứng arm của operator. Explicit `~+` / `~0` / `~-` chỉ cần khi **override** sang arm khác.

| Operator | Body's plain value → auto-wrap | Body returns full outcome (matches outer type) | Override sang arm khác |
|---|---|---|---|
| `~+>` | `~+ value` | Use as-is | Explicit `~- err` hoặc `~0` |
| `~0>` | `~+ value` (recover null → success) | Use as-is | Explicit `~- err` hoặc `~0` (stay null) |
| `~->` | `~- value` | Use as-is | Explicit `~+ val` (recover) hoặc `~0` |

**Lý do:**

1. **Operator name carries semantic.** `~->` đã chỉ định "error arm context"; viết `return ~- err` trong body là redundant. Bỏ `~-` redundant cho code cleaner.
2. **Precedent đã có.** ADR-0020 §10.4 lock implicit `T ⊂ T?` widening — auto-wrap là extension cùng nguyên lý: constructor inference khi context unambiguous.
3. **Brand-coherent.** `feedback_explicit_strictness` áp dụng cho **panic-possible ops** (force-unwrap với message argument). Auto-wrap chỉ là constructor inference từ context — không panic, không hidden control flow.
4. **AI-friendly.** Operator carry context info → AI generate `return X` ngắn hơn, đúng intent.

**Examples:**

```triet
// OLD verbose (deprecated style):
let cfg = parse_config(input) ~-> |e| return ~- AppError.parse(e)

// NEW auto-wrap (canonical):
let cfg = parse_config(input) ~-> |e| return AppError.parse(e)
//                                           ^^^^^^^^^^^^^^^^^^ auto-wrap ~-

// Tail expression form (no `return`):
let v = parse(input) ~-> |e| AppError.parse(e)
//                           ^^^^^^^^^^^^^^^^ auto-wrap ~-

// Override khi recover:
let v: Integer = parse(input) ~-> |_| return ~+ 0
//                                           ^^^^ explicit ~+ override
```

**Edge case 1: T ≡ E (success type same as error type)**

Hiếm gặp (code smell). Compiler force explicit để tránh ambiguity:

```triet
function ambiguous() -> Integer~Integer {
    inner() ~-> |e| return e   // E1039 AmbiguousAutoWrap
}
```

→ E1039 fires (định nghĩa §9.4). Author phải gõ `return ~- e` (hoặc `~+ e` nếu intent recover).

**Edge case 2: Body returns full outcome trực tiếp**

```triet
function outer() -> Integer~AppError {
    inner() ~-> |e| return alternative()
    //                     ^^^^^^^^^^^^^ alternative() returns Integer~AppError
}
```

Compiler check return value type:
- Matches outer outcome (Integer~AppError) → **use as-is**, không double-wrap.
- Matches payload type only (E = AppError) → auto-wrap `~-`.
- Matches success type (T = Integer) → would auto-wrap `~+`, nhưng đây là `~->` operator → conflict; require explicit override.

**Edge case 3: Block body với nhiều `return`**

```triet
inner() ~-> |e| {
    log(e)
    if can_recover(e) {
        return ~+ default_value   // explicit ~+ override (recovery, early-return)
    }
    return enriched(e)             // auto-wrap ~- (matches operator arm)
}
```

Mỗi `return` typecheck độc lập. Auto-wrap khi value matches operator arm payload; explicit khi override.

### 3.0.1 — Two modes: MAP vs EARLY-RETURN (consistent across all operators)

**Lock:** Mọi operator (`~+>`, `~0>`, `~->`) hỗ trợ **2 modes** body. Sự khác biệt KHÔNG nằm ở operator mà ở việc body có dùng `return` keyword hay không.

| Mode | Cú pháp body | Tác động lên outcome chain | Outer binding type |
|---|---|---|---|
| **MAP (tail expression)** | Tail expr, KHÔNG `return` | Body's value thay arm value, **chain tiếp tục** | Outcome type (full discriminator) |
| **EARLY-RETURN (with `return`)** | `return <value>` | Body's value làm **function exit ngay** | Unwrapped success type (path non-exit) |

**Auto-wrap rule §3.0 áp dụng cho CẢ 2 modes** — body's plain value tự wrap với operator's arm constructor.

#### Mode 1 minh họa — MAP (no `return`)

```triet
// Pure map chain: tất cả operators dùng tail-expr form
let final_outcome: NormalizedString?~WrapErr = parse(input)
    ~+> |v| v.normalize().to_string()    // map success: Config → NormalizedString
    ~-> |e| WrapErr.from(e)               // map error: ParseError → WrapErr
    ~0> default_value()                   // map null: convert to success default

// final_outcome VẪN là outcome — chưa unwrapped.
// Caller phải pattern match hoặc dùng `.unwrap_value(message)` method.
```

#### Mode 2 minh họa — EARLY-RETURN (with `return`)

```triet
function run() -> Output~AppError {
    let cfg = parse_config(input) ~-> |e| return AppError.parse(e)
    //                                    ^^^^^^^^^^^^^^^^^^^^^^^^^^ exit function nếu error
    //                                    cfg = unwrapped Config trên path success
    
    let result = compute(cfg) ~+> |v| return ~+ v.shortcut()
    //                                ^^^^^^^^^^^^^^^^^^^^^^^^^ exit function với value mới (rare)
}
```

#### Pattern thực tế: Mixed modes

Phổ biến nhất là **mix 2 modes** trong cùng chain:

```triet
function run() -> Output~AppError {
    let cfg = parse_config(input)
        ~-> |e| return AppError.parse(e)         // Mode 2: propagate error (exit)
    // cfg = unwrapped Config
    
    let result = process(cfg)
        ~+> |v| v.normalize()                     // Mode 1: map success (continue)
        ~+> |v| v.serialize()                     // Mode 1: chain another map
        ~-> |e| return AppError.processing(e)    // Mode 2: propagate at end
    // result = unwrapped serialized value
    
    return ~+ result
}
```

**Asymmetry là về intent, KHÔNG về syntax.** Typical usage:
- `~+>` thường MAP (transform success → tiếp tục dùng)
- `~->` thường EARLY-RETURN (error → exit function, propagate up)

Cả 2 operators support cả 2 modes — author chọn dựa trên intent. Compiler typecheck consistent regardless.

#### Chú thích trong examples từ §3.1 trở đi

Để rõ ràng, các examples sau dùng comment label:
- `// Mode 1 MAP` cho tail-expr form
- `// Mode 2 EARLY-RETURN` cho return form

### 3.1 — `~+>` success-arm transformer

```triet
let value = expression ~+> |bind| body
```

**Semantics:**

- Evaluate `expression` (type `T~E` hoặc `T?~E`).
- Nếu `~+ payload` → bind `payload` to `bind`, evaluate `body`, dùng kết quả thay thế success arm.
- Nếu `~0` hoặc `~- err` → **pass through unchanged**.

**Body return:**

- Plain `T'` → auto-wrap thành `~+ T'`. Success type chuyển từ T sang T'.
- Outcome `T'~E` hoặc `T'?~E` (same error type) → flatten; nested outcome unfolded.
- Early-return form (`return ...`, `panic(...)`) → exit enclosing function.

**Examples (Mode 1 MAP — common cho `~+>`):**

```triet
let normalized = parse(input) ~+> |v| v.normalize()
//   Mode 1 MAP: tail expr, auto-wrap ~+
//   parse returns Config~ParseError
//   result type: Config~ParseError (success type unchanged, just transformed)

let str_count = read_file(path) ~+> |contents| count_chars(contents)
//   Mode 1 MAP: tail expr, auto-wrap ~+
//   read_file returns String~IoError; ~+> transforms String to Integer
//   result type: Integer~IoError (success type changed)
```

**Example (Mode 2 EARLY-RETURN — rare cho `~+>`):**

```triet
function shortcut_lookup() -> Result~Err {
    let v = compute() ~+> |val| return ~+ val.optimized()
    //                          ^^^^^^^^^^^^^^^^^^^^^^^^^^ exit function với value mới
    // ... regular logic chỉ chạy nếu compute() failed
}
```

**Discard underscore:** Body có thể dùng `|_|` để discard payload và trả về fixed value:

```triet
let success_flag: Trilean = operation() ~+> |_| true   // Mode 1 MAP
```

### 3.2 — `~0>` null-arm transformer (T?~E only)

```triet
let value = expression ~0> body
```

**Semantics:**

- Evaluate `expression` (type **must be** `T?~E` ternary outcome).
- Nếu `~0` (null) → evaluate `body`, dùng kết quả thay thế null arm.
- Nếu `~+ payload` hoặc `~- err` → **pass through unchanged**.

**Body return:** Plain `T` (auto-wrap `~+`), outcome trực tiếp, hoặc early-return form. No closure capture (null arm carries no payload).

**Type restriction:** Sử dụng `~0>` trên `T~E` (binary, không có null arm) → **E1025 NullStateInBinaryOutcome**.
(Nguyên bản ADR §3.2 gán E1037; E1037 bị APP.2b chiếm cho `ArmHandlerMapModeRejected` — "body must be Bậc A scalar". E1025 tái dùng cho cùng bản chất "null operation on binary".)

**Examples:**

```triet
// Provide default for null
let user: User~DbError = find_user(id) ~0> anonymous_user()
//   find_user returns User?~DbError
//   ~0> fires on null → substitute anonymous_user()
//   ~+ và ~- pass through
//   result: User~DbError (null arm eliminated → binary)

// Propagate null up to caller
function lookup_chain(id: UserId) -> Profile?~LoadError {
    let user = find_user(id) ~0> return ~0       // propagate null
    let profile = load_profile(user) ~0> return ~0
    return ~+ profile
}
```

### 3.3 — `~->` error-arm handler

```triet
let value = expression ~-> |bind| body
```

**Semantics:**

- Evaluate `expression` (type `T~E` hoặc `T?~E`).
- Nếu `~- error` → bind `error` to `bind`, evaluate `body`, dùng kết quả thay thế error arm.
- Nếu `~+ payload` hoặc `~0` → **pass through unchanged**.

**Body return (per §3.0 auto-wrap):**

- Plain `E` (error type) → auto-wrap `~- E`. Default behavior (propagation/transform).
- Plain `T` với explicit `~+` → wrap `~+ T`. Recovery (override, error arm eliminated).
- Outcome trực tiếp (matches outer type) → use as-is.
- Early-return form → exit enclosing function.

**Examples (Mode 2 EARLY-RETURN — common cho `~->`):**

```triet
// Propagate error verbatim (auto-wrap ~-, function exits)
let v = inner() ~-> |e| return e

// Transform error type (auto-wrap ~-)
let v = inner() ~-> |e| return OuterError.from(e)

// Recover with default (explicit ~+ override, error arm eliminated)
let count: Integer = parse_count(input) ~-> |_| return ~+ 0

// Add context to propagated error (auto-wrap ~-)
let cfg = parse_config(s) ~-> |e| return e.context("config phase")
```

**Example (Mode 1 MAP — khi muốn transform error type và chain tiếp tục):**

```triet
// Map error type, chain continues with new outcome
let outcome: Config~AppError = parse_config(s) ~-> |e| AppError.from(e)
//   Mode 1 MAP: tail expr, auto-wrap ~-
//   parse_config returns Config~ParseError
//   ~-> transforms ParseError to AppError, chain continues
//   outcome type: Config~AppError (vẫn outcome, chưa unwrap)
```

**Explicit type conversion required.** Khi inner error type `E_inner` ≠ caller's `E_outer`, body phải construct outer error explicitly (compiler không tự convert giữa E types). Không có implicit `From` magic (Rust `?` làm vậy; Triết refuses-over-guess per VISION §6). Auto-wrap chỉ apply constructor `~-`, không apply conversion giữa E types.

```text
// VALID — explicit conversion (auto-wrap ~- applied):
expression ~-> |inner_err| return OuterError.from(inner_err)

// INVALID — no implicit conversion AND no capture:
expression ~->   // E1030: closure capture form required
```

### 3.4 — Chaining: composition of arm handlers

Các operator chain left-to-right. Mỗi operator "tiêu thụ" 1 arm; chain xử lý đủ arm sẽ "narrow" outcome type:

```triet
function run() -> Output?~AppError {
    let cfg = parse_config(input)
        ~-> |e| return AppError.parse(e)           // Mode 2 EARLY-RETURN, auto-wrap ~-
    // cfg = unwrapped Config (error path exited above)
    
    let data = read_data(cfg.path)
        ~-> |e| return AppError.io(e)              // Mode 2 EARLY-RETURN, auto-wrap ~-
        ~0> return ~0                              // Mode 2 EARLY-RETURN, explicit ~0 propagate
    // data = unwrapped success value
    
    let result = process(data)
        ~+> |v| v.normalize()                       // Mode 1 MAP, tail expr auto-wrap ~+
        ~-> |e| return AppError.processing(e)      // Mode 2 EARLY-RETURN, auto-wrap ~-
    // result = unwrapped normalized value
    
    return ~+ result
}
```

Chain mix Mode 1 (`~+>` map success type qua `.normalize()`) và Mode 2 (`~->` propagate error). Comments label rõ intent.

**Type narrowing through chain:**

```triet
let outcome_initial: Config?~ParseError = parse_optional_config(s)

let outcome_after_null_default: Config~ParseError =
    outcome_initial ~0> Config.default()           // ~0 eliminated → binary

let val_recovered: Config =
    outcome_after_null_default ~-> |_| Config.default()  // ~- eliminated → bare value
```

Compiler tracks remaining arms qua flow analysis; final type = subset của initial trit state space dựa trên handler nào "consume" arm nào.

### 3.5 — Capture syntax + discard underscore

Cú pháp `|bind|` cho `~+>` và `~->` reuse từ closure parameter form (sẽ chính thức hóa ADR closure tương lai). Hiện tại không có first-class closure expose form này, nhưng ternary outcome operators claim precedent.

`|_|` form discard payload — match wildcard convention từ Triết `match` arms.

`~0>` **không có capture form** vì null arm không có payload. Cú pháp `~0> |_| body` → **E1038 NullArmHasNoPayload** (syntax error).

### 3.6 — Operator precedence and chaining

Cả 3 operators bind **lower** hơn method call/field access, **higher** hơn assignment:

```triet
let value = outcome.try_value() ~-> |_| default()    // method first, then ~->
let value = (outcome ~-> |_| default).field          // ~-> first → REQUIRES PARENS
```

**Left-associative chaining:** `a ~+> f ~-> g ~0> d` parse như `((a ~+> f) ~-> g) ~0> d`.

Style guide: parenthesize khi chain operator với field access hoặc method call cho readability. Format chuẩn: mỗi operator trên 1 dòng riêng khi chain ≥ 2 (xem example §3.4).

### 3.7 — Migration từ `~?` / `~:` (deprecated)

Pre-2026-05-26 ADR-0020 có `~?` (propagate) và `~:` (default). Author chốt full migration:

| Old (deprecated) | New (canonical, with auto-wrap §3.0) |
|---|---|
| `expr ~? \|e\| return ~- e` | `expr ~-> \|e\| return e` |
| `expr ~? \|e\| return ~- WrapErr(e)` | `expr ~-> \|e\| return WrapErr(e)` |
| `expr ~: default` (cho `T~E`) | `expr ~-> \|_\| return ~+ default` |
| `expr ~: default` (cho `T?~E`, same default cả 2 arm) | `expr ~0> default ~-> \|_\| return ~+ default` |

**Tool migration:** `dao fmt --fix --migrate-outcome-ops` (planned v0.7.4.3-error.4) tự rewrite. Implementation v0.7.4.3-error.3c chưa ship → migrate trước khi user gặp `~?`/`~:` trong production code.

Lexer dứt khoát refuse parse bare `~?` và `~:` tokens (chỉ accept `~+>`, `~0>`, `~->` compound) từ implementation v0.7.4.3-error.3c trở đi. Vì design vẫn pre-ship, không cần warning period — break the symbol immediately. Test corpus + stdlib + examples migrate trong same sub-task.

## §4 — Safe properties and dangerous methods

**Lock:** Per [`feedback_explicit_strictness.md`](../../README.md) — property access is 100% safe contract; panic-possible operations are verbose methods with mandatory message argument.

### 4.1 — Safe properties (no panic, no message)

Three `Trilean`-typed properties expose outcome state. All return strict 2-state Trilean (True / False, never Unknown — per Q3-A precedent from v0.7.3.3):

```triet
outcome.is_success   // Trilean — True if Trit::Positive, False otherwise
outcome.is_null      // Trilean — True if Trit::Zero (T?~E only; always False for T~E)
outcome.is_error     // Trilean — True if Trit::Negative, False otherwise
```

For `T~E`, `is_null` always returns `False` (the Zero state is invalid and would have panicked at construction or runtime). The property still exists for syntactic uniformity — caller code that handles both `T~E` and `T?~E` polymorphically need not change.

### 4.2 — Safe extraction (returns nullable)

Two methods convert outcome to nullable, never panic:

```triet
outcome.try_value() -> T?     // Some(value) if success, null otherwise
outcome.try_error() -> E?     // Some(error) if failure, null otherwise
```

Caller chains with existing `T?` ergonomics:

```triet
let value: Integer = outcome.try_value() ?: 0
let error_message: String = outcome.try_error()?.message() ?: "no error"
```

For `T?~E`, `try_value()` returns `null` for BOTH the null state and the error state — caller distinguishes via `is_error` if needed. This is a deliberate flattening; if granular distinction is required, use pattern match (§5).

### 4.3 — Dangerous methods (panic-possible, REQUIRE message argument)

Two methods extract payload with panic on wrong state. Both require a `String` message argument explaining why the caller believes the panic-condition is impossible:

```triet
outcome.unwrap_value(message: String) -> T
    // Returns T if success. Panics with `message` if not success.

outcome.unwrap_error(message: String) -> E
    // Returns E if failure. Panics with `message` if not failure.
```

The mandatory `message: String` parameter is the explicit-strictness contract: reading `outcome.unwrap_value("config must exist after init check")` immediately tells the next developer that this is panic-possible code path with stated invariant. Java's `Optional.get()` (no message) is the anti-pattern this defends against.

**No shorter alias is provided.** A `.unwrap()` (no message) variant would tempt callers to skip the explanation, defeating the principle.

### 4.4 — Why no force-unwrap operator?

Earlier draft proposed `outcome~~` as parallel to `!!` null-unwrap. Author 2026-05-17 explicitly rejected: dangerous operations MUST be verbose to remain visible at call sites. The `~~` operator would be too easy to scatter through code without thinking — exactly the failure mode this design prevents.

Note: `!!` on `T?` remains as is. It is a historical primitive on the language's first-class nullable type, predates this design principle, and stays for compatibility. New struct-like APIs (Outcome, future container types) follow the stricter rule.

## §5 — Pattern matching

**Lock:** Outcome values pattern-match using the same constructor forms `~+`, `~0`, `~-`. Style guide mandates space.

```triet
match read_file("config.toml") {
    ~+ contents => use(contents),
    ~- error    => log_error(error),
}

match lookup_symbol("foo") {
    ~+ symbol => println(f"found: {symbol}"),
    ~0        => println("not found"),
    ~- error  => println(f"i/o error: {error}"),
}
```

### 5.1 — Exhaustiveness

The typechecker enforces exhaustive match per existing SPEC §7.3 rules:

- For `T~E`: match must cover `~+` and `~-` arms (the `~0` arm is structurally absent).
- For `T?~E`: match must cover `~+`, `~0`, and `~-` arms.
- Wildcard `_` arm covers any remaining states.

```triet
match outcome {              // outcome: String~IoError (binary)
    ~+ value => use(value),
    // ERROR E1026: non-exhaustive match — missing `~-` arm
}

match outcome {              // outcome: Symbol?~IoError (ternary)
    ~+ symbol => use(symbol),
    ~- err    => log(err),
    // ERROR E1026: non-exhaustive match — missing `~0` arm
}
```

```text
E1026 NonExhaustiveOutcomeMatch
    Match on outcome type `T?~E` does not cover all states.
    Missing arm: `~0` (null state).
    
    --> src/example.tri:5:5
       |
    5  |     match outcome {
       |     ^^^^^ match is non-exhaustive
    6  |         ~+ value => use(value),
    7  |         ~- error => log(error)
       |
       = note: `~0` arm not covered
    
    Suggested fixes:
    
    [Fix 1] Add an explicit arm for the null state:
    Add a `~0 => <handler>` arm to the match
    
    [Fix 2] Use a wildcard catch-all (only when null and error share a handler):
    Add `_ => <handler>` as the last arm
```

### 5.2 — Pattern binding

The expression after `~+` or `~-` in a pattern position is a **binding name** (or literal for value-equality match):

```triet
match outcome {
    ~+ 0      => println("zero!"),         // literal match — success with value 0
    ~+ value  => println(f"positive: {value}"),
    ~- error  => log(error),
}
```

The `~0` arm takes **no expression** (the null state has no payload).

## §6 — Trit::Zero policy

**Lock:** The Zero state of the outcome discriminator has three valid interpretations depending on phase:

| Phase | Type | Zero state means |
|---|---|---|
| v0.7+ | `T~E` (binary) | **Invalid.** Typecheck rejects construction (`~0` for binary type → E1025). Runtime encountering Zero in binary outcome → panic E2210 `InvalidOutcomeState`. |
| v0.7+ | `T?~E` (ternary) | **Valid null state.** Constructed via `~0`. |
| v0.8+ | `T~E` (binary) | **Reserved for actor model "pending" state** (async I/O not yet complete). Not yet implementable; placeholder. |

Refuse-over-guess: v0.7 does not allow `~0` in binary outcome types. The slot is reserved, not silently coerced.

```text
E2210 InvalidOutcomeState
    Outcome value of type `T~E` has discriminator Trit::Zero,
    which is reserved. This indicates corrupt wire data or a
    future-version pending state encountered by a pre-v0.8 reader.
```

(No span or fix block — runtime corruption diagnostic per [ADR-0027 §3](0027-diagnostic-format-standard.md). User code did not cause this; file a bug report or update reader version.)

## §7 — Wire format: `.triv` v4 → v5 patch bump

**Lock:** New `TypeTag::Outcome` variant encoded with discriminant 10 (extending the v0.7.3.1 collection discriminants 8/9). Patch bump per [ADR-0008 §"Version compatibility"](0008-triv-binary-format.md) (additive type discriminants).

### 7.1 — Type table encoding

```text
TypeTag::Outcome encoding:
    discriminant       : u8         = 10
    allow_null_state   : u8         = 0 (T~E) | 1 (T?~E)
    value_type_index   : LEB128 u32 → references types table entry
    error_type_index   : LEB128 u32 → references types table entry

Total: 1 + 1 + (varint × 2) bytes per outcome type entry.
```

Like Vector (discriminant 8) and HashMap (discriminant 9), the inner type indices are **post-order** — the value type and error type entries must precede the Outcome entry in the type table. The `add_type` helper in `triet-ir/src/serde.rs` already implements this pattern; v0.7.4.3-error extends to cover the new Outcome composite.

### 7.2 — Constant pool

Outcome values **do not appear in the constant pool**. They are constructed at runtime via the constructor instructions. The constant pool reader/writer rejects Outcome types with the same error as Vector/HashMap (v0.7.3.1):

```text
TrivError::Corrupted(
    "Outcome has no constant-pool encoding — outcome values are built at runtime
     via constructor opcodes (ADR-0020 §2)"
)
```

### 7.3 — Constructor / dispatch opcodes

Two new IR opcodes added to handle outcome construction. Sub-version additive within `.triv` v5 (no further bump needed):

| Opcode | Mnemonic | Operands | Semantic |
|---|---|---|---|
| 0xC1 | `OUTCOME_NEW_POSITIVE` | `dest: varint, payload: operand` | Build outcome with Trit::Positive arm. |
| 0xC2 | `OUTCOME_NEW_NEGATIVE` | `dest: varint, payload: operand` | Build outcome with Trit::Negative arm. |
| 0xC3 | `OUTCOME_NEW_NULL` | `dest: varint` | Build outcome with Trit::Zero arm (T?~E only). |
| 0xC4 | `OUTCOME_DISCRIMINANT` | `dest: varint, source: operand` | Read trit discriminator → Trit value (`-1`/`0`/`+1`). |
| 0xC5 | `OUTCOME_UNWRAP_VALUE` | `dest: varint, source: operand` | Extract success payload; panic E2210 if not Positive. |
| 0xC6 | `OUTCOME_UNWRAP_ERROR` | `dest: varint, source: operand` | Extract failure payload; panic E2210 if not Negative. |

`OUTCOME_DISCRIMINANT` is the lowering target for the safe properties (`.is_success`, `.is_null`, `.is_error`) and for pattern match dispatch (which becomes a `BR_TRILEAN` on the discriminator). It is also the lowering target for the ternary operator family `~+>`/`~0>`/`~->` (§3) — each operator lowers to a discriminator check plus a branch into the matching-arm handler.

### 7.4 — Pre-v5 reader behavior

Pre-v0.7.4.3 readers encountering type discriminant 10 emit `TrivError::UnknownTypeDiscriminant` (E2104). Pre-v0.7.4.3 readers encountering opcodes 0xC1–0xC6 emit `TrivError::UnknownOpcode` (E2105). Same forward-compat contract as v0.7.3.1 Vector/HashMap (additive primitives, refuse on unknown).

## §8 — Coexistence with existing `Result<T, E>`

**Lock:** Existing `std.result::Result<T, E>` enum (v0.4) is NOT removed. New `T~E` / `T?~E` is the **primary error mechanism** for code from v0.7.4.3 onward. Legacy `Result<T, E>` stays for:

1. **Backwards compatibility** — pre-v0.7.4.3 code using `Result<T, E>` continues to work unchanged.
2. **User-defined structural enums** — when a user needs an algebraic-data-type with custom variants and methods, `Result<T, E>` as a generic enum (or any user-defined enum) is the right tool.
3. **Cross-package APIs where author prefers explicit struct shape** — `Result<T, E>` is a regular enum, serializes deterministically, and integrates with pattern match. Authors may keep using it.

### 8.1 — Migration policy

- **No automatic rewriting tool.** Authors who want to migrate `Result<T, E>` call sites to `T~E` do so manually. The two types are not auto-convertible — typecheck E1027 if mixed without explicit conversion.
- **Stdlib stubs v0.7.3 are NOT migrated.** They already use `T?` and `Trilean` returns (per Q4-A IO strict 2-state). No `Result<T, E>` exposure currently.
- **Self-host compiler v0.7.4.3+ adopts `T~E`** as the primary form. No `Result<T, E>` in `compiler/*.tri` source.

### 8.2 — Conversion utilities (deferred)

A `std.outcome` module providing `Result<T, E> → T~E` and reverse conversions is **deferred**. When the first concrete use case appears (likely a multi-package compile where one package uses `Result` and another `T~E`), open a sub-task in `v0.7.x.review` to add the converters. Refuse-over-guess: do not pre-build infrastructure for hypothetical migration.

### 8.3 — Documentation policy

SPEC §2.5 (nullable + error-handling primary section) is updated in v0.7.4.3-error commit to document:
1. `T?` for absent values (primary).
2. `T~E` / `T?~E` for fallible operations (primary, **new**).
3. `Trilean` for Ł3 semantic uncertainty (primary).
4. `Result<T, E>` for structural enum needs (legacy, valid).
5. Runtime panic (bug-tier; not recoverable).

## §9 — Pattern match codegen + type inference details

### 9.1 — Lowering match-on-outcome

Match-on-outcome lowers to `OUTCOME_DISCRIMINANT` → `BR_TRILEAN` (existing v0.3.x.ternary opcode). Three branches map directly to the three arms; binary-only outcomes use the standard `BR_TRILEAN` with the Zero arm pointing to an `UNREACHABLE` opcode (because Trit::Zero is invalid for `T~E` per §6).

This reuses the v0.3.x.ternary ternary-branch infrastructure — no new control-flow primitive needed.

### 9.2 — Type inference rules

When the typechecker encounters a call to a function declared to return `T~E`:

- Caller using `~->` with `return ~- err` body: typecheck checks that the caller's own return type is fallible (else E1028 PropagateInNonFallibleContext), and that the inner E is compatible with the caller's error type (else E1029 ErrorTypeMismatch). `~+>` and `~0>` with `return ~+ x` / `return ~0` bodies follow the same fallible-context rule.
- Caller using `~:`: typecheck checks default expression type matches T.
- Caller using `match`: typecheck enforces exhaustiveness (§5.1).
- Caller using `.unwrap_value(msg)`: returns T, no type-system fence.
- Caller using `.try_value()`: returns `T?`.
- Caller using `.is_success`/`.is_null`/`.is_error`: returns `Trilean`.

```text
E1027 OutcomeTypeMismatch
    Cannot mix `Result<T, E>` (legacy enum) and `T~E` (primitive)
    without explicit conversion.
    
    --> src/example.tri:4:18
       |
    4  |     let x: T~E = legacy_result_value
       |                  ^^^^^^^^^^^^^^^^^^^ Result type, T~E expected
    
    Suggested fixes:
    
    [Fix 1] Convert from Result to outcome at the boundary:
    Wrap the value in `legacy_result_value.to_outcome()`
    
    [Fix 2] Migrate the source API to return T~E directly:
    Refactor the producing function's return type: change `Result<T, E>` to `T~E`
```

```text
E1028 PropagateInNonFallibleContext
    Operator `~->` (with auto-wrap propagation body) requires the enclosing function
    to have a fallible return type (`T~E` or `T?~E`).
    Function `foo` declared return type `Integer`.
    
    --> src/example.tri:6:20
       |
    3  | function foo() -> Integer {
       |                   ------- non-fallible return type
    ...
    6  |     let v = expr ~-> |err| return err
       |                  ^^^ propagate in non-fallible function (return auto-wraps to ~- err)
    
    Suggested fixes:
    
    [Fix 1] Make the enclosing function fallible:
    Change `Integer` to `Integer~SomeError` in the function return-type annotation
    
    [Fix 2] Handle the outcome locally with match instead of propagating:
    Replace `expr ~-> |err| return err` with an explicit `match expr { ~+ v => ..., ~- e => ... }`
    
    [Fix 3] Recover from the error instead of propagating (eliminates the error arm):
    Replace `~-> |err| return err` with `~-> |_| return ~+ <default_value>` to provide a fallback
```

```text
E1029 ErrorTypeMismatch
    Outcome error type mismatch in `~->` propagation: inner outcome has `E_inner`,
    caller expects `E_outer`. Auto-wrap applies the `~-` constructor but does NOT
    convert between error types.
    
    --> src/example.tri:8:13
       |
    8  |     let v = parse() ~-> |err| return err
       |             ^^^^^^^ inner returns ParseError, caller expects IoError
    
    Suggested fixes:
    
    [Fix 1] Convert the inner error type at the propagate boundary:
    Change `return err` to `return IoError.from(err)`
    
    [Fix 2] Unify error types across the call chain:
    Refactor the inner function's return-type annotation to use the same error type as the caller
```

### 9.3 — Explicit closure capture in `~+>` / `~->` right-hand side

Section 3.1 và 3.3 locks `|binding_name|` capture form trên RHS của `~+>` và `~->` (riêng `~0>` không có capture vì null arm không có payload — xem E1038). Typecheck rules:

1. Lexer/parser produces an `OutcomeArmHandler { inner_expr, target_arm, capture_name, body }` AST node from each source operator. `target_arm` ∈ { Positive, Zero, Negative }; `capture_name` is None khi `target_arm == Zero`.
2. Inside `body` typecheck scope, the parser pushes a frame and declares the captured binding (name = `capture_name`):
   - Cho `~+>`: binding type = `T` (inner outcome's success payload type)
   - Cho `~->`: binding type = `E` (inner outcome's failure payload type)
   - Cho `~0>`: no binding declared
3. `capture_name` may be `_` to discard the payload — typecheck does not declare a binding in that case; references to `_` inside the form are a separate error per existing wildcard rules.
4. If `capture_name` shadows an outer variable, this is treated identically to a regular `let capture_name = ...` shadow — no special-case shadowing rule; the developer is responsible for picking a non-conflicting name.
5. The binding is read-only (cannot be reassigned within the form) and goes out of scope when the form ends.

**No implicit magic.** Triết has zero implicit bindings — the developer always sees the name they're using. This matches author's clean-code principle: every variable in scope is traceable to a `let`/`function param`/`|capture|` site. Connects to [`feedback_explicit_strictness.md`](../../README.md) — explicit > convenient.

```text
E1030 OutcomePropagateMissingCapture
    Operators `~+>` and `~->` require explicit closure capture form on the
    right-hand side. Found bare statement — implicit payload bindings
    are not supported (per `feedback_explicit_strictness`).
    Note: `~0>` does not take a capture (null arm has no payload); see E1038.
    
    --> src/example.tri:5:22
       |
    5  |     let v = parse() ~-> return ~- DefaultError
       |                     ^^^ missing |capture_name| form
    
    Suggested fixes:
    
    [Fix 1] Name the captured payload explicitly:
    Change `~-> return ~- DefaultError` to `~-> |err| return ~- err`
    
    [Fix 2] Discard the payload explicitly with underscore:
    Change `~-> return ~- DefaultError` to `~-> |_| return ~- DefaultError`
```

```text
E1031 OutcomePropagateMalformedReturn
    A `~->` (or `~+>` / `~0>`) body whose result is not the outcome's own type
    must be a terminating form: `return`, `panic`, or a fully-evaluated outcome
    expression. Falling through with a bare expression that does not produce
    a compatible outcome leaves the chained binding unbound.
    
    --> src/example.tri:5:32
       |
    5  |     let v = parse() ~-> |err| log(err)
       |                              ^^^^^^^^ not a terminating form, log() returns Unit
    
    Suggested fixes:
    
    [Fix 1] Terminate the closure with an early return:
    Change `|err| log(err)` to `|err| { log(err); return ~- err }`
    
    [Fix 2] Use a panic when error is unrecoverable in the surrounding scope:
    Change `|err| log(err)` to `|err| panic("operation failed: {err}")`
    
    [Fix 3] Recover by returning a plain value of the success type (auto-wrapped as ~+):
    Change `|err| log(err)` to `|err| { log(err); default_value }`
```

### 9.4 — Ternary-family operator typecheck rules (new in 2026-05-26 revision)

```text
E1025 NullStateInBinaryOutcome
    (Nguyên bản E1037; đổi thành E1025 vì E1037 bị APP.2b chiếm — cùng bản chất.)
    Operator `~0>` targets the null arm (Trit::Zero), which only exists
    in ternary outcome type `T?~E`. Inner expression has type `T~E` (binary),
    which has no null arm.
    
    --> src/example.tri:7:25
       |
    6  | function parse(s: String) -> Integer~ParseError { ... }
       |                              ----------------- binary outcome (no null arm)
    7  |     let v = parse(input) ~0> default
       |                          ^^^ ~0> not applicable to binary outcome
    
    Suggested fixes:
    
    [Fix 1] Use `~-> |_|` to provide a default on the error arm (the only non-success arm here):
    Change `~0> default` to `~-> |_| default`
    
    [Fix 2] Change the inner function to return a ternary outcome if null is a meaningful state:
    Refactor inner return type: change `Integer~ParseError` to `Integer?~ParseError`
```

```text
E1038 NullArmHasNoPayload
    Operator `~0>` does not accept a closure capture form because the null arm
    (Trit::Zero) carries no payload to bind.
    
    --> src/example.tri:5:25
       |
    5  |     let v = optional() ~0> |x| default
       |                            ^^^ unexpected |...| capture on ~0>
    
    Suggested fixes:
    
    [Fix 1] Remove the capture clause (the null arm has no payload):
    Change `~0> |x| default` to `~0> default`
    
    [Fix 2] If you meant to handle the success arm, switch operator:
    Change `~0> |x| default` to `~+> |x| default`
```

```text
E1039 AmbiguousAutoWrap
    Auto-wrap (§3.0) cannot disambiguate the constructor because the outcome's
    success type `T` and error type `E` are the same. Compiler cannot infer
    whether `return v` means `~+ v` (recover) or `~- v` (propagate).
    
    --> src/example.tri:5:23
       |
    3  | function f() -> Integer~Integer { ... }
       |                 --------------- T == E (both Integer)
    ...
    5  |     let n = inner() ~-> |e| return e
       |                                    ^ ambiguous: ~+ e or ~- e?
    
    Suggested fixes:
    
    [Fix 1] Write the constructor explicitly to disambiguate (most common — propagate):
    Change `return e` to `return ~- e`
    
    [Fix 2] If you meant to recover, write the success constructor explicitly:
    Change `return e` to `return ~+ e`
    
    [Fix 3] Use distinct types for success and error (recommended — T == E is usually a code smell):
    Refactor the function signature so success and error have semantically distinct types
```

## §10 — Unification of `null` keyword with `~0` literal

**Lock (author 2026-05-17):** The historical `null` keyword (SPEC §1.5.x + §2.5, used as the Trit::Zero literal for `T?` nullable) is **unified with `~0`** — the same Trit::Zero literal introduced in §2 for outcome types. `~0` becomes canonical at every Trit::Zero state across the language; `null` is **deprecated** as a synonym with a clear removal timeline.

### 10.1 — Why unify

`null` and `~0` express the same semantic value: "Trit::Zero arm of a trit-encoded discriminator, no payload". Pre-ADR-0020 they were two syntactic forms for the identical underlying state:

- `T?` discriminator: `Trit::Positive` (T value) / `Trit::Zero` (null) / `Trit::Negative` (reserved)
- `T?~E` discriminator: `Trit::Positive` (T value) / `Trit::Zero` (null) / `Trit::Negative` (E error)

Keeping both spellings violates **refuse-over-guess** (one canonical form per concept) and the **balanced-ternary identity** principle (`~+`/`~0`/`~-` is a complete mathematical triple — adding `null` as a fourth way to spell Trit::Zero breaks the symmetry).

Trade-offs considered:

| Path | Rejected because |
|---|---|
| Keep both (`null` for T?, `~0` for T?~E only) | Two ways for one concept; linter would still need to pick canonical |
| Drop `~0`, use `null` everywhere | `null` has no parallel for positive/negative arms; 2-state vocabulary applied to 3-state design |
| **Drop `null`, use `~0` everywhere** (chosen) | Triết-native math identity; AI-first (`~0` distinct from training-data noise); pattern match unified across T? and T?~E |

### 10.2 — Canonical form: `~0` at every Trit::Zero site

```triet
// Nullable T? — assignment, comparison, return:
let user: User? = ~0
if found_user == ~0 { return ~0 }
function find_user(id: Integer) -> User? = if missing then ~0 else ~+ user

// Ternary outcome T?~E — same constructor:
function lookup(name: String) -> Symbol?~IoError {
    if io_error { return ~- IoError::CacheCorrupt }
    if not_in_cache { return ~0 }
    return ~+ symbol
}

// Pattern match — explicit `~0` arm in both contexts (§10.4):
match user {
    ~+ u => use(u),
    ~0   => log_missing(),
}

match lookup_result {
    ~+ symbol => use(symbol),
    ~0        => log_not_found(),
    ~- error  => log_error(error),
}
```

### 10.3 — Deprecation timeline (Q2 lock)

**v0.7.4.3-error (this ADR's implementation phase):**

- Lexer accepts both `null` and `~0` tokens.
- Parser normalizes both to the same AST node (`Expr::TritZero` or equivalent — implementation chooses).
- Typecheck emits warning **W2001 NullDeprecated** at every `null` token site with fix-hint *"replace `null` with `~0` (canonical Trit::Zero literal per ADR-0020 §10)"*.
- Existing code (examples, demos, stdlib stubs, anywhere using `null`) **continues to compile and run** with warnings.
- `dao fmt --fix --migrate-null` flag introduced — auto-rewrites every `null` → `~0` across a project tree.

**v1.0 (production stability cutoff):**

- `null` keyword **removed** from grammar.
- Warning W2001 promoted to error **E2002 NullRemoved** with same fix-hint.
- Migration tool (`dao fmt --fix --migrate-null`) ships with v1.0 release for one-shot cleanup of legacy codebases.

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

```text
E2002 NullRemoved    (active at v1.0+)
    `null` keyword is no longer valid. Use `~0` (canonical Trit::Zero
    literal per ADR-0020 §10).
    
    --> src/example.tri:5:23
       |
    5  |     let maybe_x: Integer? = null
       |                             ^^^^ removed keyword
    
    Suggested fixes:
    
    [Fix 1] Replace with canonical Trit::Zero literal:
    Change `null` to `~0`
    
    [Fix 2] Run the automated migration tool across the codebase:
    Use `dao fmt --fix --migrate-null` from project root
```

Per [ADR-0009 version gate policy](0009-version-gate-policy.md), no behavior breakage on minor bumps within v0.7.x — `null` keeps working until v1.0 freeze. This matches Triết's "stability over speed" principle: long migration window, automated tooling.

### 10.4 — `T?` widening rules (Q3 lock — allowed-not-required)

The existing `T ⊂ T?` implicit widening (SPEC §2.5) stays primary for the success arm:

```triet
let x: Integer? = 5         // implicit widening (preferred, terse)
let x: Integer? = ~+ 5      // explicit constructor (allowed, symmetric)
let x: Integer? = ~0        // null state (canonical)
```

In **pattern match arms**, the explicit form `~+ binding` is **MANDATORY** — pattern matching has no widening concept, so the positive arm must be spelled explicitly. The null arm always uses `~0`:

```triet
match maybe_user {
    ~+ user => greet(user),      // explicit positive arm
    ~0      => prompt_login(),    // null arm
}

// Wrong — pattern doesn't auto-widen `user` binding:
match maybe_user {
    user => greet(user),         // E1032: pattern binding does not implicitly widen
    ~0   => prompt_login(),
}
```

```text
E1032 PatternMissingExplicitConstructor
    Pattern arm for `T?` or `T?~E` type must use explicit `~+ binding`
    constructor — pattern matching does not perform `T ⊂ T?` widening.
    
    --> src/example.tri:5:5
       |
    5  |     user => greet(user),
       |     ^^^^ bare binding does not match T? success arm
    6  |     ~0   => prompt_login(),
    
    Suggested fixes:
    
    [Fix 1] Wrap the binding in the explicit positive constructor:
    Change `user => greet(user)` to `~+ user => greet(user)`
    
    [Fix 2] Use a wildcard when the value is not needed:
    Change `user => greet(user)` to `_ => greet(...)` (if greet does not need the unwrapped value)
```

### 10.5 — `dao fmt --fix --migrate-null` specification

The migration tool is a **non-trivial requirement** of this ADR — it carries the cost of unification across all user codebases. Implementation locked here:

1. **Token-level rewrite:** `null` → `~0` everywhere. No semantic analysis required (the unification is exact).
2. **Preserve formatting:** spaces, comments, line breaks adjacent to `null` token are preserved verbatim. Only the 4 characters `null` change to the 2 characters `~0` (with surrounding spaces handled per the existing `dao fmt` rules).
3. **In-place by default**, with `--dry-run` option for preview.
4. **Recursive directory traversal** when given a directory argument; respects `.gitignore` (mirror existing `dao fmt` behavior).
5. **Idempotent:** running `--migrate-null` twice produces no further changes.
6. **No-op on already-canonical files:** emit `No migration needed: <path>` per file with zero `null` tokens.
7. **W2001 warnings suppressed during migration run** — the tool's purpose is to fix them, no need to also report them.

**Acceptance criteria for v0.7.4.3-error:**
- All `examples/*.tri` files migrate cleanly (after audit, em estimate ~5-10 occurrences across the example set).
- All `std/*.tri` files migrate cleanly (em estimate ~3-5 occurrences).
- All `demos/**/*.tri` files migrate cleanly.
- Self-host compiler source (`compiler/*.tri`, when written from v0.7.4.3+) uses `~0` from day one — no `null` introduced.

### 10.6 — Not affected: Trilean `unknown` literal

Trilean (Ł3 logic) keeps its three named literals `true` / `false` / `unknown` — these are **domain-specific** vocabulary for logical truth values, distinct from outcome-state discriminators. No unification proposed.

Mental model split:

| Domain | Three states | Literals |
|---|---|---|
| Outcome (T?, T~E, T?~E) | success / null / failure | `~+ value` / `~0` / `~- error` |
| Logic (Trilean Ł3) | true / unknown / false | `true` / `unknown` / `false` |
| Numeric (Trit) | +1 / 0 / -1 | `1_trit` / `0_trit` / `-1_trit`, or `0t+` / `0t0` / `0t-` |

Three different naming systems for three different mental models. Author 2026-04 already locked this split when picking `unknown` over `null` for Trilean (SPEC §1.5.2). ADR-0020 §10 simply applies the same principle to outcome-state discriminators — they get `~+`/`~0`/`~-`, distinct from both Trit literals and Trilean literals.

### 10.7 — Implementation impact

**No IR / wire-format change.** The existing `Constant::Null` IR opcode (ADR-0010) already encodes "canonical Trit::Zero discriminator state of T?". It just gains an additional source-syntax form (`~0` in addition to `null`). The lowerer normalizes both source spellings to the same `Constant::Null` IR opcode.

ADR-0001 (nullable memory layout) and ADR-0010 (ternary-native IR) each receive a brief **Addendum** (no decision change, syntactic clarification only) — see [ADR-0001 Addendum — v0.7.4.3-error] and [ADR-0010 Addendum — v0.7.4.3-error].

**Implementation lands in v0.7.4.3-error sub-task** (lexer/parser/typecheck/diagnostic):
- Lexer accepts `~0` literal token.
- Parser produces same AST node from `null` and `~0`.
- Typecheck emits W2001 for every `null` token.
- New code paths and examples use `~0`; legacy `null` still works through v1.0.

Tracked in [ADR-0019 Addendum §A7](0019-self-hosting-compiler-bootstrap.md#a7--deferred-items-log-technical-debt-surfaced-by-v073) deferred items log under "Null keyword deprecation (W2001) + migration tool".

## Hệ quả

### For SPEC §2.5

Major rewrite. Section becomes "Nullable, Outcome, and error-handling primitives" with the 5-mechanism table from §8.3 above. Existing `T?` content preserved; new content explains `T~E` / `T?~E` semantics, constructor syntax, operators, and methods.

### For ADR-0003 (Iterator protocol)

`Iterator::next() -> T?` signature stays. The Iterator trait is about absence-of-next-element, not about failure. Failure-yielding iterators (e.g., `LineReader` that returns `String?~IoError` per line) are a **separate trait** to be designed in the v0.8 concurrency phase. No ADR-0003 update from v0.7.4.3-error.

### For std.result existing enum

No code change. Documentation updated in v0.7.4.3-error commit to mark `Result<T, E>` as "legacy convention" (still supported, no deprecation).

### For self-host compiler (v0.7.4.3+)

All error paths use `T~E` / `T?~E`. No `Result<T, E>` imports in `compiler/*.tri` source. Idiomatic patterns: `~->` propagate (most common), `~+>` post-success transform, pattern match (when all arms have logic), `~-> |_| default` for recovery (rare).

### For VM + interpreter

VM (`triet-ir::vm`): new opcodes 0xC1–0xC6 added to dispatch (§7.3). Outcome values stored as:

```rust
RuntimeValue::Outcome {
    discriminator: Trit,
    payload: Option<Box<RuntimeValue>>,
}
```

`payload` is `None` only when `discriminator == Trit::Zero` (null state for T?~E). Trit::Positive carries the success value as `Some(Box::new(...))`; Trit::Negative carries the failure value as `Some(Box::new(...))`.

**Memory deallocation rule (MANDATORY across all backends):**

When a `RuntimeValue::Outcome` is dropped or its register is reassigned, the implementation MUST recursively deallocate the inner `Box<RuntimeValue>` payload — no leaks, no double-free. The rule applies at every register-death point:

1. **Frame teardown** — when a VM frame is popped (function return), every Outcome-typed register's payload heap allocation must be released along with the register.
2. **Register reassignment** — when an IR register holding an Outcome is overwritten (phi node merge, loop iteration, sequential SSA versioning), the previous Outcome's payload must be deallocated before the new value occupies the slot.
3. **Pattern-match payload extraction** — when `OUTCOME_UNWRAP_VALUE` / `OUTCOME_UNWRAP_ERROR` extracts the payload, ownership transfers from the outcome to the destination register. The outcome's `payload` field becomes `None` and the outcome itself can be dropped without freeing the (now-moved) payload — no double-free.
4. **`OUTCOME_NEW_NULL` opcode** — emits an outcome with `payload: None`. Drop is a no-op for null-state outcomes.

For each backend tier (per [VISION §4.2](../../VISION.md)):

- **VM tier (Rust impl, v0.3)**: handled automatically by Rust's `Drop` trait on `Box<T>`. No manual code needed; correctness inherits from the borrow checker. Explicit unit tests will verify (a) frame teardown drops payloads, (b) phi-merge in loops doesn't accumulate, (c) `~+`/`~-` round-trip through pattern match leaves no leak (using Rust's `Box::leak` audit in test code).
- **JIT tier (v0.9 Cranelift)**: codegen MUST emit explicit deallocation calls at register-death points. Cranelift's memory management is manual — this rule pins the contract.
- **AOT tier (v2.0 LLVM)**: same rule via LLVM lifetime intrinsics (`@llvm.lifetime.end`). Compatible with planned ARC-style memory model per [ADR-0007 §"Memory model deferred to v0.3 implementation"](0007-ir-design.md).
- **Trytecode tier (v∞ ternary native)**: ternary CPU memory model is TBD at v∞, but this ADR commits the equivalent ownership semantics — the design requires safe payload deallocation regardless of underlying hardware.

Interpreter parity for outcome — **deferred** per same §A7 entry as v0.7.3 builtin parity. Self-host compiler runs via VM path only; interpreter catches up in v0.7.x.review or is dropped at v0.9 JIT (per prior §A7 plan).

### For capability resolver (ADR-0017)

`CapabilityResolver::resolve(req) -> CachedDecision { outcome: Trit, source: DecisionSource }` is **not** an Outcome type — it's an existing `Trit`-discriminator-plus-payload struct that predates this ADR. No migration. Future ADR may reframe as `Trit?~CapabilityError` if useful, but not in scope of v0.7.4.3-error.

### For wire format `.triv`

v4 → v5 patch bump. Type discriminant 10 added. Six new opcodes (0xC1–0xC6) added. Pre-v5 readers refuse with existing E2104/E2105 errors. No breaking change to v4-and-earlier content.

### For v0.9 JIT

Cranelift backend reading `.triv` v5 must lower the six new opcodes. Each maps to straightforward branch-and-extract code; no JIT-specific complications. Outcome value layout (trit + payload union) is JIT-friendly — trit fits in a register byte, payload union sized at max(sizeof T, sizeof E).

### For v2.0 AOT (LLVM)

Same as JIT. LLVM IR types for Outcome: `{ i8, [N x i8] }` where N = max payload size. Straightforward.

## Không làm

- **Force-unwrap operator (e.g. `~~`).** Author 2026-05-17 explicit rejection. Dangerous extraction is method-only.
- **`.value` / `.error` field access** without panic-message argument. Property access must be 100% safe contract per [`feedback_explicit_strictness.md`](../../README.md).
- **Implicit `error` binding in `~->` form.** Author 2026-05-17 rejection (carried into 2026-05-26 ternary family). Every variable in scope must trace to an explicit declaration site (`let` / function param / `|capture|`). No magic.
- **Whitespace-tolerant `?~` compound** (e.g. accepting `T ? ~ E` with internal space). Author 2026-05-17: compound tokens must be adjacent at the lexer level.
- **Preserving `null` keyword permanently** (Q2 rejection). `null` is deprecated v0.7.4.3-error onward and removed at v1.0 per §10.3 timeline. Migration tool `dao fmt --fix --migrate-null` automates the cleanup. Refuse-over-guess: one canonical Trit::Zero literal across the language.
- **Pattern-arm implicit widening** (Q3 lock). Pattern match arms for T? / T?~E must use explicit `~+ binding` constructor. No implicit `T → T?` widening inside patterns — patterns operate on discriminator state, not on widening rules. Implicit widening at expression position stays (allowed for terseness).
- **Unifying Trilean `unknown` with `~0`.** Trilean is a different domain (Ł3 logic truth values, not outcome discriminator). `true`/`false`/`unknown` literals stay per SPEC §1.5.2 — no unification across domains. See §10.6.
- **Implicit `From` conversion** (Rust's `?` operator semantics). Authors writing `~-> |err| return ~- E_outer.from(err)` is explicit. Refuse over guess — no silent type conversion.
- **`?~` reverse compound** (e.g. `Result<T, E>` → `T?~E` syntactic sugar). The two type families coexist (§8); no auto-conversion.
- **Multiple-error union types** (e.g. `T~(E1 | E2)`). Authors define a sum-type enum for the union and use `T~MyError`. Anonymous sum types in outcome position are out of scope.
- **`async T~E`** for futures. Trit::Zero is reserved for actor pending state (§6), but the v0.8 concurrency ADR will define how async values compose with outcome. Out of v0.7.4.3-error scope.
- **`try!` macro / built-in keyword** equivalent to `~->`. Operator family is sufficient.
- **`.unwrap()` (no-message) shorter alias** for `.unwrap_value(msg)`. Author rejection — message is mandatory contract.

## Prior art

- **Rust `Result<T, E>` + `?` operator** — direct inspiration for the propagate concept, but Triết rejects `?`-on-Result because Triết's `?` family already operates on `T?` nullable. Adopting a different operator family (`~+>` / `~0>` / `~->`) and a different type family avoids overload confusion.
- **Swift `throws` + `try` / `try?` / `try!`** — closest in spirit. Author rejected "đồ cổ" (try-catch) framing; Outcome is value-returning, not control-flow-jumping. Swift's `try!` (force-unwrap throw) is the exact anti-pattern §4 defends against.
- **Zig error union `!T`** — closest mechanically. Zig's `!T` is value-returning, no exceptions, error set tracked in type. Triết's `T~E` is more explicit (named error type) and exposes the trit discriminator (Triết-native). Zig has `try expr` for propagate, similar to `~-> |e| return ~- e` but more implicit. Author favored explicit form.
- **Kotlin sealed class + smart cast** — author considered (Option 2 in design discussion). Rejected because flow typing in typecheck is multi-week effort; the explicit `is_success` check + `unwrap_value(msg)` method is verbose-but-Java-friendly.
- **Go `(value, error)` tuple** — author considered (Option 3). Rejected: `if err != nil` boilerplate, zero-value pitfall, ambiguous (Some,Some)/(None,None) invariant, requires tuple opcodes (deferred post-v1.0).
- **Effect systems (Koka, Eff, Algebraic Effects)** — author considered. Rejected as research-grade and overkill for Triết v0.7. Compiler-level transparency missing for AI-first design.
- **`Outcome<T, E>` generic struct (Java/Kotlin idiom)** — author considered (Option 4). Rejected because it doesn't exploit balanced ternary identity (just a 2-state struct) and requires the smart-cast typecheck infrastructure that Kotlin's `is` checks provide.

## Tham chiếu

- [VISION §2 — Balanced ternary identity](../../VISION.md)
- [VISION §6 — Refuse over guess principle](../../VISION.md)
- [SPEC §2.5 — Nullable + (v0.7.4.3+) Outcome primary section](../../SPEC.md)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) — `T?` precedent that `T~E` parallels
- [ADR-0003 — Iterator protocol](0003-iterator-protocol.md) — unaffected; `next() -> T?` stays
- [ADR-0007 — IR design](0007-ir-design.md) — new Outcome opcodes 0xC1–0xC6 land in the existing IR contract
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) — v4 → v5 patch bump per §"Version compatibility"
- [ADR-0010 — Ternary-native IR](0010-ternary-native-ir.md) — BR_TRILEAN is the existing primitive that match-on-outcome lowers to
- [ADR-0019 + Addendum §A7](0019-self-hosting-compiler-bootstrap.md) — error handling primitive deferred item this ADR closes
- [`feedback_explicit_strictness.md`](https://github.com/gianghoang/triet) (author memory, 2026-05-17) — explicit-strictness-over-dangerous-ergonomics principle this ADR enacts
- Zig error union `!T` — primary technical precedent; Outcome is a Triết-native ternary refinement
- Rust `Result<T, E> + ?` — propagate concept; Triết's `~->` (with explicit closure) is the explicit form, plus `~+>` / `~0>` complete the ternary family

---

*Quyết định này lock outcome error handling cho Triết. Breaking change ở §1–§9 cần ADR mới supersede. Implementation lands ở sub-task v0.7.4.3-error (parser + AST + typecheck + lowerer + VM dispatch + tests + SPEC §2.5 rewrite + std.result documentation update + .triv v5 bump). Self-host compiler v0.7.4.3+ adopts `T~E` as primary; existing `Result<T, E>` is legacy-convention.*
