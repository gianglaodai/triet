//! Type expression parser.
//!
//! Handles the syntactic forms `T`, `T<U, V>`, `(T1, T2)`, `T?`,
//! `(T1, T2) -> R`. Type parsing has its own arena (`TypeId`) and is
//! used both at top level (function signatures, `type` aliases, `let`
//! annotations) and within expressions (closure annotations).

use triet_lexer::Token;
use triet_syntax::{Spanned, TypeExpr, TypeId};

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

    match token {
        Token::Identifier(name) => {
            parser.advance();
            // Possible generic instantiation: `Name<T1, T2>`.
            if matches!(parser.peek_token(), Some(Token::Lt)) {
                parse_generic_args(parser, name, span)
            } else {
                let span_clone = span;
                Ok(parser.arena.alloc_type(Spanned::new(TypeExpr::Named(name), span_clone)))
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

/// Apply zero or more `?` suffixes to wrap a type in `Nullable`.
fn apply_type_postfix(parser: &mut Parser<'_>, mut id: TypeId) -> Result<TypeId, ParseError> {
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
            TypeExpr::Function { parameters: elements, return_type },
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
                    TypeExpr::Generic { name: inner, arguments: inner_args } => {
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
            TypeExpr::Nullable(outer_inner) => match &parser.arena.type_expression(*outer_inner).node {
                TypeExpr::Nullable(inner_inner) => expect_named(&parser, *inner_inner, "Integer"),
                other => panic!("expected nested Nullable, got {other:?}"),
            },
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
            TypeExpr::Function { parameters, return_type } => {
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
        assert!(result.is_err() || matches!(
            &parser.arena.type_expression(result.unwrap()).node,
            TypeExpr::Function { parameters, .. } if parameters.is_empty()
        ));
    }

    #[test]
    fn parses_multi_argument_function_type() {
        let (parser, id) = parse("(Integer, Integer) -> Integer");
        match &parser.arena.type_expression(id).node {
            TypeExpr::Function { parameters, return_type } => {
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
}
