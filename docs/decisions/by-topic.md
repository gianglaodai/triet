# ADR Index — by Topic

Cross-reference vào 27 ADRs theo **topic cluster** thay vì chronological number. Hữu ích khi câu hỏi là "rule về X ở đâu?" thay vì "ADR-0NNN nói gì?".

> **Note:** ADRs là *immutable historical records* — file content không thay đổi sau khi đạt "Quyết định" status. Index này chỉ point đến chúng, không duplicate. Active language semantics nằm ở [`SPEC.md`](../../SPEC.md).
>
> **Hai axis của index:**
> - [`README.md`](README.md) — chronological (0001 → 0027), phase-grouped. Trace "decision khi nào, trong phase nào".
> - **Đây** ([`by-topic.md`](by-topic.md)) — topic-clustered. Trace "rule về X ở đâu".

---

## 1. Language surface (lexical, literals, syntax niceties)

| ADR | Title | Status |
|---|---|---|
| [0002](0002-fstring-format-spec.md) | F-string format spec — Python-style `f"..."` với `{expr}` interpolation | Locked |
| [0004](0004-multiline-string-indent.md) | Multi-line string indentation — auto-dedent rule | Locked |

> Khác liên quan: [ADR-0005](0005-module-system.md) cho verbose keywords + dot paths; [SPEC §1](../../SPEC.md) cho lexical structure tổng quát.

---

## 2. Type system (types, refinement, outcome, iterator)

| ADR | Title | Status |
|---|---|---|
| [0001](0001-nullable-memory-layout.md) | Nullable memory layout — `T?` discriminator trit-encoded, `Trit::Zero` = null state | Locked |
| [0003](0003-iterator-protocol.md) | Iterator protocol — `Iterator<T>` trait + `.enumerate()` adapter | Locked |
| [0020](0020-outcome-error-handling.md) | Outcome error handling — `T~E` 2-state binary + `T?~E` 3-state ternary, `~+`/`~0`/`~-` constructors, `~?`/`~:` postfix ops | Locked |
| [0021](0021-trilean-refinement.md) | Trilean! refinement — typecheck-only refinement, strict `if cond` requires non-Unknown, E1033/E1034 | Locked |

> Cross-cutting: [ADR-0010](0010-ternary-native-ir.md) cho IR-level Trilean semantics (`BrTrilean`, Ł3-aware `Eq`).

---

## 3. Memory model, Ownership & Concurrency

| ADR | Title | Status |
|---|---|---|
| [0022](0022-trit-balanced-ownership.md) | S6 ownership — 5-form reference family `&+` strong / `&0` neutral / `&-` weak / `&` bare / `owned` transfer; định lý vô-chu-trình; capability-as-unsafe | Locked |
| [0025](0025-borrow-checker-rules.md) | Borrow checker rules — NLL + 3-rule lifetime elision + no-annotation policy; E24XX namespace (E2400 lifetime / E2410 mutability / E2420 move / E2430 namespace / E2440 NLL / E2450+ drop) | Locked |
| [0026](0026-actor-boundary-send-rules.md) | Concurrency Primitives & Send Rules (**BYOS**) — Triết core provides Send rules + Atomic primitives + capability gates, scheduler stdlib hoặc external. Refuse list: `actor`/`spawn`/`receive`/`send`/`async`/`await`. E25XX namespace | Locked v2 |
| [0028](0028-atomic-primitive.md) | Atomic primitive design — refines ADR-0026 v2 §4 placeholder. Rust-shim builtins + AtomicValue marker + 3-level Ordering ↔ Trit mapping + full API surface + `&+ Atomic<T>` interior mutability pattern (fixes v2 §4.3 contradiction) | Locked |
| [0031](0031-borrow-expression-syntax.md) | Borrow expression syntax — closes SPEC §10 v0.7 warning + unblocks ADR-0028 §6 example. Prefix `&FORM operand` (all 5 forms), operand IDENT/field/index, lowerer passthrough (refs erase). Borrow checker enforcement defers v0.9.x.borrow.* | Draft |

> Liên quan: [ADR-0001](0001-nullable-memory-layout.md) cho memory header pattern (Trit discriminator); ObjectHeader memory layout chi tiết ở `triet-core/src/memory.rs`.

---

## 4. Module system & Package distribution

| ADR | Title | Status |
|---|---|---|
| [0005](0005-module-system.md) | Module system — verbose keywords (`function`/`module`/`mutable`/...), dot paths, Python-style imports, 3-level visibility, multi-arena `ResolvedProgram` | Locked |
| [0011](0011-abi-metadata-format.md) | ABI metadata format — BLAKE3 two-level hash (`iface_hash` + `impl_hash`), canonical sort-by-name encoding | Locked |
| [0013](0013-semver-linking-policy.md) | Semver linking policy — E2300-E2399 decision matrix, `iface_hash_pin` là final arbiter, auto-shim NOT promised | Locked |
| [0014](0014-hash-scheme-refinement.md) | Hash scheme refinement — 3-cấp hash tree (term + module + package), `abi_version` 1 → 2 additive | Locked |
| [0015](0015-package-store-layout.md) | Package store layout — `~/.triet/store/`, atomic install (tmp + rename), mark-sweep GC, `dao.lock` hand-rolled line format | Locked |

> Cross-cutting: [ADR-0024](0024-khi-dao-identity-naming.md) đổi `.tri.bin` → `.khi` cho compiled artifact identity.

---

## 5. IR & Wire format

| ADR | Title | Status |
|---|---|---|
| [0007](0007-ir-design.md) | IR design — register-based SSA, vô hạn virtual register, type-tagged per register | Locked |
| [0008](0008-triv-binary-format.md) | `.triv` bytecode binary format — magic bytes + version + section layout + LEB128 varint, currently v5 sau ADR-0010/0012/0020 bumps | Locked |
| [0010](0010-ternary-native-ir.md) | Ternary-native IR — `BrTrilean` 3-way branch, Ł3-aware `Eq`/`Ne` propagate Unknown, `Constant::Null` = Trit::Zero canonical encoding | Locked |
| [0012](0012-witness-table-dispatch.md) | Witness table dispatch — Swift-style, hybrid intra/inter-package (monomorphize intra, witness inter) | Locked |

> Cross-cutting: [ADR-0011](0011-abi-metadata-format.md) cho IR artifact container; [ADR-0023](0023-lowerer-ssa-struct-tracking.md) cho lowerer internals.

---

## 6. Capability system

| ADR | Title | Status |
|---|---|---|
| [0016](0016-capability-type-system.md) | Capability type system — namespace attribute trong `dao.package`, 4-state level (Grant/Ambient/Deny/Defer), wire format reuses caps section, root authority semantics | Locked |
| [0017](0017-trilean-policy-hook.md) | Trilean policy hook — `dao.policy` rules + per-session cache + TTY prompt fallback, E2205 sub-variants, parser strict + `/dev/tty` source + Abstain errata | Locked |
| [0018](0018-capability-loader-semantics.md) | Capability loader semantics — `dao.package` grammar, eager Step 6a refuse at link, TTY provenance prompt, E2208 sub-variants, `CapabilityClaim` Rust struct | Locked |

> Cross-cutting: [VISION §3.5 + §5](../../VISION.md) cho trụ cột bản sắc #5; v0.6 phase ở [ROADMAP](../../ROADMAP.md).

---

## 7. Compiler internals & Self-hosting

| ADR | Title | Status |
|---|---|---|
| [0019](0019-self-hosting-compiler-bootstrap.md) | Self-hosting compiler bootstrap — 3-stage chain (Stage 1 Rust → 2 → 3), bottom-up incremental component order, canonical emission invariants, Rust-shim builtin stdlib, perf gate deferred v0.9 | Locked |
| [0023](0023-lowerer-ssa-struct-tracking.md) | Lowerer SSA struct-tracking — unified `ValueKind` enum (Struct / Outcome / Nullable / Other) replaces 4 ad-hoc HashMap patterns | Locked |
| [0024](0024-khi-dao-identity-naming.md) | Khí + Đạo identity naming — `.tri.bin` → `.khi`, CLI binary `triet` → `dao`, manifest `dao.package`, lockfile `dao.lock`; source `.tri` + IR `.triv` + language name "Triết" giữ nguyên | Locked |
| [0030](0030-jit-cranelift-integration.md) | JIT integration (Cranelift backend) — 3-tier model (Interpreter→VM→JIT), 100-call threshold trigger, register-SSA 1:1 mapping, AOT cache per impl_hash, sync JIT v0.9, no capability gate. Stage 2/3 byte-identical gate lift conditions | Locked |

> Self-host source code: `compiler/` directory (~23K LOC). Cross-cutting: [ADR-0009](0009-version-gate-policy.md) cho gate matrix; [ADR-0027](0027-diagnostic-format-standard.md) cho diagnostic format.

---

## 8. Cross-cutting / Process

| ADR | Title | Status |
|---|---|---|
| [0006](0006-ternary-packaging-vision.md) | Ternary packaging vision (informational, points at v0.4+ work) | Informational |
| [0009](0009-version-gate-policy.md) | Version gate policy — 4-gate matrix (Functional / Hygiene / Docs / Self-consistency) applied to mọi version bump. *+ v0.8.x.cadence-fix Addendum: enforcement automation (release-check.sh + git hooks)* | Locked |
| [0027](0027-diagnostic-format-standard.md) | Diagnostic format standard (AI-first) — header `EXXXX ErrorName` + body + optional span + `[Fix N]` numbered blocks, pure ASCII, no diff `-/+`. Language-wide retroactive scope | Locked |
| [0029](0029-self-host-port-policy.md) | Self-host port policy — Layer A (language surface) mandatory lockstep, Layer B (internal compiler) defer-OK, Layer C (runtime/backend) independent. 3-layer detection (smoke + count-based release-check + TODO checklist). Stage 2/3 byte-identical gate lift chained to JIT (ADR-0030) | Locked |

---

## How to add a new ADR

1. Pick next chronological number (`ls docs/decisions/ | tail -3`).
2. Copy structure từ recent locked ADR (e.g., ADR-0011 hoặc ADR-0022).
3. **Add row to both indexes:**
   - [`README.md`](README.md) — chronological phase section.
   - **This file** — appropriate topic cluster. Nếu chưa fit cluster nào → mở cluster mới.
4. Commit `docs(<phase>): ADR-NNNN — <title>`.
