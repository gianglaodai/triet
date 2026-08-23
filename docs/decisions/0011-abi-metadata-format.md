# ADR 0011 — ABI metadata format

**Status:** Decision. Applicable to v0.4 Crate-Pack format and all linkers/loaders reading cross-package interfaces from v0.4 onwards. This format is decoupled from the IR bytecode (ADR-0008) to allow the linker to reject mismatches before loading code.

**Issue:** v0.3 was limited to single-package scope. Each `.triv` file was a flat `IrProgram`—lacking crate-pack boundaries and the concept of "exposed interface." To enable distribution and cross-package linking in v0.4, a binary format is required to describe the **ABI surface** of a package:

- Function exports with full signatures (param types + return type).
- Type definitions (struct, enum) referenced by exported functions.
- Generic constraints and type parameter slots.
- Dependency declarations (package dependencies and version ranges).
- Capability claims (placeholder for v0.6).
- Version field for semver linking policy (ADR-0013).

VISION §3.3 mandates: *"The compiler is the gatekeeper: cross-package mismatch = refuse-to-link with clear diagnostics."* This requires the ABI metadata to be:

1. **Hash-stable** — same source ⇒ same bytes. Prerequisite for v0.5 CAS.
2. **Compact** — fast to read for the linker, without needing to decode the entire code section.
3. **Versioned** — backwards-compatible encoding (additive only after v1.0).
4. **Self-describing** — readable by external tools without the Triet source.

This ADR locks the binary format for ABI metadata and its relationship with `.triv` (ADR-0008) and `.khi` (ADR-0014, the container format).

## Decision

ABI metadata is an **independent binary section** within the `.khi` container, encoded using the same convention as `.triv` (little-endian, LEB128 varint, length-prefixed UTF-8). The linker reads **only this section** to decide whether to refuse or accept a link, without needing to load the IR code section.

### 1. Top-level layout

```
┌─────────────────────────────────────────────────────────────┐
│ ABI metadata section (in .tripack)                          │
├──────────────┬──────────────────────────────────────────────┤
│ abi_version  │ u32 LE — bumped when format changes (start = 1) │
│ pkg_name     │ length-prefixed UTF-8 — e.g. "std", "user.app"  │
│ pkg_version  │ semver triple (u32 major, u32 minor, u32 patch) │
│ iface_hash   │ 32 bytes — BLAKE3 of canonical ABI surface     │
│ impl_hash    │ 32 bytes — BLAKE3 of ABI + IR code (v0.5 prep) │
├──────────────┴──────────────────────────────────────────────┤
│ types        │ Type definition table (struct, enum, generic)  │
│ exports      │ Function export table (signature + capability) │
│ deps         │ Dependency declaration table                   │
│ caps         │ Capability claims (v0.6 placeholder, 0 entries)│
└─────────────────────────────────────────────────────────────┘
```

### 2. Type definition table

Each entry describes a user-defined type referenced by exports. Distinguishes between struct vs. enum vs. generic-shell:

```
type_count: varint
for i in 0..type_count:
    type_kind: u8  // 0 = struct, 1 = enum, 2 = generic-shell
    name: length-prefixed UTF-8
    type_param_count: varint
    for j in 0..type_param_count:
        param_name: length-prefixed UTF-8
        // future: constraint slots (v0.6 capability)
    body: encoded inline (kind-specific)
```

**Struct body**:
```
field_count: varint
for each field:
    field_name: length-prefixed UTF-8
    field_type: TypeRef
    visibility: u8 (0 = public, 1 = package, 2 = private)
```

**Enum body**:
```
variant_count: varint
for each variant:
    variant_name: length-prefixed UTF-8
    payload_type: Option<TypeRef> (1 byte flag + TypeRef if Some)
```

**TypeRef** (referenced types — primitive, defined, or type-param):
```
ref_kind: u8
  0x00 = primitive (next byte = TypeTag from ADR-0007)
  0x01 = local type (next varint = type table index)
  0x02 = type parameter (next varint = type_param index in current scope)
  0x03 = external type (next varint = dep table index, then varint = type index in that pkg)
  0x04 = nullable wrapper (next: inner TypeRef)
  0x05 = generic instantiation (next varint = base type idx, then count + sequence of TypeRef)
```

### 3. Function export table

```
export_count: varint
for each export:
    name: length-prefixed UTF-8
    visibility: u8 (only 0 = public exported, but slot reserved)
    type_param_count: varint
    for each type param: name (length-prefixed UTF-8)
    param_count: varint
    for each param:
        param_name: length/prefixed UTF-8
        param_type: TypeRef
    return_type: TypeRef
    capability_count: varint (placeholder, 0 in v0.4)
    for each capability: (reserved encoding)
    body_offset: varint
      // Offset into the .khi IR code section — the linker does not need to read this;
      // only runtime/JIT requires it for dispatch. 0 = abstract (no body, future).
```

### 4. Dependency table

```
dep_count: varint
for each dep:
    pkg_name: length-prefixed UTF-8
    version_min: semver triple (major, minor, patch)
    version_max_exclusive: semver triple  // 0,0,0 = open-ended
    iface_hash_pin: 32 bytes  // 0s = no hash pin (allow any matching version)
```

When `iface_hash_pin` is non-zero, the linker must match the exact hash—this is the mechanism for v0.5 CAS-pinning (preliminary, not enforced in v0.4).

### 5. Capability claims (v0.6 placeholder)

```
cap_count: varint  // always 0 in v0.4
// each entry will encode: namespace (sys/dev/usr), capability name,
// grant/deny trit. Format finalized in v0.6 ADR.
```

### 6. Canonical encoding rules (for hash stability)

To ensure `iface_hash` stability across re-compilation (required for v0.5 CAS):

- **Type table order**: sort lexicographically by `name`.
- **Export table order**: sort lexicographically by `name`.
- **Dep table order**: sort lexicographically by `pkg_name`.
- **Type param names**: preserve declaration order from source (positional).
- **No comments / no whitespace** — binary format, every byte is significant.
- **Variable encoding**: LEB128 does not include trailing zero padding.

`iface_hash` = BLAKE3 of the bytes from `pkg_name` to the end of the `caps` section. Excludes `abi_version`, `pkg_version`, and `impl_hash` (as those fields change with every commit even if the ABI surface remains unchanged).

`impl_hash` = BLAKE3 of (`iface_hash` bytes + IR code section bytes). If the implementation changes but the ABI does not, `iface_hash` remains constant, preventing downstream rebuilds.

### 7. ABI version policy

`abi_version = 1` at v0.4 launch. Bump when:
- Adding new fields without backwards compatibility (rare—use reserved space for additive fields).
- Changing the encoding of an existing field (avoid this).

Bumping requires a new ADR. A linker encountering `abi_version > supported` → refuses with error code E2301.

### 8. Relationship with `.triv` and `.khi`

| Format | Purpose | ADR |
|---|---|---|
| `.triv` | IR bytecode of a compilation unit | ADR-0008 |
| ABI metadata | Interface surface, version, hashes, deps | **ADR-0011 (this)** |
| `.khi` | Container: ABI metadata + N `.triv` units + manifest | ADR-0014 (TBD) |

Linker workflow:
1. Open `.khi` → read ABI metadata section first (cheap).
2. Resolve deps (read ABI sections of dependent `.khi` files).
3. Version check per ADR-0013 → refuse or accept.
4. Upon acceptance: load `.triv` code section into the VM, build cross-package symbol table.
5. At runtime/JIT: witness table dispatch (ADR-0012) for generic cross-pkg calls.

## Consequences

### For v0.5 (CAS)

- `iface_hash` is hash-stable → ready for use by the CAS resolver.
- Two-tier hashing (`iface_hash` (ABI) + `impl_hash` (full content)) is defined, avoiding future redesign.

### For v0.6 (Capability)

- Capability claims slot is reserved in the format. v0.6 only needs to populate it, without bumping `abi_version`.

### For linker performance

- Linker reads ~1-10 KB of metadata vs. a full ~100KB-1MB `.khi`. Enables cheap version checking before loading code.
- Refuse-to-link diagnostics can show metadata differences without needing to show IR.

### For generic ABI stability

- Generic type slots are encoded via index (varint) rather than monomorphized types → cross-pkg call sites maintain constant metadata bytes even when the caller changes instantiation.
- Witness table layout (ADR-0012) references type table indices → stable across recompiles.

## Alternatives Considered

- **No accompanying text format**: binary only. The `triet pack inspect` tool dumps human-readable output, but it is not the canonical format.
- **No per-export version field**: package-level versioning is sufficient. Granular versioning is avoided to prevent "Rust SemVer hell."
- **No capability runtime enforcement in v0.4**: only a reserved slot. Refuse-to-link based on capability mismatch applies from v0.6 onwards.
- **No cross-arch ABI** (32-bit vs. 64-bit): IR is architecture-independent (per ADR-0007 §4); ABI metadata inherits this.
- **No custom hash scheme**: BLAKE3 — industry standard, patent-free, fast, 32-byte output.

## Prior art

- **Swift `.swiftmodule` + `.swiftinterface`** — primary example. Separates interface (text) and metadata (binary). Triet uses pure binary for hash stability.
- **Mojo `.mojopkg`** — container format with metadata. Triet's design is similar but simpler (no Mojo-specific tracing).
- **.NET assembly metadata tables** — same concept, but with more entry kinds. Triet implements a minimal subset.
- **Java `.class` files** — not a good prior art; Java mixes bytecode and ABI into a single format, making separation of concerns difficult.

## References

- [VISION §3.3 — Stable ABI: Interface-forst Design](../../VISION.md)
- [SPEC §10 — Memory model + ABI hooks (TBD in v0.4)](../../SPEC.md)
- [ADR-0007 — IR design](0007-ir-design.md)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md)
- [ADR-0012 — Witness table dispatch](0012-witness-table-dispatch.md) (companion)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (companion)
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs)
