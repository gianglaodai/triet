//! v0.7.4.3-debt.1 — end-to-end test for `Trilean!` annotation
//! parsing + typechecking per [ADR-0021] §2.7.
//!
//! Verifies the round-trip parser → typecheck for the four canonical
//! shapes: declared `-> Trilean!`, parameter `: Trilean!`, body that
//! produces a refined value, and the E1034 failure mode for bodies
//! producing generic `Trilean`.
//!
//! [ADR-0021]: ../../../../docs/decisions/0021-trilean-refinement.md

use miette::Diagnostic;
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Lex + parse + resolve + typecheck `src`, return the list of hard
/// errors (warnings filtered).
fn hard_errors(src: &str) -> Vec<String> {
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .map(|err| {
            err.code()
                .map_or_else(|| format!("{err}"), |code| code.to_string())
        })
        .collect()
}

#[test]
fn return_type_trilean_bang_accepts_refined_body() {
    let src = r"
        function is_positive(n: Integer) -> Trilean! = n > 0
        function main() {
            let r: Trilean = is_positive(5)
        }
    ";
    let errors = hard_errors(src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

#[test]
fn return_type_trilean_bang_rejects_generic_trilean_body_with_e1034() {
    let src = r"
        function maybe_unknown(t: Trilean) -> Trilean! = t
        function main() {}
    ";
    let errors = hard_errors(src);
    assert!(
        errors.iter().any(|e| e.contains("E1034")),
        "expected E1034, got: {errors:?}",
    );
}

#[test]
fn refined_trilean_param_accepted_by_plain_if() {
    let src = r"
        function gate(c: Trilean!) -> String = {
            if c { 'yes' } else { 'no' }
        }
        function main() {
            let s: String = gate(true)
        }
    "
    .replace('\'', "\"");
    let errors = hard_errors(&src);
    assert!(
        errors.is_empty(),
        "expected no errors, got: {errors:?} (refined Trilean must satisfy plain `if`)",
    );
}

#[test]
fn refined_trilean_widens_to_plain_trilean_at_call_site() {
    // Passing a `Trilean!` value where `Trilean` is expected must
    // typecheck (widening per ADR-0021 §4.1).
    let src = r"
        function consume_plain(t: Trilean) -> String = match t {
            true => 'yes',
            false => 'no',
            unknown => '?',
        }
        function main() {
            let r: String = consume_plain(5 > 0)
        }
    "
    .replace('\'', "\"");
    let errors = hard_errors(&src);
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

#[test]
fn integer_bang_in_type_position_is_a_parse_error() {
    // `Integer!` is NOT a valid type expression — the parser only
    // admits `!` after a bare `Trilean` atom. Parse failures surface
    // as E2105 from the modules layer.
    use triet_modules::load_program_from_source;
    let src = r"
        function bad(n: Integer!) -> Integer = n
        function main() {}
    ";
    let load_result = load_program_from_source(src);
    // Either load_program_from_source returned an error (parse-stage
    // refusal) or it returned Ok and the typecheck has hard errors.
    match load_result {
        Err(_) => { /* parse refused — expected */ }
        Ok(resolved) => {
            let diagnostics = check_resolved(&resolved);
            let any_hard = diagnostics
                .iter()
                .any(|err| err.severity() != Some(miette::Severity::Warning));
            assert!(
                any_hard,
                "expected parse / typecheck refusal for `Integer!`, got clean check",
            );
        }
    }
}
