//! Visibility / export levels for top-level items.
//!
//! Triết uses three visibility levels (intentionally simpler than Rust's
//! five), per ADR-0005:
//!
//! - `Public` (`pub`) — visible from any module that can name this item.
//! - `PublicPkg` (`pub(pkg)`) — visible within the same crate-pack only.
//! - `Private` — visible only within the defining module (default).
//!
//! Items without an explicit modifier are `Private`. Visibility is
//! captured at parse time, consumed by the name resolver (v0.2.x module
//! loader) and the ABI surface generator (v0.4 Crate-Pack).

use std::fmt;

/// Visibility level for a top-level item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// `pub` — exported from the defining module.
    Public,
    /// `pub(pkg)` — visible within the same crate-pack only.
    PublicPkg,
    /// Default — visible only within the defining module.
    #[default]
    Private,
}

impl fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => formatter.write_str("pub"),
            Self::PublicPkg => formatter.write_str("pub(pkg)"),
            Self::Private => formatter.write_str("(private)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    #[test]
    fn display_is_readable() {
        assert_eq!(Visibility::Public.to_string(), "pub");
        assert_eq!(Visibility::PublicPkg.to_string(), "pub(pkg)");
        assert_eq!(Visibility::Private.to_string(), "(private)");
    }

    #[test]
    fn variants_distinct() {
        assert_ne!(Visibility::Public, Visibility::PublicPkg);
        assert_ne!(Visibility::Public, Visibility::Private);
        assert_ne!(Visibility::PublicPkg, Visibility::Private);
    }
}
