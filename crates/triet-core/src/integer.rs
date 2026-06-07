//! 27-trit balanced ternary integer — Triết's default integer type.

// `as` casts are unavoidable inside `const fn` until `From::from` is const-stable.
// Every `i128 → i64` truncation is guarded by an explicit range check at the
// call site; widening casts are equivalent to the corresponding `From` impl.
#![allow(clippy::cast_possible_truncation, clippy::cast_lossless)]

use std::{
    fmt,
    ops::{Add, Div, Mul, Neg, Rem, Sub},
};

use crate::error::{DivisionByZeroError, OverflowError};

/// 27-trit balanced ternary integer — Triết's default integer type.
///
/// Range: `[-3_812_798_742_493, +3_812_798_742_493]` (inclusive).
/// Stored internally as `i64` for native arithmetic.
///
/// Default arithmetic operators (`+`, `-`, `*`, `/`, `%`) panic on overflow.
/// Use the verb-first method variants to handle overflow explicitly:
/// - `add_and_truncate(other)` — wrap modulo `3²⁷`
/// - `add_and_saturate(other)` — clamp to `[MIN, MAX]`
/// - `try_add(other)` — return `Option<Integer>`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integer(i64);

impl Integer {
    /// Number of balanced ternary digits in an `Integer`.
    pub const TRIT_COUNT: u32 = 27;

    /// Number of distinct values: `3²⁷ = 7_625_597_484_987`.
    pub const MODULUS: i128 = 7_625_597_484_987;

    /// Half-modulus: `(3²⁷ - 1) / 2 = 3_812_798_742_493`.
    const HALF_MODULUS: i128 = (Self::MODULUS - 1) / 2;

    /// Smallest representable value.
    pub const MIN: Self = Self(-3_812_798_742_493);

    /// Largest representable value.
    pub const MAX: Self = Self(3_812_798_742_493);

    /// The value `0`.
    pub const ZERO: Self = Self(0);

    /// The value `+1`.
    pub const ONE: Self = Self(1);

    /// The value `-1`.
    pub const MINUS_ONE: Self = Self(-1);

    const TYPE_NAME: &'static str = "Integer";
    const MIN_STR: &'static str = "-3812798742493";
    const MAX_STR: &'static str = "3812798742493";

    /// Constructs an `Integer` from a signed 64-bit integer.
    ///
    /// Returns `None` if `value` falls outside `[MIN, MAX]`.
    #[inline]
    #[must_use]
    pub const fn new(value: i64) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Constructs an `Integer` from any signed 128-bit integer, saturating
    /// at the boundary if the input is out of range.
    #[inline]
    #[must_use]
    pub const fn new_saturating(value: i128) -> Self {
        if value < Self::MIN.0 as i128 {
            Self::MIN
        } else if value > Self::MAX.0 as i128 {
            Self::MAX
        } else {
            Self(value as i64)
        }
    }

    /// Returns this `Integer` as `i128`.
    #[inline]
    #[must_use]
    pub const fn to_i128(self) -> i128 {
        self.0 as i128
    }

    /// Returns the inner value as `i64`. The result is always in `[MIN, MAX]`.
    #[inline]
    #[must_use]
    pub const fn to_i64(self) -> i64 {
        self.0
    }

    /// Adds `other`, wrapping modulo `3²⁷` (truncation of high-order trits).
    #[must_use]
    pub const fn add_and_truncate(self, other: Self) -> Self {
        let sum = self.0 as i128 + other.0 as i128;
        Self(Self::wrap_to_range(sum) as i64)
    }

    /// Adds `other`, clamping the result at `MIN` or `MAX` on overflow.
    #[must_use]
    pub const fn add_and_saturate(self, other: Self) -> Self {
        let sum = self.0 as i128 + other.0 as i128;
        if sum > Self::HALF_MODULUS {
            Self::MAX
        } else if sum < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(sum as i64)
        }
    }

    /// Adds `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_add(self, other: Self) -> Option<Self> {
        let sum = self.0 as i128 + other.0 as i128;
        Self::checked_from_i128(sum)
    }

    /// Subtracts `other`, wrapping modulo `3²⁷`.
    #[must_use]
    pub const fn subtract_and_truncate(self, other: Self) -> Self {
        let diff = self.0 as i128 - other.0 as i128;
        Self(Self::wrap_to_range(diff) as i64)
    }

    /// Subtracts `other`, clamping at the boundary on overflow.
    #[must_use]
    pub const fn subtract_and_saturate(self, other: Self) -> Self {
        let diff = self.0 as i128 - other.0 as i128;
        if diff > Self::HALF_MODULUS {
            Self::MAX
        } else if diff < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(diff as i64)
        }
    }

    /// Subtracts `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_subtract(self, other: Self) -> Option<Self> {
        let diff = self.0 as i128 - other.0 as i128;
        Self::checked_from_i128(diff)
    }

    /// Multiplies by `other`, wrapping modulo `3²⁷`.
    #[must_use]
    pub const fn multiply_and_truncate(self, other: Self) -> Self {
        let product = self.0 as i128 * other.0 as i128;
        Self(Self::wrap_to_range(product) as i64)
    }

    /// Multiplies by `other`, clamping at the boundary on overflow.
    #[must_use]
    pub const fn multiply_and_saturate(self, other: Self) -> Self {
        let product = self.0 as i128 * other.0 as i128;
        if product > Self::HALF_MODULUS {
            Self::MAX
        } else if product < -Self::HALF_MODULUS {
            Self::MIN
        } else {
            Self(product as i64)
        }
    }

    /// Multiplies by `other`, returning `None` if the result would overflow.
    #[must_use]
    pub const fn try_multiply(self, other: Self) -> Option<Self> {
        let product = self.0 as i128 * other.0 as i128;
        Self::checked_from_i128(product)
    }

    /// Divides by `other`.
    ///
    /// Division in balanced ternary rounds the quotient toward the nearest
    /// representable value (no bias) — see SPEC.md §3.2. This implementation
    /// uses Rust's `i64` integer division semantics, which truncates toward
    /// zero; balanced-ternary-correct rounding is left to a follow-up.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Integer::ZERO`.
    pub const fn try_divide(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == 0 {
            return Err(DivisionByZeroError);
        }
        Ok(Self(self.0 / other.0))
    }

    /// Returns the remainder after balanced-ternary division.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Integer::ZERO`.
    pub const fn try_modulo(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == 0 {
            return Err(DivisionByZeroError);
        }
        Ok(Self(self.0 % other.0))
    }

    /// Maps an `i128` value into `[MIN, MAX]` by modular reduction.
    #[inline]
    const fn wrap_to_range(value: i128) -> i128 {
        let shifted = value + Self::HALF_MODULUS;
        let modulated = shifted.rem_euclid(Self::MODULUS);
        modulated - Self::HALF_MODULUS
    }

    /// Returns `Some(Integer)` if `value` is in range, else `None`.
    #[inline]
    const fn checked_from_i128(value: i128) -> Option<Self> {
        if value >= Self::MIN.0 as i128 && value <= Self::MAX.0 as i128 {
            Some(Self(value as i64))
        } else {
            None
        }
    }
}

impl Neg for Integer {
    type Output = Self;

    /// Negates the value. Never overflows — balanced ternary range is
    /// symmetric around zero (see SPEC.md §3.2 invariant 1).
    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for Integer {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        self.try_add(other)
            .expect("Integer addition overflow; use add_and_truncate / add_and_saturate / try_add")
    }
}

impl Sub for Integer {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        self.try_subtract(other).expect(
            "Integer subtraction overflow; use subtract_and_truncate / subtract_and_saturate / try_subtract",
        )
    }
}

impl Mul for Integer {
    type Output = Self;

    fn mul(self, other: Self) -> Self::Output {
        self.try_multiply(other).expect(
            "Integer multiplication overflow; use multiply_and_truncate / multiply_and_saturate / try_multiply",
        )
    }
}

impl Div for Integer {
    type Output = Self;

    fn div(self, other: Self) -> Self::Output {
        self.try_divide(other).expect("Integer division by zero")
    }
}

impl Rem for Integer {
    type Output = Self;

    fn rem(self, other: Self) -> Self::Output {
        self.try_modulo(other).expect("Integer modulo by zero")
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl TryFrom<i128> for Integer {
    type Error = OverflowError;

    fn try_from(value: i128) -> Result<Self, Self::Error> {
        i64::try_from(value)
            .ok()
            .and_then(Self::new)
            .ok_or(OverflowError {
                type_name: Self::TYPE_NAME,
                min: Self::MIN_STR,
                max: Self::MAX_STR,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_definition() {
        assert_eq!(Integer::TRIT_COUNT, 27);
        assert_eq!(Integer::MODULUS, 7_625_597_484_987);
        assert_eq!(Integer::MIN.to_i64(), -3_812_798_742_493);
        assert_eq!(Integer::MAX.to_i64(), 3_812_798_742_493);
    }

    #[test]
    fn range_is_symmetric_around_zero() {
        assert_eq!(Integer::MIN.to_i64(), -Integer::MAX.to_i64());
    }

    #[test]
    fn negate_max_does_not_overflow() {
        assert_eq!(-Integer::MAX, Integer::MIN);
        assert_eq!(-Integer::MIN, Integer::MAX);
    }

    #[test]
    fn double_negation_is_identity() {
        for value in [
            -3_812_798_742_493_i64,
            -1_000_000,
            -1,
            0,
            1,
            1_000_000,
            3_812_798_742_493,
        ] {
            let n = Integer::new(value).unwrap();
            assert_eq!(-(-n), n);
        }
    }

    #[test]
    fn new_accepts_in_range_values() {
        assert!(Integer::new(0).is_some());
        assert!(Integer::new(3_812_798_742_493).is_some());
        assert!(Integer::new(-3_812_798_742_493).is_some());
    }

    #[test]
    fn new_rejects_out_of_range_values() {
        assert!(Integer::new(3_812_798_742_494).is_none());
        assert!(Integer::new(-3_812_798_742_494).is_none());
        assert!(Integer::new(i64::MAX).is_none());
    }

    #[test]
    fn add_in_range_succeeds() {
        let a = Integer::new(1_000_000).unwrap();
        let b = Integer::new(2_000_000).unwrap();
        assert_eq!((a + b).to_i64(), 3_000_000);
    }

    #[test]
    #[should_panic(expected = "Integer addition overflow")]
    fn add_overflow_panics() {
        let _ = Integer::MAX + Integer::ONE;
    }

    #[test]
    fn try_add_returns_none_on_overflow() {
        assert_eq!(Integer::MAX.try_add(Integer::ONE), None);
        assert_eq!(Integer::MAX.try_add(Integer::MAX), None);
    }

    #[test]
    fn add_and_saturate_clamps() {
        assert_eq!(Integer::MAX.add_and_saturate(Integer::ONE), Integer::MAX);
        assert_eq!(
            Integer::MIN.add_and_saturate(Integer::MINUS_ONE),
            Integer::MIN,
        );
    }

    #[test]
    fn add_and_truncate_wraps() {
        assert_eq!(Integer::MAX.add_and_truncate(Integer::ONE), Integer::MIN);
        assert_eq!(
            Integer::MIN.add_and_truncate(Integer::MINUS_ONE),
            Integer::MAX
        );
    }

    #[test]
    fn multiply_overflow_handled_in_variants() {
        let big = Integer::new(2_000_000_000).unwrap();
        // 2e9 * 2e9 = 4e18 exceeds Integer max (3.8e12)
        assert_eq!(big.try_multiply(big), None);
        assert_eq!(big.multiply_and_saturate(big), Integer::MAX);
    }

    #[test]
    fn try_divide_by_zero_returns_error() {
        assert_eq!(
            Integer::ONE.try_divide(Integer::ZERO),
            Err(DivisionByZeroError),
        );
    }

    #[test]
    fn try_from_i128_succeeds_in_range() {
        assert_eq!(
            Integer::try_from(3_812_798_742_493_i128).unwrap(),
            Integer::MAX
        );
        assert_eq!(Integer::try_from(0_i128).unwrap(), Integer::ZERO);
    }

    #[test]
    fn try_from_i128_fails_out_of_range() {
        assert!(Integer::try_from(i128::MAX).is_err());
        assert!(Integer::try_from(3_812_798_742_494_i128).is_err());
    }

    #[test]
    fn negate_addition_yields_zero() {
        for value in [-3_812_798_742_493_i64, -1, 0, 1, 3_812_798_742_493] {
            let n = Integer::new(value).unwrap();
            assert_eq!((n + (-n)).to_i64(), 0);
        }
    }

    #[test]
    fn zero_is_additive_identity() {
        for value in [-1_000_000_i64, 0, 1_000_000] {
            let n = Integer::new(value).unwrap();
            assert_eq!(n + Integer::ZERO, n);
        }
    }

    #[test]
    fn one_is_multiplicative_identity() {
        for value in [-1_000_000_i64, 0, 1_000_000] {
            let n = Integer::new(value).unwrap();
            assert_eq!(n * Integer::ONE, n);
        }
    }

    // ── Overflow panic tests (v0.3 safety audit) ──────────────────

    #[test]
    #[should_panic(expected = "overflow")]
    fn integer_add_overflow_panics() {
        let _ = Integer::MAX + Integer::ONE;
    }

    #[test]
    #[should_panic(expected = "overflow")]
    fn integer_sub_overflow_panics() {
        let _ = Integer::MIN - Integer::ONE;
    }

    #[test]
    #[should_panic(expected = "overflow")]
    fn integer_mul_overflow_panics() {
        let big = Integer::new(2_000_000_000).unwrap();
        let _ = big * big;
    }

    #[test]
    #[should_panic(expected = "zero")]
    fn integer_div_by_zero_panics() {
        let _ = Integer::ONE / Integer::ZERO;
    }

    #[test]
    #[should_panic(expected = "zero")]
    fn integer_rem_by_zero_panics() {
        let _ = Integer::ONE % Integer::ZERO;
    }

    #[test]
    fn integer_negate_min_is_max() {
        // Balanced ternary: -MIN == MAX (no overflow unlike 2's complement)
        assert_eq!(-Integer::MIN, Integer::MAX);
    }

    #[test]
    fn integer_balanced_range_is_symmetric() {
        assert_eq!(-Integer::MAX, Integer::MIN);
        assert_eq!(Integer::MAX.to_i64(), -Integer::MIN.to_i64());
    }

    /// ADR-0044 canary: MAX computed as literal == (3^27 - 1) / 2
    /// computed from actual 3^27. If someone changes MODULUS without
    /// updating MAX (or vice versa), this goes red.
    #[test]
    fn integer_range_canary_modulus_consistent() {
        // 3^27
        let three_pow_27: i128 = 3_i128.pow(Integer::TRIT_COUNT);
        assert_eq!(three_pow_27, Integer::MODULUS);
        // MAX = (3^27 - 1) / 2
        let max_from_formula: i128 = (three_pow_27 - 1) / 2;
        assert_eq!(max_from_formula, Integer::MAX.to_i64() as i128);
        // MIN = -MAX (balanced ternary symmetry)
        let min_from_formula: i128 = -max_from_formula;
        assert_eq!(min_from_formula, Integer::MIN.to_i64() as i128);
    }
}
