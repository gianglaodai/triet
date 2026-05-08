//! Expression parser — Pratt-style with explicit binding-power table.
//!
//! Entry point: [`parse_expression`]. Supports the full SPEC §12.1
//! precedence/associativity ladder, including no-chain rules for
//! comparison, equality, and range operators. F-strings are parsed
//! inline (consuming the mode-aware token sequence emitted by the lexer).

use triet_lexer::{IntLiteral as LexIntLiteral, Span, Token};
use triet_syntax::{
    BinaryOperator, Block, Expr, ExprId, FStringPart, FStringSegments, LambdaParam, MatchArm,
    NumericSuffix as AstSuffix, Spanned, TrileanValue, UnaryOperator,
};

use crate::{error::ParseError, parser::Parser, pattern::parse_pattern, type_expr::parse_type};

/// Parse a full expression at minimum binding power 0.
pub(crate) fn parse_expression(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    parse_expression_bp(parser, 0)
}

/// Parse an expression that binds at least `min_bp`.
// `while let` cannot replace this `loop` cleanly: each iteration peeks,
// classifies as postfix or infix, and may break via several distinct
// guards beyond the leading match. The explicit `loop` keeps the
// control flow readable.
#[allow(clippy::while_let_loop)]
fn parse_expression_bp(parser: &mut Parser<'_>, min_bp: u8) -> Result<ExprId, ParseError> {
    let mut lhs = parse_prefix(parser)?;
    let mut last_no_chain_class: Option<NoChainClass> = None;

    loop {
        let Some(token) = parser.peek_token() else {
            break;
        };

        // Postfix operators bind tighter than any infix; handle them first.
        if let Some(_postfix_bp) = postfix_binding_power(token) {
            // Postfix has only a left binding power; if it's lower than
            // min_bp, stop.
            if postfix_binding_power(token).unwrap() < min_bp {
                break;
            }
            lhs = parse_postfix(parser, lhs)?;
            continue;
        }

        // Infix operators.
        let Some(op_kind) = classify_binary(token) else {
            break;
        };
        let (l_bp, r_bp) = op_kind.binding_power();
        if l_bp < min_bp {
            break;
        }

        // Enforce no-chain rules for comparison / equality / range.
        if let Some(class) = op_kind.no_chain_class()
            && let Some(prev) = last_no_chain_class
                && prev == class {
                    let span = parser.current_span();
                    parser.record_error(ParseError::ChainedNoChainOperator {
                        class: prev.label().to_owned(),
                        span,
                    });
                    // Continue parsing for recovery; the resulting AST
                    // associates left-to-right but the error is reported.
                }

        let op_span = parser.current_span();
        parser.advance(); // consume the operator token

        // Special structural operators that don't fit the BinaryOp shape.
        match op_kind {
            BinaryOpKind::Range { inclusive } => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::Range { start: lhs, end: rhs, inclusive },
                    span,
                ));
            }
            BinaryOpKind::Elvis => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::ElvisOp { object: lhs, default: rhs },
                    span,
                ));
            }
            BinaryOpKind::Plain(operator) => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::BinaryOp { operator, left: lhs, right: rhs },
                    span,
                ));
            }
        }

        last_no_chain_class = op_kind.no_chain_class();
        // The operator span is unused for further error context, but is
        // available if future passes want it.
        let _ = op_span;
    }

    Ok(lhs)
}

// ============================================================================
// Prefix parsing — atoms, prefix operators, and structural starts
// ============================================================================

fn parse_prefix(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let Some((token, span)) = parser.peek().cloned() else {
        return Err(ParseError::UnexpectedEof {
            expected: "expression".to_owned(),
            span: parser.eof_span(),
        });
    };

    match token {
        Token::IntegerLiteral(LexIntLiteral { value, suffix }) => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::IntegerLiteral { value, suffix: suffix.map(convert_suffix) },
                span,
            )))
        }
        Token::TernaryLiteral(LexIntLiteral { value, .. }) => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_expression(Spanned::new(Expr::TernaryLiteral { value }, span)))
        }
        Token::StringLiteral(text) => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_expression(Spanned::new(Expr::StringLiteral(text), span)))
        }
        Token::True => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral(TrileanValue::True),
                span,
            )))
        }
        Token::False => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral(TrileanValue::False),
                span,
            )))
        }
        Token::Unknown => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral(TrileanValue::Unknown),
                span,
            )))
        }
        Token::Null => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_expression(Spanned::new(Expr::NullLiteral, span)))
        }
        Token::Identifier(name) => {
            parser.advance();
            Ok(parser
                .arena
                .alloc_expression(Spanned::new(Expr::Identifier(name), span)))
        }
        Token::FStringStart => parse_f_string(parser),
        Token::LParen => parse_paren_or_tuple(parser, span),
        Token::LBrace => parse_block_expression(parser, span),
        Token::If | Token::IfQ => parse_if_expression(parser),
        Token::Match => parse_match_expression(parser),
        Token::Pipe | Token::OrOr => parse_lambda(parser),
        // Unary prefix operators
        Token::Minus | Token::Bang | Token::Not => parse_unary(parser),
        other => Err(ParseError::UnexpectedToken {
            expected: "expression".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

fn parse_unary(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let (op_token, op_span) = parser.peek().cloned().expect("caller checked peek");
    debug_assert!(matches!(op_token, Token::Minus | Token::Bang | Token::Not));
    parser.advance();
    let operand = parse_expression_bp(parser, UNARY_RIGHT_BP)?;
    let span = op_span.start..arena_span(parser, operand).end;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::UnaryOp { operator: UnaryOperator::Negate, operand },
        span,
    )))
}

fn parse_paren_or_tuple(
    parser: &mut Parser<'_>,
    open_span: Span,
) -> Result<ExprId, ParseError> {
    parser.expect(&Token::LParen, "`(`")?;

    if matches!(parser.peek_token(), Some(Token::RParen)) {
        // Empty tuple `()` — useful as Unit literal.
        let close = parser.expect(&Token::RParen, "`)`")?;
        let span = open_span.start..close.end;
        return Ok(parser
            .arena
            .alloc_expression(Spanned::new(Expr::Tuple(Vec::new()), span)));
    }

    let mut elements = vec![parse_expression(parser)?];
    let mut had_comma = false;
    while parser.eat(&Token::Comma) {
        had_comma = true;
        if matches!(parser.peek_token(), Some(Token::RParen)) {
            break;
        }
        elements.push(parse_expression(parser)?);
    }
    let close_span = parser.expect(&Token::RParen, "`)`")?;

    if elements.len() == 1 && !had_comma {
        return Ok(elements.into_iter().next().unwrap());
    }

    let span = open_span.start..close_span.end;
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Tuple(elements), span)))
}

fn parse_block_expression(
    parser: &mut Parser<'_>,
    open_span: Span,
) -> Result<ExprId, ParseError> {
    let block = parse_block(parser, open_span.clone())?;
    // Reconstruct span: start at `{`, end at `}` (the parse_block helper
    // returns the closing-brace span via its own bookkeeping below).
    let span = open_span.start..parser.previous_token_end(open_span.end);
    Ok(parser.arena.alloc_expression(Spanned::new(Expr::Block(block), span)))
}

/// Parse `{ stmts? final_expr? }` into a `Block`. Used both as block
/// expression and as function/match-arm body.
pub(crate) fn parse_block(
    parser: &mut Parser<'_>,
    _open_span: Span,
) -> Result<Block, ParseError> {
    parser.expect(&Token::LBrace, "`{`")?;

    let mut statements = Vec::new();
    let mut final_expression: Option<ExprId> = None;

    while !matches!(parser.peek_token(), Some(Token::RBrace)) && !parser.at_end() {
        // Try statement first; statement parser decides whether to treat
        // a leading expression as expr-stmt or final expression.
        match crate::stmt::parse_statement_or_final_expr(parser)? {
            crate::stmt::StatementOrFinal::Statement(stmt_id) => statements.push(stmt_id),
            crate::stmt::StatementOrFinal::FinalExpression(expr_id) => {
                final_expression = Some(expr_id);
                break;
            }
        }
    }

    parser.expect(&Token::RBrace, "`}`")?;

    Ok(Block {
        statements,
        final_expression,
    })
}

fn parse_if_expression(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let (head_token, head_span) = parser.peek().cloned().expect("caller checked");
    let treat_unknown_as_false = matches!(head_token, Token::IfQ);
    parser.advance();

    let condition = parse_expression(parser)?;

    let then_open = parser.current_span();
    let then_branch = parse_block(parser, then_open)?;

    let else_branch = if parser.eat(&Token::Else) {
        if matches!(parser.peek_token(), Some(Token::If | Token::IfQ)) {
            // `else if` chain — wrap as block whose final expression is
            // the inner `if` expression.
            let inner = parse_if_expression(parser)?;
            Some(Block {
                statements: Vec::new(),
                final_expression: Some(inner),
            })
        } else {
            let else_open = parser.current_span();
            Some(parse_block(parser, else_open)?)
        }
    } else {
        None
    };

    let end_span = parser.previous_token_end(head_span.end);
    let span = head_span.start..end_span;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::If {
            condition,
            then_branch,
            else_branch,
            treat_unknown_as_false,
        },
        span,
    )))
}

fn parse_match_expression(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let head_span = parser.expect(&Token::Match, "`match`")?;
    let scrutinee = parse_expression(parser)?;
    parser.expect(&Token::LBrace, "`{`")?;

    let mut arms = Vec::new();
    while !matches!(parser.peek_token(), Some(Token::RBrace)) && !parser.at_end() {
        let pattern = parse_pattern(parser)?;
        let guard = if parser.eat(&Token::If) {
            // Parse with min_bp above implication so the guard does not
            // greedily consume the arm separator `=>`. Implication has
            // l_bp=1 in our table; using min_bp=2 leaves `=>` for the
            // outer match arm to claim.
            Some(parse_expression_bp(parser, 2)?)
        } else {
            None
        };
        parser.expect(&Token::FatArrow, "`=>`")?;
        let body = parse_expression(parser)?;
        // Arms separated by `,`; trailing comma allowed.
        let _ = parser.eat(&Token::Comma);

        // Per-arm spans are reachable via the arena (pattern.span,
        // body.span); no need to record them on the MatchArm directly.
        arms.push(MatchArm {
            pattern,
            guard,
            body,
        });
    }

    let close = parser.expect(&Token::RBrace, "`}`")?;
    let span = head_span.start..close.end;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::Match { scrutinee, arms },
        span,
    )))
}

fn parse_lambda(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let (head_token, head_span) = parser.peek().cloned().expect("caller checked");
    parser.advance();

    let parameters = match head_token {
        Token::OrOr => Vec::new(), // `||` — no params
        Token::Pipe => {
            let mut params = Vec::new();
            if !matches!(parser.peek_token(), Some(Token::Pipe)) {
                loop {
                    let (name_token, _name_span) = parser
                        .peek()
                        .cloned()
                        .ok_or_else(|| ParseError::UnexpectedEof {
                            expected: "lambda parameter name".to_owned(),
                            span: parser.eof_span(),
                        })?;
                    let name = match name_token {
                        Token::Identifier(name) => {
                            parser.advance();
                            name
                        }
                        other => {
                            return Err(ParseError::UnexpectedToken {
                                expected: "identifier".to_owned(),
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
                    params.push(LambdaParam { name, type_annotation });
                    if !parser.eat(&Token::Comma) {
                        break;
                    }
                }
            }
            parser.expect(&Token::Pipe, "`|`")?;
            params
        }
        _ => unreachable!("caller filtered"),
    };

    let return_type = if parser.eat(&Token::ThinArrow) {
        Some(parse_type(parser)?)
    } else {
        None
    };

    let body = parse_expression(parser)?;
    let body_span = arena_span(parser, body);
    let span = head_span.start..body_span.end;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::Lambda {
            parameters,
            return_type,
            body,
        },
        span,
    )))
}

// ============================================================================
// Postfix
// ============================================================================

const fn postfix_binding_power(token: &Token) -> Option<u8> {
    match token {
        Token::Dot
        | Token::QuestionDot
        | Token::LParen
        | Token::LBracket
        | Token::BangBang => Some(POSTFIX_LEFT_BP),
        _ => None,
    }
}

fn parse_postfix(parser: &mut Parser<'_>, lhs: ExprId) -> Result<ExprId, ParseError> {
    let (token, _span) = parser.peek().cloned().expect("caller checked postfix BP");
    match token {
        Token::Dot => parse_dot_postfix(parser, lhs, false),
        Token::QuestionDot => parse_dot_postfix(parser, lhs, true),
        Token::LParen => parse_call(parser, lhs),
        Token::BangBang => {
            let bang_span = parser.expect(&Token::BangBang, "`!!`")?;
            let span = arena_span(parser, lhs).start..bang_span.end;
            Ok(parser
                .arena
                .alloc_expression(Spanned::new(Expr::ForceUnwrap(lhs), span)))
        }
        Token::LBracket => {
            // Subscript / index access — not in v0.1 SPEC; treat as error
            // for now to keep semantics tight.
            let span = parser.current_span();
            Err(ParseError::UnexpectedToken {
                expected: "operator".to_owned(),
                found: "`[` (indexing not supported in v0.1)".to_owned(),
                span,
            })
        }
        _ => unreachable!("caller filtered postfix tokens"),
    }
}

fn parse_dot_postfix(
    parser: &mut Parser<'_>,
    receiver: ExprId,
    safe: bool,
) -> Result<ExprId, ParseError> {
    parser.advance(); // consume `.` or `?.`

    let (token, span) = parser.peek().cloned().ok_or_else(|| ParseError::UnexpectedEof {
        expected: "field name, method name, or tuple index".to_owned(),
        span: parser.eof_span(),
    })?;

    match token {
        Token::Identifier(name) => {
            parser.advance();
            // Method call iff next is `(`.
            if matches!(parser.peek_token(), Some(Token::LParen)) {
                let arguments = parse_call_args(parser)?;
                // Span end = end of last `)` parsed by parse_call_args.
                let span_end = parser.previous_token_end(span.end);
                let full_span = arena_span(parser, receiver).start..span_end;
                let expr = if safe {
                    Expr::SafeMethodCall {
                        receiver,
                        method: name,
                        arguments,
                    }
                } else {
                    Expr::MethodCall {
                        receiver,
                        method: name,
                        arguments,
                    }
                };
                Ok(parser.arena.alloc_expression(Spanned::new(expr, full_span)))
            } else {
                let full_span = arena_span(parser, receiver).start..span.end;
                let expr = if safe {
                    Expr::SafeFieldAccess {
                        object: receiver,
                        field: name,
                    }
                } else {
                    Expr::FieldAccess {
                        object: receiver,
                        field: name,
                    }
                };
                Ok(parser.arena.alloc_expression(Spanned::new(expr, full_span)))
            }
        }
        Token::IntegerLiteral(LexIntLiteral { value, suffix: None }) if !safe => {
            // Tuple index: `pair.0`. Only base-10 unsuffixed integers
            // make sense as indices.
            parser.advance();
            let index = usize::try_from(value).map_err(|_| ParseError::InvalidLiteral {
                message: "tuple index must be a non-negative integer".to_owned(),
                span: span.clone(),
            })?;
            let full_span = arena_span(parser, receiver).start..span.end;
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TupleIndex { tuple: receiver, index },
                full_span,
            )))
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "field name or method name".to_owned(),
            found: format!("{other:?}"),
            span,
        }),
    }
}

fn parse_call(parser: &mut Parser<'_>, callee: ExprId) -> Result<ExprId, ParseError> {
    let arguments = parse_call_args(parser)?;
    let span_end = parser.previous_token_end(arena_span(parser, callee).end);
    let span = arena_span(parser, callee).start..span_end;
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Call { callee, arguments }, span)))
}

fn parse_call_args(parser: &mut Parser<'_>) -> Result<Vec<ExprId>, ParseError> {
    parser.expect(&Token::LParen, "`(`")?;
    let mut arguments = Vec::new();
    if !matches!(parser.peek_token(), Some(Token::RParen)) {
        loop {
            arguments.push(parse_expression(parser)?);
            if !parser.eat(&Token::Comma) {
                break;
            }
            if matches!(parser.peek_token(), Some(Token::RParen)) {
                break;
            }
        }
    }
    parser.expect(&Token::RParen, "`)`")?;
    Ok(arguments)
}

// ============================================================================
// F-string parsing
// ============================================================================

fn parse_f_string(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let start_span = parser.expect(&Token::FStringStart, "`f\"`")?;
    let mut parts = Vec::new();

    loop {
        let Some((token, span)) = parser.peek().cloned() else {
            return Err(ParseError::UnexpectedEof {
                expected: "f-string body".to_owned(),
                span: parser.eof_span(),
            });
        };
        match token {
            Token::FStringText(text) => {
                parser.advance();
                let _ = span;
                parts.push(FStringPart::Text(text));
            }
            Token::InterpolationStart => {
                parser.advance();
                let expression = parse_expression(parser)?;
                // Optional `:format_spec` — currently only supported as
                // a single identifier-like sequence; defer formal spec
                // grammar to v0.2.
                let format_spec = if parser.eat(&Token::Colon) {
                    Some(parse_format_spec(parser)?)
                } else {
                    None
                };
                parser.expect(&Token::InterpolationEnd, "`}`")?;
                parts.push(FStringPart::Interpolation { expression, format_spec });
            }
            Token::FStringEnd => {
                parser.advance();
                let end_span = span.end;
                let segments = FStringSegments { parts };
                let full_span = start_span.start..end_span;
                return Ok(parser.arena.alloc_expression(Spanned::new(
                    Expr::FStringLiteral(segments),
                    full_span,
                )));
            }
            other => {
                return Err(ParseError::InvalidInterpolation {
                    message: format!("unexpected {other:?}"),
                    span,
                });
            }
        }
    }
}

/// Read tokens until `}` (the matching `InterpolationEnd`). Returns the
/// raw spec text. v0.1 keeps this minimal — only one identifier, integer,
/// or punctuation is concatenated. Real format-spec grammar is v0.2.
fn parse_format_spec(parser: &mut Parser<'_>) -> Result<String, ParseError> {
    let mut spec = String::new();
    while !matches!(parser.peek_token(), Some(Token::InterpolationEnd) | None) {
        let (token, _) = parser.peek().cloned().expect("checked Some");
        // Render token kind as text. Imperfect but adequate for v0.1.
        spec.push_str(&render_format_token(&token));
        parser.advance();
    }
    Ok(spec)
}

fn render_format_token(token: &Token) -> String {
    match token {
        Token::Identifier(name) => name.clone(),
        Token::IntegerLiteral(LexIntLiteral { value, .. }) => value.to_string(),
        Token::Dot => ".".to_owned(),
        Token::Plus => "+".to_owned(),
        Token::Minus => "-".to_owned(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parse_expr(source: &str) -> (Parser<'static>, ExprId) {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let id = parse_expression(&mut parser).expect("parse failed");
        (parser, id)
    }

    fn try_parse_expr(source: &str) -> Result<(Parser<'static>, ExprId), ParseError> {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let id = parse_expression(&mut parser)?;
        Ok((parser, id))
    }

    fn binary_op_of(parser: &Parser<'_>, id: ExprId) -> BinaryOperator {
        match &parser.arena.expression(id).node {
            Expr::BinaryOp { operator, .. } => *operator,
            other => panic!("expected BinaryOp, got {other:?}"),
        }
    }

    fn left_of(parser: &Parser<'_>, id: ExprId) -> ExprId {
        match parser.arena.expression(id).node {
            Expr::BinaryOp { left, .. } => left,
            _ => panic!("not a binary op"),
        }
    }

    fn right_of(parser: &Parser<'_>, id: ExprId) -> ExprId {
        match parser.arena.expression(id).node {
            Expr::BinaryOp { right, .. } => right,
            _ => panic!("not a binary op"),
        }
    }

    fn integer_value_of(parser: &Parser<'_>, id: ExprId) -> i128 {
        match &parser.arena.expression(id).node {
            Expr::IntegerLiteral { value, .. } => *value,
            other => panic!("expected IntegerLiteral, got {other:?}"),
        }
    }

    // === Literals ===

    #[test]
    fn parses_integer_literal() {
        let (parser, id) = parse_expr("42");
        assert_eq!(integer_value_of(&parser, id), 42);
    }

    #[test]
    fn parses_ternary_literal() {
        let (parser, id) = parse_expr("0t+0-+");
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::TernaryLiteral { value: 25 },
        ));
    }

    #[test]
    fn parses_string_literal() {
        let (parser, id) = parse_expr(r#""hi""#);
        match &parser.arena.expression(id).node {
            Expr::StringLiteral(s) => assert_eq!(s, "hi"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_trilean_literals() {
        for (source, expected) in [
            ("true", TrileanValue::True),
            ("false", TrileanValue::False),
            ("unknown", TrileanValue::Unknown),
        ] {
            let (parser, id) = parse_expr(source);
            match &parser.arena.expression(id).node {
                Expr::TrileanLiteral(value) => assert_eq!(*value, expected),
                other => panic!("expected Trilean, got {other:?}"),
            }
        }
    }

    #[test]
    fn parses_null_literal() {
        let (parser, id) = parse_expr("null");
        assert!(matches!(parser.arena.expression(id).node, Expr::NullLiteral));
    }

    #[test]
    fn parses_identifier() {
        let (parser, id) = parse_expr("name");
        match &parser.arena.expression(id).node {
            Expr::Identifier(n) => assert_eq!(n, "name"),
            other => panic!("got {other:?}"),
        }
    }

    // === Arithmetic precedence ===

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        // a + b * c → a + (b * c)
        let (parser, id) = parse_expr("a + b * c");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Add);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::Multiply);
    }

    #[test]
    fn power_binds_tighter_than_multiplication() {
        // a * b ** c → a * (b ** c)
        let (parser, id) = parse_expr("a * b ** c");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Multiply);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::Power);
    }

    #[test]
    fn power_is_right_associative() {
        // 2 ** 3 ** 2 → 2 ** (3 ** 2)
        let (parser, id) = parse_expr("2 ** 3 ** 2");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Power);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::Power);
        let left = left_of(&parser, id);
        assert_eq!(integer_value_of(&parser, left), 2);
    }

    #[test]
    fn addition_is_left_associative() {
        // a + b + c → (a + b) + c
        let (parser, id) = parse_expr("a + b + c");
        let left = left_of(&parser, id);
        assert_eq!(binary_op_of(&parser, left), BinaryOperator::Add);
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // -2 ** 2 → -(2 ** 2) = -4
        let (parser, id) = parse_expr("-2 ** 2");
        match &parser.arena.expression(id).node {
            Expr::UnaryOp { operator: UnaryOperator::Negate, operand } => {
                assert_eq!(binary_op_of(&parser, *operand), BinaryOperator::Power);
            }
            other => panic!("expected UnaryOp wrapping Power, got {other:?}"),
        }
    }

    #[test]
    fn unary_minus_binds_tighter_than_multiplication() {
        // -a * b → (-a) * b
        let (parser, id) = parse_expr("-a * b");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Multiply);
        let left = left_of(&parser, id);
        assert!(matches!(parser.arena.expression(left).node, Expr::UnaryOp { .. }));
    }

    // === Logic precedence ===

    #[test]
    fn and_binds_tighter_than_or() {
        // a or b and c → a or (b and c)
        let (parser, id) = parse_expr("a or b and c");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Or);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::And);
    }

    #[test]
    fn xor_between_and_and_or() {
        // a or b xor c and d → a or (b xor (c and d))
        let (parser, id) = parse_expr("a or b xor c and d");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Or);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::Xor);
    }

    #[test]
    fn implication_is_right_associative() {
        // a => b => c → a => (b => c)
        let (parser, id) = parse_expr("a => b => c");
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::Implies);
        let right = right_of(&parser, id);
        assert_eq!(binary_op_of(&parser, right), BinaryOperator::Implies);
    }

    #[test]
    fn keyword_and_symbol_logic_ops_produce_same_ast() {
        let (parser1, id1) = parse_expr("a and b");
        let (parser2, id2) = parse_expr("a && b");
        assert_eq!(
            binary_op_of(&parser1, id1),
            binary_op_of(&parser2, id2),
        );
    }

    #[test]
    fn implies_keyword_and_arrow_produce_same_ast() {
        let (p1, id1) = parse_expr("a implies b");
        let (p2, id2) = parse_expr("a => b");
        assert_eq!(binary_op_of(&p1, id1), binary_op_of(&p2, id2));
    }

    #[test]
    fn kleene_implies_distinct_from_implies() {
        let (p1, id1) = parse_expr("a => b");
        let (p2, id2) = parse_expr("a ~> b");
        assert_ne!(binary_op_of(&p1, id1), binary_op_of(&p2, id2));
    }

    // === Comparison no-chain ===

    #[test]
    fn comparison_chain_is_recorded_as_error_but_recovers() {
        // `a < b < c` is a comparison chain — error recorded, but parse
        // continues and produces an AST.
        let tokens: Vec<_> = lex("a < b < c").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let _ = parse_expression(&mut parser).unwrap();
        let (_, errors) = parser.finish();
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::ChainedNoChainOperator { .. }));
    }

    #[test]
    fn equality_chain_is_recorded_as_error() {
        let tokens: Vec<_> = lex("a == b == c").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let _ = parse_expression(&mut parser).unwrap();
        let (_, errors) = parser.finish();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ParseError::ChainedNoChainOperator { .. })));
    }

    #[test]
    fn comparison_with_arithmetic_does_not_chain() {
        // `a + b < c + d` is fine — no chain.
        let tokens: Vec<_> = lex("a + b < c + d").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let _ = parse_expression(&mut parser).unwrap();
        let (_, errors) = parser.finish();
        assert!(errors.is_empty());
    }

    // === Range ===

    #[test]
    fn range_produces_range_node_not_binary_op() {
        let (parser, id) = parse_expr("0..100");
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::Range { inclusive: false, .. },
        ));
    }

    #[test]
    fn inclusive_range_recognized() {
        let (parser, id) = parse_expr("0..=100");
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::Range { inclusive: true, .. },
        ));
    }

    #[test]
    fn range_chain_is_recorded_as_error() {
        let tokens: Vec<_> = lex("a..b..c").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let _ = parse_expression(&mut parser).unwrap();
        let (_, errors) = parser.finish();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ParseError::ChainedNoChainOperator { .. })));
    }

    // === Elvis ===

    #[test]
    fn elvis_produces_elvis_op_node() {
        let (parser, id) = parse_expr("x ?: 0");
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::ElvisOp { .. },
        ));
    }

    #[test]
    fn elvis_is_right_associative() {
        // a ?: b ?: c → a ?: (b ?: c)
        let (parser, id) = parse_expr("a ?: b ?: c");
        match &parser.arena.expression(id).node {
            Expr::ElvisOp { default, .. } => {
                assert!(matches!(
                    parser.arena.expression(*default).node,
                    Expr::ElvisOp { .. },
                ));
            }
            other => panic!("expected outer ElvisOp, got {other:?}"),
        }
    }

    #[test]
    fn elvis_default_includes_arithmetic() {
        // a ?: b + c → a ?: (b + c)
        let (parser, id) = parse_expr("a ?: b + c");
        match &parser.arena.expression(id).node {
            Expr::ElvisOp { default, .. } => {
                assert_eq!(binary_op_of(&parser, *default), BinaryOperator::Add);
            }
            other => panic!("expected ElvisOp, got {other:?}"),
        }
    }

    // === Postfix ===

    #[test]
    fn parses_function_call() {
        let (parser, id) = parse_expr("foo(1, 2)");
        match &parser.arena.expression(id).node {
            Expr::Call { arguments, .. } => assert_eq!(arguments.len(), 2),
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn parses_method_call() {
        let (parser, id) = parse_expr("n.to_tryte()");
        match &parser.arena.expression(id).node {
            Expr::MethodCall { method, .. } => assert_eq!(method, "to_tryte"),
            other => panic!("expected MethodCall, got {other:?}"),
        }
    }

    #[test]
    fn parses_field_access() {
        let (parser, id) = parse_expr("point.x");
        match &parser.arena.expression(id).node {
            Expr::FieldAccess { field, .. } => assert_eq!(field, "x"),
            other => panic!("expected FieldAccess, got {other:?}"),
        }
    }

    #[test]
    fn parses_tuple_index() {
        let (parser, id) = parse_expr("pair.0");
        match &parser.arena.expression(id).node {
            Expr::TupleIndex { index, .. } => assert_eq!(*index, 0),
            other => panic!("expected TupleIndex, got {other:?}"),
        }
    }

    #[test]
    fn parses_safe_call_chain() {
        let (parser, id) = parse_expr("name?.length");
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::SafeFieldAccess { .. },
        ));
    }

    #[test]
    fn parses_force_unwrap() {
        let (parser, id) = parse_expr("name!!");
        assert!(matches!(parser.arena.expression(id).node, Expr::ForceUnwrap(_)));
    }

    #[test]
    fn chained_method_calls() {
        // a.b().c.d()
        let (parser, id) = parse_expr("a.b().c.d()");
        // Outer is MethodCall on (a.b().c)
        assert!(matches!(parser.arena.expression(id).node, Expr::MethodCall { .. }));
    }

    // === Unary forms ===

    #[test]
    fn unary_minus_bang_not_all_negate() {
        for source in ["-x", "!x", "not x"] {
            let (parser, id) = parse_expr(source);
            assert!(matches!(
                parser.arena.expression(id).node,
                Expr::UnaryOp { operator: UnaryOperator::Negate, .. },
            ));
        }
    }

    #[test]
    fn double_unary_compounds() {
        let (parser, id) = parse_expr("--x");
        match &parser.arena.expression(id).node {
            Expr::UnaryOp { operand, .. } => {
                assert!(matches!(
                    parser.arena.expression(*operand).node,
                    Expr::UnaryOp { .. },
                ));
            }
            other => panic!("got {other:?}"),
        }
    }

    // === Tuple / paren ===

    #[test]
    fn parens_around_single_expr_unwrap() {
        let (parser, id) = parse_expr("(42)");
        assert!(matches!(parser.arena.expression(id).node, Expr::IntegerLiteral { .. }));
    }

    #[test]
    fn tuple_with_two_elements() {
        let (parser, id) = parse_expr("(1, 2)");
        match &parser.arena.expression(id).node {
            Expr::Tuple(elements) => assert_eq!(elements.len(), 2),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn empty_tuple() {
        let (parser, id) = parse_expr("()");
        match &parser.arena.expression(id).node {
            Expr::Tuple(elements) => assert!(elements.is_empty()),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn singleton_tuple_with_trailing_comma() {
        let (parser, id) = parse_expr("(42,)");
        match &parser.arena.expression(id).node {
            Expr::Tuple(elements) => assert_eq!(elements.len(), 1),
            other => panic!("expected 1-tuple, got {other:?}"),
        }
    }

    // === Block ===

    #[test]
    fn parses_block_expression_with_final_value() {
        let (parser, id) = parse_expr("{ 42 }");
        match &parser.arena.expression(id).node {
            Expr::Block(block) => {
                assert!(block.statements.is_empty());
                assert!(block.final_expression.is_some());
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn parses_block_with_statements_and_final_expr() {
        let (parser, id) = parse_expr("{ let x = 5; x }");
        match &parser.arena.expression(id).node {
            Expr::Block(block) => {
                assert_eq!(block.statements.len(), 1);
                assert!(block.final_expression.is_some());
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // === If ===

    #[test]
    fn parses_simple_if_expression() {
        let (parser, id) = parse_expr("if cond { 1 } else { 2 }");
        match &parser.arena.expression(id).node {
            Expr::If { else_branch, treat_unknown_as_false, .. } => {
                assert!(else_branch.is_some());
                assert!(!*treat_unknown_as_false);
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn parses_if_question_variant() {
        let (parser, id) = parse_expr("if? cond { 1 } else { 2 }");
        match &parser.arena.expression(id).node {
            Expr::If { treat_unknown_as_false, .. } => assert!(*treat_unknown_as_false),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_if_without_else() {
        let (parser, id) = parse_expr("if cond { 1 }");
        match &parser.arena.expression(id).node {
            Expr::If { else_branch: None, .. } => {}
            other => panic!("expected If(None), got {other:?}"),
        }
    }

    #[test]
    fn parses_if_else_if_chain() {
        let (parser, id) = parse_expr("if a { 1 } else if b { 2 } else { 3 }");
        match &parser.arena.expression(id).node {
            Expr::If { else_branch: Some(_), .. } => {}
            other => panic!("expected If with else, got {other:?}"),
        }
    }

    // === Match ===

    #[test]
    fn parses_simple_match() {
        let (parser, id) = parse_expr("match x { 0 => \"zero\", _ => \"other\" }");
        match &parser.arena.expression(id).node {
            Expr::Match { arms, .. } => assert_eq!(arms.len(), 2),
            other => panic!("expected Match, got {other:?}"),
        }
    }

    #[test]
    fn parses_match_with_guard() {
        let (parser, id) = parse_expr("match n { x if x > 0 => \"pos\", _ => \"other\" }");
        match &parser.arena.expression(id).node {
            Expr::Match { arms, .. } => {
                assert_eq!(arms.len(), 2);
                assert!(arms[0].guard.is_some());
            }
            other => panic!("got {other:?}"),
        }
    }

    // === F-strings ===

    #[test]
    fn parses_simple_f_string_with_interpolation() {
        let (parser, id) = parse_expr(r#"f"hi {name}""#);
        match &parser.arena.expression(id).node {
            Expr::FStringLiteral(segments) => {
                assert_eq!(segments.parts.len(), 2);
                assert!(matches!(&segments.parts[0], FStringPart::Text(t) if t == "hi "));
                assert!(matches!(&segments.parts[1], FStringPart::Interpolation { .. }));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_empty_f_string() {
        let (parser, id) = parse_expr(r#"f"""#);
        match &parser.arena.expression(id).node {
            Expr::FStringLiteral(segments) => assert!(segments.parts.is_empty()),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_f_string_with_complex_interpolation() {
        let (parser, id) = parse_expr(r#"f"sum: {a + b * 2}""#);
        match &parser.arena.expression(id).node {
            Expr::FStringLiteral(segments) => {
                assert_eq!(segments.parts.len(), 2);
                if let FStringPart::Interpolation { expression, .. } = &segments.parts[1] {
                    assert!(matches!(
                        parser.arena.expression(*expression).node,
                        Expr::BinaryOp { .. },
                    ));
                } else {
                    panic!("expected Interpolation as second part");
                }
            }
            other => panic!("got {other:?}"),
        }
    }

    // === Lambda ===

    #[test]
    fn parses_no_param_lambda() {
        let (parser, id) = parse_expr("|| 5");
        match &parser.arena.expression(id).node {
            Expr::Lambda { parameters, .. } => assert!(parameters.is_empty()),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_one_param_lambda() {
        let (parser, id) = parse_expr("|x| x + 1");
        match &parser.arena.expression(id).node {
            Expr::Lambda { parameters, .. } => assert_eq!(parameters.len(), 1),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_lambda_with_typed_param() {
        let (parser, id) = parse_expr("|x: Integer| x");
        match &parser.arena.expression(id).node {
            Expr::Lambda { parameters, .. } => assert!(parameters[0].type_annotation.is_some()),
            other => panic!("got {other:?}"),
        }
    }

    // === Errors ===

    #[test]
    fn errors_on_expression_at_eof() {
        let result = try_parse_expr("");
        assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn errors_on_unexpected_token() {
        // `;` cannot start an expression
        let result = try_parse_expr(";");
        assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    }

    // === Realistic samples ===

    #[test]
    fn parses_fizzbuzz_match_rhs() {
        let source = r#"match (n %% 3, n %% 5) {
            (0, 0) => "FizzBuzz",
            (0, _) => "Fizz",
            (_, 0) => "Buzz",
            _ => to_string(n),
        }"#;
        let (parser, id) = parse_expr(source);
        match &parser.arena.expression(id).node {
            Expr::Match { arms, .. } => assert_eq!(arms.len(), 4),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_logical_chain_from_measles_demo() {
        let source = "fever and rash and not vaccinated";
        let (parser, id) = parse_expr(source);
        // Outer should be a binary And.
        assert_eq!(binary_op_of(&parser, id), BinaryOperator::And);
    }
}

// ============================================================================
// Operator classification
// ============================================================================

/// What kind of binary expression a token introduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOpKind {
    Plain(BinaryOperator),
    Range { inclusive: bool },
    Elvis,
}

impl BinaryOpKind {
    const fn binding_power(self) -> (u8, u8) {
        match self {
            // Right-associative implication — lowest precedence
            Self::Plain(BinaryOperator::Implies | BinaryOperator::KleeneImplies) => (1, 0),
            // Left-associative biconditional
            Self::Plain(BinaryOperator::Iff | BinaryOperator::KleeneIff) => (2, 3),
            // Left-associative or — loosest of the boolean trio
            Self::Plain(BinaryOperator::Or) => (4, 5),
            // Left-associative xor — between or and and (per SPEC §12.1
            // level 5; AND-tighter-than-XOR-tighter-than-OR)
            Self::Plain(BinaryOperator::Xor | BinaryOperator::KleeneXor) => (6, 7),
            // Left-associative and — tightest of the boolean trio
            Self::Plain(BinaryOperator::And) => (8, 9),
            // Equality (no chain — the no-chain rule is enforced by the
            // class check; the binding-power table itself is consistent
            // with left-associative).
            Self::Plain(BinaryOperator::Equal | BinaryOperator::NotEqual) => (10, 11),
            // Comparison
            Self::Plain(
                BinaryOperator::LessThan
                | BinaryOperator::LessEqual
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterEqual,
            ) => (12, 13),
            // Elvis — right-associative
            Self::Elvis => (15, 14),
            // Range — no chain
            Self::Range { .. } => (16, 17),
            // Additive
            Self::Plain(BinaryOperator::Add | BinaryOperator::Subtract) => (18, 19),
            // Multiplicative
            Self::Plain(
                BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo,
            ) => (20, 21),
            // Power — right-associative, higher than unary (handled by
            // unary right_bp = 23 so prefix `-` binds looser than `**`)
            Self::Plain(BinaryOperator::Power) => (26, 25),
        }
    }

    const fn no_chain_class(self) -> Option<NoChainClass> {
        match self {
            Self::Plain(BinaryOperator::Equal | BinaryOperator::NotEqual) => {
                Some(NoChainClass::Equality)
            }
            Self::Plain(
                BinaryOperator::LessThan
                | BinaryOperator::LessEqual
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterEqual,
            ) => Some(NoChainClass::Comparison),
            Self::Range { .. } => Some(NoChainClass::Range),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoChainClass {
    Equality,
    Comparison,
    Range,
}

impl NoChainClass {
    const fn label(self) -> &'static str {
        match self {
            Self::Equality => "equality",
            Self::Comparison => "comparison",
            Self::Range => "range",
        }
    }
}

/// Map a token to its binary-operator kind. Returns `None` if the token
/// does not introduce a binary operator at this position.
const fn classify_binary(token: &Token) -> Option<BinaryOpKind> {
    Some(match token {
        Token::Plus => BinaryOpKind::Plain(BinaryOperator::Add),
        Token::Minus => BinaryOpKind::Plain(BinaryOperator::Subtract),
        Token::Star => BinaryOpKind::Plain(BinaryOperator::Multiply),
        Token::Slash => BinaryOpKind::Plain(BinaryOperator::Divide),
        Token::PercentPercent => BinaryOpKind::Plain(BinaryOperator::Modulo),
        Token::StarStar => BinaryOpKind::Plain(BinaryOperator::Power),

        Token::EqEq => BinaryOpKind::Plain(BinaryOperator::Equal),
        Token::NotEq => BinaryOpKind::Plain(BinaryOperator::NotEqual),
        Token::Lt => BinaryOpKind::Plain(BinaryOperator::LessThan),
        Token::LtEq => BinaryOpKind::Plain(BinaryOperator::LessEqual),
        Token::Gt => BinaryOpKind::Plain(BinaryOperator::GreaterThan),
        Token::GtEq => BinaryOpKind::Plain(BinaryOperator::GreaterEqual),

        Token::AndAnd | Token::And => BinaryOpKind::Plain(BinaryOperator::And),
        Token::OrOr | Token::Or => BinaryOpKind::Plain(BinaryOperator::Or),

        Token::Caret | Token::Xor => BinaryOpKind::Plain(BinaryOperator::Xor),
        Token::TildeCaret | Token::KleeneXor => BinaryOpKind::Plain(BinaryOperator::KleeneXor),
        Token::LtEqGt | Token::Iff => BinaryOpKind::Plain(BinaryOperator::Iff),
        Token::LtTildeGt | Token::KleeneIff => BinaryOpKind::Plain(BinaryOperator::KleeneIff),
        Token::FatArrow | Token::Implies => BinaryOpKind::Plain(BinaryOperator::Implies),
        Token::TildeArrow | Token::KleeneImplies => {
            BinaryOpKind::Plain(BinaryOperator::KleeneImplies)
        }

        Token::QuestionColon => BinaryOpKind::Elvis,
        Token::DotDot => BinaryOpKind::Range { inclusive: false },
        Token::DotDotEq => BinaryOpKind::Range { inclusive: true },

        _ => return None,
    })
}

// ============================================================================
// Constants for binding powers used by prefix/postfix
// ============================================================================

const UNARY_RIGHT_BP: u8 = 23;
const POSTFIX_LEFT_BP: u8 = 28;

// ============================================================================
// Helpers
// ============================================================================

fn arena_span(parser: &Parser<'_>, id: ExprId) -> Span {
    parser.arena.expression(id).span.clone()
}

const fn convert_suffix(suffix: triet_lexer::NumericSuffix) -> AstSuffix {
    match suffix {
        triet_lexer::NumericSuffix::Trit => AstSuffix::Trit,
        triet_lexer::NumericSuffix::Tryte => AstSuffix::Tryte,
        triet_lexer::NumericSuffix::Integer => AstSuffix::Integer,
        triet_lexer::NumericSuffix::Long => AstSuffix::Long,
    }
}

