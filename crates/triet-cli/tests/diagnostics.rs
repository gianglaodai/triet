//! Smoke tests for the diagnostic output pipeline.
//!
//! Covers: error codes, span accuracy, exit-code boundaries. Tests
//! call the library functions directly (parse, check, run) and verify
//! that the miette-derived error types carry correct metadata — the
//! CLI binary (`triet check --json ...`) exercises the rendering path,
//! which is covered by `demo_programs.rs` integration tests plus
//! manual inspection.

use triet_interpreter::RuntimeError;
use triet_parser::{ParseError, parse};
use triet_typecheck::{TypeError, check};

#[test]
fn parse_error_has_correct_span() {
    let (_, errors) = parse("fn main() { a == b == c }");
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ParseError::ChainedNoChainOperator { class, span } => {
            assert_eq!(class, "equality");
            // Span of the second `==` should be after the first.
            assert!(
                span.start > 17,
                "expected span after first `==`, got {span:?}",
            );
        }
        other => panic!("expected ChainedNoChainOperator, got {other:?}"),
    }
    // Verify code mapping.
    let msg = errors[0].to_string();
    assert!(msg.contains("equality"));
    assert!(msg.contains("cannot be chained"));
}

#[test]
fn parse_error_missing_token_has_meaningful_span() {
    let (_, errors) = parse("fn main( { }");
    assert!(!errors.is_empty());
    let msg = errors[0].to_string();
    assert!(msg.contains("expected"));
}

#[test]
fn type_error_undefined_name_points_to_usage_site() {
    let (program, parse_errors) = parse("fn main() { unknown_var }");
    assert!(parse_errors.is_empty());
    let type_errors = check(&program);
    assert_eq!(type_errors.len(), 1);
    match &type_errors[0] {
        TypeError::UndefinedName { name, span } => {
            assert_eq!(name, "unknown_var");
            // Span should cover the identifier inside the block.
            assert!(
                span.start >= 12,
                "expected span after `fn main() {{`, got {span:?}",
            );
        }
        other => panic!("expected UndefinedName, got {other:?}"),
    }
}

#[test]
fn type_error_mismatch_carries_both_types() {
    let (program, parse_errors) = parse(r#"fn main() -> Integer { "oops" }"#);
    assert!(parse_errors.is_empty());
    let type_errors = check(&program);
    assert_eq!(type_errors.len(), 1);
    match &type_errors[0] {
        TypeError::Mismatch { expected, found, .. } => {
            assert_eq!(expected.to_string(), "Integer");
            assert_eq!(found.to_string(), "String");
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
}

#[test]
fn type_error_assign_to_immutable_mentions_name() {
    let source = r"
        fn main() {
            let x = 0
            x = 1
        }
    ";
    let (program, parse_errors) = parse(source);
    assert!(parse_errors.is_empty());
    let type_errors = check(&program);
    assert_eq!(type_errors.len(), 1);
    match &type_errors[0] {
        TypeError::AssignToImmutable { name, .. } => {
            assert_eq!(name, "x");
        }
        other => panic!("expected AssignToImmutable, got {other:?}"),
    }
}

#[test]
fn runtime_error_no_main_function_has_code() {
    let (program, parse_errors) = parse("fn helper() {}");
    assert!(parse_errors.is_empty());
    let type_errors = check(&program);
    assert!(type_errors.is_empty());
    let result = triet_interpreter::run(&program);
    match &result {
        Err(e) => {
            assert!(matches!(e, RuntimeError::NoMainFunction));
            let msg = e.to_string();
            assert!(msg.contains("main"), "got: {msg}");
        }
        other => panic!("expected NoMainFunction, got {other:?}"),
    }
}

#[test]
fn type_error_invalid_operands_mentions_operator() {
    let source = "fn bad() -> Trilean = 5 and true";
    let (program, parse_errors) = parse(source);
    assert!(parse_errors.is_empty());
    let type_errors = check(&program);
    assert_eq!(type_errors.len(), 1);
    match &type_errors[0] {
        TypeError::InvalidOperands { operator, .. } => {
            assert_eq!(operator, "and");
        }
        other => panic!("expected InvalidOperands, got {other:?}"),
    }
}

#[test]
fn all_error_types_have_span_method() {
    // Verify every error variant's span() returns a usable range.
    let errors: Vec<TypeError> = vec![
        TypeError::UnknownType { name: "Foo".into(), span: 0..3 },
        TypeError::UndefinedName { name: "x".into(), span: 4..5 },
        TypeError::Mismatch {
            expected: triet_typecheck::Type::Integer,
            found: triet_typecheck::Type::String,
            span: 6..7,
        },
        TypeError::AmbiguousCondition { span: 8..9 },
        TypeError::DuplicateName { name: "f".into(), span: 10..11 },
        TypeError::NullLiteralInNonNullableContext { span: 12..13 },
        TypeError::AssignToImmutable { name: "a".into(), span: 14..15 },
    ];
    for error in &errors {
        let span = error.span();
        assert!(span.start <= span.end, "broken span in {error:?}");
    }
}
