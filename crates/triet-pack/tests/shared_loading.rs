//! v0.5.6 — Shared loading demo.
//!
//! VISION §3.1: *"10 ứng dụng dùng `String.format` chỉ load 1 bản
//! vào RAM."*
//!
//! This integration test proves the v0.5 CAS dedup machinery delivers
//! that promise at the **iface level**: when two packs export terms
//! with identical canonical signatures, the store reuses a single
//! `term/<hash>/` directory.
//!
//! ### Gate status (per ADR-0014 + ADR-0015)
//!
//! ✅ **Iface-level dedup** (this test) — `iface_hash_term` is a pure
//! function of canonical signature bytes, so structurally identical
//! exports collapse to one CAS address regardless of which pack they
//! came from.
//!
//! ⏳ **Body-level dedup** (deferred to v0.5.8 / v0.6) — requires the
//! lowerer to split per-term IR bodies so `compute_term_impl_hash`
//! sees real bytes. At v0.5.3 we compute `impl_hash_term` from empty
//! bodies, which means structurally identical exports also share
//! `impl_hash_term`, so the term dir (keyed by `impl_hash` per
//! ADR-0015 §2) dedups too — but `body.bin` files aren't yet written.
//! The full RAM-sharing promise lands when bodies are wired through.
//!
//! ### What this test verifies end-to-end
//!
//! 1. Build two packs (`app_a`, `app_b`) that both declare a shared
//!    function `format(s: Text, x: Integer) -> Text` in a `std.text`
//!    module.
//! 2. Install both into a single store via `Store::install_pack`.
//! 3. Filesystem inspection: exactly one `term/<hash>/iface.bin`
//!    exists for the shared function — even though two packs were
//!    installed.
//! 4. Module-level rollup also dedups when the module contents match.
//! 5. Pack-level dirs differ (different `impl_hash_pkg` because
//!    `pkg_name` differs).
//! 6. Resolver picks the same `std` for both apps when both depend on
//!    it.

// Tests pair variables for two apps ("a" + "b"). Clippy thinks
// `app_a_meta` / `app_b_meta` etc. are too similar — but the parallel
// naming is exactly the point here.
#![allow(clippy::similar_names)]

use std::collections::HashSet;
use std::fs;

use tempfile::TempDir;
use triet_pack::{
    AbiMetadata, Dep, FunctionExport, IfaceHash, Param, SemVer, Store, TermIfaceHash, TermImplHash,
    TypeRef, Visibility, read_khi, write_khi,
};

/// Build the canonical `std.text` package shared by both apps.
fn build_std_pkg() -> Vec<u8> {
    let mut meta = AbiMetadata::empty("std", SemVer::new(0, 5, 0));
    let text_ty = TypeRef::Primitive(0x07); // Text
    let int_ty = TypeRef::Primitive(0x02); // Integer
    meta.exports.push(FunctionExport {
        name: "format".into(),
        module_path: "std.text".into(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: vec![
            Param {
                name: "s".into(),
                type_ref: text_ty.clone(),
            },
            Param {
                name: "x".into(),
                type_ref: int_ty,
            },
        ],
        return_type: text_ty,
        body_offset: 0,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    write_khi(&meta, &[0xC0, 0xDE])
}

/// Build an app package that re-exports `format` (same canonical
/// signature as `std.text.format`) plus its own `main`. The
/// `body_suffix` byte makes the code section unique between apps so
/// pkg hashes differ even when ABI surfaces overlap.
fn build_app_pkg(name: &str, body_suffix: u8) -> Vec<u8> {
    let mut meta = AbiMetadata::empty(name, SemVer::new(0, 1, 0));

    let text_ty = TypeRef::Primitive(0x07);
    let int_ty = TypeRef::Primitive(0x02);

    // Same canonical signature as std.text.format → same iface_hash_term.
    meta.exports.push(FunctionExport {
        name: "format".into(),
        module_path: "std.text".into(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: vec![
            Param {
                name: "s".into(),
                type_ref: text_ty.clone(),
            },
            Param {
                name: "x".into(),
                type_ref: int_ty,
            },
        ],
        return_type: text_ty,
        body_offset: 0,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });

    // App-specific entry point — differs between apps by name.
    meta.exports.push(FunctionExport {
        name: format!("{name}_main"),
        module_path: name.into(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRef::Primitive(0x06), // Unit
        body_offset: 0,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });

    // Declare dep on std so the resolver can be exercised below.
    meta.deps.push(Dep {
        pkg_name: "std".into(),
        version_min: SemVer::new(0, 5, 0),
        version_max_exclusive: SemVer::new(1, 0, 0),
        iface_hash_pin: IfaceHash::default(),
    });

    write_khi(&meta, &[body_suffix])
}

/// Count subdirectories under `parent` whose names look like 64-hex
/// hashes. Returns the names so tests can sanity-check identity.
fn list_hash_dirs(parent: &std::path::Path) -> HashSet<String> {
    fs::read_dir(parent)
        .map(|iter| {
            iter.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.len() == 64 && n.chars().all(|c| c.is_ascii_hexdigit()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn shared_format_dedups_to_one_term_dir() {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(tmp.path()).unwrap();

    let app_a_bytes = build_app_pkg("app_a", 0x01);
    let app_b_bytes = build_app_pkg("app_b", 0x02);

    let (app_a_meta, _) = read_khi(&app_a_bytes).unwrap();
    let (app_b_meta, _) = read_khi(&app_b_bytes).unwrap();

    // The `format` export's iface_hash_term must match across apps —
    // it's a function of canonical signature bytes only.
    let fmt_a = app_a_meta
        .exports
        .iter()
        .find(|e| e.name == "format")
        .expect("app_a exports format");
    let fmt_b = app_b_meta
        .exports
        .iter()
        .find(|e| e.name == "format")
        .expect("app_b exports format");
    assert_eq!(
        fmt_a.iface_hash_term, fmt_b.iface_hash_term,
        "shared canonical signature must produce identical iface hash"
    );
    assert_eq!(
        fmt_a.impl_hash_term, fmt_b.impl_hash_term,
        "with empty bodies (v0.5.3 placeholder), impl hash also matches"
    );

    // Install both packs.
    store.install_pack(&app_a_bytes).unwrap();
    store.install_pack(&app_b_bytes).unwrap();

    // Filesystem: terms dir contains shared `format` once + each app's
    // private `<name>_main` once each = 3 dirs total (NOT 4).
    let term_dirs = list_hash_dirs(&store.root().join("term"));
    assert_eq!(
        term_dirs.len(),
        3,
        "expected 3 term dirs (1 shared format + 2 distinct mains), got {term_dirs:?}"
    );
}

#[test]
fn shared_module_dedups_when_contents_match() {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(tmp.path()).unwrap();

    let app_a_bytes = build_app_pkg("app_a", 0x01);
    let app_b_bytes = build_app_pkg("app_b", 0x02);
    let (app_a_meta, _) = read_khi(&app_a_bytes).unwrap();
    let (app_b_meta, _) = read_khi(&app_b_bytes).unwrap();

    // Both apps have a `std.text` module containing only `format`.
    // Modules rollup → identical impl_hash_mod for that module.
    let mod_a = app_a_meta
        .modules
        .iter()
        .find(|m| m.path == "std.text")
        .expect("app_a has std.text module");
    let mod_b = app_b_meta
        .modules
        .iter()
        .find(|m| m.path == "std.text")
        .expect("app_b has std.text module");
    assert_eq!(
        mod_a.impl_hash_mod, mod_b.impl_hash_mod,
        "shared module contents → shared module hash"
    );

    store.install_pack(&app_a_bytes).unwrap();
    store.install_pack(&app_b_bytes).unwrap();

    // Filesystem inspection: mod/ contains exactly 3 dirs
    // (1 shared `std.text` + each app's own module for `<name>_main`).
    let mod_dirs = list_hash_dirs(&store.root().join("mod"));
    assert_eq!(
        mod_dirs.len(),
        3,
        "expected 3 mod dirs (1 shared + 2 distinct), got {mod_dirs:?}"
    );
}

#[test]
fn pkg_dirs_remain_distinct_for_distinct_pkg_names() {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let h_a = store.install_pack(&build_app_pkg("app_a", 0x01)).unwrap();
    let h_b = store.install_pack(&build_app_pkg("app_b", 0x02)).unwrap();
    assert_ne!(h_a, h_b, "different pkg_name → different pkg impl_hash");

    let pkg_dirs = list_hash_dirs(&store.root().join("pkg"));
    assert_eq!(pkg_dirs.len(), 2);
}

#[test]
fn standalone_std_pack_installs_and_resolves() {
    use triet_pack::Resolver;

    let tmp = TempDir::new().unwrap();
    let store = Store::open(tmp.path()).unwrap();

    // Publish `std` once.
    let std_hash = store.install_pack(&build_std_pkg()).unwrap();

    // Now two apps come along, both depending on std @ ≥0.5.0 <1.0.0.
    let (app_a_meta, _) = read_khi(&build_app_pkg("app_a", 0x01)).unwrap();
    let (app_b_meta, _) = read_khi(&build_app_pkg("app_b", 0x02)).unwrap();
    let _ = store.install_pack(&build_app_pkg("app_a", 0x01)).unwrap();
    let _ = store.install_pack(&build_app_pkg("app_b", 0x02)).unwrap();

    // Resolver picks the same std pack for both apps.
    let mut resolver_a = Resolver::new(&store);
    let res_a = resolver_a.resolve(&app_a_meta.deps).unwrap();
    assert_eq!(res_a.len(), 1);
    assert_eq!(res_a[0].impl_hash, std_hash);

    let mut resolver_b = Resolver::new(&store);
    let res_b = resolver_b.resolve(&app_b_meta.deps).unwrap();
    assert_eq!(res_b.len(), 1);
    assert_eq!(res_b[0].impl_hash, std_hash);

    // Concrete signal of VISION §3.1: both apps share one std install.
    assert_eq!(res_a[0].impl_hash, res_b[0].impl_hash);
}
