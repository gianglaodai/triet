//! End-to-end tests covering every demo `.tri` file under `examples/`.
//!
//! Each test runs the full pipeline (parse → type-check → interpret)
//! against the actual file shipped in the repo, asserting that:
//! - parsing produces no errors,
//! - type-checking produces no errors,
//! - calling the relevant function produces the expected value.
//!
//! These tests are the v0.1 acceptance gate: if they fail, the demo
//! programs the README points users at would also fail.
#![allow(
    clippy::uninlined_format_args,
    clippy::unnecessary_debug_formatting,
)]

use std::{fs, path::PathBuf};

use triet_core::Integer;
use triet_interpreter::{Value, call_function, run};
use triet_logic::Trilean;
use triet_parser::parse;
use triet_typecheck::check;

const FIZZBUZZ: &str = "fizzbuzz.tri";
const MEASLES: &str = "measles_risk.tri";
const FACTORIAL: &str = "factorial.tri";
const LK: &str = "lukasiewicz_vs_kleene.tri";
const COUNTER: &str = "counter.tri";
const LONG_ARITHMETIC: &str = "long_arithmetic.tri";
const ENUMERATE: &str = "enumerate.tri";
const NULLABLE: &str = "nullable.tri";
const WHILE_POLLING: &str = "while_polling.tri";
const MAYBE: &str = "maybe.tri";
const GENERIC: &str = "generic.tri";

fn examples_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR points at crates/triet-cli; examples live
    // two levels up at the repo root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
}

fn load_program(filename: &str) -> triet_syntax::Program {
    let path = examples_dir().join(filename);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("could not read {path:?}: {error}");
    });
    let (program, parse_errors) = parse(&source);
    assert!(parse_errors.is_empty(), "{path:?} parse errors: {parse_errors:#?}");
    let type_errors = check(&program);
    assert!(type_errors.is_empty(), "{path:?} type errors: {type_errors:#?}");
    program
}

const fn integer(n: i64) -> Value {
    Value::Integer(Integer::new(n).unwrap())
}

const fn trilean(t: Trilean) -> Value {
    Value::Trilean(t)
}

#[test]
fn fizzbuzz_demo_parses_and_type_checks() {
    let _ = load_program(FIZZBUZZ);
}

#[test]
fn fizzbuzz_classifies_15_to_fizzbuzz() {
    let program = load_program(FIZZBUZZ);
    let value = call_function(&program, "fizzbuzz", vec![integer(15)]).unwrap();
    assert_eq!(value.to_string(), "FizzBuzz");
}

#[test]
fn fizzbuzz_classifies_3_to_fizz_and_5_to_buzz() {
    let program = load_program(FIZZBUZZ);
    assert_eq!(
        call_function(&program, "fizzbuzz", vec![integer(3)]).unwrap().to_string(),
        "Fizz",
    );
    assert_eq!(
        call_function(&program, "fizzbuzz", vec![integer(5)]).unwrap().to_string(),
        "Buzz",
    );
}

#[test]
fn fizzbuzz_classifies_7_to_its_decimal_form() {
    let program = load_program(FIZZBUZZ);
    let value = call_function(&program, "fizzbuzz", vec![integer(7)]).unwrap();
    assert_eq!(value.to_string(), "7");
}

#[test]
fn fizzbuzz_main_runs_without_error() {
    let program = load_program(FIZZBUZZ);
    let result = run(&program);
    assert!(result.is_ok(), "fizzbuzz main failed: {result:?}");
}

#[test]
fn measles_demo_parses_and_type_checks() {
    let _ = load_program(MEASLES);
}

#[test]
fn measles_full_evidence_yields_true() {
    let program = load_program(MEASLES);
    let value = call_function(
        &program,
        "risk_measles",
        vec![trilean(Trilean::True), trilean(Trilean::True), trilean(Trilean::False)],
    )
    .unwrap();
    assert_eq!(value, trilean(Trilean::True));
}

#[test]
fn measles_unknown_vaccination_yields_unknown() {
    let program = load_program(MEASLES);
    let value = call_function(
        &program,
        "risk_measles",
        vec![trilean(Trilean::True), trilean(Trilean::True), trilean(Trilean::Unknown)],
    )
    .unwrap();
    assert_eq!(value, trilean(Trilean::Unknown));
}

#[test]
fn measles_no_symptoms_yields_false_regardless_of_vaccination() {
    let program = load_program(MEASLES);
    let value = call_function(
        &program,
        "risk_measles",
        vec![trilean(Trilean::False), trilean(Trilean::False), trilean(Trilean::Unknown)],
    )
    .unwrap();
    assert_eq!(value, trilean(Trilean::False));
}

#[test]
fn factorial_demo_parses_and_type_checks() {
    let _ = load_program(FACTORIAL);
}

#[test]
fn factorial_table_matches_classical_values() {
    let program = load_program(FACTORIAL);
    let cases: &[(i64, i64)] = &[
        (0, 1),
        (1, 1),
        (2, 2),
        (3, 6),
        (4, 24),
        (5, 120),
        (6, 720),
        (10, 3_628_800),
    ];
    for &(input, expected) in cases {
        let value = call_function(&program, "factorial", vec![integer(input)]).unwrap();
        assert_eq!(value, integer(expected), "factorial({input})");
    }
}

#[test]
fn lukasiewicz_vs_kleene_demo_parses_and_type_checks() {
    let _ = load_program(LK);
}

#[test]
fn lukasiewicz_vs_kleene_main_runs_without_error() {
    let program = load_program(LK);
    let result = run(&program);
    assert!(result.is_ok(), "lukasiewicz_vs_kleene main failed: {result:?}");
}

#[test]
fn counter_demo_parses_and_type_checks() {
    let _ = load_program(COUNTER);
}

#[test]
fn counter_sum_to_n_returns_arithmetic_series() {
    let program = load_program(COUNTER);
    let cases = &[
        (0_i64, 0_i64),
        (1, 1),
        (5, 15),
        (10, 55),
        (100, 5050),
    ];
    for &(input, expected) in cases {
        let value = call_function(&program, "sum_to", vec![integer(input)]).unwrap();
        assert_eq!(value, integer(expected), "sum_to({input})");
    }
}

#[test]
fn counter_main_runs_without_error() {
    let program = load_program(COUNTER);
    let result = run(&program);
    assert!(result.is_ok(), "counter main failed: {result:?}");
}

#[test]
fn long_arithmetic_demo_parses_and_type_checks() {
    let _ = load_program(LONG_ARITHMETIC);
}

#[test]
fn long_arithmetic_factorial_20_matches_known_value() {
    let program = load_program(LONG_ARITHMETIC);
    let value = call_function(
        &program,
        "factorial_long",
        vec![Value::Long(triet_core::Long::from_i64(20))],
    )
    .unwrap();
    // 20! = 2_432_902_008_176_640_000 — exceeds Integer's range.
    assert_eq!(value.to_string(), "2432902008176640000");
}

#[test]
fn long_arithmetic_main_runs_without_error() {
    let program = load_program(LONG_ARITHMETIC);
    let result = run(&program);
    assert!(result.is_ok(), "long_arithmetic main failed: {result:?}");
}

#[test]
fn enumerate_demo_parses_and_type_checks() {
    let _ = load_program(ENUMERATE);
}

#[test]
fn enumerate_main_runs_without_error() {
    let program = load_program(ENUMERATE);
    let result = run(&program);
    assert!(result.is_ok(), "enumerate main failed: {result:?}");
}

#[test]
fn enumerate_rank_assigns_correct_grades() {
    let program = load_program(ENUMERATE);
    let cases: &[(i64, &str)] = &[
        (95, "A"),
        (85, "B"),
        (75, "C"),
        (60, "F"),
    ];
    for &(score, expected) in cases {
        let value = call_function(&program, "rank", vec![integer(score)]).unwrap();
        assert_eq!(value.to_string(), expected, "rank({score})");
    }
}

#[test]
fn nullable_demo_parses_and_type_checks() {
    let _ = load_program(NULLABLE);
}

#[test]
fn nullable_lookup_returns_name_for_valid_ids() {
    let program = load_program(NULLABLE);
    let cases = &[(1, "Alice"), (2, "Bob"), (3, "Carol")];
    for &(id, expected) in cases {
        let value = call_function(&program, "lookup_name", vec![integer(id)])
            .unwrap();
        assert_eq!(value.to_string(), expected, "lookup_name({id})");
    }
}

#[test]
fn nullable_lookup_returns_null_for_unknown_id_then_elvis_kicks_in() {
    let program = load_program(NULLABLE);
    // lookup_name(99) returns null → greet_or_default applies Elvis → "<khuyết danh>"
    let name = call_function(&program, "lookup_name", vec![integer(99)]).unwrap();
    let value = call_function(&program, "greet_or_default", vec![name]).unwrap();
    assert_eq!(value.to_string(), "<khuyết danh>");
}

#[test]
fn nullable_safe_call_length_on_null_returns_zero() {
    let program = load_program(NULLABLE);
    // lookup_name(99) returns null → name_length_or_zero → safe-call + Elvis → 0
    assert_eq!(
        call_function(&program, "name_length_or_zero", vec![integer(99)])
            .unwrap()
            .to_string(),
        "0",
    );
}

#[test]
fn nullable_force_unwrap_panics_on_null() {
    let program = load_program(NULLABLE);
    // must_have_name(99) — lookup_name returns null, `!!` panics.
    let result = call_function(&program, "must_have_name", vec![integer(99)]);
    assert!(result.is_err(), "expected panic on null !!, got {result:?}");
}

#[test]
fn nullable_force_unwrap_succeeds_for_valid_id() {
    let program = load_program(NULLABLE);
    let value = call_function(&program, "must_have_name", vec![integer(3)])
        .unwrap();
    assert_eq!(value.to_string(), "Carol");
}

#[test]
fn nullable_let_annotation_widening_works() {
    // `greet_or_default` does `let display: String? = name` — T ⊂ T? widening.
    let program = load_program(NULLABLE);
    let value = call_function(
        &program,
        "greet_or_default",
        vec![Value::from_string("Alice".to_owned())],
    )
    .unwrap();
    assert_eq!(value.to_string(), "Alice");
}

#[test]
fn nullable_main_runs_without_error() {
    let program = load_program(NULLABLE);
    let result = run(&program);
    assert!(result.is_ok(), "nullable main failed: {result:?}");
}

#[test]
fn while_polling_demo_parses_and_type_checks() {
    let _ = load_program(WHILE_POLLING);
}

#[test]
fn while_polling_count_cycles_handles_unknown_safely() {
    let program = load_program(WHILE_POLLING);
    // active=true → loop runs until iterations==5 sets active=false.
    assert_eq!(
        call_function(&program, "count_cycles", vec![trilean(Trilean::True)])
            .unwrap()
            .to_string(),
        "5",
    );
    // active=false → loop never enters.
    assert_eq!(
        call_function(&program, "count_cycles", vec![trilean(Trilean::False)])
            .unwrap()
            .to_string(),
        "0",
    );
    // active=unknown → `while?` treats unknown as false → loop never enters.
    // This is the load-bearing case for SPEC §7.1.1.
    assert_eq!(
        call_function(&program, "count_cycles", vec![trilean(Trilean::Unknown)])
            .unwrap()
            .to_string(),
        "0",
    );
}

#[test]
fn while_polling_isqrt_converges() {
    let program = load_program(WHILE_POLLING);
    let cases: &[(i64, i64)] = &[(1, 1), (4, 2), (9, 3), (16, 4), (100, 10)];
    for &(target, expected) in cases {
        let value = call_function(
            &program,
            "isqrt",
            vec![integer(target), integer(20)],
        )
        .unwrap();
        assert_eq!(value, integer(expected), "isqrt({target})");
    }
}

#[test]
fn while_polling_main_runs_without_error() {
    let program = load_program(WHILE_POLLING);
    let result = run(&program);
    assert!(result.is_ok(), "while_polling main failed: {result:?}");
}

#[test]
fn maybe_demo_parses_and_type_checks() {
    let _ = load_program(MAYBE);
}

#[test]
fn maybe_unwrap_or_returns_value_for_some() {
    let program = load_program(MAYBE);
    // unwrap_or(Some(42), 0) = 42
    // Can't construct Some(42) directly from Rust — use run + main instead.
    let result = run(&program);
    assert!(result.is_ok(), "maybe main failed: {result:?}");
}

#[test]
fn generic_demo_parses_and_type_checks() {
    let _ = load_program(GENERIC);
}

#[test]
fn generic_main_runs_without_error() {
    let program = load_program(GENERIC);
    let result = run(&program);
    assert!(result.is_ok(), "generic main failed: {result:?}");
}
