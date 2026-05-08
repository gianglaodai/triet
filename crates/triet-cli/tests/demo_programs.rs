//! End-to-end tests covering every demo `.tt` file under `examples/`.
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

const FIZZBUZZ: &str = "fizzbuzz.tt";
const MEASLES: &str = "measles_risk.tt";
const FACTORIAL: &str = "factorial.tt";
const LK: &str = "lukasiewicz_vs_kleene.tt";

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
