//! Constant pool — compile-time known values that are not allocated to
//! registers but referenced inline as operands.
//!
//! Per [ADR-0007], constants do not consume a virtual register:
//! `const Integer 42_integer` is an inline operand.
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md

use std::collections::HashMap;

use triet_core::{Integer, Long, Trit, Tryte};
use triet_logic::Trilean;

use crate::types::ConstId;

/// A compile-time constant value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    /// 1-trit value: `-1`, `0`, `+1`.
    Trit(Trit),
    /// 9-trit integer.
    Tryte(Tryte),
    /// 27-trit integer.
    Integer(Integer),
    /// 81-trit integer.
    Long(Long),
    /// 3-valued logic constant.
    Trilean(Trilean),
    /// UTF-8 string literal.
    String(String),
    /// Zero-sized unit.
    Unit,
}

impl Constant {
    /// Return the type tag for this constant.
    #[must_use]
    pub const fn type_tag(&self) -> super::TypeTag {
        match self {
            Self::Trit(_) => super::TypeTag::Trit,
            Self::Tryte(_) => super::TypeTag::Tryte,
            Self::Integer(_) => super::TypeTag::Integer,
            Self::Long(_) => super::TypeTag::Long,
            Self::Trilean(_) => super::TypeTag::Trilean,
            Self::String(_) => super::TypeTag::String,
            Self::Unit => super::TypeTag::Unit,
        }
    }
}

/// Interned constant pool with deduplication.
///
/// Constants are interned by identity: inserting the same value twice
/// returns the same `ConstId`. This keeps the pool compact and makes
/// constant comparison trivial (pointer equality on `ConstId`).
///
/// Equality compares only the entry vectors — the internal dedup index
/// is not part of the logical value.
#[derive(Clone, Debug, Default)]
pub struct ConstantPool {
    entries: Vec<Constant>,
    /// Inverse index for deduplication: hash → `ConstId`.
    index: HashMap<Constant, ConstId>,
}

impl PartialEq for ConstantPool {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for ConstantPool {}

impl ConstantPool {
    /// Create an empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a constant, returning its ID. If the value is already
    /// present, returns the existing ID without duplication.
    pub fn intern(&mut self, value: Constant) -> ConstId {
        if let Some(&id) = self.index.get(&value) {
            return id;
        }
        let id = ConstId(self.entries.len() as u32);
        self.index.insert(value.clone(), id);
        self.entries.push(value);
        id
    }

    /// Look up a constant by ID.
    #[must_use]
    pub fn get(&self, id: ConstId) -> Option<&Constant> {
        self.entries.get(id.0 as usize)
    }

    /// Return the number of interned constants.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the pool is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all constants in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (ConstId, &Constant)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, c)| (ConstId(i as u32), c))
    }
}
