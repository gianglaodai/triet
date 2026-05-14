//! 81-trit balanced ternary integer.
//!
//! Range: `[-(3⁸¹-1)/2, (3⁸¹-1)/2]` ≈ `±2.21 × 10³⁸`. Backed by
//! `bnum::types::I256` (256-bit signed, fixed-size, no heap allocation).
//! 256 bits comfortably covers the Long range with room for intermediate
//! products (`MAX² ≈ 4.9 × 10⁷⁶`, well under `2²⁵⁵`) before reduction.
//!
//! See SPEC.md §2.1 for the type-system view and §3.3 for arithmetic
//! semantics. `Long` mirrors `Integer`'s API surface so the interpreter
//! can dispatch numeric methods uniformly across `Tryte` / `Integer` /
//! `Long`.

use std::{
    fmt,
    ops::{Add, Div, Mul, Neg, Rem, Sub},
};

use bnum::cast::As;
use bnum::types::I256;

use crate::error::DivisionByZeroError;
use crate::integer::Integer;
use crate::tryte::Tryte;

// ---------------------------------------------------------------------------
// Internal const constants
//
// `bnum` keeps `I256::ZERO/ONE/NEG_ONE` `pub(crate)`, and the only public
// const factory for arbitrary values is `from_str_radix`. We materialise
// the constants we need (zero/one/half-modulus/modulus/bounds) once at
// compile time via `match` on the parser's `Result` — `Result::expect`
// isn't required, and the `match`-with-panic form is const-stable.
// ---------------------------------------------------------------------------

const fn parse_i256(text: &str) -> I256 {
    match I256::from_str_radix(text, 10) {
        Ok(value) => value,
        Err(_) => panic!("invalid I256 literal — fix the source string"),
    }
}

const ZERO_RAW: I256 = parse_i256("0");
const ONE_RAW: I256 = parse_i256("1");
const NEG_ONE_RAW: I256 = parse_i256("-1");
const TWO_RAW: I256 = parse_i256("2");

/// `(3⁸¹ - 1) / 2`.
const HALF_MODULUS_RAW: I256 = parse_i256("221713244121518884974124815309574946401");

/// `3⁸¹` — the cardinality of the balanced ternary 81-trit space.
const MODULUS_RAW: I256 = parse_i256("443426488243037769948249630619149892803");

const MIN_RAW: I256 = parse_i256("-221713244121518884974124815309574946401");
const MAX_RAW: I256 = HALF_MODULUS_RAW;

/// 81-trit balanced ternary integer.
///
/// Range: `±221_713_244_121_518_884_974_124_815_309_574_946_401`
/// (≈ `±2.21 × 10³⁸`). Exceeds the `i128` range, so all internal
/// arithmetic uses 256-bit signed integers (`bnum::types::I256`).
///
/// Default arithmetic operators (`+`, `-`, `*`, `/`, `%`) panic on
/// overflow. Use the verb-first method variants to handle overflow
/// explicitly:
/// - `add_and_truncate(other)` — wrap modulo `3⁸¹`
/// - `add_and_saturate(other)` — clamp to `[MIN, MAX]`
/// - `try_add(other)` — return `Option<Long>`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Long(I256);

impl Long {
    /// Number of balanced ternary digits in a `Long`.
    pub const TRIT_COUNT: u32 = 81;

    /// Type name string for diagnostics. Mirrors the convention used by
    /// `Integer`/`Tryte` even though `Long` doesn't currently raise an
    /// `OverflowError` against an `i128` source.
    pub const TYPE_NAME: &'static str = "Long";

    /// Decimal string for the inclusive minimum
    /// (`-(3⁸¹-1)/2 = -221_713_244_121_518_884_974_124_815_309_574_946_401`).
    pub const MIN_STR: &'static str = "-221713244121518884974124815309574946401";

    /// Decimal string for the inclusive maximum
    /// (`+(3⁸¹-1)/2 = +221_713_244_121_518_884_974_124_815_309_574_946_401`).
    pub const MAX_STR: &'static str = "221713244121518884974124815309574946401";

    /// The value `0`.
    pub const ZERO: Self = Self(ZERO_RAW);

    /// The value `+1`.
    pub const ONE: Self = Self(ONE_RAW);

    /// The value `-1`.
    pub const MINUS_ONE: Self = Self(NEG_ONE_RAW);

    /// The smallest representable value.
    pub const MIN: Self = Self(MIN_RAW);

    /// The largest representable value.
    pub const MAX: Self = Self(MAX_RAW);

    /// Constructs a `Long` from a signed 64-bit integer. Always succeeds —
    /// `i64::MAX` (≈ 9.2 × 10¹⁸) sits well inside Long's range.
    #[inline]
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        Self(value.as_())
    }

    /// Constructs a `Long` from a signed 128-bit integer. Always succeeds —
    /// `i128::MAX` (≈ 1.7 × 10³⁸) sits inside Long's range
    /// (≈ 2.2 × 10³⁸).
    #[inline]
    #[must_use]
    pub fn from_i128(value: i128) -> Self {
        Self(value.as_())
    }

    /// Constructs a `Long` from any `i128`, returning `None` if the
    /// value is outside `[MIN, MAX]`. Provided for API symmetry; in
    /// practice every `i128` fits in a `Long`.
    #[inline]
    #[must_use]
    pub fn new(value: i128) -> Option<Self> {
        Self::checked_from_i256(value.as_())
    }

    /// Returns the value as an `i128`, panicking if outside `i128` range.
    /// Use [`try_to_i128`](Self::try_to_i128) for a checked variant.
    ///
    /// # Panics
    ///
    /// Panics if `self` is outside the `i128` range. Long can hold
    /// values up to `±2.21 × 10³⁸` while `i128` only reaches `±1.7 × 10³⁸`.
    #[must_use]
    pub fn to_i128(self) -> i128 {
        self.try_to_i128()
            .expect("Long value outside i128 range; use try_to_i128")
    }

    /// Returns the value as an `i128`, or `None` if outside `i128` range.
    #[must_use]
    pub fn try_to_i128(self) -> Option<i128> {
        let max_i128: I256 = i128::MAX.as_();
        let min_i128: I256 = i128::MIN.as_();
        if self.0 > max_i128 || self.0 < min_i128 {
            None
        } else {
            Some(self.0.as_())
        }
    }

    /// Adds `other`, wrapping modulo `3⁸¹`.
    #[must_use]
    pub fn add_and_truncate(self, other: Self) -> Self {
        Self(Self::wrap_to_range(self.0 + other.0))
    }

    /// Adds `other`, clamping the result at `MIN` or `MAX` on overflow.
    #[must_use]
    pub fn add_and_saturate(self, other: Self) -> Self {
        Self(Self::clamp_to_range(self.0 + other.0))
    }

    /// Adds `other`, returning `None` if the result would overflow.
    #[must_use]
    pub fn try_add(self, other: Self) -> Option<Self> {
        Self::checked_from_i256(self.0 + other.0)
    }

    /// Subtracts `other`, wrapping modulo `3⁸¹`.
    #[must_use]
    pub fn subtract_and_truncate(self, other: Self) -> Self {
        Self(Self::wrap_to_range(self.0 - other.0))
    }

    /// Subtracts `other`, clamping at the boundary on overflow.
    #[must_use]
    pub fn subtract_and_saturate(self, other: Self) -> Self {
        Self(Self::clamp_to_range(self.0 - other.0))
    }

    /// Subtracts `other`, returning `None` if the result would overflow.
    #[must_use]
    pub fn try_subtract(self, other: Self) -> Option<Self> {
        Self::checked_from_i256(self.0 - other.0)
    }

    /// Multiplies by `other`, wrapping modulo `3⁸¹`.
    ///
    /// The intermediate product can reach roughly 5 × 10⁷⁶, well below
    /// the `I256` range — overflow is detected by range-check after the
    /// multiplication (or via `wrap_to_range` for the truncating form).
    #[must_use]
    pub fn multiply_and_truncate(self, other: Self) -> Self {
        Self(Self::wrap_to_range(self.0 * other.0))
    }

    /// Multiplies by `other`, clamping at the boundary on overflow.
    #[must_use]
    pub fn multiply_and_saturate(self, other: Self) -> Self {
        Self(Self::clamp_to_range(self.0 * other.0))
    }

    /// Multiplies by `other`, returning `None` if the result would overflow.
    #[must_use]
    pub fn try_multiply(self, other: Self) -> Option<Self> {
        Self::checked_from_i256(self.0 * other.0)
    }

    /// Divides by `other`.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Long::ZERO`.
    pub fn try_divide(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == ZERO_RAW {
            return Err(DivisionByZeroError);
        }
        Ok(Self(self.0 / other.0))
    }

    /// Returns the remainder after division.
    ///
    /// # Errors
    ///
    /// Returns `DivisionByZeroError` if `other` is `Long::ZERO`.
    pub fn try_modulo(self, other: Self) -> Result<Self, DivisionByZeroError> {
        if other.0 == ZERO_RAW {
            return Err(DivisionByZeroError);
        }
        Ok(Self(self.0 % other.0))
    }

    /// Convert to `Integer`, panicking on overflow.
    ///
    /// # Panics
    ///
    /// Panics if `self` falls outside `Integer`'s range
    /// (`±3_812_798_742_493`). Use [`try_to_integer`](Self::try_to_integer)
    /// or [`to_integer_and_saturate`](Self::to_integer_and_saturate) for
    /// non-panicking alternatives.
    #[must_use]
    pub fn to_integer(self) -> Integer {
        self.try_to_integer()
            .expect("Long → Integer overflow; use try_to_integer or to_integer_and_saturate")
    }

    /// Convert to `Integer`, returning `None` on overflow.
    #[must_use]
    pub fn try_to_integer(self) -> Option<Integer> {
        self.try_to_i128().and_then(|v| Integer::try_from(v).ok())
    }

    /// Convert to `Integer`, clamping at the boundary on overflow.
    ///
    /// # Panics
    ///
    /// Does not panic on overflow (the bound check guards the
    /// conversion); the inner `expect` is dead-code documentation
    /// for the post-clamp invariant.
    #[must_use]
    pub fn to_integer_and_saturate(self) -> Integer {
        let integer_max: I256 = Integer::MAX.to_i128().as_();
        let integer_min: I256 = Integer::MIN.to_i128().as_();
        if self.0 > integer_max {
            Integer::MAX
        } else if self.0 < integer_min {
            Integer::MIN
        } else {
            self.try_to_integer()
                .expect("value is within Integer range after the bound check")
        }
    }

    /// Convert to `Tryte`, panicking on overflow.
    ///
    /// # Panics
    ///
    /// Panics if `self` falls outside `Tryte`'s range (`±9_841`).
    /// Use [`try_to_tryte`](Self::try_to_tryte) for the non-panicking
    /// alternative.
    #[must_use]
    pub fn to_tryte(self) -> Tryte {
        self.try_to_tryte()
            .expect("Long → Tryte overflow; use try_to_tryte")
    }

    /// Convert to `Tryte`, returning `None` on overflow.
    #[must_use]
    pub fn try_to_tryte(self) -> Option<Tryte> {
        self.try_to_i128()
            .and_then(|v| i64::try_from(v).ok())
            .and_then(|v| Tryte::try_from(v).ok())
    }

    fn wrap_to_range(value: I256) -> I256 {
        // Shift up by HALF_MODULUS, take modulo MODULUS to land in
        // `[0, MODULUS)`, shift back down. `rem_euclid` keeps the result
        // non-negative, which is what we want before subtracting the
        // half-modulus.
        let shifted = value + HALF_MODULUS_RAW;
        let modulated = shifted.rem_euclid(MODULUS_RAW);
        modulated - HALF_MODULUS_RAW
    }

    fn clamp_to_range(value: I256) -> I256 {
        if value > MAX_RAW {
            MAX_RAW
        } else if value < MIN_RAW {
            MIN_RAW
        } else {
            value
        }
    }

    fn checked_from_i256(value: I256) -> Option<Self> {
        if value > MAX_RAW || value < MIN_RAW {
            None
        } else {
            Some(Self(value))
        }
    }
}

// Suppress an unused-const warning if a refactor temporarily removes a
// caller — TWO_RAW currently unused but a useful primitive constant.
#[allow(dead_code)]
const _UNUSED_TWO: I256 = TWO_RAW;

impl From<Integer> for Long {
    /// Lossless: every `Integer` value fits in a `Long`.
    fn from(value: Integer) -> Self {
        Self::from_i128(value.to_i128())
    }
}

impl From<Tryte> for Long {
    /// Lossless: every `Tryte` value fits in a `Long`.
    fn from(value: Tryte) -> Self {
        Self::from_i64(value.to_i64())
    }
}

impl From<i64> for Long {
    fn from(value: i64) -> Self {
        Self::from_i64(value)
    }
}

impl From<i128> for Long {
    fn from(value: i128) -> Self {
        Self::from_i128(value)
    }
}

impl Neg for Long {
    type Output = Self;

    /// Negates the value. Never overflows — balanced ternary range is
    /// symmetric around zero (SPEC.md §3.2 invariant 1).
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for Long {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        self.try_add(other)
            .expect("Long addition overflow; use add_and_truncate / add_and_saturate / try_add")
    }
}

impl Sub for Long {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        self.try_subtract(other).expect(
            "Long subtraction overflow; use subtract_and_truncate / subtract_and_saturate / try_subtract",
        )
    }
}

impl Mul for Long {
    type Output = Self;

    fn mul(self, other: Self) -> Self::Output {
        self.try_multiply(other).expect(
            "Long multiplication overflow; use multiply_and_truncate / multiply_and_saturate / try_multiply",
        )
    }
}

impl Div for Long {
    type Output = Self;

    fn div(self, other: Self) -> Self::Output {
        self.try_divide(other).expect("Long division by zero")
    }
}

impl Rem for Long {
    type Output = Self;

    fn rem(self, other: Self) -> Self::Output {
        self.try_modulo(other).expect("Long modulo by zero")
    }
}

impl fmt::Display for Long {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// `TryFrom<i128>` is provided by std's blanket `impl<T, U> TryFrom<U>
// for T where U: Into<T>` — every `i128` is `Into<Long>` via the
// `From<i128>` impl above, so the resulting `try_from` returns
// `Result<Long, Infallible>`. We do not provide a custom error-typed
// variant: an `i128` *always* fits a `Long` (Long range ≈ ±2.21×10³⁸
// strictly contains `i128` range ≈ ±1.7×10³⁸), so an `OverflowError`
// shape would be dead code.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_definition() {
        assert_eq!(Long::TRIT_COUNT, 81);
        assert_eq!(Long::ZERO.to_i128(), 0);
        assert_eq!(Long::ONE.to_i128(), 1);
        assert_eq!(Long::MINUS_ONE.to_i128(), -1);
    }

    #[test]
    fn range_is_symmetric_around_zero() {
        assert_eq!(Long::MIN, -Long::MAX);
        assert_eq!(-Long::MIN, Long::MAX);
    }

    #[test]
    fn min_max_decimal_strings_match_three_to_the_eighty_one() {
        // 3⁸¹ = 443_426_488_243_037_769_948_249_630_619_149_892_803
        // (3⁸¹ - 1) / 2 = 221_713_244_121_518_884_974_124_815_309_574_946_401
        let max_str = Long::MAX.to_string();
        let min_str = Long::MIN.to_string();
        assert_eq!(max_str, "221713244121518884974124815309574946401");
        assert_eq!(min_str, "-221713244121518884974124815309574946401");
    }

    #[test]
    fn from_i64_round_trips() {
        for value in [0_i64, 1, -1, 1_000_000, -1_000_000, i64::MAX, i64::MIN] {
            let long = Long::from_i64(value);
            assert_eq!(long.to_i128(), i128::from(value));
        }
    }

    #[test]
    fn from_i128_round_trips() {
        for value in [0_i128, 1, -1, i128::MAX, i128::MIN] {
            let long = Long::from_i128(value);
            assert_eq!(long.to_i128(), value);
        }
    }

    #[test]
    fn negate_max_does_not_overflow() {
        assert_eq!(-Long::MAX, Long::MIN);
        assert_eq!(-Long::MIN, Long::MAX);
    }

    #[test]
    fn double_negation_is_identity() {
        for value in [
            -1_i128,
            0,
            1,
            1_000_000_000_000,
            -1_000_000_000_000,
            i128::MAX,
        ] {
            let n = Long::from_i128(value);
            assert_eq!(-(-n), n);
        }
    }

    // === Default arithmetic ===

    #[test]
    fn add_in_range_succeeds() {
        let a = Long::from_i64(1_000_000);
        let b = Long::from_i64(2_000_000);
        assert_eq!((a + b).to_i128(), 3_000_000);
    }

    #[test]
    fn add_at_long_scale_succeeds() {
        // 10³⁸ + 10³⁸ = 2·10³⁸ — overflows `i128` (≈1.7·10³⁸) but fits
        // comfortably inside `Long` (≈2.2·10³⁸). This is the load-bearing
        // case for using a 256-bit backing instead of `i128`.
        let billion_pow_4_then_some: i128 = 100_000_000_000_000_000_000_000_000_000_000_000_000;
        // 10^38; but i128::MAX ≈ 1.7·10^38, so `10^38` itself fits in i128.
        let a = Long::from_i128(billion_pow_4_then_some);
        let result = a + a;
        let expected = "200000000000000000000000000000000000000";
        assert_eq!(result.to_string(), expected);
    }

    #[test]
    #[should_panic(expected = "Long addition overflow")]
    fn add_overflow_panics() {
        let _ = Long::MAX + Long::ONE;
    }

    #[test]
    #[should_panic(expected = "Long subtraction overflow")]
    fn sub_overflow_panics() {
        let _ = Long::MIN - Long::ONE;
    }

    #[test]
    #[should_panic(expected = "Long multiplication overflow")]
    fn mul_overflow_panics() {
        let _ = Long::MAX * Long::from_i64(2);
    }

    #[test]
    #[should_panic(expected = "Long division by zero")]
    fn div_by_zero_panics() {
        let _ = Long::ONE / Long::ZERO;
    }

    // === try_* variants ===

    #[test]
    fn try_add_returns_none_on_overflow() {
        assert_eq!(Long::MAX.try_add(Long::ONE), None);
        assert_eq!(Long::MIN.try_add(Long::MINUS_ONE), None);
    }

    #[test]
    fn try_add_returns_some_in_range() {
        let a = Long::from_i64(5);
        let b = Long::from_i64(7);
        assert_eq!(a.try_add(b).unwrap().to_i128(), 12);
    }

    #[test]
    fn try_divide_by_zero_returns_error() {
        assert_eq!(Long::ONE.try_divide(Long::ZERO), Err(DivisionByZeroError));
    }

    // === _and_saturate variants ===

    #[test]
    fn add_and_saturate_clamps_at_max() {
        assert_eq!(Long::MAX.add_and_saturate(Long::ONE), Long::MAX);
        assert_eq!(Long::MAX.add_and_saturate(Long::MAX), Long::MAX);
    }

    #[test]
    fn add_and_saturate_clamps_at_min() {
        assert_eq!(Long::MIN.add_and_saturate(Long::MINUS_ONE), Long::MIN);
    }

    // === _and_truncate variants ===

    #[test]
    fn add_and_truncate_wraps_past_max() {
        // MAX + 1 wraps to MIN (modulus 3⁸¹).
        assert_eq!(Long::MAX.add_and_truncate(Long::ONE), Long::MIN);
    }

    #[test]
    fn add_and_truncate_wraps_past_min() {
        // MIN - 1 wraps to MAX.
        assert_eq!(Long::MIN.add_and_truncate(Long::MINUS_ONE), Long::MAX);
    }

    // === Cross-type conversions ===

    #[test]
    fn from_integer_is_lossless() {
        assert_eq!(Long::from(Integer::MAX).to_i128(), Integer::MAX.to_i128());
        assert_eq!(Long::from(Integer::MIN).to_i128(), Integer::MIN.to_i128());
        assert_eq!(Long::from(Integer::ZERO), Long::ZERO);
    }

    #[test]
    fn from_tryte_is_lossless() {
        assert_eq!(Long::from(Tryte::MAX).to_i128(), 9_841);
        assert_eq!(Long::from(Tryte::MIN).to_i128(), -9_841);
    }

    #[test]
    fn try_to_integer_returns_none_when_out_of_range() {
        // i128::MAX is below Long::MAX but above Integer::MAX.
        let big = Long::from_i128(i128::MAX);
        assert!(big.try_to_integer().is_none());
    }

    #[test]
    fn try_to_integer_returns_some_in_range() {
        let value = Long::from_i64(42);
        assert_eq!(value.try_to_integer().unwrap(), Integer::new(42).unwrap());
    }

    #[test]
    fn to_integer_and_saturate_clamps() {
        assert_eq!(Long::MAX.to_integer_and_saturate(), Integer::MAX);
        assert_eq!(Long::MIN.to_integer_and_saturate(), Integer::MIN);
        assert_eq!(
            Long::from_i64(42).to_integer_and_saturate(),
            Integer::new(42).unwrap(),
        );
    }

    #[test]
    #[should_panic(expected = "Long → Integer overflow")]
    fn to_integer_panics_when_out_of_range() {
        let _ = Long::MAX.to_integer();
    }

    #[test]
    fn try_to_tryte_returns_some_in_range() {
        assert_eq!(
            Long::from_i64(100).try_to_tryte().unwrap(),
            Tryte::new(100).unwrap(),
        );
    }

    #[test]
    fn try_to_tryte_returns_none_out_of_range() {
        assert!(Long::from_i64(10_000).try_to_tryte().is_none());
    }

    // === From i128 (always succeeds — Long range strictly contains i128) ===

    #[test]
    fn from_i128_succeeds_at_boundaries() {
        let max = Long::from(i128::MAX);
        let min = Long::from(i128::MIN);
        assert_eq!(max.to_i128(), i128::MAX);
        assert_eq!(min.to_i128(), i128::MIN);
        // Both boundaries sit strictly inside Long's range.
        assert!(max < Long::MAX);
        assert!(min > Long::MIN);
    }

    // === Algebraic properties ===

    #[test]
    fn negate_addition_yields_zero() {
        for value in [-1_i128, 0, 1, i128::MAX, i128::MIN] {
            let n = Long::from_i128(value);
            assert_eq!(n + (-n), Long::ZERO);
        }
    }

    #[test]
    fn zero_is_additive_identity() {
        for value in [-1_000_000_i64, 0, 1_000_000] {
            let n = Long::from_i64(value);
            assert_eq!(n + Long::ZERO, n);
            assert_eq!(Long::ZERO + n, n);
        }
    }

    #[test]
    fn one_is_multiplicative_identity() {
        for value in [-1_000_000_i64, 0, 1_000_000] {
            let n = Long::from_i64(value);
            assert_eq!(n * Long::ONE, n);
        }
    }

    // === Display ===

    #[test]
    fn display_is_decimal() {
        assert_eq!(Long::from_i64(42).to_string(), "42");
        assert_eq!(Long::from_i64(-42).to_string(), "-42");
        assert_eq!(Long::ZERO.to_string(), "0");
    }

    // ── Overflow panic tests (v0.3 safety audit) ──────────────────

    #[test]
    #[should_panic(expected = "overflow")]
    fn long_add_overflow_panics() {
        let _ = Long::MAX + Long::ONE;
    }

    #[test]
    #[should_panic(expected = "overflow")]
    fn long_sub_overflow_panics() {
        let _ = Long::MIN - Long::ONE;
    }

    #[test]
    #[should_panic(expected = "overflow")]
    fn long_mul_overflow_panics() {
        let two = Long::from_i64(2);
        let _ = Long::MAX * two;
    }

    #[test]
    #[should_panic(expected = "zero")]
    fn long_div_by_zero_panics() {
        let _ = Long::ONE / Long::ZERO;
    }

    #[test]
    fn long_negate_min_is_max() {
        assert_eq!(-Long::MIN, Long::MAX);
    }

    #[test]
    fn long_balanced_range_is_symmetric() {
        assert_eq!(-Long::MAX, Long::MIN);
    }

    #[test]
    fn long_i64_always_fits() {
        assert_eq!(Long::from_i64(i64::MAX).to_string(), i64::MAX.to_string());
        assert_eq!(Long::from_i64(i64::MIN).to_string(), i64::MIN.to_string());
    }
}
