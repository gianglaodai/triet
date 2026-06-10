//! Type-checker error types.

use miette::Diagnostic;
use thiserror::Error;
use triet_syntax::Span;

use crate::types::Type;

/// An error raised while type-checking a `Program`.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum TypeError {
    /// A type expression names a type the checker doesn't recognize.
    #[error("unknown type `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1001),
        help("built-in types are: Trit, Tryte, Integer, Long, Trilean, String")
    )]
    UnknownType {
        /// The unrecognized name.
        name: String,
        /// Source location.
        #[label("unknown type")]
        span: Span,
    },

    /// An identifier is referenced but not bound in scope.
    #[error("undefined name `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1002),
        help(
            "did you forget to declare this variable with `let`, or define this function with `function`?"
        )
    )]
    UndefinedName {
        /// The unbound identifier.
        name: String,
        /// Source location.
        #[label("not found in scope")]
        span: Span,
    },

    /// A function name has overloaded signatures but none match the
    /// given argument types.
    #[error("no overload of `{name}` matches the given argument types")]
    #[diagnostic(
        code(triet::typecheck::E1041),
        help("available overloads: {candidates}")
    )]
    NoMatchingOverload {
        /// The function name.
        name: String,
        /// Human-readable list of candidate signatures.
        candidates: String,
        /// Source location.
        #[label("no matching overload")]
        span: Span,
    },

    /// A bare enum variant name (e.g. `None`) matches variants in
    /// multiple enum types. The user must fully qualify with `Enum.Variant`.
    #[error(
        "ambiguous enum variant `{variant}` — found in {enum_a} and {enum_b}. Use fully qualified syntax: `{enum_a}.{variant}` or `{enum_b}.{variant}`"
    )]
    #[diagnostic(
        code(triet::typecheck::E1018),
        help("prefix the variant with the enum type name to disambiguate")
    )]
    AmbiguousEnumVariant {
        /// The bare variant name.
        variant: String,
        /// First enum containing this variant.
        enum_a: String,
        /// Second enum containing this variant.
        enum_b: String,
        /// Source location.
        #[label("`{variant}` is ambiguous")]
        span: Span,
    },

    /// Two values were expected to share a type but didn't.
    #[error("E1003: type mismatch: expected {expected}, found {found}")]
    #[diagnostic(code(triet::typecheck::E1003))]
    Mismatch {
        /// Type the checker expected based on context.
        expected: Type,
        /// Type the checker actually saw.
        found: Type,
        /// Source location of the mismatched expression.
        #[label("expected `{expected}`, found `{found}`")]
        span: Span,
    },

    /// An operator was applied to operands whose types are not allowed.
    #[error(
        "invalid operands for `{operator}`: expected {expected_description}, found {left} and {right}"
    )]
    #[diagnostic(code(triet::typecheck::E1004))]
    InvalidOperands {
        /// Operator symbol or name.
        operator: String,
        /// Description of acceptable operand types.
        expected_description: String,
        /// Type of the left operand.
        left: Type,
        /// Type of the right operand.
        right: Type,
        /// Source location of the operator.
        #[label("`{operator}` applied to `{left}` and `{right}`")]
        span: Span,
    },

    /// A unary operator was applied to a type that doesn't support it.
    #[error("invalid operand for `{operator}`: found {operand}")]
    #[diagnostic(
        code(triet::typecheck::E1005),
        help("`-`/`!`/`not` work on numeric types (Trit, Tryte, Integer, Long) and Trilean")
    )]
    InvalidUnary {
        /// Operator symbol.
        operator: String,
        /// Operand type encountered.
        operand: Type,
        /// Source location.
        #[label("cannot apply `{operator}` to `{operand}`")]
        span: Span,
    },

    /// Function called with the wrong number of arguments.
    #[error("wrong number of arguments: expected {expected}, found {found}")]
    #[diagnostic(code(triet::typecheck::E1006))]
    WrongArity {
        /// Expected argument count.
        expected: usize,
        /// Actual argument count.
        found: usize,
        /// Source location of the call.
        #[label("expected {expected} argument(s), got {found}")]
        span: Span,
    },

    /// A non-callable expression appeared in call position.
    #[error("type {found} is not callable")]
    #[diagnostic(
        code(triet::typecheck::E1007),
        help("only functions and closures can be called with `(...)`")
    )]
    NotCallable {
        /// Type the checker found at the callee position.
        found: Type,
        /// Source location.
        #[label("`{found}` is not a function")]
        span: Span,
    },

    /// `if` (without `?`) used a possibly-unknown Trilean condition.
    #[error("condition may be `unknown`")]
    #[diagnostic(
        code(triet::typecheck::E1008),
        help(
            "replace `if` with `if?` to treat unknown as false, or call `.assume_known()` if you are certain the value is known"
        )
    )]
    AmbiguousCondition {
        /// Source location of the condition.
        #[label("this condition could be `unknown`")]
        span: Span,
    },

    /// `if` condition is not `Trilean`.
    #[error("condition must be `Trilean`, found {found}")]
    #[diagnostic(
        code(triet::typecheck::E1009),
        help("condition expressions must evaluate to a `Trilean` value (true, false, or unknown)")
    )]
    NonTrileanCondition {
        /// Type encountered.
        found: Type,
        /// Source location.
        #[label("this is `{found}`, not `Trilean`")]
        span: Span,
    },

    /// A duplicate name was declared in the same scope.
    #[error("duplicate declaration of `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1010),
        help("rename one of the declarations, or remove the duplicate")
    )]
    DuplicateName {
        /// The duplicated name.
        name: String,
        /// Source location of the second declaration.
        #[label("`{name}` already declared in this scope")]
        span: Span,
    },

    /// `null` literal used in a context that doesn't expect a nullable.
    #[error("`null` literal is only valid where a `T?` is expected")]
    #[diagnostic(
        code(triet::typecheck::E1011),
        help("wrap the expected type in `T?` (e.g. `Integer?`) to allow null")
    )]
    NullLiteralInNonNullableContext {
        /// Source location.
        #[label("`null` is not valid here")]
        span: Span,
    },

    /// `?.`, `?:`, or `!!` applied to a non-nullable receiver.
    #[error("`{operator}` requires a nullable receiver, found {found}")]
    #[diagnostic(
        code(triet::typecheck::E1012),
        help(
            "`{operator}` only works on nullable types (e.g. `Integer?`); the receiver `{found}` is not nullable"
        )
    )]
    NotNullable {
        /// Operator symbol.
        operator: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        #[label("`{found}` is not nullable")]
        span: Span,
    },

    /// Match arm body types disagree.
    #[error("match arm returns {found} but earlier arms return {expected}")]
    #[diagnostic(
        code(triet::typecheck::E1013),
        help("all arms of a `match` must have the same return type")
    )]
    MatchArmMismatch {
        /// Type of earlier arms.
        expected: Type,
        /// Type of this arm.
        found: Type,
        /// Source location of the offending arm.
        #[label("this arm returns `{found}`")]
        span: Span,
    },

    /// Tuple index out of range.
    #[error("tuple index {index} out of range (tuple has {length} element(s))")]
    #[diagnostic(code(triet::typecheck::E1014))]
    TupleIndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Tuple length.
        length: usize,
        /// Source location.
        #[label("index {index} exceeds tuple length {length}")]
        span: Span,
    },

    /// Field access on a type without that member.
    #[error("type {found} has no field or method named `{member}`")]
    #[diagnostic(code(triet::typecheck::E1015))]
    UnknownMember {
        /// Member name as written.
        member: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        #[label("`{found}` has no member `{member}`")]
        span: Span,
    },

    /// Assignment target is bound `let` (immutable).
    #[error("cannot assign to immutable binding `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1016),
        help("declare this binding with `let mut {name} = ...` to allow reassignment")
    )]
    AssignToImmutable {
        /// Target binding name.
        name: String,
        /// Source location of the assignment statement.
        #[label("`{name}` is immutable")]
        span: Span,
    },

    // === Outcome / `T~E` errors (v0.7.4.3-error.2, ADR-0020 §9) ===
    /// E1024: `T~E?` is not a valid type — nullable error not allowed.
    #[error("outcome error type cannot itself be nullable")]
    #[diagnostic(
        code(triet::typecheck::E1024),
        help(
            "if the operation may fail, the error must be present — use `T?~E` for null-able success path"
        )
    )]
    NullableErrorInOutcomeType {
        /// Source location of the outer outcome type expression.
        #[label("nullable error type is meaningless: if it failed, the error must exist")]
        span: Span,
    },

    /// E1025: `~0` constructor used in non-`T?~E` outcome (binary `T~E`).
    #[error("E1025: `~0` constructor requires outcome type with null state (`T?~E`)")]
    #[diagnostic(
        code(triet::typecheck::E1025),
        help(
            "declared return type is binary outcome `T~E`. Change to `T?~E` to allow null state, or replace with `~- DefaultError`."
        )
    )]
    NullStateInBinaryOutcome {
        /// Source location of the offending `~0` constructor.
        #[label("`~0` requires `T?~E` (ternary), got binary `T~E`")]
        span: Span,
    },

    /// E1026: non-exhaustive match on outcome type.
    #[error("E1026: non-exhaustive `match`: missing arm(s) {missing}")]
    #[diagnostic(
        code(triet::typecheck::E1026),
        help("add the missing arm(s) or use `_` wildcard to cover them")
    )]
    NonExhaustiveOutcomeMatch {
        /// Comma-separated list of missing arm tokens (e.g. "`~+`, `~-`").
        missing: String,
        /// Source location of the `match` expression.
        #[label("this match does not cover all arms")]
        span: Span,
    },

    /// E1026: non-exhaustive match on enum type.
    #[error("E1026: non-exhaustive `match`: missing enum variant(s) {missing}")]
    #[diagnostic(
        code(triet::typecheck::E1026),
        help("add the missing variant(s) or use `_` wildcard to cover them")
    )]
    NonExhaustiveEnumMatch {
        /// Comma-separated list of missing variant names.
        missing: String,
        /// Source location of the `match` expression.
        #[label("this match does not cover all enum variants")]
        span: Span,
    },

    /// E1027: mixing `Result<T, E>` and `T~E` without explicit conversion.
    #[error("cannot mix `Result<T, E>` and `T~E` without explicit conversion")]
    #[diagnostic(
        code(triet::typecheck::E1027),
        help(
            "Result and outcome are distinct types; use pattern match on one and reconstruct the other"
        )
    )]
    OutcomeTypeMismatch {
        /// Source location of the conversion site.
        #[label("Result/outcome boundary must be explicit")]
        span: Span,
    },

    /// E1028: propagate used outside fallible function.
    #[error(
        "E1028: `~->` propagate operator requires the enclosing function to return `T~E` or `T?~E`"
    )]
    #[diagnostic(
        code(triet::typecheck::E1028),
        help(
            "change the function's return type to outcome, or handle the error explicitly with `match`"
        )
    )]
    PropagateInNonFallibleContext {
        /// Source location of the propagate operator.
        #[label("`~->` requires fallible enclosing function")]
        span: Span,
    },

    /// E1029: outcome error type mismatch in propagate path.
    #[error(
        "E1029: outcome error type mismatch in `~->`: inner has {inner_error}, caller expects {outer_error}"
    )]
    #[diagnostic(
        code(triet::typecheck::E1029),
        help(
            "explicitly convert the error inside the `|capture|` body: `|err| ~- OuterError::Wrap(err)`"
        )
    )]
    ErrorTypeMismatch {
        /// Inner outcome's error type.
        inner_error: Type,
        /// Outer (caller's) error type.
        outer_error: Type,
        /// Source location of the propagate operator.
        #[label("error type does not match — explicit conversion required")]
        span: Span,
    },

    /// E1030: `~->` right-hand side missing closure capture form.
    #[error("E1030: `~->` operator requires explicit closure capture form")]
    #[diagnostic(
        code(triet::typecheck::E1030),
        help(
            "write `~-> |binding_name| return expression` or `~-> |_| return expression` to discard the error"
        )
    )]
    OutcomePropagateMissingCapture {
        /// Source location of the propagate operator.
        #[label("missing `|capture|` form")]
        span: Span,
    },

    /// E1031: `~->` early-return form must be return/panic/re-propagate.
    #[error("`~->` early-return form must be a `return` statement, panic, or another `~->`")]
    #[diagnostic(
        code(triet::typecheck::E1031),
        help(
            "falling through after `~->` would leave the binding unbound; emit a `return` or panic"
        )
    )]
    OutcomePropagateMalformedReturn {
        /// Source location of the malformed body.
        #[label("must terminate this branch with return/panic")]
        span: Span,
    },

    /// E1032: pattern binding does not implicitly widen `T → T?`.
    #[error("pattern arm for nullable / outcome type must use explicit `~+ binding` constructor")]
    #[diagnostic(
        code(triet::typecheck::E1032),
        help(
            "replace bare `binding` with `~+ binding` — patterns do not perform implicit T ⊂ T? widening"
        )
    )]
    PatternMissingExplicitConstructor {
        /// Source location of the pattern arm.
        #[label("explicit `~+ binding` required here")]
        span: Span,
    },

    /// E1033: condition might be `Trilean::Unknown` — plain `if`
    /// requires `Trilean!` per [ADR-0021] §3. Suggest the four
    /// canonical remediations (`if?`, `match`, `.assume_known`, comparison
    /// narrowing). Closes the runtime-panic-as-primary-safety gap
    /// originally left by ADR-0010 §1.
    ///
    /// [ADR-0021]: ../../../docs/decisions/0021-trilean-refinement.md
    #[error("condition might be Trilean::Unknown — plain `if` requires `Trilean!`")]
    #[diagnostic(
        code(triet::typecheck::E1033),
        help(
            "choose one:\n\
             1) Use `if?` to treat Unknown as false: `if? cond {{ ... }} else {{ ... }}`\n\
             2) Use `match cond {{ true => ..., false => ..., unknown => ... }}`\n\
             3) Narrow with `.assume_known(\"reason\")` (panics at runtime if Unknown)\n\
             4) Compare against `true` — works only if both sides already `Trilean!`\n\
             See SPEC §7.1.1 and ADR-0021 for the full design."
        )
    )]
    PossiblyUnknownCondition {
        /// Source location of the condition expression.
        #[label("this is `Trilean` (might be Unknown) — need `Trilean!`")]
        span: Span,
    },

    /// E1034: function declared `-> Trilean!` returns a value of type
    /// `Trilean` (un-refined) — implicit narrowing is rejected per
    /// [ADR-0021] §2.7. Author must either widen the return annotation
    /// to `Trilean`, or narrow the body via `.assume_known(msg)`,
    /// `match`, or refactor to produce a refined Trilean.
    ///
    /// [ADR-0021]: ../../../docs/decisions/0021-trilean-refinement.md
    #[error("function declared `-> Trilean!` but body returns `Trilean` (might be Unknown)")]
    #[diagnostic(
        code(triet::typecheck::E1034),
        help(
            "either change return type annotation to `Trilean`, or narrow the body \
             with `.assume_known(\"reason\")` / `match` to produce a refined Trilean!"
        )
    )]
    TrileanReturnNotRefined {
        /// Source location of the function return expression / body.
        #[label("body produces `Trilean`, declared returns `Trilean!`")]
        span: Span,
    },

    /// E1035: `~-` arm on nullable type (`T?` has no error state).
    /// Per [ADR-0020] §10.1: `T?` discriminator values are `+` (value)
    /// and `0` (null). `-` is reserved for outcome types `T~E` / `T?~E`.
    ///
    /// [ADR-0020]: ../../../docs/decisions/0020-outcome-error-handling.md
    #[error(
        "`~-` arm is not valid on nullable type `T?` — `~-` is reserved for outcome types `T~E`/`T?~E`"
    )]
    #[diagnostic(
        code(triet::typecheck::E1035),
        help(
            "nullable `T?` has only two states: present (`~+`) and null (`~0`). Remove the `~-` arm."
        )
    )]
    NegativeArmOnNullable {
        /// Source location of the offending `~-` arm.
        #[label("`~-` not valid on nullable type")]
        span: Span,
    },

    /// E1036: integer literal exceeds `Integer` range (±(3²⁷−1)/2 ≈ ±3.81×10¹²).
    /// Per [ADR-0044] Q2, literals are checked at compile time separately from
    /// runtime overflow traps.
    ///
    /// [ADR-0044]: ../../../docs/decisions/0044-arithmetic-range-enforcement.md
    #[error("integer literal `{value}` exceeds `Integer` range (±{max})")]
    #[diagnostic(
        code(triet::typecheck::E1036),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Use a smaller value within ±3_812_798_742_493 (27-trit Integer range)\n\n\
            [Fix 2] Use `{value}_long` for 81-trit Long precision"
        )
    )]
    IntegerLiteralOverflow {
        /// The literal value that exceeds the range.
        value: i64,
        /// The maximum absolute value allowed for `Integer`.
        max: i64,
        /// Source location of the literal.
        #[label("literal exceeds `Integer` range")]
        span: Span,
    },

    /// E1037: `~+>` / `~->` body type rejected.
    /// For both operators: body must be Bậc A scalar (heap/struct not allowed
    /// in the 8-byte Outcome payload slot).
    #[error(
        "E1037: `~+>` / `~->` body type must be a Bậc A scalar (Integer, Trit, Trilean, Tryte, Long) — heap/struct/enum types cannot fit in the Outcome payload slot"
    )]
    #[diagnostic(
        code(triet::typecheck::E1037),
        help(
            "change the body expression to return a scalar type, or use `match` for full control"
        )
    )]
    ArmHandlerMapModeRejected {
        /// Source location of the body expression.
        #[label("body type must be a Bậc A scalar")]
        span: Span,
    },

    /// E1039: Ambiguous `~->` map body when success type equals error type
    /// and the body does not use explicit rewrap (`~- expr`).  The auto-wrap
    /// is ambiguous because both outcome slots share the same type — the
    /// compiler cannot determine intent without an explicit rewrap.
    #[error(
        "E1039: ambiguous `~->` map body — success and error types are both \
         `{ty}`, so the auto-wrap is ambiguous; use explicit `~- <expr>` to \
         confirm error-map intent, or switch to `match` for full control"
    )]
    #[diagnostic(
        code(triet::typecheck::E1039),
        help("rewrap the body explicitly with `~- <expr>`, or use `match` for full control")
    )]
    AmbiguousAutoWrap {
        /// The type that is common to both success and error slots.
        ty: Type,
        /// Source location of the body expression.
        #[label("ambiguous body expression")]
        span: Span,
    },

    /// E1040: `Atomic<T>` payload `T` is not a member of `AtomicValue`
    /// per [ADR-0028] §2. Only ternary primitives với hardware atomic
    /// support qualify: Trit, Tryte, Integer, Trilean. Long excluded
    /// (81-trit exceeds hardware atomic width — split into 3× Atomic<Integer>
    /// or wait for `std.concurrency.Mutex` v0.10).
    ///
    /// [ADR-0028]: ../../../docs/decisions/0028-atomic-primitive.md
    #[error("type `{ty}` is not a valid `Atomic<T>` payload")]
    #[diagnostic(
        code(triet::typecheck::E1040),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Use a primitive AtomicValue type:\n\
            Change `Atomic<{ty}>` to `Atomic<Integer>` (or Tryte/Trit/Trilean)\n\n\
            [Fix 2] Wrap composite types in Mutex (ships v0.10 stdlib):\n\
            Use `Mutex<{ty}>` instead of `Atomic<{ty}>`\n\n\
            [Fix 3] Long (81-trit) exceeds hardware atomic width:\n\
            Split into 3× `Atomic<Integer>` slots với explicit synchronization"
        )
    )]
    NonAtomicValueType {
        /// The non-AtomicValue type used as Atomic payload.
        ty: String,
        /// Source location.
        #[label("`Atomic<{ty}>` not valid — `{ty}` is not a member of AtomicValue")]
        span: Span,
    },

    /// E1042: borrowed return type `-> &0 T` / `-> &+ T` / `-> &- T` is
    /// not yet supported (ADR-0045 §5). Returning a reference from a
    /// function requires PropagatedLoan wiring in the borrow checker
    /// (deferred to a future slice). This is a temporary refusal — it
    /// will be lifted when the return-borrow feature lands.
    #[error("E1042: returning a reference type `-> {return_ty}` is not yet supported")]
    #[diagnostic(
        code(triet::typecheck::E1042),
        help(
            "E1042: `-> {return_ty}` borrow-return is not yet implemented.\n\n\
            [Fix 1] Return a non-reference type instead:\n\
            Change `-> {return_ty}` to `-> Integer` (or the payload type)\n\n\
            [Fix 2] Clone/heap-copy (not available — defer to clone support):\n\
            When clone is implemented, return a cloned value instead of a reference\n\n\
            This refusal is temporary per ADR-0045 §5."
        )
    )]
    BorrowReturnNotYetSupported {
        /// The reference return type (e.g. `&0 String`).
        return_ty: String,
        /// Source location of the return type annotation.
        #[label("return-borrow `-> {return_ty}` not yet supported")]
        span: Span,
    },

    // === Warning-severity diagnostics (Q2-C: miette severity field) ===
    /// W2001: deprecated `null` keyword (use `~0` canonical literal).
    /// Severity: WARNING (does not block compile until v1.0 per
    /// ADR-0020 §10.3). At v1.0, W2001 promotes to E2002 (`NullRemoved`).
    #[error("`null` keyword is deprecated; use `~0` instead")]
    #[diagnostic(
        severity(Warning),
        code(triet::typecheck::W2001),
        help(
            "replace `null` with `~0` (canonical Trit::Zero literal per ADR-0020 §10). Auto-fix: `dao fmt --fix --migrate-null`"
        )
    )]
    NullDeprecated {
        /// Source location of the `null` token.
        #[label("deprecated keyword")]
        span: Span,
    },

    /// A concurrency-related error (e.g., crossing thread boundaries).
    #[error(transparent)]
    #[diagnostic(transparent)]
    Concurrency(#[from] ConcurrencyError),

    /// A borrow checker error (E24XX series).
    #[error(transparent)]
    #[diagnostic(transparent)]
    Borrow(#[from] BorrowError),
}

/// Errors related to concurrency primitives and thread boundaries.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum ConcurrencyError {
    /// E2500: A type was passed across a thread boundary but is not Send.
    #[error("type `{ty}` cannot be sent across thread boundaries")]
    #[diagnostic(
        code(triet::actor::E2500),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Ensure the type is Send (primitive or `&+` holding Send types):\n\
            Change `&0 T` or `&- T` to `&+ T` if applicable\n\n\
            [Fix 2] Encapsulate inside an Actor if shared state is needed:\n\
            Wrap type in `Actor<T>`"
        )
    )]
    NotSendCannotCrossBoundary {
        /// The type that failed the Send check.
        ty: String,
        /// Source location of the bound or argument.
        #[label("this type is not Send")]
        span: Span,
    },

    /// E2510: Scope-ref / weak-ref boundary violations.
    #[error("scope-ref leakage: reference escapes its permitted scope")]
    #[diagnostic(
        code(triet::actor::E2510),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Prevent the borrow from escaping:\n\
            Remove escaping assignment or return\n\n\
            [Fix 2] Upgrade to owned value for cross-boundary transport:\n\
            Change `&0 T` to `&+ T`"
        )
    )]
    ScopeRefLeakage {
        /// Source location of the leak.
        #[label("reference escapes here")]
        span: Span,
    },

    /// E2520: Mutable-share anti-pattern.
    #[error("mutable-share anti-pattern: attempting to share mutable state")]
    #[diagnostic(
        code(triet::actor::E2520),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Use message passing instead of shared state:\n\
            Change shared mutable state to Actor messaging\n\n\
            [Fix 2] Restrict to single-threaded logic if sharing is required:\n\
            Remove concurrency boundaries"
        )
    )]
    MutableShareAntiPattern {
        /// Source location of the anti-pattern.
        #[label("mutable state shared here")]
        span: Span,
    },

    /// E2530: `compare_exchange` success ordering weaker than failure
    /// ordering. Per [ADR-0028] §10 — semantically nonsensical: the
    /// failure path observing stronger synchronization than the success
    /// path is always a bug. Ordering strength is `Relaxed` (0) <
    /// `Synchronized` (1) < `Strict` (2).
    ///
    /// [ADR-0028]: ../../../../docs/decisions/0028-atomic-primitive.md
    #[error(
        "compare_exchange success ordering `{success}` weaker than failure ordering `{failure}`"
    )]
    #[diagnostic(
        code(triet::actor::E2530),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Raise the success ordering to match (or exceed) failure:\n\
            Change `success={success}` to `success={failure}`\n\n\
            [Fix 2] Lower the failure ordering to match success:\n\
            Change `failure={failure}` to `failure={success}`\n\n\
            [Fix 3] Use the same ordering on both paths (safe default):\n\
            Change both to `Synchronized`"
        )
    )]
    InvalidAtomicOrdering {
        /// Name of the success-ordering variant (e.g. `Relaxed`).
        success: String,
        /// Name of the failure-ordering variant (e.g. `Strict`).
        failure: String,
        /// Source span of the entire call.
        #[label("success={success}, failure={failure}")]
        span: Span,
    },
}

impl ConcurrencyError {
    /// Returns the source span associated with this concurrency error.
    pub fn span(&self) -> Span {
        match self {
            Self::NotSendCannotCrossBoundary { span, .. }
            | Self::ScopeRefLeakage { span }
            | Self::MutableShareAntiPattern { span }
            | Self::InvalidAtomicOrdering { span, .. } => span.clone(),
        }
    }
}

impl TypeError {
    /// Returns the byte span of the error for diagnostic anchoring.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Concurrency(err) => err.span(),
            Self::Borrow(err) => err.span(),
            Self::UnknownType { span, .. }
            | Self::UndefinedName { span, .. }
            | Self::NoMatchingOverload { span, .. }
            | Self::Mismatch { span, .. }
            | Self::InvalidOperands { span, .. }
            | Self::InvalidUnary { span, .. }
            | Self::WrongArity { span, .. }
            | Self::NotCallable { span, .. }
            | Self::AmbiguousCondition { span }
            | Self::AmbiguousEnumVariant { span, .. }
            | Self::NonTrileanCondition { span, .. }
            | Self::DuplicateName { span, .. }
            | Self::NullLiteralInNonNullableContext { span }
            | Self::NotNullable { span, .. }
            | Self::MatchArmMismatch { span, .. }
            | Self::TupleIndexOutOfRange { span, .. }
            | Self::UnknownMember { span, .. }
            | Self::AssignToImmutable { span, .. }
            | Self::NullableErrorInOutcomeType { span }
            | Self::NullStateInBinaryOutcome { span }
            | Self::NonExhaustiveOutcomeMatch { span, .. }
            | Self::NonExhaustiveEnumMatch { span, .. }
            | Self::OutcomeTypeMismatch { span }
            | Self::PropagateInNonFallibleContext { span }
            | Self::ErrorTypeMismatch { span, .. }
            | Self::OutcomePropagateMissingCapture { span }
            | Self::OutcomePropagateMalformedReturn { span }
            | Self::PatternMissingExplicitConstructor { span }
            | Self::PossiblyUnknownCondition { span }
            | Self::TrileanReturnNotRefined { span }
            | Self::NegativeArmOnNullable { span }
            | Self::ArmHandlerMapModeRejected { span, .. }
            | Self::AmbiguousAutoWrap { span, .. }
            | Self::IntegerLiteralOverflow { span, .. }
            | Self::NonAtomicValueType { span, .. }
            | Self::BorrowReturnNotYetSupported { span, .. }
            | Self::NullDeprecated { span } => span.clone(),
        }
    }
}

/// Errors emitted by the borrow checker (v0.9+ algorithm placeholder, ADR-0025).
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum BorrowError {
    /// E2400: `BorrowLifetimeInferenceFailed` (ADR-0025 §3.4)
    #[error("E2400: cannot infer which input the returned borrow ties to")]
    #[diagnostic(
        code(triet::borrow::E2400),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Return owned value instead (requires cloning inside body):\n\
            Change `-> &0 {ty}` to `-> &+ {ty}`\n\n\
            [Fix 2] Group inputs into a collection with a single borrow scope:\n\
            Refactor parameter list to a single struct or vector\n\n\
            [Fix 3] Encapsulate inside a struct method (ties return to `self`):\n\
            Wrap logic in an `impl` block"
        )
    )]
    BorrowLifetimeInferenceFailed {
        /// Type string of the returned borrow.
        ty: String,
        /// Source location of the return type expression.
        #[label("ambiguous return borrow")]
        span: Span,
    },

    /// E2402: `BorrowInStructField` (ADR-0025 §8.1)
    #[error("struct fields cannot hold non-owned borrows (`&0` or `&-`)")]
    #[diagnostic(
        code(triet::borrow::E2402),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Make the field own its data:\n\
            Change `&0 {ty}` or `&- {ty}` to `&+ {ty}`\n\n\
            [Fix 2] Pass the borrow as a function parameter instead of storing it:\n\
            Remove `{field_name}` field from struct"
        )
    )]
    BorrowInStructField {
        /// The name of the field.
        field_name: String,
        /// The type string.
        ty: String,
        /// Source location of the field.
        #[label("struct fields must be `&+` (owned)")]
        span: Span,
    },

    /// E2403: `EscapingBorrow` (ADR-0025 §8.2)
    #[error("borrow escapes its lexical scope")]
    #[diagnostic(
        code(triet::borrow::E2403),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Return owned value instead:\n\
            Change the return type to `&+ T`\n\n\
            [Fix 2] Keep the borrow strictly within its scope:\n\
            Remove the assignment to the outer scope"
        )
    )]
    EscapingBorrow {
        /// Source location where the borrow escapes.
        #[label("borrow escapes here")]
        span: Span,
    },

    // CannotMutateFrozenOwner: deleted (ADR-0051 B2 cleanup — dead variant).
    /// E2411: `CannotPromoteFrozenToMutable` (ADR-0025 §7.2)
    ///
    /// Frozen ownership (`&+ T`) is permanent — cannot be promoted to
    /// mutable ownership (`&+ mutable T`). Per ADR-0022 §3.4 +
    /// ADR-0026 §3 ("safe to share across actor boundary" invariant).
    #[error("cannot promote `&+ {ty}` (frozen owner) to `&+ mutable {ty}`")]
    #[diagnostic(
        code(triet::borrow::E2411),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Declare as mutable at construction, derive frozen view only when sharing:\n\
            Change the source binding's type from `&+ {ty}` to `&+ mutable {ty}`\n\n\
            [Fix 2] Keep frozen ownership and construct a fresh mutable owner with fields \
            copied explicitly:\n\
            Replace `let mutable_handle: &+ mutable {ty} = frozen` with a fresh \
            `&+ mutable {ty}` constructor that reads each field from `frozen`"
        )
    )]
    CannotPromoteFrozenToMutable {
        /// The inner type string (without the `&+` prefix).
        ty: String,
        /// Source location of the promotion expression (the RHS of the
        /// let binding or the assignment target).
        #[label("frozen-to-mutable promotion")]
        span: Span,
    },

    /// E2420: `UseAfterMove` (ADR-0025 §5.1)
    #[error("use of moved value `{name}`")]
    #[diagnostic(
        code(triet::borrow::E2420),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Borrow instead of moving if you still need it:\n\
            Change argument to `&0 {name}` or `&- {name}`\n\n\
            [Fix 2] Clone the value before moving:\n\
            Use `{name}.clone()`"
        )
    )]
    UseAfterMove {
        /// The name of the variable.
        name: String,
        /// Source location.
        #[label("value used here after move")]
        span: Span,
    },

    /// E2421: `SelfOwnershipParadox` (ADR-0025 §5.2)
    #[error("self-ownership paradox: struct cannot own itself")]
    #[diagnostic(
        code(triet::borrow::E2421),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Store an ID or index instead of the struct itself:\n\
            Replace the self-reference with an identifier"
        )
    )]
    SelfOwnershipParadox {
        /// Source location.
        #[label("self-ownership created here")]
        span: Span,
    },

    /// E2422: `NonTerminatingConstruction` (ADR-0025 §6.2)
    #[error(
        "non-terminating construction: struct requires an owned instance of itself to be constructed"
    )]
    #[diagnostic(
        code(triet::borrow::E2422),
        help(
            "Suggested fixes:\n\n\
            [Fix 1] Use an Outcome type to break the cycle (allows null state):\n\
            Change `&+ {ty}` to `(&+ {ty})?~E` or similar null-able type\n\n\
            [Fix 2] Use an array/vector for recursive ownership:\n\
            Change `&+ {ty}` to `Vector<&+ {ty}>`"
        )
    )]
    NonTerminatingConstruction {
        /// The type string.
        ty: String,
        /// Source location.
        #[label("field makes construction impossible")]
        span: Span,
    },
    // NamespaceInferenceFailed: deleted (ADR-0051 B2 cleanup — dead variant).
    // BorrowExclusivityViolation: deleted (ADR-0051 B2.1b — E2440 moved to MIR).
}

impl BorrowError {
    /// Returns the source span associated with this borrow error.
    pub fn span(&self) -> Span {
        match self {
            Self::BorrowLifetimeInferenceFailed { span, .. }
            | Self::BorrowInStructField { span, .. }
            | Self::EscapingBorrow { span }
            | Self::CannotPromoteFrozenToMutable { span, .. }
            | Self::UseAfterMove { span, .. }
            | Self::SelfOwnershipParadox { span }
            | Self::NonTerminatingConstruction { span, .. } => span.clone(),
        }
    }
}
