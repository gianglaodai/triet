//! Runtime evaluation helpers — arithmetic, comparison, pattern matching.
//!
//! Pure functions with no `&mut Interpreter` dependency. Called by
//! `Interpreter` methods; also callable directly for built-in dispatch.

use triet_core::{Integer, Long, Trit, Tryte};
use triet_logic::Trilean;
use triet_syntax::{
    Arena, BinaryOperator, LiteralPattern, NumericSuffix, Pattern, PatternId, Span, TrileanValue,
};

use crate::error::RuntimeError;
use crate::value::Value;

// ====================================================================
// Free helpers
// ====================================================================

pub(super) fn integer_literal_value(
    value: i128,
    suffix: Option<NumericSuffix>,
    span: &Span,
) -> Result<Value, RuntimeError> {
    match suffix {
        Some(NumericSuffix::Trit) => match Trit::from_i8(value as i8) {
            Some(trit) if i128::from(trit.to_i8()) == value => Ok(Value::Trit(trit)),
            _ => Err(RuntimeError::Panic {
                message: format!("integer literal {value} out of Trit range"),
                span: span.clone(),
            }),
        },
        Some(NumericSuffix::Tryte) => {
            Tryte::new(value as i16)
                .map(Value::Tryte)
                .ok_or_else(|| RuntimeError::Panic {
                    message: format!("integer literal {value} out of Tryte range"),
                    span: span.clone(),
                })
        }
        Some(NumericSuffix::Long) => {
            // Long range strictly contains i128; the lexer produces an
            // i128 value, so this is always lossless.
            Ok(Value::Long(Long::from_i128(value)))
        }
        Some(NumericSuffix::Integer) | None => Integer::new(value as i64)
            .map(Value::Integer)
            .ok_or_else(|| RuntimeError::Panic {
                message: format!("integer literal {value} out of Integer range"),
                span: span.clone(),
            }),
    }
}

pub(super) fn execute_binary(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    match operator {
        BinaryOperator::Add
        | BinaryOperator::Subtract
        | BinaryOperator::Multiply
        | BinaryOperator::Divide
        | BinaryOperator::Modulo
        | BinaryOperator::Power => execute_arithmetic(operator, left, right, span),
        BinaryOperator::Equal => Ok(Value::Trilean(if left == right {
            Trilean::True
        } else {
            Trilean::False
        })),
        BinaryOperator::NotEqual => Ok(Value::Trilean(if left == right {
            Trilean::False
        } else {
            Trilean::True
        })),
        BinaryOperator::LessThan
        | BinaryOperator::LessEqual
        | BinaryOperator::GreaterThan
        | BinaryOperator::GreaterEqual => execute_comparison(operator, left, right, span),
        BinaryOperator::And => trilean_op(left, right, span, triet_logic::Trilean::and),
        BinaryOperator::Or => trilean_op(left, right, span, triet_logic::Trilean::or),
        BinaryOperator::Xor => trilean_op(left, right, span, triet_logic::Trilean::xor),
        BinaryOperator::Iff => trilean_op(left, right, span, triet_logic::Trilean::iff),
        BinaryOperator::Implies => trilean_op(left, right, span, triet_logic::Trilean::implies),
        BinaryOperator::KleeneXor => {
            trilean_op(left, right, span, triet_logic::Trilean::kleene_xor)
        }
        BinaryOperator::KleeneIff => {
            trilean_op(left, right, span, triet_logic::Trilean::kleene_iff)
        }
        BinaryOperator::KleeneImplies => {
            trilean_op(left, right, span, triet_logic::Trilean::kleene_implies)
        }
    }
}

pub(super) fn execute_arithmetic(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Tryte(a), Value::Tryte(b)) => {
            apply_tryte_arithmetic(operator, a, b, span).map(Value::Tryte)
        }
        (Value::Integer(a), Value::Integer(b)) => {
            apply_integer_arithmetic(operator, a, b, span).map(Value::Integer)
        }
        (Value::Long(a), Value::Long(b)) => {
            apply_long_arithmetic(operator, a, b, span).map(Value::Long)
        }
        (left, right) => Err(RuntimeError::TypeError {
            message: format!("arithmetic on incompatible types {left} and {right}"),
            span,
        }),
    }
}

pub(super) fn apply_long_arithmetic(
    operator: BinaryOperator,
    a: Long,
    b: Long,
    span: Span,
) -> Result<Long, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => long_power(a, b),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Long ({a} {operator:?} {b})"),
        span,
    })
}

pub(super) fn long_power(base: Long, exponent: Long) -> Long {
    let mut result = Long::ONE;
    let count = exponent.try_to_i128().unwrap_or(0);
    if count < 0 {
        return Long::ZERO;
    }
    for _ in 0..count {
        result = result * base;
    }
    result
}

pub(super) fn apply_tryte_arithmetic(
    operator: BinaryOperator,
    a: Tryte,
    b: Tryte,
    span: Span,
) -> Result<Tryte, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => panic!("** on Tryte not supported in v0.1"),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Tryte ({a} {operator:?} {b})"),
        span,
    })
}

pub(super) fn apply_integer_arithmetic(
    operator: BinaryOperator,
    a: Integer,
    b: Integer,
    span: Span,
) -> Result<Integer, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => integer_power(a, b),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Integer ({a} {operator:?} {b})"),
        span,
    })
}

pub(super) fn integer_power(base: Integer, exponent: Integer) -> Integer {
    let mut result = Integer::ONE;
    let mut count = exponent.to_i64();
    if count < 0 {
        return Integer::ZERO;
    }
    while count > 0 {
        result = result * base;
        count -= 1;
    }
    result
}

pub(super) fn execute_comparison(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    let ordering = match (&left, &right) {
        (Value::Tryte(a), Value::Tryte(b)) => a.to_i64().cmp(&b.to_i64()),
        (Value::Integer(a), Value::Integer(b)) => a.to_i64().cmp(&b.to_i64()),
        (Value::Long(a), Value::Long(b)) => a.cmp(b),
        _ => {
            return Err(RuntimeError::TypeError {
                message: format!("comparison on incompatible types {left} and {right}"),
                span,
            });
        }
    };
    let result = match operator {
        BinaryOperator::LessThan => ordering.is_lt(),
        BinaryOperator::LessEqual => ordering.is_le(),
        BinaryOperator::GreaterThan => ordering.is_gt(),
        BinaryOperator::GreaterEqual => ordering.is_ge(),
        _ => unreachable!(),
    };
    Ok(Value::Trilean(if result {
        Trilean::True
    } else {
        Trilean::False
    }))
}

pub(super) fn trilean_op(
    left: Value,
    right: Value,
    span: Span,
    op: impl Fn(Trilean, Trilean) -> Trilean,
) -> Result<Value, RuntimeError> {
    let (Value::Trilean(a), Value::Trilean(b)) = (left, right) else {
        return Err(RuntimeError::TypeError {
            message: "logic operator requires Trilean operands".to_owned(),
            span,
        });
    };
    Ok(Value::Trilean(op(a, b)))
}

pub(super) fn binary_int_method(
    receiver: &Integer,
    arguments: Vec<Value>,
    op: impl Fn(Integer, Integer) -> Integer,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Integer(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects an Integer argument".to_owned(),
            span,
        });
    };
    Ok(Value::Integer(op(*receiver, other)))
}

pub(super) fn binary_int_try_method(
    receiver: &Integer,
    arguments: Vec<Value>,
    op: impl Fn(Integer, Integer) -> Option<Integer>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Integer(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects an Integer argument".to_owned(),
            span,
        });
    };
    Ok(match op(*receiver, other) {
        Some(integer) => Value::Integer(integer),
        None => Value::Null,
    })
}

pub(super) fn binary_long_method(
    receiver: &Long,
    arguments: Vec<Value>,
    op: impl Fn(Long, Long) -> Long,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Long(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects a Long argument".to_owned(),
            span,
        });
    };
    Ok(Value::Long(op(*receiver, other)))
}

/// Advance any iterable `Value` by one step. Returns `Ok(None)` when the
/// iterator is exhausted, `Ok(Some(elem))` for the next element, or an
/// error if the value isn't iterable.
///
/// Internal counterpart to a future user-facing `Iterator` trait
/// (planned for v0.2 with generics). Today the dispatch is a `match`
/// over the small set of iterable `Value` shapes — Range and Enumerate.
pub(super) fn advance_iterator(
    value: &mut Value,
    span: &Span,
) -> Result<Option<Value>, RuntimeError> {
    match value {
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            let stop = if *inclusive {
                start.to_i64() > end.to_i64()
            } else {
                start.to_i64() >= end.to_i64()
            };
            if stop {
                return Ok(None);
            }
            let current = *start;
            // Advance by one. Saturating — in pathological loops where
            // start ≈ MAX this prevents a panic; the bound check above
            // still stops the loop the next iteration.
            *start = start.add_and_saturate(Integer::ONE);
            Ok(Some(Value::Integer(current)))
        }
        Value::Enumerate { inner, next_index } => match advance_iterator(inner, span)? {
            None => Ok(None),
            Some(elem) => {
                let idx = *next_index;
                *next_index = next_index.saturating_add(1);
                let index_value = Value::Integer(Integer::new(idx).unwrap_or(Integer::MAX));
                Ok(Some(Value::from_tuple(vec![index_value, elem])))
            }
        },
        other => Err(RuntimeError::TypeError {
            message: format!("`for` expects an iterable, found {other}"),
            span: span.clone(),
        }),
    }
}

pub(super) fn binary_long_try_method(
    receiver: &Long,
    arguments: Vec<Value>,
    op: impl Fn(Long, Long) -> Option<Long>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Long(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects a Long argument".to_owned(),
            span,
        });
    };
    Ok(match op(*receiver, other) {
        Some(value) => Value::Long(value),
        None => Value::Null,
    })
}

/// Test whether `value` matches `pattern`, calling `bind` for every
/// variable introduced in the pattern. Returns true on a match.
pub(super) fn pattern_matches<F>(
    arena: &Arena,
    pattern_id: PatternId,
    value: &Value,
    bind: &mut F,
) -> bool
where
    F: FnMut(&str, Value),
{
    let pattern = &arena.pattern(pattern_id).node;
    match pattern {
        Pattern::Wildcard => true,
        Pattern::Null => value.is_null(),
        Pattern::Variable(name) => {
            bind(name, value.clone());
            true
        }
        Pattern::Tuple(elements) => match value {
            Value::Tuple(tuple_values) => {
                if elements.len() != tuple_values.len() {
                    return false;
                }
                elements
                    .iter()
                    .zip(tuple_values.iter())
                    .all(|(child, child_value)| pattern_matches(arena, *child, child_value, bind))
            }
            _ => false,
        },
        Pattern::Or(alternatives) => alternatives
            .iter()
            .any(|alt| pattern_matches(arena, *alt, value, bind)),
        Pattern::Range {
            start,
            end,
            inclusive,
        } => match value {
            Value::Integer(i) => {
                let start_i = literal_pattern_to_i64(start);
                let end_i = literal_pattern_to_i64(end);
                let v = i.to_i64();
                if *inclusive {
                    v >= start_i && v <= end_i
                } else {
                    v >= start_i && v < end_i
                }
            }
            Value::Tryte(t) => {
                let start_i = literal_pattern_to_i64(start);
                let end_i = literal_pattern_to_i64(end);
                let v = t.to_i64();
                if *inclusive {
                    v >= start_i && v <= end_i
                } else {
                    v >= start_i && v < end_i
                }
            }
            _ => false,
        },
        Pattern::Literal(literal) => literal_matches(literal, value),
        Pattern::EnumVariant {
            variant_name,
            payload: sub_pattern,
            ..
        } => match value {
            Value::EnumVariant {
                variant, payload, ..
            } => {
                if variant != variant_name {
                    return false;
                }
                match (sub_pattern, payload) {
                    (None, None) => true,
                    (Some(pat), Some(val)) => pattern_matches(arena, *pat, val, bind),
                    _ => false,
                }
            }
            _ => false,
        },
    }
}

pub(super) fn literal_pattern_to_i64(literal: &LiteralPattern) -> i64 {
    match literal {
        LiteralPattern::Integer { value, .. } | LiteralPattern::Ternary(value) => {
            i64::try_from(*value).unwrap_or(i64::MAX)
        }
        _ => 0,
    }
}

pub(super) fn literal_matches(literal: &LiteralPattern, value: &Value) -> bool {
    match (literal, value) {
        (LiteralPattern::Integer { value: lit, .. }, Value::Integer(i)) => {
            i.to_i64() == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Integer { value: lit, .. }, Value::Tryte(t)) => {
            i64::from(t.to_i16()) == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Ternary(lit), Value::Integer(i)) => {
            i.to_i64() == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Trilean(lit), Value::Trilean(value)) => {
            matches!(
                (lit, value),
                (TrileanValue::True, Trilean::True)
                    | (TrileanValue::False, Trilean::False)
                    | (TrileanValue::Unknown, Trilean::Unknown)
            )
        }
        (LiteralPattern::String(lit), Value::String(actual)) => &**actual == lit,
        _ => false,
    }
}
