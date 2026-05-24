# ADR 0013 — Semver linking policy

**Trạng thái:** Quyết định. Áp dụng cho linker/loader của v0.4 và mọi tool đọc cross-package dep relationship (linker, package manager future). Tham chiếu trực tiếp ABI metadata (ADR-0011) và witness dispatch (ADR-0012).

**Issue:** VISION §3.3 cam kết: *"compiler refuse-to-link với diff rõ ràng"* khi cross-package ABI mismatch. Nhưng "mismatch" có nhiều cấp độ:

- Patch version đổi (`1.2.3 → 1.2.4`): bug fix, ABI giữ nguyên → link OK.
- Minor version đổi (`1.2.x → 1.3.x`): additive (new exports), backwards-compat → link OK với warning.
- Major version đổi (`1.x → 2.x`): breaking, ABI có thể changed → refuse-to-link với diagnostic.
- Iface hash mismatch dù version range OK: physical drift → cảnh báo nghiêm trọng.

VISION §3.3 cũng nói **KHÔNG hứa auto-shim**. Linker không tự sinh adapter; user phải explicit migrate. ADR này lock exact rules để linker biết khi nào accept/warn/refuse.

## Quyết định

### 1. Semver triple (major.minor.patch)

Mỗi `.khi` mang `pkg_version: (u32, u32, u32)` theo ABI metadata (ADR-0011 §1). Triple bumping rules:

| Bump | Khi nào | ABI impact |
|---|---|---|
| **major** | Remove export / đổi signature / đổi semantic | Breaking |
| **minor** | Add export / add variant cho enum exhaustive-not-required | Additive |
| **patch** | Fix bug nội bộ / optimize, không touch ABI surface | None |

**Tác giả package chịu trách nhiệm tuân thủ rule này.** Linker enforce via `iface_hash` (xem §4) — nếu bumping rule violated, linker refuse-to-link bất kể version triple.

### 2. Linker decision matrix

Cho dep declaration `pkg "foo" ≥1.2.0 <2.0.0`:

| Available `foo` version | iface_hash match dep pin? | Linker action |
|---|---|---|
| `1.2.0` (exact min) | n/a hoặc match | ✅ Accept |
| `1.3.5` (in range, newer minor) | match | ✅ Accept với warning E2310 nếu iface_hash khác lúc consumer compile |
| `1.5.0` (newer minor, much later) | mismatch | ⚠️ Accept với warning E2310 |
| `2.0.0` (major bump) | — | ❌ Refuse with E2320 |
| `1.1.9` (below min) | — | ❌ Refuse with E2321 |
| `1.2.0` nhưng iface_hash khác | mismatch | ⚠️ Accept với warning E2311 + diagnostic showing iface diff |

### 3. Error code namespace E23XX

ADR-0008 reserved E2200-2299 cho IR runtime. E2300-2399 dành cho linker:

| Code | Severity | Meaning |
|---|---|---|
| E2300 | Error | Package not found in search path |
| E2301 | Error | Unsupported `abi_version` (newer than linker supports) |
| E2310 | Warning | iface_hash drift trong minor range (rebuild recommended) |
| E2311 | Warning | iface_hash mismatch with dep pin (force-rebuild if intentional) |
| E2320 | Error | Major version bump — refuse-to-link |
| E2321 | Error | Version below declared minimum |
| E2322 | Error | Dependency cycle in package graph |
| E2330 | Error | Witness table can't be built (generic instantiation invalid) |
| E2340 | Error | ABI surface mismatch (specific function/type diff) |

Tất cả implement `miette::Diagnostic` cho per-ADR-0008 convention.

### 4. iface_hash là final arbiter

Linker không chỉ dựa vào semver triple — **iface_hash mismatch luôn là warning hoặc error**, ngay cả khi version trong range:

```
[E2311] iface_hash drift for package `foo` v1.3.5
  Declared at compile time: 0xa1b2c3d4...
  Found at link time:       0xe5f6789a...
  
  This usually means `foo` was rebuilt after consumer was last compiled.
  
  hint: rebuild consumer with `triet build --force` to refresh iface_hash
  hint: or pin dep với hash: "1.3.5+0xa1b2c3d4" if intentional
```

iface_hash ≠ semver. Semver là **declaration**, iface_hash là **proof**. Khi conflict, hash thắng.

### 5. iface_hash pinning trong dep declaration

ABI metadata dep table (ADR-0011 §4) có field `iface_hash_pin: 32 bytes`. Khi non-zero, linker enforce strict hash match:

| `iface_hash_pin` | Behavior |
|---|---|
| Zero (default) | Match by version range; warn on hash drift |
| Non-zero | Refuse-to-link nếu actual `iface_hash` ≠ pin (E2311 promoted to error) |

Pinning là tiền đề cho v0.5 CAS — package manager pin hash cho reproducible builds.

### 6. Auto-shim explicitly NOT promised

Per VISION §3.3, linker **không** tự sinh adapter code khi major mismatch. Diagnostic E2320 thay vào đó show:

```
[E2320] cannot link: package `foo` major version mismatch
  Declared:  ≥1.2.0 <2.0.0
  Found:     2.1.0
  
  Major version bumps signal breaking changes. The linker won't
  guess how to adapt — please migrate consumer code explicitly.
  
  hint: bump dep declaration to `foo ≥2.0.0 <3.0.0` after verifying
        API surface compatibility, OR pin older version explicitly.
```

Tool `triet pack diff old.tripack new.khi` (v0.4.4+) hiển thị ABI surface diff để hỗ trợ migration. Đây không phải auto-shim — chỉ là human-readable diff tool.

### 7. Workspace-local development override

Trong development (cùng workspace, chưa publish), version số thường không bumped. Linker support **path-based deps** (tương tự Cargo path dep):

```toml
# triet.toml (hypothetical, v0.5+ package manager)
[dependencies]
foo = { path = "../foo" }  # ignore version, always rebuild
```

Path dep bỏ qua semver check, dùng iface_hash để decide rebuild. **Không thay đổi linker logic — chỉ là tool convention layer.**

### 8. Diagnostic format

Tất cả E23XX errors implement `miette::Diagnostic` với:
- Span pointing to dep declaration site (hoặc symbolic location nếu no source).
- Concrete version numbers found vs expected.
- Hash bytes truncated to 8 hex chars cho readability (full 32 bytes available in JSON output).
- `hint:` blocks pointing tới remediation.

Per ADR-0009, JSON output mode đã wired up — chỉ cần add mapping cho mỗi E23XX vào `link_error_code()` trong `crates/triet-cli/src/main.rs`.

## Hệ quả

### Đối với v0.5 (CAS)

- iface_hash đã là final arbiter → CAS resolver dùng same logic.
- Pinning mechanism (§5) đã có → CAS lockfile reuse được.
- iface_hash drift warning (E2310) trở thành signal cho package manager rebuild graph.

### Đối với v0.6 (Capability)

- Capability claims (ADR-0011 §5) compare ở linker level. Mismatch → new E2350-series codes.

### Đối với v0.7 (Self-hosting)

- Compiler-in-Triết phải re-implement E23XX logic. ADR này là spec.

### Đối với JSON output mode

- `link_error_code()` mapper cần thêm trong CLI khi linker land (v0.4.5).

### Đối với tooling

- `triet pack inspect foo.khi` — show metadata (read-only).
- `triet pack diff old.tripack new.khi` — show ABI diff (v0.4.5+).
- `triet link app.tripack lib1.tripack lib2.tripack -o out.triv` — explicit linker invocation (v0.4.5).

## Không làm

- **Auto-rebuild trên iface_hash drift**: linker chỉ warn. Rebuild là user decision (hoặc package manager logic ở v0.5).
- **Patch version compatibility check**: assumed compat (no ABI surface impact). Author chịu trách nhiệm.
- **Compatibility levels giữa minor versions** (e.g. 1.2 vs 1.5): chỉ check iface_hash drift, không có "semantic compatibility score".
- **Deprecation warnings**: future v0.5+ feature qua attribute trong source. Không phải linker concern.
- **Resolver algorithm** (multiple version trong dep graph): defer v0.5 với package manager. v0.4 linker assume single version per package.
- **Network access**: linker hoàn toàn local. Package resolution không touch network.

## Prior art

- **Cargo (Rust)** — SemVer range syntax, ưu tiên highest compatible. Triết theo gần đúng nhưng strict hơn về iface_hash.
- **Maven (Java)** — version conflict resolution complex. Triết tránh — single version per pkg, hash thắng.
- **Swift Package Manager** — semver triple + hash pin. Triết design gần đúng.
- **Go modules** — minimum version selection (MVS) + sum.db. Triết không adopt MVS — explicit ranges thông minh hơn cho system code.
- **npm** — version range hell. Anti-prior-art.

## Tham chiếu

- [VISION §3.3 — Stable ABI: refuse-to-link policy](../../VISION.md)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (this ADR is per phase gate)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (semver triple field)
- [ADR-0012 — Witness table dispatch](0012-witness-table-dispatch.md) (E2330)
- [Semver 2.0.0 specification](https://semver.org/spec/v2.0.0.html)
