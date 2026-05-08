//! Resolved-type representation used by the type checker.
//!
//! Distinct from `triet_syntax::TypeExpr`: that one is *syntactic*
//! (what the parser saw); this one is *semantic* (what the type
//! checker resolved it to). Built-in types live as their own variants;
//! generics, tuples, nullables, and function types are recursive.

use std::fmt;

/// A resolved Triết type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// `Trit` — the 1-trit numeric atom.
    Trit,
    /// `Tryte` — 9-trit integer.
    Tryte,
    /// `Integer` — 27-trit integer (default).
    Integer,
    /// `Long` — 81-trit integer (deferred, accepted in annotations).
    Long,
    /// `Trilean` — three-valued truth.
    Trilean,
    /// UTF-8 owned text.
    String,
    /// `()` zero-sized value.
    Unit,
    /// Nullable wrapper `T?`.
    Nullable(Box<Self>),
    /// Tuple type.
    Tuple(Vec<Self>),
    /// Function type `(P1, P2, ...) -> R`.
    Function {
        /// Parameter types, positionally.
        parameters: Vec<Self>,
        /// Return type.
        return_type: Box<Self>,
    },
    /// `Range<T>` produced by `a..b` or `a..=b`. The element type is
    /// the operand type (e.g. `Integer` for `0..100`).
    Range(Box<Self>),
    /// A type the checker could not determine — used as a recovery
    /// placeholder so cascading errors don't compound.
    Unknown,
}

impl Type {
    /// Returns true if this type is a numeric ternary integer (Trit,
    /// Tryte, Integer, Long).
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Trit | Self::Tryte | Self::Integer | Self::Long)
    }

    /// Returns true if `self` and `other` are the same type, treating
    /// `Unknown` as compatible with everything (so an earlier error
    /// doesn't trigger a chain of follow-up errors).
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        if matches!(self, Self::Unknown) || matches!(other, Self::Unknown) {
            return true;
        }
        self == other
    }

    /// If this is a `Nullable(T)`, return `T`; otherwise return `self`
    /// unchanged. Used by `?:`, `?.`, `!!` post-strip semantics.
    #[must_use]
    pub fn unwrap_nullable(&self) -> &Self {
        match self {
            Self::Nullable(inner) => inner,
            other => other,
        }
    }

    /// Returns true if a value of `self` may be `null` (i.e. the type
    /// is `Nullable<_>`).
    #[must_use]
    pub const fn is_nullable(&self) -> bool {
        matches!(self, Self::Nullable(_))
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trit => formatter.write_str("Trit"),
            Self::Tryte => formatter.write_str("Tryte"),
            Self::Integer => formatter.write_str("Integer"),
            Self::Long => formatter.write_str("Long"),
            Self::Trilean => formatter.write_str("Trilean"),
            Self::String => formatter.write_str("String"),
            Self::Unit => formatter.write_str("Unit"),
            Self::Nullable(inner) => write!(formatter, "{inner}?"),
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
            Self::Function {
                parameters,
                return_type,
            } => {
                formatter.write_str("(")?;
                for (i, parameter) in parameters.iter().enumerate() {
                    if i > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{parameter}")?;
                }
                write!(formatter, ") -> {return_type}")
            }
            Self::Range(element) => write!(formatter, "Range<{element}>"),
            Self::Unknown => formatter.write_str("?"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_predicate_includes_all_ternary_ints() {
        assert!(Type::Trit.is_numeric());
        assert!(Type::Tryte.is_numeric());
        assert!(Type::Integer.is_numeric());
        assert!(Type::Long.is_numeric());
        assert!(!Type::Trilean.is_numeric());
        assert!(!Type::String.is_numeric());
    }

    #[test]
    fn matches_treats_unknown_as_universal() {
        assert!(Type::Integer.matches(&Type::Unknown));
        assert!(Type::Unknown.matches(&Type::Integer));
        assert!(Type::Unknown.matches(&Type::Unknown));
    }

    #[test]
    fn matches_requires_exact_for_non_unknown() {
        assert!(Type::Integer.matches(&Type::Integer));
        assert!(!Type::Integer.matches(&Type::Tryte));
    }

    #[test]
    fn unwrap_nullable_strips_one_layer() {
        let nullable = Type::Nullable(Box::new(Type::Integer));
        assert_eq!(nullable.unwrap_nullable(), &Type::Integer);
        // Non-nullable returns self unchanged.
        assert_eq!(Type::Integer.unwrap_nullable(), &Type::Integer);
    }

    #[test]
    fn display_is_readable() {
        assert_eq!(Type::Integer.to_string(), "Integer");
        assert_eq!(
            Type::Nullable(Box::new(Type::String)).to_string(),
            "String?",
        );
        assert_eq!(
            Type::Tuple(vec![Type::Integer, Type::Trilean]).to_string(),
            "(Integer, Trilean)",
        );
        assert_eq!(
            Type::Function {
                parameters: vec![Type::Integer, Type::Integer],
                return_type: Box::new(Type::Integer),
            }
            .to_string(),
            "(Integer, Integer) -> Integer",
        );
    }
}
