# ADR Index

Architectural Decision Records for Triết. Each ADR captures one
significant choice — *why* this design over alternatives — so future
readers (including AI assistants) can reconstruct the reasoning
without spelunking through git history.

ADRs are immutable once "Quyết định" status is reached. To change a
prior decision, write a new ADR that supersedes it.

## By phase

### Pre-v0.2 (language semantics groundwork)

| ADR | Title | Status |
|---|---|---|
| [0001](0001-nullable-memory-layout.md) | Nullable memory layout (`T?` discriminator) | Locked |
| [0002](0002-fstring-format-spec.md) | F-string format spec | Locked |
| [0003](0003-iterator-protocol.md) | Iterator protocol | Locked |
| [0004](0004-multiline-string-indent.md) | Multi-line string indentation | Locked |

### v0.2.x — Module system

| ADR | Title | Status |
|---|---|---|
| [0005](0005-module-system.md) | Module system (Java JPMS aesthetic, dot paths, Python imports) | Locked |
| [0006](0006-ternary-packaging-vision.md) | Ternary packaging vision (informational, points at v0.4+) | Informational |

### v0.3 — Bytecode VM + Stable IR

| ADR | Title | Status |
|---|---|---|
| [0007](0007-ir-design.md) | IR design (register-based SSA) | Locked |
| [0008](0008-triv-binary-format.md) | `.triv` bytecode binary format (currently v3 after ADR-0010 + ADR-0012 bumps) | Locked |

### v0.3.x.cleanup — Gate-closing phase

| ADR | Title | Status |
|---|---|---|
| [0009](0009-version-gate-policy.md) | Version gate policy (4-gate matrix applied to every version bump) | Locked |

### v0.3.x.ternary — Ternary-native IR refactor

| ADR | Title | Status |
|---|---|---|
| [0010](0010-ternary-native-ir.md) | Ternary-native IR (`BrTrilean` 3-way branch, strict `if` Unknown→panic, Ł3-aware `Eq`/`Ne`) | Locked |

### v0.4 — Crate-Pack + Stable ABI

| ADR | Title | Status |
|---|---|---|
| [0011](0011-abi-metadata-format.md) | ABI metadata format (BLAKE3, two-level hash, canonical encoding) | Locked |
| [0012](0012-witness-table-dispatch.md) | Witness table dispatch (Swift-style, hybrid intra/inter-package) | Locked |
| [0013](0013-semver-linking-policy.md) | Semver linking policy (E2300–E2399, `iface_hash` is final arbiter) | Locked |

### v0.5 — CAS Packaging

| ADR | Title | Status |
|---|---|---|
| [0014](0014-hash-scheme-refinement.md) | Hash scheme refinement (3-cấp hash tree: term + module + package, `abi_version` 1 → 2) | Locked |
| [0015](0015-package-store-layout.md) | Package store layout (`~/.triet/store/`, atomic install, mark-and-sweep GC) — *+ v0.5.x.review Addendum: resolver origin 3-state + GC conservative-on-corruption* | Locked |

## How to read an ADR

Every ADR follows the same shape:

1. **Trạng thái** — Locked / Informational / Superseded.
2. **Issue** — what problem forced the decision now.
3. **Quyết định** — the actual choice, in compact form.
4. **Hệ quả** — what becomes possible / constrained / costly.
5. **Không làm** — alternatives explicitly rejected and why.
6. **Prior art** — what we copied vs. invented.
7. **Tham chiếu** — links to SPEC sections, sibling ADRs, external papers.

Search tip: `grep -rn "Quyết định" docs/decisions/` lists every
decision summary in <100 lines total.

## How to write a new ADR

1. Pick the next number (`ls docs/decisions/ | tail -3`).
2. Copy the structure from a recent locked ADR (e.g. ADR-0011).
3. State the issue as a question; let the decision be the answer.
4. List alternatives in "Không làm" — silent rejection is worse
   than explicit rejection because it leaves no record.
5. Commit with `docs(<phase>): ADR-NNNN — <title>` to keep the git
   log scannable.
