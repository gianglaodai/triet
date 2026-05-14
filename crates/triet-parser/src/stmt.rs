//! Statement parser — `let`, `const`, `return`, `break`, `continue`,
//! `for`, `while`, `while?`, `loop`, plus expression-statements.

use triet_lexer::{Span, Token};
use triet_syntax::{Block, Expr, ExprId, Spanned, Stmt, StmtId};

use crate::{
    error::ParseError,
    expr::{parse_block, parse_expression},
    parser::Parser,
    pattern::parse_pattern,
    type_expr::parse_type,
};

/// Result of trying to parse the next item inside a block: either a
/// finished statement (allocated into the arena) or an expression that
/// turned out to be the block's final value (no trailing `;`).
pub(crate) enum StatementOrFinal {
    Statement(StmtId),
    FinalExpression(ExprId),
}

/// Parse the next block element. Each block in a brace-delimited body
/// calls this until it sees `}`.
pub(crate) fn parse_statement_or_final_expr(
    parser: &mut Parser<'_>,
) -> Result<StatementOrFinal, ParseError> {
    let Some((token, span)) = parser.peek().cloned() else {
        return Err(ParseError::UnexpectedEof {
            expected: "statement or expression".to_owned(),
            span: parser.eof_span(),
        });
    };

    match token {
        Token::Let => Ok(StatementOrFinal::Statement(parse_let(parser, span)?)),
        Token::Constant => Ok(StatementOrFinal::Statement(parse_const(parser, span)?)),
        Token::Return => Ok(StatementOrFinal::Statement(parse_return(parser, span)?)),
        Token::Break => Ok(StatementOrFinal::Statement(parse_break(parser, span)?)),
        Token::Continue => {
            parser.advance();
            // optional trailing `;`
            let _ = parser.eat(&Token::Semi);
            Ok(StatementOrFinal::Statement(
                parser
                    .arena
                    .alloc_statement(Spanned::new(Stmt::Continue, span)),
            ))
        }
        Token::For => Ok(StatementOrFinal::Statement(parse_for(parser, span)?)),
        Token::While | Token::WhileQ => Ok(StatementOrFinal::Statement(parse_while(parser, span)?)),
        Token::Loop => Ok(StatementOrFinal::Statement(parse_loop(parser, span)?)),
        _ => parse_expression_or_final(parser),
    }
}

fn parse_let(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::Let, "`let`")?;
    let mutable = parser.eat(&Token::Mutable);

    let (name_token, _name_span) =
        parser
            .peek()
            .cloned()
            .ok_or_else(|| ParseError::UnexpectedEof {
                expected: "identifier after `let`".to_owned(),
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

    parser.expect(&Token::Assign, "`=`")?;
    let value = parse_expression(parser)?;
    let _ = parser.eat(&Token::Semi);

    let value_span = parser.arena.expression(value).span.clone();
    let span = head_span.start..value_span.end;
    Ok(parser.arena.alloc_statement(Spanned::new(
        Stmt::Let {
            name,
            mutable,
            type_annotation,
            value,
        },
        span,
    )))
}

fn parse_const(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::Constant, "`constant`")?;
    let (name_token, _) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "identifier after `constant`".to_owned(),
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

    parser.expect(&Token::Assign, "`=`")?;
    let value = parse_expression(parser)?;
    let _ = parser.eat(&Token::Semi);

    let value_span = parser.arena.expression(value).span.clone();
    let span = head_span.start..value_span.end;
    Ok(parser.arena.alloc_statement(Spanned::new(
        Stmt::Const {
            name,
            type_annotation,
            value,
        },
        span,
    )))
}

fn parse_return(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::Return, "`return`")?;
    let value = if matches!(
        parser.peek_token(),
        None | Some(Token::Semi | Token::RBrace | Token::Comma)
    ) {
        None
    } else {
        Some(parse_expression(parser)?)
    };
    let _ = parser.eat(&Token::Semi);
    let end = value.map_or(head_span.end, |id| parser.arena.expression(id).span.end);
    let span = head_span.start..end;
    Ok(parser
        .arena
        .alloc_statement(Spanned::new(Stmt::Return(value), span)))
}

fn parse_break(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::Break, "`break`")?;
    let value = if matches!(
        parser.peek_token(),
        None | Some(Token::Semi | Token::RBrace | Token::Comma)
    ) {
        None
    } else {
        Some(parse_expression(parser)?)
    };
    let _ = parser.eat(&Token::Semi);
    let end = value.map_or(head_span.end, |id| parser.arena.expression(id).span.end);
    let span = head_span.start..end;
    Ok(parser
        .arena
        .alloc_statement(Spanned::new(Stmt::Break(value), span)))
}

fn parse_for(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::For, "`for`")?;
    let variable = parse_pattern(parser)?;
    parser.expect(&Token::In, "`in`")?;
    let iterable = parse_expression(parser)?;
    let body_span = parser.current_span();
    let body = parse_block(parser, body_span)?;
    let span = head_span.start..parser.previous_token_end(head_span.end);
    Ok(parser.arena.alloc_statement(Spanned::new(
        Stmt::For {
            variable,
            iterable,
            body,
        },
        span,
    )))
}

fn parse_while(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    let head_token = parser.peek_token().cloned().expect("caller checked");
    let treat_unknown_as_false = matches!(head_token, Token::WhileQ);
    parser.advance();
    let condition = parse_expression(parser)?;
    let body_span = parser.current_span();
    let body = parse_block(parser, body_span)?;
    let span = head_span.start..parser.previous_token_end(head_span.end);
    Ok(parser.arena.alloc_statement(Spanned::new(
        Stmt::While {
            condition,
            body,
            treat_unknown_as_false,
        },
        span,
    )))
}

fn parse_loop(parser: &mut Parser<'_>, head_span: Span) -> Result<StmtId, ParseError> {
    parser.expect(&Token::Loop, "`loop`")?;
    let body_span = parser.current_span();
    let body = parse_block(parser, body_span)?;
    let span = head_span.start..parser.previous_token_end(head_span.end);
    Ok(parser
        .arena
        .alloc_statement(Spanned::new(Stmt::Loop(body), span)))
}

/// Parse what looks like an expression and decide whether it is a
/// statement (terminated by `;`), the block's final expression
/// (followed by `}`), or an assignment statement (`name = expr`).
fn parse_expression_or_final(parser: &mut Parser<'_>) -> Result<StatementOrFinal, ParseError> {
    let expr = parse_expression(parser)?;

    // Assignment: `target = value`. SPEC §5 — `let mutable` declares
    // mutable bindings; `=` (in statement position, after a parsed
    // expression) reassigns. Only identifier targets are accepted.
    if matches!(parser.peek_token(), Some(Token::Assign)) {
        return parse_assignment_after_target(parser, expr);
    }

    if parser.eat(&Token::Semi) {
        let span = parser.arena.expression(expr).span.clone();
        let stmt = parser
            .arena
            .alloc_statement(Spanned::new(Stmt::ExprStmt(expr), span));
        return Ok(StatementOrFinal::Statement(stmt));
    }

    // No semicolon: if next is `}`, this is the block's final expression.
    if matches!(parser.peek_token(), Some(Token::RBrace)) || parser.at_end() {
        return Ok(StatementOrFinal::FinalExpression(expr));
    }

    // No semicolon and more content follows — treat as expr-stmt anyway.
    let span = parser.arena.expression(expr).span.clone();
    let stmt = parser
        .arena
        .alloc_statement(Spanned::new(Stmt::ExprStmt(expr), span));
    Ok(StatementOrFinal::Statement(stmt))
}

/// Continuation of `parse_expression_or_final` once `=` is observed
/// after a parsed expression. Validates the lvalue, then builds an
/// `Stmt::Assign`.
fn parse_assignment_after_target(
    parser: &mut Parser<'_>,
    target_expr: ExprId,
) -> Result<StatementOrFinal, ParseError> {
    let target_span = parser.arena.expression(target_expr).span.clone();
    let target_node = parser.arena.expression(target_expr).node.clone();

    parser.expect(&Token::Assign, "`=`")?;
    let value = parse_expression(parser)?;
    let _ = parser.eat(&Token::Semi);

    let value_span = parser.arena.expression(value).span.clone();

    let Expr::Identifier(target) = target_node else {
        // Recovery: emit an error and degrade to an expr-stmt of the
        // RHS so parsing can continue.
        parser.record_error(ParseError::InvalidAssignmentTarget {
            description: "only identifier targets are assignable".to_owned(),
            span: target_span,
        });
        let stmt = parser
            .arena
            .alloc_statement(Spanned::new(Stmt::ExprStmt(value), value_span));
        return Ok(StatementOrFinal::Statement(stmt));
    };

    let span = target_span.start..value_span.end;
    Ok(StatementOrFinal::Statement(parser.arena.alloc_statement(
        Spanned::new(Stmt::Assign { target, value }, span),
    )))
}

/// Parse a brace-less function body (the `= expr` form). Used by item.rs.
pub(crate) fn parse_assignment_body(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    parser.expect(&Token::Assign, "`=`")?;
    parse_expression(parser)
}

/// Parse a top-level block (used by `function name(...) { body }`).
pub(crate) fn parse_top_block(parser: &mut Parser<'_>) -> Result<Block, ParseError> {
    let span = parser.current_span();
    parse_block(parser, span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_lexer::lex;

    fn parse_stmt(source: &str) -> (Parser<'static>, StmtId) {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let result = parse_statement_or_final_expr(&mut parser).expect("parse failed");
        let stmt = match result {
            StatementOrFinal::Statement(id) => id,
            StatementOrFinal::FinalExpression(_) => panic!("got final expression, expected stmt"),
        };
        (parser, stmt)
    }

    fn try_parse_stmt(source: &str) -> Result<(Parser<'static>, StatementOrFinal), ParseError> {
        let tokens: Vec<_> = lex(source).unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let result = parse_statement_or_final_expr(&mut parser)?;
        Ok((parser, result))
    }

    #[test]
    fn parses_let_immutable_no_annotation() {
        let (parser, id) = parse_stmt("let x = 5");
        match &parser.arena.statement(id).node {
            Stmt::Let {
                name,
                mutable,
                type_annotation,
                ..
            } => {
                assert_eq!(name, "x");
                assert!(!*mutable);
                assert!(type_annotation.is_none());
            }
            other => panic!("expected Let, got {other:?}"),
        }
    }

    #[test]
    fn parses_let_mut() {
        let (parser, id) = parse_stmt("let mutable count = 0");
        match &parser.arena.statement(id).node {
            Stmt::Let { mutable, .. } => assert!(*mutable),
            other => panic!("expected Let, got {other:?}"),
        }
    }

    #[test]
    fn parses_let_with_type_annotation() {
        let (parser, id) = parse_stmt("let x: Integer = 5");
        match &parser.arena.statement(id).node {
            Stmt::Let {
                type_annotation, ..
            } => assert!(type_annotation.is_some()),
            other => panic!("expected Let, got {other:?}"),
        }
    }

    #[test]
    fn parses_const() {
        let (parser, id) = parse_stmt("constant PI = 3");
        match &parser.arena.statement(id).node {
            Stmt::Const { name, .. } => assert_eq!(name, "PI"),
            other => panic!("expected Const, got {other:?}"),
        }
    }

    #[test]
    fn parses_return_with_value() {
        let (parser, id) = parse_stmt("return 5");
        match &parser.arena.statement(id).node {
            Stmt::Return(Some(_)) => {}
            other => panic!("expected Return(Some), got {other:?}"),
        }
    }

    #[test]
    fn parses_return_without_value() {
        let (parser, id) = parse_stmt("return");
        match &parser.arena.statement(id).node {
            Stmt::Return(None) => {}
            other => panic!("expected Return(None), got {other:?}"),
        }
    }

    #[test]
    fn parses_break_no_value() {
        let (parser, id) = parse_stmt("break");
        assert!(matches!(parser.arena.statement(id).node, Stmt::Break(None)));
    }

    #[test]
    fn parses_break_with_value() {
        let (parser, id) = parse_stmt("break 42");
        assert!(matches!(
            parser.arena.statement(id).node,
            Stmt::Break(Some(_))
        ));
    }

    #[test]
    fn parses_continue() {
        let (parser, id) = parse_stmt("continue");
        assert!(matches!(parser.arena.statement(id).node, Stmt::Continue));
    }

    #[test]
    fn parses_for_with_range() {
        let (parser, id) = parse_stmt("for i in 0..100 { }");
        assert!(matches!(parser.arena.statement(id).node, Stmt::For { .. }));
    }

    #[test]
    fn parses_for_with_tuple_destructuring() {
        let (parser, id) = parse_stmt("for (a, b) in pairs { }");
        match &parser.arena.statement(id).node {
            Stmt::For { variable, .. } => {
                use triet_syntax::Pattern;
                assert!(matches!(
                    parser.arena.pattern(*variable).node,
                    Pattern::Tuple(_)
                ));
            }
            other => panic!("expected For, got {other:?}"),
        }
    }

    #[test]
    fn parses_while() {
        let (parser, id) = parse_stmt("while running { }");
        match &parser.arena.statement(id).node {
            Stmt::While {
                treat_unknown_as_false,
                ..
            } => assert!(!*treat_unknown_as_false),
            other => panic!("expected While, got {other:?}"),
        }
    }

    #[test]
    fn parses_while_question_variant() {
        let (parser, id) = parse_stmt("while? maybe { }");
        match &parser.arena.statement(id).node {
            Stmt::While {
                treat_unknown_as_false,
                ..
            } => assert!(*treat_unknown_as_false),
            other => panic!("expected While, got {other:?}"),
        }
    }

    #[test]
    fn parses_loop() {
        let (parser, id) = parse_stmt("loop { }");
        assert!(matches!(parser.arena.statement(id).node, Stmt::Loop(_)));
    }

    #[test]
    fn final_expression_recognized_when_no_trailing_semi() {
        let tokens: Vec<_> = lex("42").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let result = parse_statement_or_final_expr(&mut parser).unwrap();
        assert!(matches!(result, StatementOrFinal::FinalExpression(_)));
    }

    #[test]
    fn expression_with_semi_is_statement() {
        let (_, result) = try_parse_stmt("42;").unwrap();
        assert!(matches!(result, StatementOrFinal::Statement(_)));
    }

    #[test]
    fn parses_assignment_with_identifier_target() {
        let (parser, id) = parse_stmt("count = 5");
        match &parser.arena.statement(id).node {
            Stmt::Assign { target, value } => {
                assert_eq!(target, "count");
                let val_expr = &parser.arena.expression(*value).node;
                assert!(matches!(val_expr, Expr::IntegerLiteral { value: 5, .. }));
            }
            other => panic!("expected Assign, got {other:?}"),
        }
    }

    #[test]
    fn parses_assignment_with_complex_rhs() {
        let (parser, id) = parse_stmt("count = count + 1");
        match &parser.arena.statement(id).node {
            Stmt::Assign { target, value } => {
                assert_eq!(target, "count");
                assert!(matches!(
                    parser.arena.expression(*value).node,
                    Expr::BinaryOp { .. }
                ));
            }
            other => panic!("expected Assign, got {other:?}"),
        }
    }

    #[test]
    fn assignment_with_trailing_semicolon_is_a_statement() {
        let (_, result) = try_parse_stmt("count = 5;").unwrap();
        assert!(matches!(result, StatementOrFinal::Statement(_)));
    }

    #[test]
    fn assignment_to_non_identifier_emits_error_and_recovers() {
        // Try `(a, b) = pair` — tuple LHS not allowed.
        let tokens: Vec<_> = lex("(a, b) = pair").unwrap();
        let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
        let mut parser = Parser::new(leaked);
        let _ = parse_statement_or_final_expr(&mut parser).unwrap();
        let (_, errors) = parser.finish();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::InvalidAssignmentTarget { .. })),
            "expected InvalidAssignmentTarget, got {errors:?}",
        );
    }
}
