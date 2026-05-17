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

The marker for ternary outcome is a **lexer-level compound token** `?~` — emitted as a single token when `?` and `~` appear adjacent without whitespace. This is the same compound-token discipline as `~+`, `~-`, `~0` (§2): no whitespace-within is allowed.

```text
LexerToken    ::=  ...
                |  '?~'                   # ternary-outcome marker (compound)
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
    Outcome error type cannot itself be nullable. Did you mean `T?~E`?
```

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
return ~+value           // PARSER OK, STYLE-GUIDE VIOLATION (rejected by `triet fmt`)

return ~- IoError::Invalid(path)  // GOOD

return ~+ -1             // GOOD — `~+` constructs outcome, `-1` is the negative integer payload
return ~+-1              // CONFUSING (parser accepts but rejected by `triet fmt`)
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
    let cache = std.io.fs.read("symbols.cache") ~? |io_err| return ~- io_err
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
    Declared return type is `T~E` (binary). Did you mean `~- DefaultError`
    or change return type to `T?~E`?
```

## §3 — Operators: `~?` propagate, `~:` default

**Lock:** Two binary operators on outcome values. **No operator for force-unwrap** — dangerous extraction is method-only (§4). Per author 2026-05-17 directive *"hành vi nguy hiểm bắt buộc phải dùng verbose methods"*.

### 3.1 — `~?` propagate (explicit closure capture)

```triet
let value = expression ~? |binding_name| early_return_form
```

Semantics:
- Evaluate `expression` (expected to be `T~E` or `T?~E`).
- If success arm (Trit::Positive): bind `value` to the success payload, continue.
- If null arm (Trit::Zero, T?~E only): bind `value` to `null` (success-with-null is still success), continue.
- If failure arm (Trit::Negative): bind the failure payload (type `E`) to the user-named `binding_name`, then evaluate `early_return_form`, which **must** be a return statement, a re-construction expression, or a panic. The binding is scoped only to the `early_return_form` expression.

**No implicit magic binding.** The developer explicitly names the captured error via the `|binding_name|` form. This is a clean-code requirement per author 2026-05-17: implicit variables risk shadowing and obscure data flow.

Pattern variations:

```triet
function process(path: String) -> Integer~ProcessError {
    // Convert + return: name the captured error, build new outcome.
    let contents = std.io.fs.read(path) ~? |io_err| return ~- ProcessError::Io(io_err)

    // Ignore the error value, return a default-error variant.
    let parsed = std.text.parse_integer(contents) ~? |_| return ~- ProcessError::Parse

    return ~+ parsed * 2
}
```

The `|_|` form discards the error payload (useful when the caller's failure variant carries no inner payload). The underscore convention matches existing wildcard pattern usage in Triết's `match` arms.

The capture syntax `|name|` is reused from closure parameter syntax (forthcoming in a future ADR — currently no first-class closures expose this form, but `~?` claims the precedent). When closures land formally, the same `|param|` form will be used — no syntactic surprise.

**Explicit type conversion required.** When the inner outcome's error type `E_inner` differs from the caller's `E_outer`, the developer must construct the outer outcome explicitly inside the closure body — no implicit `From` trait magic (Rust's `?` does that; Triết refuses-over-guess per VISION §6).

```text
// VALID — explicit conversion:
expression ~? |inner_err| return ~- OuterError::Wrap(inner_err)

// INVALID — no implicit From:
expression ~?   // E1030: closure capture form required
```

### 3.2 — `~:` default value (substitute on failure)

```triet
let value = expression ~: default_expr
```

Semantics:
- Evaluate `expression` (expected to be `T~E`).
- If success arm: bind `value` to the success payload.
- If failure arm: evaluate `default_expr` (must have type `T`) and bind `value` to that.

For `T?~E`, the null state passes through as `null` (success-side); `~:` only fires on failure arm.

```triet
let port: Integer = std.text.parse_integer(env_port) ~: 8080
//                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//                 returns Integer~ParseError; on parse failure use 8080
```

### 3.3 — Operator precedence and chaining

Both `~?` and `~:` bind **lower** than method calls and field access, **higher** than assignment:

```triet
let value = outcome.try_value() ~: default()  // method call first, then ~:
let value = (outcome ~: default).field         // ~: first, then field access — REQUIRES PARENS
```

Style guide: parenthesize when chaining `~?` / `~:` with field access or method calls for readability.

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
    Add an explicit arm or use `_` wildcard.
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
    Outcome value of type `T~E` has discriminator Trit::Zero, which is reserved.
    This indicates corrupt wire data or a future-version pending state encountered
    by a pre-v0.8 reader.
```

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

`OUTCOME_DISCRIMINANT` is the lowering target for the safe properties (`.is_success`, `.is_null`, `.is_error`) and for pattern match dispatch (which becomes a `BR_TRILEAN` on the discriminator). It is also the lowering target for the `~?` propagate operator's discriminator check.

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

- Caller using `~?`: typecheck checks that the caller's own return type is fallible (else E1028 PropagateInNonFallibleContext), and that the inner E is compatible with the caller's error type (else E1029 ErrorTypeMismatch).
- Caller using `~:`: typecheck checks default expression type matches T.
- Caller using `match`: typecheck enforces exhaustiveness (§5.1).
- Caller using `.unwrap_value(msg)`: returns T, no type-system fence.
- Caller using `.try_value()`: returns `T?`.
- Caller using `.is_success`/`.is_null`/`.is_error`: returns `Trilean`.

```text
E1027 OutcomeTypeMismatch
    Cannot mix `Result<T, E>` and `T~E` without explicit conversion.

E1028 PropagateInNonFallibleContext
    Operator `~?` requires the enclosing function to have a fallible return type
    (`T~E` or `T?~E`). Function `foo` declared return type `Integer`.

E1029 ErrorTypeMismatch
    Outcome error type mismatch in propagate: inner outcome has `E_inner`, caller
    expects `E_outer`. Add explicit conversion `~- E_outer::from(error)`.
```

### 9.3 — Explicit closure capture in `~?` right-hand side

Section 3.1 locks `|binding_name|` capture form on the `~?` right-hand side. Typecheck rules:

1. Lexer/parser produces an `OutcomePropagate { inner_expr, capture_name, early_return_form }` AST node from the source `inner_expr ~? |capture_name| early_return_form`.
2. Inside `early_return_form` typecheck scope, the parser pushes a frame and declares the captured binding (name = `capture_name`) with type `E_inner` (the inner outcome's failure payload type).
3. `capture_name` may be `_` to discard the payload — typecheck does not declare a binding in that case; references to `_` inside the form are a separate error per existing wildcard rules.
4. If `capture_name` shadows an outer variable, this is treated identically to a regular `let capture_name = ...` shadow — no special-case shadowing rule; the developer is responsible for picking a non-conflicting name.
5. The binding is read-only (cannot be reassigned within the form) and goes out of scope when the form ends.

**No implicit magic.** Triết has zero implicit bindings — the developer always sees the name they're using. This matches author's clean-code principle: every variable in scope is traceable to a `let`/`function param`/`|capture|` site. Connects to [`feedback_explicit_strictness.md`](../../README.md) — explicit > convenient.

```text
E1030 OutcomePropagateMissingCapture
    The `~?` operator requires explicit closure capture form on the
    right-hand side. Expected `|binding_name| early_return_form` or
    `|_| early_return_form` (to discard the error payload).
    Found bare statement — implicit error bindings are not supported.

E1031 OutcomePropagateMalformedReturn
    The `~?` operator's `early_return_form` must be a `return`
    statement, a panic, or another `~?` propagate. Falling through
    after a `~?` capture is not allowed (would leave `value` unbound).
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
- `triet fmt --fix --migrate-null` flag introduced — auto-rewrites every `null` → `~0` across a project tree.

**v1.0 (production stability cutoff):**

- `null` keyword **removed** from grammar.
- Warning W2001 promoted to error **E2002 NullRemoved** with same fix-hint.
- Migration tool (`triet fmt --fix --migrate-null`) ships with v1.0 release for one-shot cleanup of legacy codebases.

```text
W2001 NullDeprecated
    `null` keyword is deprecated. Replace with `~0` (canonical Trit::Zero
    literal per ADR-0020 §10). This warning becomes error E2002 at v1.0.
    Auto-fix: run `triet fmt --fix --migrate-null`.

E2002 NullRemoved    (active at v1.0+)
    `null` keyword is no longer valid. Use `~0` (canonical Trit::Zero
    literal per ADR-0020 §10). Auto-fix: run `triet fmt --fix --migrate-null`.
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
    Pattern arm for T? or T?~E type must use explicit `~+ binding`
    constructor — pattern matching does not perform T ⊂ T? widening.
    Replace `<binding>` with `~+ <binding>`.
```

### 10.5 — `triet fmt --fix --migrate-null` specification

The migration tool is a **non-trivial requirement** of this ADR — it carries the cost of unification across all user codebases. Implementation locked here:

1. **Token-level rewrite:** `null` → `~0` everywhere. No semantic analysis required (the unification is exact).
2. **Preserve formatting:** spaces, comments, line breaks adjacent to `null` token are preserved verbatim. Only the 4 characters `null` change to the 2 characters `~0` (with surrounding spaces handled per the existing `triet fmt` rules).
3. **In-place by default**, with `--dry-run` option for preview.
4. **Recursive directory traversal** when given a directory argument; respects `.gitignore` (mirror existing `triet fmt` behavior).
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

All error paths use `T~E` / `T?~E`. No `Result<T, E>` imports in `compiler/*.tri` source. Idiomatic patterns: `~?` propagate (most common), pattern match (when both arms have logic), `~:` default (rare).

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
- **Implicit `error` binding in `~?` form.** Author 2026-05-17 rejection. Every variable in scope must trace to an explicit declaration site (`let` / function param / `|capture|`). No magic.
- **Whitespace-tolerant `?~` compound** (e.g. accepting `T ? ~ E` with internal space). Author 2026-05-17: compound tokens must be adjacent at the lexer level.
- **Preserving `null` keyword permanently** (Q2 rejection). `null` is deprecated v0.7.4.3-error onward and removed at v1.0 per §10.3 timeline. Migration tool `triet fmt --fix --migrate-null` automates the cleanup. Refuse-over-guess: one canonical Trit::Zero literal across the language.
- **Pattern-arm implicit widening** (Q3 lock). Pattern match arms for T? / T?~E must use explicit `~+ binding` constructor. No implicit `T → T?` widening inside patterns — patterns operate on discriminator state, not on widening rules. Implicit widening at expression position stays (allowed for terseness).
- **Unifying Trilean `unknown` with `~0`.** Trilean is a different domain (Ł3 logic truth values, not outcome discriminator). `true`/`false`/`unknown` literals stay per SPEC §1.5.2 — no unification across domains. See §10.6.
- **Implicit `From` conversion** (Rust's `?` operator semantics). Authors writing `~? return ~- E_outer::from(error)` is explicit. Refuse over guess — no silent type conversion.
- **`?~` reverse compound** (e.g. `Result<T, E>` → `T?~E` syntactic sugar). The two type families coexist (§8); no auto-conversion.
- **Multiple-error union types** (e.g. `T~(E1 | E2)`). Authors define a sum-type enum for the union and use `T~MyError`. Anonymous sum types in outcome position are out of scope.
- **`async T~E`** for futures. Trit::Zero is reserved for actor pending state (§6), but the v0.8 concurrency ADR will define how async values compose with outcome. Out of v0.7.4.3-error scope.
- **`try!` macro / built-in keyword** equivalent to `~?`. Operator is sufficient.
- **`.unwrap()` (no-message) shorter alias** for `.unwrap_value(msg)`. Author rejection — message is mandatory contract.

## Prior art

- **Rust `Result<T, E>` + `?` operator** — direct inspiration for the propagate concept, but Triết rejects `?`-on-Result because Triết's `?` family already operates on `T?` nullable. Adopting a different operator (`~?`) and a different type family avoids overload confusion.
- **Swift `throws` + `try` / `try?` / `try!`** — closest in spirit. Author rejected "đồ cổ" (try-catch) framing; Outcome is value-returning, not control-flow-jumping. Swift's `try!` (force-unwrap throw) is the exact anti-pattern §4 defends against.
- **Zig error union `!T`** — closest mechanically. Zig's `!T` is value-returning, no exceptions, error set tracked in type. Triết's `T~E` is more explicit (named error type) and exposes the trit discriminator (Triết-native). Zig has `try expr` for propagate, similar to `~? return ~- error` but more implicit. Author favored explicit form.
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
- Rust `Result<T, E> + ?` — propagate concept; Triết's `~?` is the explicit form

---

*Quyết định này lock outcome error handling cho Triết. Breaking change ở §1–§9 cần ADR mới supersede. Implementation lands ở sub-task v0.7.4.3-error (parser + AST + typecheck + lowerer + VM dispatch + tests + SPEC §2.5 rewrite + std.result documentation update + .triv v5 bump). Self-host compiler v0.7.4.3+ adopts `T~E` as primary; existing `Result<T, E>` is legacy-convention.*
