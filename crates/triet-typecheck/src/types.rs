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
    /// Function type `(P1, P2, ...) -> R`, optionally with generic
    /// type parameters: `<T, U>(P1, P2, ...) -> R`. Generic functions
    /// carry their type-param names so call sites can perform
    /// Rust-style inference per ADR-0019 Addendum §A7 + v0.7.4.1
    /// Q2-A. Empty `type_params` means a monomorphic function.
    Function {
        /// Generic type parameters declared on the function
        /// (empty for non-generic functions).
        type_params: Vec<String>,
        /// Parameter types, positionally. May contain `TypeParam(name)`
        /// when the function is generic.
        parameters: Vec<Self>,
        /// Return type. May contain `TypeParam(name)` when the
        /// function is generic.
        return_type: Box<Self>,
    },
    /// `Range<T>` produced by `a..b` or `a..=b`. The element type is
    /// the operand type (e.g. `Integer` for `0..100`).
    Range(Box<Self>),
    /// User-defined struct type: `struct Point { x: Integer, y: Integer }`.
    /// Carries field name → type pairs inline so the checker can resolve
    /// field access without a separate type registry lookup.
    UserStruct {
        /// Struct name (for Display and error messages).
        name: String,
        /// Generic type parameters (empty for non-generic structs).
        type_params: Vec<String>,
        /// Fields in declaration order. Stored as `(name, type)` pairs.
        fields: Vec<(String, Self)>,
    },
    /// User-defined enum type: `enum Option { Some(Integer), None }`.
    UserEnum {
        /// Enum name.
        name: String,
        /// Generic type parameters (empty for non-generic enums).
        type_params: Vec<String>,
        /// Variants in declaration order. Stored as `(name, optional_payload)`.
        variants: Vec<(String, Option<Box<Self>>)>,
    },
    /// A generic type parameter: `T` in `struct Box<T> { value: T }`.
    TypeParam(String),
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
    ///
    /// Also allows **widening** `T → T?`: in balanced ternary, `T` is a
    /// strict subset of `T?` (every trit of `T` maps to a non-null
    /// discriminator in `T?`), so a known value can always be used where
    /// a nullable is expected. This is subtyping, not coercion — zero
    /// information loss, zero runtime cost.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        if matches!(self, Self::Unknown) || matches!(other, Self::Unknown) {
            return true;
        }
        // Recurse through Nullable so Nullable(Unknown) matches any
        // Nullable(T) — needed for `if { x } else { null }` branch
        // unification where `null` infers as Nullable(Unknown).
        if let (Self::Nullable(a), Self::Nullable(b)) = (self, other) {
            return a.matches(b);
        }
        // Widening: Nullable(T) accepts T (e.g. Integer ⊂ Integer?)
        if let Self::Nullable(inner) = self
            && inner.as_ref() == other
        {
            return true;
        }
        // Same-name user types match even with different type params
        // (e.g., `Option<T>` vs `Option<Integer>`). Structural
        // comparison of variants/fields catches actual mismatches.
        if let (Self::UserStruct { name: n1, .. }, Self::UserStruct { name: n2, .. })
        | (Self::UserEnum { name: n1, .. }, Self::UserEnum { name: n2, .. }) = (self, other)
            && n1 == n2
        {
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

    /// Replace every `TypeParam(name)` with `map[name]` if present.
    /// Used during monomorphization: `Box<T>` with `T→Integer` becomes
    /// the concrete struct type.
    #[must_use]
    pub fn substitute(&self, map: &std::collections::HashMap<String, Self>) -> Self {
        match self {
            Self::TypeParam(name) => map.get(name).cloned().unwrap_or_else(|| self.clone()),
            Self::Nullable(inner) => Self::Nullable(Box::new(inner.substitute(map))),
            Self::Tuple(elements) => {
                Self::Tuple(elements.iter().map(|e| e.substitute(map)).collect())
            }
            Self::Function {
                type_params,
                parameters,
                return_type,
            } => Self::Function {
                type_params: type_params.clone(),
                parameters: parameters.iter().map(|p| p.substitute(map)).collect(),
                return_type: Box::new(return_type.substitute(map)),
            },
            Self::Range(inner) => Self::Range(Box::new(inner.substitute(map))),
            Self::UserStruct {
                name,
                type_params,
                fields,
            } => {
                // Type params are replaced with empty vec — the
                // monomorphized type has no type params.
                let local_map: std::collections::HashMap<_, _> = type_params
                    .iter()
                    .map(|p| (p.clone(), map.get(p).cloned().unwrap_or(Self::Unknown)))
                    .collect();
                let merged = {
                    let mut m = map.clone();
                    m.extend(local_map);
                    m
                };
                Self::UserStruct {
                    name: name.clone(),
                    type_params: Vec::new(),
                    fields: fields
                        .iter()
                        .map(|(n, t)| (n.clone(), t.substitute(&merged)))
                        .collect(),
                }
            }
            Self::UserEnum {
                name,
                type_params,
                variants,
            } => {
                let local_map: std::collections::HashMap<_, _> = type_params
                    .iter()
                    .map(|p| (p.clone(), map.get(p).cloned().unwrap_or(Self::Unknown)))
                    .collect();
                let merged = {
                    let mut m = map.clone();
                    m.extend(local_map);
                    m
                };
                Self::UserEnum {
                    name: name.clone(),
                    type_params: Vec::new(),
                    variants: variants
                        .iter()
                        .map(|(n, p)| {
                            (
                                n.clone(),
                                p.as_ref().map(|t| Box::new(t.substitute(&merged))),
                            )
                        })
                        .collect(),
                }
            }
            // Primitives and Unknown are unchanged.
            other => other.clone(),
        }
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
            Self::Nullable(inner) => {
                if matches!(inner.as_ref(), Self::Unknown) {
                    formatter.write_str("null")
                } else {
                    write!(formatter, "{inner}?")
                }
            }
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
                type_params,
                parameters,
                return_type,
            } => {
                if !type_params.is_empty() {
                    formatter.write_str("<")?;
                    for (i, param) in type_params.iter().enumerate() {
                        if i > 0 {
                            formatter.write_str(", ")?;
                        }
                        formatter.write_str(param)?;
                    }
                    formatter.write_str(">")?;
                }
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
            Self::UserStruct { name, .. } => formatter.write_str(name),
            Self::UserEnum { name, .. } => formatter.write_str(name),
            Self::TypeParam(name) => formatter.write_str(name),
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
                type_params: Vec::new(),
                parameters: vec![Type::Integer, Type::Integer],
                return_type: Box::new(Type::Integer),
            }
            .to_string(),
            "(Integer, Integer) -> Integer",
        );
        // Generic function display shows the type params prefix.
        assert_eq!(
            Type::Function {
                type_params: vec!["T".into()],
                parameters: vec![Type::TypeParam("T".into())],
                return_type: Box::new(Type::TypeParam("T".into())),
            }
            .to_string(),
            "<T>(T) -> T",
        );
    }
}
