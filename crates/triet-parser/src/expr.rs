//! Expression parser — Pratt-style with explicit binding-power table.
//!
//! Entry point: [`parse_expression`]. Supports the full SPEC §12.1
//! precedence/associativity ladder, including no-chain rules for
//! comparison, equality, and range operators. F-strings are parsed
//! inline (consuming the mode-aware token sequence emitted by the lexer).

use crate::{error::ParseError, parser::Parser, pattern::parse_pattern, type_expr::parse_type};
use triet_lexer::{IntLiteral as LexIntLiteral, Span, Token};
use triet_syntax::stmt::Block;
use triet_syntax::{
    BinaryOperator, Expr, ExprId, FStringPart, FStringSegments, LambdaParameter, MatchArm,
    NumericSuffix as AstSuffix, OutcomeArm, ReferenceForm, Spanned, TrileanValue, UnaryOperator,
};

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

        // Struct literal: `expr { field: value, ... }`. Tried
        // speculatively with backtracking so a failed attempt doesn't
        // consume the `{` (which may belong to a block/control-flow).
        if matches!(parser.peek_token(), Some(Token::LBrace)) {
            // Extract the type name before the speculative parse
            // (which borrows `parser` mutably).
            let struct_name = match &parser.arena.expression(lhs).node {
                Expr::Identifier { name: n } => Some(n.clone()),
                _ => None,
            };
            if let Some(name) = struct_name
                && let Some(fields) = try_parse_struct_literal(parser)
            {
                let lhs_span = arena_span(parser, lhs);
                let end = parser.previous_token_end(lhs_span.end);
                let span = lhs_span.start..end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::StructLiteral {
                        struct_name: name,
                        fields,
                    },
                    span,
                ));
                continue;
            }
            // Not an identifier LHS or struct parse failed — `{` is
            // not part of this expression. Stop the Pratt loop so the
            // `{` stays available for the enclosing context (e.g.,
            // block start after `while? cond`).
            break;
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
            && prev == class
        {
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

        // Capture the no-chain class before the match below consumes `op_kind`
        // (the `Plain` arm moves the non-`Copy` `BinaryOperator` out of it).
        let next_no_chain_class = op_kind.no_chain_class();

        // Special structural operators that don't fit the BinaryOp shape.
        match op_kind {
            BinaryOpKind::Range { inclusive } => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::Range {
                        start: lhs,
                        end: rhs,
                        inclusive,
                    },
                    span,
                ));
            }
            BinaryOpKind::Elvis => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::ElvisOp {
                        object: lhs,
                        default: rhs,
                    },
                    span,
                ));
            }
            BinaryOpKind::Plain(operator) => {
                let rhs = parse_expression_bp(parser, r_bp)?;
                let span = arena_span(parser, lhs).start..arena_span(parser, rhs).end;
                lhs = parser.arena.alloc_expression(Spanned::new(
                    Expr::BinaryOp {
                        operator,
                        left: lhs,
                        right: rhs,
                    },
                    span,
                ));
            }
        }

        last_no_chain_class = next_no_chain_class;
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
                Expr::IntegerLiteral {
                    value,
                    suffix: suffix.map(convert_suffix),
                },
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
                .alloc_expression(Spanned::new(Expr::StringLiteral { value: text }, span)))
        }
        Token::True => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral {
                    value: TrileanValue::True,
                },
                span,
            )))
        }
        Token::False => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral {
                    value: TrileanValue::False,
                },
                span,
            )))
        }
        Token::Unknown => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TrileanLiteral {
                    value: TrileanValue::Unknown,
                },
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
                .alloc_expression(Spanned::new(Expr::Identifier { name }, span)))
        }
        // ADR-0061 T2.5: `self` as a primary expression (receiver inside an
        // `implement` method body). Lexed as `SelfKw`; here it becomes a
        // plain identifier named "self" — resolution is T3/T4. Import-path
        // `self` (`use self::X`) uses a separate path (parse_use_path)
        // and is unaffected.
        Token::SelfKw => {
            parser.advance();
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::Identifier {
                    name: "self".to_owned(),
                },
                span,
            )))
        }
        Token::FStringStart => parse_f_string(parser),
        Token::LParen => parse_paren_or_tuple(parser, span),
        Token::LBrace => parse_block_expression(parser, span),
        Token::If | Token::IfQ => parse_if_expression(parser),
        Token::Match => parse_match_expression(parser),
        Token::Return => parse_return_expression(parser, span),
        Token::Pipe | Token::OrOr => parse_lambda(parser),
        // Unary prefix operators
        Token::Minus | Token::Bang | Token::Not => parse_unary(parser),
        // v0.9.x.atomic.7b: borrow expression prefix per ADR-0031 §3.
        // `&+`/`&0`/`&-` at expression position parses as Expr::Borrow
        // with operand restricted to IDENT + field-access chain per §2.
        Token::AmpersandPlus | Token::AmpersandZero | Token::AmpersandMinus => {
            parse_borrow(parser, span)
        }
        // Outcome constructors (v0.7.4.3-error per ADR-0020 §2):
        // `~+ value` (Positive), `~- error` (Negative), `~0` (Zero).
        Token::TildePlus => parse_outcome_constructor(parser, OutcomeArm::Positive, span),
        Token::TildeMinus => parse_outcome_constructor(parser, OutcomeArm::Negative, span),
        Token::TildeZero => parse_outcome_zero(parser, span),
        // ADR-0069: `mint Cap` capability-token constructor.
        Token::Mint => parse_mint(parser, span),
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
        Expr::UnaryOp {
            operator: UnaryOperator::Negate,
            operand,
        },
        span,
    )))
}

/// v0.9.x.atomic.7b: parse borrow expression per ADR-0031.
///
/// Consumes `&+` / `&0` / `&-` (with optional `mutable` keyword for
/// `&+` and `&0`), then parses the operand restricted to IDENT
/// followed by zero or more `.IDENT` field-access steps per §2 v0.9
/// scope. Function calls, method calls, array index `[i]`, compound
/// binary, and nested borrow are explicitly NOT accepted as operand
/// (deferred to ADR-0031 §10.3 backlog).
fn parse_borrow(parser: &mut Parser<'_>, op_span: Span) -> Result<ExprId, ParseError> {
    let form = parse_borrow_form(parser)?;
    let operand = parse_borrow_operand(parser)?;
    let span = op_span.start..arena_span(parser, operand).end;
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Borrow { form, operand }, span)))
}

/// Consume the `&FORM` token sequence per ADR-0031 §1 grammar:
/// - `&+` → `StrongFrozen`; `&+ mutable` → `StrongMutable`
/// - `&0` → `BorrowReadOnly`; `&0 mutable` → `BorrowExclusiveMutable`
/// - `&-` → `WeakObserver` (no `mutable` permitted — parse error if
///   `mutable` follows per ADR-0022 §2 row 5 immutability).
fn parse_borrow_form(parser: &mut Parser<'_>) -> Result<ReferenceForm, ParseError> {
    let (token, _span) = parser.peek().cloned().expect("caller checked peek");
    match token {
        Token::AmpersandPlus => {
            parser.advance();
            Ok(if parser.eat(&Token::Mutable) {
                ReferenceForm::StrongMutable
            } else {
                ReferenceForm::StrongFrozen
            })
        }
        Token::AmpersandZero => {
            parser.advance();
            Ok(if parser.eat(&Token::Mutable) {
                ReferenceForm::BorrowExclusiveMutable
            } else {
                ReferenceForm::BorrowReadOnly
            })
        }
        Token::AmpersandMinus => {
            parser.advance();
            Ok(ReferenceForm::WeakObserver)
        }
        other => {
            unreachable!("parse_borrow_form called with non-reference token: {other:?}")
        }
    }
}

/// Parse the operand of a borrow expression. Operand grammar (ADR-0031
/// §2 v0.9 scope): `IDENT ('.' IDENT)*`. The result is an
/// `Expr::Identifier` (bare) or a chain of `Expr::FieldAccess`.
fn parse_borrow_operand(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "identifier".to_owned(),
            span: parser.eof_span(),
        })?;
    let Token::Identifier(name) = token else {
        return Err(ParseError::UnexpectedToken {
            expected: "identifier".to_owned(),
            found: format!("{token:?}"),
            span,
        });
    };
    parser.advance();
    let mut node_id = parser
        .arena
        .alloc_expression(Spanned::new(Expr::Identifier { name }, span));
    while let Some((Token::Dot, _)) = parser.peek() {
        parser.advance(); // consume `.`
        let (field_tok, field_span) =
            parser
                .peek()
                .cloned()
                .ok_or_else(|| ParseError::UnexpectedEof {
                    expected: "field name".to_owned(),
                    span: parser.eof_span(),
                })?;
        let Token::Identifier(field_name) = field_tok else {
            return Err(ParseError::UnexpectedToken {
                expected: "field name".to_owned(),
                found: format!("{field_tok:?}"),
                span: field_span,
            });
        };
        parser.advance();
        let outer_span = arena_span(parser, node_id).start..field_span.end;
        node_id = parser.arena.alloc_expression(Spanned::new(
            Expr::FieldAccess {
                object: node_id,
                field: field_name,
            },
            outer_span,
        ));
    }
    Ok(node_id)
}

/// Parse `return expr` as an expression for use inside `~->` and other
/// arm-handler bodies where `return` is valid in expression position.
fn parse_return_expression(parser: &mut Parser<'_>, ret_span: Span) -> Result<ExprId, ParseError> {
    parser.advance(); // consume `return` token
    let value = parse_expression_bp(parser, 0)?;
    let span = ret_span.start..arena_span(parser, value).end;
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Return { value: Some(value) }, span)))
}

// ============================================================================
// Outcome constructors (v0.7.4.3-error per ADR-0020 §2)
// ============================================================================

/// Parse `~+ expr` (Positive arm) or `~- expr` (Negative arm). The
/// compound token (`TildePlus`/`TildeMinus`) is the current peek; this
/// function consumes it and parses the following payload expression at
/// unary-right binding power so that `~+ -1` parses as
/// `Positive(Negate(1))` and `~- IoError::NotFound(path)` parses as
/// `Negative(Call(...))` cleanly.
///
/// Style guide mandates space between the prefix and payload, but the
/// lexer emits them as separate tokens regardless of whitespace —
/// `dao fmt` enforces the space at format-time.
fn parse_outcome_constructor(
    parser: &mut Parser<'_>,
    arm: OutcomeArm,
    op_span: Span,
) -> Result<ExprId, ParseError> {
    parser.advance(); // consume TildePlus or TildeMinus
    let payload = parse_expression_bp(parser, UNARY_RIGHT_BP)?;
    let span = op_span.start..arena_span(parser, payload).end;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::OutcomeConstructor {
            arm,
            payload: Some(payload),
        },
        span,
    )))
}

/// Parse `~0` (Zero arm — null state). No payload follows. Only valid
/// in T?~E contexts at typecheck time; parse accepts unconditionally
/// per refuse-over-guess (E1025 fires at typecheck if used in T~E).
///
/// Per ADR-0020 §10, `~0` is also the canonical `Trit::Zero` literal
/// for `T?` (deprecates `null` keyword). The parser produces the same
/// AST node either way; semantic interpretation is typecheck's job.
fn parse_outcome_zero(parser: &mut Parser<'_>, span: Span) -> Result<ExprId, ParseError> {
    parser.advance(); // consume TildeZero
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::OutcomeConstructor {
            arm: OutcomeArm::Zero,
            payload: None,
        },
        span,
    )))
}

/// Parse `mint Cap` → `Expr::Mint` (ADR-0069). The operand is restricted to a
/// bare capability name (Identifier), NOT an arbitrary expression — a token is
/// minted from a `capability` declaration, not computed.
fn parse_mint(parser: &mut Parser<'_>, op_span: Span) -> Result<ExprId, ParseError> {
    parser.advance(); // consume `mint`
    let (token, name_span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
            expected: "capability name".to_owned(),
            span: parser.eof_span(),
        })?;
    let Token::Identifier(capability_name) = token else {
        return Err(ParseError::UnexpectedToken {
            expected: "capability name".to_owned(),
            found: format!("{token:?}"),
            span: name_span,
        });
    };
    parser.advance();
    let span = op_span.start..name_span.end;
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Mint { capability_name }, span)))
}

fn parse_paren_or_tuple(parser: &mut Parser<'_>, open_span: Span) -> Result<ExprId, ParseError> {
    parser.expect(&Token::LParen, "`(`")?;

    if matches!(parser.peek_token(), Some(Token::RParen)) {
        // Empty tuple `()` — useful as Unit literal.
        let close = parser.expect(&Token::RParen, "`)`")?;
        let span = open_span.start..close.end;
        return Ok(parser.arena.alloc_expression(Spanned::new(
            Expr::Tuple {
                elements: Vec::new(),
            },
            span,
        )));
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
        .alloc_expression(Spanned::new(Expr::Tuple { elements }, span)))
}

pub(crate) fn parse_block_expression(
    parser: &mut Parser<'_>,
    open_span: Span,
) -> Result<ExprId, ParseError> {
    let block = parse_block(parser, open_span.clone())?;
    // Reconstruct span: start at `{`, end at `}` (the parse_block helper
    // returns the closing-brace span via its own bookkeeping below).
    let span = open_span.start..parser.previous_token_end(open_span.end);
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::Block {
            statements: block.statements,
            final_expr: block.final_expression,
        },
        span,
    )))
}

/// Parse `{ stmts? final_expr? }` into a `Block`. Used both as block
/// expression and as function/match-arm body.
pub(crate) fn parse_block(parser: &mut Parser<'_>, _open_span: Span) -> Result<Block, ParseError> {
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
    // `if?` (IfQ) parses to the same shape as `if` with this flag set; lowering
    // it to a 3-way trit branch is triet-lower's job (SPEC grammar §if_expr).
    let treat_unknown_as_false = matches!(head_token, Token::IfQ);
    parser.advance();

    let condition = parse_expression(parser)?;

    let then_open = parser.current_span();
    let then_branch = parse_block_expression(parser, then_open)?;

    let else_branch = if parser.eat(&Token::Else) {
        if matches!(parser.peek_token(), Some(Token::If | Token::IfQ)) {
            // `else if` chain — the inner `if` expression is the else branch.
            Some(parse_if_expression(parser)?)
        } else {
            let else_open = parser.current_span();
            Some(parse_block_expression(parser, else_open)?)
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
    Ok(parser
        .arena
        .alloc_expression(Spanned::new(Expr::Match { scrutinee, arms }, span)))
}

fn parse_lambda(parser: &mut Parser<'_>) -> Result<ExprId, ParseError> {
    let (head_token, head_span) = parser.peek().cloned().expect("caller checked");
    parser.advance();

    let parameters = match head_token {
        Token::OrOr => Vec::new(), // `||` — no parameters
        Token::Pipe => {
            let mut parameters = Vec::new();
            if !matches!(parser.peek_token(), Some(Token::Pipe)) {
                loop {
                    let (name_token, _name_span) =
                        parser
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
                    parameters.push(LambdaParameter {
                        name,
                        type_annotation,
                    });
                    if !parser.eat(&Token::Comma) {
                        break;
                    }
                }
            }
            parser.expect(&Token::Pipe, "`|`")?;
            parameters
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
            return_type_annotation: return_type,
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
        | Token::BangBang
        | Token::TildePlusGt
        | Token::TildeZeroGt
        | Token::TildeMinusGt
        | Token::QuestionPlusGt
        | Token::QuestionMinusGt => Some(POSTFIX_LEFT_BP),
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
                .alloc_expression(Spanned::new(Expr::ForceUnwrap { operand: lhs }, span)))
        }
        Token::TildePlusGt => parse_outcome_arm_handler(parser, lhs, OutcomeArm::Positive),
        Token::TildeZeroGt => parse_outcome_arm_handler(parser, lhs, OutcomeArm::Zero),
        Token::TildeMinusGt => parse_outcome_arm_handler(parser, lhs, OutcomeArm::Negative),
        Token::QuestionPlusGt => parse_nullable_postfix(parser, lhs, false),
        Token::QuestionMinusGt => parse_nullable_postfix(parser, lhs, true),
        Token::LBracket => {
            // Subscript / index access — not in v0.1 SPEC; treat as error
            // for now to keep semantics tight.
            let span = parser.current_span();
            Err(ParseError::UnexpectedToken {
                expected: "operator".to_owned(),
                found: "`[` (indexing is not yet supported)".to_owned(),
                span,
            })
        }
        _ => unreachable!("caller filtered postfix tokens"),
    }
}

/// Parse ternary arm handler `inner ~+> |v| body` / `expr ~0> body` / `expr ~-> |e| body`
/// per ADR-0020 §3 (v0.7.4.3-error.4).
///
/// `~+>` and `~->` require a `|name|` capture (or `|_|` discard).
/// `~0>` takes no capture (null arm carries no payload).
fn parse_outcome_arm_handler(
    parser: &mut Parser<'_>,
    inner: ExprId,
    arm: OutcomeArm,
) -> Result<ExprId, ParseError> {
    let token_name = expected_arm_token(arm);
    // Consume the 3-char compound token.
    let expect_token = match arm {
        OutcomeArm::Positive => &Token::TildePlusGt,
        OutcomeArm::Zero => &Token::TildeZeroGt,
        OutcomeArm::Negative => &Token::TildeMinusGt,
    };
    parser.expect(expect_token, token_name)?;

    // `~0>` has no capture; `~+>` / `~->` require |name| or |_|.
    let capture_name = if arm == OutcomeArm::Zero {
        None
    } else {
        parse_outcome_capture(parser, token_name)?
    };

    let body = parse_expression_bp(parser, 0)?;
    let span = arena_span(parser, inner).start..arena_span(parser, body).end;
    Ok(parser.arena.alloc_expression(Spanned::new(
        Expr::OutcomeArmHandler {
            inner,
            arm,
            capture_name,
            body,
        },
        span,
    )))
}

/// Parse `inner ?+> |bind| body` (map/flatMap) or `inner ?-> |bind| body`
/// (forbidden error-arm) — identical grammar (ADR-0039 §1/§3), so ONE
/// helper parses the form and wraps it in the node the token selects:
/// `?+>` → `NullableMap` (valid); `?->` → `NullableErrorArm` (typecheck
/// rejects with E1046). `|_|` discard stored as the empty `bind_var`.
fn parse_nullable_postfix(
    parser: &mut Parser<'_>,
    inner: ExprId,
    error_arm: bool,
) -> Result<ExprId, ParseError> {
    let (token, label) = if error_arm {
        (&Token::QuestionMinusGt, "`?->`")
    } else {
        (&Token::QuestionPlusGt, "`?+>`")
    };
    parser.expect(token, label)?;
    let bind_var = parse_outcome_capture(parser, label)?.unwrap_or_default();
    let body = parse_expression_bp(parser, 0)?;
    let span = arena_span(parser, inner).start..arena_span(parser, body).end;
    let node = if error_arm {
        Expr::NullableErrorArm {
            inner,
            bind_var,
            body,
        }
    } else {
        Expr::NullableMap {
            inner,
            bind_var,
            body,
        }
    };
    Ok(parser.arena.alloc_expression(Spanned::new(node, span)))
}

/// Parse `|name|` or `|_|` capture form used by `~?` (legacy), `~+>`, and `~->`.
/// Returns `Some(name)` for `|name|`, `None` for `|_|`.
fn parse_outcome_capture(
    parser: &mut Parser<'_>,
    op_label: &str,
) -> Result<Option<String>, ParseError> {
    parser.expect(
        &Token::Pipe,
        &format!("`|` (closure capture for {op_label})"),
    )?;
    let (capture_token, capture_span) =
        parser
            .peek()
            .cloned()
            .ok_or_else(|| ParseError::UnexpectedEof {
                expected: "binding name or `_`".to_owned(),
                span: parser.eof_span(),
            })?;
    let capture_name = match capture_token {
        Token::Identifier(name) => {
            parser.advance();
            Some(name)
        }
        Token::Underscore => {
            parser.advance();
            None
        }
        other => {
            return Err(ParseError::UnexpectedToken {
                expected: "binding name or `_` after `|`".to_owned(),
                found: format!("{other:?}"),
                span: capture_span,
            });
        }
    };
    parser.expect(&Token::Pipe, "closing `|`")?;
    Ok(capture_name)
}

/// Human-readable label for an arm token.
const fn expected_arm_token(arm: OutcomeArm) -> &'static str {
    match arm {
        OutcomeArm::Positive => "`~+>`",
        OutcomeArm::Zero => "`~0>`",
        OutcomeArm::Negative => "`~->`",
    }
}

fn parse_dot_postfix(
    parser: &mut Parser<'_>,
    receiver: ExprId,
    safe: bool,
) -> Result<ExprId, ParseError> {
    parser.advance(); // consume `.` or `?.`

    let (token, span) = parser
        .peek()
        .cloned()
        .ok_or_else(|| ParseError::UnexpectedEof {
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
        Token::IntegerLiteral(LexIntLiteral {
            value,
            suffix: None,
        }) if !safe => {
            // Tuple index: `pair.0`. Only base-10 unsuffixed integers
            // make sense as indices.
            parser.advance();
            let index = usize::try_from(value).map_err(|_| ParseError::InvalidLiteral {
                message: "tuple index must be a non-negative integer".to_owned(),
                span: span.clone(),
            })?;
            let full_span = arena_span(parser, receiver).start..span.end;
            Ok(parser.arena.alloc_expression(Spanned::new(
                Expr::TupleIndex {
                    tuple: receiver,
                    index,
                },
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
                parts.push(FStringPart::Interpolation {
                    expression,
                    format_spec,
                });
            }
            Token::FStringEnd => {
                parser.advance();
                let end_span = span.end;
                let segments = FStringSegments { parts };
                let full_span = start_span.start..end_span;
                return Ok(parser.arena.alloc_expression(Spanned::new(
                    Expr::FStringLiteral {
                        segments: segments.parts,
                    },
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
#[allow(clippy::doc_markdown, clippy::doc_lazy_continuation)]
mod tests;

// ============================================================================
// Operator classification
// ============================================================================

/// What kind of binary expression a token introduces.
// Not `Copy`: the schema-generated `BinaryOperator` carried by `Plain` is not
// `Copy`, so neither can this be.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BinaryOpKind {
    Plain(BinaryOperator),
    Range { inclusive: bool },
    Elvis,
}

impl BinaryOpKind {
    const fn binding_power(&self) -> (u8, u8) {
        match self {
            // Right-associative implication — lowest precedence
            Self::Plain(BinaryOperator::LukImplies | BinaryOperator::KleeneImplies) => (1, 0),
            // Left-associative biconditional
            Self::Plain(BinaryOperator::LukIff | BinaryOperator::KleeneIff) => (2, 3),
            // Left-associative or — loosest of the boolean trio
            Self::Plain(BinaryOperator::LukOr) => (4, 5),
            // Left-associative xor — between or and and (per SPEC §12.1
            // level 5; AND-tighter-than-XOR-tighter-than-OR)
            Self::Plain(BinaryOperator::LukXor | BinaryOperator::KleeneXor) => (6, 7),
            // Left-associative and — tightest of the boolean trio
            Self::Plain(BinaryOperator::LukAnd) => (8, 9),
            // Equality (no chain — the no-chain rule is enforced by the
            // class check; the binding-power table itself is consistent
            // with left-associative).
            Self::Plain(BinaryOperator::Eq | BinaryOperator::Ne) => (10, 11),
            // Comparison
            Self::Plain(
                BinaryOperator::Lt | BinaryOperator::Le | BinaryOperator::Gt | BinaryOperator::Ge,
            ) => (12, 13),
            // Elvis — right-associative
            Self::Elvis => (15, 14),
            // Range — no chain
            Self::Range { .. } => (16, 17),
            // Additive
            Self::Plain(BinaryOperator::Add | BinaryOperator::Sub) => (18, 19),
            // Multiplicative
            Self::Plain(BinaryOperator::Mul | BinaryOperator::Div | BinaryOperator::Mod) => {
                (20, 21)
            }
            // Power — right-associative, higher than unary (handled by
            // unary right_bp = 23 so prefix `-` binds looser than `**`)
            Self::Plain(BinaryOperator::Pow) => (26, 25),
        }
    }

    const fn no_chain_class(&self) -> Option<NoChainClass> {
        match self {
            Self::Plain(BinaryOperator::Eq | BinaryOperator::Ne) => Some(NoChainClass::Equality),
            Self::Plain(
                BinaryOperator::Lt | BinaryOperator::Le | BinaryOperator::Gt | BinaryOperator::Ge,
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
        Token::Minus => BinaryOpKind::Plain(BinaryOperator::Sub),
        Token::Star => BinaryOpKind::Plain(BinaryOperator::Mul),
        Token::Slash => BinaryOpKind::Plain(BinaryOperator::Div),
        Token::PercentPercent => BinaryOpKind::Plain(BinaryOperator::Mod),
        Token::StarStar => BinaryOpKind::Plain(BinaryOperator::Pow),

        Token::EqEq => BinaryOpKind::Plain(BinaryOperator::Eq),
        Token::NotEq => BinaryOpKind::Plain(BinaryOperator::Ne),
        Token::Lt => BinaryOpKind::Plain(BinaryOperator::Lt),
        Token::LtEq => BinaryOpKind::Plain(BinaryOperator::Le),
        Token::Gt => BinaryOpKind::Plain(BinaryOperator::Gt),
        Token::GtEq => BinaryOpKind::Plain(BinaryOperator::Ge),

        Token::AndAnd | Token::And => BinaryOpKind::Plain(BinaryOperator::LukAnd),
        Token::OrOr | Token::Or => BinaryOpKind::Plain(BinaryOperator::LukOr),

        Token::Caret | Token::Xor => BinaryOpKind::Plain(BinaryOperator::LukXor),
        Token::TildeCaret | Token::KleeneXor => BinaryOpKind::Plain(BinaryOperator::KleeneXor),
        Token::LtEqGt | Token::Iff => BinaryOpKind::Plain(BinaryOperator::LukIff),
        Token::LtTildeGt | Token::KleeneIff => BinaryOpKind::Plain(BinaryOperator::KleeneIff),
        Token::FatArrow | Token::Implies => BinaryOpKind::Plain(BinaryOperator::LukImplies),
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

/// Parse `{ field: expr, ... }` inside a struct literal expression.
/// Speculatively try to parse `{ field: value, ... }` as a struct
/// literal body. On success returns the field list and leaves the
/// cursor past `}`. On failure restores the cursor to where it was
/// before the attempt — the `{` is NOT consumed, so the caller (or
/// subsequent parsing) can treat it as a block/control-flow brace.
fn try_parse_struct_literal(parser: &mut Parser<'_>) -> Option<Vec<(String, ExprId)>> {
    let saved = parser.save_position();

    // Must start with `{`.
    if !matches!(parser.peek_token(), Some(Token::LBrace)) {
        return None;
    }
    parser.advance(); // consume `{`

    let mut fields = Vec::new();

    // Empty body `{}` → not a struct literal (it's a block).
    if parser.eat(&Token::RBrace) {
        parser.restore_position(saved);
        return None;
    }

    loop {
        // Field name must be an identifier.
        let name = if let Some(Token::Identifier(n)) = parser.peek_token().cloned() {
            parser.advance();
            n
        } else {
            parser.restore_position(saved);
            return None;
        };

        // Must have `:` after field name.
        if !parser.eat(&Token::Colon) {
            parser.restore_position(saved);
            return None;
        }

        // Parse the field value expression.
        let Ok(value) = parse_expression(parser) else {
            parser.restore_position(saved);
            return None;
        };
        fields.push((name, value));

        // Comma → more fields. `}` → done.
        if parser.eat(&Token::Comma) {
            if matches!(parser.peek_token(), Some(Token::RBrace)) {
                parser.advance();
                break;
            }
            continue;
        }
        if parser.eat(&Token::RBrace) {
            break;
        }

        // Unexpected token after field value.
        parser.restore_position(saved);
        return None;
    }

    Some(fields)
}

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
