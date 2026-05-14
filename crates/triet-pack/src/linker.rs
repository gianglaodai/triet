//! Cross-package linker — applies ADR-0013 semver decision matrix
//! and ADR-0011 dep resolution to a set of [`AbiMetadata`] inputs.
//!
//! The linker is intentionally *side-effect free*: callers provide
//! already-decoded `AbiMetadata` values, and the linker returns a
//! [`LinkPlan`] describing what would happen if these packs were
//! linked together. Driving I/O (reading `.tripack` files) lives in
//! the CLI layer.
//!
//! [ADR-0013]: ../../../docs/decisions/0013-semver-linking-policy.md
//! [ADR-0011]: ../../../docs/decisions/0011-abi-metadata-format.md

use std::collections::HashMap;

use crate::types::{AbiMetadata, Dep, SemVer};

/// Outcome of attempting to link a *root* package against a set of
/// candidate dependency packages.
///
/// Holds both hard errors (refuse-to-link) and soft warnings (hash
/// drift, version closer-than-expected) so the CLI can render the
/// full picture in one diagnostic pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPlan {
    /// Dependencies that resolved cleanly. Keyed by `pkg_name`.
    pub resolved: HashMap<String, ResolvedDep>,
    /// Soft diagnostics (warnings). Linker still produces a plan.
    pub warnings: Vec<LinkWarning>,
    /// Hard diagnostics. Empty == linker accepts; non-empty == refuse.
    pub errors: Vec<LinkError>,
}

impl LinkPlan {
    /// True when the linker can accept this configuration. Warnings
    /// don't block accept; errors do.
    #[must_use]
    pub const fn is_acceptable(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Successful resolution for one dependency declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDep {
    /// Name of the dependency package.
    pub pkg_name: String,
    /// Version actually selected from the candidate pool.
    pub selected_version: SemVer,
}

/// Soft diagnostics — codes E2310-E2319 per ADR-0013.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkWarning {
    /// E2310 — `iface_hash` differs from what was last seen, but no
    /// dep pin demanded an exact match. Rebuilding the consumer is
    /// recommended but not required.
    IfaceHashDrift {
        /// Dependency package name.
        pkg_name: String,
        /// Version that produced the drifted hash.
        version: SemVer,
    },
    /// E2311 — `iface_hash` differs and the dep declaration carried a
    /// non-zero `iface_hash_pin`. Treated as warning here so callers
    /// can surface a richer diagnostic; CLI/linker may promote this
    /// to an error when `--strict` is set.
    IfaceHashPinMismatch {
        /// Dependency package name.
        pkg_name: String,
        /// The hash the consumer was built against.
        expected_pin: crate::hash::IfaceHash,
        /// The hash actually present at link time.
        found: crate::hash::IfaceHash,
    },
}

/// Hard diagnostics — codes E2320-E2399 per ADR-0013.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// E2300 — dependency couldn't be located in the candidate set.
    PackageNotFound {
        /// Name the consumer asked for.
        pkg_name: String,
    },
    /// E2320 — major version bump. ABI contract may have changed; the
    /// linker refuses to guess how to adapt.
    MajorVersionMismatch {
        /// Dependency package name.
        pkg_name: String,
        /// Minimum version the consumer accepts.
        required_min: SemVer,
        /// Maximum (exclusive) version the consumer accepts.
        required_max_exclusive: SemVer,
        /// Version of the candidate package.
        found: SemVer,
    },
    /// E2321 — version below the consumer's declared minimum.
    VersionBelowMinimum {
        /// Dependency package name.
        pkg_name: String,
        /// Declared minimum.
        required_min: SemVer,
        /// Version of the candidate package.
        found: SemVer,
    },
}

/// Plan the link for `root` against `available` candidate packages.
///
/// `available` is a flat slice — callers wanting per-name resolution
/// can de-dup before calling. The linker selects the *highest version
/// in range* among matching candidates, per Cargo-style convention
/// (see ADR-0013 §2 decision matrix).
///
/// This function never reads files. Hash-pin mismatches that the
/// caller intends to treat as fatal can be promoted from warnings to
/// errors by inspecting [`LinkPlan::warnings`] before continuing.
#[must_use]
pub fn plan_link(root: &AbiMetadata, available: &[AbiMetadata]) -> LinkPlan {
    let mut plan = LinkPlan::default();

    for dep in &root.deps {
        match resolve_one(dep, available) {
            Resolution::Match {
                version,
                hash_warning,
            } => {
                if let Some(w) = hash_warning {
                    plan.warnings.push(w);
                }
                plan.resolved.insert(
                    dep.pkg_name.clone(),
                    ResolvedDep {
                        pkg_name: dep.pkg_name.clone(),
                        selected_version: version,
                    },
                );
            }
            Resolution::Error(err) => plan.errors.push(err),
        }
    }
    plan
}

/// Internal helper enum so `resolve_one` can return either success
/// (with an optional warning) or failure in one shape.
enum Resolution {
    Match {
        version: SemVer,
        hash_warning: Option<LinkWarning>,
    },
    Error(LinkError),
}

fn resolve_one(dep: &Dep, available: &[AbiMetadata]) -> Resolution {
    // First, find any candidate matching the name.
    let candidates: Vec<&AbiMetadata> = available
        .iter()
        .filter(|m| m.pkg_name == dep.pkg_name)
        .collect();
    if candidates.is_empty() {
        return Resolution::Error(LinkError::PackageNotFound {
            pkg_name: dep.pkg_name.clone(),
        });
    }

    // Classify each candidate. We need to distinguish "no candidate
    // in range" from "no candidate at all" to emit the right error.
    let mut highest_in_range: Option<&AbiMetadata> = None;
    let mut saw_major_bump = false;
    let mut saw_below_min = false;
    for cand in &candidates {
        match classify_version(&cand.pkg_version, dep) {
            VersionClass::InRange => {
                let take = highest_in_range.is_none_or(|cur| {
                    semver_cmp(&cand.pkg_version, &cur.pkg_version) == std::cmp::Ordering::Greater
                });
                if take {
                    highest_in_range = Some(*cand);
                }
            }
            VersionClass::MajorMismatch => saw_major_bump = true,
            VersionClass::BelowMin => saw_below_min = true,
        }
    }

    if let Some(chosen) = highest_in_range {
        let hash_warning = if !dep.iface_hash_pin.is_zero()
            && dep.iface_hash_pin != chosen.iface_hash
        {
            Some(LinkWarning::IfaceHashPinMismatch {
                pkg_name: dep.pkg_name.clone(),
                expected_pin: dep.iface_hash_pin,
                found: chosen.iface_hash,
            })
        } else {
            // Even without a pin, if the candidate's version is well
            // ahead of the dep's declared minimum, warn about
            // potential drift — encourages rebuild without forcing it.
            // (ADR-0013 §2 — E2310 advisory warning.)
            let drift = semver_cmp(&chosen.pkg_version, &dep.version_min)
                == std::cmp::Ordering::Greater
                && chosen.pkg_version.minor > dep.version_min.minor;
            if drift {
                Some(LinkWarning::IfaceHashDrift {
                    pkg_name: dep.pkg_name.clone(),
                    version: chosen.pkg_version,
                })
            } else {
                None
            }
        };
        return Resolution::Match {
            version: chosen.pkg_version,
            hash_warning,
        };
    }

    // Pick the most informative error. A major-version bump shadows
    // a "below min" because it's almost always the more useful
    // diagnostic for the user.
    if saw_major_bump {
        let candidate = candidates
            .iter()
            .find(|c| matches!(classify_version(&c.pkg_version, dep), VersionClass::MajorMismatch))
            .copied()
            .expect("saw_major_bump implies at least one candidate matched");
        return Resolution::Error(LinkError::MajorVersionMismatch {
            pkg_name: dep.pkg_name.clone(),
            required_min: dep.version_min,
            required_max_exclusive: dep.version_max_exclusive,
            found: candidate.pkg_version,
        });
    }
    if saw_below_min {
        let candidate = candidates
            .iter()
            .find(|c| matches!(classify_version(&c.pkg_version, dep), VersionClass::BelowMin))
            .copied()
            .expect("saw_below_min implies at least one candidate matched");
        return Resolution::Error(LinkError::VersionBelowMinimum {
            pkg_name: dep.pkg_name.clone(),
            required_min: dep.version_min,
            found: candidate.pkg_version,
        });
    }

    // No reachable code path; defensive fallback.
    Resolution::Error(LinkError::PackageNotFound {
        pkg_name: dep.pkg_name.clone(),
    })
}

/// One candidate's relationship to a dep declaration.
enum VersionClass {
    /// Within `[version_min, version_max_exclusive)`.
    InRange,
    /// At or beyond `version_max_exclusive` (effectively a major bump
    /// given that the consumer's declared range stopped just before).
    MajorMismatch,
    /// Strictly below `version_min`.
    BelowMin,
}

fn classify_version(v: &SemVer, dep: &Dep) -> VersionClass {
    if semver_cmp(v, &dep.version_min) == std::cmp::Ordering::Less {
        return VersionClass::BelowMin;
    }
    // `version_max_exclusive` of (0,0,0) means open-ended.
    let max = dep.version_max_exclusive;
    if max.major == 0 && max.minor == 0 && max.patch == 0 {
        return VersionClass::InRange;
    }
    if semver_cmp(v, &max) == std::cmp::Ordering::Less {
        VersionClass::InRange
    } else {
        VersionClass::MajorMismatch
    }
}

fn semver_cmp(a: &SemVer, b: &SemVer) -> std::cmp::Ordering {
    (a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::IfaceHash;
    use crate::types::AbiMetadata;

    fn mk_pkg(name: &str, v: SemVer) -> AbiMetadata {
        AbiMetadata::empty(name, v)
    }

    fn mk_dep(name: &str, min: SemVer, max: SemVer) -> Dep {
        Dep {
            pkg_name: name.into(),
            version_min: min,
            version_max_exclusive: max,
            iface_hash_pin: IfaceHash::default(),
        }
    }

    #[test]
    fn link_accepts_in_range_version() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        ));
        let math = mk_pkg("math", SemVer::new(1, 2, 0));
        let plan = plan_link(&root, &[math]);
        assert!(plan.is_acceptable());
        assert_eq!(plan.errors, vec![]);
        assert_eq!(plan.resolved["math"].selected_version, SemVer::new(1, 2, 0));
    }

    #[test]
    fn link_refuses_major_bump() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        ));
        let math = mk_pkg("math", SemVer::new(2, 0, 0));
        let plan = plan_link(&root, &[math]);
        assert!(!plan.is_acceptable());
        assert!(matches!(
            plan.errors[0],
            LinkError::MajorVersionMismatch { .. }
        ));
    }

    #[test]
    fn link_refuses_below_minimum() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 2, 0),
            SemVer::new(2, 0, 0),
        ));
        let math = mk_pkg("math", SemVer::new(1, 1, 9));
        let plan = plan_link(&root, &[math]);
        assert!(!plan.is_acceptable());
        assert!(matches!(plan.errors[0], LinkError::VersionBelowMinimum { .. }));
    }

    #[test]
    fn link_reports_missing_package() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "missing",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        ));
        let plan = plan_link(&root, &[]);
        assert!(!plan.is_acceptable());
        assert!(matches!(plan.errors[0], LinkError::PackageNotFound { .. }));
    }

    #[test]
    fn link_picks_highest_in_range() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        ));
        let m1 = mk_pkg("math", SemVer::new(1, 0, 0));
        let m2 = mk_pkg("math", SemVer::new(1, 5, 0));
        let m3 = mk_pkg("math", SemVer::new(1, 2, 3));
        let plan = plan_link(&root, &[m1, m2, m3]);
        assert_eq!(plan.resolved["math"].selected_version, SemVer::new(1, 5, 0));
    }

    #[test]
    fn link_pin_mismatch_emits_warning_not_error() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        let mut dep = mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        );
        dep.iface_hash_pin = IfaceHash::from_bytes([0x42; 32]);
        root.deps.push(dep);
        let math = mk_pkg("math", SemVer::new(1, 0, 0)); // hash differs from pin
        let plan = plan_link(&root, &[math]);
        // Still acceptable — warning, not error.
        assert!(plan.is_acceptable());
        assert!(matches!(
            plan.warnings[0],
            LinkWarning::IfaceHashPinMismatch { .. }
        ));
    }

    #[test]
    fn link_open_ended_max_accepts_any_above_min() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(0, 0, 0), // open-ended
        ));
        let math = mk_pkg("math", SemVer::new(9, 9, 9));
        let plan = plan_link(&root, &[math]);
        assert!(plan.is_acceptable());
        assert_eq!(plan.resolved["math"].selected_version, SemVer::new(9, 9, 9));
    }

    #[test]
    fn link_minor_drift_emits_warning() {
        let mut root = mk_pkg("app", SemVer::new(1, 0, 0));
        root.deps.push(mk_dep(
            "math",
            SemVer::new(1, 0, 0),
            SemVer::new(2, 0, 0),
        ));
        let math = mk_pkg("math", SemVer::new(1, 5, 0));
        let plan = plan_link(&root, &[math]);
        assert!(plan.is_acceptable());
        assert!(
            plan.warnings
                .iter()
                .any(|w| matches!(w, LinkWarning::IfaceHashDrift { .. }))
        );
    }
}
