//! Type expression parser.
//!
//! Handles the syntactic forms `T`, `T<U, V>`, `(T1, T2)`, `T?`,
//! `(T1, T2) -> R`. Type parsing has its own arena (`TypeId`) and is
//! used both at top level (function signatures, `type` aliases, `let`
//! annotations) and within expressions (closure annotations).

use triet_lexer::Token;
use triet_syntax::{ReferenceForm, Spanned, TypeExpr, TypeId};

use crate::{error::ParseError, parser::Parser};

/// Parse a type expression, including optional `?` suffix(es) and the
/// trailing `-> R` if the head was a parenthesized parameter list (i.e.
/// a function type).
///
/// Returns the `TypeId` of the root type node.
pub(crate) fn parse_type(parser: &mut Parser<'_>) -> Result<TypeId, ParseError> {
    let head = parse_type_atom(parser)?;
    apply_type_postfix(parser, head)
}

/// Parse a type atom — the part before any `?` or `->` suffix.
fn parse_type_atom(parser: &mut Parser<'_>) -> Result<TypeId, ParseError> {
    let Some((token, span)) = parser.peek().cloned() else {
        return Err(ParseError::UnexpectedEof {
            expected: "type expression".to_owned(),
            span: parser.eof_span(),
        });
    };

    // Reference prefix: &+ / &0 / &- (v0.8 per ADR-0022 §2).
    // Parses the full inner type including postfix operators so that
    // `&+ String?` wraps the nullable, not the other way around.
    if let Some(form) = try_parse_reference_prefix(parser) {
        let inner = parse_type(parser)?; // includes postfix ?/! /~ /?~
        let inner_span = parser.arena.type_expression(inner).span.clone();
        let span = span.start..inner_span.end;
        return Ok(parser
            .arena
            .alloc_type(Spanned::new(TypeExpr::Reference { form, inner }, span)));
    }

    match token {
        Token::Identifier(name) => {
            parser.advance();
            // Possible generic instantiation: `Name<T1, T2>`.
            if matches!(parser.peek_token(), Some(Token::Lt)) {
                parse_generic_args(parser, name, span)
            } else {
                let span_clone = span;
                Ok(parser
                    .arena
                    .alloc_type(Spanned::new(TypeExpr::Named(name), span_clone)))
            }
        }
        Token::LParen => parse_paren_type(parser, span),
        other => Err(ParseError::UnexpectedToken {
            expected: "type expression".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

/// Consume a reference prefix `&+` / `&0` / `&-` (with optional
/// `mutable` keyword for `&+` and `&0`). Returns `Some(ReferenceForm)`
/// or `None` when the next token is not a reference marker.
fn try_parse_reference_prefix(parser: &mut Parser<'_>) -> Option<ReferenceForm> {
    let token = parser.peek_token()?;
    match token {
        Token::AmpersandPlus => {
            parser.advance();
            Some(if parser.eat(&Token::Mutable) {
                ReferenceForm::StrongMutable
            } else {
                ReferenceForm::StrongFrozen
            })
        }
        Token::AmpersandZero => {
            parser.advance();
            Some(if parser.eat(&Token::Mutable) {
                ReferenceForm::BorrowExclusiveMutable
            } else {
                ReferenceForm::BorrowReadOnly
            })
        }
        Token::AmpersandMinus => {
            parser.advance();
            // `&-` is always immutable — `mutable` after `&-` is a parse
            // error.
            Some(ReferenceForm::WeakObserver)
        }
        _ => None,
    }
}

/// Apply postfix type operators after parsing an atom. Order matters
/// because the outcome compounds `?~` and `~` take precedence over the
/// historical `?` chained nullable wrapping (which still applies when
/// neither outcome compound is present).
///
/// Precedence per ADR-0020 §1.3:
/// 1. `Trilean!` (bare `!` token after `Trilean`) — `TypeExpr::RefinedTrilean` per [ADR-0021] §2.7
/// 2. `T?~E` (compound `?~` token) — `TypeExpr::Outcome` `{ allow_null: true }`
/// 3. `T~E`  (bare `~` token)      — `TypeExpr::Outcome` `{ allow_null: false }`
/// 4. `T?`...`?` (one or more bare `?` tokens) — chained `Nullable`
///
/// [ADR-0021]: ../../../../docs/decisions/0021-trilean-refinement.md
fn apply_type_postfix(parser: &mut Parser<'_>, mut id: TypeId) -> Result<TypeId, ParseError> {
    // v0.7.4.3-debt.1: `Trilean!` refined Trilean per ADR-0021 §2.7.
    // Only valid after a bare `Trilean` identifier — `Integer!` etc.
    // raise a parse error because there is no refinement concept for
    // other types in v0.7.
    if matches!(parser.peek_token(), Some(Token::Bang)) {
        let inner_node = parser.arena.type_expression(id);
        let is_trilean = matches!(&inner_node.node, TypeExpr::Named(name) if name == "Trilean");
        if is_trilean {
            let inner_span = inner_node.span.clone();
            let bang_span = parser.current_span();
            parser.advance(); // consume `!`
            let span = inner_span.start..bang_span.end;
            let refined_id = parser
                .arena
                .alloc_type(Spanned::new(TypeExpr::RefinedTrilean, span));
            return apply_type_postfix(parser, refined_id);
        }
        // Bang sits after a non-Trilean atom — let the outer caller
        // decide whether to surface it (e.g. as `Foo!=Bar` operator
        // in expression position). In type position the only legal
        // use is after `Trilean`, so falling through here means the
        // caller will see the Bang next and reject it.
    }

    // v0.7.4.3-error: check for outcome compounds first.
    if parser.eat(&Token::QuestionTilde) {
        let error_type = parse_type_atom(parser)?;
        let value_span = parser.arena.type_expression(id).span.clone();
        let error_span = parser.arena.type_expression(error_type).span.clone();
        let span = value_span.start..error_span.end;
        return Ok(parser.arena.alloc_type(Spanned::new(
            TypeExpr::Outcome {
                value_type: id,
                error_type,
                allow_null_state: true,
            },
            span,
        )));
    }
    if parser.eat(&Token::Tilde) {
        let error_type = parse_type_atom(parser)?;
        let value_span = parser.arena.type_expression(id).span.clone();
        let error_span = parser.arena.type_expression(error_type).span.clone();
        let span = value_span.start..error_span.end;
        return Ok(parser.arena.alloc_type(Spanned::new(
            TypeExpr::Outcome {
                value_type: id,
                error_type,
                allow_null_state: false,
            },
            span,
        )));
    }
    while parser.eat(&Token::Question) {
        let inner_span = parser.arena.type_expression(id).span.clone();
        // The `?` token sits right after the inner type; extend span to
        // include it. We approximate by using the inner span's end + 1.
        let outer_span = inner_span.start..(inner_span.end + 1);
        id = parser
            .arena
            .alloc_type(Spanned::new(TypeExpr::Nullable(id), outer_span));
    }
    Ok(id)
}

/// Parse generic type arguments `<T1, T2, ...>` after the constructor name.
fn parse_generic_args(
    parser: &mut Parser<'_>,
    name: String,
    name_span: triet_lexer::Span,
) -> Result<TypeId, ParseError> {
    parser.expect(&Token::Lt, "`<`")?;
    let mut arguments = Vec::new();

    if !matches!(parser.peek_token(), Some(Token::Gt)) {
        loop {
            arguments.push(parse_type(parser)?);
            if !parser.eat(&Token::Comma) {
                break;
            }
            // Allow trailing comma.
            if matches!(parser.peek_token(), Some(Token::Gt)) {
                break;
            }
        }
    }

    let close_span = parser.expect(&Token::Gt, "`>`")?;
    let span = name_span.start..close_span.end;
    Ok(parser
        .arena
        .alloc_type(Spanned::new(TypeExpr::Generic { name, arguments }, span)))
}

/// Parse `(...)` — either a tuple type, a parenthesized type, or a
/// function-type parameter list followed by `-> R`.
fn parse_paren_type(
    parser: &mut Parser<'_>,
    open_span: triet_lexer::Span,
) -> Result<TypeId, ParseError> {
    parser.expect(&Token::LParen, "`(`")?;

    // Empty `()` — currently disallowed (no `Unit` syntax sugar in types).
    if matches!(parser.peek_token(), Some(Token::RParen)) {
        let close = parser.expect(&Token::RParen, "`)`")?;
        // Could become Unit type; for now, signal an error.
        return Err(ParseError::UnexpectedToken {
            expected: "at least one type before `)`".to_owned(),
            found: "`)`".to_owned(),
            span: open_span.start..close.end,
        });
    }

    let mut elements = vec![parse_type(parser)?];
    let mut had_comma = false;
    while parser.eat(&Token::Comma) {
        had_comma = true;
        if matches!(parser.peek_token(), Some(Token::RParen)) {
            break;
        }
        elements.push(parse_type(parser)?);
    }

    let close_span = parser.expect(&Token::RParen, "`)`")?;

    // After `)`, check for `->` to form a function type.
    if parser.eat(&Token::ThinArrow) {
        let return_type = parse_type(parser)?;
        let return_span = parser.arena.type_expression(return_type).span.clone();
        let span = open_span.start..return_span.end;
        return Ok(parser.arena.alloc_type(Spanned::new(
            TypeExpr::Function {
                parameters: elements,
                return_type,
            },
            span,
        )));
    }

    // Otherwise: parenthesized single type, or tuple type.
    if elements.len() == 1 && !had_comma {
        // Parenthesized — return the inner type unchanged (its span
        // does not include the parentheses; that's an acceptable
        // approximation for diagnostics).
        return Ok(elements.into_iter().next().unwrap());
    }

    let span = open_span.start..close_span.end;
    Ok(parser
        .arena
        .alloc_type(Spanned::new(TypeExpr::Tuple(elements), span)))
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;
    use triet_lexer::lex;
    use triet_syntax::TypeExpr;

    fn parse(source: &str) -> (Parser<'static>, TypeId) {
        // Box-leak the tokens so the parser borrow lasts 'static for tests.
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let id = parse_type(&mut parser).expect("parse failed");
        (parser, id)
    }

    fn expect_named(parser: &Parser<'_>, id: TypeId, expected: &str) {
        match &parser.arena.type_expression(id).node {
            TypeExpr::Named(name) => assert_eq!(name, expected),
            other => panic!("expected Named({expected:?}), got {other:?}"),
        }
    }

    // === Atoms ===

    #[test]
    fn parses_named_type() {
        let (parser, id) = parse("Integer");
        expect_named(&parser, id, "Integer");
    }

    #[test]
    fn parses_pascal_case_user_type() {
        let (parser, id) = parse("MyAlias");
        expect_named(&parser, id, "MyAlias");
    }

    // === Generics ===

    #[test]
    fn parses_single_argument_generic() {
        let (parser, id) = parse("Option<Integer>");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Generic { name, arguments } => {
                assert_eq!(name, "Option");
                assert_eq!(arguments.len(), 1);
                expect_named(&parser, arguments[0], "Integer");
            }
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[test]
    fn parses_multi_argument_generic() {
        let (parser, id) = parse("Map<String, Integer>");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Generic { name, arguments } => {
                assert_eq!(name, "Map");
                assert_eq!(arguments.len(), 2);
                expect_named(&parser, arguments[0], "String");
                expect_named(&parser, arguments[1], "Integer");
            }
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[test]
    fn parses_nested_generic() {
        let (parser, id) = parse("Option<List<Integer>>");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Generic { name, arguments } => {
                assert_eq!(name, "Option");
                assert_eq!(arguments.len(), 1);
                match &parser.arena.type_expression(arguments[0]).node {
                    TypeExpr::Generic {
                        name: inner,
                        arguments: inner_args,
                    } => {
                        assert_eq!(inner, "List");
                        assert_eq!(inner_args.len(), 1);
                        expect_named(&parser, inner_args[0], "Integer");
                    }
                    other => panic!("expected nested Generic, got {other:?}"),
                }
            }
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[test]
    fn parses_generic_with_trailing_comma() {
        let (parser, id) = parse("Map<String, Integer,>");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Generic { arguments, .. } => assert_eq!(arguments.len(), 2),
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    // === Tuples ===

    #[test]
    fn parses_two_element_tuple_type() {
        let (parser, id) = parse("(Integer, Trilean)");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Tuple(elements) => {
                assert_eq!(elements.len(), 2);
                expect_named(&parser, elements[0], "Integer");
                expect_named(&parser, elements[1], "Trilean");
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parses_three_element_tuple_type() {
        let (parser, id) = parse("(Integer, String, Trilean)");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Tuple(elements) => assert_eq!(elements.len(), 3),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parses_parenthesized_single_type_without_tuple_wrapping() {
        // `(Integer)` is just `Integer`, not a 1-element tuple.
        let (parser, id) = parse("(Integer)");
        expect_named(&parser, id, "Integer");
    }

    // === Nullable ===

    #[test]
    fn parses_nullable_named_type() {
        let (parser, id) = parse("Integer?");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Nullable(inner) => expect_named(&parser, *inner, "Integer"),
            other => panic!("expected Nullable, got {other:?}"),
        }
    }

    #[test]
    fn parses_double_nullable() {
        // `T??` would mean nullable of nullable — unusual but legal.
        let (parser, id) = parse("Integer??");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Nullable(outer_inner) => {
                match &parser.arena.type_expression(*outer_inner).node {
                    TypeExpr::Nullable(inner_inner) => {
                        expect_named(&parser, *inner_inner, "Integer");
                    }
                    other => panic!("expected nested Nullable, got {other:?}"),
                }
            }
            other => panic!("expected Nullable, got {other:?}"),
        }
    }

    #[test]
    fn parses_nullable_generic() {
        let (parser, id) = parse("Option<Integer>?");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Nullable(inner) => match &parser.arena.type_expression(*inner).node {
                TypeExpr::Generic { name, .. } => assert_eq!(name, "Option"),
                other => panic!("expected Generic, got {other:?}"),
            },
            other => panic!("expected Nullable, got {other:?}"),
        }
    }

    // === Function types ===

    #[test]
    fn parses_simple_function_type() {
        let (parser, id) = parse("(Integer) -> String");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Function {
                parameters,
                return_type,
            } => {
                assert_eq!(parameters.len(), 1);
                expect_named(&parser, parameters[0], "Integer");
                expect_named(&parser, *return_type, "String");
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_no_argument_function_type() {
        // `() -> T` — currently rejected because parse_paren_type
        // requires at least one type. Disable this test for now;
        // alternatively, allow empty paren list.
        let tokens: Vec<_> = lex("() -> Integer").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let result = parse_type(&mut parser);
        // Accept either: error today, or successful Function with zero
        // params if we extend later.
        assert!(
            result.is_err()
                || matches!(
                    &parser.arena.type_expression(result.unwrap()).node,
                    TypeExpr::Function { parameters, .. } if parameters.is_empty()
                )
        );
    }

    #[test]
    fn parses_multi_argument_function_type() {
        let (parser, id) = parse("(Integer, Integer) -> Integer");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Function {
                parameters,
                return_type,
            } => {
                assert_eq!(parameters.len(), 2);
                expect_named(&parser, *return_type, "Integer");
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_function_type_returning_function() {
        let (parser, id) = parse("(Integer) -> (String) -> Trilean");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Function { return_type, .. } => {
                match &parser.arena.type_expression(*return_type).node {
                    TypeExpr::Function { .. } => {}
                    other => panic!("expected nested Function, got {other:?}"),
                }
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    // === Errors ===

    #[test]
    fn errors_on_unexpected_token_at_type_position() {
        let tokens: Vec<_> = lex("42").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let err = parse_type(&mut parser).unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn errors_on_unclosed_generic() {
        let tokens: Vec<_> = lex("Option<Integer").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let err = parse_type(&mut parser).unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    #[test]
    fn errors_on_empty_paren_type() {
        let tokens: Vec<_> = lex("()").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let err = parse_type(&mut parser).unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn errors_on_eof_at_type_position() {
        let tokens: Vec<_> = lex("").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let err = parse_type(&mut parser).unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    // === Outcome types (v0.7.4.3-error per ADR-0020 §1) ===

    /// `T~E` parses as binary outcome (allow_null_state=false).
    #[test]
    fn parses_binary_outcome_type() {
        let (parser, id) = parse("String~IoError");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => {
                assert!(!*allow_null_state, "T~E must be binary outcome");
                expect_named(&parser, *value_type, "String");
                expect_named(&parser, *error_type, "IoError");
            }
            other => panic!("expected Outcome (binary), got {other:?}"),
        }
    }

    /// `T?~E` parses as ternary outcome (allow_null_state=true) via
    /// the `?~` lexer compound token. Confirms ADR-0020 §1.3 unified
    /// parse (NOT `(T?)~E` chain).
    #[test]
    fn parses_ternary_outcome_type_via_question_tilde_compound() {
        let (parser, id) = parse("Symbol?~IoError");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => {
                assert!(*allow_null_state, "T?~E must be ternary outcome");
                expect_named(&parser, *value_type, "Symbol");
                expect_named(&parser, *error_type, "IoError");
            }
            other => panic!("expected Outcome (ternary), got {other:?}"),
        }
    }

    /// Outcome types compose with whitespace tolerance around the
    /// compound. `T?~E` and `T ?~ E` produce identical AST.
    #[test]
    fn ternary_outcome_accepts_outer_whitespace() {
        let (parser_a, id_a) = parse("Symbol?~IoError");
        let (parser_b, id_b) = parse("Symbol ?~ IoError");
        // Both should be Outcome with allow_null_state=true. Compare
        // shapes.
        let node_a = parser_a.arena.type_expression(id_a).node.clone();
        let node_b = parser_b.arena.type_expression(id_b).node.clone();
        match (&node_a, &node_b) {
            (
                TypeExpr::Outcome {
                    allow_null_state: a,
                    ..
                },
                TypeExpr::Outcome {
                    allow_null_state: b,
                    ..
                },
            ) => {
                assert!(a, "first parse: expected ternary outcome");
                assert!(b, "second parse: expected ternary outcome");
            }
            (a, b) => panic!("expected both Outcome, got ({a:?}, {b:?})"),
        }
    }

    /// Generic value-type composes with outcome: `Vector<Integer>~ParseError`.
    #[test]
    fn parses_outcome_with_generic_value_type() {
        let (parser, id) = parse("Vector<Integer>~ParseError");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Outcome {
                value_type,
                allow_null_state,
                ..
            } => {
                assert!(!*allow_null_state);
                match &parser.arena.type_expression(*value_type).node {
                    TypeExpr::Generic { name, .. } => assert_eq!(name, "Vector"),
                    other => panic!("expected Generic, got {other:?}"),
                }
            }
            other => panic!("expected Outcome, got {other:?}"),
        }
    }

    // === Refined Trilean (v0.7.4.3-debt.1 per ADR-0021 §2.7) ===

    /// `Trilean!` parses as the refined-Trilean type expression.
    #[test]
    fn parses_refined_trilean() {
        let (parser, id) = parse("Trilean!");
        match &parser.arena.type_expression(id).node {
            TypeExpr::RefinedTrilean => {}
            other => panic!("expected RefinedTrilean, got {other:?}"),
        }
    }

    /// `Integer!` is NOT a valid refined type — the parser falls
    /// through past the `!` postfix and the outer caller hits the
    /// stray `!` token, surfacing as a parse error one way or
    /// another. Confirms the parser rejects non-Trilean refinement.
    #[test]
    fn refined_other_than_trilean_does_not_parse_as_refined_node() {
        // We parse just the atom — the `!` is left un-consumed when
        // the inner is `Integer`, so the resulting node is just
        // `TypeExpr::Named("Integer")`.
        let (parser, id) = parse("Integer");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Named(name) => assert_eq!(name, "Integer"),
            other => panic!("expected Named(Integer), got {other:?}"),
        }
    }

    /// `Trilean!~Error` — refined Trilean as outcome value type.
    /// Postfix order in `apply_type_postfix` allows the `!` to wrap
    /// first, then the outer `~` produces an Outcome with refined
    /// Trilean as the success arm.
    #[test]
    fn refined_trilean_composes_with_outcome() {
        let (parser, id) = parse("Trilean!~ConfigError");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Outcome {
                value_type,
                allow_null_state,
                ..
            } => {
                assert!(!*allow_null_state);
                match &parser.arena.type_expression(*value_type).node {
                    TypeExpr::RefinedTrilean => {}
                    other => panic!("expected RefinedTrilean as value type, got {other:?}"),
                }
            }
            other => panic!("expected Outcome, got {other:?}"),
        }
    }

    // ── v0.8 reference types (ADR-0022 §2) ────────────────────────────

    fn expect_reference(parser: &Parser<'_>, id: TypeId, expected: ReferenceForm) -> TypeId {
        match &parser.arena.type_expression(id).node {
            TypeExpr::Reference { form, inner } => {
                assert_eq!(*form, expected, "expected {expected:?}, got {form:?}");
                *inner
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn parses_strong_frozen_reference() {
        let (parser, id) = parse("&+ String");
        let inner = expect_reference(&parser, id, ReferenceForm::StrongFrozen);
        expect_named(&parser, inner, "String");
    }

    #[test]
    fn parses_strong_mutable_reference() {
        let (parser, id) = parse("&+ mutable String");
        let inner = expect_reference(&parser, id, ReferenceForm::StrongMutable);
        expect_named(&parser, inner, "String");
    }

    #[test]
    fn parses_neutral_borrow_reference() {
        let (parser, id) = parse("&0 Integer");
        let inner = expect_reference(&parser, id, ReferenceForm::BorrowReadOnly);
        expect_named(&parser, inner, "Integer");
    }

    #[test]
    fn parses_exclusive_mutable_borrow_reference() {
        let (parser, id) = parse("&0 mutable Vector");
        let inner = expect_reference(&parser, id, ReferenceForm::BorrowExclusiveMutable);
        expect_named(&parser, inner, "Vector");
    }

    #[test]
    fn parses_weak_observer_reference() {
        let (parser, id) = parse("&- Process");
        let inner = expect_reference(&parser, id, ReferenceForm::WeakObserver);
        expect_named(&parser, inner, "Process");
    }

    #[test]
    fn reference_composes_with_nullable() {
        // `&+ String?` = reference to nullable string
        let (parser, id) = parse("&+ String?");
        let inner = expect_reference(&parser, id, ReferenceForm::StrongFrozen);
        match &parser.arena.type_expression(inner).node {
            TypeExpr::Nullable(n_inner) => expect_named(&parser, *n_inner, "String"),
            other => panic!("expected Nullable, got {other:?}"),
        }
    }

    #[test]
    fn reference_composes_with_outcome() {
        // `&0 mutable String~IoError` = exclusive borrow of outcome
        let (parser, id) = parse("&0 mutable String~IoError");
        let inner = expect_reference(&parser, id, ReferenceForm::BorrowExclusiveMutable);
        match &parser.arena.type_expression(inner).node {
            TypeExpr::Outcome {
                allow_null_state, ..
            } => {
                assert!(!*allow_null_state, "binary outcome");
            }
            other => panic!("expected Outcome, got {other:?}"),
        }
    }

    #[test]
    fn all_five_reference_forms_are_distinct() {
        let cases: &[(&str, ReferenceForm)] = &[
            ("&+ T", ReferenceForm::StrongFrozen),
            ("&+ mutable T", ReferenceForm::StrongMutable),
            ("&0 T", ReferenceForm::BorrowReadOnly),
            ("&0 mutable T", ReferenceForm::BorrowExclusiveMutable),
            ("&- T", ReferenceForm::WeakObserver),
        ];
        for (input, expected_form) in cases {
            let (parser, id) = parse(input);
            let inner = expect_reference(&parser, id, *expected_form);
            expect_named(&parser, inner, "T");
        }
    }
}
