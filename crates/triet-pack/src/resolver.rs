//! Hash-based dependency resolver — layer on top of [`Store`] + an
//! optional [`Lockfile`].
//!
//! Resolution flow per ADR-0014 + ADR-0015 §5:
//!
//! 1. For each declared dep, check the lockfile first. If present, use
//!    the pinned `(version, iface_hash, impl_hash)`. Authoritative.
//! 2. Otherwise enumerate matching versions in the store, pick the
//!    highest in range, record the chosen entry into the lockfile.
//! 3. If a dep declares its own `iface_hash_pin` (non-zero), that
//!    overrides the lockfile — declaration wins over build-cache.
//!
//! The resolver doesn't touch the network. v0.5 store is local-only;
//! distributed registry is v1.0+ work.
//!
//! [`Store`]: crate::Store
//! [`Lockfile`]: crate::Lockfile

use crate::error::StoreError;
use crate::hash::ImplHash;
use crate::lockfile::{LockEntry, Lockfile};
use crate::store::Store;
use crate::types::{Dep, SemVer};

/// Why the resolver picked this particular pack — one of three
/// distinct decision paths. Kept as a 3-state enum rather than a
/// `bool` so callers (telemetry, capability gates, diagnostics) can
/// distinguish *lockfile-authoritative*, *iface-pin-authoritative*,
/// and *fresh-by-range* origins without re-deriving the path.
///
/// Aligns with VISION §5 *bản sắc tam phân* — resolution decisions
/// are ternary by nature; binary `from_lockfile` collapsed two cases
/// (pin-match vs. plain enumeration) into one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResolutionOrigin {
    /// Pinned by an existing `triet.lock` entry whose hash is still
    /// installed. Authoritative — no enumeration was performed.
    Lockfile,
    /// `dep.iface_hash_pin` was non-zero. Resolver enumerated store
    /// versions and picked the highest one whose `iface_hash` matches
    /// the pin. ADR-0013 declaration-wins-over-cache.
    IfacePin,
    /// No lockfile entry, no iface pin — resolver picked the highest
    /// installed version satisfying the dep's semver range.
    Fresh,
}

/// Outcome for one declared dep — which pack the resolver picked and
/// why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Dep package name.
    pub pkg_name: String,
    /// Resolved version triple.
    pub version: SemVer,
    /// CAS store address of the chosen pack.
    pub impl_hash: ImplHash,
    /// Which of the three decision paths produced this resolution.
    pub origin: ResolutionOrigin,
}

/// Errors that can prevent resolution. Wraps [`StoreError`] for
/// underlying I/O and adds resolution-specific failure modes.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum ResolveError {
    /// No version of `pkg_name` in the store matches the dep's
    /// declared range — caller needs to install the package first
    /// (via `dao store import` or a build of the dep).
    #[error("no installed version of `{pkg_name}` satisfies the dep range")]
    #[diagnostic(
        code(triet::pack::E2380),
        help("install the dependency: `dao store import path/to/{pkg_name}.khi`")
    )]
    NoMatchingVersion {
        /// Package the caller asked for.
        pkg_name: String,
    },

    /// The lockfile pins a hash that isn't installed in the store.
    /// User should re-fetch or delete the lockfile to force resolution.
    #[error("lockfile pins `{pkg_name}` at a hash that isn't in the store")]
    #[diagnostic(
        code(triet::pack::E2381),
        help(
            "install the pinned pack, or delete `triet.lock` and re-run to resolve a fresh version"
        )
    )]
    LockfileHashMissing {
        /// Package name whose lockfile pin is dangling.
        pkg_name: String,
    },

    /// `dep.iface_hash_pin` is non-zero and the chosen pack's iface
    /// hash doesn't match — author asked for an exact ABI surface.
    #[error("dep `{pkg_name}` pins an iface_hash that no installed version matches")]
    #[diagnostic(
        code(triet::pack::E2382),
        help("either install a pack whose iface_hash matches the pin, or remove the pin")
    )]
    IfaceHashPinMismatch {
        /// Package name whose pin couldn't be satisfied.
        pkg_name: String,
    },

    /// Underlying store I/O error.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Store(#[from] StoreError),
}

/// Convenience alias for `Result<T, ResolveError>`.
pub type ResolveResult<T> = Result<T, ResolveError>;

/// Resolves declared deps to concrete pack hashes using a `Store` +
/// `Lockfile`. The lockfile is updated in place when new entries are
/// added; callers should save it via [`Lockfile::save`] after a
/// successful resolve.
pub struct Resolver<'a> {
    store: &'a Store,
    lockfile: Lockfile,
}

impl<'a> Resolver<'a> {
    /// Build a resolver with an empty lockfile (every dep gets freshly
    /// resolved).
    #[must_use]
    pub const fn new(store: &'a Store) -> Self {
        Self {
            store,
            lockfile: Lockfile::empty(),
        }
    }

    /// Build a resolver seeded with an existing lockfile.
    #[must_use]
    pub const fn with_lockfile(store: &'a Store, lockfile: Lockfile) -> Self {
        Self { store, lockfile }
    }

    /// Borrow the current lockfile state (possibly mutated by
    /// [`Resolver::resolve`] calls).
    #[must_use]
    pub const fn lockfile(&self) -> &Lockfile {
        &self.lockfile
    }

    /// Consume the resolver and return the (possibly updated)
    /// lockfile for persisting.
    #[must_use]
    pub fn into_lockfile(self) -> Lockfile {
        self.lockfile
    }

    /// Resolve every dep in `deps` against the store + lockfile.
    /// Returns one [`Resolution`] per dep in the same order.
    ///
    /// Side effect: any dep resolved fresh (not from lockfile) is
    /// upserted into [`Resolver::lockfile`].
    ///
    /// # Errors
    /// Returns [`ResolveError`] for the per-dep failures documented
    /// on each variant.
    pub fn resolve(&mut self, deps: &[Dep]) -> ResolveResult<Vec<Resolution>> {
        let mut out = Vec::with_capacity(deps.len());
        for dep in deps {
            out.push(self.resolve_one(dep)?);
        }
        Ok(out)
    }

    fn resolve_one(&mut self, dep: &Dep) -> ResolveResult<Resolution> {
        // If the dep declares an `iface_hash_pin`, that's a hard
        // constraint regardless of lockfile state — ADR-0013 §5.
        let has_pin = !dep.iface_hash_pin.is_zero();

        // First: try the lockfile (unless overridden by a non-zero
        // dep pin that demands a fresh check).
        if !has_pin && let Some(entry) = self.lockfile.find(&dep.pkg_name).cloned() {
            if !in_range(entry.version, dep) {
                // Lockfile out of sync with the dep range — fall
                // through to fresh resolution + overwrite.
            } else if self.store.resolve_pack(&entry.impl_hash)?.is_some() {
                return Ok(Resolution {
                    pkg_name: entry.pkg_name,
                    version: entry.version,
                    impl_hash: entry.impl_hash,
                    origin: ResolutionOrigin::Lockfile,
                });
            } else {
                return Err(ResolveError::LockfileHashMissing {
                    pkg_name: dep.pkg_name.clone(),
                });
            }
        }

        // Fresh resolution: enumerate installed versions, pick highest
        // in range. (Could be hooked up to plan_link for full ADR-0013
        // decision matrix later — for v0.5.5 the highest-in-range rule
        // matches Cargo behaviour and keeps the resolver predictable.)
        let candidates = self.store.list_versions(&dep.pkg_name)?;
        let chosen = candidates
            .into_iter()
            .filter(|(v, _)| in_range(*v, dep))
            .filter(|(_, h)| pin_matches(dep, h, self.store))
            .max_by(|(va, _), (vb, _)| {
                va.major
                    .cmp(&vb.major)
                    .then(va.minor.cmp(&vb.minor))
                    .then(va.patch.cmp(&vb.patch))
            });

        let Some((version, impl_hash)) = chosen else {
            if has_pin {
                return Err(ResolveError::IfaceHashPinMismatch {
                    pkg_name: dep.pkg_name.clone(),
                });
            }
            return Err(ResolveError::NoMatchingVersion {
                pkg_name: dep.pkg_name.clone(),
            });
        };

        // Look up the iface_hash from the installed manifest so the
        // lockfile records both sides of the hash pair.
        let iface_hash = self.read_iface_hash(&impl_hash)?;

        // Upsert into the lockfile so subsequent builds are pinned.
        self.lockfile.upsert(LockEntry {
            pkg_name: dep.pkg_name.clone(),
            version,
            iface_hash,
            impl_hash,
        });

        Ok(Resolution {
            pkg_name: dep.pkg_name.clone(),
            version,
            impl_hash,
            origin: if has_pin {
                ResolutionOrigin::IfacePin
            } else {
                ResolutionOrigin::Fresh
            },
        })
    }

    fn read_iface_hash(&self, impl_hash: &ImplHash) -> ResolveResult<crate::hash::IfaceHash> {
        // Cheap path: read manifest.bin and pull `iface_hash` out of
        // the parsed metadata. We don't load the full IR section.
        if let Some(manifest_bytes) = self.store.resolve_manifest_bytes(impl_hash)?
            && let Ok(meta) = parse_manifest_for_iface(&manifest_bytes)
        {
            return Ok(meta);
        }
        Err(ResolveError::Store(StoreError::Pack(
            crate::error::PackError::Corrupted(
                "could not read iface_hash from installed manifest".into(),
            ),
        )))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn in_range(v: SemVer, dep: &Dep) -> bool {
    if cmp_semver(&v, &dep.version_min) < 0 {
        return false;
    }
    // version_max_exclusive of (0,0,0) means open-ended.
    let max = dep.version_max_exclusive;
    if max == SemVer::default() {
        return true;
    }
    cmp_semver(&v, &max) < 0
}

fn cmp_semver(a: &SemVer, b: &SemVer) -> i32 {
    let cmp = a
        .major
        .cmp(&b.major)
        .then(a.minor.cmp(&b.minor))
        .then(a.patch.cmp(&b.patch));
    match cmp {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn pin_matches(dep: &Dep, candidate_impl: &ImplHash, store: &Store) -> bool {
    if dep.iface_hash_pin.is_zero() {
        return true;
    }
    // Read candidate's iface_hash from manifest and compare.
    let Ok(Some(bytes)) = store.resolve_manifest_bytes(candidate_impl) else {
        return false;
    };
    let Ok(iface) = parse_manifest_for_iface(&bytes) else {
        return false;
    };
    iface == dep.iface_hash_pin
}

/// Pull `iface_hash` out of an extracted manifest.bin. We reuse the
/// full metadata parser but throw away everything except the field
/// we need.
fn parse_manifest_for_iface(
    manifest_bytes: &[u8],
) -> Result<crate::hash::IfaceHash, crate::error::PackError> {
    // Wrap the manifest in a minimal pack envelope so we can reuse
    // `read_khi`. Same trick the store uses in `parse_manifest_only`.
    let mut wrap = Vec::with_capacity(manifest_bytes.len() + 20);
    wrap.extend_from_slice(&[0x74, 0x72, 0x69, 0x70]); // MAGIC
    wrap.extend_from_slice(&1u32.to_le_bytes()); // pack_version
    wrap.extend_from_slice(&1u32.to_le_bytes()); // section_count
    wrap.push(1); // ABI_METADATA section id
    wrap.extend_from_slice(&(manifest_bytes.len() as u32).to_le_bytes());
    wrap.extend_from_slice(manifest_bytes);
    let (meta, _code) = crate::serde::read_khi(&wrap)?;
    Ok(meta.iface_hash)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{IFACE_HASH_LEN, IfaceHash, TermIfaceHash, TermImplHash};
    use crate::serde::write_khi;
    use crate::types::{AbiMetadata, FunctionExport, Param, TypeRef, Visibility};
    use tempfile::TempDir;

    fn mk_pack(pkg: &str, version: SemVer, body_suffix: u8) -> Vec<u8> {
        let mut meta = AbiMetadata::empty(pkg, version);
        meta.exports.push(FunctionExport {
            name: "f".into(),
            module_path: String::new(),
            visibility: Visibility::Public,
            type_params: Vec::new(),
            params: vec![Param {
                name: "x".into(),
                type_ref: TypeRef::Primitive(0x02),
            }],
            return_type: TypeRef::Primitive(0x02),
            body_offset: 0,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
        // Distinct body bytes per version so each pack has its own
        // impl_hash even with identical ABI surface.
        write_khi(&meta, &[body_suffix])
    }

    fn dep_range(name: &str, min: SemVer, max: SemVer) -> Dep {
        Dep {
            pkg_name: name.into(),
            version_min: min,
            version_max_exclusive: max,
            iface_hash_pin: IfaceHash::default(),
        }
    }

    #[test]
    fn resolve_picks_highest_in_range_and_pins_lockfile() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .install_pack(&mk_pack("math", SemVer::new(1, 0, 0), 1))
            .unwrap();
        store
            .install_pack(&mk_pack("math", SemVer::new(1, 5, 2), 2))
            .unwrap();
        let expected_hash = store
            .install_pack(&mk_pack("math", SemVer::new(1, 7, 0), 3))
            .unwrap();

        let dep = dep_range("math", SemVer::new(1, 0, 0), SemVer::new(2, 0, 0));
        let mut resolver = Resolver::new(&store);
        let resolutions = resolver.resolve(&[dep]).unwrap();

        assert_eq!(resolutions.len(), 1);
        let r = &resolutions[0];
        assert_eq!(r.pkg_name, "math");
        assert_eq!(r.version, SemVer::new(1, 7, 0));
        assert_eq!(r.impl_hash, expected_hash);
        assert_eq!(r.origin, ResolutionOrigin::Fresh);

        // Lockfile now has the entry.
        let lf = resolver.lockfile();
        assert_eq!(lf.entries().len(), 1);
        assert_eq!(lf.find("math").unwrap().version, SemVer::new(1, 7, 0));
    }

    #[test]
    fn lockfile_pin_is_used_when_present() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let h_old = store
            .install_pack(&mk_pack("math", SemVer::new(1, 0, 0), 1))
            .unwrap();
        let _h_new = store
            .install_pack(&mk_pack("math", SemVer::new(1, 5, 0), 2))
            .unwrap();

        // Seed a lockfile pinning 1.0.0.
        let manifest = store.resolve_manifest_bytes(&h_old).unwrap().unwrap();
        let iface = parse_manifest_for_iface(&manifest).unwrap();
        let mut lf = Lockfile::empty();
        lf.upsert(LockEntry {
            pkg_name: "math".into(),
            version: SemVer::new(1, 0, 0),
            iface_hash: iface,
            impl_hash: h_old,
        });

        let dep = dep_range("math", SemVer::new(1, 0, 0), SemVer::new(2, 0, 0));
        let mut resolver = Resolver::with_lockfile(&store, lf);
        let res = resolver.resolve(&[dep]).unwrap();
        assert_eq!(res[0].version, SemVer::new(1, 0, 0));
        assert_eq!(res[0].impl_hash, h_old);
        assert_eq!(res[0].origin, ResolutionOrigin::Lockfile);
    }

    #[test]
    fn missing_version_in_store_errors() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let dep = dep_range("ghost", SemVer::new(1, 0, 0), SemVer::new(2, 0, 0));
        let mut resolver = Resolver::new(&store);
        let err = resolver.resolve(&[dep]).unwrap_err();
        assert!(matches!(err, ResolveError::NoMatchingVersion { .. }));
    }

    #[test]
    fn lockfile_hash_missing_in_store_errors() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        // Lockfile claims a hash that was never installed.
        let mut lf = Lockfile::empty();
        lf.upsert(LockEntry {
            pkg_name: "foo".into(),
            version: SemVer::new(1, 0, 0),
            iface_hash: IfaceHash::from_bytes([1; IFACE_HASH_LEN]),
            impl_hash: ImplHash::from_bytes([2; IFACE_HASH_LEN]),
        });

        let dep = dep_range("foo", SemVer::new(1, 0, 0), SemVer::new(2, 0, 0));
        let mut resolver = Resolver::with_lockfile(&store, lf);
        let err = resolver.resolve(&[dep]).unwrap_err();
        assert!(matches!(err, ResolveError::LockfileHashMissing { .. }));
    }

    #[test]
    fn dep_iface_hash_pin_overrides_lockfile() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let h_a = store
            .install_pack(&mk_pack("math", SemVer::new(1, 0, 0), 1))
            .unwrap();
        let h_b = store
            .install_pack(&mk_pack("math", SemVer::new(1, 2, 0), 2))
            .unwrap();
        let manifest_a = store.resolve_manifest_bytes(&h_a).unwrap().unwrap();
        let iface_a = parse_manifest_for_iface(&manifest_a).unwrap();
        let manifest_b = store.resolve_manifest_bytes(&h_b).unwrap().unwrap();
        let iface_b = parse_manifest_for_iface(&manifest_b).unwrap();
        // Two identical ABI surfaces (same exports) → iface_hash equal.
        assert_eq!(iface_a, iface_b);

        // Lockfile pins 1.0.0 / h_a.
        let mut lf = Lockfile::empty();
        lf.upsert(LockEntry {
            pkg_name: "math".into(),
            version: SemVer::new(1, 0, 0),
            iface_hash: iface_a,
            impl_hash: h_a,
        });

        // Dep declaration pins an iface_hash equal to iface_a/iface_b.
        // Pin overrides lockfile → resolver does fresh enumeration,
        // picks highest matching version (1.2.0).
        let dep = Dep {
            pkg_name: "math".into(),
            version_min: SemVer::new(1, 0, 0),
            version_max_exclusive: SemVer::new(2, 0, 0),
            iface_hash_pin: iface_a,
        };
        let mut resolver = Resolver::with_lockfile(&store, lf);
        let res = resolver.resolve(&[dep]).unwrap();
        assert_eq!(res[0].version, SemVer::new(1, 2, 0));
        // Pin path — not Fresh, not Lockfile.
        assert_eq!(res[0].origin, ResolutionOrigin::IfacePin);
    }

    #[test]
    fn iface_hash_pin_mismatch_returns_e2382() {
        // dep declares an iface_hash_pin that doesn't match any
        // installed version → ResolveError::IfaceHashPinMismatch
        // (E2382). The audit found the success path covered but no
        // explicit test for the mismatch error code.
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .install_pack(&mk_pack("math", SemVer::new(1, 0, 0), 1))
            .unwrap();
        store
            .install_pack(&mk_pack("math", SemVer::new(1, 2, 0), 2))
            .unwrap();

        // Pin a bogus iface_hash that matches neither installed pack.
        let dep = Dep {
            pkg_name: "math".into(),
            version_min: SemVer::new(1, 0, 0),
            version_max_exclusive: SemVer::new(2, 0, 0),
            iface_hash_pin: IfaceHash::from_bytes([0x77; IFACE_HASH_LEN]),
        };
        let mut resolver = Resolver::new(&store);
        let err = resolver.resolve(&[dep]).unwrap_err();
        match err {
            ResolveError::IfaceHashPinMismatch { pkg_name } => {
                assert_eq!(pkg_name, "math");
            }
            other => panic!("expected IfaceHashPinMismatch (E2382), got {other:?}"),
        }
    }

    #[test]
    fn lockfile_out_of_range_falls_back_to_fresh() {
        let tmp = TempDir::new().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let h_v2 = store
            .install_pack(&mk_pack("foo", SemVer::new(2, 0, 0), 1))
            .unwrap();
        let manifest = store.resolve_manifest_bytes(&h_v2).unwrap().unwrap();
        let iface_v2 = parse_manifest_for_iface(&manifest).unwrap();

        // Lockfile pins 1.0.0 but dep wants >=2.0.0. The pinned hashes
        // are bogus on purpose — resolver should not consult them.
        let mut lf = Lockfile::empty();
        lf.upsert(LockEntry {
            pkg_name: "foo".into(),
            version: SemVer::new(1, 0, 0),
            iface_hash: IfaceHash::from_bytes([7; IFACE_HASH_LEN]),
            impl_hash: ImplHash::from_bytes([7; IFACE_HASH_LEN]),
        });
        let dep = dep_range("foo", SemVer::new(2, 0, 0), SemVer::new(3, 0, 0));
        let mut resolver = Resolver::with_lockfile(&store, lf);
        let res = resolver.resolve(&[dep]).unwrap();
        assert_eq!(res[0].version, SemVer::new(2, 0, 0));
        assert_eq!(res[0].impl_hash, h_v2);
        assert_eq!(res[0].origin, ResolutionOrigin::Fresh);
        // Lockfile updated to the new resolution — including the real
        // iface hash read off the installed manifest.
        let updated = resolver.lockfile().find("foo").unwrap();
        assert_eq!(updated.version, SemVer::new(2, 0, 0));
        assert_eq!(updated.iface_hash, iface_v2);
    }
}
