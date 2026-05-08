//! The single ternary digit type.

use std::{fmt, ops::Neg};

/// A balanced ternary digit: `Negative` (-1), `Zero` (0), or `Positive` (+1).
///
/// `Trit` is the atomic unit of information in Triết — analogous to a `bit`
/// in binary, but with three distinguishable values.
///
/// Note: `Trit` is a *numeric* value and is distinct from `Trilean`, which
/// is a *truth* value. Both occupy 1 trit of information, but the language
/// treats them as separate types so intent is clear at the call site.
/// See `triet-logic` for `Trilean`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trit {
    /// The trit `-1`.
    Negative,
    /// The trit `0` (default).
    #[default]
    Zero,
    /// The trit `+1`.
    Positive,
}

impl Trit {
    /// All three trit values in numeric ascending order. Useful for tests
    /// and exhaustive iteration.
    pub const ALL: [Self; 3] = [Self::Negative, Self::Zero, Self::Positive];

    /// Returns this trit as a signed integer (-1, 0, or +1).
    #[inline]
    #[must_use]
    pub const fn to_i8(self) -> i8 {
        match self {
            Self::Negative => -1,
            Self::Zero => 0,
            Self::Positive => 1,
        }
    }

    /// Constructs a trit from a signed integer. Returns `None` if `value`
    /// is outside `-1..=1`.
    #[inline]
    #[must_use]
    pub const fn from_i8(value: i8) -> Option<Self> {
        match value {
            -1 => Some(Self::Negative),
            0 => Some(Self::Zero),
            1 => Some(Self::Positive),
            _ => None,
        }
    }

    /// Returns `true` if this trit is `Negative` (-1).
    #[inline]
    #[must_use]
    pub const fn is_negative(self) -> bool {
        matches!(self, Self::Negative)
    }

    /// Returns `true` if this trit is `Zero` (0).
    #[inline]
    #[must_use]
    pub const fn is_zero(self) -> bool {
        matches!(self, Self::Zero)
    }

    /// Returns `true` if this trit is `Positive` (+1).
    #[inline]
    #[must_use]
    pub const fn is_positive(self) -> bool {
        matches!(self, Self::Positive)
    }
}

impl Neg for Trit {
    type Output = Self;

    /// Inverts the trit: `Negative ↔ Positive`, `Zero` stays `Zero`.
    ///
    /// Negation in balanced ternary is the per-trit invariant — see
    /// SPEC.md §3.2.
    #[inline]
    fn neg(self) -> Self::Output {
        match self {
            Self::Negative => Self::Positive,
            Self::Zero => Self::Zero,
            Self::Positive => Self::Negative,
        }
    }
}

impl fmt::Display for Trit {
    /// Prints `+`, `0`, or `-` matching the literal trit syntax.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Negative => "-",
            Self::Zero => "0",
            Self::Positive => "+",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negation_inverts_signs() {
        assert_eq!(-Trit::Negative, Trit::Positive);
        assert_eq!(-Trit::Zero, Trit::Zero);
        assert_eq!(-Trit::Positive, Trit::Negative);
    }

    #[test]
    fn double_negation_is_identity() {
        for trit in Trit::ALL {
            assert_eq!(-(-trit), trit);
        }
    }

    #[test]
    fn i8_round_trip_is_identity() {
        for trit in Trit::ALL {
            assert_eq!(Trit::from_i8(trit.to_i8()), Some(trit));
        }
    }

    #[test]
    fn from_i8_rejects_out_of_range_values() {
        assert_eq!(Trit::from_i8(2), None);
        assert_eq!(Trit::from_i8(-2), None);
        assert_eq!(Trit::from_i8(i8::MAX), None);
        assert_eq!(Trit::from_i8(i8::MIN), None);
    }

    #[test]
    fn predicates_are_exclusive() {
        for trit in Trit::ALL {
            let count = u8::from(trit.is_negative())
                + u8::from(trit.is_zero())
                + u8::from(trit.is_positive());
            assert_eq!(count, 1, "exactly one predicate must hold for {trit:?}");
        }
    }

    #[test]
    fn ordering_matches_numeric() {
        assert!(Trit::Negative < Trit::Zero);
        assert!(Trit::Zero < Trit::Positive);
        assert!(Trit::Negative < Trit::Positive);
    }

    #[test]
    fn display_uses_sign_characters() {
        assert_eq!(Trit::Negative.to_string(), "-");
        assert_eq!(Trit::Zero.to_string(), "0");
        assert_eq!(Trit::Positive.to_string(), "+");
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(Trit::default(), Trit::Zero);
    }
}
