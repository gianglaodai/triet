//! Cross-root capability check (ADR-0016 §5).
//!
//! Compile-time enforcement that every cross-root import in the
//! resolved program has a corresponding `requires` claim in the
//! package manifest. Two rules at v0.6.7:
//!
//! - **Rule 1 (E2200 `MissingCapabilityClaim`)** — an import path
//!   crosses the package boundary (`sys.*` / `dev.*` / `usr.*`) but
//!   the manifest has no matching `requires`. The package must
//!   declare what it intends to use.
//! - **Rule 2 (E2201 `SelfContradictoryCapability`)** — the manifest
//!   claims `deny` for a path the source imports. The package is
//!   refusing what it's about to use.
//!
//! Other rules from ADR-0016 §6 fire at different stages:
//!
//! - `E2202 UnresolvedCapabilityPath` → link-time (v0.6.8)
//! - `E2203 CapabilityRefused` → link-time, root authority check
//! - `E2204 DuplicateCapabilityDecl` → parse stage (v0.6.5)
//! - `E2205 …` → runtime resolver (v0.6.9+)
//! - `E2206 InvalidCapabilityRoot` → parse stage (v0.6.5)
//! - `E2207 InvalidCapabilityLevel` → wire parse (v0.6.4)
//! - `E2208 …` → loader (v0.6.8)
//!
//! Per ADR-0016 §5 rule 3 the check fires **only on cross-root
//! boundaries**:
//!
//! | Import root | Behaviour |
//! |---|---|
//! | `crate` / `self` / `super` | Intra-package, skipped |
//! | `std` / `core` | Ambient, skipped |
//! | `sys` / `dev` / `usr` | Cross-root, checked against manifest |
//! | anything else | Already rejected by name resolution |
//!
//! Per ADR-0016 §2 path matching is **exact**, no inheritance.
//! `requires sys.io grant` does not cover `sys.io.async`.
//!
//! **Span placeholder** — bindings carry no span at v0.6.7 (the
//! resolver records local-name → `AbsolutePath` only). Diagnostics
//! emit `Span(0..0)`; span recovery via `Item::Import` walk lands in
//! v0.6.8 with the linker integration.

use std::collections::HashSet;

use miette::Diagnostic;
use thiserror::Error;
use triet_modules::ResolvedProgram;
use triet_pack::{CapabilityLevel, PackageManifest};
use triet_syntax::Span;

/// One capability-check failure at the source → manifest boundary.
///
/// Lives in the `triet::capability::E22XX` namespace (ADR-0016 §6).
/// v0.6.7 ships the two compile-stage variants; link- and run-time
/// variants land alongside the linker (v0.6.8) and resolver (v0.6.9+).
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum CapabilityError {
    /// The source imports a cross-root path that the manifest doesn't
    /// declare. ADR-0016 §5 rule 1.
    #[error(
        "package `{requester_pkg}` imports `{cap_path}` but `triet.package` has no \
         matching `requires` entry"
    )]
    #[diagnostic(
        code(triet::capability::E2200),
        help(
            "add `requires {cap_path} grant` (or `defer` to leave it to the deploy-time \
             policy) to `triet.package`. ADR-0016 §5 rule 1."
        )
    )]
    MissingCapabilityClaim {
        /// Package name from the manifest — identifies the requester
        /// in dep-chain diagnostics later (link + runtime stages).
        requester_pkg: String,
        /// Dotted module path the source attempted to reach
        /// (e.g. `"sys.io"`, `"dev.disk"`).
        cap_path: String,
        /// Source location. v0.6.7 emits `Span(0..0)` since the
        /// resolver's binding map carries no span; v0.6.8 will refine
        /// by walking `Item::Import` declarations.
        #[label("missing manifest entry for this import")]
        span: Span,
    },

    /// The manifest itself contradicts the source — `requires <path>
    /// deny` but the source imports `<path>`. ADR-0016 §5 rule 2.
    #[error(
        "package `{requester_pkg}` denies `{cap_path}` in `triet.package` but the \
         source imports it"
    )]
    #[diagnostic(
        code(triet::capability::E2201),
        help(
            "either remove the `requires {cap_path} deny` entry from `triet.package`, \
             or remove the import. ADR-0016 §5 rule 2."
        )
    )]
    SelfContradictoryCapability {
        /// Package name from the manifest.
        requester_pkg: String,
        /// Dotted module path simultaneously imported and denied.
        cap_path: String,
        /// Source location placeholder; refines in v0.6.8.
        #[label("manifest denies this import")]
        span: Span,
    },
}

/// Check every cross-root import against the manifest's `requires`
/// table. Returns an empty `Vec` on success; one entry per violation
/// otherwise.
///
/// Per-module deduplication: an import path appearing multiple times
/// in one module produces a single error per module. The same path
/// imported from two distinct modules produces two errors (callers
/// can tie each one back to its module).
///
/// Run **after** [`crate::check_resolved`] — name resolution must
/// have completed so every binding maps to a real `AbsolutePath`.
///
/// Span quality: v0.6.7 emits `Span(0..0)`. v0.6.8 will refine by
/// walking each module's `Item::Import` declarations to recover the
/// source location of each cross-root import statement.
#[must_use]
pub fn check_capabilities(
    program: &ResolvedProgram,
    manifest: &PackageManifest,
) -> Vec<CapabilityError> {
    let mut errors = Vec::new();

    for module in &program.modules {
        // Per-module dedupe so importing `sys.io` ten times in one
        // module surfaces one diagnostic, not ten.
        let mut checked: HashSet<String> = HashSet::new();

        for abs_path in module.bindings.values() {
            let mod_path = abs_path.module_path();

            // Skip self-imports of the module's own definitions.
            if *mod_path == module.path {
                continue;
            }

            // Cross-root check fires only on sys/dev/usr roots. All
            // other roots are intra-pkg (crate/self/super), ambient
            // (std/core), or already rejected by name resolution.
            let Some(root) = mod_path.root() else {
                continue;
            };
            if !matches!(root, "sys" | "dev" | "usr") {
                continue;
            }

            let cap_path = mod_path.to_string();
            if !checked.insert(cap_path.clone()) {
                // Already checked this path inside this module.
                continue;
            }

            // ADR-0016 §2 — exact match, no path inheritance.
            let claim = manifest
                .requires
                .iter()
                .find(|c| c.cap_path == cap_path);

            match claim {
                None => errors.push(CapabilityError::MissingCapabilityClaim {
                    requester_pkg: manifest.name.clone(),
                    cap_path,
                    span: 0..0,
                }),
                Some(c) if c.level == CapabilityLevel::Deny => {
                    errors.push(CapabilityError::SelfContradictoryCapability {
                        requester_pkg: manifest.name.clone(),
                        cap_path,
                        span: 0..0,
                    });
                }
                // Grant / Ambient / Defer all pass compile-time
                // self-consistency. Link-time root authority decides
                // Ambient (root collapses to Deny if no caller); the
                // runtime resolver decides Defer.
                Some(_) => {}
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_modules::{AbsolutePath, Module, ModuleId, ModulePath};
    use triet_pack::{CapabilityClaim, CapabilityLevel as PackLevel, SemVer};

    // ── Test fixtures ──────────────────────────────────────────────

    fn manifest_with(requires: Vec<(&str, PackLevel)>) -> PackageManifest {
        let mut m = PackageManifest::new("myapp", SemVer::new(0, 1, 0));
        m.requires = requires
            .into_iter()
            .map(|(path, level)| CapabilityClaim {
                cap_path: path.into(),
                level,
            })
            .collect();
        m
    }

    /// Build a `ResolvedProgram` with one user module containing the
    /// given imports as bindings. Useful as a minimal fixture — we
    /// don't need real arenas / items for the cap-check pass.
    fn program_with_imports(imports: &[(&str, &str)]) -> ResolvedProgram {
        use std::collections::HashMap;
        use triet_modules::ArenaId;
        use triet_syntax::Arena;

        let mut bindings = HashMap::new();
        for (local, abs_path) in imports {
            // Split "sys.io.println" into ("sys.io", "println").
            let dotted = (*abs_path).to_string();
            let mut segs: Vec<&str> = dotted.split('.').collect();
            let item = segs.pop().expect("non-empty path");
            let module = ModulePath::new(segs.iter().map(|s| (*s).to_owned()).collect());
            bindings.insert(
                (*local).to_owned(),
                AbsolutePath::new(module, item.to_owned()),
            );
        }
        let module = Module {
            path: ModulePath::crate_root(),
            source_path: None,
            arena_id: ArenaId(0),
            items: Vec::new(),
            bindings,
            parent: None,
            children: Vec::new(),
        };
        ResolvedProgram {
            arenas: vec![Arena::new()],
            modules: vec![module],
            root: ModuleId(0),
        }
    }

    // ── Happy paths ────────────────────────────────────────────────

    #[test]
    fn import_with_matching_grant_passes() {
        let program = program_with_imports(&[("println", "sys.io.println")]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Grant)]);
        let errs = check_capabilities(&program, &manifest);
        assert!(errs.is_empty(), "expected no errors, got: {errs:?}");
    }

    #[test]
    fn import_with_defer_passes_compile_stage() {
        // Defer is for the runtime resolver — compile only checks
        // self-consistency.
        let program = program_with_imports(&[("dns", "sys.net.dns.lookup")]);
        let manifest = manifest_with(vec![("sys.net.dns", PackLevel::Defer)]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    #[test]
    fn import_with_ambient_claim_passes() {
        // Ambient is link-time; compile doesn't flag it.
        let program = program_with_imports(&[("foo", "sys.io.foo")]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Ambient)]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    #[test]
    fn std_imports_skip_check() {
        let program = program_with_imports(&[("println", "std.io.println")]);
        let manifest = manifest_with(vec![]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    #[test]
    fn core_imports_skip_check() {
        let program = program_with_imports(&[("Trit", "core.types.Trit")]);
        let manifest = manifest_with(vec![]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    #[test]
    fn crate_imports_skip_check() {
        let program = program_with_imports(&[("helper", "crate.util.helper")]);
        let manifest = manifest_with(vec![]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    #[test]
    fn orphan_claim_without_import_passes() {
        // Manifest claims something the source never imports. Allowed
        // — orphan claims are a sysadmin's prerogative; no warning
        // at v0.6.7. (Future enhancement: lint-level warning.)
        let program = program_with_imports(&[]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Grant)]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    // ── Rule 1 — Missing claim (E2200) ─────────────────────────────

    #[test]
    fn import_without_claim_fires_e2200() {
        let program = program_with_imports(&[("println", "sys.io.println")]);
        let manifest = manifest_with(vec![]);
        let errs = check_capabilities(&program, &manifest);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            CapabilityError::MissingCapabilityClaim {
                cap_path,
                requester_pkg,
                ..
            } => {
                assert_eq!(cap_path, "sys.io");
                assert_eq!(requester_pkg, "myapp");
            }
            other @ CapabilityError::SelfContradictoryCapability { .. } => {
                panic!("unexpected error: {other:?}")
            }
        }
    }

    #[test]
    fn dev_import_without_claim_fires_e2200() {
        let program = program_with_imports(&[("disk", "dev.disk.read")]);
        let manifest = manifest_with(vec![]);
        let errs = check_capabilities(&program, &manifest);
        assert!(matches!(
            errs.as_slice(),
            [CapabilityError::MissingCapabilityClaim { cap_path, .. }]
                if cap_path == "dev.disk",
        ));
    }

    #[test]
    fn usr_import_without_claim_fires_e2200() {
        let program = program_with_imports(&[("foo", "usr.somelib.foo")]);
        let manifest = manifest_with(vec![]);
        let errs = check_capabilities(&program, &manifest);
        assert!(matches!(
            errs.as_slice(),
            [CapabilityError::MissingCapabilityClaim { cap_path, .. }]
                if cap_path == "usr.somelib",
        ));
    }

    // ── Rule 2 — Self-contradictory (E2201) ────────────────────────

    #[test]
    fn import_with_deny_claim_fires_e2201() {
        let program = program_with_imports(&[("println", "sys.io.println")]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Deny)]);
        let errs = check_capabilities(&program, &manifest);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            CapabilityError::SelfContradictoryCapability {
                cap_path,
                requester_pkg,
                ..
            } => {
                assert_eq!(cap_path, "sys.io");
                assert_eq!(requester_pkg, "myapp");
            }
            other @ CapabilityError::MissingCapabilityClaim { .. } => {
                panic!("unexpected error: {other:?}")
            }
        }
    }

    // ── ADR-0016 §2 — Exact match, no inheritance ──────────────────

    #[test]
    fn parent_claim_does_not_cover_child_path() {
        // ADR-0016 §2: `requires sys.io grant` does NOT cover
        // `sys.io.async`. Each path is a separate declaration.
        // Import resolves to module `sys.io.async`, claim is for
        // `sys.io` — should fire E2200.
        let program = program_with_imports(&[("async_op", "sys.io.async.op")]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Grant)]);
        let errs = check_capabilities(&program, &manifest);
        assert!(matches!(
            errs.as_slice(),
            [CapabilityError::MissingCapabilityClaim { cap_path, .. }]
                if cap_path == "sys.io.async",
        ));
    }

    #[test]
    fn exact_match_for_deep_path_passes() {
        // Same setup but claim matches exactly.
        let program = program_with_imports(&[("async_op", "sys.io.async.op")]);
        let manifest = manifest_with(vec![("sys.io.async", PackLevel::Grant)]);
        assert!(check_capabilities(&program, &manifest).is_empty());
    }

    // ── Dedupe semantics ──────────────────────────────────────────

    #[test]
    fn multiple_imports_same_path_one_error_per_module() {
        let program = program_with_imports(&[
            ("println", "sys.io.println"),
            ("print", "sys.io.print"),
            ("eprintln", "sys.io.eprintln"),
        ]);
        let manifest = manifest_with(vec![]);
        let errs = check_capabilities(&program, &manifest);
        assert_eq!(errs.len(), 1, "expected dedupe, got: {errs:?}");
    }

    #[test]
    fn distinct_missing_paths_emit_distinct_errors() {
        let program = program_with_imports(&[
            ("a", "sys.io.a"),
            ("b", "dev.disk.b"),
            ("c", "usr.somelib.c"),
        ]);
        let manifest = manifest_with(vec![]);
        let errs = check_capabilities(&program, &manifest);
        assert_eq!(errs.len(), 3);
        let paths: HashSet<String> = errs
            .iter()
            .filter_map(|e| match e {
                CapabilityError::MissingCapabilityClaim { cap_path, .. } => {
                    Some(cap_path.clone())
                }
                CapabilityError::SelfContradictoryCapability { .. } => None,
            })
            .collect();
        assert!(paths.contains("sys.io"));
        assert!(paths.contains("dev.disk"));
        assert!(paths.contains("usr.somelib"));
    }

    #[test]
    fn mixed_passing_and_failing_imports() {
        let program = program_with_imports(&[
            ("ok_print", "sys.io.println"),       // claim present
            ("std_print", "std.io.println"),      // ambient, skip
            ("crate_helper", "crate.util.h"),     // intra-pkg, skip
            ("missing", "dev.disk.read"),         // no claim → E2200
        ]);
        let manifest = manifest_with(vec![("sys.io", PackLevel::Grant)]);
        let errs = check_capabilities(&program, &manifest);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            CapabilityError::MissingCapabilityClaim { cap_path, .. } if cap_path == "dev.disk",
        ));
    }
}
