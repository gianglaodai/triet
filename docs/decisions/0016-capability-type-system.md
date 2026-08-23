# ADR 0016 — Capability type system (Trit-level grant/deny/ambient + Ł3 Unknown)

**Status:** Decided. Applies to v0.6 Capability System and all tools reading/writing the `caps section` in ABI metadata from v0.6 onwards. Completes the slot placeholder from [ADR-0011 §5](0011-abi-metadata-format.md); **no bump** to `abi_version` (remains `v=2` per [ADR-0014](0014-hash-scheme-refinement.md)). No change to IR shape (reuses the existing namespace tag from [ADR-0007 §3.4 + §6.7](0007-ir-design.md)).

**Issue:** [VISION §3.5](../../VISION.md) establishes the 5th pillar — *"OS-Native Capability Namespaces"* — with 2 of the 3 Triet identities ([VISION §5](../../VISION.md)):

1. **Trit-level capability** — `Trit ∈ {-1, 0, +1}` is a level, not a boolean `enum { Allow, Deny }`.
2. **Łukasiewicz capability checking** — `Trilean::Unknown` defers to runtime, removing the need for a bolt-on policy engine.

v0.6 must define the technical shape. This ADR must resolve three questions:

1. **Where do capabilities live?** Runtime value (token argument) / function-level annotation / namespace attribute?
2. **Granularity** — package / module / function?
3. **Encoding** — the `caps section` slot ([ADR-0011 §5](0011-abi-metadata-format.md)) must finalize the binary format.

The decisions here are **prerequisites** for [ADR-0017](0017-trilean-policy-hook.md) (runtime policy protocol — TBD) and [ADR-0018](0018-capability-loader-semantics.md) (loader refuse-to-load — TBD). All three ADRs are part of the v0.6 phase.

## Decision

### 1. Capability lives in **namespace + manifest**, not runtime value

A capability is an **attribute of the module path** (`AbsolutePath` from [triet-modules](../../crates/triet-modules)), declared in the `.khi` manifest of the package, rather than a runtime value passed via function arguments or a per-function annotation.

```triet
// CLEAN source code — no token threading, no effect annotation:
function main() {
    let content = sys.io.read_file("/etc/hosts")
    let buf = dev.disk.raw_read(0x1000, 512)
}
```

```text
// Manifest (.tripack caps section) is the declaration site:
package myapp 0.1.0
requires:
    sys.io       = +1   // Grant — explicit
    dev.disk     = -1   // Deny  — refused even if transitively requested
    sys.net.dns  = unknown   // Defer to runtime policy hook (Ł3)
    usr.somelib  = +1   // Cross-application boundary
```

Three alternatives were rejected:

| Rejected | Reason (anchored to documentation) |
|---|---|
| **A — Capability token (Pony/Roc)** | `caps section` ADR-0011 §5 would become a dead slot. Every function touching a syscall would require a `Capability<X>` argument $\rightarrow$ verbose, violates "AI-first stays" ([VISION §6](../../VISION.md)) due to code bloat. |
| **B — Effect annotation (Koka/F\*/checked exception)** | Effects propagate per-function $\rightarrow$ `iface_hash` changes every time a function in the call chain is touched $\rightarrow$ breaks compile-time scaling ([ADR-0014 §1](0014-hash-scheme-refinement.md). Java's checked exceptions demonstrated ergonomic failure. |
| **C — Namespace + manifest** | ✅ Selected. Rationale provided in §10 (Consequences). |

### 2. Granularity — module level, no wildcards, no function level

Each entry in the `caps section` locks **one `AbsolutePath` module**. The following are not supported:

- **Wildcards** (`sys.* = +1`): violates "Explicit > implicit" ([VISION §6](../../VISION.md)). To include 3 modules under `sys.*`, they must be listed as 3 separate entries.
- **Path inheritance** (`sys.io = +1` does not automatically grant `sys.io.async`): each module path is an independent declaration. The linker does not infer parent-child relationships.
- **Function-level** (`sys.io.read_file = +1` but `sys.io.write_file = -1`): deferred until post-v1.0. If separation is required, the stdlib author must split the modules (`sys.io.read` vs `DISCARD `sys.io.write`).

**Rationale for module-level:**
- Aligns with [ADR-0007 §6.7](0007-ir-design.md): IR cross-module calls carry the `AbsolutePath` at the module scope, not the function scope. Capability checks read the same metadata — zero IR change.
- Matches VISION §3.5 wording: the pillar is *"capability NAMESPACES"*, not capability functions.
- Matches the Android `<uses-permission>` mental model: apps declare permissions per resource, not per API call.

### 3. Capability level — `Trit` + `Trilean::Unknown`

Each entry in the `caps section` carries a **CapabilityLevel** with 4 states — following the syntax of [VISION §3.5.1](../../VISION.md) (3 Trit values) **combined** with [VISION §3.5.2](../../VISION.md) (Trilean defer):

| Source Value | Name | Compile-time Meaning | Link-time Meaning |
|---|---|---|---|
| `Trit::Positive` (`+1`) | **Grant** | Explicitly granted | Namespace importer passes check |
| `Trit::Zero` (`0`) | **Ambient** | "I do not decide — inherit from caller" | Root package: ambient $\equiv$ deny (no caller). Non-root: overridden by the root package's declaration. |
| `Trit::Negative` (`-1`) | **Deny** | Explicitly forbidden | Refuse-to-link; **deny always wins** for the same path (refuse over guess) |
| `Trilean::Unknown` | **Defer** | "Consult runtime policy" | Loader hook called at runtime, returns `Trit` $\rightarrow$ cached in session. Specific protocol: ADR-0017. |

**Reference source syntax** (manifest parsing is not yet finalized — ADR-0018 will finalize specific syntax):

```text
sys.io        = +1            // or: grant
dev.disk      = -1            // or: deny
core.fs       =  0            // or: ambient
sys.net.dns   = unknown       // or: defer
```

### 4. ABI encoding — finalize `caps section` (ADR-0011 §5)

The v0.6 `caps section` extends the placeholder from [ADR-0011 §5](0011-abi-metadata-format.md). The format is binary, canonical, and sorted by `namespace_path`:

```
cap_count: varint
for each cap entry:
    namespace_path: length-preposed UTF-8    // AbsolutePath of module, e.g. "sys.io"
                                              //   - root MUST be one of: sys, dev, usr
                                              //   - std, core, crate, self, super $\rightarrow$ refused (E2206 — invalid capability namespace)
    level: u8                                 // 0x00 = Deny    (Trit::Negative)
                                              //   0x01 = Ambient (Trit::Zero)
                                              //   0x02 = Grant   (Trit::Positive)
                                              //   0x03 = Defer   (Trilean::Unknown)
                                              //   0x04..0xFF: refused (E2207 — invalid level encoding)
    reserved: u8                              // 0x00 in v0.6. Future use: per-cap policy ID, witness ref...
```

**Canonical rules** (strengthening [ADR-0011 §6](0011-abi-metadata-format.md) + [ADR-0014 §7](0014-hash-scheme-refinement.md) for package level):

- Entries are sorted lexicographically by `namespace_path` bytes.
- Duplicate paths $\rightarrow$ parse error (E2204 — duplicate cap declaration). There is NO merge/last-wins rule.
- Empty section (`cap_count = 0`) = package requests no capabilities $\rightarrow$ still valid (e.g., a leaf library containing only pure logic). The package-level `iface_hash` still includes the trailing `cap_count = 0` bytes — ensuring hash stability.

The `abi_version` **remains `v=2`** ([ADR-0014 §5](0014-hash-scheme-refinement.md)) — this is a population of a reserved slot, not an additive field change. Pre-v0.6 readers (unaware of cap semantics) can still parse the bytes (`cap_count + entries`) but must refuse-to-link if `cap_count > 0` with error E2208 (capability section present but reader is pre-v0.6) — specific implementation in ADR-0018.

### 5. Compile-time enforcement rules

The type checker adds a new pass after name resolution ([triet-modules](../../crates/triet-modules)): **capability check**. This pass reads:

- Imports of the current package (`from sys.io import read_file` $\rightarrow$ required path `sys.io`).
- The `caps section` of the current package (its own claims regarding what it needs).
- The root package's effective grants (applies only during linking — compile-time only verifies self-claim consistency).

Rules:

1. **Mandatory self-declaration:** Every import from `sys.*`/`dev.*`/`usr.<other>` must have a corresponding entry in the current package's `caps section`. Missing $\rightarrow$ **E2200 `MissingCapabilityClaim`**.
2. **Self-contradictory deny:** If a package imports `sys.io.read_file` but its `caps section` declares `sys.io = -1` $\rightarrow$ **E2201 `SelfContradictoryCapability`** (refusing what you are currently using).
3. **Namespace root scope:**
   - Imports from the **same root** (intra-`usr` lib-to-lib, intra-`sys` stdlib-to-stdlib) $\rightarrow$ no claim required (cap-check only applies to cross-root boundaries).
   - Imports from **`std.*` or `core.*`** $\rightarrow$ **ambient, no claim required** (per [VISION §3.5](../../VISION.md): *"std is ambient by default"*; `core.*` contains foundational types — Trit/Tryte/Integer/Long/Trilean).
   - Imports from `crate.*` / `self.*` / `super.*` $\rightarrow$ intra-package, no capability concept.
4. **Path validity:**
   - Cap entry with root $\notin$ {sys, dev, usr} $\rightarrow$ **E2206 `InvalidCapabilityRoot`**.
   - Path does not exist in any dependency's exports $\rightarrow$ **E2202 `UnresolvedCapabilityPath`** (detection deferred to link-time because dependencies are not visible at compile-time).
5. **Conflict resolution at link time** (root package's manifest = authority):
   - For each cap path $P$ in the union of all dependency claims: the root manifest's level for $P$ decides.
   - Root: `+1` $\rightarrow$ pass. `-1` or `0` (ambient at root = no caller = effectively deny) $\rightarrow$ **E2203 `CapabilityRefused`**.
   - Root: `Unknown` $\rightarrow$ defer (load-time policy hook resolves; ADR-0017 defines protocol).
   - Path not in root manifest but claimed by a dependency $\rightarrow$ **E2200 `MissingCapabilityClaim`** at the root.

### 6. Error code namespace — `triet::capability::E22XX`

The v0.6 phase occupies the E2200–E2299 slot, **avoiding collision** with `triet::modules::E21XX` (loader/resolver) or `triet::semver::E23XX` (linker version/iface drift). Separation is necessary because these three error types appear at three different stages (resolve $\rightarrow$ compile $\rightarrow$ link).

Initial assignments (v0.6.1 — ADR-0016):

| Code | Name | Stage | Trigger |
|---|---|---|---|
| `E2200` | `MissingCapabilityClaim` | compile + link | Import uses a capability not declared in the manifest |
| `E2201` | `SelfContradictoryCapability` | compile | Package imports a path that it explicitly denies |
| `E2202` | `UnresolvedCapabilityPath` | link | Cap path does not match any dependency export |
| `E2203` | `CapabilityRefused` | link | Root manifest refuses a capability requested by a dependency |
| `E2204` | `DuplicateCapabilityDecl` | parse | Same path appears $\ge$ 2 times in the `caps section` |
| `Ecap` | `E2205` | reserved for ADR-0017 (policy hook protocol error) | — |
| `E2206` | `InvalidCapabilityRoot` | parse | Root $\notin$ {sys, dev, usr} |
| `E2207` | `InvalidCapabilityLevel` | parse | `level` byte $\notin$ {0x00..0x03} |
| `E2208` | `E2208` | reserved for ADR-0018 (loader refuse-to-load) | — |

E2205 and E2208 are left empty for future ADRs; **no pre-claiming of semantics**.

### 7. Root package = authority. Dependency claims are requests, not decisions.

Enforcing the "Refuse over guess" rule ([VISION §6](../../VISION.md)) at the link level:

- Each dependency's `caps section` is a **claim**: "I need these paths to run." The linker reads this but does not enforce it on the dependency itself.
- The root package's `caps section` is a **decision**: it dictates what is granted and what is denied. The linker enforces root decisions across the entire closure.
- There is **NO "auto-promotion"**: a dependency claiming `sys.io = +1` does not automatically grant `sys.cap = +1` at the root. The root manifest must explicitly grant it.
- There is **NO "implicit union"** (Cargo features pattern): the root manifest's grant set is not automatically expanded by transitive dependency needs.

Ergonomic consequence: adding a new dependency that requests a new capability requires the user to explicitly add a grant to the root manifest. This is a **feature**, not a bug — it ensures the capability surface is always auditable in a single location.

### 8. ResolutionOrigin dispatch (placeholder hook for ADR-0017)

[ADR-0015 Addendum](0015-package-store-layout.md#addendum--v05xreview-pre-v06-audit) added `ResolutionOrigin { Lockfile, IfacePin, Fresh }` for each resolved package. ADR-0017 will define a policy protocol that can dispatch based on origin (e.g., only `Lockfile` origins auto-trust grants for `dev.*`; `Fresh` dependencies must prompt the user). ADR-0016 only commits **that** the dispatch slot exists — `Trilean::Unknown` resolution has the right to inspect the `ResolutionOrigin` of the requesting dependency — it does not commit to the detailed protocol.

## Consequences

### For v0.5 hash scheme ([ADR-0014](0014-hash-scheme-refinement.md))

- The `caps section` has been within the scope of the `iface_hash_pkg` hash since v0.4 (per [ADR-0014 §4](0014-hash-scheme-refinement.md): *"Rollup: BLAKE3(sorted module hashes + deps + caps)"*). v0.6 only populates the slot, **without** changing the rollup. Two packages with different capability claims will result in different `iface_hash` values $\rightarrow$ different CAS addresses $\rightarrow$ naturally avoiding collisions in the store.
- An empty `caps section` (`cap_count = 0`) for pre-v0.6 packages is hash-stable: identical bytes (`varint 0` = 1 byte `0 $\rightarrow$ 0x00`) $\rightarrow$ identical hash. This provides natural backward compatibility.

### For ADR-0007 IR — zero change

[ADR-0007 §6.7](0007-ir-design.md) already preserves the `AbsolutePath` in cross-module calls. The capability check reads the namespace tag from the existing IR. **No change to the `.triv` wire format** (v3 from [ADR-0010](0010-ternary-native-ir.md) + [ADR-0012](0012-witness-table-dispatch.md) remains unchanged).

### For ADR-0011 ABI — populate slot, do not bump

[ADR-0011 §7](0011-abi-metadata-format.md) promised: *"v0.6 only needs to populate, no bump to abi_version"*. ADR-0016 adheres to this. `abi_version = 2` ([ADR-0014 §5](0014-hash-scheme-refinement.md)) covers both v0.5 CAS Packaging and the v0.6 Capability System. Pre-v0.6 readers can still parse the `cap_count` field; they will only refuse if `cap_count > 0` (E2208, ADR-0018 will define details).

### For ADR-0013 linker policy

The E22XX namespace is separate from E23XX. The linker workflow ([ADR-0011 §8](0011-abi-metadata-format.md)) adds a step:

```
1. Open .tripack $\rightarrow$ read ABI metadata.
2. Resolve dependencies.
3. Version check (E23XX) $\rightarrow$ refuse/accept.        $\leftarrow$ ADR-0013
4. Capability check (E22XX) $\rightarrow$ refuse/accept.     $\leftarrow$ ADR-0016 (new)
5. Accept $\rightarrow$ load .triv, build symbol table.
6. Runtime/JIT: witness dispatch + cap defer hook.
```

Step 4 is inserted between steps 3 and 5. Diagnostics will show a diff of the manifest capability entries (miette-style).

### For ADR-0017 (Trilean policy hook) — TBD

ADR-0017 must define:
- The protocol for calling the policy hook when the capability level is `Defer`.
- Caching scope (per-session vs. per-call).
- Return type (`Trit` final answer, or a chained `Trilean` Unknown?).
- Failure mode if the user policy crashes.

ADR-0016 only commits: the hook **exists**, and the hook input includes `(namespace_path, requester_pkg, dep_chain, ResolutionOrigin)`.

### For ADR-0018 (loader semantics) — TBD

ADR-0018 must define:
- The wire-level refuse-to-load behavior when `cap_count > 0` in a pre-v0.6 reader (E2208).
- Manifest source syntax (parsing rules for the user-facing `requires:` block).
- Capability checking at JIT-load-time (v0.9 Cranelift) when a function is lifted across a capability boundary.

### For v0.7 self-hosting compiler

The Triet-written compiler must honor the `caps section` semantics — this is a contract that the bootstrap chain must preserve bit-identically. Self-hosting tests ([ROADMAP §v0.7](../../ROADMAP.md)) must verify that capability enforcement does not depend on the Rust implementation.

### For v0.8 concurrency

[ROADMAP §v0.8](../../ROADMAP.md) hints at *"Actor + structured concurrency"* alignment with capabilities. An Actor's mailbox capability could reuse the namespace mechanism (e.g., `usr.actor.mailbox = +1`). ADR-0016 does not pre-commit to this, but the namespace shape does not preclude v0.8.

## Alternatives Considered

- **Per-function capability** (`sys.io.read_file = +1` while `sys.io.write_file = -int`). Deferred until post-v1.0. Workaround: stdlib authors can split modules if that granularity is required.
- **Wildcard grants** (`sys.* = +1`). Violates "Explicit > implicit". Every module must be explicit. The ergonomic "pain" is intentional — capability audits must be readable linearly within the manifest.
- **Path inheritance** (parent grant covers children). The module path is the leaf identifier at the cap-check level. `sys.io = +1` does not cover `sys.io.async` — it must be declared separately.
- **Implicit union via dependencies** (Cargo features pattern). The root must explicitly grant. Refuse over guess.
- **Auto-shim cap mismatch** ([ADR-0013](0013-semver-linking-policy.md) rejected auto-shim for ABI; ADR-0016 inherits this principle for capabilities). Refuse-to-link, output a miette-friendly diff, and require the user to fix the manifest.
- **Bump `abi_version`** — the slot was already reserved; populating it is a data edit, not a schema change.
- **Hardware enforcement** in v0.6. Requires ternary hardware or a bytecode VM sandbox to fence addresses. VM v0.3 runs in-process Rust without a sandbox — deferred to v0.8+ when concurrency lands.
- **Distributed capability** (cross-machine grant tokens). Local-only in v0.6. Distributed capabilities are deferred to v1.0+ alongside a remote registry.
- **Cross-arch cap** — capability declarations are architecture-independent (definitions are namespace strings, not hardware). Inherits from [ADR-0007 §4](0007-ir-design.md) architecture-independent IR.
- **`std.*` / `core.*` cap enforcement** — ambient, no check. `std.io.println` does not require a grant (similar to `printf` in C — no hardware fence). Future tightening: deferred.
- **Capability runtime hot-reload** — grant set is frozen at link time + Defer resolution occurs at load time. NO dynamic re-granting mid-session (violates capability monotonicity).

## Prior art

- **[Java JPMS](https://openjdk.org/jeps/261)** — `module-info.java` with `requires`/`exports`/`opens`. Borrowed: declarative module-level capability in the manifest, no runtime token. Difference: Triet adds Trit levels (deny + ambient in addition to grant) and Trilean defer.
- **[Android `<uses-permission>`](https://developer.android.com/guide/topics/manifest/uses-permission-element)** — root app manifest declares all permissions; OS enforces at runtime. Borrowed: root manifest = authority; dependency claims are requests. Difference: Triet enforces at compile + link, not just runtime.
- **[Pony object capabilities](https://www.ponylang.io/discover/#object-capabilities)** — capability as a type modifier on object refs (`iso`, `tag`, ...). Rejected: per-object token-passing is too verbose and does not match the namespace mental model.
- **[Genode OS](https://genode.org/documentation/general-overview/index) + [seL4](https://sel4.systems/About/seL4-whitepaper.pdf)** — cap-based microkernel; parent components grant caps to children explicitly. Borrowed: parent (root pkg) is authoritative; refuse-by-default. Difference: Triet is a language-level static check, not a kernel.
- **[E language](http://www.erights.org/)** — object cap with a defer-to-vat mechanism. Borrowed: the Trilean::Unknown defer pattern was inspired by this (vat $\approx$ runtime policy hook).
- **[Mojo capabilities](https://docs.modular.com/mojo/manual/structs/) (status: tentative)** — declared in the roadmap but not yet landed. On the watch list, but not adopted.
- **[Roc platform](https://www.roc-lang.org/platforms)** — platform-injected capabilities. Rejected for similar reasons to Pony: verbose token passing.

**Anti-prior-art:**

- **Java checked exception** — function-level `throws` propagate through the entire call chain $\rightarrow$ community backlash. Triet avoids this by keeping capabilities at the module level, not the function level.
- **POSIX setuid + capabilities(7)** — Linux runtime capability system; prone to numerous CVEs due to the "confused deputy" problem. Triet avoids this via compile + link enforcement; the runtime hook is only for explicit Defer.
- **C++ `friend` keyword** — allows fine-grained leaks. Triet rejects fine-grained capabilities; only explicit module-level is allowed.

## References

- [VISION §3.5 — OS-Native Capability Names/Namespaces](../../VISION.md) — Pillar 5
- [VISION §5 — Triet Identity](../../VISION.md) — Trit-level cap + Łukasiewicz cap check
- [VISION §6 — Design Principles](../../VISION.md) — Refuse over guess, Explicit > implicit
- [SPEC §1.4 — Keywords](../../SPEC.md), [SPEC §10 — Reserved namespace roots](../../SPEC.md)
- [ADR-0005 — Module system](0005-module-system.md) — `AbsolutePath` shape; reserved roots
- [ADR-0006 §3 — Ternary Tree namespace](0006-ternary-packaging-vision.md) — North star: `module sys.io (layer: -1)` syntax direction
- [ADR-0007 §3.4, §6.7 — IR namespace tag preserved](0007-ir-design.md) — zero IR change for cap check
- [ADR-0011 §5, §7 — `caps section` reserved + abi_version policy](0011-abi-metadata-format.md) — slot to populate, no bump
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) — E23XX namespace, refuse-to-link pattern
- [ADR-0014 §4, §5 — Package hash includes caps](0014-hash-scheme-refinement.md) — hash-stable across pre/post v0.6
- [ADR-0015 Addendum — ResolutionOrigin 3-state](0015-package-store-layout.md) — dispatch slot for ADR-0017
- ADR-0017 — Trilean policy hook protocol (TBD, v0.6 phase)
- ADR-0018 — Capability loader semantics (TBD, v0.6 phase)
- [ROADMAP §v0.6 — Capability System](../../ROADMAP.md)
