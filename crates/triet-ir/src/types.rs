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
