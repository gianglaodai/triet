//! v0.7.x.runtime-fix-debt.3 — `runtime_eq` now handles Struct,
//! Enum, Outcome, Closure, and Vector values. Pre-fix, `==` between
//! two structurally-identical struct or enum values returned `false`
//! because the match fell through to the `_ => false` catch-all.
//!
//! These tests exercise the full pipeline (load → typecheck → lower →
//! VM execute) with user-defined types compared via `==`.

use triet_ir::{RuntimeValue, Vm, lower_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;
use miette::Diagnostic;

fn run_main(source: &str) -> RuntimeValue {
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(blocking.is_empty(), "type errors: {blocking:?}");
    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main()")
        .id;
    let mut vm = Vm::new(ir);
    vm.execute(main_id, vec![]).expect("vm execute")
}

fn assert_trilean(result: RuntimeValue, expected: triet_logic::Trilean) {
    match result {
        RuntimeValue::Trilean(t) => assert_eq!(t, expected),
        other => panic!("expected Trilean({expected:?}), got {other:?}"),
    }
}

fn assert_integer(result: RuntimeValue, expected: i64) {
    match result {
        RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), expected),
        other => panic!("expected Integer({expected}), got {other:?}"),
    }
}

#[test]
fn struct_eq_identical_returns_true() {
    let source = r"
public struct Point { x: Integer, y: Integer }
function main() -> Trilean {
    let a: Point = Point { x: 1, y: 2 }
    let b: Point = Point { x: 1, y: 2 }
    a == b
}
";
    assert_trilean(run_main(source), triet_logic::Trilean::True);
}

#[test]
fn struct_eq_different_returns_false() {
    let source = r"
public struct Point { x: Integer, y: Integer }
function main() -> Trilean {
    let a: Point = Point { x: 1, y: 2 }
    let b: Point = Point { x: 3, y: 4 }
    a == b
}
";
    assert_trilean(run_main(source), triet_logic::Trilean::False);
}

#[test]
fn bare_unit_enum_eq_identical_returns_true() {
    let source = r"
public enum Color { Red, Green, Blue }
function main() -> Trilean {
    let a: Color = Red
    let b: Color = Red
    a == b
}
";
    assert_trilean(run_main(source), triet_logic::Trilean::True);
}

#[test]
fn bare_unit_enum_eq_different_returns_false() {
    let source = r"
public enum Color { Red, Green, Blue }
function main() -> Trilean {
    let a: Color = Red
    let b: Color = Green
    a == b
}
";
    assert_trilean(run_main(source), triet_logic::Trilean::False);
}

#[test]
fn enum_with_integer_payload_eq() {
    let source = r"
public enum Status { Ok(Integer), Err(Integer) }
function main() -> Trilean {
    let a: Status = Ok(42)
    let b: Status = Ok(42)
    let c: Status = Ok(99)
    a == b && a != c
}
";
    assert_trilean(run_main(source), triet_logic::Trilean::True);
}

#[test]
fn outcome_value_eq() {
    let source = r"
function main() -> Integer {
    let a: Integer~String = ~+ 42
    let b: Integer~String = ~+ 42
    if? a == b { 1 } else { 0 }
}
";
    assert_integer(run_main(source), 1);
}
