# ADR 0015 — Package store layout (CAS filesystem)

**Trạng thái:** Quyết định. Áp dụng cho v0.5 CAS store implementation và mọi tool đọc/ghi `~/.triet/store/` kể từ v0.5. Sibling của [ADR-0014](0014-hash-scheme-refinement.md) (hash scheme) — ADR-0014 định nghĩa **identity bytes**, ADR-0015 định nghĩa **filesystem nơi identity ở**.

**Issue:** ADR-0014 lock 3-cấp hash (term/module/pkg) nhưng không nói hash ở đâu trên đĩa. Để v0.5 ship lời hứa VISION §3.1 (shared loading), cần spec:

1. **Filesystem root** — store ở đâu? (per-user? per-project?)
2. **Directory shape** — đường dẫn hash → file thế nào?
3. **Content per directory** — mỗi hash dir chứa file gì?
4. **Symbolic resolution** — `use foo` tìm `impl_hash_pkg` của `foo` qua đâu?
5. **Garbage collection** — pack/module/term không reference thì dọn ra sao?
6. **Concurrent access** — nhiều process write store cùng lúc?
7. **Migration** — pack hiện đang load từ filesystem path → vào store thế nào?

[ROADMAP § v0.5](../../ROADMAP.md) đã hint `~/.triet/store/<hash>/` nhưng chưa lock. ADR-0015 lock.

## Quyết định

### 1. Filesystem root

```
$TRIET_STORE  (env override)
    │
    ▼  fallback
~/.triet/store/   (Linux/macOS)
%APPDATA%\triet\store\   (Windows, v0.5 không support nhưng path reserved)
```

**Một store per-user**, không per-project. Lý do: dedup chỉ có ý nghĩa khi N project chia sẻ cùng store. Per-project nullify lời hứa VISION §3.1.

`$TRIET_STORE` override cho:
- CI builds (isolated store per job).
- Self-hosting bootstrap (v0.7) cần multi-store cô lập.
- Test fixtures (`tests/fixtures/store/`).

### 2. Directory layout — 3 nhánh mirror ADR-0014

```
$TRIET_STORE/
├── term/
│   └── <64-hex(impl_hash_term)>/
│       ├── iface.bin     # canonical signature bytes của term
│       └── body.bin      # IR body bytes của term
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

**Three top-level branches map directly to Trit identity** (xem ADR-0014 §1): `term/` ≈ `Trit::Negative` cấp thấp nhất / fine-grained, `mod/` ≈ `Trit::Zero` trung gian, `pkg/` ≈ `Trit::Positive` cấp cao nhất / distribution unit. Không phải metaphor — đây là design choice nhất quán với bản sắc.

### 3. Hash → path encoding

- **Hex lowercase**, 64 ký tự (32 byte BLAKE3 → 64 hex chars).
- **Không prefix-split** (Git dùng `ab/cdef...`, Nix flat). Triết theo Nix — modern filesystem (ext4/btrfs/apfs/zfs) handle 100k+ entries/dir mà không degrade. Đơn giản hơn cho tool dump/inspect.
- **Không base32/base64** — hex universally readable, copy-paste safe, case-insensitive disambiguation không cần.

Example:
```
~/.triet/store/term/a1b2c3d4e5f6...64hex/body.bin
```

### 4. Content per directory

**`term/<hash>/`:**
- `iface.bin` — canonical signature bytes đã hash thành `iface_hash_term` (ADR-0014 §2). Cho phép verify hash post-install và inspect mà không cần parent pkg.
- `body.bin` — IR body bytes đã hash vào `impl_hash_term`. Đây là code thực thi.

**`mod/<hash>/`:**
- `index.bin` — sorted list of `(term_name_len, term_name_bytes, impl_hash_term)`. Cho phép resolve "module này gồm term nào" mà không load parent pkg.

**`pkg/<hash>/`:**
- `pack.khi` — full container đã ship (ABI metadata + IR code + manifest, per ADR-0011).
- `manifest.bin` — extracted ABI metadata bytes (re-read cheap; pack.tripack chỉ load khi cần code).

**`names/<pkg_name>/<semver>.link`:**
- File text 1 dòng: 64-hex của `impl_hash_pkg`. Là **alias từ symbolic name → CAS hash**. Resolver tra `names/foo/1.2.3.link` → đọc hash → `cd pkg/<hash>/`.

**`roots/<project_id>.root`:**
- File text n dòng. Mỗi dòng: 1 `impl_hash_pkg` mà project đang reference. Là **GC root** — `dao store gc` không xoá hash trong roots.
- `<project_id>` = BLAKE3 hash của absolute path tới project root (anonymous, deterministic per-project).

**`tmp/`:**
- Staging cho atomic install (xem §6). Cleaned up by `dao store gc`.

### 5. Symbolic name resolution flow

`use foo` trong source (ADR-0005 import) resolve qua:

```
1. Đọc dao.lock (per-project): find dep `foo` → có (pkg_name, impl_hash_pkg pin).
2. Lookup ~/.triet/store/pkg/<impl_hash_pkg>/manifest.bin.
3. Nếu missing: trigger install (path-based source rebuild hoặc network fetch — v0.5 chỉ local).
4. Manifest có module table → mỗi module có `impl_hash_mod` → resolve qua store/mod/.
5. Module index → term hash → store/term/<hash>/body.bin để load IR.
```

**Không có lockfile:** fallback lookup theo `names/foo/<version>.link` với version constraint từ source manifest. Ghi vào lockfile sau khi resolve thành công (consistency với Cargo/npm pattern).

### 6. Atomic install protocol

Để N process cùng install một hash mà không corrupt:

```
1. Compute target hash H.
2. Check store/pkg/<H>/pack.tripack exists → done, no-op.
3. Otherwise: write to store/tmp/<random_uuid>/pack.tripack.
4. fsync(file). fsync(dir).
5. rename(tmp/<uuid>, pkg/<H>) — atomic on POSIX.
6. If rename fails with EEXIST (another process won) → cleanup tmp, treat as success.
```

Tương đương cho `term/` và `mod/`. **No locks** — rename atomicity đủ.

### 7. Garbage collection

CLI command: `dao store gc` (manual, v0.5 không auto).

```
Mark phase:
  - Đọc tất cả roots/*.root files.
  - Mỗi pkg hash trong roots → mark pkg/<hash>/.
  - Load pkg manifest → mark module hashes referenced.
  - Load module index → mark term hashes referenced.

Sweep phase:
  - For each dir in {term, mod, pkg}: nếu hash chưa marked → rm -rf dir.
  - For each file in names/*/*.link: nếu target hash đã sweep → unlink file.
  - rm -rf tmp/* unconditionally (no in-progress install survives gc — user re-run).
```

**Không auto-GC ở v0.5.** Thêm vào cron / pre-build hook là future work.

### 8. Concurrent access guarantees

- **Read-read**: trivially safe (no mutation).
- **Read-write**: writer dùng `tmp/<uuid>` rồi rename → reader luôn thấy fully-written hoặc absent.
- **Write-write same hash**: race ⇒ một bên thắng rename, bên kia EEXIST → no-op. Both see consistent final state.
- **GC vs install race**: GC chạy mark-then-sweep. Nếu install happens DURING gc → install pack chưa ở roots → có thể bị sweep. Mitigation v0.5: GC requires no-other-triet-process advisory (check via `lsof ~/.triet/store/` heuristic, warn user).

V0.5 không guarantee GC strong consistency với concurrent install. Future v0.6+ có thể add file-lock-based exclusion.

### 9. Migration path

CLI: `dao store import <path/to/foo.tripack>`:
```
1. Read foo.tripack.
2. Verify abi_version compatible (≥1 — v0.4 reads OK at pkg level).
3. If abi_version = 1 (v0.4 pack): compute term/module hashes ad-hoc (lossy — IR body bytes split heuristically by export). Issue warning E2360 (lossy import).
4. If abi_version ≥ 2 (v0.5+ pack): hashes already in metadata → direct install.
5. Write to store atomically (per §6).
6. Update names/<pkg_name>/<version>.link.
```

**E2360** (new, namespace E23XX per ADR-0013): warn that pre-v0.5 packs có thể không dedup tốt vì hash tree incomplete.

### 10. Cross-platform notes

- Path separator: Rust `std::path::PathBuf` handle.
- Hex case: always lowercase. Case-insensitive filesystems (macOS HFS+ default) safe.
- Symlinks NOT used — store dùng plain dirs + hash content. Avoid Windows symlink permission hell.
- `fsync` on directories: skipped on Windows (NTFS atomic rename mà không cần dir fsync).

## Hệ quả

### Cho v0.5 deliverables

- `triet-pack` crate gains `Store` API: `Store::open()`, `Store::install_pack()`, `Store::resolve_term()`, `Store::gc()`.
- CLI new subcommands: `dao store {add, list, import, gc, root}`.
- Resolver (v0.5.5) uses `Store::resolve_*` instead of filesystem walk.

### Cho VISION §3.1 (shared loading)

- 2 apps reference cùng `impl_hash_term` → loader maps cùng `body.bin` (mmap khi VM v0.5+ support). 1 bản RAM, gate đạt.

### Cho v0.6 Capability

- Caps stored ở pkg manifest (ADR-0011 §5). Resolver có thể refuse-to-link pkg với cap claim không match. ADR-0015 chỉ store; enforcement ở loader.

### Cho v0.7 Self-hosting

- Bootstrap chain: Rust-compiler-v0.6 install xong → `~/.triet/store/pkg/<rust_compiler_hash>/`. Triết-compiler-v0.7 read same store → cross-impl deduplication. ADR-0015 là spec.

### Cho disk footprint

- Per-term overhead ~64 bytes metadata. Pack 100KB → store có thể tách thành 50 term × 2KB = 100KB total (no overhead) + module index ~5KB. Total ~5% overhead cho granularity.
- GC reclaims abandoned hashes — manual nhưng deterministic.

### Cho linker performance

- Cold cache (everything missing): linker spawn install pipeline, chậm.
- Warm cache (deps đã trong store): linker chỉ read manifest.bin per dep → milliseconds.

## Không làm

- **Không network fetch** ở v0.5. Tất cả install là local (qua `dao store import` hoặc per-project rebuild). Distributed registry là v1.0+ topic.
- **Không content compression**. body.bin là raw IR bytes. Compression (zstd) là disk-saving optimization defer cho v0.8+ nếu cần.
- **Không auto-GC**. User control. v0.5 nguyên tắc *Refuse over guess* (VISION §6) — auto-delete code là dangerous default.
- **Không filesystem encryption**. Store không chứa secret. User responsibility nếu cần (disk-level encryption đủ).
- **Không signature/provenance**. v0.5 trust local install path. Sigstore/Notary kiểu chain-of-trust là v1.0+ feature.
- **Không Windows-first support** ở v0.5. Path reserved (`%APPDATA%\triet\store\`) nhưng implementation Linux/macOS first.
- **Không multi-version simultaneous load của cùng pack** (run-app-with-pkg-v1-AND-pkg-v2 ở cùng process). Store *cho phép* (different hash dirs) nhưng loader v0.5 chỉ load 1 version per pkg name. Đây là VM concern, không phải store concern.

## Prior art

- **Nix store** (`/nix/store/<hash>-<name>/`) — chính. Triết theo Nix style cho hex-flat layout + GC root mechanism. Khác: Nix là per-derivation, Triết là 3-cấp.
- **Git objects** (`.git/objects/ab/cdef...`) — prefix-split inspiration (Triết reject vì modern fs handle flat OK, đơn giản hơn).
- **Cargo registry cache** (`~/.cargo/registry/cache/`) — flat pkg-level cache; không có term-level dedup. Triết extend.
- **npm `node_modules`** — anti-prior-art. Pkg-name resolution by directory tree → DLL Hell incarnate. Triết hash-based resolution thay vì name-based.
- **Bazel `~/.cache/bazel/`** — action cache với hash addressing. Triết gần đúng nhưng dedup ở artifact level, không action level.
- **OCI image registry** (`/var/lib/containers/storage/`) — layer-based hash sharing. Triết term ≈ OCI layer ở concept.

## Tham chiếu

- [VISION §3.1 — CAS Packaging](../../VISION.md) (RAM-sharing lời hứa)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (manifest.bin content)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (E23XX namespace cho E2360 lossy import)
- [ADR-0014 — Hash scheme refinement](0014-hash-scheme-refinement.md) (defines hashes that name dirs)
- [ROADMAP § v0.5](../../ROADMAP.md)
- [Nix store spec](https://nixos.org/manual/nix/stable/store/file-system-object.html)

---

## Addendum — v0.5.x.review (pre-v0.6 audit)

Audit window trước khi mở v0.6 capability system. Không thay đổi
quyết định gốc; làm rõ behavior + bít blind spot trong test coverage.

### Resolver origin — 3-state thay vì bool

`Resolution.from_lockfile: bool` (v0.5.5 initial) gộp 2 đường khác nhau:
*iface_hash_pin matching* và *plain enumeration*. Audit gắn cờ là binary
leak so với bản sắc tam phân (VISION §5).

Refactor thành `ResolutionOrigin { Lockfile, IfacePin, Fresh }`. Đây là
3 đường thực sự §5 đã thiết kế nhưng v0.5.5 chỉ encode được 2. Cần
thiết cho v0.6 capability gates muốn áp policy khác nhau theo origin
(ví dụ: chỉ `Lockfile` được auto-trust, `IfacePin` cần admin grant).

Commit: `20076d5`.

### GC conservative under manifest corruption

§6 (mark-and-sweep) trước đây không quy định behavior khi
`pkg/<hash>/manifest.bin` parse fail trong mark phase. v0.5.4 default:
silently skip → mod/term refs không được mark → sweep remove các deps
đó → silent orphan + data leak.

Behavior chốt lại:

1. Nếu BẤT KỲ live pkg nào có manifest corrupt → push hash vào
   `GcReport.corrupt_pkgs`.
2. Khi `corrupt_pkgs` non-empty → **skip mod + term sweeps hoàn toàn**
   (conservative mode). Pkg-level sweep vẫn chạy (unreferenced pkgs đi
   bình thường — không ảnh hưởng bởi corruption ở pkg khác).
3. User thấy `corrupt_pkgs` trong report → fix corruption → re-run GC.

Khớp nguyên tắc VISION §6 *Refuse over guess*. Commit: `d7f1beb`.

### Race-safety verified

§6 quy định EEXIST = race-loss = success. v0.5.4 có code path nhưng
không có integration test với threading thật. v0.5.x.review.2 thêm
`concurrent_install_same_hash_is_race_safe` — 8 threads cùng install
identical bytes → tất cả nhận cùng hash, 1 pkg dir duy nhất, `tmp/`
sạch.

### Multi-root invariant verified

§7 quy định "pkg sống iff reachable từ ≥1 root". v0.5.4 test duy nhất
1-to-1 root↔pkg. v0.5.x.review.3 thêm
`gc_keeps_pkg_referenced_by_multiple_roots` — 2 projects cùng pin 1
pkg, remove 1 root → pkg vẫn sống; remove root thứ 2 → mới sweep.

### `$TRIET_STORE` fallback chain

Hai arm bổ sung của `resolve_store_root()` giờ có test tường minh:
HOME fallback (TRIET_STORE unset) tạo store ở `$HOME/.triet/store/`;
both-unset trả lỗi rõ ràng. Không thay đổi spec — chỉ verify.

### Windows defer (explicit)

ADR-0015 đã ghi "Không Windows-first support ở v0.5". v0.5.x.review
xác nhận không thêm Windows-specific test (POSIX `rename` atomicity
là semantic chính của atomic install). Khi v0.6+ mở rộng platform
support, cần ADR riêng cho Windows rename behavior + lock file
strategy (POSIX advisory locks không có trên NTFS).
