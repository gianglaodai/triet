//! Binary serializer/deserializer for `.khi` files.
//!
//! Encoding rules follow [ADR-0011 §6] (canonical encoding) and
//! [ADR-0014] (3-cấp hash tree + format bump `abi_version` 1 → 2):
//! little-endian multi-byte integers, LEB128 varints for size + index
//! fields, length-prefixed UTF-8 for strings. Sort orders match the
//! canonical-encoding rules so every hash level stays stable across
//! re-builds.
//!
//! [ADR-0011 §6]: ../../../docs/decisions/0011-abi-metadata-format.md
//! [ADR-0014]: ../../../docs/decisions/0014-hash-scheme-refinement.md

use std::collections::BTreeMap;

use crate::error::{PackError, PackResult};
use crate::hash::{
    IFACE_HASH_LEN, IMPL_HASH_LEN, IfaceHash, ImplHash, ModuleIfaceHash, ModuleImplHash,
    TermIfaceHash, TermImplHash, compute_module_iface_hash, compute_module_impl_hash,
    compute_term_iface_hash, compute_term_impl_hash,
};
use crate::types::{
    AbiMetadata, CapabilityClaim, CapabilityLevel, Dep, EnumDefinition, EnumVariant, FieldDef,
    FunctionExport, Module, Param, SemVer, StructDefinition, TypeDef, TypeKind, TypeRef,
    Visibility,
};

// ── Constants ──────────────────────────────────────────────────────

/// Magic bytes "trip" (ASCII) — distinguishes `.khi` from `.triv`.
const MAGIC: [u8; 4] = [0x74, 0x72, 0x69, 0x70];

/// Top-level pack format version (separate from `abi_version` inside
/// the metadata block). Bump only when the container framing changes.
const PACK_VERSION: u32 = 1;

/// ABI metadata format version. v0.4 shipped 1; ADR-0014 §5 bumps to 2
/// for the term + module hash fields and the modules table. Readers
/// refuse anything else (no shim — ADR-0014 explicit "refuse over guess").
const ABI_VERSION: u32 = 2;

/// Term-kind discriminants used inside canonical term signature bytes.
/// Functions and type kinds share a single byte namespace so hashes
/// stay distinct even when a function and a type happen to share a name.
mod term_kind {
    /// Function-shaped term — sigs as in `FunctionExport`.
    pub(super) const FUNCTION: u8 = 0;
    /// Struct type — body is `StructDefinition`.
    pub(super) const STRUCT: u8 = 1;
    /// Enum type — body is `EnumDefinition`.
    pub(super) const ENUM: u8 = 2;
    /// Generic shell — no body.
    pub(super) const GENERIC_SHELL: u8 = 3;
}

/// Section IDs inside a `.khi`. Section IDs unknown to a reader
/// MUST be skipped (forward-compat per ADR-0011). The constants for
/// not-yet-emitted sections are intentionally retained so future
/// sub-tasks (v0.5 manifest, v0.6 capabilities) can plug in without
/// reshuffling IDs.
mod section {
    pub(super) const ABI_METADATA: u8 = 1;
    pub(super) const IR_CODE: u8 = 2;
    /// ADR-0012 witness tables — populated by v0.4.6.
    #[allow(dead_code)]
    pub(super) const WITNESS_TABLES: u8 = 3;
    /// Cross-package manifest — populated by v0.4.5 linker.
    #[allow(dead_code)]
    pub(super) const MANIFEST: u8 = 4;
}

// ── Public API ─────────────────────────────────────────────────────

/// Serialize a `.khi` file from its ABI metadata + IR code bytes.
///
/// `code_section` is the canonical bytes of the IR section (an entire
/// `.triv` payload, or just the code body — caller decides format).
/// The writer:
///
/// 1. Sorts tables canonically (ADR-0011 §6).
/// 2. Computes `iface_hash_term` + `impl_hash_term` for every type +
///    export (ADR-0014 §2). v0.5.3 passes empty body bytes — v0.5.4
///    wires real per-term IR via `.triv` v4.
/// 3. Groups terms by `module_path` and computes `iface_hash_mod` +
///    `impl_hash_mod` (ADR-0014 §3), populating `meta.modules`.
/// 4. Computes pkg `iface_hash` from the module rollup (ADR-0014 §4)
///    and `impl_hash` over `iface_hash ‖ code_section` (v0.5.3 carry-
///    over from v0.4; switches to module rollup at v0.5.4).
#[must_use]
pub fn write_khi(meta: &AbiMetadata, code_section: &[u8]) -> Vec<u8> {
    let canon = canonicalize_for_hash(meta);
    let iface = crate::hash::compute_iface_hash(&canon);
    let impl_h = crate::hash::compute_impl_hash(&iface, code_section);

    let mut header = canon;
    header.iface_hash = iface;
    header.impl_hash = impl_h;

    let mut buf = Vec::with_capacity(1024 + code_section.len());
    buf.extend_from_slice(&MAGIC);
    write_u32_le(&mut buf, PACK_VERSION);

    // Section count: 2 (ABI metadata + IR code). Manifest + witness
    // tables are reserved for later sub-tasks.
    write_u32_le(&mut buf, 2);

    write_section(&mut buf, section::ABI_METADATA, |out| {
        write_abi_metadata(out, &header);
    });
    write_section(&mut buf, section::IR_CODE, |out| {
        out.extend_from_slice(code_section);
    });

    buf
}

/// Parse a `.khi` file into `(metadata, code_section_bytes)`.
///
/// # Errors
///
/// Returns [`PackError::BadMagic`] for non-`.khi` input,
/// [`PackError::UnsupportedAbiVersion`] when the file's `abi_version`
/// differs from this reader's (v0.5 = `2` — strict, no shim for v=1
/// per ADR-0014 §5), and [`PackError::Corrupted`] /
/// [`PackError::UnknownDiscriminant`] for structural problems found
/// while decoding.
pub fn read_khi(data: &[u8]) -> PackResult<(AbiMetadata, Vec<u8>)> {
    let mut pos = 0usize;
    if data.len() < 4 || data[..4] != MAGIC {
        return Err(PackError::BadMagic);
    }
    pos += 4;

    let pack_version = read_u32_le(data, &mut pos)?;
    if pack_version > PACK_VERSION {
        return Err(PackError::UnsupportedAbiVersion {
            found: pack_version,
            supported: PACK_VERSION,
        });
    }

    let section_count = read_u32_le(data, &mut pos)?;
    let mut meta: Option<AbiMetadata> = None;
    let mut code: Option<Vec<u8>> = None;

    for _ in 0..section_count {
        let section_id = read_u8(data, &mut pos)?;
        let size = read_u32_le(data, &mut pos)? as usize;
        let end = pos
            .checked_add(size)
            .ok_or_else(|| PackError::Corrupted("section size overflows".into()))?;
        if end > data.len() {
            return Err(PackError::Corrupted(format!(
                "section {section_id} length {size} runs past end of file"
            )));
        }
        let payload = &data[pos..end];
        match section_id {
            section::ABI_METADATA => {
                meta = Some(read_abi_metadata(payload)?);
            }
            section::IR_CODE => {
                code = Some(payload.to_vec());
            }
            // Unknown / future section: skip per ADR-0011.
            _ => {}
        }
        pos = end;
    }

    let meta = meta.ok_or_else(|| PackError::Corrupted("missing ABI metadata section".into()))?;
    let code = code.unwrap_or_default();
    Ok((meta, code))
}

// ── Canonicalization + hash pass ───────────────────────────────────

/// Per-module accumulator used by [`canonicalize_for_hash`] when
/// grouping terms before computing module rollups. Lives in a named
/// struct so the BTreeMap value type stays readable.
#[derive(Default)]
struct ModuleTerms {
    iface: Vec<(String, TermIfaceHash)>,
    impls: Vec<(String, TermImplHash)>,
}

/// Produce a canonicalized clone of `meta` with all hash levels
/// populated:
///
/// - tables sorted by name (ADR-0011 §6),
/// - term hashes computed from canonical signature bytes (ADR-0014 §2),
/// - module hashes rolled up by `module_path` (ADR-0014 §3),
/// - `meta.modules` rebuilt from scratch (caller's old list discarded
///   so stale entries can't leak into the hash).
///
/// Pkg-level `iface_hash` / `impl_hash` fields are left zero here —
/// they're filled in by `write_khi` from `compute_iface_hash` and
/// `compute_impl_hash` after this function returns.
pub(crate) fn canonicalize_for_hash(meta: &AbiMetadata) -> AbiMetadata {
    let mut out = meta.clone();

    out.types.sort_by(|a, b| a.name.cmp(&b.name));
    out.exports.sort_by(|a, b| a.name.cmp(&b.name));
    out.deps.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));

    // Compute term hashes in place. v0.5.3 uses empty body bytes for
    // impl — v0.5.4 wires real bodies once `.triv` v4 is in.
    for t in &mut out.types {
        let sig = canonical_term_signature_type(t);
        t.iface_hash_term = compute_term_iface_hash(&sig);
        t.impl_hash_term = compute_term_impl_hash(t.iface_hash_term, &[]);
    }
    for e in &mut out.exports {
        let sig = canonical_term_signature_function(e);
        e.iface_hash_term = compute_term_iface_hash(&sig);
        e.impl_hash_term = compute_term_impl_hash(e.iface_hash_term, &[]);
    }

    // Group by module_path → fresh modules table. BTreeMap gives
    // sorted iteration so we don't need to re-sort afterwards.
    let mut by_module: BTreeMap<String, ModuleTerms> = BTreeMap::new();
    for t in &out.types {
        let entry = by_module.entry(t.module_path.clone()).or_default();
        entry.iface.push((t.name.clone(), t.iface_hash_term));
        entry.impls.push((t.name.clone(), t.impl_hash_term));
    }
    for e in &out.exports {
        let entry = by_module.entry(e.module_path.clone()).or_default();
        entry.iface.push((e.name.clone(), e.iface_hash_term));
        entry.impls.push((e.name.clone(), e.impl_hash_term));
    }

    out.modules = by_module
        .into_iter()
        .map(|(path, terms)| {
            let iface_hash_mod = compute_module_iface_hash(&path, &terms.iface);
            let impl_hash_mod = compute_module_impl_hash(iface_hash_mod, &terms.impls);
            Module {
                path,
                iface_hash_mod,
                impl_hash_mod,
            }
        })
        .collect();

    out.iface_hash = IfaceHash::default();
    out.impl_hash = ImplHash::default();
    out
}

/// Canonical signature bytes for a [`TypeDef`] — fed to
/// [`compute_term_iface_hash`]. `pub(crate)` so `store` can reuse the
/// same encoding when populating `term/<hash>/iface.bin`.
pub(crate) fn canonical_term_signature_type(t: &TypeDef) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    let kind = match t.kind {
        TypeKind::Struct => term_kind::STRUCT,
        TypeKind::Enum => term_kind::ENUM,
        TypeKind::GenericShell => term_kind::GENERIC_SHELL,
    };
    write_u8(&mut buf, kind);
    write_string(&mut buf, &t.name);
    write_varint(&mut buf, t.type_parameters.len() as u32);
    for p in &t.type_parameters {
        write_string(&mut buf, p);
    }
    match (t.kind, &t.struct_body, &t.enum_body) {
        (TypeKind::Struct, Some(s), _) => write_struct_def(&mut buf, s),
        (TypeKind::Enum, _, Some(e)) => write_enum_def(&mut buf, e),
        _ => {}
    }
    buf
}

/// Canonical signature bytes for a [`FunctionExport`] — excludes
/// `body_offset` and capability claims per ADR-0014 §2. `pub(crate)`
/// shared with `store` for `term/<hash>/iface.bin` writes.
pub(crate) fn canonical_term_signature_function(f: &FunctionExport) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    write_u8(&mut buf, term_kind::FUNCTION);
    write_string(&mut buf, &f.name);
    write_visibility(&mut buf, f.visibility);
    write_varint(&mut buf, f.type_parameters.len() as u32);
    for p in &f.type_parameters {
        write_string(&mut buf, p);
    }
    write_varint(&mut buf, f.parameters.len() as u32);
    for p in &f.parameters {
        write_string(&mut buf, &p.name);
        write_type_ref(&mut buf, &p.type_ref);
    }
    write_type_ref(&mut buf, &f.return_type);
    buf
}

/// Encode the dependency table in canonical form for hashing.
/// `compute_iface_hash` calls this — kept here so the encoding rule
/// lives next to `write_dep_table`.
pub(crate) fn encode_deps_for_hash(deps: &[Dep]) -> Vec<u8> {
    let mut sorted: Vec<&Dep> = deps.iter().collect();
    sorted.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));
    let mut buf = Vec::with_capacity(64);
    write_varint(&mut buf, sorted.len() as u32);
    for d in sorted {
        write_string(&mut buf, &d.pkg_name);
        write_semver(&mut buf, d.version_min);
        write_semver(&mut buf, d.version_max_exclusive);
        buf.extend_from_slice(&d.iface_hash_pin.0);
    }
    buf
}

/// Encode the capability claims table in canonical form for hashing.
/// Canonical rule (ADR-0016 §4): sort by `cap_path` lexicographically;
/// each entry serializes as `(path: length-prefixed UTF-8, level: u8,
/// reserved: u8 = 0x00)`. Reused by both the iface-hash rollup and
/// the on-disk caps section (`write_caps_table`).
pub(crate) fn encode_caps_for_hash(caps: &[CapabilityClaim]) -> Vec<u8> {
    let mut sorted: Vec<&CapabilityClaim> = caps.iter().collect();
    sorted.sort_by(|a, b| a.cap_path.cmp(&b.cap_path));
    let mut buf = Vec::with_capacity(8);
    write_varint(&mut buf, sorted.len() as u32);
    for c in sorted {
        write_string(&mut buf, &c.cap_path);
        write_u8(&mut buf, c.level.as_byte());
        write_u8(&mut buf, 0x00);
    }
    buf
}

// ── ABI metadata block ────────────────────────────────────────────

fn write_abi_metadata(buf: &mut Vec<u8>, meta: &AbiMetadata) {
    write_u32_le(buf, meta.abi_version);
    write_string(buf, &meta.pkg_name);
    write_semver(buf, meta.pkg_version);
    buf.extend_from_slice(&meta.iface_hash.0);
    buf.extend_from_slice(&meta.impl_hash.0);
    write_module_table(buf, &meta.modules);
    write_type_table(buf, &meta.types);
    write_export_table(buf, &meta.exports);
    write_dep_table(buf, &meta.deps);
    write_caps_table(buf, &meta.caps);
}

fn read_abi_metadata(data: &[u8]) -> PackResult<AbiMetadata> {
    let mut pos = 0usize;
    let abi_version = read_u32_le(data, &mut pos)?;
    if abi_version != ABI_VERSION {
        return Err(PackError::UnsupportedAbiVersion {
            found: abi_version,
            supported: ABI_VERSION,
        });
    }
    let pkg_name = read_string(data, &mut pos)?;
    let pkg_version = read_semver(data, &mut pos)?;
    let iface_hash = read_hash(data, &mut pos)?;
    let impl_hash_bytes = read_hash(data, &mut pos)?;
    let modules = read_module_table(data, &mut pos)?;
    let types = read_type_table(data, &mut pos)?;
    let exports = read_export_table(data, &mut pos)?;
    let deps = read_dep_table(data, &mut pos)?;
    let caps = read_caps_table(data, &mut pos)?;
    Ok(AbiMetadata {
        abi_version,
        pkg_name,
        pkg_version,
        iface_hash: IfaceHash(iface_hash),
        impl_hash: ImplHash(impl_hash_bytes),
        modules,
        types,
        exports,
        deps,
        caps,
    })
}

// ── Module table (ADR-0014 §5) ────────────────────────────────────

fn write_module_table(buf: &mut Vec<u8>, modules: &[Module]) {
    // Canonical sort-at-boundary per ADR-0011 §6 + ADR-0019 §3:
    // on-disk byte order must match hash-input order so two runs over
    // the same logical input produce byte-identical `.khi`.
    let mut sorted: Vec<&Module> = modules.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    write_varint(buf, sorted.len() as u32);
    for m in sorted {
        write_string(buf, &m.path);
        buf.extend_from_slice(&m.iface_hash_mod.0);
        buf.extend_from_slice(&m.impl_hash_mod.0);
    }
}

fn read_module_table(data: &[u8], pos: &mut usize) -> PackResult<Vec<Module>> {
    let count = read_varint(data, pos)?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let path = read_string(data, pos)?;
        let iface = read_hash(data, pos)?;
        let impl_h = read_hash(data, pos)?;
        out.push(Module {
            path,
            iface_hash_mod: ModuleIfaceHash(iface),
            impl_hash_mod: ModuleImplHash(impl_h),
        });
    }
    Ok(out)
}

// ── Type table ────────────────────────────────────────────────────

fn write_type_table(buf: &mut Vec<u8>, types: &[TypeDef]) {
    // Canonical sort-at-boundary — see `write_module_table`.
    let mut sorted: Vec<&TypeDef> = types.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    write_varint(buf, sorted.len() as u32);
    for t in sorted {
        write_type_def(buf, t);
    }
}

fn read_type_table(data: &[u8], pos: &mut usize) -> PackResult<Vec<TypeDef>> {
    let count = read_varint(data, pos)?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(read_type_def(data, pos)?);
    }
    Ok(out)
}

fn write_type_def(buf: &mut Vec<u8>, t: &TypeDef) {
    let kind_byte = match t.kind {
        TypeKind::Struct => 0,
        TypeKind::Enum => 1,
        TypeKind::GenericShell => 2,
    };
    write_u8(buf, kind_byte);
    write_string(buf, &t.name);
    write_string(buf, &t.module_path);
    write_varint(buf, t.type_parameters.len() as u32);
    for p in &t.type_parameters {
        write_string(buf, p);
    }
    match (t.kind, &t.struct_body, &t.enum_body) {
        (TypeKind::Struct, Some(s), _) => write_struct_def(buf, s),
        (TypeKind::Enum, _, Some(e)) => write_enum_def(buf, e),
        (TypeKind::GenericShell, _, _) => {
            // No body. Future generic-shell encoding can land here
            // without bumping `abi_version` if we keep it length-
            // prefixed; for now writes nothing.
        }
        // Mismatched kind/body — write an empty body so the file
        // still round-trips. Caller should validate before writing.
        _ => {}
    }
    buf.extend_from_slice(&t.iface_hash_term.0);
    buf.extend_from_slice(&t.impl_hash_term.0);
}

fn read_type_def(data: &[u8], pos: &mut usize) -> PackResult<TypeDef> {
    let kind_byte = read_u8(data, pos)?;
    let kind = match kind_byte {
        0 => TypeKind::Struct,
        1 => TypeKind::Enum,
        2 => TypeKind::GenericShell,
        b => {
            return Err(PackError::UnknownDiscriminant {
                field: "TypeKind",
                discriminant: b,
            });
        }
    };
    let name = read_string(data, pos)?;
    let module_path = read_string(data, pos)?;
    let type_param_count = read_varint(data, pos)?;
    let mut type_parameters = Vec::with_capacity(type_param_count as usize);
    for _ in 0..type_param_count {
        type_parameters.push(read_string(data, pos)?);
    }
    let (struct_body, enum_body) = match kind {
        TypeKind::Struct => (Some(read_struct_def(data, pos)?), None),
        TypeKind::Enum => (None, Some(read_enum_def(data, pos)?)),
        TypeKind::GenericShell => (None, None),
    };
    let iface_hash_term = TermIfaceHash(read_hash(data, pos)?);
    let impl_hash_term = TermImplHash(read_hash(data, pos)?);
    Ok(TypeDef {
        kind,
        name,
        module_path,
        type_parameters,
        struct_body,
        enum_body,
        iface_hash_term,
        impl_hash_term,
    })
}

fn write_struct_def(buf: &mut Vec<u8>, s: &StructDefinition) {
    write_varint(buf, s.fields.len() as u32);
    for f in &s.fields {
        write_string(buf, &f.name);
        write_type_ref(buf, &f.type_ref);
        write_visibility(buf, f.visibility);
    }
}

fn read_struct_def(data: &[u8], pos: &mut usize) -> PackResult<StructDefinition> {
    let count = read_varint(data, pos)?;
    let mut fields = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = read_string(data, pos)?;
        let type_ref = read_type_ref(data, pos)?;
        let visibility = read_visibility(data, pos)?;
        fields.push(FieldDef {
            name,
            type_ref,
            visibility,
        });
    }
    Ok(StructDefinition { fields })
}

fn write_enum_def(buf: &mut Vec<u8>, e: &EnumDefinition) {
    write_varint(buf, e.variants.len() as u32);
    for v in &e.variants {
        write_string(buf, &v.name);
        match &v.payload {
            Some(t) => {
                write_u8(buf, 1);
                write_type_ref(buf, t);
            }
            None => write_u8(buf, 0),
        }
    }
}

fn read_enum_def(data: &[u8], pos: &mut usize) -> PackResult<EnumDefinition> {
    let count = read_varint(data, pos)?;
    let mut variants = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = read_string(data, pos)?;
        let tag = read_u8(data, pos)?;
        let payload = match tag {
            0 => None,
            1 => Some(read_type_ref(data, pos)?),
            b => {
                return Err(PackError::UnknownDiscriminant {
                    field: "EnumVariant.payload",
                    discriminant: b,
                });
            }
        };
        variants.push(EnumVariant { name, payload });
    }
    Ok(EnumDefinition { variants })
}

// ── TypeRef ────────────────────────────────────────────────────────

fn write_type_ref(buf: &mut Vec<u8>, t: &TypeRef) {
    match t {
        TypeRef::Primitive(tag) => {
            write_u8(buf, 0x00);
            write_u8(buf, *tag);
        }
        TypeRef::Local(idx) => {
            write_u8(buf, 0x01);
            write_varint(buf, *idx);
        }
        TypeRef::TypeParameter(idx) => {
            write_u8(buf, 0x02);
            write_varint(buf, *idx);
        }
        TypeRef::External { dep_idx, type_idx } => {
            write_u8(buf, 0x03);
            write_varint(buf, *dep_idx);
            write_varint(buf, *type_idx);
        }
        TypeRef::Nullable(inner) => {
            write_u8(buf, 0x04);
            write_type_ref(buf, inner);
        }
        TypeRef::Instantiation { base, args } => {
            write_u8(buf, 0x05);
            write_varint(buf, *base);
            write_varint(buf, args.len() as u32);
            for a in args {
                write_type_ref(buf, a);
            }
        }
    }
}

fn read_type_ref(data: &[u8], pos: &mut usize) -> PackResult<TypeRef> {
    let kind = read_u8(data, pos)?;
    Ok(match kind {
        0x00 => TypeRef::Primitive(read_u8(data, pos)?),
        0x01 => TypeRef::Local(read_varint(data, pos)?),
        0x02 => TypeRef::TypeParameter(read_varint(data, pos)?),
        0x03 => {
            let dep_idx = read_varint(data, pos)?;
            let type_idx = read_varint(data, pos)?;
            TypeRef::External { dep_idx, type_idx }
        }
        0x04 => TypeRef::Nullable(Box::new(read_type_ref(data, pos)?)),
        0x05 => {
            let base = read_varint(data, pos)?;
            let count = read_varint(data, pos)?;
            let mut args = Vec::with_capacity(count as usize);
            for _ in 0..count {
                args.push(read_type_ref(data, pos)?);
            }
            TypeRef::Instantiation { base, args }
        }
        b => {
            return Err(PackError::UnknownDiscriminant {
                field: "TypeRef",
                discriminant: b,
            });
        }
    })
}

// ── Visibility ─────────────────────────────────────────────────────

fn write_visibility(buf: &mut Vec<u8>, v: Visibility) {
    let b = match v {
        Visibility::Public => 0,
        Visibility::Package => 1,
        Visibility::Private => 2,
    };
    write_u8(buf, b);
}

fn read_visibility(data: &[u8], pos: &mut usize) -> PackResult<Visibility> {
    Ok(match read_u8(data, pos)? {
        0 => Visibility::Public,
        1 => Visibility::Package,
        2 => Visibility::Private,
        b => {
            return Err(PackError::UnknownDiscriminant {
                field: "Visibility",
                discriminant: b,
            });
        }
    })
}

// ── Export table ──────────────────────────────────────────────────

fn write_export_table(buf: &mut Vec<u8>, exports: &[FunctionExport]) {
    // Canonical sort-at-boundary — see `write_module_table`.
    let mut sorted: Vec<&FunctionExport> = exports.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    write_varint(buf, sorted.len() as u32);
    for e in sorted {
        write_string(buf, &e.name);
        write_string(buf, &e.module_path);
        write_visibility(buf, e.visibility);
        write_varint(buf, e.type_parameters.len() as u32);
        for p in &e.type_parameters {
            write_string(buf, p);
        }
        write_varint(buf, e.parameters.len() as u32);
        for p in &e.parameters {
            write_string(buf, &p.name);
            write_type_ref(buf, &p.type_ref);
        }
        write_type_ref(buf, &e.return_type);
        // capability count — reserved, always 0 at v0.5
        write_varint(buf, 0);
        write_varint(buf, e.body_offset);
        buf.extend_from_slice(&e.iface_hash_term.0);
        buf.extend_from_slice(&e.impl_hash_term.0);
    }
}

fn read_export_table(data: &[u8], pos: &mut usize) -> PackResult<Vec<FunctionExport>> {
    let count = read_varint(data, pos)?;
    let mut exports = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = read_string(data, pos)?;
        let module_path = read_string(data, pos)?;
        let visibility = read_visibility(data, pos)?;
        let tp_count = read_varint(data, pos)?;
        let mut type_parameters = Vec::with_capacity(tp_count as usize);
        for _ in 0..tp_count {
            type_parameters.push(read_string(data, pos)?);
        }
        let p_count = read_varint(data, pos)?;
        let mut parameters = Vec::with_capacity(p_count as usize);
        for _ in 0..p_count {
            let pname = read_string(data, pos)?;
            let ptype = read_type_ref(data, pos)?;
            parameters.push(Param {
                name: pname,
                type_ref: ptype,
            });
        }
        let return_type = read_type_ref(data, pos)?;
        let cap_count = read_varint(data, pos)?;
        if cap_count > 0 {
            // Per-export (function-level) capability slots remain
            // reserved. v0.6 populated the *package-level* caps
            // section (ADR-0016 §4); per-function granularity was
            // explicitly deferred post-v1.0 (ADR-0016 "Không làm").
            // A forward-compat `.khi` arriving with per-export
            // claims is treated as corruption until the future ADR
            // specifies the wire shape.
            return Err(PackError::Corrupted(
                "function export carries capability claims (per-function granularity \
                 deferred post-v1.0 per ADR-0016)"
                    .into(),
            ));
        }
        let body_offset = read_varint(data, pos)?;
        let iface_hash_term = TermIfaceHash(read_hash(data, pos)?);
        let impl_hash_term = TermImplHash(read_hash(data, pos)?);
        exports.push(FunctionExport {
            name,
            module_path,
            visibility,
            type_parameters,
            parameters,
            return_type,
            body_offset,
            iface_hash_term,
            impl_hash_term,
        });
    }
    Ok(exports)
}

// ── Dep table ─────────────────────────────────────────────────────

fn write_dep_table(buf: &mut Vec<u8>, deps: &[Dep]) {
    // Canonical sort-at-boundary — mirror `encode_deps_for_hash` so the
    // on-disk dep table matches the iface_hash input byte-for-byte.
    let mut sorted: Vec<&Dep> = deps.iter().collect();
    sorted.sort_by(|a, b| a.pkg_name.cmp(&b.pkg_name));
    write_varint(buf, sorted.len() as u32);
    for d in sorted {
        write_string(buf, &d.pkg_name);
        write_semver(buf, d.version_min);
        write_semver(buf, d.version_max_exclusive);
        buf.extend_from_slice(&d.iface_hash_pin.0);
    }
}

fn read_dep_table(data: &[u8], pos: &mut usize) -> PackResult<Vec<Dep>> {
    let count = read_varint(data, pos)?;
    let mut deps = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let pkg_name = read_string(data, pos)?;
        let version_min = read_semver(data, pos)?;
        let version_max_exclusive = read_semver(data, pos)?;
        let pin = read_hash(data, pos)?;
        deps.push(Dep {
            pkg_name,
            version_min,
            version_max_exclusive,
            iface_hash_pin: IfaceHash(pin),
        });
    }
    Ok(deps)
}

// ── Caps table (ADR-0016 §4 wire format, populated v0.6) ──────────

fn write_caps_table(buf: &mut Vec<u8>, caps: &[CapabilityClaim]) {
    // Reuse the canonical (sort + per-entry) encoding so the on-disk
    // bytes match what `encode_caps_for_hash` fed into `iface_hash`.
    buf.extend_from_slice(&encode_caps_for_hash(caps));
}

fn read_caps_table(data: &[u8], pos: &mut usize) -> PackResult<Vec<CapabilityClaim>> {
    let count = read_varint(data, pos)?;
    let mut caps = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let cap_path = read_string(data, pos)?;
        let level_byte = read_u8(data, pos)?;
        let level = CapabilityLevel::from_byte(level_byte).ok_or_else(|| {
            PackError::Corrupted(format!(
                "invalid capability level byte 0x{level_byte:02X} (expected 0x00..=0x03)"
            ))
        })?;
        let reserved = read_u8(data, pos)?;
        if reserved != 0x00 {
            return Err(PackError::Corrupted(format!(
                "capability entry reserved byte must be 0x00, got 0x{reserved:02X}"
            )));
        }
        caps.push(CapabilityClaim { cap_path, level });
    }
    Ok(caps)
}

// ── Section framing ───────────────────────────────────────────────

fn write_section(buf: &mut Vec<u8>, section_id: u8, body: impl FnOnce(&mut Vec<u8>)) {
    write_u8(buf, section_id);
    // Reserve 4 bytes for size; back-patch after writing the body.
    let size_pos = buf.len();
    write_u32_le(buf, 0);
    let body_start = buf.len();
    body(buf);
    let body_len = (buf.len() - body_start) as u32;
    buf[size_pos..size_pos + 4].copy_from_slice(&body_len.to_le_bytes());
}

// ── Primitive readers/writers (LEB128, LE, length-prefixed) ───────

fn write_u8(buf: &mut Vec<u8>, b: u8) {
    buf.push(b);
}

fn write_u32_le(buf: &mut Vec<u8>, n: u32) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn write_varint(buf: &mut Vec<u8>, mut n: u32) {
    while n >= 0x80 {
        buf.push((n as u8) | 0x80);
        n >>= 7;
    }
    buf.push(n as u8);
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_varint(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn write_semver(buf: &mut Vec<u8>, v: SemVer) {
    write_u32_le(buf, v.major);
    write_u32_le(buf, v.minor);
    write_u32_le(buf, v.patch);
}

fn read_u8(data: &[u8], pos: &mut usize) -> PackResult<u8> {
    let v = *data
        .get(*pos)
        .ok_or_else(|| PackError::Corrupted("unexpected EOF reading u8".into()))?;
    *pos += 1;
    Ok(v)
}

fn read_u32_le(data: &[u8], pos: &mut usize) -> PackResult<u32> {
    let end = pos
        .checked_add(4)
        .ok_or_else(|| PackError::Corrupted("position overflow reading u32".into()))?;
    if end > data.len() {
        return Err(PackError::Corrupted("unexpected EOF reading u32".into()));
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[*pos..end]);
    *pos = end;
    Ok(u32::from_le_bytes(bytes))
}

fn read_varint(data: &[u8], pos: &mut usize) -> PackResult<u32> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = read_u8(data, pos)?;
        // Cap shift at 28 — a u32 fits in 5 LEB128 bytes max.
        if shift > 28 {
            return Err(PackError::Corrupted("varint overflows u32".into()));
        }
        result |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
}

fn read_string(data: &[u8], pos: &mut usize) -> PackResult<String> {
    let len = read_varint(data, pos)? as usize;
    let end = pos
        .checked_add(len)
        .ok_or_else(|| PackError::Corrupted("string length overflow".into()))?;
    if end > data.len() {
        return Err(PackError::Corrupted("string runs past end".into()));
    }
    let s = std::str::from_utf8(&data[*pos..end])
        .map_err(|e| PackError::Corrupted(format!("invalid UTF-8: {e}")))?
        .to_owned();
    *pos = end;
    Ok(s)
}

fn read_semver(data: &[u8], pos: &mut usize) -> PackResult<SemVer> {
    let major = read_u32_le(data, pos)?;
    let minor = read_u32_le(data, pos)?;
    let patch = read_u32_le(data, pos)?;
    Ok(SemVer {
        major,
        minor,
        patch,
    })
}

fn read_hash(data: &[u8], pos: &mut usize) -> PackResult<[u8; IFACE_HASH_LEN]> {
    // IFACE_HASH_LEN == IMPL_HASH_LEN == 32. Sharing the reader is
    // safe because both consume exactly 32 bytes.
    debug_assert_eq!(IFACE_HASH_LEN, IMPL_HASH_LEN);
    let end = pos
        .checked_add(IFACE_HASH_LEN)
        .ok_or_else(|| PackError::Corrupted("position overflow reading hash".into()))?;
    if end > data.len() {
        return Err(PackError::Corrupted("hash runs past end".into()));
    }
    let mut out = [0u8; IFACE_HASH_LEN];
    out.copy_from_slice(&data[*pos..end]);
    *pos = end;
    Ok(out)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{IFACE_HASH_LEN, IfaceHash, TermImplHash};

    fn integer_primitive() -> TypeRef {
        // 0x02 = Integer per TypeTag (matches the IR crate's tag byte).
        TypeRef::Primitive(0x02)
    }

    fn mk_export(name: &str) -> FunctionExport {
        FunctionExport {
            name: name.into(),
            module_path: String::new(),
            visibility: Visibility::Public,
            type_parameters: Vec::new(),
            parameters: Vec::new(),
            return_type: integer_primitive(),
            body_offset: 0,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        }
    }

    /// Empty package round-trips cleanly.
    #[test]
    fn empty_pack_round_trip() {
        let meta = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        let bytes = write_khi(&meta, &[]);
        let (decoded, code) = read_khi(&bytes).unwrap();
        assert_eq!(decoded.abi_version, 2);
        assert_eq!(decoded.pkg_name, "foo");
        assert_eq!(decoded.pkg_version, SemVer::new(1, 0, 0));
        assert!(code.is_empty());
        // iface_hash is non-zero (something was hashed).
        assert!(!decoded.iface_hash.is_zero());
        // Empty package has zero modules (no terms).
        assert!(decoded.modules.is_empty());
    }

    /// Package with a struct + an export round-trips byte-identical
    /// and populates a module entry for the (empty path) root module.
    #[test]
    fn struct_and_export_round_trip() {
        let mut meta = AbiMetadata::empty("math", SemVer::new(1, 2, 3));
        meta.types.push(TypeDef {
            kind: TypeKind::Struct,
            name: "Vec2".into(),
            module_path: String::new(),
            type_parameters: Vec::new(),
            struct_body: Some(StructDefinition {
                fields: vec![
                    FieldDef {
                        name: "x".into(),
                        type_ref: integer_primitive(),
                        visibility: Visibility::Public,
                    },
                    FieldDef {
                        name: "y".into(),
                        type_ref: integer_primitive(),
                        visibility: Visibility::Public,
                    },
                ],
            }),
            enum_body: None,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
        meta.exports.push(FunctionExport {
            name: "dot".into(),
            module_path: String::new(),
            visibility: Visibility::Public,
            type_parameters: Vec::new(),
            parameters: vec![
                Param {
                    name: "a".into(),
                    type_ref: TypeRef::Local(0),
                },
                Param {
                    name: "b".into(),
                    type_ref: TypeRef::Local(0),
                },
            ],
            return_type: integer_primitive(),
            body_offset: 0,
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
        let bytes = write_khi(&meta, &[0xDE, 0xAD, 0xBE, 0xEF]);
        let (decoded, code) = read_khi(&bytes).unwrap();
        assert_eq!(decoded.types.len(), 1);
        assert_eq!(decoded.types[0].name, "Vec2");
        // Term iface hash populated by canonical pass.
        assert!(!decoded.types[0].iface_hash_term.is_zero());
        assert_eq!(decoded.exports.len(), 1);
        assert_eq!(decoded.exports[0].name, "dot");
        assert!(!decoded.exports[0].iface_hash_term.is_zero());
        // Vec2 + dot share the empty `module_path` → one module entry.
        assert_eq!(decoded.modules.len(), 1);
        assert_eq!(decoded.modules[0].path, "");
        assert!(!decoded.modules[0].iface_hash_mod.is_zero());
        assert_eq!(code, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    /// Two terms in different modules produce two module entries.
    #[test]
    fn modules_grouped_by_module_path() {
        let mut meta = AbiMetadata::empty("app", SemVer::new(0, 1, 0));
        let mut a = mk_export("first");
        a.module_path = "app.core".into();
        let mut b = mk_export("second");
        b.module_path = "app.util".into();
        meta.exports.push(a);
        meta.exports.push(b);
        let bytes = write_khi(&meta, &[]);
        let (decoded, _) = read_khi(&bytes).unwrap();
        assert_eq!(decoded.modules.len(), 2);
        let paths: Vec<&str> = decoded.modules.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["app.core", "app.util"]);
    }

    /// Generic enum (`Option<T>`) round-trips with the type param
    /// slot + payload type-param reference.
    #[test]
    fn generic_enum_round_trip() {
        let mut meta = AbiMetadata::empty("std", SemVer::new(0, 5, 0));
        meta.types.push(TypeDef {
            kind: TypeKind::Enum,
            name: "Option".into(),
            module_path: "std.option".into(),
            type_parameters: vec!["T".into()],
            struct_body: None,
            enum_body: Some(EnumDefinition {
                variants: vec![
                    EnumVariant {
                        name: "Some".into(),
                        payload: Some(TypeRef::TypeParameter(0)),
                    },
                    EnumVariant {
                        name: "None".into(),
                        payload: None,
                    },
                ],
            }),
            iface_hash_term: TermIfaceHash::default(),
            impl_hash_term: TermImplHash::default(),
        });
        let bytes = write_khi(&meta, &[]);
        let (decoded, _) = read_khi(&bytes).unwrap();
        assert_eq!(decoded.types.len(), 1);
        let opt = &decoded.types[0];
        assert_eq!(opt.name, "Option");
        assert_eq!(opt.module_path, "std.option");
        assert_eq!(opt.type_parameters, vec!["T"]);
        let body = opt.enum_body.as_ref().unwrap();
        assert_eq!(body.variants.len(), 2);
        assert!(matches!(
            body.variants[0].payload,
            Some(TypeRef::TypeParameter(0))
        ));
    }

    /// Dependency table with hash pin survives the round-trip.
    #[test]
    fn dep_with_hash_pin_round_trip() {
        let mut meta = AbiMetadata::empty("app", SemVer::new(1, 0, 0));
        let pin_bytes = [0x42u8; IFACE_HASH_LEN];
        meta.deps.push(Dep {
            pkg_name: "math".into(),
            version_min: SemVer::new(1, 0, 0),
            version_max_exclusive: SemVer::new(2, 0, 0),
            iface_hash_pin: IfaceHash::from_bytes(pin_bytes),
        });
        let bytes = write_khi(&meta, &[]);
        let (decoded, _) = read_khi(&bytes).unwrap();
        assert_eq!(decoded.deps.len(), 1);
        assert_eq!(decoded.deps[0].pkg_name, "math");
        assert_eq!(decoded.deps[0].iface_hash_pin.0, pin_bytes);
        assert!(!decoded.deps[0].iface_hash_pin.is_zero());
    }

    /// Re-ordering tables on input should not change `iface_hash` —
    /// canonicalization guarantees stability across the 3-cấp tree.
    #[test]
    fn iface_hash_is_order_independent() {
        let mut a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        a.exports.push(mk_export("alpha"));
        a.exports.push(mk_export("beta"));
        let mut b = a.clone();
        b.exports.reverse();
        let bytes_a = write_khi(&a, &[]);
        let bytes_b = write_khi(&b, &[]);
        let (da, _) = read_khi(&bytes_a).unwrap();
        let (db, _) = read_khi(&bytes_b).unwrap();
        assert_eq!(da.iface_hash, db.iface_hash);
    }

    /// Renaming an export changes its term hash, the module hash, and
    /// the package iface hash — full propagation through the tree.
    #[test]
    fn renaming_export_propagates_up_the_tree() {
        let mut a = AbiMetadata::empty("foo", SemVer::new(1, 0, 0));
        a.exports.push(mk_export("alpha"));
        let mut b = a.clone();
        b.exports[0].name = "renamed".into();

        let (da, _) = read_khi(&write_khi(&a, &[])).unwrap();
        let (db, _) = read_khi(&write_khi(&b, &[])).unwrap();

        assert_ne!(da.exports[0].iface_hash_term, db.exports[0].iface_hash_term);
        assert_ne!(da.modules[0].iface_hash_mod, db.modules[0].iface_hash_mod);
        assert_ne!(da.iface_hash, db.iface_hash);
    }

    /// Bad magic bytes fail fast with `PackError::BadMagic`.
    #[test]
    fn bad_magic_rejected() {
        let bytes = vec![0u8; 64];
        let err = read_khi(&bytes).unwrap_err();
        assert_eq!(err, PackError::BadMagic);
    }

    /// Truncated file (less than the magic length) also fails with
    /// `BadMagic` — first sanity gate.
    #[test]
    fn truncated_input_rejected() {
        let bytes = vec![0x74, 0x72]; // only 2 bytes
        assert_eq!(read_khi(&bytes).unwrap_err(), PackError::BadMagic);
    }

    /// Unsupported pack format version produces the dedicated error
    /// rather than a generic corruption diagnostic.
    #[test]
    fn future_pack_version_rejected() {
        // Hand-craft a header: magic + future-version + 0 sections.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&999u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = read_khi(&bytes).unwrap_err();
        assert!(matches!(err, PackError::UnsupportedAbiVersion { .. }));
    }

    /// v0.4 `abi_version = 1` packs are explicitly refused per
    /// ADR-0014 §5 — no shim. Migration is via `dao store import`
    /// (v0.5.7) which handles the lossy upgrade.
    #[test]
    fn legacy_abi_version_1_rejected() {
        // Hand-craft a minimal ABI section claiming abi_version = 1.
        let mut payload = Vec::new();
        write_u32_le(&mut payload, 1); // abi_version
        write_string(&mut payload, "legacy"); // pkg_name
        write_semver(&mut payload, SemVer::new(1, 0, 0));
        payload.extend_from_slice(&[0u8; IFACE_HASH_LEN]);
        payload.extend_from_slice(&[0u8; IMPL_HASH_LEN]);
        let err = read_abi_metadata(&payload).unwrap_err();
        assert!(matches!(
            err,
            PackError::UnsupportedAbiVersion {
                found: 1,
                supported: 2,
            }
        ));
    }

    // ── Capability claims (ADR-0016 §4 + ADR-0018 §6, v0.6.4) ────

    fn cap(path: &str, level: CapabilityLevel) -> CapabilityClaim {
        CapabilityClaim {
            cap_path: path.into(),
            level,
        }
    }

    #[test]
    fn caps_roundtrip_non_empty() {
        // Mix of all four levels, deliberately unsorted on the way in
        // to prove the writer normalizes.
        let caps = vec![
            cap("sys.net.dns", CapabilityLevel::Defer),
            cap("dev.disk", CapabilityLevel::Deny),
            cap("sys.io", CapabilityLevel::Grant),
            cap("usr.somelib", CapabilityLevel::Ambient),
        ];
        let mut buf = Vec::new();
        write_caps_table(&mut buf, &caps);
        let mut pos = 0;
        let decoded = read_caps_table(&buf, &mut pos).expect("round-trip ok");
        assert_eq!(pos, buf.len(), "consumed exactly the bytes written");

        // Output must arrive sorted by cap_path (canonical per ADR-0016 §4).
        let expected = vec![
            cap("dev.disk", CapabilityLevel::Deny),
            cap("sys.io", CapabilityLevel::Grant),
            cap("sys.net.dns", CapabilityLevel::Defer),
            cap("usr.somelib", CapabilityLevel::Ambient),
        ];
        assert_eq!(decoded, expected);
    }

    #[test]
    fn caps_empty_roundtrips_to_one_zero_byte() {
        // Empty caps section = single `cap_count = 0` varint. Preserves
        // hash stability for pre-v0.6 packs (ADR-0016 §4 promise).
        let mut buf = Vec::new();
        write_caps_table(&mut buf, &[]);
        assert_eq!(buf, vec![0x00]);
        let mut pos = 0;
        let decoded = read_caps_table(&buf, &mut pos).expect("round-trip ok");
        assert_eq!(pos, 1);
        assert!(decoded.is_empty());
    }

    #[test]
    fn caps_hash_encoding_is_order_independent() {
        // Sort canonical means the hash input must be identical regardless
        // of how the caller orders the input vector.
        let a = vec![
            cap("sys.io", CapabilityLevel::Grant),
            cap("dev.disk", CapabilityLevel::Deny),
        ];
        let b = vec![
            cap("dev.disk", CapabilityLevel::Deny),
            cap("sys.io", CapabilityLevel::Grant),
        ];
        assert_eq!(encode_caps_for_hash(&a), encode_caps_for_hash(&b));
    }

    #[test]
    fn caps_reject_invalid_level_byte() {
        // Hand-craft caps section with level=0x04 (outside 0x00..=0x03).
        let mut buf = Vec::new();
        write_varint(&mut buf, 1); // cap_count
        write_string(&mut buf, "sys.io");
        write_u8(&mut buf, 0x04); // invalid level
        write_u8(&mut buf, 0x00); // reserved
        let mut pos = 0;
        let err = read_caps_table(&buf, &mut pos).expect_err("must reject");
        match err {
            PackError::Corrupted(msg) => {
                assert!(
                    msg.contains("invalid capability level"),
                    "unexpected message: {msg}"
                );
                assert!(msg.contains("0x04"), "must surface the bad byte: {msg}");
            }
            other => panic!("expected Corrupted, got {other:?}"),
        }
    }

    #[test]
    fn caps_reject_non_zero_reserved_byte() {
        // Reserved must stay 0x00 at v0.6 (ADR-0016 §4) — guard the slot
        // against accidental forward-compat writes.
        let mut buf = Vec::new();
        write_varint(&mut buf, 1);
        write_string(&mut buf, "sys.io");
        write_u8(&mut buf, CapabilityLevel::Grant.as_byte());
        write_u8(&mut buf, 0xAB); // non-zero reserved
        let mut pos = 0;
        let err = read_caps_table(&buf, &mut pos).expect_err("must reject");
        match err {
            PackError::Corrupted(msg) => {
                assert!(msg.contains("reserved"), "unexpected message: {msg}");
            }
            other => panic!("expected Corrupted, got {other:?}"),
        }
    }

    #[test]
    fn caps_populated_does_not_bump_abi_version() {
        // ADR-0016 §4 promise: populating the caps slot reuses the
        // v=2 layout (no bump). Round-trip an AbiMetadata with caps and
        // assert the wire still says 2.
        let mut meta = AbiMetadata::empty("withcaps", SemVer::new(0, 1, 0));
        meta.caps = vec![cap("sys.io", CapabilityLevel::Grant)];
        let mut buf = Vec::new();
        write_abi_metadata(&mut buf, &meta);
        let decoded = read_abi_metadata(&buf).expect("decode ok");
        assert_eq!(decoded.abi_version, 2);
        assert_eq!(decoded.caps, meta.caps);
    }
}
