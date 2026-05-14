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
    /// Canonical `null` marker for nullable types `T?`.
    ///
    /// Per ADR-0010, this is the IR-level representation of the
    /// **`Trit::Zero` state of the nullable discriminator**. Conceptually
    /// equivalent to writing `Const(Trit::Zero)` and `NullWrap`-ing it,
    /// but kept as a dedicated variant so the wire format stays compact
    /// (1 byte instead of an instruction + operand) and so `NullCheck`
    /// can pattern-match without inspecting a payload.
    ///
    /// **Not** a separate "thing" alongside `Some`/`None`. The three trit
    /// states of a nullable discriminator are:
    /// - `Trit::Positive`  → wrapped value (Some)
    /// - `Trit::Zero`      → this variant (canonical null)
    /// - `Trit::Negative`  → reserved (definitely-missing, future use)
    Null,
}

impl Constant {
    /// Return the type tag for this constant.
    #[must_use]
    pub fn type_tag(&self) -> super::TypeTag {
        match self {
            Self::Trit(_) => super::TypeTag::Trit,
            Self::Tryte(_) => super::TypeTag::Tryte,
            Self::Integer(_) => super::TypeTag::Integer,
            Self::Long(_) => super::TypeTag::Long,
            Self::Trilean(_) => super::TypeTag::Trilean,
            Self::String(_) => super::TypeTag::String,
            Self::Unit => super::TypeTag::Unit,
            Self::Null => super::TypeTag::Nullable(Box::new(super::TypeTag::Unit)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeTag;

    // ── Constant type tags ──────────────────────────────────────

    #[test]
    fn trit_constant_type_tag() {
        let c = Constant::Trit(triet_core::Trit::Positive);
        assert_eq!(c.type_tag(), TypeTag::Trit);
    }

    #[test]
    fn tryte_constant_type_tag() {
        let c = Constant::Tryte(triet_core::Tryte::new(42).unwrap());
        assert_eq!(c.type_tag(), TypeTag::Tryte);
    }

    #[test]
    fn integer_constant_type_tag() {
        let c = Constant::Integer(triet_core::Integer::new(100).unwrap());
        assert_eq!(c.type_tag(), TypeTag::Integer);
    }

    #[test]
    fn long_constant_type_tag() {
        let c = Constant::Long(triet_core::Long::from_i64(1000));
        assert_eq!(c.type_tag(), TypeTag::Long);
    }

    #[test]
    fn trilean_constant_type_tag() {
        let c = Constant::Trilean(triet_logic::Trilean::True);
        assert_eq!(c.type_tag(), TypeTag::Trilean);
    }

    #[test]
    fn string_constant_type_tag() {
        let c = Constant::String("hello".into());
        assert_eq!(c.type_tag(), TypeTag::String);
    }

    #[test]
    fn unit_constant_type_tag() {
        assert_eq!(Constant::Unit.type_tag(), TypeTag::Unit);
    }

    // ── ConstantPool ────────────────────────────────────────────

    #[test]
    fn empty_pool_is_empty() {
        let pool = ConstantPool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn intern_increments_len() {
        let mut pool = ConstantPool::new();
        pool.intern(Constant::Unit);
        assert_eq!(pool.len(), 1);
        pool.intern(Constant::Integer(triet_core::Integer::new(42).unwrap()));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn intern_deduplicates_by_value() {
        let mut pool = ConstantPool::new();
        let a = pool.intern(Constant::String("same".into()));
        let b = pool.intern(Constant::String("same".into()));
        assert_eq!(a, b);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn get_returns_correct_constant() {
        let mut pool = ConstantPool::new();
        let id = pool.intern(Constant::Integer(triet_core::Integer::new(-5).unwrap()));
        assert_eq!(
            pool.get(id),
            Some(&Constant::Integer(triet_core::Integer::new(-5).unwrap()))
        );
    }

    #[test]
    fn get_out_of_bounds_returns_none() {
        let pool = ConstantPool::new();
        assert_eq!(pool.get(ConstId(999)), None);
    }

    #[test]
    fn iter_yields_all_entries_in_order() {
        let mut pool = ConstantPool::new();
        pool.intern(Constant::Unit);
        pool.intern(Constant::Trit(triet_core::Trit::Zero));
        let entries: Vec<_> = pool.iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, ConstId(0));
        assert_eq!(entries[1].0, ConstId(1));
    }

    #[test]
    fn pool_equality_ignores_dedup_index() {
        let mut a = ConstantPool::new();
        let mut b = ConstantPool::new();
        a.intern(Constant::Unit);
        a.intern(Constant::Unit); // deduplicated
        b.intern(Constant::Unit);
        assert_eq!(a, b);
    }

    #[test]
    fn large_integer_edge_values() {
        let mut pool = ConstantPool::new();
        pool.intern(Constant::Integer(triet_core::Integer::MAX));
        pool.intern(Constant::Integer(triet_core::Integer::MIN));
        assert_eq!(pool.len(), 2);
    }
}
