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

    /// Two values were expected to share a type but didn't.
    #[error("type mismatch: expected {expected}, found {found}")]
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
    #[error("`~0` constructor requires outcome type with null state (`T?~E`)")]
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
    #[error("non-exhaustive `match` on outcome type: missing arm(s) {missing}")]
    #[diagnostic(
        code(triet::typecheck::E1026),
        help("add the missing arm(s) or use `_` wildcard to cover them")
    )]
    NonExhaustiveOutcomeMatch {
        /// Comma-separated list of missing arm tokens (e.g. "`~+`, `~-`").
        missing: String,
        /// Source location of the `match` expression.
        #[label("this match does not cover all outcome arms")]
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

    /// E1028: `~?` propagate used outside fallible function.
    #[error("`~?` propagate operator requires the enclosing function to return `T~E` or `T?~E`")]
    #[diagnostic(
        code(triet::typecheck::E1028),
        help(
            "change the function's return type to outcome, or handle the error explicitly with `match`"
        )
    )]
    PropagateInNonFallibleContext {
        /// Source location of the propagate operator.
        #[label("`~?` requires fallible enclosing function")]
        span: Span,
    },

    /// E1029: outcome error type mismatch in propagate path.
    #[error(
        "outcome error type mismatch in `~?`: inner has {inner_error}, caller expects {outer_error}"
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

    /// E1030: `~?` right-hand side missing closure capture form.
    #[error("`~?` operator requires explicit closure capture form")]
    #[diagnostic(
        code(triet::typecheck::E1030),
        help(
            "write `~? |binding_name| early_return_form` or `~? |_| early_return_form` to discard the error"
        )
    )]
    OutcomePropagateMissingCapture {
        /// Source location of the propagate operator.
        #[label("missing `|capture|` form")]
        span: Span,
    },

    /// E1031: `~?` early-return form must be return/panic/re-propagate.
    #[error("`~?` early-return form must be a `return` statement, panic, or another `~?`")]
    #[diagnostic(
        code(triet::typecheck::E1031),
        help(
            "falling through after `~?` would leave the binding unbound; emit a `return` or panic"
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

    // === Warning-severity diagnostics (Q2-C: miette severity field) ===
    /// W2001: deprecated `null` keyword (use `~0` canonical literal).
    /// Severity: WARNING (does not block compile until v1.0 per
    /// ADR-0020 §10.3). At v1.0, W2001 promotes to E2002 (`NullRemoved`).
    #[error("`null` keyword is deprecated; use `~0` instead")]
    #[diagnostic(
        severity(Warning),
        code(triet::typecheck::W2001),
        help(
            "replace `null` with `~0` (canonical Trit::Zero literal per ADR-0020 §10). Auto-fix: `triet fmt --fix --migrate-null`"
        )
    )]
    NullDeprecated {
        /// Source location of the `null` token.
        #[label("deprecated keyword")]
        span: Span,
    },
}

impl TypeError {
    /// Returns the byte span of the error for diagnostic anchoring.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::UnknownType { span, .. }
            | Self::UndefinedName { span, .. }
            | Self::Mismatch { span, .. }
            | Self::InvalidOperands { span, .. }
            | Self::InvalidUnary { span, .. }
            | Self::WrongArity { span, .. }
            | Self::NotCallable { span, .. }
            | Self::AmbiguousCondition { span }
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
            | Self::OutcomeTypeMismatch { span }
            | Self::PropagateInNonFallibleContext { span }
            | Self::ErrorTypeMismatch { span, .. }
            | Self::OutcomePropagateMissingCapture { span }
            | Self::OutcomePropagateMalformedReturn { span }
            | Self::PatternMissingExplicitConstructor { span }
            | Self::PossiblyUnknownCondition { span }
            | Self::TrileanReturnNotRefined { span }
            | Self::NullDeprecated { span } => span.clone(),
        }
    }
}
