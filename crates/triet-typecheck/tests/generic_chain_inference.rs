//! v0.7.4.3-debt.4 — integration tests for WA-7.
//!
//! Pre-fix: `push(new(), 99)` failed with "expected T, found Integer"
//! because `new()`'s un-inferrable return type `Vector<TypeParam("T"))`
//! poisoned `sub_map[T] = TypeParam("T")` via `or_insert_with`. The
//! subsequent `99` arg couldn't override the entry, so the substitution
//! `T → TypeParam("T")` left the second param expecting an unbound
//! `TypeParam`, which doesn't match the concrete `Integer`.
//!
//! Fix: `extract_type_params` now prefers concrete bindings over
//! `TypeParam` ones — same-name `TypeParam("T") → TypeParam("T")`
//! self-references give way when a concrete arg comes in.
//!
//! Tests pin both positive (chains now work) and negative
//! (mismatched generics still error) behavior.

use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

fn check_codes(src: &str) -> Vec<String> {
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    diagnostics.iter().map(|e| format!("{e}")).collect()
}

#[test]
fn push_new_inline_chain_resolves_t_from_second_arg() {
    // The canonical lexer-port pattern: a zero-arg generic call
    // (`new()`) feeds a multi-arg generic call (`push(_, x)`).
    // The second arg's concrete type must back-flow to T.
    let src = r"
        from std.collections.vector import new, push, length

        function build() -> Integer = {
            let v: Vector<Integer> = push(new(), 99)
            length(v)
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "expected clean check, got: {errors:#?}");
}

#[test]
fn push_then_push_inline_chain_resolves_t() {
    // Nested generic chain — two `push` calls layered, the outer
    // sees `Vector<Integer>` from the second arg of the inner.
    let src = r"
        from std.collections.vector import new, push, length

        function build_two() -> Integer = {
            let v: Vector<Integer> = push(push(new(), 1), 2)
            length(v)
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "expected clean check, got: {errors:#?}");
}

#[test]
fn user_pair_function_first_concrete_arg_still_wins() {
    // `pair<T>(a: T, b: T) -> T` with `(Integer, String)` must
    // still emit a Mismatch — the prefer-concrete rule only fires
    // when the EXISTING binding is a TypeParam; a concrete first
    // binding is preserved.
    let src = r#"
        function pair<T>(a: T, b: T) -> T = a

        function main() {
            let bad: Integer = pair(1, "hello")
        }
    "#;
    let errors = check_codes(src);
    assert!(
        errors.iter().any(|e| e.contains("type mismatch")),
        "expected Mismatch on second arg; got: {errors:?}",
    );
}

#[test]
fn user_pair_function_both_concrete_args_succeed() {
    let src = r"
        function pair<T>(a: T, b: T) -> T = a

        function main() {
            let x: Integer = pair(1, 2)
        }
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "expected clean check; got: {errors:?}");
}

#[test]
fn vector_new_alone_returns_unresolved_until_bound() {
    // `let v: Vector<Integer> = new()` should typecheck because the
    // let-binding's expected type pins T = Integer via .matches().
    let src = r"
        from std.collections.vector import new

        function main() {
            let v: Vector<Integer> = new()
        }
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "expected clean check; got: {errors:?}");
}
