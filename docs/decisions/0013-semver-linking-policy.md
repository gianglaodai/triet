# ADR 0013 — Semver Linking Policy

**Status:** Decided. Applies to the v0.4 linker/loader and all tools reading cross-package dependency relationships (linker, future package manager). Directly references ABI metadata (ADR-0011) and witness dispatch (ADR-0012).

**Issue:** VISION §3.3 commits: *"compiler refuse-to-link with a clear diff"* on cross-package ABI mismatches. However, "mismatches" exist across multiple severity levels:

- Patch version change (`1.2.3 → 1.2.4`): bug fix, ABI preserved → link OK.
- Minor version change (`1.2.x → 1.3.x`): additive (new exports), backward-compatible → link OK with warning.
- Major version change (`1.x → 2.x`): breaking, ABI potentially changed → refuse-to-link with diagnostic.
- Iface hash mismatch despite valid version range: physical drift → serious warning.

VISION §3.3 also states **NO auto-shims are promised**. The linker does not generate synthetic adapters; users must migrate explicitly. This ADR locks the exact rules so the linker knows when to accept, warn, or refuse.

## Decision

### 1. Semver triple (major.minor.patch)

Each `.khi` carries `pkg_version: (u32, u32, u32)` per ABI metadata (ADR-0011 §1). Triple bumping rules:

| Bump | When | ABI impact |
|---|---|---|
| **major** | Remove export / change signature / change semantics | Breaking |
| **minor** | Add export / add variant for enum where exhaustiveness is not required | Additive |
| **patch** | Internal bug fix / optimization, does not touch ABI surface | None |

**Package authors are responsible for adhering to these rules.** The linker enforces compliance via `iface_hash` (see §4) — if bumping rules are violated, the linker refuses to link regardless of the version triple.

### 2. Linker decision matrix

For dependency declaration `pkg "foo" ≥1.2.0 <2.0.0`:

| Available `foo` version | iface_hash matches dep pin? | Linker action |
|---|---|---|
| `1.2.0` (exact min) | n/a or match | ✅ Accept |
| `1.3.5` (in range, newer minor) | match | ✅ Accept with warning E2310 if iface_hash differs from consumer compilation |
| `1.5.0` (newer minor, much later) | mismatch | ⚠️ Accept with warning E2310 |
| `2.0.0` (major bump) | — | ❌ Refuse with E2320 |
| `1.1.9` (below min) | — | ❌ Refuse with E2321 |
| `1.2.0` but iface_hash differs | mismatch | ⚠️ Accept with warning E2311 + diagnostic showing iface diff |

### 3. Error code namespace E23XX

ADR-0008 reserved E2200–2299 for IR runtime. E2300–2399 are allocated for the linker:

| Code | Severity | Meaning |
|---|---|---|
| E2300 | Error | Package not found in search path |
| E2301 | Error | Unsupported `abi_version` (newer than linker supports) |
| E2310 | Warning | iface_hash drift within minor range (rebuild recommended) |
| E2311 | Warning | iface_hash mismatch with dep pin (force-rebuild if intentional) |
| E2320 | Error | Major version bump — refuse-to-link |
| E2321 | Error | Version below declared minimum |
| E2322 | Error | Dependency cycle in package graph |
| E2330 | Error | Witness table cannot be built (generic instantiation invalid) |
| E2340 | Error | ABI surface mismatch (specific function/type diff) |

All implement `miette::Diagnostic` per ADR-0008 conventions.

### 4. iface_hash is the final arbiter

The linker does not rely solely on the semver triple — **iface_hash mismatch is always a warning or error**, even when the version falls within range:

```
[E2311] iface_hash drift for package `foo` v1.3.5
  Declared at compile time: 0xa1b2c3d4...
  Found at link time:       0xe5f6789a...
  
  This usually means `foo` was rebuilt after consumer was last compiled.
  
  hint: rebuild consumer with `dao build --force` to refresh iface_hash
  hint: or pin dep with hash: "1.3.5+0xa1b2c3d4" if intentional
```

iface_hash ≠ semver. Semver is a **declaration**, iface_hash is **proof**. In case of conflict, the hash prevails.

### 5. iface_hash pinning in dependency declarations

The ABI metadata dependency table (ADR-0011 §4) includes the field `iface_hash_pin: 32 bytes`. When non-zero, the linker enforces strict hash matching:

| `iface_hash_pin` | Behavior |
|---|---|
| Zero (default) | Match by version range; warn on hash drift |
| Non-zero | Refuse-to-link if actual `iface_hash` ≠ pin (E2311 promoted to error) |

Pinning is a prerequisite for v0.5 CAS — allowing package managers to pin hashes for reproducible builds.

### 6. Auto-shim explicitly NOT promised

Per VISION §3.3, the linker **does not** generate synthetic adapter code upon major mismatches. Diagnostic E2320 instead displays:

```
[E2320] cannot link: package `foo` major version mismatch
  Declared:  ≥1.2.0 <2.0.0
  Found:     2.1.0
  
  Major version bumps signal breaking changes. The linker won't
  guess how to adapt — please migrate consumer code explicitly.
  
  hint: bump dep declaration to `foo ≥2.0.0 <3.0.0` after verifying
        API surface compatibility, OR pin older version explicitly.
```

The `triet pack diff old.tripack new.khi` tool (v0.4.4+) displays ABI surface diffs to assist migration. This is not an auto-shim — merely a human-readable diffing tool.

### 7. Workspace-local development override

During development (within the same workspace, prior to publishing), version numbers are typically not bumped. The linker supports **path-based dependencies** (similar to Cargo path dependencies):

```toml
# triet.toml (hypothetical, v0.5+ package manager)
[dependencies]
foo = { path = "../foo" }  # ignore version, always rebuild
```

Path dependencies bypass semver checks and use iface_hash to determine rebuild necessity. **This does not alter linker logic — it is purely a tooling convention layer.**

### 8. Diagnostic format

All E23XX errors implement `miette::Diagnostic` with:
- Span pointing to dependency declaration site (or symbolic location if no source is available).
- Concrete version numbers found vs. expected.
- Hash bytes truncated to 8 hex characters for readability (full 32 bytes available in JSON output).
- `hint:` blocks indicating remediation steps.

Per ADR-0009, JSON output mode is already wired up — requiring only mapping each E23XX to `link_error_code()` in `crates/triet-cli/src/main.rs`.

## Consequences

### For v0.5 (CAS)

- iface_hash is already the final arbiter → CAS resolver uses identical logic.
- Pinning mechanism (§5) is in place → CAS lockfiles can be reused.
- iface_hash drift warnings (E2310) serve as a signal for the package manager graph rebuild.

### For v0.6 (Capabilities)

- Capability claims (ADR-0011 §5) are compared at the linker level. Mismatches → new E2350-series error codes.

### For v0.7 (Self-hosting)

- The self-hosted compiler must re-implement E23XX logic. This ADR serves as its specification.

### For JSON output mode

- `link_error_code()` mapper must be added to the CLI when the linker lands (v0.4.5).

### For tooling

- `triet pack inspect foo.khi` — display metadata (read-only).
- `triet pack diff old.tripack new.khi` — display ABI diff (v0.4.5+).
- `triet link app.tripack lib1.tripack lib2.tripack -o out.triv` — explicit linker invocation (v0.4.5).

## Alternatives Considered

- **Auto-rebuild on iface_hash drift**: linker only warns. Rebuilding is a user decision (or package manager logic in v0.5).
- **Patch version compatibility check**: assumed compatible (no ABI surface impact). Responsibility lies with package author.
- **Compatibility levels between minor versions** (e.g. 1.2 vs 1.5): checks iface_hash drift only, no "semantic compatibility scoring".
- **Deprecation warnings**: future v0.5+ feature via source attributes. Not a linker concern.
- **Resolver algorithm** (multiple versions in dependency graph): deferred to v0.5 with package manager. v0.4 linker assumes single version per package.
- **Network access**: linker is strictly local. Package resolution never accesses the network.

## Prior Art

- **Cargo (Rust)** — SemVer range syntax, preferring highest compatible. Triet follows closely but is stricter regarding iface_hash.
- **Maven (Java)** — complex version conflict resolution. Triet avoids this — single version per package, hash prevails.
- **Swift Package Manager** — semver triple + hash pinning. Triet's design is very similar.
- **Go modules** — Minimal Version Selection (MVS) + sum.db. Triet does not adopt MVS — explicit ranges are more appropriate for systems code.
- **npm** — dependency range hell. Anti-prior-art.

## References

- [VISION §3.3 — Stable ABI: refuse-to-link policy](../../VISION.md)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (this ADR is per phase gate)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (semver triple field)
- [ADR-0012 — Witness table dispatch](0012-witness-table-dispatch.md) (E2330)
- [Semver 2.0.0 specification](https://semver.org/spec/v2.0.0.html)
