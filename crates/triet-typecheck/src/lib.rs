//! Triết type checker.
//!
//! Walks a `Program` produced by `triet-parser` and accumulates
//! `TypeError`s. The v0.1 checker is intentionally pragmatic:
//!
//! - All built-in scalar / Trilean / String / Unit types are known.
//! - User-defined generics, traits, and structs are not yet supported.
//! - The checker is recovery-friendly: on any error it substitutes
//!   `Type::Unknown` and continues, so a single run can surface every
//!   independent error.
//!
//! # Public API
//!
//! [`check`] takes a `&Program` and returns a `Vec<TypeError>` (empty
//! on success).

#![warn(missing_docs)]
// The checker has many small case-analysis branches that share short
// bodies, and several spots where moving from `if let` to `map_or`
// would obscure the no-op vs error paths. The clippy warnings flagged
// here are stylistic in this codebase, so we silence them at module
// level and review the affected code holistically instead.
#![allow(
    clippy::redundant_pub_crate,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::match_same_arms,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::missing_panics_doc
)]

mod capability_check;
mod check;
mod check_resolved;
mod env;
mod error;
mod types;

pub use capability_check::{CapabilityError, check_capabilities};
pub use check::check;
pub use check_resolved::check_resolved;
pub use env::TypeEnvironment;
pub use error::TypeError;
pub use types::Type;

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;
    use triet_parser::parse;

    fn check_source(source: &str) -> Vec<TypeError> {
        let (program, parse_errors) = parse(source);
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:#?}");
        check(&program)
    }

    fn assert_ok(source: &str) {
        let errors = check_source(source);
        assert!(errors.is_empty(), "type errors: {errors:#?}");
    }

    fn assert_has_error<F>(source: &str, predicate: F)
    where
        F: Fn(&TypeError) -> bool,
    {
        let errors = check_source(source);
        assert!(
            errors.iter().any(predicate),
            "expected matching error, got: {errors:#?}",
        );
    }

    // ===== Happy paths =====

    #[test]
    fn checks_identity_function() {
        assert_ok("function id(n: Integer) -> Integer = n");
    }

    /// v0.7.4.1: generic function with single type param.
    /// ADR-0019 Addendum §A7 — unblocks self-host stdlib stubs.
    /// `T` resolves to `TypeParam`, body uses x flowing as `T`, type
    /// inferred at call site by argument context.
    #[test]
    fn checks_generic_identity_function() {
        assert_ok(
            r#"
            function id<T>(x: T) -> T = x

            function main() {
                let a: Integer = id(42)
                let b: String = id("hello")
            }
            "#,
        );
    }

    /// v0.7.4.1: generic function with two type params.
    /// Targets the inference pattern for `function pair<K, V>(...) -> V`.
    #[test]
    fn checks_generic_function_with_two_params() {
        assert_ok(
            r#"
            function second<K, V>(k: K, v: V) -> V = v

            function main() {
                let result: String = second(42, "world")
            }
            "#,
        );
    }

    #[test]
    fn checks_simple_arithmetic() {
        assert_ok("function add(a: Integer, b: Integer) -> Integer = a + b");
    }

    #[test]
    fn checks_call_to_prelude_function() {
        assert_ok(r#"function greet() -> Unit = print("hello")"#);
    }

    #[test]
    fn checks_let_with_inferred_type() {
        assert_ok(
            r"
            function main() {
                let x = 5
                let y = x + 1
                println(to_string(y))
            }
        ",
        );
    }

    #[test]
    fn checks_let_with_matching_annotation() {
        assert_ok("function main() { let x: Integer = 5 }");
    }

    #[test]
    fn checks_if_question_with_trilean_condition() {
        // Updated v0.7.4.3-error.3d per ADR-0021: plain `if b` on a
        // bare Trilean parameter raises E1033; relaxed `if? b` accepts.
        assert_ok(
            r"
            function check(b: Trilean) -> Integer {
                if? b { 1 } else { 0 }
            }
        ",
        );
    }

    #[test]
    fn checks_match_with_consistent_arms() {
        assert_ok(
            r#"
            function classify(n: Integer) -> String =
                match n {
                    0 => "zero",
                    _ => "nonzero",
                }
        "#,
        );
    }

    #[test]
    fn checks_for_loop_over_range() {
        assert_ok(
            r"
            function count() {
                for i in 0..10 {
                    print(to_string(i))
                }
            }
        ",
        );
    }

    #[test]
    fn checks_method_call_on_integer() {
        assert_ok("function shrink(n: Integer) -> Tryte = n.to_tryte()");
    }

    #[test]
    fn checks_logic_expression_with_trileans() {
        assert_ok(
            r"
            function risk(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean =
                fever and rash and not vaccinated
        ",
        );
    }

    #[test]
    fn checks_implication_returns_trilean() {
        assert_ok(
            r"
            function entail(p: Trilean, q: Trilean) -> Trilean = p implies q
        ",
        );
    }

    #[test]
    fn checks_block_with_final_expression() {
        assert_ok(
            r"
            function compute() -> Integer {
                let a = 5
                let b = 7
                a + b
            }
        ",
        );
    }

    #[test]
    fn checks_force_unwrap_on_nullable() {
        assert_ok("function force(name: String?) -> String = name!!");
    }

    #[test]
    fn checks_tuple_index_lookup() {
        assert_ok(
            r"
            function first(pair: (Integer, Trilean)) -> Integer = pair.0
        ",
        );
    }

    // ===== Error cases =====

    #[test]
    fn flags_unknown_type_in_annotation() {
        assert_has_error(
            "function bad(x: Foobar) -> Integer = 0",
            |e| matches!(e, TypeError::UnknownType { name, .. } if name == "Foobar"),
        );
    }

    #[test]
    fn flags_undefined_name() {
        assert_has_error(
            "function bad() -> Integer = does_not_exist",
            |e| matches!(e, TypeError::UndefinedName { name, .. } if name == "does_not_exist"),
        );
    }

    #[test]
    fn flags_arithmetic_type_mismatch() {
        assert_has_error(
            r"function bad(a: Integer, b: Tryte) -> Integer = a + b",
            |e| matches!(e, TypeError::InvalidOperands { .. }),
        );
    }

    #[test]
    fn flags_arithmetic_on_non_numeric() {
        assert_has_error(
            r"function bad(a: String, b: String) -> String = a + b",
            |e| matches!(e, TypeError::InvalidOperands { .. }),
        );
    }

    #[test]
    fn flags_logic_op_on_non_trilean() {
        assert_has_error(
            "function bad(a: Integer, b: Integer) -> Trilean = a and b",
            |e| matches!(e, TypeError::InvalidOperands { .. }),
        );
    }

    #[test]
    fn flags_let_annotation_mismatch() {
        assert_has_error(r#"function bad() { let x: Integer = "hi" }"#, |e| {
            matches!(e, TypeError::Mismatch { .. })
        });
    }

    #[test]
    fn flags_function_return_mismatch() {
        assert_has_error(r#"function bad() -> Integer = "hi""#, |e| {
            matches!(e, TypeError::Mismatch { .. })
        });
    }

    #[test]
    fn flags_call_arity_mismatch() {
        assert_has_error(r"function main() { print() }", |e| {
            matches!(
                e,
                TypeError::WrongArity {
                    expected: 1,
                    found: 0,
                    ..
                }
            )
        });
    }

    #[test]
    fn flags_call_argument_type_mismatch() {
        assert_has_error(r"function main() { print(42) }", |e| {
            matches!(e, TypeError::Mismatch { .. })
        });
    }

    #[test]
    fn flags_if_with_non_trilean_condition() {
        assert_has_error(
            r"function bad(n: Integer) -> Integer { if n { 1 } else { 0 } }",
            |e| matches!(e, TypeError::NonTrileanCondition { .. }),
        );
    }

    #[test]
    fn flags_if_branches_with_different_types() {
        assert_has_error(
            r#"function bad(b: Trilean) -> Integer { if b { 1 } else { "two" } }"#,
            |e| matches!(e, TypeError::Mismatch { .. }),
        );
    }

    #[test]
    fn flags_force_unwrap_on_non_nullable() {
        assert_has_error(r"function bad(s: String) -> String = s!!", |e| {
            matches!(e, TypeError::NotNullable { .. })
        });
    }

    #[test]
    fn flags_tuple_index_out_of_range() {
        assert_has_error(
            r"function bad(pair: (Integer, Trilean)) -> Integer = pair.7",
            |e| matches!(e, TypeError::TupleIndexOutOfRange { .. }),
        );
    }

    #[test]
    fn flags_unknown_method() {
        assert_has_error(
            r"function bad(n: Integer) -> Integer = n.no_such_method()",
            |e| matches!(e, TypeError::UnknownMember { .. }),
        );
    }

    #[test]
    fn flags_duplicate_function_name() {
        assert_has_error(
            r"
                function dup() {}
                function dup() {}
            ",
            |e| matches!(e, TypeError::DuplicateName { name, .. } if name == "dup"),
        );
    }

    // ===== Realistic samples =====

    #[test]
    fn checks_nested_if_else_chain() {
        assert_ok(
            r#"
            function classify(score: Integer) -> String {
                if score >= 90 { "A" }
                else if score >= 80 { "B" }
                else if score >= 70 { "C" }
                else { "F" }
            }
        "#,
        );
    }

    #[test]
    fn checks_program_with_multiple_items() {
        assert_ok(
            r"
            constant MAX: Integer = 100

            function double(n: Integer) -> Integer = n * 2

            function main() {
                let x = double(MAX)
                println(to_string(x))
            }
        ",
        );
    }

    #[test]
    fn forward_reference_to_later_function_resolves() {
        assert_ok(
            r"
            function one() -> Integer = two()
            function two() -> Integer = 2
        ",
        );
    }

    // ===== Assignment (SPEC §5) =====

    #[test]
    fn checks_assignment_to_mut_binding() {
        assert_ok(
            r"
            function main() {
                let mutable count = 0
                count = count + 1
            }
        ",
        );
    }

    #[test]
    fn flags_assignment_to_immutable_binding() {
        assert_has_error(
            r"
                function main() {
                    let x = 0
                    x = 1
                }
            ",
            |e| matches!(e, TypeError::AssignToImmutable { name, .. } if name == "x"),
        );
    }

    #[test]
    fn flags_assignment_to_undefined_name() {
        assert_has_error(
            r"
                function main() {
                    nope = 1
                }
            ",
            |e| matches!(e, TypeError::UndefinedName { name, .. } if name == "nope"),
        );
    }

    #[test]
    fn flags_assignment_with_type_mismatch() {
        assert_has_error(
            r#"
                function main() {
                    let mutable x: Integer = 0
                    x = "hi"
                }
            "#,
            |e| matches!(e, TypeError::Mismatch { .. }),
        );
    }

    #[test]
    fn checks_assignment_in_inner_scope_to_outer_mut_binding() {
        assert_ok(
            r"
            function main() {
                let mutable count = 0
                if? true {
                    count = count + 1
                }
            }
        ",
        );
    }

    // ===== Iterator: enumerate (SPEC §7.2) =====

    #[test]
    fn checks_enumerate_on_range_with_tuple_destructuring() {
        assert_ok(
            r"
            function main() {
                for (i, v) in (0..5).enumerate() {
                    println(to_string(i + v))
                }
            }
        ",
        );
    }

    #[test]
    fn flags_enumerate_on_non_iterable_receiver() {
        assert_has_error(
            r"function bad(n: Integer) -> Integer = n.enumerate()",
            |e| matches!(e, TypeError::UnknownMember { .. }),
        );
    }

    // ===== v0.7.4.3-error.2: Outcome typecheck (ADR-0020) =====

    /// `T~E?` (nullable error type) parses as `T~(E?)` because the
    /// type-position `~` only consumes an atom, then trailing `?` is
    /// invalid syntax (current parser refuses it as malformed
    /// function-body). E1024 NullableErrorInOutcomeType will fire if
    /// future syntax sugar allows the form to parse — the typecheck
    /// arm exists to catch the semantic case.
    ///
    /// For now, asserting that the typecheck arm exists. Future test
    /// when parser sugar lands.
    #[test]
    fn typecheck_nullable_error_arm_exists() {
        // Confirm the error variant is reachable. Construction is
        // a quick smoke check — actual triggering requires parser
        // sugar that doesn't exist at v0.7.4.3-error.2.
        let _ = TypeError::NullableErrorInOutcomeType { span: 0..0 };
    }

    /// `~?` propagate operator requires the enclosing function to
    /// return an outcome type (E1028).
    #[test]
    fn flags_propagate_outside_fallible_context() {
        assert_has_error(
            r#"
            function dangerous() -> Integer {
                let result: String~IoError = ~- "err"
                let value = result ~? |err| ~- err
                42
            }
            "#,
            |e| matches!(e, TypeError::PropagateInNonFallibleContext { .. }),
        );
    }

    /// `~?` inside fallible function compiles cleanly when error
    /// types match.
    #[test]
    fn checks_propagate_in_fallible_function() {
        assert_ok(
            r"
            function safe() -> Integer~String {
                let result: Integer~String = ~+ 7
                let value = result ~? |err| ~- err
                ~+ value
            }
            ",
        );
    }

    /// `null` keyword emits W2001 warning (severity = Warning,
    /// does not block compile). We assert the warning IS produced
    /// (counts as a TypeError in the accumulator, with miette
    /// severity flag distinguishing it).
    #[test]
    fn null_keyword_emits_deprecation_warning() {
        use miette::Diagnostic;
        let errors = check_source(r"function main() -> Integer? = null");
        let warnings: Vec<_> = errors
            .iter()
            .filter(|e| e.severity() == Some(miette::Severity::Warning))
            .collect();
        assert!(
            warnings
                .iter()
                .any(|e| matches!(e, TypeError::NullDeprecated { .. })),
            "expected W2001 NullDeprecated warning, got: {errors:#?}"
        );
    }

    /// `~0` literal does NOT emit the deprecation warning — it's the
    /// canonical form.
    #[test]
    fn outcome_zero_literal_does_not_warn() {
        use miette::Diagnostic;
        let errors = check_source(r"function main() -> Integer? = ~0");
        let warnings: Vec<_> = errors
            .iter()
            .filter(|e| e.severity() == Some(miette::Severity::Warning))
            .collect();
        assert!(
            warnings.is_empty(),
            "~0 should not emit deprecation warning, got: {warnings:#?}"
        );
    }
}
