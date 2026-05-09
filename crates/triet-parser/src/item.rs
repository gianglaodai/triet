//! Top-level item parser — `fn`, `const`, `type`, `import`.

use triet_lexer::{Span, Token};
use triet_syntax::{
    EnumDef, EnumVariant, FunctionBody, FunctionDef, FunctionParam, ImportPath, Item,
    ParameterPassing, Spanned, StructDef, StructField, Visibility,
};

use crate::{
    error::ParseError,
    expr::parse_expression,
    parser::Parser,
    stmt::{parse_assignment_body, parse_top_block},
    type_expr::parse_type,
};

/// Parse a top-level item.
///
/// Items may be prefixed with a visibility modifier (`pub`, `pub(pkg)`).
/// The captured [`Visibility`] is stored on the resulting AST node;
/// downstream passes (name resolver, ABI extractor) read it from there.
pub(crate) fn parse_item(parser: &mut Parser<'_>) -> Result<Spanned<Item>, ParseError> {
    // Capture span start before optional `pub` prefix so the item's
    // overall span includes the visibility keyword.
    let head_span_start = parser
        .peek()
        .map_or_else(|| parser.eof_span().start, |(_, span)| span.start);

    let visibility = parse_visibility(parser)?;

    let Some((token, kw_span)) = parser.peek().cloned() else {
        let expected = if visibility == Visibility::Private {
            "item".to_owned()
        } else {
            format!("item after `{visibility}`")
        };
        return Err(ParseError::UnexpectedEof {
            expected,
            span: parser.eof_span(),
        });
    };

    let head_span = head_span_start..kw_span.end;

    match token {
        Token::Function => parse_function(parser, head_span, visibility),
        Token::Constant => parse_const_item(parser, head_span, visibility),
        Token::Type => parse_type_alias(parser, head_span, visibility),
        Token::Struct => parse_struct(parser, head_span, visibility),
        Token::Enum => parse_enum(parser, head_span, visibility),
        Token::Import => {
            if visibility != Visibility::Private {
                // `pub use` re-exports are a post-v0.2.x feature (ADR-0005).
                return Err(ParseError::UnexpectedToken {
                    expected: "`function`, `constant`, `type`, `struct`, or `enum` after `pub`"
                        .to_owned(),
                    found: "`import` (re-exports use `public use`, not yet implemented)"
                        .to_owned(),
                    span: head_span,
                });
            }
            parse_import(parser, kw_span)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "`function`, `constant`, `type`, `struct`, `enum`, `import`, or `pub`".to_owned(),
            found: format!("{other:?}"),
            span: kw_span,
        }),
    }
}

/// Parse an optional visibility prefix.
///
/// Recognized forms (per ADR-0005):
/// - (nothing) → `Visibility::Private`
/// - `public` → `Visibility::Public`
/// - `public(package)` → `Visibility::PublicPackage`
///
/// Anything else after `public(` is rejected. Triết deliberately omits
/// `public(super)` / `public(in path)` to keep the ABI surface model
/// simple.
fn parse_visibility(parser: &mut Parser<'_>) -> Result<Visibility, ParseError> {
    if !matches!(parser.peek_token(), Some(Token::Public)) {
        return Ok(Visibility::Private);
    }
    parser.advance(); // consume `public`

    if !matches!(parser.peek_token(), Some(Token::LParen)) {
        return Ok(Visibility::Public);
    }
    parser.advance(); // consume `(`

    let (token, span) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: "`package` after `public(`".to_owned(),
            span: parser.eof_span(),
        }
    })?;
    match token {
        Token::Identifier(ref name) if name == "package" => {
            parser.advance();
            parser.expect(&Token::RParen, "`)`")?;
            Ok(Visibility::PublicPackage)
        }
        other => Err(ParseError::UnexpectedToken {
            expected:
                "`package` (the only restriction allowed in `public(...)`)".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

fn parse_function(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Function, "`function`")?;

    let (name, _) = parse_item_name(parser, "function name")?;

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
            visibility,
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
    let passing = if parser.eat(&Token::Mutable) {
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
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Constant, "`constant`")?;
    let (name, _) = parse_item_name(parser, "constant name")?;

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
            visibility,
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
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Type, "`type`")?;
    let (name, _) = parse_item_name(parser, "type alias name")?;

    parser.expect(&Token::Assign, "`=`")?;
    let target = parse_type(parser)?;
    let _ = parser.eat(&Token::Semi);

    let end = parser.arena.type_expression(target).span.end;
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::TypeAlias {
            visibility,
            name,
            target,
        },
        span,
    ))
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

fn parse_struct(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Struct, "`struct`")?;

    let (name, _) = parse_item_name(parser, "struct name")?;
    let type_params = parse_generic_params(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let fields = parse_struct_fields(parser)?;
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Struct(StructDef {
            visibility,
            name,
            type_params,
            fields,
        }),
        span,
    ))
}

fn parse_struct_fields(
    parser: &mut Parser<'_>,
) -> Result<Vec<StructField>, ParseError> {
    let mut fields = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RBrace)) {
        return Ok(fields);
    }
    loop {
        let name = parse_ident(parser, "field name")?;
        parser.expect(&Token::Colon, "`:`")?;
        let type_annotation = parse_type(parser)?;
        fields.push(StructField { name, type_annotation });

        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RBrace)) {
            break;
        }
    }
    Ok(fields)
}

fn parse_enum(
    parser: &mut Parser<'_>,
    head_span: Span,
    visibility: Visibility,
) -> Result<Spanned<Item>, ParseError> {
    parser.expect(&Token::Enum, "`enum`")?;

    let (name, _) = parse_item_name(parser, "enum name")?;
    let type_params = parse_generic_params(parser)?;

    parser.expect(&Token::LBrace, "`{`")?;
    let variants = parse_enum_variants(parser)?;
    parser.expect(&Token::RBrace, "`}`")?;

    let end = parser.previous_token_end(head_span.end);
    let span = head_span.start..end;
    Ok(Spanned::new(
        Item::Enum(EnumDef {
            visibility,
            name,
            type_params,
            variants,
        }),
        span,
    ))
}

fn parse_enum_variants(
    parser: &mut Parser<'_>,
) -> Result<Vec<EnumVariant>, ParseError> {
    let mut variants = Vec::new();
    if matches!(parser.peek_token(), Some(Token::RBrace)) {
        return Ok(variants);
    }
    loop {
        let name = parse_ident(parser, "variant name")?;
        let payload = if parser.eat(&Token::LParen) {
            let ty = parse_type(parser)?;
            parser.expect(&Token::RParen, "`)`")?;
            Some(ty)
        } else {
            None
        };
        variants.push(EnumVariant { name, payload });

        if !parser.eat(&Token::Comma) {
            break;
        }
        if matches!(parser.peek_token(), Some(Token::RBrace)) {
            break;
        }
    }
    Ok(variants)
}

/// Parse optional generic type parameters: `<T, U>`. Returns an empty
/// vec if the next token is not `<`.
fn parse_generic_params(parser: &mut Parser<'_>) -> Result<Vec<String>, ParseError> {
    if !matches!(parser.peek_token(), Some(Token::Lt)) {
        return Ok(Vec::new());
    }
    parser.advance(); // consume `<`
    let mut params = Vec::new();
    loop {
        let name = parse_ident(parser, "type parameter")?;
        params.push(name);
        if !parser.eat(&Token::Comma) {
            break;
        }
    }
    parser.expect(&Token::Gt, "`>`")?;
    Ok(params)
}

fn parse_ident(parser: &mut Parser<'_>, expected: &str) -> Result<String, ParseError> {
    let (token, _) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: expected.to_owned(),
            span: parser.eof_span(),
        }
    })?;
    match token {
        Token::Identifier(name) => {
            parser.advance();
            Ok(name)
        }
        other => Err(ParseError::UnexpectedToken {
            expected: expected.to_owned(),
            found: format!("{other:?}"),
            span: parser.current_span(),
        }),
    }
}

/// Reserved namespace roots — top-level items cannot be defined with
/// these names. They're kept for the standard library and OS-native
/// namespaces (ADR-0005, trụ cột #5).
///
/// Path keywords (`crate`, `self`, `super`) are reserved at the lexer
/// level and surface as `UnexpectedToken` if used in identifier position.
const RESERVED_ITEM_NAMES: &[&str] = &["std", "sys", "dev", "usr", "core"];

/// Parse an identifier intended as a top-level item name (function,
/// const, type alias, struct, enum). Rejects reserved namespace roots
/// per ADR-0005.
///
/// Returns `(name, span_of_identifier)` so callers can re-use the span
/// for error reporting downstream.
fn parse_item_name(
    parser: &mut Parser<'_>,
    expected: &str,
) -> Result<(String, Span), ParseError> {
    let (token, span) = parser.peek().cloned().ok_or_else(|| {
        ParseError::UnexpectedEof {
            expected: expected.to_owned(),
            span: parser.eof_span(),
        }
    })?;
    let name = match token {
        Token::Identifier(name) => {
            parser.advance();
            name
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: expected.to_owned(),
                found: format!("{other:?}"),
                span,
            });
        }
    };
    if RESERVED_ITEM_NAMES.contains(&name.as_str()) {
        return Err(ParseError::ReservedItemName { name, span });
    }
    Ok((name, span))
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
        let (_, item) = parse("function main() { }");
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
        let (_, item) = parse("function double(n: Integer) -> Integer = n * 2");
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
        let (_, item) = parse("function add(a: Integer, b: Integer) -> Integer = a + b");
        match &item.node {
            Item::Function(def) => assert_eq!(def.parameters.len(), 2),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_mut_parameter() {
        let (_, item) = parse("function append(mutable buffer: String, suffix: String) { }");
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
        let (_, item) = parse("function consume(owned data: String) -> String = data");
        match &item.node {
            Item::Function(def) => {
                assert_eq!(def.parameters[0].passing, ParameterPassing::Owned);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_const_with_annotation() {
        let (_, item) = parse("constant PI: Integer = 3");
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
        let (_, item) = parse("constant ANSWER = 42");
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
    fn default_function_visibility_is_private() {
        let (_, item) = parse("function greet() { }");
        let Item::Function(def) = &item.node else { panic!("expected function") };
        assert_eq!(def.visibility, Visibility::Private);
    }

    #[test]
    fn pub_function_captures_public_visibility() {
        let (_, item) = parse("public function greet() { }");
        let Item::Function(def) = &item.node else { panic!("expected function") };
        assert_eq!(def.visibility, Visibility::Public);
        assert_eq!(def.name, "greet");
    }

    #[test]
    fn pub_pkg_function_captures_publicpkg_visibility() {
        let (_, item) = parse("public(package) function helper() { }");
        let Item::Function(def) = &item.node else { panic!("expected function") };
        assert_eq!(def.visibility, Visibility::PublicPackage);
    }

    #[test]
    fn pub_struct_captures_visibility() {
        let (_, item) = parse("public struct Point { x: Integer, y: Integer }");
        let Item::Struct(def) = &item.node else { panic!("expected struct") };
        assert_eq!(def.visibility, Visibility::Public);
        assert_eq!(def.name, "Point");
    }

    #[test]
    fn pub_enum_captures_visibility() {
        let (_, item) = parse("public enum Option<T> { Some(T), None }");
        let Item::Enum(def) = &item.node else { panic!("expected enum") };
        assert_eq!(def.visibility, Visibility::Public);
    }

    #[test]
    fn pub_const_captures_visibility() {
        let (_, item) = parse("public constant PI: Integer = 3");
        let Item::Const { visibility, name, .. } = &item.node else {
            panic!("expected constant")
        };
        assert_eq!(*visibility, Visibility::Public);
        assert_eq!(name, "PI");
    }

    #[test]
    fn pub_pkg_type_alias_captures_visibility() {
        let (_, item) = parse("public(package) type Username = String");
        let Item::TypeAlias { visibility, name, .. } = &item.node else {
            panic!("expected type alias")
        };
        assert_eq!(*visibility, Visibility::PublicPackage);
        assert_eq!(name, "Username");
    }

    #[test]
    fn pub_on_import_is_rejected() {
        // Re-exports are post-v0.2.x — `pub use` will land later.
        let result = try_parse("public import std.io");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn pub_with_invalid_restriction_is_rejected() {
        // Only `pub(pkg)` is accepted; `pub(crate)` / `pub(super)` are not.
        let result = try_parse("public(crate) function foo() { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn pub_at_eof_errors() {
        let result = try_parse("public");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn item_span_includes_pub_keyword() {
        // Span should start at `pub`, not at the inner keyword.
        let (_, item) = parse("public function greet() { }");
        // `pub` starts at byte 0, so the item span must too.
        assert_eq!(item.span.start, 0);
    }

    // === Reserved-name validation (ADR-0005, task v0.2.x.4) ===

    #[test]
    fn struct_named_std_is_rejected() {
        let result = try_parse("struct std { x: Integer }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "std"
        ));
    }

    #[test]
    fn fn_named_sys_is_rejected() {
        let result = try_parse("function sys() { }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "sys"
        ));
    }

    #[test]
    fn enum_named_dev_is_rejected() {
        let result = try_parse("enum dev { A, B }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "dev"
        ));
    }

    #[test]
    fn const_named_usr_is_rejected() {
        let result = try_parse("constant usr: Integer = 5");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "usr"
        ));
    }

    #[test]
    fn type_alias_named_core_is_rejected() {
        let result = try_parse("type core = Integer");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "core"
        ));
    }

    #[test]
    fn fn_named_crate_is_rejected_via_unexpected_token() {
        // `crate` lexes as Token::Crate, not Token::Identifier.
        let result = try_parse("function crate() { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn struct_named_self_is_rejected_via_unexpected_token() {
        let result = try_parse("struct self { }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn enum_named_super_is_rejected_via_unexpected_token() {
        let result = try_parse("enum super { A }");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn substring_of_reserved_name_is_allowed() {
        // `crater`, `stderr`, `system` start with a reserved prefix but
        // are themselves valid identifiers — keyword matching is not
        // greedy, and reserved-name comparison is exact.
        let (_, item) = parse("function crater() { }");
        let Item::Function(def) = &item.node else { panic!("expected function") };
        assert_eq!(def.name, "crater");

        let (_, item) = parse("struct Stderr { }");
        let Item::Struct(def) = &item.node else { panic!("expected struct") };
        assert_eq!(def.name, "Stderr");
    }

    #[test]
    fn pub_reserved_name_still_rejected() {
        // Visibility prefix doesn't bypass reservation.
        let result = try_parse("public struct std { }");
        assert!(matches!(
            result,
            Err(ParseError::ReservedItemName { name, .. }) if name == "std"
        ));
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
        let result = try_parse("function foo()");
        assert!(matches!(
            result,
            Err(ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn errors_on_function_missing_param_name() {
        let result = try_parse("function foo(: Integer) { }");
        assert!(result.is_err());
    }

    #[test]
    fn parses_empty_struct() {
        let (_, item) = parse("struct Empty { }");
        assert!(matches!(item.node, Item::Struct(_)));
        if let Item::Struct(def) = &item.node {
            assert_eq!(def.name, "Empty");
            assert!(def.fields.is_empty());
        }
    }

    #[test]
    fn parses_struct_with_fields() {
        let (_, item) = parse("struct Point { x: Integer, y: Integer }");
        assert!(matches!(item.node, Item::Struct(_)));
        if let Item::Struct(def) = &item.node {
            assert_eq!(def.name, "Point");
            assert_eq!(def.fields.len(), 2);
            assert_eq!(def.fields[0].name, "x");
            assert_eq!(def.fields[1].name, "y");
        }
    }

    #[test]
    fn parses_enum_with_unit_variants() {
        let (_, item) = parse("enum Color { Red, Green, Blue }");
        assert!(matches!(item.node, Item::Enum(_)));
        if let Item::Enum(def) = &item.node {
            assert_eq!(def.name, "Color");
            assert_eq!(def.variants.len(), 3);
            assert_eq!(def.variants[0].name, "Red");
            assert!(def.variants[0].payload.is_none());
        }
    }

    #[test]
    fn parses_enum_with_payload_variants() {
        let (_, item) = parse("enum Maybe { Some(Integer), None }");
        assert!(matches!(item.node, Item::Enum(_)));
        if let Item::Enum(def) = &item.node {
            assert_eq!(def.name, "Maybe");
            assert_eq!(def.variants.len(), 2);
            assert_eq!(def.variants[0].name, "Some");
            assert!(def.variants[0].payload.is_some());
            assert_eq!(def.variants[1].name, "None");
            assert!(def.variants[1].payload.is_none());
        }
    }
}
