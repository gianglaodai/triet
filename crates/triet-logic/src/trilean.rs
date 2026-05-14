//! `Trilean`: Triết's three-valued truth type.

use std::{
    fmt,
    ops::{BitAnd, BitOr, BitXor, Not},
};

use triet_core::Trit;

/// Three-valued truth: `False`, `Unknown`, `True`.
///
/// `Trilean` parallels `Boolean` but admits a third state for "missing /
/// not yet determined". It is the truth type returned by all logic
/// operators in Triết.
///
/// # Distinction from `Trit`
///
/// `Trit` and `Trilean` both occupy 1 trit of storage, but Triết treats
/// them as distinct types so intent is unambiguous at the call site:
/// - `Trit` is a *numeric* digit (`-1`, `0`, `+1`)
/// - `Trilean` is a *truth* value (`False`, `Unknown`, `True`)
///
/// # Default
///
/// Default value is `Unknown`. This forces explicit initialization in
/// most contexts and matches Triết's philosophy: an uninitialized truth
/// is "not yet known", not "false".
///
/// # Logic systems
///
/// All universal operators (`not`, `and`, `or`) produce identical results
/// in both Łukasiewicz Ł3 and Kleene K3.
///
/// Łukasiewicz Ł3 is the default for `implies`, `iff`, `xor`. Kleene K3
/// equivalents are exposed as `kleene_implies`, `kleene_iff`, `kleene_xor`.
/// The two systems differ only at `Unknown`/`Unknown` cases — see SPEC §4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trilean {
    /// The trilean `false` (-1).
    False,
    /// The trilean `unknown` (0). Default.
    #[default]
    Unknown,
    /// The trilean `true` (+1).
    True,
}

impl Trilean {
    /// All three trilean values in numeric ascending order.
    pub const ALL: [Self; 3] = [Self::False, Self::Unknown, Self::True];

    // === Conversion ===

    /// Returns the corresponding `Trit`: `False`→`Negative`,
    /// `Unknown`→`Zero`, `True`→`Positive`.
    #[inline]
    #[must_use]
    pub const fn to_trit(self) -> Trit {
        match self {
            Self::False => Trit::Negative,
            Self::Unknown => Trit::Zero,
            Self::True => Trit::Positive,
        }
    }

    /// Constructs a `Trilean` from a `Trit`.
    #[inline]
    #[must_use]
    pub const fn from_trit(trit: Trit) -> Self {
        match trit {
            Trit::Negative => Self::False,
            Trit::Zero => Self::Unknown,
            Trit::Positive => Self::True,
        }
    }

    // === Predicates ===

    /// Returns `true` if this is `False`.
    #[inline]
    #[must_use]
    pub const fn is_false(self) -> bool {
        matches!(self, Self::False)
    }

    /// Returns `true` if this is `Unknown`.
    #[inline]
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Returns `true` if this is `True`.
    #[inline]
    #[must_use]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    /// Asserts this `Trilean` is not `Unknown`. Panics if it is.
    ///
    /// Used by Triết's `if` expression: `if cond.assume_known() { ... }`
    /// to convert a possibly-unknown trilean into a definite truth at
    /// runtime, with an explicit panic if the assumption is violated.
    ///
    /// # Panics
    ///
    /// Panics if `self == Trilean::Unknown`.
    #[inline]
    #[must_use]
    pub fn assume_known(self) -> Self {
        assert!(
            !self.is_unknown(),
            "Trilean is Unknown; expected True or False",
        );
        self
    }

    // === Universal logic ops (identical in Ł3 and K3) ===

    /// Logical NOT: `False ↔ True`, `Unknown` stays `Unknown`.
    #[inline]
    #[must_use]
    pub const fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
            Self::True => Self::False,
        }
    }

    /// Logical AND (= min). Universal across Ł3 and K3.
    ///
    /// `False` dominates: `False ∧ x = False` for any `x`.
    /// `True` is identity: `True ∧ x = x`.
    #[inline]
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    /// Logical OR (= max). Universal across Ł3 and K3.
    ///
    /// `True` dominates: `True ∨ x = True` for any `x`.
    /// `False` is identity: `False ∨ x = x`.
    #[inline]
    #[must_use]
    pub const fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    // === Łukasiewicz Ł3 ops (default) ===

    /// Łukasiewicz implication: `min(1, 1 - a + b)`.
    ///
    /// Distinctive case: `Unknown → Unknown = True` (vacuously equivalent).
    #[inline]
    #[must_use]
    #[expect(
        clippy::match_same_arms,
        reason = "explicit truth-table layout — clarity over compactness"
    )]
    pub const fn implies(self, other: Self) -> Self {
        match (self, other) {
            // False → anything = True (vacuous)
            (Self::False, _) => Self::True,
            // True → x = x
            (Self::True, x) => x,
            // Unknown → x
            (Self::Unknown, Self::True) => Self::True,
            (Self::Unknown, Self::Unknown) => Self::True, // Łukasiewicz signature
            (Self::Unknown, Self::False) => Self::Unknown,
        }
    }

    /// Łukasiewicz biconditional (iff): `(a → b) ∧ (b → a)`.
    ///
    /// Distinctive case: `Unknown ↔ Unknown = True`.
    #[inline]
    #[must_use]
    pub const fn iff(self, other: Self) -> Self {
        Self::and(self.implies(other), other.implies(self))
    }

    /// Łukasiewicz XOR: `¬(a ↔ b)`.
    ///
    /// Distinctive case: `Unknown ⊕ Unknown = False`.
    #[inline]
    #[must_use]
    pub const fn xor(self, other: Self) -> Self {
        self.iff(other).not()
    }

    // === Kleene K3 ops ===

    /// Kleene implication: `max(¬a, b)`.
    ///
    /// Distinctive case: `Unknown → Unknown = Unknown` (conservative).
    #[inline]
    #[must_use]
    pub const fn kleene_implies(self, other: Self) -> Self {
        Self::or(self.not(), other)
    }

    /// Kleene biconditional.
    ///
    /// Distinctive case: `Unknown ↔ Unknown = Unknown`.
    #[inline]
    #[must_use]
    pub const fn kleene_iff(self, other: Self) -> Self {
        Self::and(self.kleene_implies(other), other.kleene_implies(self))
    }

    /// Kleene XOR.
    ///
    /// Distinctive case: `Unknown ⊕ Unknown = Unknown`.
    #[inline]
    #[must_use]
    pub const fn kleene_xor(self, other: Self) -> Self {
        self.kleene_iff(other).not()
    }
}

// === Operator overloads for ergonomic Rust use ===

impl Not for Trilean {
    type Output = Self;

    #[inline]
    fn not(self) -> Self {
        Self::not(self)
    }
}

impl BitAnd for Trilean {
    type Output = Self;

    /// Logical AND via `&` operator.
    #[inline]
    fn bitand(self, other: Self) -> Self {
        Self::and(self, other)
    }
}

impl BitOr for Trilean {
    type Output = Self;

    /// Logical OR via `|` operator.
    #[inline]
    fn bitor(self, other: Self) -> Self {
        Self::or(self, other)
    }
}

impl BitXor for Trilean {
    type Output = Self;

    /// Łukasiewicz XOR via `^` operator (default logic system).
    #[inline]
    fn bitxor(self, other: Self) -> Self {
        Self::xor(self, other)
    }
}

impl fmt::Display for Trilean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::False => "false",
            Self::Unknown => "unknown",
            Self::True => "true",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Trilean::{False, True, Unknown};

    // === NOT (universal) ===

    #[test]
    fn not_truth_table() {
        assert_eq!(False.not(), True);
        assert_eq!(Unknown.not(), Unknown);
        assert_eq!(True.not(), False);
    }

    #[test]
    fn double_negation_is_identity() {
        for trilean in Trilean::ALL {
            assert_eq!(trilean.not().not(), trilean);
        }
    }

    // === AND truth table (universal) ===

    #[test]
    fn and_truth_table() {
        assert_eq!(False.and(False), False);
        assert_eq!(False.and(Unknown), False);
        assert_eq!(False.and(True), False);
        assert_eq!(Unknown.and(False), False);
        assert_eq!(Unknown.and(Unknown), Unknown);
        assert_eq!(Unknown.and(True), Unknown);
        assert_eq!(True.and(False), False);
        assert_eq!(True.and(Unknown), Unknown);
        assert_eq!(True.and(True), True);
    }

    #[test]
    fn false_dominates_and() {
        for trilean in Trilean::ALL {
            assert_eq!(trilean.and(False), False);
            assert_eq!(False.and(trilean), False);
        }
    }

    #[test]
    fn true_is_and_identity() {
        for trilean in Trilean::ALL {
            assert_eq!(trilean.and(True), trilean);
            assert_eq!(True.and(trilean), trilean);
        }
    }

    #[test]
    fn and_is_commutative() {
        for a in Trilean::ALL {
            for b in Trilean::ALL {
                assert_eq!(a.and(b), b.and(a));
            }
        }
    }

    // === OR truth table (universal) ===

    #[test]
    fn or_truth_table() {
        assert_eq!(False.or(False), False);
        assert_eq!(False.or(Unknown), Unknown);
        assert_eq!(False.or(True), True);
        assert_eq!(Unknown.or(False), Unknown);
        assert_eq!(Unknown.or(Unknown), Unknown);
        assert_eq!(Unknown.or(True), True);
        assert_eq!(True.or(False), True);
        assert_eq!(True.or(Unknown), True);
        assert_eq!(True.or(True), True);
    }

    #[test]
    fn true_dominates_or() {
        for trilean in Trilean::ALL {
            assert_eq!(trilean.or(True), True);
            assert_eq!(True.or(trilean), True);
        }
    }

    #[test]
    fn false_is_or_identity() {
        for trilean in Trilean::ALL {
            assert_eq!(trilean.or(False), trilean);
            assert_eq!(False.or(trilean), trilean);
        }
    }

    #[test]
    fn or_is_commutative() {
        for a in Trilean::ALL {
            for b in Trilean::ALL {
                assert_eq!(a.or(b), b.or(a));
            }
        }
    }

    // === De Morgan's laws ===

    #[test]
    fn de_morgan_for_and() {
        // ¬(a ∧ b) = (¬a) ∨ (¬b)
        for a in Trilean::ALL {
            for b in Trilean::ALL {
                assert_eq!(a.and(b).not(), a.not().or(b.not()));
            }
        }
    }

    #[test]
    fn de_morgan_for_or() {
        // ¬(a ∨ b) = (¬a) ∧ (¬b)
        for a in Trilean::ALL {
            for b in Trilean::ALL {
                assert_eq!(a.or(b).not(), a.not().and(b.not()));
            }
        }
    }

    // === Łukasiewicz IMPLIES ===

    #[test]
    fn lukasiewicz_implies_truth_table() {
        // True row
        assert_eq!(True.implies(True), True);
        assert_eq!(True.implies(Unknown), Unknown);
        assert_eq!(True.implies(False), False);
        // False row — vacuous truth
        assert_eq!(False.implies(True), True);
        assert_eq!(False.implies(Unknown), True);
        assert_eq!(False.implies(False), True);
        // Unknown row — Łukasiewicz signature at U → U
        assert_eq!(Unknown.implies(True), True);
        assert_eq!(Unknown.implies(Unknown), True);
        assert_eq!(Unknown.implies(False), Unknown);
    }

    // === Kleene IMPLIES ===

    #[test]
    fn kleene_implies_truth_table() {
        assert_eq!(True.kleene_implies(True), True);
        assert_eq!(True.kleene_implies(Unknown), Unknown);
        assert_eq!(True.kleene_implies(False), False);
        assert_eq!(False.kleene_implies(True), True);
        assert_eq!(False.kleene_implies(Unknown), True);
        assert_eq!(False.kleene_implies(False), True);
        assert_eq!(Unknown.kleene_implies(True), True);
        assert_eq!(Unknown.kleene_implies(Unknown), Unknown); // Kleene signature
        assert_eq!(Unknown.kleene_implies(False), Unknown);
    }

    #[test]
    fn lukasiewicz_kleene_implies_differ_only_at_unknown_unknown() {
        for a in Trilean::ALL {
            for b in Trilean::ALL {
                let lukasiewicz = a.implies(b);
                let kleene = a.kleene_implies(b);
                if a == Unknown && b == Unknown {
                    assert_eq!(lukasiewicz, True, "Łukasiewicz U→U must be True");
                    assert_eq!(kleene, Unknown, "Kleene U→U must be Unknown");
                    assert_ne!(lukasiewicz, kleene);
                } else {
                    assert_eq!(lukasiewicz, kleene, "Disagreement at ({a:?} → {b:?})");
                }
            }
        }
    }

    // === IFF ===

    #[test]
    fn lukasiewicz_iff_truth_table() {
        assert_eq!(True.iff(True), True);
        assert_eq!(False.iff(False), True);
        assert_eq!(True.iff(False), False);
        assert_eq!(False.iff(True), False);
        assert_eq!(True.iff(Unknown), Unknown);
        assert_eq!(False.iff(Unknown), Unknown);
        assert_eq!(Unknown.iff(True), Unknown);
        assert_eq!(Unknown.iff(False), Unknown);
        assert_eq!(Unknown.iff(Unknown), True); // Łukasiewicz signature
    }

    #[test]
    fn kleene_iff_truth_table() {
        assert_eq!(True.kleene_iff(True), True);
        assert_eq!(False.kleene_iff(False), True);
        assert_eq!(True.kleene_iff(False), False);
        assert_eq!(False.kleene_iff(True), False);
        assert_eq!(Unknown.kleene_iff(Unknown), Unknown); // Kleene signature
    }

    // === XOR ===

    #[test]
    fn lukasiewicz_xor_unknown_unknown_is_false() {
        // ¬(U ↔ U) = ¬True = False
        assert_eq!(Unknown.xor(Unknown), False);
    }

    #[test]
    fn kleene_xor_unknown_unknown_is_unknown() {
        assert_eq!(Unknown.kleene_xor(Unknown), Unknown);
    }

    #[test]
    fn xor_truth_table_for_known_values() {
        assert_eq!(True.xor(True), False);
        assert_eq!(False.xor(False), False);
        assert_eq!(True.xor(False), True);
        assert_eq!(False.xor(True), True);
    }

    // === Conversion ===

    #[test]
    fn trilean_trit_round_trip() {
        for trilean in Trilean::ALL {
            assert_eq!(Trilean::from_trit(trilean.to_trit()), trilean);
        }
    }

    #[test]
    fn trit_trilean_round_trip() {
        for trit in Trit::ALL {
            assert_eq!(Trilean::from_trit(trit).to_trit(), trit);
        }
    }

    #[test]
    fn conversion_preserves_numeric_order() {
        // False < Unknown < True corresponds to -1 < 0 < +1
        assert!(Trilean::False.to_trit().to_i8() < Trilean::Unknown.to_trit().to_i8());
        assert!(Trilean::Unknown.to_trit().to_i8() < Trilean::True.to_trit().to_i8());
    }

    // === Predicates ===

    #[test]
    fn predicates_are_exclusive() {
        for trilean in Trilean::ALL {
            let count = u8::from(trilean.is_false())
                + u8::from(trilean.is_unknown())
                + u8::from(trilean.is_true());
            assert_eq!(count, 1, "exactly one predicate must hold for {trilean:?}");
        }
    }

    // === assume_known ===

    #[test]
    fn assume_known_passes_through_known() {
        assert_eq!(True.assume_known(), True);
        assert_eq!(False.assume_known(), False);
    }

    #[test]
    #[should_panic(expected = "Trilean is Unknown")]
    fn assume_known_panics_on_unknown() {
        let _ = Unknown.assume_known();
    }

    // === Default ===

    #[test]
    fn default_is_unknown() {
        // Triết philosophy: uninitialized truth = "not yet known", not "false".
        assert_eq!(Trilean::default(), Unknown);
    }

    // === Operator traits ===

    #[test]
    fn not_operator() {
        assert_eq!(!True, False);
        assert_eq!(!Unknown, Unknown);
        assert_eq!(!False, True);
    }

    #[test]
    fn bit_and_operator() {
        assert_eq!(True & True, True);
        assert_eq!(True & False, False);
        assert_eq!(Unknown & Unknown, Unknown);
    }

    #[test]
    fn bit_or_operator() {
        assert_eq!(False | False, False);
        assert_eq!(True | False, True);
        assert_eq!(Unknown | Unknown, Unknown);
    }

    #[test]
    fn bit_xor_operator_uses_lukasiewicz() {
        assert_eq!(True ^ True, False);
        assert_eq!(True ^ False, True);
        assert_eq!(Unknown ^ Unknown, False); // Łukasiewicz default
    }

    // === Display ===

    #[test]
    fn display_uses_lowercase_keywords() {
        assert_eq!(False.to_string(), "false");
        assert_eq!(Unknown.to_string(), "unknown");
        assert_eq!(True.to_string(), "true");
    }
}
