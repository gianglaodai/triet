//! Integration tests for `LowerError` diagnostic codes (ADR-0086,
//! `triet::lower::E11XX`).
//!
//! Mirrors `crates/triet-typecheck/tests/diagnostics_format.rs`: asserts
//! every one of the 8 taxonomy codes renders its exact `miette::Diagnostic`
//! code string. Where a real fixture already exercises the site
//! (`triet-driver/tests/fixtures/`), the test runs the actual
//! parse → typecheck → lower pipeline against that fixture's source instead
//! of hand-building the variant, proving the code fires for a real program —
//! not just that the variant's `#[diagnostic]` attribute is well-formed.

use miette::Diagnostic;
use triet_lower::LowerError;

/// Run one `.tri` source string through parse → typecheck → `lower_program`
/// and return the lowerer's `Err`, panicking if any earlier phase fails or
/// lowering unexpectedly succeeds.
fn lower_err(source: &str) -> LowerError {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect_err("expected lower_program to fail")
}

#[test]
fn e1100_construct_not_yet_lowered_code() {
    let err = LowerError::ConstructNotYetLowered {
        message: "lowerer does not yet support this expression: <dummy>".to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1100");
}

/// Real trigger: fixture 414 declares `struct S { e: E?, tail: Integer }`
/// where `E` has an aggregate (payload-bearing) variant — the declaration-
/// site chokepoint in `lower_program` refuses this before codegen can ever
/// silently overflow the 8-byte slot reserved for `e` (see the fixture's own
/// comment for the pre-refuse miscompile it pins shut).
#[test]
fn e1120_nullable_enum_payload_unsupported_code_via_fixture_414() {
    let source = include_str!(
        "../../triet-driver/tests/fixtures/414_nullable_enum_payload_field_rejected.tri"
    );
    let err = lower_err(source);
    assert!(
        matches!(err, LowerError::NullableEnumPayloadUnsupported { .. }),
        "expected NullableEnumPayloadUnsupported, got: {err:?}"
    );
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1120");
}

/// Real trigger: fixture 440 returns `H?` where `H` has a heap-bearing field
/// (`String`) — refused per ADR-0065 §4 (B8), the tag-prepend sret buffer
/// carries no drop-glue.
#[test]
fn e1121_nullable_struct_return_heap_field_code_via_fixture_440() {
    let source = include_str!(
        "../../triet-driver/tests/fixtures/440_struct_nullable_return_heap_field_refused.tri"
    );
    let err = lower_err(source);
    assert!(
        matches!(err, LowerError::NullableStructReturnHeapField { .. }),
        "expected NullableStructReturnHeapField, got: {err:?}"
    );
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1121");
}

#[test]
fn e1122_escaping_closure_sealed_code() {
    let err = LowerError::EscapingClosureSealed {
        message: "general escaping closure sealed (YAGNI per ADR-0039 recon — \
                  nullable/Outcome ops use inline nodes, no first-class closure consumer)"
            .to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1122");
}

#[test]
fn e1140_undefined_local_code() {
    let err = LowerError::UndefinedLocal {
        message: "undefined local variable: x".to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1140");
}

#[test]
fn e1141_null_literal_without_expected_type_code() {
    let err = LowerError::NullLiteralWithoutExpectedType {
        message: "Outcome/nullable constructor (`~+`/`~0`/`~-`) requires an expected \
                  type from context"
            .to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1141");
}

#[test]
fn e1142_literal_out_of_range_code() {
    let err = LowerError::LiteralOutOfRange {
        message: "Trit literal value 99 out of range".to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1142");
}

#[test]
fn e1190_internal_invariant_code() {
    let err = LowerError::InternalInvariant {
        message: "internal: return-borrow elision expects exactly 1 ref-param".to_string(),
        span: 0..0,
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::lower::E1190");
}
