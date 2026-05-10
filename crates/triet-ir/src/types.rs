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
