# ADR 0014 — Hash scheme refinement (3-cấp hash tree)

**Trạng thái:** Quyết định. Áp dụng cho v0.5 CAS Packaging và mọi tool đọc cấu trúc hash trong `.tripack` kể từ v0.5. Extend [ADR-0011 §6](0011-abi-metadata-format.md) (canonical encoding) và section table layout của ABI metadata; **không phá** invariants của [ADR-0013](0013-semver-linking-policy.md) (iface_hash vẫn là final arbiter).

**Issue:** v0.4 land hash scheme 2 cấp **per-package**: `iface_hash` (ABI surface) + `impl_hash` (ABI + IR code). Đủ cho cross-package linking refuse/accept, nhưng **không đủ** cho lời hứa [VISION §3.1](../../VISION.md):

> *"10 ứng dụng dùng `String.format` chỉ load 1 bản vào RAM."*

Pack-level hash ≠ function-level identity. Hai `.tripack` khác nhau cùng chứa byte-identical `std.text.format` → 2 `impl_hash` khác nhau → CAS store load 2 bản. VISION §3.1 yêu cầu dedup ở mức term, không phải pack.

Bốn câu hỏi ADR phải khoá trước khi viết CAS store (ADR-0015):

1. **Namespace** — hash địa chỉ cái gì? package? module? function?
2. **Granularity** — sharing/dedup ở cấp nào?
3. **Normalization** — canonical form rules đủ chặt để determinism qua re-compile?
4. **Content-vs-interface separation** — `iface_hash`/`impl_hash` extend xuống cấp nhỏ thế nào?

Tension với [ADR-0006](0006-ternary-packaging-vision.md) §2 (Ternary Vector Versioning) là vấn đề **versioning**, không phải hashing — defer ra ADR riêng sau v0.5 ship CAS cơ bản.

## Quyết định

### 1. Hash tree 3 cấp

Triết addresses content ở **đúng 3 cấp**, mirror Trit identity `{-1, 0, +1}`:

```
┌─────────────────────────────────────────────────────────────┐
│  Cấp 3 — Package    iface_hash_pkg  +  impl_hash_pkg        │
│      (= iface_hash / impl_hash hiện tại từ ADR-0011)        │
│      Rollup: BLAKE3(sorted module hashes + deps + caps)     │
├─────────────────────────────────────────────────────────────┤
│  Cấp 2 — Module     iface_hash_mod  +  impl_hash_mod        │
│      Rollup: BLAKE3(sorted term hashes within module)       │
├─────────────────────────────────────────────────────────────┤
│  Cấp 1 — Term       iface_hash_term + impl_hash_term        │
│      Per export: function, struct, enum, generic-shell      │
│      iface = BLAKE3(canonical signature bytes)              │
│      impl  = BLAKE3(iface_hash_term ‖ term IR body bytes)   │
└─────────────────────────────────────────────────────────────┘
```

**Tại sao 3 cấp, không N:** `{term, module, package}` là tam giác tự nhiên của module system Triết (ADR-0005 đã khoá hierarchical namespace). Hash sâu hơn (AST node như Unison thuần) tốn cost canonicalization mà không ai consume. Hash nông hơn (1 cấp pack-only) phá VISION §3.1. 3 là điểm cân bằng — và mỗi cấp tương ứng một state của Trit khi LLM/AI address: `Trit::Negative` = term (cấp thấp nhất), `Trit::Zero` = module (trung gian), `Trit::Positive` = package (cấp cao nhất).

### 2. Cấp 1 — Term hash

Một "term" là một export item ở module-system boundary (ADR-0005):
- function declaration
- struct declaration
- enum declaration
- generic-shell declaration

Term hash **không** đệ quy xuống statement/expression/AST-node. Boundary giới hạn ở public ABI surface — cùng granularity mà ADR-0011 §2-§3 đã track.

**`iface_hash_term`** = `BLAKE3(domain_sep_term ‖ canonical_signature_bytes)`

Canonical signature bytes (deterministic, exclude debug/source location):
```
term_kind: u8              // 0=function, 1=struct, 2=enum, 3=generic-shell
name: length-prefixed UTF-8
visibility: u8             // 0=public, 1=package, 2=private
type_param_count: varint
  for each: param_name (length-prefixed UTF-8)  // positional, preserve source order
body: kind-specific encoding (per ADR-0011 §2 struct/enum body or §3 function signature)
```

**Loại trừ:** `body_offset` (storage detail), capability claims (pkg-level — ADR-0011 §5), doc comments, span info.

**`impl_hash_term`** = `BLAKE3(domain_sep_term_impl ‖ iface_hash_term ‖ term_ir_body_bytes)`

`term_ir_body_bytes` = canonical bytes của block IR riêng cho term này trong `.triv` code section. Yêu cầu format change: code section phải có per-term offset index (xem §5).

### 3. Cấp 2 — Module hash

Một "module" identified bằng dotted path từ ADR-0005 (`crate.foo.bar`, `std.text`, etc.). Top-level inline module và file-bound module đều count.

**`iface_hash_mod`** = `BLAKE3(domain_sep_mod_iface ‖ module_path_bytes ‖ sorted_term_iface_hashes)`

Sorted_term_iface_hashes = sequence of (`term_name_len: u32 LE`, `term_name_bytes`, `iface_hash_term: 32 bytes`) cho mỗi term thuộc module này, sort theo `term_name` lexicographically.

**`impl_hash_mod`** = `BLAKE3(domain_sep_mod_impl ‖ iface_hash_mod ‖ sorted_term_impl_hashes)`

### 4. Cấp 3 — Package hash

Replace ADR-0011 §6 hash inputs. **Cùng output shape (32 bytes)**, cùng field name (`iface_hash`/`impl_hash`) — bytes thay đổi vì rollup formula đổi.

**`iface_hash_pkg`** = `BLAKE3(`
- `domain_sep_pkg_iface ‖`
- `pkg_name (length-prefixed) ‖`
- `sorted module entries: (mod_path_len, mod_path_bytes, iface_hash_mod) ‖`
- `deps table bytes (canonical, per ADR-0011 §4) ‖`
- `caps table bytes (canonical, per ADR-0011 §5)`
- `)`

**`impl_hash_pkg`** = `BLAKE3(domain_sep_pkg_impl ‖ iface_hash_pkg ‖ sorted impl_hash_mod sequence)`

### 5. Encoding thay đổi trong `.tripack` (abi_version bump 1 → 2)

Additive — v1 readers gặp `abi_version = 2` phải refuse với E2301 (per ADR-0013 §3). Không có shim đọc partial v2 — Triết là **refuse over guess**.

**Types table (ADR-0011 §2):** mỗi type entry thêm cuối:
```
iface_hash_term: 32 bytes
impl_hash_term:  32 bytes
```

**Exports table (ADR-0011 §3):** mỗi export entry thêm cuối:
```
iface_hash_term: 32 bytes
impl_hash_term:  32 bytes
```

**Modules table (mới, section ID giữa exports và deps):**
```
mod_count: varint
for each:
    mod_path: length-prefixed UTF-8
    iface_hash_mod: 32 bytes
    impl_hash_mod:  32 bytes
```

**Code section (`.triv` reference từ ADR-0008):** thêm **per-term offset index** trước instruction stream:
```
term_offset_count: varint
for each: term_name (length-prefixed UTF-8) + body_start: varint + body_len: varint
[instruction bytes — như cũ]
```

`.triv` wire format bump **v3 → v4** (v3 đã bumped ở ADR-0012 cho WitnessCall). v3 readers gặp v4 file → E2301.

### 6. Domain separation

BLAKE3 không bị length-extension như SHA-2 nhưng vẫn cần domain separation để chống ambiguity khi cùng input bytes được hash ở 2 cấp khác nhau (ví dụ term name trùng module path).

Domain separator là **16-byte ASCII prefix với NUL pad**:
```
b"triet/term-i  \0\0"   // iface_hash_term     (16 bytes)
b"triet/term-m  \0\0"   // impl_hash_term      (m = "mut/impl")
b"triet/mod-i   \0\0"   // iface_hash_mod
b"triet/mod-m   \0\0"   // impl_hash_mod
b"triet/pkg-i   \0\0"   // iface_hash_pkg
b"triet/pkg-m   \0\0"   // impl_hash_pkg
```

Chuỗi cố định, lock ở constants trong `triet-pack/src/hash.rs`. Đổi separator = đổi mọi hash = bump abi_version. Không-được-đổi-im-lặng.

### 7. Normalization rules (strengthen ADR-0011 §6)

- **Sort**: lexicographic theo raw UTF-8 bytes của name (không phải Unicode collation — implementation-independent).
- **Varint**: LEB128 minimal encoding (no trailing-zero pad). Decoder reject non-minimal — strict mode.
- **Length-prefixed string**: `u32 LE length`, **no NUL terminator**, no BOM, no validation rerun (caller-provided UTF-8 trusted).
- **TypeRef ordering** (ADR-0011 §2): ref_kind byte trước, sau đó payload — deterministic per discriminator value.
- **Type param order**: positional (source declaration order), không sort.
- **Sub-table sort key**: name primary; nếu trùng name (cross-namespace shouldn't happen post-ADR-0005, nhưng defensive): secondary key = full canonical path bytes.

Test invariant cho `triet-pack`: round-trip một AbiMetadata → encode → hash → re-encode → hash → bytes ≡, hash ≡. Existing `iface_hash_ignores_pkg_version` test extends to 3 cấp.

### 8. iface_hash là final arbiter — không đổi

[ADR-0013 §4](0013-semver-linking-policy.md) lock policy "semver là declaration, hash là proof". ADR-0014 **không** đổi điều này. Linker vẫn check `iface_hash_pkg` (cấp 3) — đó là arbiter. Cấp 1 + cấp 2 là **enabler cho dedup**, không phải linker contract.

**Hệ quả:** linker không reject pack khi term-level hash drift nhưng pkg-level match. Author chịu trách nhiệm — nếu term hash đổi mà pkg hash không, đó là rollup error (defensive test trong `triet-pack`).

## Hệ quả

### Cho v0.5 CAS store (ADR-0015 sắp viết)

- Filesystem layout có thể address ở 3 cấp:
  - `~/.triet/store/term/<hex(impl_hash_term)>/code.bin` — function-level dedup
  - `~/.triet/store/mod/<hex(impl_hash_mod)>/index.bin` — module-level metadata
  - `~/.triet/store/pkg/<hex(impl_hash_pkg)>/pack.tripack` — pack-level distribution unit
- VISION §3.1 gate đạt được: `std.text.format` chia sẻ qua N apps lookup-by-term-hash.

### Cho v0.6 Capability

- Caps table giữ nguyên ở pkg-level (ADR-0011 §5). Term-level capability annotations (nếu cần) đi vào term signature → đã được hash bởi `iface_hash_term`. Không phá v0.5 invariants.

### Cho v0.7 Self-hosting

- Re-implement hash computation trong Triết. ADR-0014 là canonical spec. Cross-bootstrap diff: cùng AbiMetadata → cùng 3-tuple hashes qua Rust impl vs Triết impl.

### Cho `.triv` wire format

- Bump v3 → v4 (per-term offset index). Linker/VM v3 reader gặp v4 file → E2301. Không lossy-fallback.

### Cho linker performance

- Per-export hash thêm 64 bytes overhead mỗi export trong ABI metadata. Pack typical ~50 exports = ~3KB overhead. Negligible.

### Cho generic dispatch (ADR-0012 witness tables)

- Witness table reference giờ có thể pin bằng `iface_hash_term` (per-function) thay vì pkg-level. v0.5 chưa exploit — slot reserved.

## Không làm

- **Không hash AST node** (Unison thuần). Triết hash ở module-system boundary, không sâu hơn. Term-of-term hashing thêm cost canonicalize mỗi sub-expression, không ai consume ở v0.5 — over-engineering.
- **Không Ternary Vector Versioning** ([ADR-0006](0006-ternary-packaging-vision.md) §2). Tách thành ADR sau v0.5 ship CAS cơ bản. Versioning là semantic intent, hashing là content identity — orthogonal concerns.
- **Không capability per-term** ở v0.5. Caps remain pkg-level. v0.6 ADR sẽ revisit.
- **Không network CAS** (`triet pull <hash>` style). Local store first; distributed registry là v1.0+ topic.
- **Không content-defined chunking** (FastCDC/rsync-style). Term boundary đã là natural chunk — không cần thêm layer.
- **Không cross-platform hash variance**. BLAKE3 deterministic; ADR-0011 §6 đã lock little-endian + arch-independent IR.
- **Không Merkle proof / inclusion proof** API. v0.5 store là trust-local-filesystem; cryptographic proof không cần thiết khi không có untrusted peer.

## Prior art

- **Unison** ([unison-lang.org](https://www.unison-lang.org/)) — main inspiration. Term-level hashing là core idea. Triết khác: hash boundary ở module-system level (function/type), không xuống AST node. Trade-off: ít dedup hơn Unison, nhưng canonicalization simpler + alignment với ADR-0005 module structure.
- **Git Merkle tree** — blob → tree → commit là 3 cấp parallel với term → module → pkg. Git inspire trên cấu trúc cây, không trên content (Git hash arbitrary blob bytes; Triết hash canonical signature).
- **Nix derivations** ([nixos.org](https://nixos.org/)) — pkg-only CAS. Triết extend xuống term-level cho RAM-sharing use case mà Nix không cover.
- **IPFS / Merkle DAG** — general theory của content-addressed graphs. Triết là instance đặc biệt (3-cấp, không arbitrary depth).
- **Bazel action cache** — input-hash → output-hash mapping. Triết tương đương ở cấp pack (impl_hash là cache key), thêm 2 cấp dưới.
- **Anti-prior-art:** Java `.jar` (no hash identity, ClassNotFoundException hell); npm (semver-only resolution, content drift unobserved); Maven Central (sha checksum chỉ for download integrity, không cho identity/dedup).

## Tham chiếu

- [VISION §3.1 — CAS Packaging](../../VISION.md) (lời hứa RAM-sharing)
- [VISION §3.3 — Stable ABI](../../VISION.md) (iface_hash là arbiter)
- [ADR-0005 — Module system](0005-module-system.md) (định nghĩa term/module boundary)
- [ADR-0006 — Ternary packaging vision](0006-ternary-packaging-vision.md) (informational, ADR-0014 chỉ implement phần "CAS hash" — Ternary Versioning tách)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (per-term offset index = v3 → v4 bump)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (ADR-0014 extends §2, §3, §6)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (final arbiter rule giữ nguyên)
- [ADR-0015 — Package store layout](0015-package-store-layout.md) (sibling, viết sau ADR-0014)
- [ROADMAP § v0.5](../../ROADMAP.md)
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs)
