# ADR Index — by Topic

Cross-reference into 44 ADRs by **topic cluster** rather than chronological number. Useful when the question is "where is the rule about X?" rather than "what did ADR-0NNN say?". (0001-0036: legacy compiler; 0037-0044: rewrite backend — see §7b.)

> **Author & Architect:** Giang Hoang ([@gianglaodai](https://github.com/gianglaodai))  
> **Note:** ADRs are *immutable historical records* — file content does not change after reaching "Decision" status. This index merely points to them without duplication. Active language semantics reside in [`SPEC.md`](../../SPEC.md).
>
> **Two axes of the index:**
> - [`README.md`](README.md) — chronological (0001 → 0036), phase-grouped. Traces "when was a decision made, in which phase".
> - **This document** ([`by-topic.md`](by-topic.md)) — topic-clustered. Traces "where is the rule about X".

---

## 1. Language surface (lexical, literals, syntax niceties)

| ADR | Title | Status |
|---|---|---|
| [0002](0002-fstring-format-spec.md) | F-string format spec — Python-style `f"..."` with `{expr}` interpolation | Locked |
| [0004](0004-multiline-string-indent.md) | Multi-line string indentation — auto-dedent rule | Locked |

> Related: [ADR-0005](0005-module-system.md) for verbose keywords + dot paths; [SPEC §1](../../SPEC.md) for general lexical structure.

---

## 2. Type system (types, refinement, outcome, iterator)

| ADR | Title | Status |
|---|---|---|
| [0001](0001-nullable-memory-layout.md) | Nullable memory layout — `T?` discriminator trit-encoded, `Trit::Zero` = null state | Locked |
| [0003](0003-iterator-protocol.md) | Iterator protocol — `Iterator<T>` trait + `.enumerate()` adapter | Locked |
| [0020](0020-outcome-error-handling.md) | Outcome error handling — `T~E` 2-state binary + `T?~E` 3-state ternary, `~+`/`~0`/`~-` constructors, `~?`/`~:` postfix ops | Locked |
| [0021](0021-trilean-refinement.md) | Trilean! refinement — typecheck-only refinement, strict `if cond` requires non-Unknown, E1033/E1034 | Locked |
| [0036](0036-typetag-opaque-aggregate.md) | `TypeTag::Opaque` — user-aggregate disambiguation from `Unit` (disc 12, .triv version 7 → 8, self-host lockstep, resolves Unit ambiguity to unblock 410 cross-mode tier-downs) | Locked |

> Cross-cutting: [ADR-0010](0010-ternary-native-ir.md) for IR-level Trilean semantics (`BrTrilean`, Ł3-aware `Eq`).

---

## 3. Memory model, Ownership & Concurrency

| ADR | Title | Status |
|---|---|---|
| [0022](0022-trit-balanced-ownership.md) | S6 ownership — 5-form reference family `&+` strong / `&0` neutral / `&-` weak / `&` bare / `owned` transfer; acyclicity theorem; capability-as-unsafe | Locked |
| [0025](0025-borrow-checker-rules.md) | Borrow checker rules — NLL + 3-rule lifetime elision + no-annotation policy; E24XX namespace (E2400 lifetime / E2410 mutability / E2420 move / E2430 namespace / E2440 NLL / E2450+ drop) | Locked |
| [0026](0026-actor-boundary-send-rules.md) | Concurrency Primitives & Send Rules (**BYOS**) — Triet core provides Send rules + Atomic primitives + capability gates, scheduler in stdlib or external. Refuse list: `actor`/`spawn`/`receive`/`send`/`async`/`await`. E25XX namespace | Locked v2 |
| [0028](0028-atomic-primitive.md) | Atomic primitive design — refines ADR-0026 v2 §4 placeholder. Rust-shim builtins + AtomicValue marker + 3-level Ordering ↔ Trit mapping + full API surface + `&+ Atomic<T>` interior mutability pattern (fixes v2 §4.3 contradiction) | Locked |
| [0031](0031-borrow-expression-syntax.md) | Borrow expression syntax — closes SPEC §10 v0.7 warning + unblocks ADR-0028 §6 example. Prefix `&FORM operand` (5 forms total — no bare `&`), operand IDENT + field-access (index/compound deferred §10.3 backlog), lowerer passthrough. **Option A:** E2420 UseAfterMove ships v0.9 (.7d); NLL + lifetime elision + `&-` upgrade deferred to v0.10 per §10.1 | Locked |

> Related: [ADR-0001](0001-nullable-memory-layout.md) for memory header pattern (Trit discriminator); ObjectHeader memory layout details in `triet-core/src/memory.rs`.

---

## 4. Module system & Package distribution

| ADR | Title | Status |
|---|---|---|
| [0005](0005-module-system.md) | Module system — verbose keywords (`function`/`module`/`mutable`/...), dot paths, Python-style imports, 3-level visibility, multi-arena `ResolvedProgram` | Locked |
| [0011](0011-abi-metadata-format.md) | ABI metadata format — BLAKE3 two-level hash (`iface_hash` + `impl_hash`), canonical sort-by-name encoding | Locked |
| [0013](0013-semver-linking-policy.md) | Semver linking policy — E2300-E2399 decision matrix, `iface_hash_pin` is final arbiter, auto-shim NOT promised | Locked |
| [0014](0014-hash-scheme-refinement.md) | Hash scheme refinement — 3-level hash tree (term + module + package), `abi_version` 1 → 2 additive | Locked |
| [0015](0015-package-store-layout.md) | Package store layout — `~/.triet/store/`, atomic install (tmp + rename), mark-sweep GC, `dao.lock` hand-rolled line format | Locked |

> Cross-cutting: [ADR-0024](0024-khi-dao-identity-naming.md) changes `.tri.bin` → `.khi` for compiled artifact identity. [ADR-0033](0033-aot-cache-cranelift-object.md) adds `jit/{triple}/{impl_hash}/` subtree to store with GC integration (v0.10 AOT cache).

---

## 5. IR & Wire format

| ADR | Title | Status |
|---|---|---|
| [0007](0007-ir-design.md) | IR design — register-based SSA, infinite virtual registers, type-tagged per register | Locked |
| [0008](0008-triv-binary-format.md) | `.triv` bytecode binary format — magic bytes + version + section layout + LEB128 varint, currently v5 after ADR-0010/0012/0020 bumps | Locked |
| [0010](0010-ternary-native-ir.md) | Ternary-native IR — `BrTrilean` 3-way branch, Ł3-aware `Eq`/`Ne` propagate Unknown, `Constant::Null` = Trit::Zero canonical encoding | Locked |
| [0012](0012-witness-table-dispatch.md) | Witness table dispatch — Swift-style, hybrid intra/inter-package (monomorphize intra, witness inter) | Locked |
| [0036](0036-typetag-opaque-aggregate.md) | `TypeTag::Opaque` — user-aggregate disambiguation from `Unit` (disc 12, .triv version 7 → 8, self-host lockstep, resolves Unit ambiguity to unblock 410 cross-mode tier-downs) | Locked |

> Cross-cutting: [ADR-0011](0011-abi-metadata-format.md) for IR artifact container; [ADR-0023](0023-lowerer-ssa-struct-tracking.md) for lowerer internals.

---

## 6. Capability system

| ADR | Title | Status |
|---|---|---|
| [0016](0016-capability-type-system.md) | Capability type system — namespace attribute in `dao.package`, 4-state level (Grant/Ambient/Deny/Defer), wire format reuses caps section, root authority semantics | Locked |
| [0017](0017-trilean-policy-hook.md) | Trilean policy hook — `dao.policy` rules + per-session cache + TTY prompt fallback, E2205 sub-variants, parser strict + `/dev/tty` source + Abstain errata | Locked |
| [0018](0018-capability-loader-semantics.md) | Capability loader semantics — `dao.package` grammar, eager Step 6a refuse at link, TTY provenance prompt, E2208 sub-variants, `CapabilityClaim` Rust struct | Locked |

> Cross-cutting: [VISION §3.5 + §5](../../VISION.md) for identity pillar #5; v0.6 phase in [ROADMAP](../../ROADMAP.md).

---

## 7. Compiler internals & Self-hosting

| ADR | Title | Status |
|---|---|---|
| [0019](0019-self-hosting-compiler-bootstrap.md) | Self-hosting compiler bootstrap — 3-stage chain (Stage 1 Rust → 2 → 3), bottom-up incremental component order, canonical emission invariants, Rust-shim builtin stdlib, perf gate deferred to v0.9 | Locked |
| [0023](0023-lowerer-ssa-struct-tracking.md) | Lowerer SSA struct-tracking — unified `ValueKind` enum (Struct / Outcome / Nullable / Other) replaces 4 ad-hoc HashMap patterns | Locked |
| [0024](0024-khi-dao-identity-naming.md) | Khi + Dao identity naming — `.tri.bin` → `.khi`, CLI binary `triet` → `dao`, manifest `dao.package`, lockfile `dao.lock`; source `.tri` + IR `.triv` + language name "Triet" retained | Locked |
| [0030](0030-jit-cranelift-integration.md) | JIT integration (Cranelift backend) — 3-tier model (Interpreter→VM→JIT), 100-call threshold trigger, register-SSA 1:1 mapping, AOT cache per impl_hash, sync JIT v0.9, no capability gate. Stage 2/3 byte-identical gate lift conditions | Locked |
| [0032](0032-builtin-shim-abi.md) | Builtin shim ABI — refines ADR-0030 §12 backlog. Hybrid `RuntimeValue` ABI (primitives unboxed, composites Rc-boxed). `Rc::into_raw` + `__triet_drop_arc` at SSA last-use per ADR-0023 ValueKind. Capability gate compile-time hoist (inherits ADR-0017 program-load invariant). `extern "C-unwind"` + TLS error context + dispatcher `catch_unwind`. `unsafe_code = "deny"` ONLY in `triet-jit` crate. Static `SHIM_TABLE` + `__triet_*` symbol prefix. 3-layer test gates (framework smoke + 43-builtin parity + ABI proptest). Unblocks v0.10.x.jit.1+.2 | Locked |
| [0033](0033-aot-cache-cranelift-object.md) | AOT cache via `cranelift-object` — refines ADR-0030 §13 backlog. Backend hybrid (`cranelift-jit` Path B fresh compile + `cranelift-object` Path A persistence). Version pinning manifest (`cranelift_version` + `shim_abi_version` + `target_triple`) — mismatch silent-fallback Path B. Symbol resolution via direct `SHIM_TABLE`/`LIBCALL_TABLE` lookup (NOT `dlsym`) — reuses ADR-0032 §6. GC integration: `jit/{triple}/{impl_hash}/` swept against `live_mods`; new `GcReport.swept_jit_dirs`; conservative-on-corruption uniform. Per-triple path separation. Determinism preserved (cache state runtime-only). Synchronous atomic-install on Path B success. Silent-fallback corruption recovery. Unblocks v0.10.x.jit.3 + chained .4 bootstrap gate lift | Locked |

> Self-host source code: `compiler/` directory (~23K LOC). Cross-cutting: [ADR-0009](0009-version-gate-policy.md) for gate matrix; [ADR-0027](0027-diagnostic-format-standard.md) for diagnostic format.

---

## 7b. Rewrite backend (2026-06-04+ — NEW compiler, all dual-signed O+G)

| ADR | Title | Status |
|---|---|---|
| [0037](0037-enum-layout.md) | Enum tagged-union layout — discriminant + payload StackSlot (Tier A) | Locked |
| [0038](0038-comparable-trait-deferred.md) | Comparable trait — `compare() -> Trit` locked, deferred pending Trait system | Deferred-locked |
| [0039](0039-nullable-operator-family.md) | Nullable operator family — `?+>` map/flatMap auto-flatten, `?0>` forbidden, `?->` E1041 | Locked (defer impl) |
| [0040](0040-heap-aggregate-layout.md) | Heap aggregate layout — String/Vector shims, ObjectHeader, M1-M4 zeroing-on-move | Locked |
| [0041](0041-nullable-representation-bac-a.md) | Nullable `T?` repr — Option 3c uniform `i64::MIN` sentinel, trap-on-0, Elvis; §12: match 2-arm + E1035/E1026 | Locked |
| [0042](0042-ownership-across-boundary.md) | Ownership across boundary (B7-lift) — move-only, `Deinit` tombstone, borrowck M3+ `CallTarget::Jit` | Locked |
| [0043](0043-hashmap-representation.md) | HashMap repr & shims — open addressing 24B slot, insert-or-update, D2 reject-MIN | Locked |
| [0044](0044-arithmetic-range-enforcement.md) | Arithmetic range enforcement — trap-on-overflow (2 signal families: JIT SIGILL / shim SIGABRT), smulhi Mul carrier, E1036; §5 Pow addendum | Locked |

> Debt registry lives in `TODO.md` (D1-D3 + F6). Test infra: N7 subprocess
> (`spawn_n7_child --exact`), `scripts/gate.sh`.

---

## 8. Cross-cutting / Process

| ADR | Title | Status |
|---|---|---|
| [0006](0006-ternary-packaging-vision.md) | Ternary packaging vision (informational, points at v0.4+ work) | Informational |
| [0009](0009-version-gate-policy.md) | Version gate policy — 4-gate matrix (Functional / Hygiene / Docs / Self-consistency) applied to every version bump. *+ v0.8.x.cadence-fix Addendum: enforcement automation (release-check.sh + git hooks)* | Locked |
| [0027](0027-diagnostic-format-standard.md) | Diagnostic format standard (AI-first) — header `EXXXX ErrorName` + body + optional span + `[Fix N]` numbered blocks, pure ASCII, no diff `-/+`. Language-wide retroactive scope | Locked |
| [0029](0029-self-host-port-policy.md) | Self-host port policy — Layer A (language surface) mandatory lockstep, Layer B (internal compiler) defer-OK, Layer C (runtime/backend) independent. 3-layer detection (smoke + count-based release-check + TODO checklist). Stage 2/3 byte-identical gate lift chained to JIT (ADR-0030) | Locked |

---

## How to add a new ADR

1. Pick next chronological number (`ls docs/decisions/ | tail -3`).
2. Copy structure from a recent locked ADR (e.g., ADR-0011 or ADR-0022).
3. **Add row to both indexes:**
   - [`README.md`](README.md) — chronological phase section.
   - **This file** — appropriate topic cluster. If it does not fit any cluster → create a new cluster.
4. Commit `docs(<phase>): ADR-NNNN — <title>`.
