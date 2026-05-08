//! Error types for `triet-core` operations.

use thiserror::Error;

/// Error returned when an arithmetic operation overflows the type's range.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq, Hash)]
#[error("balanced ternary overflow in {type_name}: result outside [{min}, {max}]")]
pub struct OverflowError {
    /// Name of the type that overflowed (e.g. `"Tryte"`, `"Integer"`).
    pub type_name: &'static str,
    /// Inclusive minimum of the type's range.
    pub min: i128,
    /// Inclusive maximum of the type's range.
    pub max: i128,
}

/// Error returned when parsing a value into a ternary type fails.
#[derive(Clone, Debug, Error, PartialEq, Eq, Hash)]
pub enum ParseError {
    /// Source string was empty.
    #[error("empty input")]
    Empty,

    /// Character outside the balanced ternary alphabet (`+`, `0`, `-`, `_`).
    #[error("invalid character {0:?} (expected `+`, `0`, `-`, or `_`)")]
    InvalidCharacter(char),

    /// Parsed value falls outside the target type's range.
    #[error("value {value} outside range of {type_name}")]
    OutOfRange {
        /// The value that was parsed.
        value: i128,
        /// Name of the target type.
        type_name: &'static str,
    },
}

/// Error returned when dividing by zero.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq, Hash)]
#[error("division by zero")]
pub struct DivisionByZeroError;
