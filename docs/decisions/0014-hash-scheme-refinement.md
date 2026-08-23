# ADR 0014 — Hash scheme refinement (3-level hash tree)

**Status:** Decided. Applicable to v0.5 CAS Packaging and all tools reading `.khi` hash structures from v0.5 onwards. Extends [ADR-0011 §6](0011-abi-metadata-format.md) (canonical encoding) and the ABI metadata section table layout; **does not break** the invariants of [ADR-0013](0013-semver-linking-policy.md) (iface_hash remains the final arbiter).

**Issue:** v0.4 introduced a 2-level hash scheme **per-package**: `iface_hash` (ABI surface) + `impl_hash` (ABI + IR code). This is sufficient for cross-package linking (refuse/accept), but **insufficient** for the [VISION §3.1](../../VISION.md) promise:

> *"10 applications using `String.format` only load one instance into RAM."*

Pack-level hash $\neq$ function-level identity. Two different `.khi` files containing byte-identical `std.text.format` $\rightarrow$ two different `impl_hash` values $\rightarrow$ CAS store loads two instances. VISION §3.1 requires deduplication at the term level, not the package level.

Four questions this ADR must resolve before writing the CAS store (ADR-0015):

1. **Namespace** — what does the hash address? package? module? function?
2. **Granularity** — at what level is sharing/deduplication occurring?
3. **Normalization** — are canonical form rules strict enough to ensure determinism across re-compilation?
4. **Content-vs-interface separation** — how do `iface_hash`/`impl_hash` extend to lower levels?

The tension with [ADR-0006](0006-ternary-packaging-vision.md) §2 (Ternary Vector Versioning) is a **versioning** issue, not a hashing issue — deferred to a separate ADR after the basic CAS is shipped in v0.5.

## Decision

### 1. 3-level Hash tree

Addresses content at **exactly 3 levels**, mirroring the Triet identity `{-1, 0, +1}`:

```
┌─────────────────────────────────────────────────────────────┐
│  Level 3 — Package    iface_hash_pkg  +  impl_hash_pkg        │
│      (= current iface_hash / impl_hash from ADR-0011)        │
│      Rollup: BLAKE3(sorted module hashes + deps + caps)     │
├─────────────────────────────────────────────────────────────┤
│  Level 2 — Module     ifanc_hash_mod  +  impl_hash_mod        │
│      Rollup: BLAKE3(sorted term hashes within module)       │
├─────────────────────────────────────────────────────────────┤
│  Level 1 — Term       iface_hash_term + impl_hash_term      │
│      Per export: function, struct, enum, generic-shell      │
│      iface = BLAKE3(canonical signature bytes)              │
│      impl  = BLAKE3(iface_hash_term ‖ term IR body bytes)   │
└─────────────────────────────────────────────────────────────┘
```

**Why 3 levels, not N:** `{term, module, package}` is the natural triangle of the Triet module system (ADR-0005 locked the hierarchical namespace). Deeper hashing (AST nodes, as in pure Unison) incurs canonicalization costs that no one consumes. Shallower hashing (1-level pack-only) breaks VISION §3.1. 3 is the equilibrium — and each level corresponds to a state of a Trit when an LLM/AI addresses it: `Trit::Negative` = term (lowest level), `Trit::Zero` = module (intermediate), `Trit::Positive` = package (highest level).

### 2. Level 1 — Term hash

A "term" is an exported item at the module-system boundary (ADR-0005):
- function declaration
- struct declaration
- enum declaration
- generic-shell declaration

Term hash **does not** recurse into statements/expressions/AST-nodes. The boundary is limited to the public ABI surface — the same granularity tracked by ADR-0011 §2-§3.

**`iface_hash_term`** = `BLAKE3(domain_sep_term ‖ canonical_signature_bytes)`

Canonical signature bytes (deterministic, excludes debug/source location):
```
term_kind: u8              // 0=function, 1=struct, 2=enum, 3=generic-shell
name: length-prefixed UTF-8
visibility: u8             // 0=public, 1=package, 2=private
type_param_count: varint
  for each: param_name (length-prefixed UTF-8)  // positional, preserve source order
body: kind-specific encoding (per ADR-0011 §2 struct/enum body or §3 function signature)
```

**Exclusions:** `body_offset` (storage detail), capability claims (pkg-level — ADR-0011 §5), doc comments, span info.

**`impl_hash_term`** = `BLAKE3(domain_sep_term_impl ‖ iface_hash_term ‖ term_ir_body_bytes)`

`term_ir_body_bytes` = canonical bytes of the specific IR block for this term in the `.triv` code section. Requires format change: the code section must have a per-term offset index (see §5).

### 3. Level 2 — Module hash

A "module" is identified by a dotted path from ADR-0005 (`crate.foo.bar`, `std.text`, etc.). Both top-level inline modules and file-bound modules are included.

**`iface_hash_mod`** = `BLAKE3(domain_sep_mod_iface ‖ module_path_bytes ‖ sorted_term_iface_hashes)`

Sorted_term_iface_hashes = sequence of (`term_name_len: u32 LE`, `term_name_bytes`, `iface_hash_term: 32 bytes`) for each term in this module, sorted lexicographically by `term_name`.

**`impl_hash_mod`** = `BLAKE3(domain_sep_mod_impl ‖ iface_hash_mod ‖ sorted_term_impl_hashes)`

### 4. Level 3 — Package hash

Replaces ADR-0011 §6 hash inputs. **Same output shape (32 bytes)**, same field names (`iflag_hash`/`impl_hash`) — bytes change because the rollup formula changes.

**`iface_hash_pkg`** = `BLAKE3(`
- `domain_sep_pkg_iface ‖`
- `pkg_name (length-prefixed) ‖`
- `sorted module entries: (mod_path_len, mod_path_bytes, iface_hash_mod) ‖`
- `deps table bytes (canonical, per ADR-0011 §4) ‖`
- `caps table bytes (canonical, per ADR-0011 §5)`
- `)`

**`impl_hash_pkg`** = `BLAKE3(domain_sep_pkg_impl ‖ iface_hash_pkg ‖ sorted impl_hash_mod sequence)`

### 5. Encoding changes in `.khi` (abi_version bump 1 $\rightarrow$ 2)

Additive — v1 readers encountering `abi_version = 2` must refuse with E2301 (per ADR-0013 §3). No shim for partial v2 reading — Triet follows **refuse over guess**.

**Types table (ADR-0011 §2):** each type entry adds at the end:
```
iface_hash_term: 32 bytes
impl_hash_term:  32 bytes
```

**Exports table (ADR-0011 §3):** each export entry adds at the end:
```
iface_hash_term: 32 bytes
impl_hash_term:  32 bytes
```

**Modules table (new, section ID between exports and deps):**
```
mod_count: varint
for each:
    mod_path: length-prefixed UTF-8
    iface_hash_mod: 32 bytes
    impl_hash_mod:  32 bytes
```

**Code section (`.triv` reference from ADR-0008):** adds a **per-term offset index** before the instruction stream:
```
term_offset_count: varint
for each: term_name (length-prefixed UTF-8) + body_start: varint + body_len: varint
[instruction bytes — as before]
```

`.triv` wire format bump **v3 $\rightarrow$ v4** (v3 was bumped in ADR-0012 for WitnessCall). v3 readers encountering v4 files $\rightarrow$ E2301.

### 6. Domain separation

BLAKE3 is not susceptible to length-extension attacks like SHA-2, but domain separation is still required to prevent ambiguity when the same input bytes are hashed at different levels (e.g., a term name matching a module path).

The domain separator is a **16-byte ASCII prefix with NUL padding**:
```
b"triet/term-i  \0\0"   // iface_hash_term     (16 bytes)
b"triet/term-m  \0\0"   // impl_hash_term      (m = "mut/impl")
b"triet/mod-i   \0\0"   // iface_hash_mod
b"triet/mod-m   \0\0"   // impl_hash_mod
b"triet/pkg-i   \0\0"   // iface_hash_pkg
b"triet/pkg-m   \0\0"   // impl_hash_pkg
```

Fixed strings, locked in constants in `triet-pack/src/hash.rs`. Changing the separator = changing all hashes = bumping `abi_version`. No silent changes allowed.

### 7. Normalization rules (strengthen ADR-0011 §6)

- **Sort**: lexicographical by raw UTF-8 bytes of the name (not Unicode collation — implementation-independent).
- **Varint**: LEB128 minimal encoding (no trailing-zero padding). Decoder rejects non-minimal — strict mode.
- **Length-prefixed string**: `u32 LE length`, **no NUL terminator**, no BOM, no validation rerun (caller-provided UTF-8 is trusted).
- **TypeRef ordering** (ADR-0011 §2): `ref_kind` byte first, then payload — deterministic per discriminator value.
- **Type param order**: positional (source declaration order), no sorting.
- **Sub-table sort key**: name is primary; if names collide (cross-namespace shouldn't happen post-ADR-0005, but for defense): secondary key = full canonical path bytes.

Test invariant for `triet-pack`: round-trip an `AbiMetadata` $\rightarrow$ encode $\rightarrow$ hash $\rightarrow$ re-encode $\rightarrow$ hash $\rightarrow$ bytes $\equiv$, hash $\equiv$. Existing `iface_hash_ignores_pkg_version` test extends to 3 levels.

### 8. iface_hash is the final arbiter — unchanged

[ADR-0013 §4](0013-semver-linking-policy.md) locks the policy: "semver is declaration, hash is proof." ADR-0014 **does not** change this. The linker still checks `iface_hash_pkg` (level 3) — that is the arbiter. Level 1 + Level 2 are **enablers for deduplication**, not the linker contract.

**Consequence:** The linker does not reject a package when term-level hashes drift but the package-level matches. The author is responsible — if a term hash changes but the package hash does not, it is a rollup error (defensive test in `triet-pack`).

## Consequences

### For v0.5 CAS store (ADR-0015 to be written)

- Filesystem layout can address 3 levels:
  - `~/.triet/store/term/<hex(impl_hash_term)>/code.bin` — function-level dedup
  - `~/.triet/store/mod/<hex(impl_hash_mod)>/index.bin` — module-level metadata
  - `~/.triet/store/pkg/<hex(impl_hash_pkg)>/pack.khi` — package-level distribution unit
- VISION §3.1 goal achieved: `std.text.format` is shared via N apps using lookup-by-term-hash.

### For v0.6 Capability

- Caps table remains at the package level (ADR-0011 §5). Term-level capability annotations (if needed) will enter the term signature $\rightarrow$ already hashed by `iface_hash_term`. Does not break v0.5 invariants.

### For v0.7 Self-hosting

- Re-implement hash computation in Triet. ADR-0014 is the canonical spec. Cross-bootstrap diff: same `AbiMetadata` $\rightarrow$ same 3-tuple hashes via Rust impl vs. Triet impl.

### For `.triv` wire format

- Bump v3 $\rightarrow$ v4 (per-term offset index). Linker/VM v3 readers encountering v4 files $\rightarrow$ E2031. No lossy fallback.

### For linker performance

- Per-export hash adds 64 bytes of overhead per export in the ABI metadata. A typical package with ~50 exports $\approx$ ~3KB overhead. Negligible.

### For generic dispatch (ADR-0012 witness tables)

- Witness table references can now be pinned by `iface_hash_term` (per-function) instead of package-level. v0.5 does not exploit this yet — slot reserved.

## Rejected Alternatives

- **Do not hash AST nodes** (pure Unison). Triet hashes at the module-system boundary, not deeper. Term-of-term hashing adds canonicalization costs for every sub-expression, which no one consumes in v0.5 — over-engineering.
- **Do not implement Ternary Vector Versioning** ([ADR-0006](0006-ternary-packaging-vision.md) §2). Split into a separate ADR after the basic CAS is shipped in v0.5. Versioning is semantic intent; hashing is content identity — orthogonal concerns.
- **No per-term capability** in v0.5. Caps remain package-level. v0.6 ADR will revisit.
- **No network CAS** (`triet pull <hash>` style). Local store first; distributed registry is a v1.0+ topic.
- **No content-defined chunking** (FastCDC/rsync-style). The term boundary is already a natural chunk — no additional layer needed.
- **No cross-platform hash variance**. BLAKE3 is deterministic; ADR-0011 §6 has locked little-endian + architecture-independent IR.
- **No Merkle proof / inclusion proof API**. The v0.5 store assumes a trust-local-filesystem model; cryptographic proofs are unnecessary when there are no untrusted peers.

## Prior art

- **Unison** ([unison-lang.org](https://www.unison-lang.org/)) — main inspiration. Term-level hashing is the core idea. Triet differs: the hash boundary is at the module-system level (function/type), not at the AST node level. Trade-off: less deduplication than Unison, but simpler canonicalization + alignment with ADR-0005 module structure.
- **Git Merkle tree** — blob $\rightarrow$ tree $\rightarrow$ commit is a 3-level structure parallel to term $\rightarrow$ module $\rightarrow$ package. Git inspired the tree structure, not the content (Git hashes arbitrary blob bytes; Triet hashes canonical signatures).
- **Nix derivations** ([nixos.org](https://nixos.org/)) — package-only CAS. Triet extends this to the term level for RAM-sharing use cases that Nix does not cover.
- **IPFS / Merkle DAG** — general theory of content-addressed graphs. Triet is a specific instance (3-level, not arbitrary depth).
- **Bazel action cache** — input-hash $\rightarrow$ output-hash mapping. Triet is equivalent at the package level (impl_hash is the cache key), with two additional lower levels.
- **Anti-prior-art:** Java `.jar` (no hash identity, ClassNotFoundException hell); npm (semver-only resolution, content drift unobserved); Maven Central (SHA checksums only for download integrity, not for identity/deduplication).

## References

- [VISION §3.1 — CAS Packaging](../../VISION.md) (RAM-sharing promise)
- [VISION §3.3 — Stable ABI](../../VISION.md) (iface_hash is the arbiter)
- [ADR-0005 — Module system](0005-module-system.md) (defines term/module boundary)
- [ADR-0006 — Ternary packaging vision](0006-ternary-packaging-vision.md) (informational, ADR-0014 only implements the "CAS hash" part — Ternary Versioning is separate)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (per-term offset index = v3 $\rightarrow$ v4 bump)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (ADR-0014 extends §2, §3, §6)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (final arbiter rule remains)
- [ADR-0015 — Package store layout](0015-package-store-layout.md) (sibling, to be written after ADR-0014)
- [ROADMAP § v0.5](../../ROADMAP.md)
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs)
