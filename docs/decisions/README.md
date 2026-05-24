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
| [0010](0010-ternary-native-ir.md) | Ternary-native IR (`BrTrilean` 3-way branch, strict `if` Unknown→panic, Ł3-aware `Eq`/`Ne`) — *+ v0.7.4.3-error Addendum: null→~0 unification; + v0.7.4.3-error.3c Addendum §C: BrTrilean unknown_block demoted to defense-in-depth (primary safety moved to typecheck E1033 per ADR-0021); + v0.7.4.3-error.6a Addendum §D: outcome-null runtime unification (lowerer emits Constant::Null for `~0`; OutcomeDiscriminant + NullCheck become cross-tolerant)* | Locked |

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

### v0.6 — Capability System

| ADR | Title | Status |
|---|---|---|
| [0016](0016-capability-type-system.md) | Capability type system (namespace + manifest, Trit-level grant/deny/ambient + Trilean::Unknown defer, `triet::capability::E22XX`) | Locked |
| [0017](0017-trilean-policy-hook.md) | Trilean policy hook protocol (`triet.policy` rules + TTY prompt fallback, per-session cache, E2205 sub-variants) — *+ Addendum: parser strictness + `/dev/tty` source + Abstain errata* | Locked |
| [0018](0018-capability-loader-semantics.md) | Capability loader semantics (`triet.package` source manifest, eager link-time check, TTY provenance display, E2208 sub-variants, `CapabilityClaim` Rust struct) — *+ v0.6.x.review Addendum: monotonicity-under-mutation, policy round-trip, requester sort, strict_parser positional contracts* | Locked |

### v0.7 — Self-hosting Compiler

| ADR | Title | Status |
|---|---|---|
| [0019](0019-self-hosting-compiler-bootstrap.md) | Self-hosting compiler bootstrap (3-stage chain, bottom-up incremental component order, canonical emission invariants, full `.khi` byte-identical gate, Rust-shim builtin stdlib, 3-layer testing, perf parity gate deferred to v0.9) — *+ v0.7.3 Addendum: collection TypeTags first-class, .triv v3 → v4 patch bump, Vector rename (Java naming), drop duplicate builtin IDs 23/26, sub-task split v0.7.3.1–4; + v0.7.4 Addendum: generic function syntax type-erased, stdlib stubs Java-aesthetic, interpreter parity deferred* | Locked |
| [0020](0020-outcome-error-handling.md) | Outcome error handling (`T~E` 2-state / `T?~E` 3-state — trit-encoded fallibility, `~+`/`~0`/`~-` constructors, `~?` propagate + `~:` default operators, verbose `.unwrap_value(message)` / `.unwrap_error(message)` methods, `.triv` v4 → v5 patch bump, Trit::Zero reserved for v0.8 async pending, std.result coexistence) | Locked |
| [0021](0021-trilean-refinement.md) | Compile-time `Trilean!` refinement for strict `if` (typecheck-only single-bit refinement on `Type::Trilean`, E1033 `PossiblyUnknownCondition` for plain `if` on possibly-Unknown cond, E1034 `TrileanReturnNotRefined`, `.assume_known(message)` returns `Trilean!`, no IR / wire-format / VM changes — refinement erased at lowering, aligns implementation with SPEC §7.1.1) | Locked |
| [0023](0023-lowerer-ssa-struct-tracking.md) | Lowerer SSA struct-tracking — unified `ValueKind` enum (Struct / Outcome / Nullable / Other) replaces 4 ad-hoc HashMap tracking patterns + ~13 per-construct propagation rules with a single `value_kinds: HashMap<ValueId, ValueKind>` + one recursive `type_expr_to_value_kind` helper. Lowerer crate only — no wire format / VM / typecheck / SPEC impact. Closes v0.7 review finding "patch-stack pattern violating VISION §6 *Refuse over guess*". | Locked |
| [0024](0024-khi-dao-identity-naming.md) | Khí + Đạo identity naming (Đạo Đức Kinh §28 phác tán tắc vi khí + §42 tam sinh vạn vật) — rename 5 Rust-inherited user-facing surface terms: path keyword `crate` → `khi`, compiled artifact `.khi` → `.khi`, CLI binary `triet` → `dao`, manifest `triet.package` → `dao.package`, lockfile `triet.lock` → `dao.lock`. Source `.tri` + IR `.triv` + language name "Triết" giữ. Hard cutover, 5-stage commit series ship trước v0.7.10 mở. Justify Vietnamese-rooted philosophical depth + direct ternary tie via Đạo §42. | Locked |

### Future research (post-v0.7)

ADRs in this bucket capture exploratory design directions — recorded so the context isn't lost, but **explicitly NOT locked**. They will be revisited only after the v0.7 self-hosting compiler ships. Each carries a "deferred research" status and lists the open questions that must be answered before promotion to Locked.

| ADR | Title | Status |
|---|---|---|
| [0022](0022-trit-balanced-ownership.md) | Trit-balanced ownership — polarity-typed references (`~+ T` strong / `~- T` weak), cycle-balance conservation law (cycle sum = 0 at compile time), candidate 4th memory-management mechanism alongside GC / Rc-Weak / Manual. Origin: author's intuition that `+1`/`0`/`-1` can encode ownership direction. Research window: post-v0.7.13. | Draft (exploratory) |

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
