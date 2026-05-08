//! 9-trit balanced ternary integer.

// `as` casts are unavoidable inside `const fn` until `From::from` is const-stable.
// Every `i32 → i16` truncation is guarded by an explicit range check at the
// call site; widening casts are equivalent to the corresponding `From` impl.
#![allow(clippy::cast_possible_truncation, clippy::cast_lossless)]

use std::{
    fmt,
    ops::{Add, Div, Mul, Neg, Rem, Sub},
};

use crate::error::{DivisionByZeroError, OverflowError};

/// 9-trit balanced ternary integer.
///
/// Range: `[-9_841, +9_841]` (inclusive). Stored internally as `i16` for
/// efficient native arithmetic.
///
/// Default arithmetic operators (`+`, `-`, `*`, `/`, `%`) panic on overflow.
/// Use the verb-first method variants to handle overflow explicitly:
/// - `add_and_truncate(other)` — wrap modulo 3⁹
/// - `add_and_saturate(other)` — clamp to `[MIN, MAX]`
/// - `try_add(other)` — return `Option<Tryte>`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tryte(i16);

impl Tryte {
    /// Number of balanced ternary digits in a `Tryte`.
    pub const TRIT_COUNT: u32 = 9;

    /// Number of distinct values: `3⁹ = 19_683`.
    pub const MODULUS: i32 = 19_683;

    /// Half-modulus: `(3⁹ - 1) / 2 = 9_841`.
    const HALF_MODULUS: i32 = (Self::MODULUS - 1) / 2;

    /// Smallest representable value: `-9_841`.
    pub const MIN: Self = Self(-9_841);

    /// Largest representable value: `+9_841`.
    pub const MAX: Self = Self(9_841);

    /// The value `0`.
    pub const ZERO: Self = Self(0);

    /// The value `+1`.
    pub const ONE: Self = Self(1);

    /// The value `-1`.
    pub const MINUS_ONE: Self = Self(-1);

    const TYPE_NAME: &'static str = "Tryte";

    /// Constructs a `Tryte` from a signed integer.
    ///
    /// Returns `None` if `value` is outside `[-9_841, +9_841]`.
    #[inline]
    #[must_use]
    pub const fn new(value: i16) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Constructs a `Tryte` from any signed integer, saturating at the
    /// boundary if the input is out of range.
    #[inline]
    #[must_use]
    pub const fn new_saturating(value: i64) -> Self {
        if value < Self::MIN.0 as i64 {
            Self::MIN
        } else if value > Self::MAX.0 as i64 {
            Self::MAX
        } else {
            Self(value as i16)
        }
    }

    /// Returns this `Tryte` as a signed 64-bit integer.
    #[inline]
    #[must_use]
    pub const fn to_i64(self) -> i64 {
        self.0 as i64
    }

    /// Returns the inner value as `i16`. The result is always in
    /// `[-9_841, +9_841]`.
    #[inline]
    #[must_use]
    pub const fn to_i16(self) -> i16 {
        self.0
    }

    /// Adds `other`, wrapping modulo `3⁹` (truncation of high-order trits).
    ///
    /// Always succeeds; the result re-enters the balanced ternary range
    /// from the opposite end on overflow.
    #[must_use]
    pub const fn add_and_truncate(self, other: Self) -> Self {
        let sum = self.0 as i32 + other.0 as i32;
        Self(Self::wrap_to_range(sum) as i16)
    }

    /// Adds `other`, clamping the result at `MIN` or `MAX` on overflow.
    #[must_use]
    pub const fn add_and_saturate(self, other: Self) -> Self {
        let sum = self.0 as i32 + other.0 as i32;
        if sum > Self::HALF_MODULUS {
            Self::MAX
        } else if sum < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(sum as i16)
        }
    }

    /// Adds `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_add(self, other: Self) -> Option<Self> {
        let sum = self.0 as i32 + other.0 as i32;
        Self::checked_from_i32(sum)
    }

    /// Subtracts `other`, wrapping modulo `3⁹`.
    #[must_use]
    pub const fn subtract_and_truncate(self, other: Self) -> Self {
        let diff = self.0 as i32 - other.0 as i32;
        Self(Self::wrap_to_range(diff) as i16)
    }

    /// Subtracts `other`, clamping at the boundary on overflow.
    #[must_use]
    pub const fn subtract_and_saturate(self, other: Self) -> Self {
        let diff = self.0 as i32 - other.0 as i32;
        if diff > Self::HALF_MODULUS {
            Self::MAX
        } else if diff < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(diff as i16)
        }
    }

    /// Subtracts `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_subtract(self, other: Self) -> Option<Self> {
        let diff = self.0 as i32 - other.0 as i32;
        Self::checked_from_i32(diff)
    }

    /// Multiplies by `other`, wrapping modulo `3⁹`.
    #[must_use]
    pub const fn multiply_and_truncate(self, other: Self) -> Self {
        let product = self.0 as i32 * other.0 as i32;
        Self(Self::wrap_to_range(product) as i16)
    }

    /// Multiplies by `other`, clamping at the boundary on overflow.
    #[must_use]
    pub const fn multiply_and_saturate(self, other: Self) -> Self {
        let product = self.0 as i32 * other.0 as i32;
        if product > Self::HALF_MODULUS {
            Self::MAX
        } else if product < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(product as i16)
        }
    }

    /// Multiplies by `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_multiply(self, other: Self) -> Option<Self> {
        let product = self.0 as i32 * other.0 as i32;
        Self::checked_from_i32(product)
    }

    /// Divides by `other`, returning `Err` on division by zero.
    ///
    /// Division in balanced ternary rounds the quotient toward the nearest
    /// representable value (no bias) — see SPEC.md §3.2. This implementation
    /// uses Rust's `i32` integer division semantics, which truncates toward
    /// zero; balanced-ternary-correct rounding is left to a follow-up.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Tryte::ZERO`.
    pub const fn try_divide(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == 0 {
            return Err(DivisionByZeroError);
        }
        // Quotient of two values in [-9_841, 9_841] never exceeds the range.
        Ok(Self(self.0 / other.0))
    }

    /// Returns the remainder after balanced-ternary division.
    ///
    /// Sign convention: result has the same sign as the dividend (Rust
    /// default). True balanced-ternary modulo is a follow-up.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Tryte::ZERO`.
    pub const fn try_modulo(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == 0 {
            return Err(DivisionByZeroError);
        }
        Ok(Self(self.0 % other.0))
    }

    /// Maps an `i32` value into `[-9_841, +9_841]` by modular reduction
    /// (used by `*_and_truncate` operations).
    #[inline]
    const fn wrap_to_range(value: i32) -> i32 {
        let shifted = value + Self::HALF_MODULUS;
        let modulated = shifted.rem_euclid(Self::MODULUS);
        modulated - Self::HALF_MODULUS
    }

    /// Returns `Some(Tryte)` if `value` is in range, else `None`.
    #[inline]
    const fn checked_from_i32(value: i32) -> Option<Self> {
        if value >= Self::MIN.0 as i32 && value <= Self::MAX.0 as i32 {
            Some(Self(value as i16))
        } else {
            None
        }
    }
}

impl Neg for Tryte {
    type Output = Self;

    /// Negates the value. Never overflows: balanced ternary range is
    /// symmetric around zero (see SPEC.md §3.2 invariant 1).
    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for Tryte {
    type Output = Self;

    /// Adds two `Tryte`s, panicking on overflow.
    ///
    /// Use `add_and_truncate`, `add_and_saturate`, or `try_add` for
    /// non-panicking variants.
    fn add(self, other: Self) -> Self::Output {
        self.try_add(other)
            .expect("Tryte addition overflow; use add_and_truncate / add_and_saturate / try_add")
    }
}

impl Sub for Tryte {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        self.try_subtract(other).expect(
            "Tryte subtraction overflow; use subtract_and_truncate / subtract_and_saturate / try_subtract",
        )
    }
}

impl Mul for Tryte {
    type Output = Self;

    fn mul(self, other: Self) -> Self::Output {
        self.try_multiply(other).expect(
            "Tryte multiplication overflow; use multiply_and_truncate / multiply_and_saturate / try_multiply",
        )
    }
}

impl Div for Tryte {
    type Output = Self;

    fn div(self, other: Self) -> Self::Output {
        self.try_divide(other).expect("Tryte division by zero")
    }
}

impl Rem for Tryte {
    type Output = Self;

    fn rem(self, other: Self) -> Self::Output {
        self.try_modulo(other).expect("Tryte modulo by zero")
    }
}

impl fmt::Display for Tryte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl TryFrom<i64> for Tryte {
    type Error = OverflowError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        i16::try_from(value).ok().and_then(Self::new).ok_or(OverflowError {
            type_name: Self::TYPE_NAME,
            min: Self::MIN.0 as i128,
            max: Self::MAX.0 as i128,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_definition() {
        assert_eq!(Tryte::TRIT_COUNT, 9);
        assert_eq!(Tryte::MODULUS, 19_683);
        assert_eq!(Tryte::MIN.to_i64(), -9_841);
        assert_eq!(Tryte::MAX.to_i64(), 9_841);
        assert_eq!(Tryte::ZERO.to_i64(), 0);
    }

    #[test]
    fn range_is_symmetric_around_zero() {
        assert_eq!(Tryte::MIN.to_i64(), -Tryte::MAX.to_i64());
    }

    #[test]
    fn new_accepts_in_range_values() {
        assert!(Tryte::new(0).is_some());
        assert!(Tryte::new(9_841).is_some());
        assert!(Tryte::new(-9_841).is_some());
    }

    #[test]
    fn new_rejects_out_of_range_values() {
        assert!(Tryte::new(9_842).is_none());
        assert!(Tryte::new(-9_842).is_none());
        assert!(Tryte::new(i16::MAX).is_none());
        assert!(Tryte::new(i16::MIN).is_none());
    }

    // === Negation ===

    #[test]
    fn negate_max_does_not_overflow() {
        // Critical balanced ternary property: range is symmetric, so
        // negate(MAX) = MIN and vice versa, no overflow possible.
        assert_eq!(-Tryte::MAX, Tryte::MIN);
        assert_eq!(-Tryte::MIN, Tryte::MAX);
    }

    #[test]
    fn double_negation_is_identity() {
        for value in [-9_841, -100, -1, 0, 1, 100, 9_841] {
            let t = Tryte::new(value).unwrap();
            assert_eq!(-(-t), t);
        }
    }

    // === Default arithmetic (panic on overflow) ===

    #[test]
    fn add_in_range_succeeds() {
        let a = Tryte::new(100).unwrap();
        let b = Tryte::new(200).unwrap();
        assert_eq!((a + b).to_i64(), 300);
    }

    #[test]
    #[should_panic(expected = "Tryte addition overflow")]
    fn add_overflow_panics() {
        let _ = Tryte::MAX + Tryte::ONE;
    }

    #[test]
    #[should_panic(expected = "Tryte subtraction overflow")]
    fn sub_overflow_panics() {
        let _ = Tryte::MIN - Tryte::ONE;
    }

    #[test]
    #[should_panic(expected = "Tryte multiplication overflow")]
    fn mul_overflow_panics() {
        let _ = Tryte::new(1000).unwrap() * Tryte::new(1000).unwrap();
    }

    #[test]
    #[should_panic(expected = "Tryte division by zero")]
    fn div_by_zero_panics() {
        let _ = Tryte::ONE / Tryte::ZERO;
    }

    // === try_* variants ===

    #[test]
    fn try_add_returns_none_on_overflow() {
        assert_eq!(Tryte::MAX.try_add(Tryte::ONE), None);
        assert_eq!(Tryte::MIN.try_add(Tryte::MINUS_ONE), None);
        assert_eq!(Tryte::MAX.try_add(Tryte::MAX), None);
    }

    #[test]
    fn try_add_returns_some_in_range() {
        let a = Tryte::new(5).unwrap();
        let b = Tryte::new(7).unwrap();
        assert_eq!(a.try_add(b).unwrap().to_i64(), 12);
    }

    #[test]
    fn try_divide_by_zero_returns_error() {
        assert_eq!(Tryte::ONE.try_divide(Tryte::ZERO), Err(DivisionByZeroError));
    }

    // === _and_saturate variants ===

    #[test]
    fn add_and_saturate_clamps_at_max() {
        assert_eq!(Tryte::MAX.add_and_saturate(Tryte::ONE), Tryte::MAX);
        assert_eq!(Tryte::MAX.add_and_saturate(Tryte::MAX), Tryte::MAX);
    }

    #[test]
    fn add_and_saturate_clamps_at_min() {
        assert_eq!(Tryte::MIN.add_and_saturate(Tryte::MINUS_ONE), Tryte::MIN);
    }

    #[test]
    fn add_and_saturate_passes_through_in_range() {
        let a = Tryte::new(100).unwrap();
        let b = Tryte::new(200).unwrap();
        assert_eq!(a.add_and_saturate(b).to_i64(), 300);
    }

    // === _and_truncate variants ===

    #[test]
    fn add_and_truncate_wraps_past_max() {
        // 9_841 + 1 wraps to -9_841 (modulus 19_683)
        assert_eq!(Tryte::MAX.add_and_truncate(Tryte::ONE), Tryte::MIN);
    }

    #[test]
    fn add_and_truncate_wraps_past_min() {
        // -9_841 - 1 wraps to +9_841
        assert_eq!(Tryte::MIN.add_and_truncate(Tryte::MINUS_ONE), Tryte::MAX);
    }

    #[test]
    fn add_and_truncate_passes_through_in_range() {
        let a = Tryte::new(100).unwrap();
        let b = Tryte::new(200).unwrap();
        assert_eq!(a.add_and_truncate(b).to_i64(), 300);
    }

    #[test]
    fn add_truncate_is_modular() {
        // Adding the modulus should be a no-op
        // 5 + 19_683 should wrap back to 5, but 19_683 itself isn't
        // representable. So test that wrap is consistent: a + (b mod 19_683) = (a + b) mod 19_683.
        let a = Tryte::new(7_000).unwrap();
        let b = Tryte::new(7_000).unwrap();
        // 7_000 + 7_000 = 14_000, which exceeds MAX (9_841)
        // Wrap: 14_000 + 9_841 = 23_841, mod 19_683 = 4_158, - 9_841 = -5_683
        assert_eq!(a.add_and_truncate(b).to_i64(), -5_683);
    }

    // === try_from ===

    #[test]
    fn try_from_i64_succeeds_in_range() {
        assert_eq!(Tryte::try_from(0_i64).unwrap(), Tryte::ZERO);
        assert_eq!(Tryte::try_from(9_841_i64).unwrap(), Tryte::MAX);
    }

    #[test]
    fn try_from_i64_fails_out_of_range() {
        let err = Tryte::try_from(i64::MAX).unwrap_err();
        assert_eq!(err.type_name, "Tryte");
    }

    // === Display ===

    #[test]
    fn display_is_decimal() {
        assert_eq!(Tryte::new(25).unwrap().to_string(), "25");
        assert_eq!(Tryte::new(-9_841).unwrap().to_string(), "-9841");
    }

    // === Algebraic properties ===

    #[test]
    fn addition_is_commutative_in_range() {
        for (a, b) in [(5, 7), (-100, 50), (9_000, -1_000)] {
            let ta = Tryte::new(a).unwrap();
            let tb = Tryte::new(b).unwrap();
            assert_eq!(ta + tb, tb + ta);
        }
    }

    #[test]
    fn negate_addition_yields_zero() {
        for value in [-9_841, -100, 0, 100, 9_841] {
            let t = Tryte::new(value).unwrap();
            assert_eq!((t + (-t)).to_i64(), 0);
        }
    }

    #[test]
    fn zero_is_additive_identity() {
        for value in [-9_841, -1, 0, 1, 9_841] {
            let t = Tryte::new(value).unwrap();
            assert_eq!(t + Tryte::ZERO, t);
            assert_eq!(Tryte::ZERO + t, t);
        }
    }

    #[test]
    fn one_is_multiplicative_identity() {
        for value in [-9_841, -1, 0, 1, 9_841] {
            let t = Tryte::new(value).unwrap();
            assert_eq!(t * Tryte::ONE, t);
            assert_eq!(Tryte::ONE * t, t);
        }
    }
}
