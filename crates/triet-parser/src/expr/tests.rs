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
    assert!(matches!(
        parser.arena.expression(id).node,
        Expr::NullLiteral
    ));
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
        Expr::UnaryOp {
            operator: UnaryOperator::Negate,
            operand,
        } => {
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
    assert!(matches!(
        parser.arena.expression(left).node,
        Expr::UnaryOp { .. }
    ));
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
    assert_eq!(binary_op_of(&parser1, id1), binary_op_of(&parser2, id2),);
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
    assert!(matches!(
        errors[0],
        ParseError::ChainedNoChainOperator { .. }
    ));
}

#[test]
fn equality_chain_is_recorded_as_error() {
    let tokens: Vec<_> = lex("a == b == c").unwrap();
    let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
    let mut parser = Parser::new(leaked);
    let _ = parse_expression(&mut parser).unwrap();
    let (_, errors) = parser.finish();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, ParseError::ChainedNoChainOperator { .. }))
    );
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
        Expr::Range {
            inclusive: false,
            ..
        },
    ));
}

#[test]
fn inclusive_range_recognized() {
    let (parser, id) = parse_expr("0..=100");
    assert!(matches!(
        parser.arena.expression(id).node,
        Expr::Range {
            inclusive: true,
            ..
        },
    ));
}

#[test]
fn range_chain_is_recorded_as_error() {
    let tokens: Vec<_> = lex("a..b..c").unwrap();
    let leaked: &'static [_] = Box::leak(tokens.into_boxed_slice());
    let mut parser = Parser::new(leaked);
    let _ = parse_expression(&mut parser).unwrap();
    let (_, errors) = parser.finish();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, ParseError::ChainedNoChainOperator { .. }))
    );
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
    assert!(matches!(
        parser.arena.expression(id).node,
        Expr::ForceUnwrap(_)
    ));
}

#[test]
fn chained_method_calls() {
    // a.b().c.d()
    let (parser, id) = parse_expr("a.b().c.d()");
    // Outer is MethodCall on (a.b().c)
    assert!(matches!(
        parser.arena.expression(id).node,
        Expr::MethodCall { .. }
    ));
}

// === Unary forms ===

#[test]
fn unary_minus_bang_not_all_negate() {
    for source in ["-x", "!x", "not x"] {
        let (parser, id) = parse_expr(source);
        assert!(matches!(
            parser.arena.expression(id).node,
            Expr::UnaryOp {
                operator: UnaryOperator::Negate,
                ..
            },
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
    assert!(matches!(
        parser.arena.expression(id).node,
        Expr::IntegerLiteral { .. }
    ));
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
        Expr::If {
            else_branch,
            treat_unknown_as_false,
            ..
        } => {
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
        Expr::If {
            treat_unknown_as_false,
            ..
        } => assert!(*treat_unknown_as_false),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn parses_if_without_else() {
    let (parser, id) = parse_expr("if cond { 1 }");
    match &parser.arena.expression(id).node {
        Expr::If {
            else_branch: None, ..
        } => {}
        other => panic!("expected If(None), got {other:?}"),
    }
}

#[test]
fn parses_if_else_if_chain() {
    let (parser, id) = parse_expr("if a { 1 } else if b { 2 } else { 3 }");
    match &parser.arena.expression(id).node {
        Expr::If {
            else_branch: Some(_),
            ..
        } => {}
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
            assert!(matches!(
                &segments.parts[1],
                FStringPart::Interpolation { .. }
            ));
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
