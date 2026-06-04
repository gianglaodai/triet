//! `Display` for the schema-generated [`Visibility`] enum.
//!
//! `Visibility` itself is generated from the schema
//! (`crate::generated::types::Visibility`) and re-exported at the crate root —
//! it is the single canonical type. Codegen does not emit a `Display` impl, so
//! we provide one here (rendering matches ADR-0005 surface syntax).
//!
//! Triết uses three visibility levels (intentionally simpler than Rust's five),
//! per ADR-0005: `Public` (`public`), `PublicPackage` (`public(package)`), and
//! `Private` (default — no modifier).

use std::fmt;

pub use crate::generated::types::Visibility;

impl fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => formatter.write_str("public"),
            Self::PublicPackage => formatter.write_str("public(package)"),
            Self::Private => formatter.write_str("(private)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_readable() {
        assert_eq!(Visibility::Public.to_string(), "public");
        assert_eq!(Visibility::PublicPackage.to_string(), "public(package)");
        assert_eq!(Visibility::Private.to_string(), "(private)");
    }

    #[test]
    fn variants_distinct() {
        assert_ne!(Visibility::Public, Visibility::PublicPackage);
        assert_ne!(Visibility::Public, Visibility::Private);
        assert_ne!(Visibility::PublicPackage, Visibility::Private);
    }
}
