//! Triết logic: three-valued logic with Łukasiewicz Ł3 (default) and
//! Kleene K3 variants.
//!
//! Defines the truth type [`Trilean`] and its operations. See SPEC.md §4.
//!
//! # Logic systems
//!
//! - **Universal** (identical in Ł3 and K3): `not`, `and`, `or`
//! - **Łukasiewicz Ł3** (default): `implies`, `iff`, `xor`
//! - **Kleene K3** (alternative): `kleene_implies`, `kleene_iff`, `kleene_xor`
//!
//! The two systems differ only at `Unknown`/`Unknown` cases of implication
//! (and derived operators iff/xor).

#![warn(missing_docs)]

mod trilean;

pub use trilean::Trilean;
