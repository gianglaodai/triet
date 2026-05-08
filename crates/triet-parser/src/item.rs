//! Top-level item parser — `fn`, `const`, `type`, `import`.

use triet_lexer::{Span, Token};
use triet_syntax::{
    FunctionBody, FunctionDef, FunctionParam, ImportPath, Item, ParameterPassing, Spanned,
};

use crate::{
    error::ParseError,
    expr::parse_expression,
    parser::Parser,
    stmt::{parse_assignment_body, parse_top_block},
    type_expr::parse_type,
};

/// Parse a top-level item.
pub(crate) fn parse_item(parser: &mut Parser<'_>) -> Result<Spanned<Item>, ParseError> {
    let Some((token, span)) = parser.peek().cloned() else {
        return Err(ParseError::UnexpectedEof {
            expected: "item".to_owned(),
            span: parser.eof_span(),
        });
    };

    match token {
        Token::Fn => parse_function(parser, span),
        Token::Const => parse_const_item(parser, span),
        Token::Type => parse_type_alias(parser, span),
        Token::Import => parse_import(parser, span),
        Token::Pub => {
            // v0.1: accept `pub` as a no-op modifier prefix.
            parser.advance();
            parse_item(parser)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "`fn`, `const`, `type`, or `import`".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

fn parse_function(
    parser: &mut Parser<'_>,
    head_span: Span,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Fn, "`fn`")?;

    let (name_token, _) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "function name".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    let name = match name_token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "function name".to_owned(),
                found: format!("{other:?}"),
                span: parser.current_span(),
            });
        }
    };

    parser.expect(&Token::LParen, "`(`")?;
    let parameters = parse_parameter_list(parser)?;
    parser.expect(&Token::RParen, "`)`")?;

    let return_type = if parser.eat(&Token::ThinArrow) {
        Some(parse_type(parser)?)
    } else {
        None
    };

    // Body — either `{ ... }` or `= expr`.
    let body = match parser.peek_token() {
        Some(Token::LBrace) => FunctionBody::Block(parse_top_block(parser)?),
        Some(Token::Assign) => FunctionBody::Expression(parse_assignment_body(parser)?),
        _ => {
            return Err(ParseError::UnexpectedToken {
                expected: "function body (`{...}` or `= expr`)".to_owned(),
                found: format!("{:?}", parser.peek_token()),
                span: parser.current_span(),
            });
        }
    };

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Function(FunctionDef {
            name,
            parameters,
            return_type,
            body,
        }),
        span,
    ))
}

fn parse_parameter_list(parser: &mut Parser<'_>) -> Result<Vec<FunctionParam>, ParseError> {
    let mut params = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RParen)) {
        return Ok(params);
    }
    loop {
        params.push(parse_parameter(parser)?);
        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RParen)) {
            break;
        }
    }
    Ok(params)
}

fn parse_parameter(parser: &mut Parser<'_>) -> Result<FunctionParam, ParseError> {
    // Optional passing mode prefix: `mut` or `owned`.
    let passing = if parser.eat(&Token::Mut) {
        ParameterPassing::Mutable
    } else if parser.eat(&Token::Owned) {
        ParameterPassing::Owned
    } else {
        ParameterPassing::Borrowed
    };

    let (name_token, _) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "parameter name".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    let name = match name_token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "parameter name".to_owned(),
                found: format!("{other:?}"),
                span: parser.current_span(),
            });
        }
    };

    parser.expect(&Token::Colon, "`:`")?;
    let type_annotation = parse_type(parser)?;

    Ok(FunctionParam {
        name,
        type_annotation,
        passing,
    })
}

fn parse_const_item(
    parser: &mut Parser<'_>,
    head_span: Span,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Const, "`const`")?;
    let (name_token, _) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "const name".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    let name = match name_token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "const name".to_owned(),
                found: format!("{other:?}"),
                span: parser.current_span(),
            });
        }
    };

    let type_annotation = if parser.eat(&Token::Colon) {
        Some(parse_type(parser)?)
    } else {
        None
    };

    parser.expect(&Token::Assign, "`=`")?;
    let value = parse_expression(parser)?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.arena.expression(value).span.end;
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Const {
            name,
            type_annotation,
            value,
        },
        span,
    ))
}

fn parse_type_alias(
    parser: &mut Parser<'_>,
    head_span: Span,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Type, "`type`")?;
    let (name_token, _) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "type alias name".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    let name = match name_token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "type alias name".to_owned(),
                found: format!("{other:?}"),
                span: parser.current_span(),
            });
        }
    };

    parser.expect(&Token::Assign, "`=`")?;
    let target = parse_type(parser)?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.arena.type_expression(target).span.end;
    let span = head_span.start..end;
    Ok(Spanned::new(Item::TypeAlias { name, target }, span))
}

fn parse_import(parser: &mut Parser<'_>, head_span: Span) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Import, "`import`")?;
    let mut segments = Vec::new();

    let (first_token, first_span) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "import path".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    match first_token {
        Token::Identifier(name) => {
            parser.advance();
            segments.push(name);
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "import path identifier".to_owned(),
                found: format!("{other:?}"),
                span: first_span,
            });
        }
    }

    while parser.eat(&Token::Dot) {
        let (token, span) = parser.peek().cloned().ok_or_else(|| {
            ParseError::UnexpectedEof {
                expected: "identifier after `.`".to_owned(),
                span: parser.eof_span(),
            }
        })?;
        match token {
            Token::Identifier(name) => {
                parser.advance();
                segments.push(name);
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    expected: "identifier after `.`".to_owned(),
                    found: format!("{other:?}"),
                    span,
                });
            }
        }
    }
    let _ = parser.eat(&Token::Semi);

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(Item::Import(ImportPath { segments }), span))
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parse(source: &str) -> (Parser<'static>, Spanned<Item>) {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let item = parse_item(&mut parser).expect("parse failed");
        (parser, item)
    }

    fn try_parse(source: &str) -> Result<(Parser<'static>, Spanned<Item>), ParseError> {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let item = parse_item(&mut parser)?;
        Ok((parser, item))
    }

    #[test]
    fn parses_no_arg_function_with_block_body() {
        let (_, item) = parse("fn main() { }");
        match &item.node {
            Item::Function(def) => {
                assert_eq!(def.name, "main");
                assert!(def.parameters.is_empty());
                assert!(matches!(def.body, FunctionBody::Block(_)));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_expression_body() {
        let (_, item) = parse("fn double(n: Integer) -> Integer = n * 2");
        match &item.node {
            Item::Function(def) => {
                assert_eq!(def.parameters.len(), 1);
                assert!(def.return_type.is_some());
                assert!(matches!(def.body, FunctionBody::Expression(_)));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_multiple_params() {
        let (_, item) = parse("fn add(a: Integer, b: Integer) -> Integer = a + b");
        match &item.node {
            Item::Function(def) => assert_eq!(def.parameters.len(), 2),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_mut_parameter() {
        let (_, item) = parse("fn append(mut buffer: String, suffix: String) { }");
        match &item.node {
            Item::Function(def) => {
                assert_eq!(def.parameters[0].passing, ParameterPassing::Mutable);
                assert_eq!(def.parameters[1].passing, ParameterPassing::Borrowed);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_owned_parameter() {
        let (_, item) = parse("fn consume(owned data: String) -> String = data");
        match &item.node {
            Item::Function(def) => {
                assert_eq!(def.parameters[0].passing, ParameterPassing::Owned);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_const_with_annotation() {
        let (_, item) = parse("const PI: Integer = 3");
        match &item.node {
            Item::Const { name, type_annotation, .. } => {
                assert_eq!(name, "PI");
                assert!(type_annotation.is_some());
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_const_without_annotation() {
        let (_, item) = parse("const ANSWER = 42");
        match &item.node {
            Item::Const { name, .. } => assert_eq!(name, "ANSWER"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_type_alias() {
        let (_, item) = parse("type Username = String");
        match &item.node {
            Item::TypeAlias { name, .. } => assert_eq!(name, "Username"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_type_alias_with_generic_target() {
        let (_, item) = parse("type Lookup = Map<String, Integer>");
        match &item.node {
            Item::TypeAlias { name, .. } => assert_eq!(name, "Lookup"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_simple_import() {
        let (_, item) = parse("import std");
        match &item.node {
            Item::Import(path) => assert_eq!(path.segments, vec!["std"]),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_dotted_import() {
        let (_, item) = parse("import std.io.println");
        match &item.node {
            Item::Import(path) => assert_eq!(path.segments.len(), 3),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn pub_is_consumed_as_modifier() {
        let (_, item) = parse("pub fn greet() { }");
        assert!(matches!(item.node, Item::Function(_)));
    }

    #[test]
    fn errors_on_unrecognized_item_keyword() {
        let result = try_parse("nonsense foo");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn errors_on_eof_at_item_start() {
        let result = try_parse("");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn errors_on_function_missing_body() {
        let result = try_parse("fn foo()");
        assert!(matches!(
            result,
            Err(ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn errors_on_function_missing_param_name() {
        let result = try_parse("fn foo(: Integer) { }");
        assert!(result.is_err());
    }
}
