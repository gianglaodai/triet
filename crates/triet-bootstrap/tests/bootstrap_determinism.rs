//! Bootstrap determinism gate per [ADR-0019 §3].
//!
//! Verifies that the Rust impl's emission path is canonical: same
//! input produces byte-identical output across rebuilds. This is the
//! precondition for the v0.7 bootstrap loop gate (Stage 2 ≡ Stage 3
//! byte-identical, [ADR-0019 §4]).
//!
//! Two test layers per Q5-C:
//!
//! 1. **`.triv` determinism** — for each `examples/*.tri`, run the
//!    full `parse → resolve → typecheck → lower → write_program`
//!    pipeline 3 times and assert all 3 output buffers are byte-equal.
//!    3 runs × 11 examples = 33 builds per CI invocation (Q4-B).
//! 2. **`.khi` sort-at-boundary** — construct equivalent
//!    `AbiMetadata` values with `types`/`exports`/`deps` shuffled into
//!    two different orders, run `write_khi` on both, assert
//!    byte-equal. This pins the v0.7.2 fix (sort-at-boundary in
//!    `write_module_table` / `write_type_table` / `write_export_table`
//!    / `write_dep_table`) against future regressions.
//!
//! [ADR-0019 §3]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
//! [ADR-0019 §4]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::path::{Path, PathBuf};

use triet_pack::{
    AbiMetadata, CapabilityClaim, CapabilityLevel, Dep, FieldDef, FunctionExport, IfaceHash,
    Module, ModuleIfaceHash, ModuleImplHash, Param, SemVer, StructDef, TermIfaceHash, TermImplHash,
    TypeDef, TypeKind, TypeRef, Visibility, write_khi,
};

/// Number of rebuilds per example per Q4-B decision. Nondeterminism
/// from a single 1-bit source typically reproduces within 2 runs; 3
/// is the smallest count that still gives statistical confidence
/// without ballooning CI time. Raise here if a missed bug shows up
/// in a `v0.7.x.review` audit.
const REBUILDS_PER_EXAMPLE: usize = 3;

/// All `examples/*.tri` files exercised by the determinism gate. Kept
/// as an explicit list rather than a directory scan so the test does
/// not itself depend on filesystem iteration order — that would
/// reintroduce exactly the nondeterminism source we're guarding
/// against.
const EXAMPLE_NAMES: &[&str] = &[
    "counter",
    "enumerate",
    "factorial",
    "fizzbuzz",
    "generic",
    "long_arithmetic",
    "lukasiewicz_vs_kleene",
    "maybe",
    "measles_risk",
    "nullable",
    "while_polling",
];

/// Walk up from the bootstrap crate's manifest dir to the workspace
/// root and join `examples/<name>.tri`. Same convention as
/// `triet-modules` uses for the stdlib lookup.
fn example_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("examples")
        .join(format!("{name}.tri"))
}

/// Run the full Rust-impl emission path on `path`, returning the
/// `.triv` bytes. Panics on any pipeline failure — for the
/// determinism test we only care about reproducibility of *successful*
/// builds.
///
/// Filters out Warning-severity diagnostics (W2001 `NullDeprecated`
/// per ADR-0020 §10.3) — these do not block compile until v1.0. The
/// `examples/*.tri` files still use legacy `null` syntax until the
/// `triet fmt --fix --migrate-null` tool ships in v0.7.4.3-error.4.
fn build_triv(path: &Path) -> Vec<u8> {
    use miette::Diagnostic;
    let resolved = triet_modules::load_program(path).expect("load_program");
    let diagnostics = triet_typecheck::check_resolved(&resolved);
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(
        blocking.is_empty(),
        "type errors in {}: {:?}",
        path.display(),
        blocking
    );
    let ir = triet_ir::lower_program(&resolved);
    triet_ir::write_program(&ir)
}

#[test]
fn triv_emission_byte_identical_across_rebuilds() {
    for name in EXAMPLE_NAMES {
        let path = example_path(name);
        assert!(
            path.is_file(),
            "missing example file: {} — adjust EXAMPLE_NAMES",
            path.display()
        );

        let baseline = build_triv(&path);
        assert!(!baseline.is_empty(), "{name}: empty .triv");

        for run in 1..REBUILDS_PER_EXAMPLE {
            let again = build_triv(&path);
            assert_eq!(
                baseline.len(),
                again.len(),
                "{name}: .triv length diverged on rebuild #{run} \
                 ({} vs {} bytes)",
                baseline.len(),
                again.len(),
            );
            assert_eq!(
                baseline, again,
                "{name}: .triv bytes diverged on rebuild #{run} \
                 — nondeterminism in emit path (ADR-0019 §3)",
            );
        }
    }
}

/// Build the same logical `AbiMetadata` in two different input
/// orderings. The on-disk `.khi` bytes must be byte-equal because
/// the writers sort at the boundary per ADR-0019 §3 / Q2-C / commit
/// landing in v0.7.2.
fn build_meta_v1() -> AbiMetadata {
    let mut meta = AbiMetadata::empty("multi", SemVer::new(0, 1, 0));
    // modules — three with non-sorted paths
    meta.modules.push(Module {
        path: "khi.helper".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    meta.modules.push(Module {
        path: "khi".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    meta.modules.push(Module {
        path: "khi.aux".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    // types — two structs with non-sorted names
    meta.types.push(TypeDef {
        name: "Zebra".into(),
        module_path: "khi".into(),
        kind: TypeKind::Struct,
        type_params: Vec::new(),
        struct_body: Some(StructDef {
            fields: vec![FieldDef {
                name: "stripes".into(),
                type_ref: TypeRef::Primitive(0x02),
                visibility: Visibility::Public,
            }],
        }),
        enum_body: None,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    meta.types.push(TypeDef {
        name: "Apple".into(),
        module_path: "khi".into(),
        kind: TypeKind::Struct,
        type_params: Vec::new(),
        struct_body: Some(StructDef {
            fields: vec![FieldDef {
                name: "weight".into(),
                type_ref: TypeRef::Primitive(0x02),
                visibility: Visibility::Public,
            }],
        }),
        enum_body: None,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    // exports — three with non-sorted names
    let int_ty = TypeRef::Primitive(0x02);
    for name in ["zoo", "alpha", "middle"] {
        meta.exports.push(FunctionExport {
            name: name.into(),
            module_path: "khi".into(),
            visibility: Visibility::Public,
            type_params: Vec::new(),
            params: vec![Param {
                name: "x".into(),
                type_ref: int_ty.clone(),
            }],
            return_type: int_ty.clone(),
            body_offset: 0,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
    }
    // deps — three with non-sorted names
    for name in ["zlib", "alpha_dep", "midware"] {
        meta.deps.push(Dep {
            pkg_name: name.into(),
            version_min: SemVer::new(1, 0, 0),
            version_max_exclusive: SemVer::new(2, 0, 0),
            iface_hash_pin: IfaceHash::default(),
        });
    }
    // caps — non-sorted paths (already covered by encode_caps_for_hash
    // but include here to lock the full canonical surface)
    meta.caps.push(CapabilityClaim {
        cap_path: "sys.net".into(),
        level: CapabilityLevel::Grant,
    });
    meta.caps.push(CapabilityClaim {
        cap_path: "sys.io".into(),
        level: CapabilityLevel::Grant,
    });
    meta
}

fn build_meta_v2_reordered() -> AbiMetadata {
    // Same logical content as `build_meta_v1` but every collection
    // built in a different insertion order. The sort-at-boundary
    // contract in the writers must collapse these to identical bytes.
    let mut meta = AbiMetadata::empty("multi", SemVer::new(0, 1, 0));
    meta.modules.push(Module {
        path: "khi.aux".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    meta.modules.push(Module {
        path: "khi.helper".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    meta.modules.push(Module {
        path: "khi".into(),
        iface_hash_mod: ModuleIfaceHash::default(),
        impl_hash_mod: ModuleImplHash::default(),
    });
    meta.types.push(TypeDef {
        name: "Apple".into(),
        module_path: "khi".into(),
        kind: TypeKind::Struct,
        type_params: Vec::new(),
        struct_body: Some(StructDef {
            fields: vec![FieldDef {
                name: "weight".into(),
                type_ref: TypeRef::Primitive(0x02),
                visibility: Visibility::Public,
            }],
        }),
        enum_body: None,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    meta.types.push(TypeDef {
        name: "Zebra".into(),
        module_path: "khi".into(),
        kind: TypeKind::Struct,
        type_params: Vec::new(),
        struct_body: Some(StructDef {
            fields: vec![FieldDef {
                name: "stripes".into(),
                type_ref: TypeRef::Primitive(0x02),
                visibility: Visibility::Public,
            }],
        }),
        enum_body: None,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    let int_ty = TypeRef::Primitive(0x02);
    for name in ["middle", "zoo", "alpha"] {
        meta.exports.push(FunctionExport {
            name: name.into(),
            module_path: "khi".into(),
            visibility: Visibility::Public,
            type_params: Vec::new(),
            params: vec![Param {
                name: "x".into(),
                type_ref: int_ty.clone(),
            }],
            return_type: int_ty.clone(),
            body_offset: 0,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
    }
    for name in ["midware", "zlib", "alpha_dep"] {
        meta.deps.push(Dep {
            pkg_name: name.into(),
            version_min: SemVer::new(1, 0, 0),
            version_max_exclusive: SemVer::new(2, 0, 0),
            iface_hash_pin: IfaceHash::default(),
        });
    }
    meta.caps.push(CapabilityClaim {
        cap_path: "sys.io".into(),
        level: CapabilityLevel::Grant,
    });
    meta.caps.push(CapabilityClaim {
        cap_path: "sys.net".into(),
        level: CapabilityLevel::Grant,
    });
    meta
}

#[test]
fn khi_writer_sorts_at_boundary() {
    let bytes_v1 = write_khi(&build_meta_v1(), &[]);
    let bytes_v2 = write_khi(&build_meta_v2_reordered(), &[]);

    assert_eq!(
        bytes_v1.len(),
        bytes_v2.len(),
        ".khi length diverged between equivalent inputs in \
         different insertion orders ({} vs {} bytes) — \
         sort-at-boundary regression (ADR-0019 §3, v0.7.2 fix)",
        bytes_v1.len(),
        bytes_v2.len(),
    );
    assert_eq!(
        bytes_v1, bytes_v2,
        ".khi bytes diverged between equivalent inputs in \
         different insertion orders — sort-at-boundary regression \
         (ADR-0019 §3, v0.7.2 fix)",
    );
}

#[test]
fn khi_writer_byte_identical_across_rebuilds() {
    let meta = build_meta_v1();
    let baseline = write_khi(&meta, &[]);

    for run in 1..REBUILDS_PER_EXAMPLE {
        let again = write_khi(&meta, &[]);
        assert_eq!(
            baseline, again,
            ".khi bytes diverged on rebuild #{run} \
             — emit-path nondeterminism (ADR-0019 §3)",
        );
    }
}
