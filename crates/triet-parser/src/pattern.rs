//! Pattern parser — `match` arms, `let` destructuring, and `for` loop
//! variables.

use triet_lexer::{IntLiteral as LexIntLiteral, Token};
use triet_syntax::{
    LiteralPattern, NumericSuffix as AstSuffix, OutcomeArm, Pattern, PatternId, Spanned,
    TrileanValue,
};

use crate::{error::ParseError, parser::Parser};

/// Parse a pattern.
///
/// Or-patterns (`a | b | c`) are parsed at this top level; sub-patterns
/// (e.g. inside tuple destructuring) call `parse_pattern_no_or` to keep
/// `|` from binding across structural boundaries.
pub(crate) fn parse_pattern(parser: &mut Parser<'_>) -> Result<PatternId, ParseError> {
    let first = parse_pattern_no_or(parser)?;

    if !matches!(parser.peek_token(), Some(Token::Pipe)) {
        return Ok(first);
    }

    let mut alternatives = vec![first];
    while parser.eat(&Token::Pipe) {
        alternatives.push(parse_pattern_no_or(parser)?);
    }

    let start_span = parser.arena.pattern(alternatives[0]).span.clone();
    let end_span = parser
        .arena
        .pattern(*alternatives.last().expect("checked non-empty"))
        .span
        .clone();
    let span = start_span.start..end_span.end;
    Ok(parser
        .arena
        .alloc_pattern(Spanned::new(Pattern::Or(alternatives), span)))
}

/// Parse a pattern that does not consume top-level `|` (or-pattern).
fn parse_pattern_no_or(parser: &mut Parser<'_>) -> Result<PatternId, ParseError> {
    let Some((token, span)) = parser.peek().cloned() else {
        return Err(ParseError::UnexpectedEof {
            expected: "pattern".to_owned(),
            span: parser.eof_span(),
        });
    };

    match token {
        Token::Underscore => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_pattern(Spanned::new(Pattern::Wildcard, span)))
        }
        Token::Null => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_pattern(Spanned::new(Pattern::Null, span)))
        }
        Token::Identifier(name) => {
            parser.advance();
            // ADR-0071 Lát 2: `Enum::Variant` / `Enum::Variant(subpat)` is the
            // qualified enum-variant pattern (fills `name: Some`). A bare
            // identifier with no `::` and no payload is a plain binding
            // (`Pattern::Variable`) — NO implicit unit-variant guess (the
            // typecheck guess-hack is gone).
            if parser.eat(&Token::ColonColon) {
                return parse_colon_variant_pattern(parser, name, span.start);
            }
            // `Variant(subpattern)` — unqualified payload variant. The variant
            // name resolves against the scrutinee enum at typecheck; this is
            // pattern matching against the match's known type, not a scan.
            if matches!(parser.peek_token(), Some(Token::LParen)) {
                parser.advance(); // consume `(`
                let payload = parse_pattern(parser)?;
                parser.expect(&Token::RParen, "`)`")?;
                let end = parser.previous_token_end(span.end);
                let span = span.start..end;
                Ok(parser.arena.alloc_pattern(Spanned::new(
                    Pattern::EnumVariant {
                        name: None,
                        variant_name: name,
                        payload: Some(payload),
                    },
                    span,
                )))
            } else {
                Ok(parser
                    .arena
                    .alloc_pattern(Spanned::new(Pattern::Variable(name), span)))
            }
        }
        Token::LParen => parse_tuple_pattern(parser, span),
        Token::True | Token::False | Token::Unknown => parse_trilean_pattern(parser, &token, span),
        Token::IntegerLiteral(_) | Token::TernaryLiteral(_) | Token::StringLiteral(_) => {
            parse_literal_or_range_pattern(parser)
        }
        Token::Minus => {
            // Negative integer literal in pattern: `-5`, `-9_841_tryte`.
            parse_negative_literal_pattern(parser)
        }
        // Outcome arm patterns (v0.7.4.3-error per ADR-0020 §5):
        // `~+ binding` (Positive), `~- binding` (Negative), `~0` (Zero).
        // Style guide mandates space; lexer is whitespace-insensitive
        // between compound and following identifier.
        Token::TildePlus => parse_outcome_arm_pattern(parser, OutcomeArm::Positive, span),
        Token::TildeMinus => parse_outcome_arm_pattern(parser, OutcomeArm::Negative, span),
        Token::TildeZero => {
            parser.advance();
            Ok(parser.arena.alloc_pattern(Spanned::new(
                Pattern::OutcomeArm {
                    arm: OutcomeArm::Zero,
                    payload: None,
                },
                span,
            )))
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "pattern".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

/// Parse the tail of a qualified enum-variant pattern after `Enum::`
/// (ADR-0071 Lát 2): the variant name and an optional single `(subpattern)`
/// → `Pattern::EnumVariant { name: Some(enum), .. }`. `enum_name`/`start`
/// carry the path head already consumed by the caller.
fn parse_colon_variant_pattern(
    parser: &mut Parser<'_>,
    enum_name: String,
    start: usize,
) -> Result<PatternId, ParseError> {
    let (vtok, vspan) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "enum variant name after `::`".to_owned(),
            span: parser.eof_span(),
        })?;
    let Token::Identifier(variant_name) = vtok else {
        return Err(ParseError::UnexpectedToken {
            expected: "enum variant name after `::`".to_owned(),
            found: format!("{vtok:?}"),
            span: vspan,
        });
    };
    parser.advance();
    let payload = if matches!(parser.peek_token(), Some(Token::LParen)) {
        parser.advance(); // consume `(`
        let sub = parse_pattern(parser)?;
        parser.expect(&Token::RParen, "`)`")?;
        Some(sub)
    } else {
        None
    };
    let end = parser.previous_token_end(vspan.end);
    let span = start..end;
    Ok(parser.arena.alloc_pattern(Spanned::new(
        Pattern::EnumVariant {
            name: Some(enum_name),
            variant_name,
            payload,
        },
        span,
    )))
}

fn parse_tuple_pattern(
    parser: &mut Parser<'_>,
    open_span: triet_lexer::Span,
) -> Result<PatternId, ParseError> {
    parser.expect(&Token::LParen, "`(`")?;

    if matches!(parser.peek_token(), Some(Token::RParen)) {
        let close = parser.expect(&Token::RParen, "`)`")?;
        let span = open_span.start..close.end;
        return Ok(parser
            .arena
            .alloc_pattern(Spanned::new(Pattern::Tuple(Vec::new()), span)));
    }

    let mut elements = vec![parse_pattern(parser)?];
    let mut had_comma = false;
    while parser.eat(&Token::Comma) {
        had_comma = true;
        if matches!(parser.peek_token(), Some(Token::RParen)) {
            break;
        }
        elements.push(parse_pattern(parser)?);
    }

    let close_span = parser.expect(&Token::RParen, "`)`")?;

    // Single-element parens without trailing comma are just grouping
    // (return inner pattern).
    if elements.len() == 1 && !had_comma {
        return Ok(elements.into_iter().next().unwrap());
    }

    let span = open_span.start..close_span.end;
    Ok(parser
        .arena
        .alloc_pattern(Spanned::new(Pattern::Tuple(elements), span)))
}

fn parse_trilean_pattern(
    parser: &mut Parser<'_>,
    token: &Token,
    span: triet_lexer::Span,
) -> Result<PatternId, ParseError> {
    let value = match token {
        Token::True => TrileanValue::True,
        Token::False => TrileanValue::False,
        Token::Unknown => TrileanValue::Unknown,
        _ => unreachable!("caller filtered"),
    };
    parser.advance();
    let pat = Pattern::Literal(LiteralPattern::Trilean(value));
    Ok(parser.arena.alloc_pattern(Spanned::new(pat, span)))
}

/// Parse an integer/ternary/string literal pattern, optionally followed
/// by `..` or `..=` to form a range pattern.
fn parse_literal_or_range_pattern(parser: &mut Parser<'_>) -> Result<PatternId, ParseError> {
    let (start_lit, start_span) = parse_literal_pattern_payload(parser)?;

    if matches!(parser.peek_token(), Some(Token::DotDot | Token::DotDotEq)) {
        let inclusive = matches!(parser.peek_token(), Some(Token::DotDotEq));
        parser.advance();
        let (end_lit, end_span) = parse_literal_pattern_payload(parser)?;
        let span = start_span.start..end_span.end;
        return Ok(parser.arena.alloc_pattern(Spanned::new(
            Pattern::Range {
                start: start_lit,
                end: end_lit,
                inclusive,
            },
            span,
        )));
    }

    Ok(parser
        .arena
        .alloc_pattern(Spanned::new(Pattern::Literal(start_lit), start_span)))
}

fn parse_negative_literal_pattern(parser: &mut Parser<'_>) -> Result<PatternId, ParseError> {
    let minus = parser.expect(&Token::Minus, "`-`")?;
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "integer literal after `-`".to_owned(),
            span: parser.eof_span(),
        })?;

    let lit = match token {
        Token::IntegerLiteral(LexIntLiteral { value, suffix }) => {
            parser.advance();
            LiteralPattern::Integer {
                value: -value,
                suffix: suffix.map(convert_suffix),
            }
        }
        Token::TernaryLiteral(LexIntLiteral { value, .. }) => {
            parser.advance();
            LiteralPattern::Ternary(-value)
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "integer literal after `-`".to_owned(),
                found: format!("{other:?}"),
                span,
            });
        }
    };

    let full_span = minus.start..span.end;

    if matches!(parser.peek_token(), Some(Token::DotDot | Token::DotDotEq)) {
        let inclusive = matches!(parser.peek_token(), Some(Token::DotDotEq));
        parser.advance();
        let (end_lit, end_span) = parse_literal_pattern_payload(parser)?;
        let span = full_span.start..end_span.end;
        return Ok(parser.arena.alloc_pattern(Spanned::new(
            Pattern::Range {
                start: lit,
                end: end_lit,
                inclusive,
            },
            span,
        )));
    }

    Ok(parser
        .arena
        .alloc_pattern(Spanned::new(Pattern::Literal(lit), full_span)))
}

/// Pull a literal value (no leading sign) and its span.
fn parse_literal_pattern_payload(
    parser: &mut Parser<'_>,
) -> Result<(LiteralPattern, triet_lexer::Span), ParseError> {
    if matches!(parser.peek_token(), Some(Token::Minus)) {
        let minus = parser.expect(&Token::Minus, "`-`")?;
        let (token, span) = parser
            .peek()
            .cloned()
            .ok_or_else(|| ParseError::UnexpectedEof {
                expected: "literal after `-`".to_owned(),
                span: parser.eof_span(),
            })?;
        return match token {
            Token::IntegerLiteral(LexIntLiteral { value, suffix }) => {
                parser.advance();
                Ok((
                    LiteralPattern::Integer {
                        value: -value,
                        suffix: suffix.map(convert_suffix),
                    },
                    minus.start..span.end,
                ))
            }
            Token::TernaryLiteral(LexIntLiteral { value, .. }) => {
                parser.advance();
                Ok((LiteralPattern::Ternary(-value), minus.start..span.end))
            }
            other => Err(ParseError::UnexpectedToken {
                expected: "integer literal".to_owned(),
                found: format!("{other:?}"),
                span,
            }),
        };
    }

    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "literal".to_owned(),
            span: parser.eof_span(),
        })?;

    match token {
        Token::IntegerLiteral(LexIntLiteral { value, suffix }) => {
            parser.advance();
            Ok((
                LiteralPattern::Integer {
                    value,
                    suffix: suffix.map(convert_suffix),
                },
                span,
            ))
        }
        Token::TernaryLiteral(LexIntLiteral { value, .. }) => {
            parser.advance();
            Ok((LiteralPattern::Ternary(value), span))
        }
        Token::StringLiteral(text) => {
            parser.advance();
            Ok((LiteralPattern::String(text), span))
        }
        Token::True => {
            parser.advance();
            Ok((LiteralPattern::Trilean(TrileanValue::True), span))
        }
        Token::False => {
            parser.advance();
            Ok((LiteralPattern::Trilean(TrileanValue::False), span))
        }
        Token::Unknown => {
            parser.advance();
            Ok((LiteralPattern::Trilean(TrileanValue::Unknown), span))
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "literal".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

/// Parse `~+ binding` or `~- binding` outcome arm pattern. Consumes
/// the compound prefix token and then parses a sub-pattern (variable
/// name, wildcard `_`, or literal/nested pattern) per ADR-0020 §5.2.
fn parse_outcome_arm_pattern(
    parser: &mut Parser<'_>,
    arm: OutcomeArm,
    op_span: triet_lexer::Span,
) -> Result<PatternId, ParseError> {
    parser.advance(); // consume TildePlus or TildeMinus
    let payload = parse_pattern_no_or(parser)?;
    let span = op_span.start..parser.arena.pattern(payload).span.end;
    Ok(parser.arena.alloc_pattern(Spanned::new(
        Pattern::OutcomeArm {
            arm,
            payload: Some(payload),
        },
        span,
    )))
}

/// Convert a lexer-side numeric suffix to the AST-side enum (they have
/// the same variants but live in different crates by design).
const fn convert_suffix(suffix: triet_lexer::NumericSuffix) -> AstSuffix {
    match suffix {
        triet_lexer::NumericSuffix::Trit => AstSuffix::Trit,
        triet_lexer::NumericSuffix::Tryte => AstSuffix::Tryte,
        triet_lexer::NumericSuffix::Integer => AstSuffix::Integer,
        triet_lexer::NumericSuffix::Long => AstSuffix::Long,
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parse(source: &str) -> (Parser<'static>, PatternId) {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let id = parse_pattern(&mut parser).expect("parse failed");
        (parser, id)
    }

    fn try_parse(source: &str) -> Result<(Parser<'static>, PatternId), ParseError> {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let id = parse_pattern(&mut parser)?;
        Ok((parser, id))
    }

    // === Atoms ===

    #[test]
    fn parses_wildcard_pattern() {
        let (parser, id) = parse("_");
        assert!(matches!(parser.arena.pattern(id).node, Pattern::Wildcard));
    }

    #[test]
    fn parses_null_pattern() {
        let (parser, id) = parse("null");
        assert!(matches!(parser.arena.pattern(id).node, Pattern::Null));
    }

    #[test]
    fn parses_variable_pattern() {
        let (parser, id) = parse("name");
        match &parser.arena.pattern(id).node {
            Pattern::Variable(n) => assert_eq!(n, "name"),
            other => panic!("expected Variable, got {other:?}"),
        }
    }

    #[test]
    fn parses_integer_literal_pattern() {
        let (parser, id) = parse("42");
        match &parser.arena.pattern(id).node {
            Pattern::Literal(LiteralPattern::Integer { value, suffix }) => {
                assert_eq!(*value, 42);
                assert!(suffix.is_none());
            }
            other => panic!("expected integer literal, got {other:?}"),
        }
    }

    #[test]
    fn parses_negative_integer_literal_pattern() {
        let (parser, id) = parse("-7");
        match &parser.arena.pattern(id).node {
            Pattern::Literal(LiteralPattern::Integer { value, .. }) => assert_eq!(*value, -7),
            other => panic!("expected -7 integer, got {other:?}"),
        }
    }

    #[test]
    fn parses_suffixed_integer_pattern() {
        let (parser, id) = parse("5_tryte");
        match &parser.arena.pattern(id).node {
            Pattern::Literal(LiteralPattern::Integer { value, suffix }) => {
                assert_eq!(*value, 5);
                assert_eq!(*suffix, Some(AstSuffix::Tryte));
            }
            other => panic!("expected suffixed integer, got {other:?}"),
        }
    }

    #[test]
    fn parses_ternary_literal_pattern() {
        let (parser, id) = parse("0t+0-+");
        // Decoded value = 25 (see lexer test).
        match &parser.arena.pattern(id).node {
            Pattern::Literal(LiteralPattern::Ternary(value)) => assert_eq!(*value, 25),
            other => panic!("expected Ternary, got {other:?}"),
        }
    }

    #[test]
    fn parses_string_literal_pattern() {
        let (parser, id) = parse(r#""hello""#);
        match &parser.arena.pattern(id).node {
            Pattern::Literal(LiteralPattern::String(text)) => assert_eq!(text, "hello"),
            other => panic!("expected String literal, got {other:?}"),
        }
    }

    #[test]
    fn parses_trilean_literal_patterns() {
        for (source, expected) in [
            ("true", TrileanValue::True),
            ("false", TrileanValue::False),
            ("unknown", TrileanValue::Unknown),
        ] {
            let (parser, id) = parse(source);
            match &parser.arena.pattern(id).node {
                Pattern::Literal(LiteralPattern::Trilean(value)) => assert_eq!(*value, expected),
                other => panic!("expected Trilean({expected:?}), got {other:?}"),
            }
        }
    }

    // === Tuple ===

    #[test]
    fn parses_two_element_tuple_pattern() {
        let (parser, id) = parse("(a, b)");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => assert_eq!(elements.len(), 2),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parses_three_element_tuple_pattern_with_wildcard() {
        let (parser, id) = parse("(a, _, c)");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => {
                assert_eq!(elements.len(), 3);
                assert!(matches!(
                    parser.arena.pattern(elements[1]).node,
                    Pattern::Wildcard
                ));
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parses_empty_tuple_pattern() {
        let (parser, id) = parse("()");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => assert!(elements.is_empty()),
            other => panic!("expected empty Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parenthesized_single_pattern_is_inner() {
        let (parser, id) = parse("(x)");
        match &parser.arena.pattern(id).node {
            Pattern::Variable(n) => assert_eq!(n, "x"),
            other => panic!("expected Variable, got {other:?}"),
        }
    }

    #[test]
    fn singleton_tuple_with_trailing_comma_is_tuple() {
        let (parser, id) = parse("(x,)");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => assert_eq!(elements.len(), 1),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    // === Or ===

    #[test]
    fn parses_or_pattern_two_alternatives() {
        let (parser, id) = parse("1 | 2");
        match &parser.arena.pattern(id).node {
            Pattern::Or(alternatives) => assert_eq!(alternatives.len(), 2),
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn parses_or_pattern_three_alternatives() {
        let (parser, id) = parse("1 | 2 | 3");
        match &parser.arena.pattern(id).node {
            Pattern::Or(alternatives) => assert_eq!(alternatives.len(), 3),
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn or_does_not_cross_tuple_boundary() {
        // `(a, b | c)` should be Tuple(a, Or(b, c)), not Or(Tuple(a, b), c).
        let (parser, id) = parse("(a, b | c)");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => {
                assert_eq!(elements.len(), 2);
                match &parser.arena.pattern(elements[1]).node {
                    Pattern::Or(alts) => assert_eq!(alts.len(), 2),
                    other => panic!("expected Or as 2nd element, got {other:?}"),
                }
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    // === Range ===

    #[test]
    fn parses_inclusive_range_pattern() {
        let (parser, id) = parse("0..=9");
        match &parser.arena.pattern(id).node {
            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                assert!(*inclusive);
                assert!(matches!(start, LiteralPattern::Integer { value: 0, .. }));
                assert!(matches!(end, LiteralPattern::Integer { value: 9, .. }));
            }
            other => panic!("expected inclusive Range, got {other:?}"),
        }
    }

    #[test]
    fn parses_exclusive_range_pattern() {
        let (parser, id) = parse("0..9");
        match &parser.arena.pattern(id).node {
            Pattern::Range { inclusive, .. } => assert!(!*inclusive),
            other => panic!("expected Range, got {other:?}"),
        }
    }

    #[test]
    fn parses_negative_lower_bound_range() {
        let (parser, id) = parse("-5..=5");
        match &parser.arena.pattern(id).node {
            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                assert!(*inclusive);
                assert!(matches!(start, LiteralPattern::Integer { value: -5, .. }));
                assert!(matches!(end, LiteralPattern::Integer { value: 5, .. }));
            }
            other => panic!("expected Range, got {other:?}"),
        }
    }

    // === Errors ===

    #[test]
    fn errors_on_unexpected_token_at_pattern_position() {
        // `+` is not a valid pattern start.
        let result = try_parse("+");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    #[test]
    fn errors_on_eof_at_pattern_position() {
        let result = try_parse("");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn errors_on_unclosed_tuple() {
        let result = try_parse("(a, b");
        assert!(matches!(
            result,
            Err(ParseError::UnexpectedEof { .. } | ParseError::UnexpectedToken { .. })
        ));
    }

    // === Realistic combinations ===

    #[test]
    fn parses_fizzbuzz_match_arm_pattern() {
        let (parser, id) = parse("(0, 0)");
        match &parser.arena.pattern(id).node {
            Pattern::Tuple(elements) => {
                assert_eq!(elements.len(), 2);
                for &elem in elements {
                    assert!(matches!(
                        parser.arena.pattern(elem).node,
                        Pattern::Literal(LiteralPattern::Integer { value: 0, .. })
                    ));
                }
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    // === Outcome arm patterns (v0.7.4.3-error per ADR-0020 §5) ===

    /// `~+ value` parses as OutcomeArm { Positive, payload: Variable }.
    #[test]
    fn parses_outcome_positive_arm_pattern_with_binding() {
        let (parser, id) = parse("~+ value");
        match &parser.arena.pattern(id).node {
            Pattern::OutcomeArm { arm, payload } => {
                assert_eq!(*arm, OutcomeArm::Positive);
                let payload_id = payload.expect("positive arm binds variable");
                match &parser.arena.pattern(payload_id).node {
                    Pattern::Variable(name) => assert_eq!(name, "value"),
                    other => panic!("expected Variable, got {other:?}"),
                }
            }
            other => panic!("expected OutcomeArm, got {other:?}"),
        }
    }

    /// `~- error` parses as OutcomeArm { Negative, payload: Variable }.
    #[test]
    fn parses_outcome_negative_arm_pattern_with_binding() {
        let (parser, id) = parse("~- error");
        match &parser.arena.pattern(id).node {
            Pattern::OutcomeArm { arm, payload } => {
                assert_eq!(*arm, OutcomeArm::Negative);
                assert!(payload.is_some());
            }
            other => panic!("expected OutcomeArm, got {other:?}"),
        }
    }

    /// `~0` parses as OutcomeArm { Zero, payload: None }. No binding —
    /// null arm carries no payload per ADR-0020.
    #[test]
    fn parses_outcome_zero_arm_pattern_no_payload() {
        let (parser, id) = parse("~0");
        match &parser.arena.pattern(id).node {
            Pattern::OutcomeArm { arm, payload } => {
                assert_eq!(*arm, OutcomeArm::Zero);
                assert!(payload.is_none(), "~0 pattern has no payload");
            }
            other => panic!("expected OutcomeArm, got {other:?}"),
        }
    }

    /// Wildcard binding: `~+ _` discards positive arm payload.
    #[test]
    fn parses_outcome_positive_arm_pattern_with_wildcard() {
        let (parser, id) = parse("~+ _");
        match &parser.arena.pattern(id).node {
            Pattern::OutcomeArm { arm, payload } => {
                assert_eq!(*arm, OutcomeArm::Positive);
                let payload_id = payload.expect("wildcard is still a sub-pattern");
                assert!(matches!(
                    parser.arena.pattern(payload_id).node,
                    Pattern::Wildcard
                ));
            }
            other => panic!("expected OutcomeArm, got {other:?}"),
        }
    }
}
