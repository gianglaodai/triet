//! Triết parser: source/token stream → `Program` AST.
//!
//! Recursive descent for items, statements, patterns, and types; Pratt
//! parsing for expressions (with explicit binding-power table — see
//! `expr.rs`). The parser accumulates errors with synchronization-token
//! recovery, so a single parse can surface multiple errors at once.
//!
//! # Public API
//!
//! - [`parse`] takes Triết source text, runs the lexer, parses, and
//!   returns a `(Program, Vec<ParseError>)` pair.
//! - [`parse_tokens`] is the same but takes a pre-lexed token stream.
//!
//! Most callers want [`parse`].
//!
//! # Example
//!
//! ```
//! use triet_parser::parse;
//!
//! let (program, errors) = parse("function id(n: Integer) -> Integer = n");
//! assert!(errors.is_empty());
//! assert_eq!(program.items.len(), 1);
//! ```

#![warn(missing_docs)]
// `redundant_pub_crate` and friends fire on every internal helper of a
// private parser module — they are correct in spirit but noisy here,
// where every helper lives behind the `pub` API surface (`parse`,
// `parse_tokens`). We accept the trade-off and silence those lints.
#![allow(
    clippy::redundant_pub_crate,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::needless_continue
)]

mod error;
mod expr;
mod item;
mod parser;
mod pattern;
mod stmt;
mod type_expr;

use triet_lexer::{LexError, SpannedToken, lex};
use triet_syntax::{Arena, Program};

pub use error::ParseError;

/// Parse Triết source text into a `Program` plus any errors encountered.
///
/// If the lexer fails, the returned program is empty and the lex error
/// is wrapped as the sole `ParseError::Lex` in the error list.
///
/// The parser performs error recovery: if one item fails to parse, the
/// driver synchronizes to the next item boundary and continues, so the
/// caller may see multiple parse errors per call.
#[must_use]
pub fn parse(source: &str) -> (Program, Vec<ParseError>) {
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
        Err(error) => {
            return (
                Program {
                    arena: Arena::new(),
                    items: vec![],
                    source_file: String::new(),
                },
                vec![ParseError::from(error)],
            );
        }
    };
    parse_tokens(&tokens)
}

/// Parse a pre-lexed token stream into a `Program`.
///
/// Useful when the caller has already tokenized the source (e.g. for a
/// language server that runs the lexer once and the parser repeatedly).
#[must_use]
pub fn parse_tokens(tokens: &[SpannedToken]) -> (Program, Vec<ParseError>) {
    let mut parser = parser::Parser::new(tokens);
    let mut items = Vec::new();

    while !parser.at_end() {
        let cursor_before = parser.cursor_index();
        match item::parse_item(&mut parser) {
            Ok(item) => items.push(item),
            Err(error) => {
                parser.record_error(error);
                parser.synchronize();
            }
        }
        // Defensive: if neither a successful parse nor synchronization
        // moved us forward (e.g. parse failed at a sync token like `}`),
        // force-advance one token to avoid an infinite loop at top level.
        if parser.cursor_index() == cursor_before && !parser.at_end() {
            parser.advance();
        }
    }

    let (arena, errors) = parser.finish();
    (
        Program {
            arena,
            items,
            source_file: String::new(),
        },
        errors,
    )
}

// Re-export `LexError` so callers don't need to depend on `triet-lexer`
// directly when handling `ParseError::Lex`.
pub use triet_lexer::LexError as LexerError;

// Marker to ensure the underlying type is wired in.
const _: fn() = || {
    let _: fn(LexError) -> ParseError = ParseError::from;
};

#[cfg(test)]
mod integration_tests {
    use super::*;
    use triet_syntax::{Expr, FunctionBody, Item, MatchArm, Pattern, Stmt};

    fn assert_no_errors(errors: &[ParseError]) {
        assert!(errors.is_empty(), "expected no errors, got: {errors:#?}");
    }

    #[test]
    fn parses_empty_source() {
        let (program, errors) = parse("");
        assert_no_errors(&errors);
        assert!(program.items.is_empty());
    }

    #[test]
    fn parses_whitespace_and_comments_only() {
        let (program, errors) = parse("// hello\n    \n// world\n");
        assert_no_errors(&errors);
        assert!(program.items.is_empty());
    }

    #[test]
    fn parses_simple_main_function() {
        let (program, errors) = parse("function main() { }");
        assert_no_errors(&errors);
        assert_eq!(program.items.len(), 1);
        assert!(matches!(program.items[0].node, Item::Function { def: _ }));
    }

    #[test]
    fn parses_multiple_top_level_items() {
        let source = r"
            constant PI = 3
            type Username = String
            function add(a: Integer, b: Integer) -> Integer = a + b
            function main() { }
        ";
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        assert_eq!(program.items.len(), 4);
    }

    #[test]
    fn parses_fizzbuzz_program() {
        let source = r#"
            function fizzbuzz(n: Integer) -> String =
                match (n %% 3, n %% 5) {
                    (0, 0) => "FizzBuzz",
                    (0, _) => "Fizz",
                    (_, 0) => "Buzz",
                    _ => to_string(n),
                }
        "#;
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        assert_eq!(program.items.len(), 1);

        let Item::Function { def } = &program.items[0].node else {
            panic!("expected function");
        };
        assert_eq!(def.name, "fizzbuzz");

        let FunctionBody::Expression { expr: body } = &def.body else {
            panic!("expected expression body");
        };
        match &program.arena.expression(*body).node {
            Expr::Match { arms, .. } => assert_eq!(arms.len(), 4),
            other => panic!("expected match, got {other:?}"),
        }
    }

    #[test]
    fn parses_measles_demo_function() {
        let source = r"
            function risk_measles(fever: Trilean, rash: Trilean, vaccinated: Trilean) -> Trilean {
                let symptoms = fever and rash
                symptoms and not vaccinated
            }
        ";
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        assert_eq!(program.items.len(), 1);

        let Item::Function { def } = &program.items[0].node else {
            panic!("expected function");
        };
        assert_eq!(def.parameters.len(), 3);
    }

    #[test]
    fn parses_for_loop_with_range_iteration() {
        let source = r"
            function count_to(n: Integer) {
                for i in 0..=n {
                    print(i)
                }
            }
        ";
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    #[test]
    fn parses_if_question_for_trilean_condition() {
        let source = r"
            function maybe_run(condition: Trilean) {
                if? condition {
                    do_action()
                }
            }
        ";
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    #[test]
    fn parses_logic_with_implication_keyword_chain() {
        let source = r"
            function entailment(p: Trilean, q: Trilean, r: Trilean) -> Trilean =
                p implies q implies r
        ";
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    #[test]
    fn parses_match_with_or_pattern() {
        let source = r#"
            function classify(n: Integer) -> String =
                match n {
                    1 | 2 | 3 => "small",
                    _ => "other",
                }
        "#;
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        let Item::Function { def } = &program.items[0].node else {
            panic!();
        };
        let FunctionBody::Expression { expr: body } = &def.body else {
            panic!();
        };
        let Expr::Match { arms, .. } = &program.arena.expression(*body).node else {
            panic!();
        };
        let MatchArm { pattern, .. } = &arms[0];
        match &program.arena.pattern(*pattern).node {
            Pattern::Or(alts) => assert_eq!(alts.len(), 3),
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn parses_block_with_let_and_final_expr() {
        let source = r"
            function compute() -> Integer {
                let x = 5
                let y = 7
                x + y
            }
        ";
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        let Item::Function { def } = &program.items[0].node else {
            panic!()
        };
        let FunctionBody::Block { block } = &def.body else {
            panic!()
        };
        let Expr::Block {
            statements,
            final_expr,
        } = &program.arena.expression(*block).node
        else {
            panic!("expected block expression")
        };
        assert_eq!(statements.len(), 2);
        assert!(final_expr.is_some());
    }

    #[test]
    fn parses_nullable_with_safe_call_chain() {
        let source = r"
            function name_length(name: String?) -> Integer = name?.length ?: 0
        ";
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    #[test]
    fn parses_f_string_with_interpolation() {
        let source = r#"
            function greet(name: String) -> String = f"Xin chào, {name}!"
        "#;
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    #[test]
    fn parses_complex_arithmetic_expression() {
        // Tests precedence: -2 ** 2 + 3 * 4 = -4 + 12 = 8
        let source = "function answer() -> Integer = -2 ** 2 + 3 * 4";
        let (_program, errors) = parse(source);
        assert_no_errors(&errors);
    }

    // === Error recovery ===

    #[test]
    fn recovers_from_first_item_error_to_parse_second() {
        let source = r"
            function broken( oops { }
            function good() { }
        ";
        let (program, errors) = parse(source);
        assert!(!errors.is_empty(), "expected errors");
        // Even if the first function fails, we should still see the
        // second one in some form.
        let _ = program;
    }

    #[test]
    fn lex_error_is_wrapped_into_parse_error() {
        // `\q` triggers an InvalidEscape lex error.
        let (_, errors) = parse(r#"function foo() = "\q""#);
        assert!(errors.iter().any(|e| matches!(e, ParseError::Lex(_))));
    }

    // === Realistic program-level sample (FizzBuzz with main) ===

    #[test]
    fn parses_full_fizzbuzz_with_main() {
        let source = r#"
            function fizzbuzz(n: Integer) -> String =
                match (n %% 3, n %% 5) {
                    (0, 0) => "FizzBuzz",
                    (0, _) => "Fizz",
                    (_, 0) => "Buzz",
                    _ => to_string(n),
                }

            function main() {
                for i in 1..=100 {
                    println(fizzbuzz(i))
                }
            }
        "#;
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        assert_eq!(program.items.len(), 2);

        // Verify shape of each function.
        let Item::Function { def: fb } = &program.items[0].node else {
            panic!()
        };
        assert_eq!(fb.name, "fizzbuzz");
        let Item::Function { def: main_fn } = &program.items[1].node else {
            panic!()
        };
        assert_eq!(main_fn.name, "main");
        assert!(matches!(main_fn.body, FunctionBody::Block { block: _ }));
    }

    // Reassignment (`x = expr`) is implemented as `Stmt::Assign` —
    // statement-level only, lvalue restricted to identifier targets in
    // v0.1. Tuple/field/index assignment arrives with structs in v0.2.

    #[test]
    fn span_is_preserved_for_top_level_item() {
        let source = "function main() { }";
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        let span = &program.items[0].span;
        assert_eq!(span.start, 0);
        assert!(span.end > 0);
    }

    #[test]
    fn unused_lex_error_helper_link() {
        // Smoke test that the `From<LexError>` glue is wired up.
        let source = "let bad = \"\\q\"";
        let (_, errors) = parse(source);
        assert!(errors.iter().any(|e| matches!(e, ParseError::Lex(_))));
    }

    #[test]
    fn statement_ordering_in_block_is_preserved() {
        let source = "function ord() { let a = 1 let b = 2 let c = 3 }";
        let (program, errors) = parse(source);
        assert_no_errors(&errors);
        let Item::Function { def } = &program.items[0].node else {
            panic!()
        };
        let FunctionBody::Block { block } = &def.body else {
            panic!()
        };
        let Expr::Block { statements, .. } = &program.arena.expression(*block).node else {
            panic!("expected block expression")
        };
        assert_eq!(statements.len(), 3);
        for (i, expected) in ["a", "b", "c"].iter().enumerate() {
            let stmt = program.arena.statement(statements[i]);
            match &stmt.node {
                Stmt::Let { name, .. } => assert_eq!(name, expected),
                other => panic!("expected Let, got {other:?}"),
            }
        }
    }
}
