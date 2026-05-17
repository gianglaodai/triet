//! Core types for the IR — identifiers, type tags, and references.

use std::fmt;

/// An SSA virtual register. Each `ValueId` must be defined exactly once
/// (the SSA invariant), enforced by the verifier.
///
/// Values are local to a function. A value defined in one function cannot
/// be referenced from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(pub u32);

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

/// A basic block label. Every function has at least one block (the entry
/// block, conventionally named `entry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

/// A reference to a function within the IR program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncId(pub u32);

impl fmt::Display for FuncId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@f{}", self.0)
    }
}

/// A reference to an entry in the constant pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstId(pub u32);

impl fmt::Display for ConstId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// The type tag carried by every IR value.
///
/// Per [ADR-0007], each register carries its type explicitly — this
/// preserves "tam phân first-class" through the entire pipeline from
/// AST → IR → backend.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeTag {
    /// 1-trit numeric: `-1`, `0`, `+1`.
    Trit,
    /// 9-trit integer.
    Tryte,
    /// 27-trit integer (the default integer type).
    Integer,
    /// 81-trit big integer.
    Long,
    /// 3-valued logic: `false`, `unknown`, `true`.
    Trilean,
    /// UTF-8 owned string (ARC-managed).
    String,
    /// Zero-sized unit type `()`.
    Unit,
    /// Nullable wrapper: `T?` — 1-trit discriminator + inner type.
    Nullable(Box<Self>),
    /// Homogeneous ordered collection: `Vector<T>`. Element type is the
    /// inner tag. Introduced at v0.7.3 alongside the Rust-shim builtin
    /// opcodes per [ADR-0019 §5] so the self-hosted compiler can
    /// express symbol tables and other dynamic sequences.
    ///
    /// Java naming convention — no abbreviations (per author 2026-05-17).
    ///
    /// [ADR-0019 §5]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
    Vector(Box<Self>),
    /// Keyed collection: `HashMap<K, V>`. Box pair carries (key, value)
    /// element types. Introduced at v0.7.3 per [ADR-0019 §5].
    ///
    /// [ADR-0019 §5]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
    HashMap(Box<Self>, Box<Self>),
    /// Outcome type per [ADR-0020]:
    ///
    /// - `T~E` binary outcome (`allow_null_state = false`): success T
    ///   or failure E. `Trit::Zero` arm is invalid (panic E2210).
    /// - `T?~E` ternary outcome (`allow_null_state = true`): success T,
    ///   null (`Trit::Zero`), or failure E.
    ///
    /// Introduced at v0.7.4.3-error.3a. Wire format `.triv` v5
    /// patch bump (type discriminant 10, ADR-0008 §"Version history").
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
}

impl fmt::Display for TypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trit => write!(f, "Trit"),
            Self::Tryte => write!(f, "Tryte"),
            Self::Integer => write!(f, "Integer"),
            Self::Long => write!(f, "Long"),
            Self::Trilean => write!(f, "Trilean"),
            Self::String => write!(f, "String"),
            Self::Unit => write!(f, "Unit"),
            Self::Nullable(inner) => write!(f, "{inner}?"),
            Self::Vector(inner) => write!(f, "Vector<{inner}>"),
            Self::HashMap(key, value) => write!(f, "HashMap<{key}, {value}>"),
            Self::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => {
                if *allow_null_state {
                    write!(f, "{value_type}?~{error_type}")
                } else {
                    write!(f, "{value_type}~{error_type}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ID types ────────────────────────────────────────────────

    #[test]
    fn value_id_display() {
        assert_eq!(ValueId(0).to_string(), "%0");
        assert_eq!(ValueId(42).to_string(), "%42");
        assert_eq!(ValueId(u32::MAX).to_string(), format!("%{}", u32::MAX));
    }

    #[test]
    fn block_id_display() {
        assert_eq!(BlockId(0).to_string(), "b0");
        assert_eq!(BlockId(7).to_string(), "b7");
    }

    #[test]
    fn func_id_display() {
        assert_eq!(FuncId(0).to_string(), "@f0");
        assert_eq!(FuncId(3).to_string(), "@f3");
    }

    #[test]
    fn const_id_display() {
        assert_eq!(ConstId(0).to_string(), "c0");
        assert_eq!(ConstId(99).to_string(), "c99");
    }

    #[test]
    fn id_ordering() {
        assert!(ValueId(0) < ValueId(1));
        assert!(BlockId(0) < BlockId(1));
        assert!(FuncId(0) < FuncId(1));
        assert!(ConstId(0) < ConstId(1));
    }

    #[test]
    fn id_equality() {
        assert_eq!(ValueId(5), ValueId(5));
        assert_ne!(ValueId(5), ValueId(6));
    }

    // ── TypeTag ─────────────────────────────────────────────────

    #[test]
    fn type_tag_display() {
        assert_eq!(TypeTag::Trit.to_string(), "Trit");
        assert_eq!(TypeTag::Tryte.to_string(), "Tryte");
        assert_eq!(TypeTag::Integer.to_string(), "Integer");
        assert_eq!(TypeTag::Long.to_string(), "Long");
        assert_eq!(TypeTag::Trilean.to_string(), "Trilean");
        assert_eq!(TypeTag::String.to_string(), "String");
        assert_eq!(TypeTag::Unit.to_string(), "Unit");
    }

    #[test]
    fn nullable_type_display() {
        assert_eq!(
            TypeTag::Nullable(Box::new(TypeTag::Integer)).to_string(),
            "Integer?"
        );
        assert_eq!(
            TypeTag::Nullable(Box::new(TypeTag::Nullable(Box::new(TypeTag::Trit)))).to_string(),
            "Trit??"
        );
    }

    #[test]
    fn vector_type_display() {
        assert_eq!(
            TypeTag::Vector(Box::new(TypeTag::Integer)).to_string(),
            "Vector<Integer>"
        );
        assert_eq!(
            TypeTag::Vector(Box::new(TypeTag::Vector(Box::new(TypeTag::Trit)))).to_string(),
            "Vector<Vector<Trit>>"
        );
    }

    #[test]
    fn hashmap_type_display() {
        assert_eq!(
            TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Integer)).to_string(),
            "HashMap<String, Integer>"
        );
    }

    // v0.7.4.3-error.3a: Outcome TypeTag (ADR-0020)
    #[test]
    fn outcome_binary_type_display() {
        assert_eq!(
            TypeTag::Outcome {
                value_type: Box::new(TypeTag::Integer),
                error_type: Box::new(TypeTag::String),
                allow_null_state: false,
            }
            .to_string(),
            "Integer~String"
        );
    }

    #[test]
    fn outcome_ternary_type_display() {
        assert_eq!(
            TypeTag::Outcome {
                value_type: Box::new(TypeTag::String),
                error_type: Box::new(TypeTag::Integer),
                allow_null_state: true,
            }
            .to_string(),
            "String?~Integer"
        );
    }

    #[test]
    fn outcome_equality() {
        let binary_a = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: false,
        };
        let binary_b = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: false,
        };
        let ternary = TypeTag::Outcome {
            value_type: Box::new(TypeTag::Integer),
            error_type: Box::new(TypeTag::String),
            allow_null_state: true,
        };
        assert_eq!(binary_a, binary_b);
        assert_ne!(binary_a, ternary, "T~E vs T?~E are distinct types");
    }

    #[test]
    fn collection_equality() {
        assert_eq!(
            TypeTag::Vector(Box::new(TypeTag::Integer)),
            TypeTag::Vector(Box::new(TypeTag::Integer))
        );
        assert_ne!(
            TypeTag::Vector(Box::new(TypeTag::Integer)),
            TypeTag::Vector(Box::new(TypeTag::Long))
        );
        assert_ne!(
            TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Integer)),
            TypeTag::HashMap(Box::new(TypeTag::String), Box::new(TypeTag::Long))
        );
    }

    #[test]
    fn type_tag_equality() {
        assert_eq!(TypeTag::Trit, TypeTag::Trit);
        assert_ne!(TypeTag::Integer, TypeTag::Long);
        assert_eq!(
            TypeTag::Nullable(Box::new(TypeTag::String)),
            TypeTag::Nullable(Box::new(TypeTag::String))
        );
        assert_ne!(
            TypeTag::Nullable(Box::new(TypeTag::Trit)),
            TypeTag::Nullable(Box::new(TypeTag::Integer))
        );
    }
}
