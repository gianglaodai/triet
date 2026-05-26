//! Triết core: balanced ternary types and arithmetic.
//!
//! Defines the foundational integer types — `Trit`, `Tryte`, `Integer`,
//! `Long` — and balanced ternary arithmetic. See `SPEC.md` §2 and §3.
//!
//! # Type hierarchy
//!
//! All integer types use balanced ternary `{-1, 0, +1}` digits. The hierarchy
//! follows powers of three:
//!
//! | Type      | Trits | Range                            |
//! |-----------|-------|----------------------------------|
//! | `Trit`    | 1     | `{-1, 0, +1}`                    |
//! | `Tryte`   | 9     | `±9_841`                         |
//! | `Integer` | 27    | `±3_812_798_742_493`             |
//! | `Long`    | 81    | very large                       |
//!
//! # Overflow handling
//!
//! Default arithmetic operators panic on overflow (fail-fast). Method
//! variants exist for each non-panicking strategy:
//! - `*_and_truncate` — wrap modulo `3ⁿ`
//! - `*_and_saturate` — clamp to `[MIN, MAX]`
//! - `try_*` — return `Option<Self>`

#![warn(missing_docs)]

mod error;
mod integer;
mod long;
pub mod memory;
mod trit;
mod tryte;

pub use error::{DivisionByZeroError, OverflowError, ParseError};
pub use integer::Integer;
pub use long::Long;
pub use memory::ObjectHeader;
pub use trit::Trit;
pub use tryte::Tryte;
