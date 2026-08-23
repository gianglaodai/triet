# ADR 0015 — Package store layout (CAS filesystem)

**Status:** Decided. Applies to v0.5 CAS store implementation and all tools reading `~/.triet/store/` from v0.5 onwards. Sibling of [ADR-0014](0014-hash-scheme-refinement.md) (hash scheme) — ADR-0014 defines **identity bytes**, ADR-0015 defines the **filesystem where identity resides**.

**Issue:** ADR-0014 locks the 3-level hash (term/module/pkg) but does not specify where the hash resides on disk. To fulfill the VISION §3.1 promise (shared loading) for v0.5, the following must be specified:

1. **Filesystem root** — where is the store located? (per-user? per-project?)
2. **Directory shape** — what is the hash $\rightarrow$ file path mapping?
3. **Content per directory** — what files does each hash directory contain?
4. **Symbolic resolution** — how does `use foo` find the `impl_hash_pkg` of `foo`?
5. **Garbage collection** — how are unreferenced packs/modules/terms cleaned up?
6. **Concurrent access** — how are multiple processes writing to the store simultaneously?
7. **Migration** — how are packs currently loaded from filesystem paths migrated into the store?

[ROADMAP § v0.5](../../ROADMAP.md) hinted at `~/.triet/store/<hash>/` but did not lock it. ADR-0015 locks it.

## Decision

### 1. Filesystem root

```
$TRIET_STORE  (env override)
    │
    ▼  fallback
~/.triet/store/   (Linux/macOS)
%APPDATA%\triet\store\   (Windows, unsupported in v0.5 but path is reserved)
```

**One store per-user**, not per-project. Reason: deduplication only provides value when N projects share the same store. Per-project storage nullifies the VISION §3.1 promise.

`$TRIET_STORE` overrides are provided for:
- CI builds (isolated store per job).
- Self-hosting bootstrap (v0.7) requiring isolated multi-stores.
- Test fixtures (`tests/fixtures/store/`).

### 2. Directory layout — 3 branches mirroring ADR-0014

```
$TRIET_STORE/
├── term/
│   └── <64-hex(impl_hash_term)>/
│       ├── iface.bin     # canonical signature bytes of the term
│       └── body.min      # IR body bytes of the term
│
├── mod/
│   └── <64-hex(impl_hash_mod)>/
│       └── index.bin     # sorted list of (term_name, impl_hash_term) entries
│
├── pkg/
│   └── <64-hex(impl_hash_pkg)>/
│       ├── pack.tripack  # full container
│       └── manifest.bin  # extracted ABI metadata (cheap re-read)
│
├── names/
│   └── <pkg_name>/
│       └── <semver>.link # contains impl_hash_pkg hex bytes
│
├── roots/
│   └── <project_id>.root # contains lockfile path + pkg hash refs
│
└── tmp/                  # staging dir for atomic install
```

**Three top-level branches map directly to Trit identity** (see ADR-0014 §1): `term/` $\approx$ lowest-level/fine-grained `T/Negative`, `mod/` $\approx$ intermediate `T/Zero`, `pkg/` $\approx$ highest-level/distribution unit `T/Positive`. This is not a metaphor — it is a design choice consistent with the identity.

### 3. Hash $\rightarrow$ path encoding

- **Lowercase hex**, 64 characters (32-byte BLAKE3 $\rightarrow$ 64 hex chars).
- **No prefix-splitting** (Git uses `ab/cdef...`, Nix is flat). Triet follows Nix — modern filesystems (ext4/btrfs/apfs/zfs) handle 100k+ entries/dir without degradation. This is simpler for dump/inspect tools.
- **No base32/base64** — hex is universally readable, copy-paste safe, and requires no case-insensitive disambiguation.

Example:
```
~/.triet/store/term/a1b2c3d4e5f6...64hex/body.bin
```

### 4. Content per directory

**`term/<hash>/`:**
- `iface.bin` — canonical signature bytes hashed into `iface_hash_term` (ADR-0014 §2). This allows hash verification post-install and inspection without the parent package.
- `body.bin` — IR body bytes hashed into `impl_hash_term`. This is the executable code.

**`mod/<hash>/`:**
- `index.bin` — sorted list of `(term_name_len, term_name_bytes, impl_hash_term)`. This allows resolving "which terms are in this module" without loading the parent package.

**`pkg/<hash>/`:**
- `pack.tripack` — the full container as shipped (ABI metadata + IR code + manifest, per ADR-0011).
- `manifest.bin` — extracted ABI metadata bytes (cheap to re-read; `pack.tripack` is only loaded when code is required).

**`names/<pkg_name>/<semver>.link`:**
- A single-line text file: the 64-hex `impl_hash_pkg`. This is an **alias from symbolic name $\rightarrow$ CAS hash**. The resolver looks up `names/foo/1.2.3.link` $\rightarrow$ reads the hash $\rightarrow$ `cd pkg/<hash>/`.

**`roots/<project_id>.root`:**
- A multi-line text file. Each line: one `impl_hash_pkg` referenced by the project. This is the **GC root** — `dao store gc` will not delete hashes present in roots.
- `<project_id>` = BLAKE3 hash of the absolute path to the project root (anonymous, deterministic per-project).

**`tmp/`:**
- Staging directory for atomic installation (see §6). Cleaned up by `dao store gc`.

### 5. Symbolic name resolution flow

`use foo` in source (ADR-0005 import) resolves via:

```
1. Read dao.lock (per-project): find dependency `foo` $\rightarrow$ found (pkg_name, pinned impl_hash_pkg).
2. Lookup ~/.triet/store/pkg/<impl_hash_pkg>/manifest.bin.
3. If missing: trigger install (path-based source rebuild or network fetch — v0.5 is local only).
4. Manifest contains module table $\rightarrow$ each module has an `impl_hash_mod` $\rightarrow$ resolve via store/mod/.
5. Module index $\rightarrow$ term hash $\rightarrow$ store/term/<hash>/body.bin to load IR.
```

**No lockfile:** fallback lookup via `names/foo/<version>.link` using version constraints from the source manifest. The lockfile is updated after successful resolution (consistent with Cargo/npm patterns).

### 6. Atomic install protocol

To ensure N processes can install the same hash without corruption:

```
1. Compute target hash H.
2. Check if store/pkg/<H>/pack.tripack exists $\rightarrow$ done, no-op.
3. Otherwise: write to store/tmp/<random_uuid>/pack.tripack.
4. fsync(file). fsync(dir).
5. rename(tmp/<uuid>, pkg/<H>) — atomic on POSIX.
6. If rename fails with EEXIST (another process won) $\rightarrow$ cleanup tmp, treat as success.
```

The same protocol applies to `term/` and `mod/`. **No locks** — rename atomicity is sufficient.

### 7. Garbage collection

CLI command: `dao store gc` (manual; v0.5 does not include auto-GC).

```
Mark phase:
  - Read all roots/*.root files.
  - For each pkg hash in roots $\rightarrow$ mark pkg/<hash>/.
  - Load pkg manifest $\rightarrow$ mark referenced module hashes.
  - Load module index $\rightarrow$ mark referenced term hashes.

Sweep phase:
  - For each dir in {term, mod, pkg}: if hash is not marked $\rightarrow$ rm -rf dir.
  - For each file in names/*/*.link: if target hash was swept $\rightarrow$ unlink file.
  - rm -rf tmp/* unconditionally (no in-progress install survives GC — user must re-run).
```

**No auto-GC in v0.5.** Adding this to a cron job or pre-build hook is future work.

### 8. Concurrent access guarantees

- **Read-read**: trivially safe (no mutation).
- **Read-write**: writer uses `tmp/<uuid>` then renames $\rightarrow$ reader always sees either a fully-written file or an absent one.
- **Write-write (same hash)**: race $\Rightarrow$ one side wins the rename, the other receives EEXIST $\rightarrow$ no-op. Both see a consistent final state.
- **GC vs. install race**: GC runs mark-then-sweep. If an install happens DURING GC $\rightarrow$ the install pack is not yet in roots $\rightarrow$ it may be swept. v0.5 Mitigation: GC requires no-other-triet-process advisory (verified via `lsof ~/.triet/store/` heuristic; warns user).

v0.5 does not guarantee strong GC consistency with concurrent installs. Future v0.6+ may add file-lock-based exclusion.

### 9. Migration path

CLI: `dao store import <path/to/foo.tripack>`:
```
1. Read foo.tripack.
2. Verify abi_version compatibility (≥1 — v0.4 reads OK at pkg level).
3. If abi_version = 1 (v0.4 pack): compute term/module hashes ad-hoc (lossy — IR body bytes split heuristically by export). Issue warning E2360 (lossy import).
4. If abi_version ≥ 2 (v0.5+ pack): hashes are already in metadata $\rightarrow$ direct install.
5. Write to store atomically (per §6).
6. Update names/<pkg_name>/<version>.link.
```

**E2360** (new, namespace E23XX per ADR-0013): warns that pre-v0.5 packs may not deduplicate effectively because the hash tree is incomplete.

### 10. Cross-platform notes

- Path separator: Handled by Rust `std::path::PathBuf`.
- Hex case: Always lowercase. Safe for case-insensitive filesystems (macOS HFS+ default).
- Symlinks NOT used — the store uses plain directories + hashed content. This avoids Windows symlink permission issues.
- `fsync` on directories: Skipped on Windows (NTFS supports atomic rename without directory `fsync`).

## Consequences

### For v0.5 deliverables

- `triet-pack` crate gains `Store` API: `Store::open()`, `perm_install_pack()`, `Store::resolve_term()`, `Store::gc()`.
- CLI new subcommands: `dao store {add, list, import, gc, root}`.
- Resolver (v0.5.5) uses `Store::resolve_*` instead of filesystem walking.

### For VISION §3.1 (shared loading)

- 2 apps referencing the same `impl_hash_term` $\rightarrow$ loader maps to the same `body.bin` (via `mmap` when VM v0.5+ supports it). 1 copy in RAM, goal achieved.

### For v0.6 Capability

- Capabilities are stored in the pkg manifest (ADR-0011 §5). The resolver can refuse to link a pkg if the capability claim does not match. ADR-0015 handles storage; enforcement is handled by the loader.

### For v0.7 Self-hosting

- Bootstrap chain: Once Rust-compiler-v0.6 is installed $\rightarrow$ `~/.triet/store/pkg/<rust_compiler_hash>/`. Triet-compiler-v0.7 reads the same store $\rightarrow$ cross-implementation deduplication. ADR-0015 provides the spec.

### For disk footprint

- Per-term overhead $\approx$ 64 bytes of metadata. A 100KB pack $\rightarrow$ the store might split into 50 terms $\times$ 2KB = 100KB total (no overhead) + module index $\approx$ 5KB. Total $\approx$ 5% overhead for granularity.
- GC reclaims abandoned hashes — manual but deterministic.

### For linker performance

- Cold cache (everything missing): Linker spawns the install pipeline, which is slow.
- Warm cache (dependencies already in store): Linker only reads `manifest.bin` per dependency $\rightarrow$ milliseconds.

## Alternatives Considered

- **No network fetch** in v0.5. All installs are local (via `dao store import` or per-project rebuild). Distributed registry is a v1.0+ topic.
- **No content compression**. `body.bin` contains raw IR bytes. Compression (zstd) is a disk-saving optimization deferred to v0.8+ if needed.
- **No auto-GC**. User control. v0.5 follows the principle of *Refuse over guess* (VISION §6) — auto-deleting code is a dangerous default.
- **No filesystem encryption**. The store does not contain secrets. This is the user's responsibility (disk-level encryption is sufficient).
- **No signature/provenance**. v0.5 trusts the local install path. Sigstore/Notary-style chain-of-trust is a v1.0+ feature.
- **No Windows-first support** in v0.5. Path is reserved (`%APHDATA%\triet\store\`), but implementation is Linux/macOS first.
- **No simultaneous multi-version loading of the same pack** (running an app with pkg-v1 AND pkg-v2 in the same process). The store *allows* this (different hash directories), but the v0.5 loader only loads 1 version per pkg name. This is a VM concern, not a store concern.

## Prior art

- **Nix store** (`/nix/store/<hash>-<name>/`) — primary inspiration. Triet follows the Nix-style hex-flat layout + GC root mechanism. Difference: Nix is per-derivation, Triet is 3-level.
- **Git objects** (`.git/objects/ab/cdef...`) — inspiration for prefix-splitting (Triet rejected this because modern filesystems handle flat directories well, and it is simpler).
- **Cargo registry cache** (`~/.cargo/registry/cache/`) — flat pkg-level cache; lacks term-level deduplication. Triet extends this.
- **npm `node_modules`** — anti-prior-art. Package-name resolution via directory tree $\rightarrow$ DLL Hell incarnate. Triet uses hash-based resolution instead of name-based.
- **Bazel `~/.cache/bazel/`** — action cache with hash addressing. Triet is similar but deduplicates at the artifact level, not the action level.
- **OCI image registry** (`/var/lib/containers/storage/`) — layer-based hash sharing. Conceptually, Triet's `term` $\approx$ OCI layer.

## References

- [VISION §3.1 — CAS Packaging](../../VISION.md) (RAM-sharing promise)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (manifest.bin content)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (E23XX namespace for E2360 lossy import)
- [ADR-0014 — Hash scheme refinement](0014-hash-scheme-refinement.md) (defines hashes that name directories)
- [ROADMAP § v0.5](../../ROADMAP.md)
- [Nix store spec](https://nixos.org/manual/nix/stable/store/file-system-object.html)

---

## Addendum — v0.5.x.review (pre-v0.6 audit)

Audit window before opening the v0.6 capability system. No changes to the original decision; clarifies behavior and identifies blind spots in test coverage.

### Resolver origin — 3-state instead of bool

`Resolution.from_lockfile: bool` (v0.5.5 initial) merged two different paths: *iface_hash_pin matching* and *plain enumeration*. The audit flagged this as a binary leak relative to the ternary identity (VISION §5).

Refactored to `ResolutionOrigin { Lockfile, IfacePin, Fresh }`. These are the 3 actual paths designed in §5, but v0.5.5 could only encode 2. This is necessary for v0.6 capability gates that want to apply different policies based on origin (e.g., only `Lockfile` is auto-trusted, while `IfacePin` requires admin grant).

Commit: `20076d5`.

### GC conservative under manifest corruption

§6 (mark-and-sweep) previously did not specify behavior when `pkg/<hash>/manifest.bin` fails to parse during the mark phase. v0.5.4 default: silently skip $\rightarrow$ mod/term refs are not marked $\rightarrow$ sweep removes those deps $\rightarrow$ silent orphaning + data leak.

Finalized behavior:

1. If ANY live pkg has a corrupt manifest $\rightarrow$ push the hash into `GcReport.corrupt_pkgs`.
2. When `corrupt_pkgs` is non-empty $\rightarrow$ **skip mod + term sweeps entirely** (conservative mode). Pkg-level sweep still proceeds (unreferenced pkgs are removed normally — unaffected by corruption in other pkgs).
3. User sees `corrupt_pkgs` in the report $\rightarrow$ fix corruption $\rightarrow$ re-run GC.

Aligns with the VISION §6 principle: *Refuse over guess*. Commit: `d7f1beb`.

### Race-safety verified

§6 specifies EEXIST = race-loss = success. v0.5.4 had the code path but lacked integration tests with real threading. v0.5.x.review.2 added `concurrent_install_same_hash_is_race_safe` — 8 threads installing identical bytes $\rightarrow$ all receive the same hash, a single pkg dir is created, and `tmp/` remains clean.

### Multi-root invariant verified

§7 specifies "pkg is live iff reachable from $\ge$ 1 root". v0.5.4 tested only 1-to-1 root $\leftrightarrow$ pkg. v0.5.x.review.3 added `gc_keeps_pkg_referenced_by_multiple_roots` — 2 projects pinning the same pkg; remove 1 root $\rightarrow$ pkg remains live; remove the second root $\rightarrow$ sweep occurs.

### `$TRIET_STORE` fallback chain

The two additional arms of `resolve_store_root()` now have explicit tests: HOME fallback (TRIET_STORE unset) creates the store at `$HOME/.triet/store/`; both unset returns a clear error. No change to spec — only verification.

### Windows defer (explicit)

ADR-0015 stated "No Windows-first support in v0.5". v0.5.x.review confirms no Windows-specific tests were added (POSIX `rename` atomicity is the primary semantic of atomic install). When v0.6+ expands platform support, a separate ADR will be required for Windows rename behavior + lock file strategy (POSIX advisory locks are not available on NTFS).
