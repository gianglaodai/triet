//! Triết tree-walking interpreter.
//!
//! Walks a parsed `Program` and produces a [`Value`]. The v0.1
//! interpreter:
//!
//! - Implements every expression form and statement type covered by
//!   the parser and type checker.
//! - Uses Mojo-style ARC (`Rc`) for heap-allocated values; stack types
//!   (Trit/Tryte/Integer/Trilean/Unit) are plain copies.
//! - Default arithmetic operators panic on overflow per SPEC §3.3,
//!   surfaced as a `RuntimeError::Panic` rather than a process crash.
//! - Built-in `print` / `println` / `to_string` / etc. are pre-bound
//!   in the runtime environment.
//!
//! # Public API
//!
//! - [`run`] runs `main()` of a program.
//! - [`call_function`] invokes a named top-level function with given
//!   arguments — useful for tests.

#![warn(missing_docs)]
#![allow(
    clippy::redundant_pub_crate,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::missing_panics_doc,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::match_same_arms,
    clippy::unnecessary_wraps,
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
)]

mod builtins;
mod env;
mod error;
mod interpret;
mod value;

pub use env::ValueEnvironment;
pub use error::RuntimeError;
pub use interpret::{call_function, run};
pub use value::Value;

#[cfg(test)]
mod tests {
    use super::*;
    use triet_core::Integer;
    use triet_logic::Trilean;
    use triet_parser::parse;

    fn parse_ok(source: &str) -> triet_syntax::Program {
        let (program, errors) = parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:#?}");
        program
    }

    fn run_function(source: &str, name: &str, arguments: Vec<Value>) -> Value {
        let program = parse_ok(source);
        call_function(&program, name, arguments).expect("runtime error")
    }

    fn integer(n: i64) -> Value {
        Value::Integer(Integer::new(n).expect("in range"))
    }

    fn trilean(t: Trilean) -> Value {
        Value::Trilean(t)
    }

    // ===== Basic literals & arithmetic =====

    #[test]
    fn evaluates_integer_literal() {
        let value = run_function("fn answer() -> Integer = 42", "answer", vec![]);
        assert_eq!(value, integer(42));
    }

    #[test]
    fn evaluates_simple_addition() {
        let value = run_function(
            "fn sum(a: Integer, b: Integer) -> Integer = a + b",
            "sum",
            vec![integer(3), integer(5)],
        );
        assert_eq!(value, integer(8));
    }

    #[test]
    fn evaluates_arithmetic_precedence() {
        let value = run_function(
            "fn calc() -> Integer = 2 + 3 * 4",
            "calc",
            vec![],
        );
        assert_eq!(value, integer(14));
    }

    #[test]
    fn evaluates_power_right_associative() {
        // 2 ** 3 ** 2 = 2 ** (3 ** 2) = 2 ** 9 = 512
        let value = run_function(
            "fn calc() -> Integer = 2 ** 3 ** 2",
            "calc",
            vec![],
        );
        assert_eq!(value, integer(512));
    }

    #[test]
    fn evaluates_unary_negation() {
        let value = run_function("fn neg(n: Integer) -> Integer = -n", "neg", vec![integer(7)]);
        assert_eq!(value, integer(-7));
    }

    #[test]
    fn evaluates_modulo() {
        let value = run_function(
            "fn rem(a: Integer, b: Integer) -> Integer = a %% b",
            "rem",
            vec![integer(10), integer(3)],
        );
        assert_eq!(value, integer(1));
    }

    // ===== Trilean / logic =====

    #[test]
    fn evaluates_trilean_literals() {
        assert_eq!(run_function("fn t() -> Trilean = true", "t", vec![]), trilean(Trilean::True));
        assert_eq!(run_function("fn f() -> Trilean = false", "f", vec![]), trilean(Trilean::False));
        assert_eq!(
            run_function("fn u() -> Trilean = unknown", "u", vec![]),
            trilean(Trilean::Unknown),
        );
    }

    #[test]
    fn evaluates_logic_and() {
        let source = "fn ann(a: Trilean, b: Trilean) -> Trilean = a and b";
        assert_eq!(
            run_function(source, "ann", vec![trilean(Trilean::True), trilean(Trilean::True)]),
            trilean(Trilean::True),
        );
        assert_eq!(
            run_function(source, "ann", vec![trilean(Trilean::True), trilean(Trilean::Unknown)]),
            trilean(Trilean::Unknown),
        );
        assert_eq!(
            run_function(source, "ann", vec![trilean(Trilean::False), trilean(Trilean::Unknown)]),
            trilean(Trilean::False),
        );
    }

    #[test]
    fn evaluates_lukasiewicz_implies_distinguishes_unknown_unknown() {
        let source = "fn imp(a: Trilean, b: Trilean) -> Trilean = a implies b";
        assert_eq!(
            run_function(source, "imp", vec![trilean(Trilean::Unknown), trilean(Trilean::Unknown)]),
            trilean(Trilean::True),
        );
    }

    #[test]
    fn evaluates_kleene_implies_distinguishes_unknown_unknown() {
        let source = "fn imp(a: Trilean, b: Trilean) -> Trilean = a kleene_implies b";
        assert_eq!(
            run_function(source, "imp", vec![trilean(Trilean::Unknown), trilean(Trilean::Unknown)]),
            trilean(Trilean::Unknown),
        );
    }

    #[test]
    fn evaluates_unary_not_on_trilean() {
        let source = "fn nott(a: Trilean) -> Trilean = !a";
        assert_eq!(
            run_function(source, "nott", vec![trilean(Trilean::True)]),
            trilean(Trilean::False),
        );
    }

    // ===== Comparison =====

    #[test]
    fn evaluates_less_than() {
        let source = "fn lt(a: Integer, b: Integer) -> Trilean = a < b";
        assert_eq!(
            run_function(source, "lt", vec![integer(1), integer(2)]),
            trilean(Trilean::True),
        );
        assert_eq!(
            run_function(source, "lt", vec![integer(2), integer(1)]),
            trilean(Trilean::False),
        );
    }

    #[test]
    fn evaluates_equality_returns_trilean_true_or_false() {
        let source = "fn eq(a: Integer, b: Integer) -> Trilean = a == b";
        assert_eq!(
            run_function(source, "eq", vec![integer(5), integer(5)]),
            trilean(Trilean::True),
        );
        assert_eq!(
            run_function(source, "eq", vec![integer(5), integer(7)]),
            trilean(Trilean::False),
        );
    }

    // ===== Control flow =====

    #[test]
    fn evaluates_if_taking_then_branch() {
        let source = r"fn pick(b: Trilean) -> Integer { if b { 1 } else { 0 } }";
        assert_eq!(run_function(source, "pick", vec![trilean(Trilean::True)]), integer(1));
        assert_eq!(run_function(source, "pick", vec![trilean(Trilean::False)]), integer(0));
    }

    #[test]
    fn evaluates_if_question_treats_unknown_as_false() {
        let source = r"fn pick(b: Trilean) -> Integer { if? b { 1 } else { 0 } }";
        assert_eq!(run_function(source, "pick", vec![trilean(Trilean::Unknown)]), integer(0));
    }

    #[test]
    fn plain_if_with_unknown_is_runtime_error() {
        let source = r"fn pick(b: Trilean) -> Integer { if b { 1 } else { 0 } }";
        let program = parse_ok(source);
        let result = call_function(&program, "pick", vec![trilean(Trilean::Unknown)]);
        assert!(matches!(result, Err(RuntimeError::UnknownCondition { .. })));
    }

    #[test]
    fn evaluates_match_picks_first_matching_arm() {
        let source = r#"
            fn classify(n: Integer) -> String =
                match n {
                    0 => "zero",
                    _ => "other",
                }
        "#;
        let value = run_function(source, "classify", vec![integer(0)]);
        assert_eq!(value.to_string(), "zero");

        let value = run_function(source, "classify", vec![integer(5)]);
        assert_eq!(value.to_string(), "other");
    }

    // ===== Method calls =====

    #[test]
    fn evaluates_assume_known_passes_through_known_trilean() {
        let source = "fn force(t: Trilean) -> Trilean = t.assume_known()";
        assert_eq!(
            run_function(source, "force", vec![trilean(Trilean::True)]),
            trilean(Trilean::True),
        );
    }

    #[test]
    fn assume_known_panics_on_unknown() {
        let source = "fn force(t: Trilean) -> Trilean = t.assume_known()";
        let program = parse_ok(source);
        let result = call_function(&program, "force", vec![trilean(Trilean::Unknown)]);
        assert!(matches!(result, Err(RuntimeError::Panic { .. })));
    }

    #[test]
    fn evaluates_string_length_method() {
        let source = r"fn len(s: String) -> Integer = s.length()";
        let value = run_function(
            source,
            "len",
            vec![Value::from_string("hello".to_owned())],
        );
        assert_eq!(value, integer(5));
    }

    // ===== Nullable =====

    #[test]
    fn evaluates_elvis_returns_default_for_null() {
        let source = r#"
            fn pick(s: String?) -> String = s ?: "fallback"
        "#;
        let value = run_function(source, "pick", vec![Value::Null]);
        assert_eq!(value.to_string(), "fallback");
    }

    #[test]
    fn evaluates_elvis_returns_unwrapped_for_non_null() {
        let source = r#"fn pick(s: String?) -> String = s ?: "fallback""#;
        let value = run_function(
            source,
            "pick",
            vec![Value::from_string("real".to_owned())],
        );
        assert_eq!(value.to_string(), "real");
    }

    #[test]
    fn force_unwrap_returns_value_for_non_null() {
        let source = "fn force(s: String?) -> String = s!!";
        let value = run_function(source, "force", vec![Value::from_string("x".to_owned())]);
        assert_eq!(value.to_string(), "x");
    }

    #[test]
    fn force_unwrap_panics_on_null() {
        let source = "fn force(s: String?) -> String = s!!";
        let program = parse_ok(source);
        let result = call_function(&program, "force", vec![Value::Null]);
        assert!(matches!(result, Err(RuntimeError::Panic { .. })));
    }

    // ===== Tuple =====

    #[test]
    fn evaluates_tuple_construction_and_index() {
        let source = r"
            fn make() -> Integer {
                let pair = (10, 20)
                pair.0
            }
        ";
        let program = parse_ok(source);
        let value = call_function(&program, "make", vec![]).unwrap();
        assert_eq!(value, integer(10));
    }

    #[test]
    fn evaluates_tuple_destructuring_in_match() {
        let source = r#"
            fn diag(pair: (Integer, Integer)) -> String =
                match pair {
                    (0, 0) => "origin",
                    _ => "elsewhere",
                }
        "#;
        let pair = Value::from_tuple(vec![integer(0), integer(0)]);
        let value = run_function(source, "diag", vec![pair]);
        assert_eq!(value.to_string(), "origin");
    }

    // ===== F-strings =====

    #[test]
    fn evaluates_f_string_with_interpolation() {
        let source = r#"fn greet(name: String) -> String = f"Xin chào, {name}!""#;
        let value = run_function(
            source,
            "greet",
            vec![Value::from_string("Giang".to_owned())],
        );
        assert_eq!(value.to_string(), "Xin chào, Giang!");
    }

    // ===== Recursion =====

    #[test]
    fn evaluates_recursive_factorial() {
        let source = r"
            fn fact(n: Integer) -> Integer =
                if? n <= 1 { 1 } else { n * fact(n - 1) }
        ";
        let value = run_function(source, "fact", vec![integer(5)]);
        assert_eq!(value, integer(120));
    }

    #[test]
    fn evaluates_mutual_recursion_via_forward_reference() {
        let source = r"
            fn even_q(n: Integer) -> Trilean =
                if? n == 0 { true } else { odd_q(n - 1) }

            fn odd_q(n: Integer) -> Trilean =
                if? n == 0 { false } else { even_q(n - 1) }
        ";
        assert_eq!(run_function(source, "even_q", vec![integer(4)]), trilean(Trilean::True));
        assert_eq!(run_function(source, "odd_q", vec![integer(3)]), trilean(Trilean::True));
    }

    // ===== End-to-end demos =====

    #[test]
    fn fizzbuzz_classifies_correctly() {
        let source = r#"
            fn fizzbuzz(n: Integer) -> String =
                match (n %% 3, n %% 5) {
                    (0, 0) => "FizzBuzz",
                    (0, _) => "Fizz",
                    (_, 0) => "Buzz",
                    _ => to_string(n),
                }
        "#;
        let cases = [
            (1, "1"),
            (2, "2"),
            (3, "Fizz"),
            (5, "Buzz"),
            (9, "Fizz"),
            (15, "FizzBuzz"),
            (30, "FizzBuzz"),
            (7, "7"),
        ];
        for (input, expected) in cases {
            let value = run_function(source, "fizzbuzz", vec![integer(input)]);
            assert_eq!(value.to_string(), expected, "for input {input}");
        }
    }

    #[test]
    fn measles_demo_propagates_unknown() {
        let source = r"
            fn risk(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean =
                fever and rash and not vaccinated
        ";
        assert_eq!(
            run_function(
                source,
                "risk",
                vec![trilean(Trilean::True), trilean(Trilean::True), trilean(Trilean::False)],
            ),
            trilean(Trilean::True),
        );
        assert_eq!(
            run_function(
                source,
                "risk",
                vec![trilean(Trilean::True), trilean(Trilean::True), trilean(Trilean::Unknown)],
            ),
            trilean(Trilean::Unknown),
        );
        assert_eq!(
            run_function(
                source,
                "risk",
                vec![trilean(Trilean::False), trilean(Trilean::False), trilean(Trilean::Unknown)],
            ),
            trilean(Trilean::False),
        );
    }

    // ===== Errors =====

    #[test]
    fn missing_main_returns_error() {
        let program = parse_ok("fn helper() -> Integer = 1");
        let result = run(&program);
        assert!(matches!(result, Err(RuntimeError::NoMainFunction)));
    }

    #[test]
    fn division_by_zero_panics() {
        let source = "fn div(a: Integer, b: Integer) -> Integer = a / b";
        let program = parse_ok(source);
        let result = call_function(&program, "div", vec![integer(10), integer(0)]);
        assert!(matches!(result, Err(RuntimeError::Panic { .. })));
    }

    #[test]
    fn arithmetic_overflow_panics() {
        let source = "fn overflow() -> Integer = 3812798742493 + 1";
        let program = parse_ok(source);
        let result = call_function(&program, "overflow", vec![]);
        assert!(matches!(result, Err(RuntimeError::Panic { .. })));
    }

    // ===== Assignment (SPEC §5) =====

    #[test]
    fn assignment_updates_mut_binding() {
        let source = r"
            fn final_value() -> Integer {
                let mut x = 0
                x = 5
                x
            }
        ";
        let value = run_function(source, "final_value", vec![]);
        assert_eq!(value, integer(5));
    }

    #[test]
    fn assignment_uses_old_value_in_rhs() {
        let source = r"
            fn final_value() -> Integer {
                let mut x = 10
                x = x + 1
                x
            }
        ";
        let value = run_function(source, "final_value", vec![]);
        assert_eq!(value, integer(11));
    }

    #[test]
    fn assignment_in_inner_block_updates_outer_binding() {
        let source = r"
            fn count_one() -> Integer {
                let mut count = 0
                if? true {
                    count = count + 1
                }
                count
            }
        ";
        let value = run_function(source, "count_one", vec![]);
        assert_eq!(value, integer(1));
    }

    #[test]
    fn counter_loop_with_while() {
        let source = r"
            fn sum_to_five() -> Integer {
                let mut i = 1
                let mut total = 0
                while? i <= 5 {
                    total = total + i
                    i = i + 1
                }
                total
            }
        ";
        let value = run_function(source, "sum_to_five", vec![]);
        assert_eq!(value, integer(15));
    }
}
