//! Link-time capability check (ADR-0018 §2 Step 6a, ADR-0016 §5 +
//! §7).
//!
//! Sibling to [`linker::plan_link`] (ADR-0013 semver). Where the
//! semver linker decides whether dep versions are compatible, the
//! capability linker decides whether the *cap closure* of root plus
//! its dependencies satisfies the root package's authority. Both
//! checks live at the same loader stage and are side-effect free —
//! callers feed already-decoded [`AbiMetadata`] and get back a
//! [`CapabilityLinkReport`].
//!
//! Three compile-time-not-yet errors from the [E22XX namespace]
//! (ADR-0016 §6) fire here:
//!
//! - **E2200 `MissingCapabilityClaim`** — a dep requests a cap path
//!   the root manifest doesn't authorize. Root authority requires an
//!   explicit `requires` entry; ADR-0016 §7 forbids auto-promotion.
//! - **E2202 `UnresolvedCapabilityPath`** — a claim's `cap_path`
//!   doesn't match any module in the package tree. The path is
//!   defer-detected here because compile-time (ADR-0016 §5 rule 4)
//!   can't see deps' export tables.
//! - **E2203 `CapabilityRefused`** — root manifest declares Deny or
//!   Ambient for a path some pack requests. Per ADR-0016 §3, Ambient
//!   at the root has no caller above to inherit from, so it
//!   collapses to effective Deny.
//!
//! `Defer` doesn't error here — paths the root marks
//! `Trilean::Unknown` are collected into [`CapabilityLinkReport::deferrals`]
//! for the runtime resolver (ADR-0017 §4, machinery in v0.6.9+) to
//! decide at load time.
//!
//! **Span / source location:** none. [`AbiMetadata`] is a binary wire
//! format with no source tracking — link-time diagnostics are
//! package-level. The diagnostic surfaces `requester_pkgs` so the
//! user can find which `.khi` is asking; ADR-0018 §5 spans on
//! `.khi` byte offsets land when the loader actually parses
//! per-section bytes (v0.6.x.cleanup or later).
//!
//! **What this check does NOT do:**
//!
//! - E2208.PreV06Reader — gated by a future `abi_version` bump
//!   (currently `v=2` understands caps natively).
//! - E2205 sub-variants — runtime resolver (v0.6.9+).
//!
//! [`linker::plan_link`]: crate::plan_link
//! [E22XX namespace]: ../../../docs/decisions/0016-capability-type-system.md

use std::collections::{BTreeMap, BTreeSet, HashSet};

use miette::Diagnostic;
use thiserror::Error;

use crate::types::{AbiMetadata, CapabilityLevel};

/// Outcome of running the link-time capability check on a root
/// package + its dependency closure.
///
/// Mirrors the semver-side [`LinkPlan`](crate::LinkPlan) shape:
/// errors block accept; deferrals carry forward to the runtime
/// resolver but don't fail the link.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityLinkReport {
    /// Hard refusals. Empty = link accepts the cap closure.
    pub errors: Vec<CapabilityLinkError>,
    /// Cap paths the root marked `Defer` (`Trilean::Unknown`). The
    /// runtime resolver (ADR-0017) decides at load time.
    pub deferrals: Vec<DeferredCap>,
}

impl CapabilityLinkReport {
    /// True when the cap closure passes link-time enforcement.
    /// Deferrals don't block — they hand the decision to the
    /// runtime resolver.
    #[must_use]
    pub const fn is_acceptable(&self) -> bool {
        self.errors.is_empty()
    }
}

/// One `Trilean::Unknown` cap path collected for runtime resolution.
/// Surfaced so callers can feed it to the policy hook (v0.6.9+).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeferredCap {
    /// The deferred cap path (e.g. `"sys.net.dns"`).
    pub cap_path: String,
    /// Packages whose claim triggered the deferral, sorted.
    pub requester_pkgs: Vec<String>,
}

/// How the root package declared a cap that ends up refused. ADR-0016
/// §3 distinguishes the two — both surface as E2203 but the
/// diagnostic carries the original level so users see why it
/// refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootRefusalLevel {
    /// Root claimed `deny`. Explicit refusal.
    Deny,
    /// Root claimed `ambient`. At root this collapses to deny per
    /// ADR-0016 §3 (no caller above to inherit from).
    Ambient,
}

impl RootRefusalLevel {
    /// Display token for diagnostics.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ambient => "ambient (= deny at root)",
        }
    }
}

/// Hard diagnostics raised by [`check_link_capabilities`]. All three
/// codes are from the [E22XX namespace] (ADR-0016 §6).
///
/// [E22XX namespace]: ../../../docs/decisions/0016-capability-type-system.md
#[derive(Clone, Debug, Diagnostic, Error, PartialEq, Eq)]
pub enum CapabilityLinkError {
    /// E2200 — a dep requests a cap path the root doesn't authorize.
    /// Root manifest must explicit-list per ADR-0016 §7 (no
    /// auto-promotion through transitive deps).
    #[error(
        "package(s) [{}] request capability `{cap_path}` but root package `{root_pkg}` has no matching `requires` entry",
        requester_pkgs.join(", "),
    )]
    #[diagnostic(
        code(triet::capability::E2200),
        help(
            "add `requires <path> grant` (or `defer`) to the root's dao.package. Root is \
             the sole authority on cap decisions — transitive grants are not auto-promoted \
             (ADR-0016 §7)."
        )
    )]
    MissingCapabilityClaim {
        /// Cap path that lacks an authorizing entry.
        cap_path: String,
        /// Packages whose claim hit this missing entry, sorted.
        requester_pkgs: Vec<String>,
        /// Name of the root package whose manifest is the authority.
        root_pkg: String,
    },

    /// E2202 — claim's `cap_path` doesn't match any module across
    /// root + deps. The path may be a typo or refer to a module
    /// that's no longer published.
    #[error(
        "capability `{cap_path}` requested by [{}] does not match any module in the package tree",
        requester_pkgs.join(", "),
    )]
    #[diagnostic(
        code(triet::capability::E2202),
        help(
            "ensure the cap path exactly matches a published module path. ADR-0016 §2: \
             matching is exact, no path inheritance."
        )
    )]
    UnresolvedCapabilityPath {
        /// Cap path that didn't resolve.
        cap_path: String,
        /// Packages whose claim referenced this path, sorted.
        requester_pkgs: Vec<String>,
    },

    /// E2203 — root manifest refuses (Deny, or Ambient-which-collapses-to-Deny)
    /// a path some pack requests. ADR-0016 §3 + §7.
    #[error(
        "root package `{root_pkg}` declares `{}` for `{cap_path}`, but package(s) [{}] request it",
        root_level.as_str(),
        requester_pkgs.join(", "),
    )]
    #[diagnostic(
        code(triet::capability::E2203),
        help(
            "change root's claim to `grant` or `defer`, or remove the request from the \
             dependent package(s). Ambient at root collapses to deny (ADR-0016 §3) — \
             root has no caller to inherit from."
        )
    )]
    CapabilityRefused {
        /// Cap path the root refused.
        cap_path: String,
        /// Root's declared level — Deny or Ambient.
        root_level: RootRefusalLevel,
        /// Packages whose claim hit the refusal, sorted.
        requester_pkgs: Vec<String>,
        /// Name of the root package whose manifest refused.
        root_pkg: String,
    },

    /// E2208 — mismatch between the source `dao.package` `requires`
    /// and the `.khi` binary's caps section. The manifest lists
    /// claims that were never encoded into the binary, or the
    /// binary carries claims absent from the manifest. Either
    /// direction indicates writer corruption / tampering.
    #[error(
        "capability claims in `.khi` diverge from `dao.package` for root `{root_pkg}`: \
         {} missing from binary, {} extra in binary",
        missing_from_binary.join(", "),
        extra_in_binary.join(", "),
    )]
    #[diagnostic(
        code(triet::capability::E2208),
        help(
            "rebuild the `.khi` from the canonical `dao.package`. If the divergence \
             persists, the writer (dao build / compiler) may have a bug in caps-section \
             emission."
        )
    )]
    CapabilityDivergence {
        /// Caps present in manifest but not in binary.
        missing_from_binary: Vec<String>,
        /// Caps present in binary but not in manifest.
        extra_in_binary: Vec<String>,
        /// Root package name for diagnostics.
        root_pkg: String,
    },
}

/// Apply ADR-0018 §2 Step 6a to `root` plus its dep closure.
///
/// Algorithm (ADR-0016 §5 + §7):
///
/// 1. Collect every cap path requested by any pack in the tree,
///    along with all requesters.
/// 2. For each cap path, check it resolves to a module exported by
///    *some* pack — else E2202.
/// 3. Look up root's authority over that path:
///    - `Grant` → accept silently.
///    - `Defer` → push to [`deferrals`](CapabilityLinkReport::deferrals)
///      for the runtime resolver.
///    - `Deny` or `Ambient` → E2203.
///    - root has no entry for the path → E2200.
///
/// Iteration order is deterministic (cap paths sorted
/// alphabetically) so two runs on the same inputs produce identical
/// reports — important for CI consumption and snapshot tests.
#[must_use]
pub fn check_link_capabilities(
    root: &AbiMetadata,
    available: &[AbiMetadata],
) -> CapabilityLinkReport {
    // Module path universe — every path that could legally back a cap.
    let mut module_paths: HashSet<&str> = HashSet::new();
    for m in &root.modules {
        module_paths.insert(m.path.as_str());
    }
    for pack in available {
        for m in &pack.modules {
            module_paths.insert(m.path.as_str());
        }
    }

    // (cap_path → sorted set of requester package names). BTreeMap
    // gives deterministic iteration so error order is stable.
    let mut requested: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for claim in &root.caps {
        requested
            .entry(claim.cap_path.clone())
            .or_default()
            .insert(root.pkg_name.clone());
    }
    for pack in available {
        for claim in &pack.caps {
            requested
                .entry(claim.cap_path.clone())
                .or_default()
                .insert(pack.pkg_name.clone());
        }
    }

    let mut report = CapabilityLinkReport::default();

    for (cap_path, requesters) in requested {
        let requester_pkgs: Vec<String> = requesters.into_iter().collect();

        // Step 1: structural — path must back to a real module.
        if !module_paths.contains(cap_path.as_str()) {
            report
                .errors
                .push(CapabilityLinkError::UnresolvedCapabilityPath {
                    cap_path,
                    requester_pkgs,
                });
            // Skip authority check — emitting both errors for the
            // same path would clutter diagnostics; structural
            // problem dominates.
            continue;
        }

        // Step 2: root authority. ADR-0016 §7 — root is sole
        // decision-maker; transitive grants don't auto-promote.
        let root_claim = root.caps.iter().find(|c| c.cap_path == cap_path);

        match root_claim {
            None => report
                .errors
                .push(CapabilityLinkError::MissingCapabilityClaim {
                    cap_path,
                    requester_pkgs,
                    root_pkg: root.pkg_name.clone(),
                }),
            Some(claim) => match claim.level {
                CapabilityLevel::Grant => {} // accept silently
                CapabilityLevel::Defer => report.deferrals.push(DeferredCap {
                    cap_path,
                    requester_pkgs,
                }),
                CapabilityLevel::Deny => {
                    report.errors.push(CapabilityLinkError::CapabilityRefused {
                        cap_path,
                        root_level: RootRefusalLevel::Deny,
                        requester_pkgs,
                        root_pkg: root.pkg_name.clone(),
                    });
                }
                CapabilityLevel::Ambient => {
                    report.errors.push(CapabilityLinkError::CapabilityRefused {
                        cap_path,
                        root_level: RootRefusalLevel::Ambient,
                        requester_pkgs,
                        root_pkg: root.pkg_name.clone(),
                    });
                }
            },
        }
    }

    report
}

/// v0.7.11.6 — compare the source `dao.package` manifest's
/// `requires` claims against the `.khi` binary's `caps` section.
/// Returns `E2208.CapabilityDivergence` when claims differ between
/// source and binary in either direction.
///
/// Canonical comparison uses `(cap_path, level)` as the identity key
/// so reordered or duplicated entries don't flag. Claims in the
/// manifest that are absent from the binary are "missing"; claims in
/// the binary absent from the manifest are "extra". Both are sorted
/// alphabetically for deterministic diagnostics.
#[must_use]
pub fn check_cap_divergence(
    manifest_requires: &[crate::CapabilityClaim],
    binary_caps: &[crate::CapabilityClaim],
    root_pkg: &str,
) -> Option<CapabilityLinkError> {
    // Build (cap_path, level) → count from each side.
    let manifest_set: HashSet<(String, u8)> = manifest_requires
        .iter()
        .map(|c| (c.cap_path.clone(), c.level as u8))
        .collect();
    let binary_set: HashSet<(String, u8)> = binary_caps
        .iter()
        .map(|c| (c.cap_path.clone(), c.level as u8))
        .collect();

    let mut missing_from_binary: Vec<String> = Vec::new();
    let mut extra_in_binary: Vec<String> = Vec::new();

    for (path, level) in &manifest_set {
        if !binary_set.contains(&(path.clone(), *level)) {
            missing_from_binary.push(path.clone());
        }
    }
    for (path, level) in &binary_set {
        if !manifest_set.contains(&(path.clone(), *level)) {
            extra_in_binary.push(path.clone());
        }
    }

    if missing_from_binary.is_empty() && extra_in_binary.is_empty() {
        return None;
    }

    missing_from_binary.sort();
    extra_in_binary.sort();

    Some(CapabilityLinkError::CapabilityDivergence {
        missing_from_binary,
        extra_in_binary,
        root_pkg: root_pkg.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{ModuleIfaceHash, ModuleImplHash};
    use crate::types::{CapabilityClaim, Module, SemVer};

    // ── Fixtures ───────────────────────────────────────────────────

    fn cap(path: &str, level: CapabilityLevel) -> CapabilityClaim {
        CapabilityClaim {
            cap_path: path.into(),
            level,
        }
    }

    fn module(path: &str) -> Module {
        Module {
            path: path.into(),
            iface_hash_mod: ModuleIfaceHash::default(),
            impl_hash_mod: ModuleImplHash::default(),
        }
    }

    fn pkg(name: &str, modules: Vec<Module>, caps: Vec<CapabilityClaim>) -> AbiMetadata {
        let mut m = AbiMetadata::empty(name, SemVer::new(0, 1, 0));
        m.modules = modules;
        m.caps = caps;
        m
    }

    // ── Happy paths ────────────────────────────────────────────────

    #[test]
    fn empty_tree_accepts() {
        let root = pkg("root", vec![], vec![]);
        let report = check_link_capabilities(&root, &[]);
        assert!(report.is_acceptable());
        assert!(report.errors.is_empty());
        assert!(report.deferrals.is_empty());
    }

    #[test]
    fn root_self_grant_with_module_passes() {
        // Root claims sys.io grant, and sys.io is a real module on root.
        let root = pkg(
            "root",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[]);
        assert!(report.is_acceptable(), "errors: {:?}", report.errors);
    }

    #[test]
    fn root_grants_dep_request_passes() {
        // Root grants sys.io; stdlib dep both exposes the module and claims it.
        let root = pkg("root", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);
        let stdlib = pkg(
            "stdlib",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[stdlib]);
        assert!(report.is_acceptable(), "errors: {:?}", report.errors);
    }

    #[test]
    fn orphan_root_grant_without_requesters_passes() {
        // Root grants sys.io; no one (including root) actually requests it
        // *as a dep claim*. But root's own .caps entry IS a request,
        // so this still hits the loop. Path validity must hold.
        let root = pkg(
            "root",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[]);
        assert!(report.is_acceptable());
    }

    // ── E2200 — Missing claim at root ─────────────────────────────

    #[test]
    fn dep_request_without_root_claim_fires_e2200() {
        let root = pkg("root", vec![], vec![]);
        let stdlib = pkg(
            "stdlib",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[stdlib]);
        assert_eq!(report.errors.len(), 1);
        match &report.errors[0] {
            CapabilityLinkError::MissingCapabilityClaim {
                cap_path,
                requester_pkgs,
                root_pkg,
            } => {
                assert_eq!(cap_path, "sys.io");
                assert_eq!(requester_pkgs, &vec!["stdlib".to_owned()]);
                assert_eq!(root_pkg, "root");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn multiple_dep_requesters_aggregated() {
        let root = pkg("root", vec![], vec![]);
        let dep_a = pkg(
            "alpha",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let dep_b = pkg("beta", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);
        let report = check_link_capabilities(&root, &[dep_a, dep_b]);
        assert_eq!(report.errors.len(), 1);
        if let CapabilityLinkError::MissingCapabilityClaim { requester_pkgs, .. } =
            &report.errors[0]
        {
            // BTreeSet → sorted iteration.
            assert_eq!(requester_pkgs, &vec!["alpha".to_owned(), "beta".to_owned()],);
        } else {
            panic!("expected E2200");
        }
    }

    #[test]
    fn requesters_sorted_when_inserted_out_of_order() {
        // v0.6.x.review.1: `multiple_dep_requesters_aggregated`
        // inserts alpha+beta — already alphabetical, so the
        // BTreeSet sort is invisible. This test inserts in
        // zeta/alpha/beta order so the sort is forced to actually
        // reorder, proving deterministic E2200 requester output
        // independent of dep iteration order.
        let root = pkg("root", vec![], vec![]);
        let dep_z = pkg(
            "zeta",
            vec![module("sys.io")], // one pack must publish the module
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let dep_a = pkg("alpha", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);
        let dep_b = pkg("beta", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);

        let report = check_link_capabilities(&root, &[dep_z, dep_a, dep_b]);
        assert_eq!(report.errors.len(), 1);
        if let CapabilityLinkError::MissingCapabilityClaim { requester_pkgs, .. } =
            &report.errors[0]
        {
            assert_eq!(
                requester_pkgs,
                &vec!["alpha".to_owned(), "beta".to_owned(), "zeta".to_owned()],
                "requesters must be alphabetical despite zeta/alpha/beta insertion order",
            );
        } else {
            panic!("expected E2200 MissingCapabilityClaim");
        }
    }

    // ── E2202 — Unresolved path ───────────────────────────────────

    #[test]
    fn unresolved_path_fires_e2202() {
        // Root claims sys.io but no pack publishes a module by that path.
        let root = pkg("root", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);
        let report = check_link_capabilities(&root, &[]);
        assert_eq!(report.errors.len(), 1);
        match &report.errors[0] {
            CapabilityLinkError::UnresolvedCapabilityPath {
                cap_path,
                requester_pkgs,
            } => {
                assert_eq!(cap_path, "sys.io");
                assert_eq!(requester_pkgs, &vec!["root".to_owned()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn e2202_skips_authority_check() {
        // Path doesn't resolve AND root would deny. Only E2202 emitted —
        // structural problem dominates; emitting both for the same path
        // is noisy.
        let root = pkg(
            "root",
            vec![],
            vec![cap("sys.notexist", CapabilityLevel::Deny)],
        );
        let report = check_link_capabilities(&root, &[]);
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(
            &report.errors[0],
            CapabilityLinkError::UnresolvedCapabilityPath { .. }
        ));
    }

    // ── E2203 — Root refusal ──────────────────────────────────────

    #[test]
    fn root_deny_with_dep_request_fires_e2203_deny() {
        let root = pkg("root", vec![], vec![cap("dev.disk", CapabilityLevel::Deny)]);
        let dep = pkg(
            "diskutil",
            vec![module("dev.disk")],
            vec![cap("dev.disk", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[dep]);
        assert_eq!(report.errors.len(), 1);
        match &report.errors[0] {
            CapabilityLinkError::CapabilityRefused {
                cap_path,
                root_level,
                requester_pkgs,
                root_pkg,
            } => {
                assert_eq!(cap_path, "dev.disk");
                assert_eq!(*root_level, RootRefusalLevel::Deny);
                // root and diskutil both requested it (root via Deny claim).
                assert_eq!(
                    requester_pkgs,
                    &vec!["diskutil".to_owned(), "root".to_owned()],
                );
                assert_eq!(root_pkg, "root");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn root_ambient_collapses_to_refusal() {
        // Ambient at root has no caller above to inherit from →
        // E2203 with Ambient level for diagnostic clarity.
        let root = pkg(
            "root",
            vec![],
            vec![cap("sys.io", CapabilityLevel::Ambient)],
        );
        let dep = pkg(
            "stdlib",
            vec![module("sys.io")],
            vec![cap("sys.io", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[dep]);
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(
            &report.errors[0],
            CapabilityLinkError::CapabilityRefused {
                root_level: RootRefusalLevel::Ambient,
                ..
            }
        ));
    }

    // ── Defer ─────────────────────────────────────────────────────

    #[test]
    fn root_defer_collects_to_deferrals() {
        let root = pkg(
            "root",
            vec![],
            vec![cap("sys.net.dns", CapabilityLevel::Defer)],
        );
        let dep = pkg(
            "netlib",
            vec![module("sys.net.dns")],
            vec![cap("sys.net.dns", CapabilityLevel::Grant)],
        );
        let report = check_link_capabilities(&root, &[dep]);
        assert!(report.is_acceptable(), "errors: {:?}", report.errors);
        assert_eq!(report.deferrals.len(), 1);
        assert_eq!(report.deferrals[0].cap_path, "sys.net.dns");
        assert_eq!(
            report.deferrals[0].requester_pkgs,
            vec!["netlib".to_owned(), "root".to_owned()],
        );
    }

    #[test]
    fn is_acceptable_only_reflects_errors_not_deferrals() {
        let root = pkg(
            "root",
            vec![module("sys.x")],
            vec![cap("sys.x", CapabilityLevel::Defer)],
        );
        let report = check_link_capabilities(&root, &[]);
        assert!(report.deferrals.len() == 1);
        assert!(report.is_acceptable());
    }

    // ── Mixed / ordering ──────────────────────────────────────────

    #[test]
    fn deterministic_error_order() {
        // Distinct cap paths must come out alphabetically sorted.
        let root = pkg(
            "root",
            vec![],
            vec![
                cap("zzz.last", CapabilityLevel::Grant),
                cap("aaa.first", CapabilityLevel::Grant),
                cap("mmm.middle", CapabilityLevel::Grant),
            ],
        );
        // No modules → all three unresolved.
        let report = check_link_capabilities(&root, &[]);
        assert_eq!(report.errors.len(), 3);
        let order: Vec<&str> = report
            .errors
            .iter()
            .filter_map(|e| match e {
                CapabilityLinkError::UnresolvedCapabilityPath { cap_path, .. } => {
                    Some(cap_path.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(order, vec!["aaa.first", "mmm.middle", "zzz.last"]);
    }

    #[test]
    fn mixed_grant_refuse_defer_aggregated() {
        let root = pkg(
            "root",
            vec![module("sys.io"), module("dev.disk"), module("sys.net.dns")],
            vec![
                cap("sys.io", CapabilityLevel::Grant),
                cap("dev.disk", CapabilityLevel::Deny),
                cap("sys.net.dns", CapabilityLevel::Defer),
            ],
        );
        let dep = pkg(
            "libs",
            vec![],
            vec![
                cap("sys.io", CapabilityLevel::Grant),
                cap("dev.disk", CapabilityLevel::Grant),
                cap("sys.net.dns", CapabilityLevel::Grant),
            ],
        );
        let report = check_link_capabilities(&root, &[dep]);
        // One refusal for dev.disk; one deferral for sys.net.dns; sys.io passes.
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.deferrals.len(), 1);
        assert!(matches!(
            &report.errors[0],
            CapabilityLinkError::CapabilityRefused {
                cap_path,
                root_level: RootRefusalLevel::Deny,
                ..
            } if cap_path == "dev.disk"
        ));
        assert_eq!(report.deferrals[0].cap_path, "sys.net.dns");
    }

    #[test]
    fn module_from_dep_resolves_for_root_claim() {
        // Root claims sys.io but root has no module by that name —
        // the stdlib dep publishes it. Path must resolve via dep.
        let root = pkg("root", vec![], vec![cap("sys.io", CapabilityLevel::Grant)]);
        let stdlib = pkg("stdlib", vec![module("sys.io")], vec![]);
        let report = check_link_capabilities(&root, &[stdlib]);
        assert!(report.is_acceptable(), "errors: {:?}", report.errors);
    }

    // ── v0.7.11.6 — E2208 CapabilityDivergence ────────────────────

    #[test]
    fn manifest_and_binary_agree_returns_none() {
        let manifest_caps = vec![
            cap("sys.io", CapabilityLevel::Grant),
            cap("dev.disk", CapabilityLevel::Defer),
        ];
        let binary_caps = vec![
            cap("sys.io", CapabilityLevel::Grant),
            cap("dev.disk", CapabilityLevel::Defer),
        ];
        let result = check_cap_divergence(&manifest_caps, &binary_caps, "root");
        assert!(result.is_none(), "identical sets should not diverge");
    }

    #[test]
    fn manifest_has_claim_binary_missing_fires_e2208() {
        let manifest_caps = vec![
            cap("sys.io", CapabilityLevel::Grant),
            cap("dev.disk", CapabilityLevel::Deny),
        ];
        let binary_caps = vec![cap("sys.io", CapabilityLevel::Grant)];
        let result = check_cap_divergence(&manifest_caps, &binary_caps, "root");
        assert!(result.is_some());
        if let Some(CapabilityLinkError::CapabilityDivergence {
            missing_from_binary,
            extra_in_binary,
            root_pkg,
        }) = result
        {
            assert_eq!(missing_from_binary, vec!["dev.disk"]);
            assert!(extra_in_binary.is_empty());
            assert_eq!(root_pkg, "root");
        } else {
            panic!("expected E2208");
        }
    }

    #[test]
    fn binary_has_extra_claim_fires_e2208() {
        let manifest_caps = vec![cap("sys.io", CapabilityLevel::Grant)];
        let binary_caps = vec![
            cap("sys.io", CapabilityLevel::Grant),
            cap("sys.secret", CapabilityLevel::Grant),
        ];
        let result = check_cap_divergence(&manifest_caps, &binary_caps, "root");
        assert!(result.is_some());
        if let Some(CapabilityLinkError::CapabilityDivergence {
            missing_from_binary,
            extra_in_binary,
            ..
        }) = result
        {
            assert!(missing_from_binary.is_empty());
            assert_eq!(extra_in_binary, vec!["sys.secret"]);
        } else {
            panic!("expected E2208");
        }
    }

    #[test]
    fn level_mismatch_fires_e2208() {
        // Same cap_path, different level → divergence.
        let manifest_caps = vec![cap("sys.io", CapabilityLevel::Grant)];
        let binary_caps = vec![cap("sys.io", CapabilityLevel::Deny)];
        let result = check_cap_divergence(&manifest_caps, &binary_caps, "root");
        assert!(result.is_some(), "level mismatch must flag");
    }

    #[test]
    fn both_missing_and_extra_reported() {
        let manifest_caps = vec![
            cap("a.first", CapabilityLevel::Grant),
            cap("b.second", CapabilityLevel::Defer),
        ];
        let binary_caps = vec![
            cap("a.first", CapabilityLevel::Grant),
            cap("c.third", CapabilityLevel::Deny),
        ];
        let result = check_cap_divergence(&manifest_caps, &binary_caps, "root");
        assert!(result.is_some());
        if let Some(CapabilityLinkError::CapabilityDivergence {
            missing_from_binary,
            extra_in_binary,
            ..
        }) = result
        {
            assert_eq!(missing_from_binary, vec!["b.second"]);
            assert_eq!(extra_in_binary, vec!["c.third"]);
        } else {
            panic!("expected E2208");
        }
    }
}
