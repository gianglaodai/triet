//! Runtime value type produced by evaluating Triết expressions.

use std::{fmt, rc::Rc};

use triet_core::{Integer, Trit, Tryte};
use triet_logic::Trilean;
use triet_syntax::{ExprId, FunctionDef, LambdaParam, TypeId};

use crate::env::ValueEnvironment;

/// A runtime value.
///
/// Heap-aware variants (`String`, `Tuple`, `Function`, `Lambda`) wrap
/// their interior in `Rc` so the interpreter can clone values cheaply
/// (Mojo-style ARC for the v0.1 tree-walker).
#[derive(Clone, Debug)]
pub enum Value {
    /// A single ternary digit.
    Trit(Trit),
    /// 9-trit integer.
    Tryte(Tryte),
    /// 27-trit integer (Triết default).
    Integer(Integer),
    /// 3-valued truth.
    Trilean(Trilean),
    /// UTF-8 owned text (cheap to clone via `Rc`).
    String(Rc<String>),
    /// Empty value `()`.
    Unit,
    /// Null marker for nullable values.
    Null,
    /// Tuple of values.
    Tuple(Rc<Vec<Self>>),
    /// Range of `Integer`s, used by `for` over `0..n` etc.
    Range {
        /// Lower bound (inclusive).
        start: Integer,
        /// Upper bound (inclusive iff `inclusive`).
        end: Integer,
        /// Whether the upper bound is inclusive.
        inclusive: bool,
    },
    /// A function defined at module level. Stored by reference into the
    /// program's `items` slice so we don't clone the body each call.
    Function(Rc<FunctionRef>),
    /// A closure with captured environment.
    Lambda(Rc<Closure>),
    /// A built-in function callable from Triết (print, `to_string`, ...).
    Builtin(BuiltinFn),
}

/// Reference into the program for a top-level function.
#[derive(Debug)]
pub struct FunctionRef {
    /// The parsed function (cloned from the AST so we don't keep a
    /// borrow on the `Program`; cheap because `Rc<FunctionRef>` shares).
    pub def: FunctionDef,
}

/// A closure: lambda parameters + body + captured environment.
#[derive(Debug)]
pub struct Closure {
    /// Parameter list (positional).
    pub parameters: Vec<LambdaParam>,
    /// Optional return type annotation (informational; checker
    /// already validated).
    pub return_type: Option<TypeId>,
    /// Body expression handle into the program arena.
    pub body: ExprId,
    /// Frame stack captured at lambda creation time.
    pub captured_env: ValueEnvironment,
}

/// A function pointer for built-in functions (print, etc.).
pub(crate) type BuiltinFn = fn(&[Value]) -> Result<Value, crate::error::RuntimeError>;

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Trit(a), Self::Trit(b)) => a == b,
            (Self::Tryte(a), Self::Tryte(b)) => a == b,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Trilean(a), Self::Trilean(b)) => a == b,
            (Self::String(a), Self::String(b)) => **a == **b,
            (Self::Unit, Self::Unit) | (Self::Null, Self::Null) => true,
            (Self::Tuple(a), Self::Tuple(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y),
            (
                Self::Range { start: s1, end: e1, inclusive: i1 },
                Self::Range { start: s2, end: e2, inclusive: i2 },
            ) => s1 == s2 && e1 == e2 && i1 == i2,
            // Functions / lambdas / builtins compare by identity (Rc
            // pointer); comparing them in source code is rare.
            (Self::Function(a), Self::Function(b)) => Rc::ptr_eq(a, b),
            (Self::Lambda(a), Self::Lambda(b)) => Rc::ptr_eq(a, b),
            (Self::Builtin(a), Self::Builtin(b)) => std::ptr::fn_addr_eq(*a, *b),
            _ => false,
        }
    }
}

impl Eq for Value {}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trit(t) => write!(formatter, "{t}"),
            Self::Tryte(t) => write!(formatter, "{t}"),
            Self::Integer(i) => write!(formatter, "{i}"),
            Self::Trilean(t) => write!(formatter, "{t}"),
            Self::String(s) => formatter.write_str(s),
            Self::Unit => formatter.write_str("()"),
            Self::Null => formatter.write_str("null"),
            Self::Tuple(elements) => {
                formatter.write_str("(")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{element}")?;
                }
                formatter.write_str(")")
            }
            Self::Range { start, end, inclusive } => {
                let separator = if *inclusive { "..=" } else { ".." };
                write!(formatter, "{start}{separator}{end}")
            }
            Self::Function(_) => formatter.write_str("<function>"),
            Self::Lambda(_) => formatter.write_str("<lambda>"),
            Self::Builtin(_) => formatter.write_str("<builtin>"),
        }
    }
}

impl Value {
    /// Construct a `Value::String` from an owned `String`.
    #[must_use]
    pub fn from_string(text: String) -> Self {
        Self::String(Rc::new(text))
    }

    /// Construct a `Value::Tuple` from owned elements.
    #[must_use]
    pub fn from_tuple(elements: Vec<Self>) -> Self {
        Self::Tuple(Rc::new(elements))
    }

    /// Returns true if the value is the null marker.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_values_compare_by_content() {
        let a = Value::Integer(Integer::new(5).unwrap());
        let b = Value::Integer(Integer::new(5).unwrap());
        let c = Value::Integer(Integer::new(7).unwrap());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn trilean_values_compare_correctly() {
        assert_eq!(Value::Trilean(Trilean::True), Value::Trilean(Trilean::True));
        assert_ne!(Value::Trilean(Trilean::True), Value::Trilean(Trilean::Unknown));
    }

    #[test]
    fn string_values_compare_by_content_not_pointer() {
        let a = Value::from_string("hi".to_owned());
        let b = Value::from_string("hi".to_owned());
        assert_eq!(a, b);
    }

    #[test]
    fn tuple_compares_element_wise() {
        let a = Value::from_tuple(vec![
            Value::Integer(Integer::new(1).unwrap()),
            Value::Trilean(Trilean::True),
        ]);
        let b = Value::from_tuple(vec![
            Value::Integer(Integer::new(1).unwrap()),
            Value::Trilean(Trilean::True),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn null_compares_only_to_null() {
        assert_eq!(Value::Null, Value::Null);
        assert_ne!(Value::Null, Value::Unit);
    }

    #[test]
    fn display_renders_basic_types() {
        assert_eq!(Value::Integer(Integer::new(42).unwrap()).to_string(), "42");
        assert_eq!(Value::Trilean(Trilean::True).to_string(), "true");
        assert_eq!(Value::Trilean(Trilean::Unknown).to_string(), "unknown");
        assert_eq!(Value::from_string("hi".to_owned()).to_string(), "hi");
        assert_eq!(Value::Unit.to_string(), "()");
        assert_eq!(Value::Null.to_string(), "null");
    }

    #[test]
    fn display_renders_tuple_with_separators() {
        let value = Value::from_tuple(vec![
            Value::Integer(Integer::new(1).unwrap()),
            Value::Integer(Integer::new(2).unwrap()),
        ]);
        assert_eq!(value.to_string(), "(1, 2)");
    }

    #[test]
    fn display_renders_range() {
        let value = Value::Range {
            start: Integer::new(0).unwrap(),
            end: Integer::new(10).unwrap(),
            inclusive: false,
        };
        assert_eq!(value.to_string(), "0..10");

        let inclusive = Value::Range {
            start: Integer::new(0).unwrap(),
            end: Integer::new(10).unwrap(),
            inclusive: true,
        };
        assert_eq!(inclusive.to_string(), "0..=10");
    }
}
