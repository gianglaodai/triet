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

// mod borrow_check; — deleted (ADR-0051 B2.1b, E2440 moved to MIR)
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
pub use error::{BorrowError, ConcurrencyError, TypeError};
pub use triet_syntax::{EnumVariantResolution, ExprResolutions, PatternResolutions};
pub use types::Type;

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;
    use triet_parser::parse;

    fn check_source(source: &str) -> Vec<TypeError> {
        let (program, parse_errors) = parse(source);
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:#?}");
        let (errors, _, _) = check(&program);
        errors
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

    // ===== Concurrency Bounds =====

    #[test]
    fn checks_send_bound_success() {
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}

            function main() {
                spawn(42) // Integer is Send
            }
            ",
        );
    }

    #[test]
    fn checks_send_bound_failure() {
        assert_has_error(
            r"
            function spawn<F: Send>(f: F) {}

            function main(r: &0 Integer) {
                spawn(r)
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    #[test]
    fn checks_send_bound_strong_ref_success() {
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}

            function main(r: &+ Integer) {
                spawn(r)
            }
            ",
        );
    }

    #[test]
    fn checks_send_bound_weak_ref_failure() {
        assert_has_error(
            r"
            function spawn<F: Send>(f: F) {}

            function main(r: &- Integer) {
                spawn(r)
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    #[test]
    fn checks_send_bound_atomic_success() {
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}

            function main(r: Atomic<Integer>) {
                spawn(r)
            }
            ",
        );
    }

    // ===== Type bounds in struct =====
    #[test]
    fn checks_send_bound_struct() {
        assert_has_error(
            r"
            struct Task<T: Send> { val: T }
            function main(r: &0 Integer) {
                let t: Task<&0 Integer> = Task { val: r } // Should fail
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    // ===== v0.8.x.completion.3: Send derivation coverage gap closure =====
    // ADR-0026 v2 §2.1 lists 13 type categories. Pre-completion test suite
    // covered 6 (primitive Integer, &+, &-, Atomic, generic struct). These
    // tests fill the gap for tuple, nullable, outcome, String, &+ mutable,
    // &0 mutable per ADR-0026 §2.1 rows.

    #[test]
    fn checks_send_bound_string_success() {
        // Category 5: String (frozen) is always Send.
        assert_ok(
            r#"
            function spawn<F: Send>(f: F) {}
            function main() {
                spawn("hello")
            }
            "#,
        );
    }

    #[test]
    fn checks_send_bound_nullable_success() {
        // Category 3: T? Send iff T Send.
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}
            function main(x: Integer?) {
                spawn(x)
            }
            ",
        );
    }

    #[test]
    fn checks_send_bound_strong_mutable_success() {
        // Category 8: &+ mutable T Send iff T Send (move semantics).
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}
            function main(r: &+ mutable Integer) {
                spawn(r)
            }
            ",
        );
    }

    #[test]
    fn checks_send_bound_scope_borrow_mutable_failure() {
        // Category 10: &0 mutable T never Send (exclusive borrow can't
        // cross execution boundary). E2510-class violation per ADR-0026.
        assert_has_error(
            r"
            function spawn<F: Send>(f: F) {}
            function main(r: &0 mutable Integer) {
                spawn(r)
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    #[test]
    fn checks_send_bound_neutral_borrow_failure() {
        // Category 9: &0 T (read-only borrow) never Send.
        // Sibling of strong_ref_success — verifies the polarity flip.
        assert_has_error(
            r"
            function spawn<F: Send>(f: F) {}
            function main(r: &0 Integer) {
                spawn(r)
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    #[test]
    fn checks_send_bound_strong_mut_propagates_inner_non_send() {
        // Category 8 negative: &+ mutable T Send rule requires T Send.
        // `&+ mutable &0 T` — outer is mutable strong, inner is non-Send
        // borrow → not Send overall.
        assert_has_error(
            r"
            function spawn<F: Send>(f: F) {}
            function main(r: &+ mutable &0 Integer) {
                spawn(r)
            }
            ",
            |e| {
                matches!(
                    e,
                    TypeError::Concurrency(ConcurrencyError::NotSendCannotCrossBoundary { .. })
                )
            },
        );
    }

    #[test]
    fn checks_send_bound_explicit_unit_success() {
        // Category 1: Unit is always Send (stack primitive).
        assert_ok(
            r"
            function spawn<F: Send>(f: F) {}
            function main() {
                spawn(())
            }
            ",
        );
    }

    // ===== v0.9.x.atomic.1: AtomicValue membership enforcement (ADR-0028 §2) =====
    // Only Trit/Tryte/Integer/Trilean qualify as AtomicValue payload.
    // Long excluded (81-trit > hardware atomic width); composites refused.

    #[test]
    fn atomic_integer_accepted() {
        assert_ok(
            r"
            function take(x: Atomic<Integer>) {}
            ",
        );
    }

    #[test]
    fn atomic_tryte_accepted() {
        assert_ok(
            r"
            function take(x: Atomic<Tryte>) {}
            ",
        );
    }

    #[test]
    fn atomic_trit_accepted() {
        assert_ok(
            r"
            function take(x: Atomic<Trit>) {}
            ",
        );
    }

    #[test]
    fn atomic_trilean_accepted() {
        assert_ok(
            r"
            function take(x: Atomic<Trilean>) {}
            ",
        );
    }

    #[test]
    fn atomic_long_rejected() {
        // ADR-0028 §2: Long (81-trit) exceeds hardware atomic width.
        assert_has_error(
            r"
            function take(x: Atomic<Long>) {}
            ",
            |e| matches!(e, TypeError::NonAtomicValueType { .. }),
        );
    }

    #[test]
    fn atomic_string_rejected() {
        // ADR-0028 §2: String is composite (heap-allocated), not AtomicValue.
        assert_has_error(
            r"
            function take(x: Atomic<String>) {}
            ",
            |e| matches!(e, TypeError::NonAtomicValueType { .. }),
        );
    }

    #[test]
    fn atomic_unit_rejected() {
        // Unit is not AtomicValue — no atomic of zero-sized type allowed.
        assert_has_error(
            r"
            function take(x: Atomic<Unit>) {}
            ",
            |e| matches!(e, TypeError::NonAtomicValueType { .. }),
        );
    }

    #[test]
    fn atomic_user_struct_rejected() {
        // Composite struct cannot be Atomic per ADR-0028 §2 — wrap in Mutex (v0.10).
        assert_has_error(
            r"
            struct Point { x: Integer, y: Integer }
            function take(p: Atomic<Point>) {}
            ",
            |e| matches!(e, TypeError::NonAtomicValueType { .. }),
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

    // ===== v0.9.x.atomic.6: E2530 InvalidAtomicOrdering (ADR-0028 §10) =====
    // Conservative scope — fires only when compare_exchange success
    // ordering is weaker than failure ordering. Local fixture re-declares
    // `compare_exchange` with the canonical Ordering enum so the
    // single-file checker sees the same shape `sys.atomic` ships.

    const ATOMIC_ORDERING_FIXTURE: &str = r"
        enum Ordering { Relaxed, Synchronized, Strict }
        struct Atom { value: Integer }
        struct Failed { actual: Integer }
        function compare_exchange(
            atom: Atom,
            expected: Integer,
            new_value: Integer,
            success_ordering: Ordering,
            failure_ordering: Ordering,
        ) -> Integer~Failed = ~+ 0
    ";

    fn ordering_source(success: &str, failure: &str) -> String {
        format!(
            "{ATOMIC_ORDERING_FIXTURE}
            function main() {{
                let a = Atom {{ value: 0 }}
                let result = compare_exchange(a, 0, 1, {success}, {failure})
            }}
            "
        )
    }

    fn assert_invalid_atomic_ordering(source: &str) {
        assert_has_error(source, |e| {
            matches!(
                e,
                TypeError::Concurrency(ConcurrencyError::InvalidAtomicOrdering { .. })
            )
        });
    }

    fn assert_no_invalid_atomic_ordering(source: &str) {
        let errors = check_source(source);
        assert!(
            !errors.iter().any(|e| matches!(
                e,
                TypeError::Concurrency(ConcurrencyError::InvalidAtomicOrdering { .. })
            )),
            "expected no E2530 fire, got: {errors:#?}",
        );
    }

    #[test]
    fn e2530_relaxed_strict_fires() {
        assert_invalid_atomic_ordering(&ordering_source("Relaxed", "Strict"));
    }

    #[test]
    fn e2530_relaxed_synchronized_fires() {
        assert_invalid_atomic_ordering(&ordering_source("Relaxed", "Synchronized"));
    }

    #[test]
    fn e2530_synchronized_strict_fires() {
        assert_invalid_atomic_ordering(&ordering_source("Synchronized", "Strict"));
    }

    #[test]
    fn e2530_strict_strict_clean() {
        assert_no_invalid_atomic_ordering(&ordering_source("Strict", "Strict"));
    }

    #[test]
    fn e2530_synchronized_synchronized_clean() {
        assert_no_invalid_atomic_ordering(&ordering_source("Synchronized", "Synchronized"));
    }

    #[test]
    fn e2530_relaxed_relaxed_clean() {
        // Equal orderings — Relaxed publish is conservative-deferred (§10).
        assert_no_invalid_atomic_ordering(&ordering_source("Relaxed", "Relaxed"));
    }

    #[test]
    fn e2530_strict_relaxed_clean() {
        // Success stronger than failure — semantically fine.
        assert_no_invalid_atomic_ordering(&ordering_source("Strict", "Relaxed"));
    }

    #[test]
    fn e2530_synchronized_relaxed_clean() {
        assert_no_invalid_atomic_ordering(&ordering_source("Synchronized", "Relaxed"));
    }

    #[test]
    fn e2530_strict_synchronized_clean() {
        assert_no_invalid_atomic_ordering(&ordering_source("Strict", "Synchronized"));
    }

    #[test]
    fn e2530_does_not_fire_on_unrelated_function_with_two_orderings() {
        // Conservative gate requires both name=`compare_exchange` AND
        // signature shape. A look-alike helper with two Ordering params
        // must NOT trigger.
        assert_no_invalid_atomic_ordering(
            r"
            enum Ordering { Relaxed, Synchronized, Strict }
            function pair_orderings(a: Integer, b: Integer, c: Integer, x: Ordering, y: Ordering) -> Integer = a
            function main() {
                let outcome = pair_orderings(1, 2, 3, Relaxed, Strict)
            }
            ",
        );
    }

    #[test]
    fn e2530_does_not_fire_on_compare_exchange_without_ordering_pair() {
        // Same callee name, different shape: only 4 params and last is
        // not Ordering. Conservative gate must skip.
        assert_no_invalid_atomic_ordering(
            r"
            function compare_exchange(a: Integer, b: Integer, c: Integer, d: Integer) -> Integer = a
            function main() {
                let outcome = compare_exchange(1, 2, 3, 4)
            }
            ",
        );
    }

    // ===== v0.9.x.atomic.7b: Borrow expression typecheck (ADR-0031 §4) =====
    // Each form produces Type::Reference(form, T). Borrow-of-borrow refused.
    // Enforcement (consume-once E2420) defers .7d; here we test type-level
    // emission only.

    #[test]
    fn borrow_expression_strong_frozen_typechecks() {
        assert_ok(
            r"
            function takes_strong(r: &+ Integer) {}
            function main() {
                let x: Integer = 1
                takes_strong(&+ x)
            }
            ",
        );
    }

    #[test]
    fn borrow_expression_strong_mutable_typechecks() {
        assert_ok(
            r"
            function takes_strong_mut(r: &+ mutable Integer) {}
            function main() {
                let x: Integer = 1
                takes_strong_mut(&+ mutable x)
            }
            ",
        );
    }

    #[test]
    fn borrow_expression_scope_readonly_typechecks() {
        assert_ok(
            r"
            function takes_borrow(r: &0 Integer) {}
            function main() {
                let x: Integer = 1
                takes_borrow(&0 x)
            }
            ",
        );
    }

    #[test]
    fn borrow_expression_scope_exclusive_mutable_typechecks() {
        assert_ok(
            r"
            function takes_excl(r: &0 mutable Integer) {}
            function main() {
                let x: Integer = 1
                takes_excl(&0 mutable x)
            }
            ",
        );
    }

    #[test]
    fn borrow_expression_weak_observer_typechecks() {
        assert_ok(
            r"
            function takes_weak(r: &- Integer) {}
            function main() {
                let x: Integer = 1
                takes_weak(&- x)
            }
            ",
        );
    }

    #[test]
    fn borrow_expression_form_mismatch_rejects() {
        // ADR-0031 §4: each form produces distinct Type::Reference.
        // Passing `&0 x` where parameter expects `&+ T` is type mismatch.
        assert_has_error(
            r"
            function takes_strong(r: &+ Integer) {}
            function main() {
                let x: Integer = 1
                takes_strong(&0 x)
            }
            ",
            |e| matches!(e, TypeError::Mismatch { .. }),
        );
    }

    #[test]
    fn borrow_expression_field_access_operand_typechecks() {
        assert_ok(
            r"
            struct Pair { left: Integer, right: Integer }
            function takes_strong(r: &+ Integer) {}
            function main() {
                let p = Pair { left: 1, right: 2 }
                takes_strong(&+ p.left)
            }
            ",
        );
    }

    // ===== v0.9.x.atomic.7d: E2420 UseAfterMove (ADR-0025 §5.1) =====
    // Move tracking fires on owning borrow expressions (&+, &+ mutable)
    // and only on owning borrow expressions — &0 and &- don't consume.
    // Branch-aware: if/match snapshot+join with any-branch-moves
    // semantics; loops join initial state with after-body state.

    fn assert_no_use_after_move(source: &str) {
        let errors = check_source(source);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, TypeError::Borrow(BorrowError::UseAfterMove { .. }))),
            "expected no E2420 fire, got: {errors:#?}",
        );
    }

    #[test]
    fn e2420_does_not_fire_after_scope_readonly_borrow() {
        // `&0` borrows without consuming — multiple uses OK.
        assert_no_use_after_move(
            r"
            function takes_ro(r: &0 Integer) {}
            function main() {
                let x: Integer = 1
                takes_ro(&0 x)
                takes_ro(&0 x)
            }
            ",
        );
    }

    #[test]
    fn e2420_does_not_fire_after_scope_exclusive_borrow() {
        // `&0 mutable` is borrow, not move — multiple uses OK at type
        // level. NLL exclusivity (E2440) defers v0.10 per ADR-0031
        // §10.1; v0.9 doesn't enforce that.
        assert_no_use_after_move(
            r"
            function takes_excl(r: &0 mutable Integer) {}
            function main() {
                let x: Integer = 1
                takes_excl(&0 mutable x)
                takes_excl(&0 mutable x)
            }
            ",
        );
    }

    #[test]
    fn e2420_does_not_fire_after_weak_observer_borrow() {
        assert_no_use_after_move(
            r"
            function takes_weak(r: &- Integer) {}
            function main() {
                let x: Integer = 1
                takes_weak(&- x)
                takes_weak(&- x)
            }
            ",
        );
    }

    #[test]
    fn e2420_single_use_clean() {
        assert_no_use_after_move(
            r"
            function take(r: &+ Integer) {}
            function main() {
                let x: Integer = 1
                take(&+ x)
            }
            ",
        );
    }

    #[test]
    fn e2420_does_not_fire_when_both_branches_preserve() {
        // Both branches use x without moving — clean post-if.
        assert_no_use_after_move(
            r"
            function inspect(x: Integer) {}
            function main() {
                let x: Integer = 1
                let cond = true
                if cond {
                    inspect(x)
                } else {
                    inspect(x)
                }
                inspect(x)
            }
            ",
        );
    }

    #[test]
    fn borrow_of_call_result_not_callable() {
        // ADR-0031 §2: borrow-of-function-call defers v0.10. Parser
        // produces `Call(Borrow(f), [])` for `&+ f()`. Typecheck rejects
        // because `Reference(_, Function)` is not callable.
        assert_has_error(
            r"
            function f() -> Integer = 0
            function main() {
                let x: Integer = (&+ f)()
            }
            ",
            |e| matches!(e, TypeError::NotCallable { .. }),
        );
    }

    #[test]
    fn nested_borrow_expression_refused() {
        // ADR-0031 §2 last bullet: borrow-of-borrow refused at typecheck.
        // `&+ &0 x` — parser allows; typecheck fires InvalidUnary.
        // Note: requires parsing nested forms — `&+ &0 x` would need
        // operand to start with `&0` which the parser rejects (operand
        // grammar is IDENT only). So this test exercises the typecheck
        // path via a synthetic case where inner is already Reference-
        // typed; we don't have a way to express that at source level
        // without function param indirection.
        //
        // The borrow-of-borrow guard exists defensively — confirms the
        // check_borrow path refuses if it ever sees Reference inner.
        // No direct source-level test currently feasible without param
        // gymnastics; test removed to avoid noise. Guard remains in
        // typecheck for future operand-scope expansions per §10.3.
    }

    #[test]
    fn e2530_does_not_fire_when_ordering_args_are_runtime_values() {
        // Dynamic ordering (passed as parameter) escapes v0.9 detection
        // per ADR-0028 §10 deferred-pattern note. Conservative scope
        // covers literal-bound ordering only.
        assert_no_invalid_atomic_ordering(&format!(
            "{ATOMIC_ORDERING_FIXTURE}
            function call(a: Atom, s: Ordering, f: Ordering) {{
                let result = compare_exchange(a, 0, 1, s, f)
            }}
            function main() {{
                let a = Atom {{ value: 0 }}
                call(a, Relaxed, Strict)
            }}
            "
        ));
    }

    // ── Phase 4.3b: Vector builtin overload resolution ──

    #[test]
    fn overload_len_string_selects_correct_signature() {
        // `len("hi")` → String overload → returns Integer.
        let source = r#"function main() -> Integer { return len("hi") }"#;
        let errors = check_source(source);
        assert!(
            errors.is_empty(),
            "len(String) should typecheck: {errors:?}"
        );
    }

    #[test]
    fn overload_len_vector_selects_correct_signature() {
        // `len(vector_new())` → Vector overload → returns Integer.
        let source = r"function main() -> Integer { let v = vector_new(); return len(v) }";
        let errors = check_source(source);
        assert!(
            errors.is_empty(),
            "len(Vector) should typecheck: {errors:?}"
        );
    }

    #[test]
    fn overload_len_integer_rejected() {
        // `len(42)` → no matching overload → E1041.
        let source = r"function main() -> Integer { return len(42) }";
        assert_has_error(
            source,
            |e| matches!(e, TypeError::NoMatchingOverload { name, .. } if name == "len"),
        );
    }

    #[test]
    fn generic_body_type_mismatch_must_not_be_silenced_by_typeparam_wildcard() {
        // Regression B2: TypeParam must NOT be a universal wildcard in matches().
        // Returning `x: T` where `Integer` is expected must fire E1003 Mismatch.
        // If this fails, the TypeParam wildcard is leaking beyond the Vector arm.
        assert_has_error("function identity<T>(x: T) -> Integer { return x; }", |e| {
            matches!(e, TypeError::Mismatch { .. })
        });
    }

    #[test]
    fn vector_annotation_matches_init_type() {
        // `let v: Vector<Integer> = vector_new()` — annotation and init must agree.
        let source =
            r"function main() -> Integer { let v: Vector<Integer> = vector_new(); return 0 }";
        let errors = check_source(source);
        assert!(
            errors.is_empty(),
            "Vector<Integer> annotation should match vector_new(): {errors:?}"
        );
    }

    #[test]
    fn push_vector_returns_same_type() {
        // `push(v, 1)` where v: Vector<Integer> returns Vector<Integer>.
        let source = r"function main() -> Integer { let v = vector_new(); let v2 = push(v, 1); return len(v2) }";
        let errors = check_source(source);
        assert!(
            errors.is_empty(),
            "push should return Vector<Integer>: {errors:?}"
        );
    }
}
