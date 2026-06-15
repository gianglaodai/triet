//! BLAKE3 hash helpers for ABI metadata — 3-cấp hash tree per ADR-0014.
//!
//! ADR-0014 §1 locks three identity levels: **term** (per export
//! function/type), **module** (rollup of terms within a module path),
//! **package** (rollup of modules + deps + caps). Each level has an
//! `iface_hash` (signature-only, stable across re-builds of same
//! surface) and an `impl_hash` (covers iface + body bytes).
//!
//! v0.5.3 ships the iface tree fully and the impl tree structurally;
//! per-term body bytes feed in once `.triv` v4 (per-term offset index)
//! lands at v0.5.4. Until then, `impl_hash_term`/`impl_hash_mod` are
//! computed from empty body bytes (deterministic but signature-only).
//! Pkg-level `impl_hash` keeps the v0.4 formula
//! `BLAKE3(iface_hash_pkg ‖ code_section)` so it still detects code
//! changes — switched to the module rollup once `.triv` v4 lands.
//!
//! Domain separators (ADR-0014 §6) prevent collisions between levels —
//! a term name happening to equal a module path can't accidentally
//! produce the same digest.
//!
//! [ADR-0014]: ../../../docs/decisions/0014-hash-scheme-refinement.md

use crate::types::AbiMetadata;

/// BLAKE3 output is 32 bytes per the spec. ADR-0011 hard-codes this.
pub const IFACE_HASH_LEN: usize = 32;
/// `impl_hash` reuses the same 32-byte BLAKE3 width.
pub const IMPL_HASH_LEN: usize = 32;

// ── Domain separators (ADR-0014 §6) ────────────────────────────────
//
// Each constant is exactly 16 bytes of ASCII + trailing NUL pad. Lock
// these strings — changing one byte invalidates every hash ever
// computed and requires an `abi_version` bump.

const DOMAIN_TERM_IFACE: &[u8; 16] = b"triet/term-i  \0\0";
const DOMAIN_TERM_IMPL: &[u8; 16] = b"triet/term-m  \0\0";
const DOMAIN_MOD_IFACE: &[u8; 16] = b"triet/mod-i   \0\0";
const DOMAIN_MOD_IMPL: &[u8; 16] = b"triet/mod-m   \0\0";
const DOMAIN_PKG_IFACE: &[u8; 16] = b"triet/pkg-i   \0\0";
const DOMAIN_PKG_IMPL: &[u8; 16] = b"triet/pkg-m   \0\0";

// ── Newtype hash wrappers (one per identity level) ──────────────────
//
// Six newtypes look verbose but each level is a distinct kind of
// identity. Mixing a `TermIfaceHash` into a `ModuleImplHash` slot is a
// type error caught at compile time — VISION §6 "explicit > implicit".

/// 32-byte BLAKE3 digest for a term's interface (signature only).
/// ADR-0014 §2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TermIfaceHash(pub [u8; IFACE_HASH_LEN]);

/// 32-byte BLAKE3 digest for a term's implementation (iface + body
/// bytes). ADR-0014 §2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TermImplHash(pub [u8; IMPL_HASH_LEN]);

/// 32-byte BLAKE3 digest for a module's interface (rollup of term
/// iface hashes within the module). ADR-0014 §3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModuleIfaceHash(pub [u8; IFACE_HASH_LEN]);

/// 32-byte BLAKE3 digest for a module's implementation (rollup of term
/// impl hashes). ADR-0014 §3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModuleImplHash(pub [u8; IMPL_HASH_LEN]);

/// 32-byte BLAKE3 digest for the package ABI surface. ADR-0014 §4.
/// Linker uses this as the final arbiter (ADR-0013 §4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IfaceHash(pub [u8; IFACE_HASH_LEN]);

/// 32-byte BLAKE3 digest covering package iface + IR code bytes.
/// ADR-0014 §4 spec is module rollup; v0.5.3 uses the v0.4 formula
/// (`BLAKE3(iface_hash ‖ code_section)`) until per-term bodies land
/// at v0.5.4. Format-stable either way (32 bytes in the same slot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImplHash(pub [u8; IMPL_HASH_LEN]);

// ── Hash-newtype helpers ────────────────────────────────────────────

macro_rules! hash_newtype_impls {
    ($name:ident, $len:expr) => {
        impl $name {
            /// Construct from raw bytes. Useful for tests + dep-pin parsing.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; $len]) -> Self {
                Self(bytes)
            }

            /// True if every byte is zero — the canonical "no pin" /
            /// "not yet computed" sentinel.
            #[must_use]
            pub fn is_zero(&self) -> bool {
                self.0.iter().all(|b| *b == 0)
            }
        }
    };
}

hash_newtype_impls!(TermIfaceHash, IFACE_HASH_LEN);
hash_newtype_impls!(TermImplHash, IMPL_HASH_LEN);
hash_newtype_impls!(ModuleIfaceHash, IFACE_HASH_LEN);
hash_newtype_impls!(ModuleImplHash, IMPL_HASH_LEN);
hash_newtype_impls!(IfaceHash, IFACE_HASH_LEN);
hash_newtype_impls!(ImplHash, IMPL_HASH_LEN);

// ── Compute functions, one per (level × variant) ────────────────────

/// Compute a term's iface hash from its canonical signature bytes.
///
/// Caller is responsible for producing canonical signature bytes per
/// ADR-0014 §2 (term_kind, name, visibility, type parameters, body). The
/// serializer in `serde::canonical_term_signature` does this.
#[must_use]
pub fn compute_term_iface_hash(canonical_signature_bytes: &[u8]) -> TermIfaceHash {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_TERM_IFACE);
    h.update(canonical_signature_bytes);
    let mut out = [0u8; IFACE_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    TermIfaceHash(out)
}

/// Compute a term's impl hash = `BLAKE3(domain ‖ iface_hash ‖ body)`.
///
/// `body_bytes` is the canonical IR encoding of the term's body. v0.5.3
/// callers pass `&[]` until `.triv` v4 (v0.5.4) carries per-term
/// offsets — same function signature, no API churn.
#[must_use]
pub fn compute_term_impl_hash(iface: TermIfaceHash, body_bytes: &[u8]) -> TermImplHash {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_TERM_IMPL);
    h.update(&iface.0);
    h.update(body_bytes);
    let mut out = [0u8; IMPL_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    TermImplHash(out)
}

/// Compute a module's iface hash from its path + sorted term iface
/// hashes. ADR-0014 §3.
///
/// `terms` is `(term_name, iface_hash)` pairs. The function sorts
/// internally — caller order doesn't matter.
#[must_use]
pub fn compute_module_iface_hash(
    module_path: &str,
    terms: &[(String, TermIfaceHash)],
) -> ModuleIfaceHash {
    let mut sorted: Vec<&(String, TermIfaceHash)> = terms.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_MOD_IFACE);
    write_lp_string(&mut h, module_path);
    for (name, hash) in sorted {
        write_lp_string(&mut h, name);
        h.update(&hash.0);
    }
    let mut out = [0u8; IFACE_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    ModuleIfaceHash(out)
}

/// Compute a module's impl hash = `BLAKE3(domain ‖ iface_hash_mod ‖
/// sorted term impl hashes)`. ADR-0014 §3.
#[must_use]
pub fn compute_module_impl_hash(
    iface: ModuleIfaceHash,
    terms: &[(String, TermImplHash)],
) -> ModuleImplHash {
    let mut sorted: Vec<&(String, TermImplHash)> = terms.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_MOD_IMPL);
    h.update(&iface.0);
    for (name, hash) in sorted {
        write_lp_string(&mut h, name);
        h.update(&hash.0);
    }
    let mut out = [0u8; IMPL_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    ModuleImplHash(out)
}

/// Compute the package iface hash by rolling up module iface hashes
/// per ADR-0014 §4.
///
/// Hash inputs (in order):
/// 1. domain separator
/// 2. `pkg_name` (length-prefixed)
/// 3. sorted module entries (path + `iface_hash_mod`)
/// 4. deps table bytes (canonical, per ADR-0011 §4)
/// 5. caps table bytes (canonical, per ADR-0011 §5)
///
/// Replaces the v0.4 formula (which hashed flat types+exports). The
/// new rollup makes pkg iface depend on module identity, enabling the
/// v0.5 CAS dedup at module granularity.
#[must_use]
pub fn compute_iface_hash(meta: &AbiMetadata) -> IfaceHash {
    let mut sorted: Vec<&crate::types::Module> = meta.modules.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));

    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_PKG_IFACE);
    write_lp_string(&mut h, &meta.pkg_name);
    for m in sorted {
        write_lp_string(&mut h, &m.path);
        h.update(&m.iface_hash_mod.0);
    }
    // Deps + caps bytes are produced canonically by `serde` to keep
    // the encoding co-located with read/write logic.
    let deps_bytes = crate::serde::encode_deps_for_hash(&meta.deps);
    let caps_bytes = crate::serde::encode_caps_for_hash(&meta.caps);
    h.update(&deps_bytes);
    h.update(&caps_bytes);

    let mut out = [0u8; IFACE_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    IfaceHash(out)
}

/// Compute the package impl hash.
///
/// v0.5.3 uses the v0.4 formula `BLAKE3(domain ‖ iface_hash ‖
/// code_section)` so the digest still reflects code-section changes
/// even while per-term bodies aren't extractable. v0.5.4 will switch
/// to the ADR-0014 §4 module rollup once `.triv` v4 lands per-term
/// offsets — same 32-byte slot in `.khi`, no format churn.
#[must_use]
pub(crate) fn compute_impl_hash(iface: &IfaceHash, code_section: &[u8]) -> ImplHash {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_PKG_IMPL);
    h.update(&iface.0);
    h.update(code_section);
    let mut out = [0u8; IMPL_HASH_LEN];
    out.copy_from_slice(h.finalize().as_bytes());
    ImplHash(out)
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Write a length-prefixed UTF-8 string into a hasher. Length is u32
/// little-endian to match `serde::write_string` semantics — keeps
/// hash inputs aligned with on-disk encoding.
fn write_lp_string(h: &mut blake3::Hasher, s: &str) {
    let bytes = s.as_bytes();
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    h.update(&len.to_le_bytes());
    h.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Module, SemVer};

    /// Domain separators are distinct across all six levels — chosen
    /// 16-byte strings shouldn't accidentally collide.
    #[test]
    fn domain_separators_are_unique() {
        let all = [
            DOMAIN_TERM_IFACE,
            DOMAIN_TERM_IMPL,
            DOMAIN_MOD_IFACE,
            DOMAIN_MOD_IMPL,
            DOMAIN_PKG_IFACE,
            DOMAIN_PKG_IMPL,
        ];
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j], "separators {i} and {j} collide");
            }
        }
    }

    /// Domain separation prevents same-bytes-different-level from
    /// producing identical digests. A term iface and a term impl with
    /// the same input bytes must differ — proves the separator does
    /// something.
    #[test]
    fn domain_separation_actually_separates() {
        let bytes = b"hello world";
        let iface = compute_term_iface_hash(bytes);
        // impl hash with the same input bytes treated as body
        let impl_h = compute_term_impl_hash(TermIfaceHash::default(), bytes);
        assert_ne!(iface.0, impl_h.0);
    }

    /// Term iface hash is deterministic: same input → same output.
    #[test]
    fn term_iface_hash_deterministic() {
        let a = compute_term_iface_hash(b"function add(a: Int, b: Int) -> Int");
        let b = compute_term_iface_hash(b"function add(a: Int, b: Int) -> Int");
        assert_eq!(a, b);
    }

    /// Different signature → different term hash.
    #[test]
    fn term_iface_hash_changes_with_signature() {
        let a = compute_term_iface_hash(b"function add(a: Int, b: Int) -> Int");
        let b = compute_term_iface_hash(b"function sub(a: Int, b: Int) -> Int");
        assert_ne!(a, b);
    }

    /// Module hash is order-independent thanks to internal sort.
    #[test]
    fn module_hash_order_independent() {
        let alpha = (
            "alpha".to_owned(),
            TermIfaceHash::from_bytes([1u8; IFACE_HASH_LEN]),
        );
        let beta = (
            "beta".to_owned(),
            TermIfaceHash::from_bytes([2u8; IFACE_HASH_LEN]),
        );
        let h1 = compute_module_iface_hash("khi.foo", &[alpha.clone(), beta.clone()]);
        let h2 = compute_module_iface_hash("khi.foo", &[beta, alpha]);
        assert_eq!(h1, h2);
    }

    /// Module path is part of the hash — same terms, different module
    /// → different hash.
    #[test]
    fn module_hash_separates_by_path() {
        let term = (
            "f".to_owned(),
            TermIfaceHash::from_bytes([3u8; IFACE_HASH_LEN]),
        );
        let h1 = compute_module_iface_hash("khi.foo", std::slice::from_ref(&term));
        let h2 = compute_module_iface_hash("khi.bar", &[term]);
        assert_ne!(h1, h2);
    }

    /// Package hash rolls up from modules and includes pkg name.
    #[test]
    fn pkg_iface_hash_uses_modules() {
        let mut a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        a.modules.push(Module {
            path: "foo.core".into(),
            iface_hash_mod: ModuleIfaceHash::from_bytes([7u8; IFACE_HASH_LEN]),
            impl_hash_mod: ModuleImplHash::default(),
        });
        let h_a = compute_iface_hash(&a);

        // Same modules, different pkg name → different hash.
        let mut b = a.clone();
        b.pkg_name = "bar".into();
        let h_b = compute_iface_hash(&b);
        assert_ne!(h_a, h_b);

        // Same pkg name, no modules → different hash again.
        let h_empty = compute_iface_hash(&AbiMetadata::empty("foo", SemVer::new(1, 0, 0)));
        assert_ne!(h_a, h_empty);
    }

    /// Pkg iface hash ignores `pkg_version` — surface stable across
    /// patch bumps. (Carry-over invariant from v0.4.)
    #[test]
    fn pkg_iface_hash_ignores_pkg_version() {
        let a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        let b = AbiMetadata::empty("foo", SemVer::new(9, 9, 9));
        assert_eq!(compute_iface_hash(&a), compute_iface_hash(&b));
    }

    /// Zero hash sentinel works for every level.
    #[test]
    fn zero_hash_sentinel_per_level() {
        assert!(TermIfaceHash::default().is_zero());
        assert!(TermImplHash::default().is_zero());
        assert!(ModuleIfaceHash::default().is_zero());
        assert!(ModuleImplHash::default().is_zero());
        assert!(IfaceHash::default().is_zero());
        assert!(ImplHash::default().is_zero());

        let nonzero = TermIfaceHash::from_bytes([1; IFACE_HASH_LEN]);
        assert!(!nonzero.is_zero());
    }
}
