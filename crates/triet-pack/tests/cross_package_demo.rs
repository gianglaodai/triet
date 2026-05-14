//! v0.4.8 — End-to-end cross-package demo.
//!
//! Demonstrates the v0.4 distribution model with two packages:
//!
//! - `math` library: exports `add(a: Integer, b: Integer) -> Integer`.
//! - `app` consumer: declares a dependency on `math >= 1.0.0 < 2.0.0`.
//!
//! This test walks the full distribution pipeline:
//!
//! 1. Each package is built into a `.tripack` (ABI metadata + IR code
//!    section bytes — IR is opaque to this test).
//! 2. Both packs are read back and the linker (`plan_link`) decides
//!    accept / refuse based on ADR-0013 §2 decision matrix.
//! 3. We re-publish `math` at `2.0.0` to demonstrate refuse-to-link.
//!
//! All ADRs from the v0.4 series participate:
//!
//! - ADR-0011 — ABI metadata format (round-trips here).
//! - ADR-0012 — Witness table dispatch (slot exists in `IrProgram`;
//!   end-to-end emit deferred to v0.4.8 demo follow-up since the
//!   lowerer doesn't yet emit `WitnessCall` for cross-package
//!   generics — single-package compile only at v0.4).
//! - ADR-0013 — Semver linking policy (exercised below).
//!
//! No CLI subcommand exists yet for invoking the linker directly; the
//! API is the integration surface, and v0.5 will layer the `triet
//! link` command on top of it. This test is the contract.

use triet_pack::{
    AbiMetadata, Dep, FunctionExport, LinkError, Param, SemVer, TypeRef, Visibility, plan_link,
    read_tripack, write_tripack,
};

/// Build the `math` package's ABI metadata. v1.0.0 publishes
/// `public function add(a: Integer, b: Integer) -> Integer`.
fn build_math_pkg(version: SemVer) -> AbiMetadata {
    let mut meta = AbiMetadata::empty("math", version);
    // TypeTag::Integer encodes as 0x02 inline per ADR-0008.
    let int_ty = TypeRef::Primitive(0x02);
    meta.exports.push(FunctionExport {
        name: "add".into(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: vec![
            Param {
                name: "a".into(),
                type_ref: int_ty.clone(),
            },
            Param {
                name: "b".into(),
                type_ref: int_ty.clone(),
            },
        ],
        return_type: int_ty,
        body_offset: 0,
    });
    meta
}

/// Build the `app` package depending on `math >= 1.0.0 < 2.0.0`.
fn build_app_pkg() -> AbiMetadata {
    let mut meta = AbiMetadata::empty("app", SemVer::new(0, 1, 0));
    meta.deps.push(Dep {
        pkg_name: "math".into(),
        version_min: SemVer::new(1, 0, 0),
        version_max_exclusive: SemVer::new(2, 0, 0),
        iface_hash_pin: triet_pack::IfaceHash::default(),
    });
    // app exports a `main` entry point that (in a real build) would
    // call `math.add`. We only need the ABI shape here; the IR body
    // section can be opaque bytes.
    meta.exports.push(FunctionExport {
        name: "main".into(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRef::Primitive(0x06), // Unit
        body_offset: 0,
    });
    meta
}

/// Serialize a metadata block into a `.tripack` file, then decode it
/// back. This proves both the wire format and the round-trip
/// invariants from ADR-0011 §6 (canonical hash).
fn round_trip(meta: &AbiMetadata) -> AbiMetadata {
    // Empty code section is fine — this test focuses on metadata.
    let bytes = write_tripack(meta, &[]);
    let (decoded, _code) = read_tripack(&bytes).expect("decode .tripack");
    decoded
}

#[test]
fn happy_path_v1_links_cleanly() {
    let math_pack = round_trip(&build_math_pkg(SemVer::new(1, 0, 0)));
    let app_pack = round_trip(&build_app_pkg());

    let plan = plan_link(&app_pack, std::slice::from_ref(&math_pack));

    assert!(
        plan.is_acceptable(),
        "expected acceptance, got errors: {:?}",
        plan.errors,
    );
    assert_eq!(plan.errors.len(), 0);
    assert_eq!(
        plan.resolved["math"].selected_version,
        SemVer::new(1, 0, 0),
    );
    // No drift expected at the floor of the range.
    assert!(plan.warnings.is_empty(), "unexpected warnings: {:?}", plan.warnings);
}

#[test]
fn minor_bump_links_but_warns_about_drift() {
    let math_pack = round_trip(&build_math_pkg(SemVer::new(1, 5, 0)));
    let app_pack = round_trip(&build_app_pkg());

    let plan = plan_link(&app_pack, &[math_pack]);

    assert!(plan.is_acceptable());
    assert!(
        plan.warnings.iter().any(|w| matches!(
            w,
            triet_pack::LinkWarning::IfaceHashDrift { pkg_name, .. } if pkg_name == "math"
        )),
        "expected hash drift warning for minor bump, got: {:?}",
        plan.warnings,
    );
}

#[test]
fn major_bump_refuses_to_link() {
    let math_pack = round_trip(&build_math_pkg(SemVer::new(2, 0, 0)));
    let app_pack = round_trip(&build_app_pkg());

    let plan = plan_link(&app_pack, &[math_pack]);

    assert!(!plan.is_acceptable(), "expected refuse, got plan: {plan:?}");
    let err = plan
        .errors
        .iter()
        .find(|e| matches!(e, LinkError::MajorVersionMismatch { .. }))
        .expect("expected MajorVersionMismatch error");
    if let LinkError::MajorVersionMismatch {
        pkg_name,
        required_min,
        required_max_exclusive,
        found,
    } = err
    {
        assert_eq!(pkg_name, "math");
        assert_eq!(*required_min, SemVer::new(1, 0, 0));
        assert_eq!(*required_max_exclusive, SemVer::new(2, 0, 0));
        assert_eq!(*found, SemVer::new(2, 0, 0));
    }
}

#[test]
fn below_minimum_refuses_to_link() {
    let math_pack = round_trip(&build_math_pkg(SemVer::new(0, 9, 0)));
    let app_pack = round_trip(&build_app_pkg());

    let plan = plan_link(&app_pack, &[math_pack]);

    assert!(!plan.is_acceptable());
    assert!(
        plan.errors
            .iter()
            .any(|e| matches!(e, LinkError::VersionBelowMinimum { .. })),
        "expected VersionBelowMinimum, got: {:?}",
        plan.errors,
    );
}

#[test]
fn missing_dep_refuses_with_clear_error() {
    let app_pack = round_trip(&build_app_pkg());
    let plan = plan_link(&app_pack, &[]); // empty pool — math not provided

    assert!(!plan.is_acceptable());
    let err = plan
        .errors
        .first()
        .expect("expected one error for missing dep");
    assert!(
        matches!(err, LinkError::PackageNotFound { pkg_name } if pkg_name == "math"),
        "expected PackageNotFound for math, got: {err:?}",
    );
}

/// ADR-0011 §1 canonical hash stability: same surface across two
/// independent builds produces the same `iface_hash`.
#[test]
fn iface_hash_stable_across_rebuilds() {
    let pack_a = round_trip(&build_math_pkg(SemVer::new(1, 0, 0)));
    let pack_b = round_trip(&build_math_pkg(SemVer::new(1, 0, 0)));
    assert_eq!(
        pack_a.iface_hash, pack_b.iface_hash,
        "same ABI surface, same hash — required for v0.5 CAS",
    );
}

/// ADR-0011 §1 + ADR-0013: bumping `pkg_version` patch must NOT
/// change `iface_hash` (patch = internal-only changes by definition).
#[test]
fn patch_bump_preserves_iface_hash() {
    let pack_v1 = round_trip(&build_math_pkg(SemVer::new(1, 0, 0)));
    let pack_v1_1 = round_trip(&build_math_pkg(SemVer::new(1, 0, 1)));
    assert_eq!(pack_v1.iface_hash, pack_v1_1.iface_hash);
    assert_ne!(pack_v1.pkg_version, pack_v1_1.pkg_version);
}
