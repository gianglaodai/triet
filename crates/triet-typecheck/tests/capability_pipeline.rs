//! End-to-end capability pipeline tests — phase v0.6 capstone.
//!
//! Each stage of the capability system (compile-time check, link-time
//! refusal, runtime resolve) has unit tests inside its own module.
//! This file is the **executable proof** for the three [ROADMAP §v0.6]
//! gates: it threads the three stages on shared synthetic data and
//! asserts the user-visible outcomes a real demo would observe.
//!
//! Gates closed here:
//!
//! 1. **Compile-time error rõ ràng khi `usr.*` import `dev.*` không có
//!    capability.** — covered by `compile_*` tests below.
//! 2. **Runtime policy hook hoạt động cho `Trilean::Unknown`.** —
//!    covered by `resolve_*` tests using a mock [`PromptCallback`].
//! 3. **Demo capability-restricted program chạy được + bị reject khi
//!    capability sai.** — covered by `full_pipeline_*` tests that
//!    walk compile → link → resolve on the same package shape.
//!
//! The CLI integration (`triet check` reading `triet.package` from a
//! project root, building real `.tripack`s with caps populated, wiring
//! `DevTtyPrompt` into the run path) is **deferred** — it needs a
//! project-layout discovery convention that lands cleaner with v0.7
//! self-hosting. The integration tests below construct synthetic data
//! to prove the pipeline's verdict without a CLI wiring layer.
//!
//! [ROADMAP §v0.6]: ../../../../ROADMAP.md

use std::collections::HashMap;

use triet_modules::{AbsolutePath, ArenaId, Module, ModuleId, ModulePath, ResolvedProgram};
use triet_pack::{
    AbiMetadata, CapabilityClaim, CapabilityLevel, CapabilityLinkError, CapabilityResolver,
    DecisionSource, Module as PackModule, ModuleIfaceHash, ModuleImplHash, PackageManifest,
    PolicyRequest, PolicyRules, PromptCallback, PromptChoice, ResolutionOrigin, ResolverError,
    SemVer, check_link_capabilities,
};
use triet_syntax::Arena;
use triet_typecheck::{CapabilityError, check_capabilities};

// ── Fixture builders ────────────────────────────────────────────────

/// Build a `ResolvedProgram` representing a single user-app module
/// that imports each `(local_name, absolute_path)` pair as a binding.
/// Mirrors the shape `triet-modules` produces after name resolution,
/// minus the AST items we don't need for cap-checking.
fn user_program_with_imports(imports: &[(&str, &str)]) -> ResolvedProgram {
    let mut bindings = HashMap::new();
    for (local, abs_path) in imports {
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
        path: ModulePath::khi_root(),
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

/// Build a `PackageManifest` named `myapp@0.1.0` with the given
/// `requires` entries. v0.6 manifest grammar takes textual level
/// tokens; here we work in the typed `CapabilityLevel` directly so the
/// test wires post-parse semantics.
fn manifest(requires: Vec<(&str, CapabilityLevel)>) -> PackageManifest {
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

/// Build an `AbiMetadata` with the given name + module paths + caps.
/// Used to simulate a `.tripack` for the link-stage check.
fn pack(name: &str, module_paths: &[&str], caps: Vec<(&str, CapabilityLevel)>) -> AbiMetadata {
    let mut m = AbiMetadata::empty(name, SemVer::new(0, 1, 0));
    m.modules = module_paths
        .iter()
        .map(|p| PackModule {
            path: (*p).into(),
            iface_hash_mod: ModuleIfaceHash::default(),
            impl_hash_mod: ModuleImplHash::default(),
        })
        .collect();
    m.caps = caps
        .into_iter()
        .map(|(path, level)| CapabilityClaim {
            cap_path: path.into(),
            level,
        })
        .collect();
    m
}

// ── Gate 1: Compile-time enforcement ────────────────────────────────

#[test]
fn compile_usr_imports_dev_without_claim_fires_e2200() {
    // ROADMAP §v0.6 GATE 1 ✓
    //
    // The classic kernel-style scenario — user-space app trying to
    // touch a hardware namespace without declaring the capability.
    // ADR-0016 §5 rule 1 says the package must explicit-list every
    // cross-root path it uses.
    let program = user_program_with_imports(&[("read_disk", "dev.disk.read")]);
    let manifest = manifest(vec![]);

    let errs = check_capabilities(&program, &manifest);

    assert_eq!(errs.len(), 1, "expected one cap error, got: {errs:?}");
    match &errs[0] {
        CapabilityError::MissingCapabilityClaim {
            cap_path,
            requester_pkg,
            ..
        } => {
            assert_eq!(cap_path, "dev.disk");
            assert_eq!(requester_pkg, "myapp");
        }
        other @ CapabilityError::SelfContradictoryCapability { .. } => {
            panic!("expected MissingCapabilityClaim (E2200), got: {other:?}")
        }
    }
}

#[test]
fn compile_usr_imports_dev_with_grant_passes() {
    // ROADMAP §v0.6 GATE 1 ✓ (positive path)
    //
    // Same import — but now the manifest declares the capability.
    // Compile passes. (Link-time root-authority + path-validity
    // checks remain — covered in the link-stage tests below.)
    let program = user_program_with_imports(&[("read_disk", "dev.disk.read")]);
    let manifest = manifest(vec![("dev.disk", CapabilityLevel::Grant)]);

    let errs = check_capabilities(&program, &manifest);

    assert!(errs.is_empty(), "expected no errors, got: {errs:?}");
}

#[test]
fn compile_usr_imports_dev_with_deny_fires_e2201() {
    // ADR-0016 §5 rule 2 — the manifest contradicting the source.
    // Distinct error code from "missing claim" so the diagnostic can
    // point at the offending `deny` line rather than telling the user
    // to add something they already wrote.
    let program = user_program_with_imports(&[("read_disk", "dev.disk.read")]);
    let manifest = manifest(vec![("dev.disk", CapabilityLevel::Deny)]);

    let errs = check_capabilities(&program, &manifest);

    assert_eq!(errs.len(), 1);
    assert!(
        matches!(
            &errs[0],
            CapabilityError::SelfContradictoryCapability { cap_path, .. }
                if cap_path == "dev.disk"
        ),
        "expected SelfContradictoryCapability (E2201), got: {errs:?}",
    );
}

#[test]
fn compile_std_imports_skip_check() {
    // ADR-0016 §5 rule 3 — `std.*` is ambient. No cap-check fires
    // for stdlib imports. Demonstrates the trade-off: stdlib calls
    // (println, etc.) don't need a manifest entry per call site.
    let program = user_program_with_imports(&[("println", "std.io.println")]);
    let manifest = manifest(vec![]);

    let errs = check_capabilities(&program, &manifest);

    assert!(errs.is_empty(), "std.* is ambient — expected zero errors");
}

// ── Link-stage enforcement (ADR-0018 §2 Step 6a) ────────────────────

#[test]
fn link_dep_request_without_root_authority_fires_e2200() {
    // Compile-stage check fires from the user's perspective ("did I
    // declare what I'm using?"). Link-stage fires from the consumer's
    // perspective ("did the root authorise what its deps want?"). A
    // dep package declaring `sys.io grant` does NOT auto-promote into
    // root's `requires` — ADR-0016 §7 forbids that.
    let root = pack("myapp", &[], vec![]);
    let stdlib = pack(
        "stdlib",
        &["sys.io"],
        vec![("sys.io", CapabilityLevel::Grant)],
    );

    let report = check_link_capabilities(&root, &[stdlib]);

    assert_eq!(report.errors.len(), 1);
    assert!(matches!(
        &report.errors[0],
        CapabilityLinkError::MissingCapabilityClaim { cap_path, .. } if cap_path == "sys.io"
    ));
}

#[test]
fn link_root_deny_with_dep_request_fires_e2203() {
    // Root authoritatively refuses a path some dep wants — refuse-to-
    // link with E2203. Mirrors `apt`'s "package X depends on Y but Y
    // is held back" but at the capability layer.
    let root = pack("myapp", &[], vec![("dev.disk", CapabilityLevel::Deny)]);
    let disk_lib = pack(
        "diskutil",
        &["dev.disk"],
        vec![("dev.disk", CapabilityLevel::Grant)],
    );

    let report = check_link_capabilities(&root, &[disk_lib]);

    assert!(!report.is_acceptable());
    assert!(report.errors.iter().any(|e| matches!(
        e,
        CapabilityLinkError::CapabilityRefused { cap_path, .. } if cap_path == "dev.disk"
    )));
}

// ── Gate 2: Runtime policy hook ─────────────────────────────────────

/// Fixed-response mock callback — lets each resolve test pin the
/// user's "answer" to the TTY prompt without opening `/dev/tty`.
struct FixedCallback(PromptChoice);

impl PromptCallback for FixedCallback {
    fn prompt(&mut self, _req: &PolicyRequest) -> std::io::Result<PromptChoice> {
        Ok(self.0)
    }
}

#[test]
fn resolve_defer_with_grant_callback_returns_positive() {
    // ROADMAP §v0.6 GATE 2 ✓
    //
    // Manifest declares Defer, policy rule says `prompt`, user grants
    // via the callback → resolver returns `Trit::Positive` with
    // `InteractivePrompt` provenance. Demonstrates the full
    // Trilean::Unknown → runtime decision flow.
    use triet_core::Trit;

    let rules = PolicyRules::parse(
        "format_version 1\n\
         rule sys.net.dns fresh prompt\n",
    )
    .expect("policy parses");

    let mut resolver = CapabilityResolver::new(rules)
        .with_prompt_callback(Box::new(FixedCallback(PromptChoice::GrantOnce)));

    let req = PolicyRequest {
        cap_path: "sys.net.dns".into(),
        requester_pkg: "myapp".into(),
        dep_chain: vec!["myapp".into(), "libdns".into()],
        origin: ResolutionOrigin::Fresh,
    };
    let decision = resolver.resolve(&req);

    assert_eq!(decision.outcome, Trit::Positive);
    assert!(matches!(decision.source, DecisionSource::InteractivePrompt));
}

#[test]
fn resolve_defer_without_callback_fails_closed_nontty() {
    // Headless run, no callback attached → policy rule of `prompt`
    // can't be resolved, so the resolver fails closed with
    // `Trit::Negative` and surfaces `E2205.NonTTYDefer`. This is the
    // CI-safe path: refuse over guess (VISION §6).
    use triet_core::Trit;

    let rules = PolicyRules::parse(
        "format_version 1\n\
         rule sys.net.dns fresh prompt\n",
    )
    .expect("policy parses");

    let mut resolver = CapabilityResolver::new(rules);
    let req = PolicyRequest {
        cap_path: "sys.net.dns".into(),
        requester_pkg: "myapp".into(),
        dep_chain: vec!["myapp".into()],
        origin: ResolutionOrigin::Fresh,
    };
    let decision = resolver.resolve(&req);

    assert_eq!(decision.outcome, Trit::Negative);
    match decision.source {
        DecisionSource::Error(ResolverError::NonTTYDefer {
            cap_path,
            requester_pkg,
        }) => {
            assert_eq!(cap_path, "sys.net.dns");
            assert_eq!(requester_pkg, "myapp");
        }
        other => panic!("expected NonTTYDefer, got: {other:?}"),
    }
}

#[test]
fn resolve_cached_decision_is_monotonic_under_replay() {
    // ADR-0017 §5 monotonicity invariant: once a (cap_path,
    // requester_pkg) key resolves, the decision is frozen for the
    // session. Subsequent resolves return the same outcome with
    // `DecisionSource::Cache`.
    use triet_core::Trit;

    let rules = PolicyRules::parse(
        "format_version 1\n\
         rule sys.io fresh +1\n",
    )
    .expect("policy parses");

    let mut resolver = CapabilityResolver::new(rules);
    let req = PolicyRequest {
        cap_path: "sys.io".into(),
        requester_pkg: "myapp".into(),
        dep_chain: vec!["myapp".into()],
        origin: ResolutionOrigin::Fresh,
    };

    let first = resolver.resolve(&req);
    assert_eq!(first.outcome, Trit::Positive);
    assert!(matches!(first.source, DecisionSource::ConfigRule));

    let replay = resolver.resolve(&req);
    assert_eq!(replay.outcome, Trit::Positive, "outcome must match");
    assert!(matches!(replay.source, DecisionSource::Cache));
}

// ── Gate 3: Capstone — full pipeline ────────────────────────────────

#[test]
fn full_pipeline_capstone_happy_path() {
    // ROADMAP §v0.6 GATE 3 ✓ (positive)
    //
    // Walks compile → link → resolve on a single coherent fixture:
    //
    // `myapp` imports `sys.io.println` and `sys.net.dns.lookup`. Its
    // manifest grants `sys.io` outright and defers `sys.net.dns` to
    // the deploy-time policy. A stdlib pack exports both modules. The
    // policy file rule says `+1` for `sys.net.dns` on Fresh origin.
    //
    // The pipeline should accept at every stage, and the Defer cap
    // should resolve to `Trit::Positive` via the policy rule (no
    // prompt fires — the rule is decisive).
    use triet_core::Trit;

    // — Compile —
    let program = user_program_with_imports(&[
        ("println", "sys.io.println"),
        ("lookup", "sys.net.dns.lookup"),
    ]);
    let user_manifest = manifest(vec![
        ("sys.io", CapabilityLevel::Grant),
        ("sys.net.dns", CapabilityLevel::Defer),
    ]);
    let compile_errs = check_capabilities(&program, &user_manifest);
    assert!(
        compile_errs.is_empty(),
        "compile stage must accept; got: {compile_errs:?}",
    );

    // — Link —
    let root = pack(
        "myapp",
        &[],
        vec![
            ("sys.io", CapabilityLevel::Grant),
            ("sys.net.dns", CapabilityLevel::Defer),
        ],
    );
    let stdlib = pack(
        "stdlib",
        &["sys.io", "sys.net.dns"],
        vec![
            ("sys.io", CapabilityLevel::Grant),
            ("sys.net.dns", CapabilityLevel::Grant),
        ],
    );
    let link_report = check_link_capabilities(&root, &[stdlib]);
    assert!(
        link_report.is_acceptable(),
        "link stage must accept; got: {:?}",
        link_report.errors,
    );
    // Defer collected for runtime resolution.
    assert_eq!(
        link_report.deferrals.len(),
        1,
        "sys.net.dns must be in deferrals",
    );
    assert_eq!(link_report.deferrals[0].cap_path, "sys.net.dns");

    // — Resolve —
    let rules = PolicyRules::parse(
        "format_version 1\n\
         rule sys.net.dns fresh +1\n",
    )
    .expect("policy parses");
    let mut resolver = CapabilityResolver::new(rules);
    let req = PolicyRequest {
        cap_path: "sys.net.dns".into(),
        requester_pkg: "myapp".into(),
        dep_chain: vec!["myapp".into(), "stdlib".into()],
        origin: ResolutionOrigin::Fresh,
    };
    let decision = resolver.resolve(&req);
    assert_eq!(
        decision.outcome,
        Trit::Positive,
        "policy rule grants → resolver returns Positive",
    );
    assert!(matches!(decision.source, DecisionSource::ConfigRule));
}

#[test]
fn demo_files_parse_with_v06_grammar() {
    // Ensures the `demos/04-capability-system/` illustrative files
    // stay in sync with the parser. If a grammar change breaks the
    // demo's `triet.package` or `triet.policy`, this test fails
    // before the change ships — the README's gate walkthrough would
    // otherwise reference text the parser refuses.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let demo_root = workspace_root.join("demos").join("04-capability-system");

    let manifest = PackageManifest::load(&demo_root.join("triet.package"))
        .expect("demo triet.package must parse with current grammar");
    assert_eq!(manifest.name, "myapp");
    assert_eq!(manifest.version, SemVer::new(0, 1, 0));
    assert_eq!(manifest.requires.len(), 3);

    let policy = PolicyRules::load(&demo_root.join("triet.policy"))
        .expect("demo triet.policy must parse with current grammar");
    assert_eq!(policy.rules().len(), 3);
}

#[test]
fn full_pipeline_capstone_refuse_path() {
    // ROADMAP §v0.6 GATE 3 ✓ (refusal)
    //
    // Same `myapp` but the manifest forgets to declare `dev.disk`
    // while the source uses it. Compile stage refuses BEFORE link or
    // resolve get a chance — the "demo capability-restricted program
    // bị reject khi capability sai" expectation.
    let program = user_program_with_imports(&[("read_disk", "dev.disk.read")]);
    let user_manifest = manifest(vec![]);

    let compile_errs = check_capabilities(&program, &user_manifest);

    assert_eq!(compile_errs.len(), 1);
    assert!(matches!(
        &compile_errs[0],
        CapabilityError::MissingCapabilityClaim { cap_path, .. } if cap_path == "dev.disk"
    ));

    // No need to proceed to link/resolve — compile stage already
    // refused. This matches the "fail fast" UX of `triet build` /
    // `triet check`: the user sees the diagnostic and fixes the
    // manifest before any link work happens.
}
