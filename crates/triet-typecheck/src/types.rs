//! Resolved-type representation used by the type checker.
//!
//! Distinct from `triet_syntax::TypeExpr`: that one is *syntactic*
//! (what the parser saw); this one is *semantic* (what the type
//! checker resolved it to). Built-in types live as their own variants;
//! generics, tuples, nullables, and function types are recursive.

use std::fmt;
use triet_syntax::{ReferenceForm, TypeParameter};

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
    /// `Trilean` — three-valued truth. Per [ADR-0021], the `refined`
    /// flag distinguishes generic `Trilean` (might be Unknown) from
    /// `Trilean!` (statically proven non-Unknown). Plain `if cond`
    /// accepts only `refined: true`; generic Trilean raises E1033.
    /// Widening `Trilean! → Trilean` is implicit via [`Self::matches`].
    ///
    /// Use [`Self::TRILEAN`] / [`Self::TRILEAN_KNOWN`] consts at call
    /// sites instead of constructing the struct literal directly.
    ///
    /// [ADR-0021]: ../../../docs/decisions/0021-trilean-refinement.md
    Trilean {
        /// True ⇔ this is `Trilean!` — refinement subtype, never Unknown.
        refined: bool,
    },
    /// UTF-8 owned text.
    String,
    /// Heap-allocated growable array. The element type is monomorphic
    /// in Bậc A (always `Integer`); generic `Vector<T>` is Bậc B/C.
    Vector(Box<Self>),
    /// Heap-allocated key-value map (ADR-0078). Key = Integer cứng in P1;
    /// key-typed (`HashMap<String, V>`) deferred to Tầng 2 (Comparable ADR-0038).
    HashMap(Box<Self>, Box<Self>),
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
    /// Q2-A. Empty `type_parameters` means a monomorphic function.
    Function {
        /// Generic type parameters declared on the function
        /// (empty for non-generic functions).
        type_parameters: Vec<TypeParameter>,
        /// Parameter types, positionally. May contain `TypeParameter(name)`
        /// when the function is generic.
        parameters: Vec<Self>,
        /// Return type. May contain `TypeParameter(name)` when the
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
        type_parameters: Vec<TypeParameter>,
        /// Fields in declaration order. Stored as `(name, type)` pairs.
        fields: Vec<(String, Self)>,
    },
    /// User-defined enum type: `enum Option { Some(Integer), None }`.
    UserEnum {
        /// Enum name.
        name: String,
        /// Generic type parameters (empty for non-generic enums).
        type_parameters: Vec<TypeParameter>,
        /// Variants in declaration order. Stored as `(name, optional_payload)`.
        variants: Vec<(String, Option<Box<Self>>)>,
    },
    /// A generic type parameter: `T` in `struct Box<T> { value: T }`.
    TypeParameter(String),
    /// Outcome type per [ADR-0020]:
    ///
    /// - `T~E` binary outcome (`allow_null_state = false`): success T
    ///   or failure E. `Trit::Zero` state is invalid (compile-time
    ///   E1025 if constructed; runtime E2210 if encountered).
    /// - `T?~E` ternary outcome (`allow_null_state = true`): success T,
    ///   null (`Trit::Zero`), or failure E.
    ///
    /// [ADR-0020]: ../../../../docs/decisions/0020-outcome-error-handling.md
    Outcome {
        /// Success-arm payload type.
        value_type: Box<Self>,
        /// Failure-arm payload type.
        error_type: Box<Self>,
        /// True for `T?~E` (3-state with null); false for `T~E` (2-state).
        allow_null_state: bool,
    },
    /// A type the checker could not determine — used as a recovery
    /// placeholder so cascading errors don't compound.
    Unknown,
    /// The bottom type — a computation that never produces a value
    /// (diverges: `return`, `break`, `continue`, infinite loop).
    /// `Never` is compatible with every type (can appear anywhere a
    /// value is expected, since control never reaches that point).
    Never,
    /// A reference with ownership semantics
    Reference(ReferenceForm, Box<Self>),
    /// Atomic wrapper
    Atomic(Box<Self>),
}

impl Type {
    /// Generic `Trilean` — might be Unknown. The default when a Trilean
    /// originates from a Trilean-typed variable, `unknown` literal,
    /// nullable comparison, or any operator that can propagate
    /// `Trilean::Unknown` per ADR-0010 §4.
    pub const TRILEAN: Self = Self::Trilean { refined: false };

    /// Refined `Trilean!` — statically proven non-Unknown per ADR-0021.
    /// Produced by `true` / `false` literals, non-nullable primitive
    /// comparisons (`Integer == Integer`, etc.), Łukasiewicz/Kleene
    /// operators where both operands are refined, and explicit
    /// narrowing methods (`.assume_known(msg)` etc.). Widens implicitly
    /// to [`Self::TRILEAN`] via [`Self::matches`].
    pub const TRILEAN_KNOWN: Self = Self::Trilean { refined: true };

    /// Returns true if this type is a numeric ternary integer (Trit,
    /// Tryte, Integer, Long).
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Trit | Self::Tryte | Self::Integer | Self::Long)
    }

    /// True for Bậc A scalar types that fit in a single i64 slot
    /// (all numeric + Trilean).
    #[must_use]
    pub const fn is_scalar(&self) -> bool {
        self.is_numeric() || matches!(self, Self::Trilean { .. })
    }

    /// True for heap-allocated types (`{ptr,len,cap}`) usable as an Outcome
    /// payload in Bậc B (HP.4). Excludes struct/enum payloads, still sealed.
    #[must_use]
    pub const fn is_heap(&self) -> bool {
        matches!(self, Self::String | Self::Vector(_) | Self::HashMap(_, _))
    }

    /// ADR-0083 §5 — is this a valid `HashMap` KEY type? `Integer`/`String`
    /// (ADR-0080), or a `Struct` all of whose leaves are hashable
    /// (`is_hashable_leaf`). An Enum key is Slice 2 (deferred → not hashable
    /// here). Structural content hash/eq — NOT `==`/Ł3 (ADR-0083 §1).
    #[must_use]
    pub fn is_hashable_key(&self) -> bool {
        match self {
            Self::Integer | Self::String => true,
            Self::UserStruct { fields, .. } => fields.iter().all(|(_, ft)| ft.is_hashable_leaf()),
            _ => false,
        }
    }

    /// ADR-0083 §5 — is this type a valid LEAF of a hashable key struct? Only
    /// scalar non-nullable (`Trit`/`Tryte`/`Integer`/`Long`/`Trilean`),
    /// `String`, or a nested struct that recursively satisfies the same rule.
    /// A `Vector`/`HashMap`/`Enum`/`Nullable`/`Outcome` (mutable, sentinel-
    /// bearing, or discriminant-tagged) leaf is NON-hashable → E1048.
    #[must_use]
    pub fn is_hashable_leaf(&self) -> bool {
        match self {
            Self::Trit
            | Self::Tryte
            | Self::Integer
            | Self::Long
            | Self::Trilean { .. }
            | Self::String => true,
            Self::UserStruct { fields, .. } => fields.iter().all(|(_, ft)| ft.is_hashable_leaf()),
            _ => false,
        }
    }

    /// Returns true if this is any Trilean (refined or not). Replaces
    /// the old unit-variant `matches!(t, Type::Trilean)` pattern.
    #[must_use]
    pub const fn is_trilean(&self) -> bool {
        matches!(self, Self::Trilean { .. })
    }

    /// Returns true if this is a refined `Trilean!`. Returns false for
    /// generic `Trilean` and every non-Trilean type.
    #[must_use]
    pub const fn is_refined_trilean(&self) -> bool {
        matches!(self, Self::Trilean { refined: true })
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
        // Unknown and Never are universal subtypes/supertypes:
        // - Unknown: recovery placeholder, matches everything
        // - Never: bottom type (diverging block), compatible with everything
        if matches!(self, Self::Unknown | Self::Never)
            || matches!(other, Self::Unknown | Self::Never)
        {
            return true;
        }
        // Trilean refinement (ADR-0021 §1): Trilean! widens to Trilean
        // implicitly. Generic Trilean does NOT satisfy Trilean! — that
        // narrowing requires explicit .assume_known() etc.
        //
        // Convention: `self.matches(other)` is read as "self (expected
        // type) accepts other (supplied value type)" — mirrors the
        // Nullable widening case below where `Nullable(T).matches(T)`
        // is true. So:
        //   self=Trilean!  other=Trilean!  → OK
        //   self=Trilean!  other=Trilean   → REFUSE (narrowing)
        //   self=Trilean   other=Trilean!  → OK     (widening)
        //   self=Trilean   other=Trilean   → OK
        if let (Self::Trilean { refined: self_r }, Self::Trilean { refined: other_r }) =
            (self, other)
        {
            // Refuse only when self is refined but other is not.
            return !*self_r || *other_r;
        }
        // Recurse through Nullable so Nullable(Unknown) matches any
        // Nullable(T) — needed for `if { x } else { null }` branch
        // unification where `null` infers as Nullable(Unknown).
        if let (Self::Nullable(a), Self::Nullable(b)) = (self, other) {
            return a.matches(b);
        }
        // Widening: Nullable(T) accepts T (e.g. Integer ⊂ Integer?)
        // TODO(ADR-0041 §watch-list.6): inner.as_ref() == other uses
        // structural equality (PartialEq), not .matches(). This means
        // `let x: Trilean? = true` (Trilean! → Trilean?) is wrongly
        // rejected because Trilean! != Trilean by ==. Should use
        // inner.as_ref().matches(other) instead. Defer to Bậc B —
        // Bậc A target type is Integer?, not Trilean?.
        if let Self::Nullable(inner) = self
            && inner.as_ref() == other
        {
            return true;
        }
        // Outcome structural match (ADR-0020 §1): same null-state +
        // both inner types match. `T~E` and `T?~E` are distinct types
        // — no implicit conversion. Refuse-over-guess.
        if let (
            Self::Outcome {
                value_type: v1,
                error_type: e1,
                allow_null_state: n1,
            },
            Self::Outcome {
                value_type: v2,
                error_type: e2,
                allow_null_state: n2,
            },
        ) = (self, other)
        {
            return n1 == n2 && v1.matches(v2) && e1.matches(e2);
        }
        // Bậc A widening: Trilean! (+1/0/-1) is a subset of Integer
        // (27-trit signed). Both are i64 at runtime, so Integer accepts
        // Trilean! — needed for `return true && false` in integration tests.
        if matches!(self, Self::Integer) && matches!(other, Self::Trilean { refined: true }) {
            return true;
        }
        // Vector structural match: Vector<X> matches Vector<Y> if X matches Y.
        // TypeParameter in element position is a wildcard — generic `Vector<T>`
        // must match concrete `Vector<Integer>` during stdlib stub validation.
        if let (Self::Vector(a), Self::Vector(b)) = (self, other) {
            return matches!(a.as_ref(), Self::TypeParameter(_))
                || matches!(b.as_ref(), Self::TypeParameter(_))
                || a.matches(b);
        }
        // HashMap structural match: HashMap<K1,V1> matches HashMap<K2,V2> if
        // K1 matches K2 and V1 matches V2. TypeParameter in key/value position
        // is a wildcard (generic `HashMap<Integer,V>` must match concrete
        // `HashMap<Integer,String>` during stdlib stub validation).
        if let (Self::HashMap(pk, pv), Self::HashMap(ak, av)) = (self, other) {
            let k_ok = matches!(pk.as_ref(), Self::TypeParameter(_))
                || matches!(ak.as_ref(), Self::TypeParameter(_))
                || pk.matches(ak);
            let v_ok = matches!(pv.as_ref(), Self::TypeParameter(_))
                || matches!(av.as_ref(), Self::TypeParameter(_))
                || pv.matches(av);
            return k_ok && v_ok;
        }
        // Same-name user types match even with different type parameters
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

    /// Implements the `Send` derivation algorithm (ADR-0026 v2 §2.1).
    #[must_use]
    pub fn is_send(&self) -> bool {
        match self {
            Self::Trit
            | Self::Tryte
            | Self::Integer
            | Self::Long
            | Self::Trilean { .. }
            | Self::Unit
            | Self::String => true,
            Self::Vector(inner) => inner.is_send(),
            Self::HashMap(k, v) => k.is_send() && v.is_send(),
            Self::Tuple(elements) => elements.iter().all(Self::is_send),
            Self::Nullable(inner) => inner.is_send(),
            Self::Outcome {
                value_type,
                error_type,
                ..
            } => value_type.is_send() && error_type.is_send(),
            Self::Range(inner) => inner.is_send(),
            Self::UserStruct { fields, .. } => fields.iter().all(|(_, t)| t.is_send()),
            Self::UserEnum { variants, .. } => variants
                .iter()
                .all(|(_, p)| p.as_ref().is_none_or(|inner| inner.is_send())),
            Self::Reference(form, inner) => form.is_owning() && inner.is_send(),
            Self::Atomic(_) => true,
            Self::Function { .. } => true,
            Self::TypeParameter(_) | Self::Unknown | Self::Never => true,
        }
    }

    /// Returns true if `self` qualifies as `AtomicValue` payload per
    /// ADR-0028 §2 (Atomic primitive types). Only ternary primitives
    /// with hardware atomic support qualify: Trit/Tryte/Integer/Trilean.
    /// Long excluded (81-trit exceeds hardware atomic width).
    /// TypeParameter/Unknown pass through as recovery (don't fire spurious
    /// errors on generic types or already-failed typecheck).
    #[must_use]
    pub const fn is_atomic_value(&self) -> bool {
        matches!(
            self,
            Self::Trit
                | Self::Tryte
                | Self::Integer
                | Self::Trilean { .. }
                | Self::TypeParameter(_)
                | Self::Unknown
        )
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

    /// Replace every `TypeParameter(name)` with `map[name]` if present.
    /// Used during monomorphization: `Box<T>` with `T→Integer` becomes
    /// the concrete struct type.
    #[must_use]
    pub fn substitute(&self, map: &std::collections::HashMap<String, Self>) -> Self {
        match self {
            Self::TypeParameter(name) => map.get(name).cloned().unwrap_or_else(|| self.clone()),
            Self::Nullable(inner) => Self::Nullable(Box::new(inner.substitute(map))),
            Self::Tuple(elements) => {
                Self::Tuple(elements.iter().map(|e| e.substitute(map)).collect())
            }
            Self::Function {
                type_parameters,
                parameters,
                return_type,
            } => Self::Function {
                type_parameters: type_parameters.clone(),
                parameters: parameters.iter().map(|p| p.substitute(map)).collect(),
                return_type: Box::new(return_type.substitute(map)),
            },
            Self::Range(inner) => Self::Range(Box::new(inner.substitute(map))),
            Self::UserStruct {
                name,
                type_parameters,
                fields,
            } => {
                // Type parameters are replaced with empty vec — the
                // monomorphized type has no type parameters.
                let local_map: std::collections::HashMap<_, _> = type_parameters
                    .iter()
                    .map(|p| {
                        (
                            p.name.clone(),
                            map.get(&p.name).cloned().unwrap_or(Self::Unknown),
                        )
                    })
                    .collect();
                let merged = {
                    let mut m = map.clone();
                    m.extend(local_map);
                    m
                };
                Self::UserStruct {
                    name: name.clone(),
                    type_parameters: Vec::new(),
                    fields: fields
                        .iter()
                        .map(|(n, t)| (n.clone(), t.substitute(&merged)))
                        .collect(),
                }
            }
            Self::UserEnum {
                name,
                type_parameters,
                variants,
            } => {
                let local_map: std::collections::HashMap<_, _> = type_parameters
                    .iter()
                    .map(|p| {
                        (
                            p.name.clone(),
                            map.get(&p.name).cloned().unwrap_or(Self::Unknown),
                        )
                    })
                    .collect();
                let merged = {
                    let mut m = map.clone();
                    m.extend(local_map);
                    m
                };
                Self::UserEnum {
                    name: name.clone(),
                    type_parameters: Vec::new(),
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
            // Outcome substitution — recurse into value + error types.
            Self::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => Self::Outcome {
                value_type: Box::new(value_type.substitute(map)),
                error_type: Box::new(error_type.substitute(map)),
                allow_null_state: *allow_null_state,
            },
            Self::Reference(form, inner) => Self::Reference(*form, Box::new(inner.substitute(map))),
            Self::Atomic(inner) => Self::Atomic(Box::new(inner.substitute(map))),
            Self::Vector(inner) => Self::Vector(Box::new(inner.substitute(map))),
            Self::HashMap(k, v) => {
                Self::HashMap(Box::new(k.substitute(map)), Box::new(v.substitute(map)))
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
            Self::Trilean { refined: true } => formatter.write_str("Trilean!"),
            Self::Trilean { refined: false } => formatter.write_str("Trilean"),
            Self::String => formatter.write_str("String"),
            Self::Vector(inner) => write!(formatter, "Vector<{inner}>"),
            Self::HashMap(k, v) => write!(formatter, "HashMap<{k}, {v}>"),
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
                type_parameters,
                parameters,
                return_type,
            } => {
                if !type_parameters.is_empty() {
                    formatter.write_str("<")?;
                    for (i, param) in type_parameters.iter().enumerate() {
                        if i > 0 {
                            formatter.write_str(", ")?;
                        }
                        formatter.write_str(&param.name)?;
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
            Self::TypeParameter(name) => formatter.write_str(name),
            Self::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => {
                if *allow_null_state {
                    write!(formatter, "{value_type}?~{error_type}")
                } else {
                    write!(formatter, "{value_type}~{error_type}")
                }
            }
            Self::Reference(form, inner) => {
                let prefix = match form {
                    ReferenceForm::StrongFrozen => "&+ ",
                    ReferenceForm::StrongMutable => "&+ mutable ",
                    ReferenceForm::BorrowReadOnly => "&0 ",
                    ReferenceForm::BorrowExclusiveMutable => "&0 mutable ",
                    ReferenceForm::WeakObserver => "&- ",
                };
                write!(formatter, "{prefix}{inner}")
            }
            Self::Atomic(inner) => write!(formatter, "Atomic<{inner}>"),
            Self::Unknown => formatter.write_str("?"),
            Self::Never => formatter.write_str("!"),
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
        assert!(!Type::TRILEAN.is_numeric());
        assert!(!Type::TRILEAN_KNOWN.is_numeric());
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
            Type::Tuple(vec![Type::Integer, Type::TRILEAN]).to_string(),
            "(Integer, Trilean)",
        );
        assert_eq!(
            Type::Function {
                type_parameters: Vec::new(),
                parameters: vec![Type::Integer, Type::Integer],
                return_type: Box::new(Type::Integer),
            }
            .to_string(),
            "(Integer, Integer) -> Integer",
        );
        // Generic function display shows the type parameters prefix.
        assert_eq!(
            Type::Function {
                type_parameters: vec![TypeParameter {
                    name: "T".into(),
                    bound: None
                }],
                parameters: vec![Type::TypeParameter("T".into())],
                return_type: Box::new(Type::TypeParameter("T".into())),
            }
            .to_string(),
            "<T>(T) -> T",
        );
    }
}
