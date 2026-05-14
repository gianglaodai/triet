//! BLAKE3 hash helpers for ABI metadata.
//!
//! Per [ADR-0011 §1 + §6], every `.tripack` carries two BLAKE3 digests:
//!
//! - `iface_hash` covers the canonical ABI surface (types + exports +
//!   deps + caps). It must stay stable across re-compiles when the
//!   surface didn't change, so we hash a normalized form that omits
//!   `abi_version`, `pkg_version`, and `impl_hash` themselves.
//! - `impl_hash` covers `iface_hash` + the IR code bytes. v0.5 CAS uses
//!   it to dedup identical artefacts when the surface stays still.
//!
//! [ADR-0011 §1 + §6]: ../../../docs/decisions/0011-abi-metadata-format.md

use crate::types::AbiMetadata;

/// BLAKE3 output is 32 bytes per the spec. ADR-0011 hard-codes this.
pub const IFACE_HASH_LEN: usize = 32;
/// `impl_hash` reuses the same 32-byte BLAKE3 width.
pub const IMPL_HASH_LEN: usize = 32;

/// Strong-typed 32-byte digest for ABI surface (cannot be mixed up
/// with `ImplHash` at compile time).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IfaceHash(pub [u8; IFACE_HASH_LEN]);

/// Strong-typed 32-byte digest covering ABI + IR code bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImplHash(pub [u8; IMPL_HASH_LEN]);

impl IfaceHash {
    /// Construct from raw bytes. Useful for tests + dep-pin parsing.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; IFACE_HASH_LEN]) -> Self {
        Self(bytes)
    }

    /// True if every byte is zero — the canonical "no pin" sentinel
    /// used by [`crate::Dep::iface_hash_pin`].
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }
}

impl ImplHash {
    /// Construct from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; IMPL_HASH_LEN]) -> Self {
        Self(bytes)
    }
}

/// Compute `iface_hash` over the canonical ABI surface of `meta`.
///
/// Hash inputs (in order):
/// 1. `pkg_name` (length-prefixed bytes)
/// 2. `types` table (length-prefix + each entry's bytes)
/// 3. `exports` table
/// 4. `deps` table
/// 5. `caps` table
///
/// Explicitly excludes `abi_version`, `pkg_version`, `iface_hash`, and
/// `impl_hash` themselves — per ADR-0011 §6, these fields change per
/// commit even when the surface doesn't, so they'd defeat hash
/// stability.
///
/// Entries are pre-sorted by name in the canonical write path (see
/// `serde::canonicalize_for_hash`) so logically identical surfaces
/// produce identical bytes regardless of source order.
#[must_use]
pub fn compute_iface_hash(meta: &AbiMetadata) -> IfaceHash {
    let mut hasher = blake3::Hasher::new();
    let bytes = crate::serde::encode_iface_for_hash(meta);
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; IFACE_HASH_LEN];
    out.copy_from_slice(digest.as_bytes());
    IfaceHash(out)
}

/// Compute `impl_hash` = BLAKE3(`iface_hash` bytes || IR code bytes).
///
/// `code_section` is the raw bytes of the `.triv` IR section embedded
/// in the same `.tripack`. The caller is responsible for passing the
/// final canonical bytes (after any encoding/compression).
///
/// Currently only used internally by `serde::write_tripack`; promoted
/// to public when the linker (v0.4.5) needs to verify a downloaded
/// pack against its declared hash.
#[must_use]
pub(crate) fn compute_impl_hash(iface: &IfaceHash, code_section: &[u8]) -> ImplHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&iface.0);
    hasher.update(code_section);
    let digest = hasher.finalize();
    let mut out = [0u8; IMPL_HASH_LEN];
    out.copy_from_slice(digest.as_bytes());
    ImplHash(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SemVer;

    /// Two metadata blocks with the same surface must produce the
    /// same `iface_hash` even when `pkg_version` differs.
    #[test]
    fn iface_hash_ignores_pkg_version() {
        let mut a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        let mut b = AbiMetadata::empty("foo", SemVer::new(9, 9, 9));
        a.iface_hash = compute_iface_hash(&a);
        b.iface_hash = compute_iface_hash(&b);
        assert_eq!(a.iface_hash, b.iface_hash);
    }

    /// Changing the package name does change the hash.
    #[test]
    fn iface_hash_changes_with_pkg_name() {
        let a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        let b = AbiMetadata::empty("bar", SemVer::new(1, 0, 0));
        assert_ne!(compute_iface_hash(&a), compute_iface_hash(&b));
    }

    /// The zero hash is the canonical "no pin" sentinel.
    #[test]
    fn iface_hash_zero_detection() {
        let zero = IfaceHash::default();
        assert!(zero.is_zero());
        let nonzero = IfaceHash::from_bytes([1; IFACE_HASH_LEN]);
        assert!(!nonzero.is_zero());
    }
}
