# ADR Index — chronological

Architectural Decision Records for Triết. Each ADR captures one
significant choice — *why* this design over alternatives — so future
readers (including AI assistants) can reconstruct the reasoning
without spelunking through git history.

ADRs are immutable once "Quyết định" status is reached. To change a
prior decision, write a new ADR that supersedes it.

> **Looking for a rule on X?** Use [`by-topic.md`](by-topic.md) — same
> 29 ADRs grouped by topic (language surface, type system, ownership,
> module/package, IR/wire format, capability, compiler internals,
> cross-cutting). This file ordered chronologically by phase.

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
| [0008](0008-triv-binary-format.md) | `.triv` bytecode binary format (currently v5 after ADR-0010 / 0012 / 0020 bumps) | Locked |

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
| [0017](0017-trilean-policy-hook.md) | Trilean policy hook protocol (`dao.policy` rules + TTY prompt fallback, per-session cache, E2205 sub-variants) — *+ Addendum: parser strictness + `/dev/tty` source + Abstain errata* | Locked |
| [0018](0018-capability-loader-semantics.md) | Capability loader semantics (`dao.package` source manifest, eager link-time check, TTY provenance display, E2208 sub-variants, `CapabilityClaim` Rust struct) — *+ v0.6.x.review Addendum: monotonicity-under-mutation, policy round-trip, requester sort, strict_parser positional contracts* | Locked |

### v0.7 — Self-hosting Compiler

| ADR | Title | Status |
|---|---|---|
| [0019](0019-self-hosting-compiler-bootstrap.md) | Self-hosting compiler bootstrap (3-stage chain, bottom-up incremental component order, canonical emission invariants, full `.khi` byte-identical gate, Rust-shim builtin stdlib, 3-layer testing, perf parity gate deferred to v0.9) — *+ v0.7.3 Addendum: collection TypeTags first-class, .triv v3 → v4 patch bump, Vector rename (Java naming), drop duplicate builtin IDs 23/26, sub-task split v0.7.3.1–4; + v0.7.4 Addendum: generic function syntax type-erased, stdlib stubs Java-aesthetic, interpreter parity deferred* | Locked |
| [0020](0020-outcome-error-handling.md) | Outcome error handling (`T~E` 2-state / `T?~E` 3-state — trit-encoded fallibility, `~+`/`~0`/`~-` constructors, `~?` propagate + `~:` default operators, verbose `.unwrap_value(message)` / `.unwrap_error(message)` methods, `.triv` v4 → v5 patch bump, Trit::Zero reserved for v0.8 async pending, std.result coexistence) | Locked |
| [0021](0021-trilean-refinement.md) | Compile-time `Trilean!` refinement for strict `if` (typecheck-only single-bit refinement on `Type::Trilean`, E1033 `PossiblyUnknownCondition` for plain `if` on possibly-Unknown cond, E1034 `TrileanReturnNotRefined`, `.assume_known(message)` returns `Trilean!`, no IR / wire-format / VM changes — refinement erased at lowering, aligns implementation with SPEC §7.1.1) | Locked |
| [0023](0023-lowerer-ssa-struct-tracking.md) | Lowerer SSA struct-tracking — unified `ValueKind` enum (Struct / Outcome / Nullable / Other) replaces 4 ad-hoc HashMap tracking patterns + ~13 per-construct propagation rules with a single `value_kinds: HashMap<ValueId, ValueKind>` + one recursive `type_expr_to_value_kind` helper. Lowerer crate only — no wire format / VM / typecheck / SPEC impact. Closes v0.7 review finding "patch-stack pattern violating VISION §6 *Refuse over guess*". | Locked |
| [0024](0024-khi-dao-identity-naming.md) | Khí + Đạo identity naming (Đạo Đức Kinh §28 phác tán tắc vi khí + §42 tam sinh vạn vật) — rename 5 Rust-inherited user-facing surface terms: path keyword `crate` → `khi`, compiled artifact `.khi` → `.khi`, CLI binary `triet` → `dao`, manifest `dao.package` → `dao.package`, lockfile `dao.lock` → `dao.lock`. Source `.tri` + IR `.triv` + language name "Triết" giữ. Hard cutover, 5-stage commit series ship trước v0.7.10 mở. Justify Vietnamese-rooted philosophical depth + direct ternary tie via Đạo §42. | Locked |

### v0.8 — Ownership Foundation + Concurrency Primitives (BYOS)

| ADR | Title | Status |
|---|---|---|
| [0022](0022-trit-balanced-ownership.md) | Trit-balanced ownership — S6 5-form reference (`&+` strong owner / `&0` neutral borrow / `&-` weak observer / bare `&` / `owned` transfer), cycle-balance acyclic invariant, capability-as-unsafe for `dev.self_ref` / `dev.custom_drop`. Foundation cho v0.8 Concurrency Model | Locked |
| [0025](0025-borrow-checker-rules.md) | Borrow checker rules — compile-time enforcement algorithm cho 5 reference forms từ ADR-0022 §2; NLL + 3-rule lifetime elision + no-annotation policy; **E24XX** namespace (E2400 lifetime inference / E2410 mutability / E2420 move + use-after-move / E2430 namespace inference / E2440 NLL exclusivity); v0.8 ships skeleton diagnostics, full NLL enforcement defer v0.9 | Locked |
| [0026](0026-actor-boundary-send-rules.md) | Concurrency Primitives & Send Rules — **BYOS (Bring Your Own Scheduler)** per 2026-05-26 v1→v2 pivot. Triết core provides Send derivation (13 type categories) + Atomic primitives + capability gates; scheduler stdlib (v0.10) or external (kernel-mode). **Refuses** `actor`/`spawn`/`receive`/`send`/`async`/`await` as keywords. **E25XX** namespace (`triet::actor::E2500/E2510/E2520`). *+ 2026-05-29 Addendum: §4 placeholder refined by ADR-0028* | Locked v2 |
| [0027](0027-diagnostic-format-standard.md) | Diagnostic format standard (AI-first) — language-wide canonical format cho mọi compiler/runtime diagnostic. Header `EXXXX ErrorName` + body + optional span block + `[Fix N]` numbered fix blocks với imperative verbs (Change/Wrap/Use/Add/Replace/Move X to Y). Pure ASCII, no diff `-/+`. Retroactive scope: ADR-0020 + ADR-0025 already comply | Locked |

### v0.9 — Wide-phased: JIT + Borrow Enforcement + Atomic + Self-host policy

| ADR | Title | Status |
|---|---|---|
| [0028](0028-atomic-primitive.md) | Atomic primitive design — refines ADR-0026 v2 §4 placeholder. Locks: Rust-shim builtin pattern (IDs 27-39, `.triv` v5→v6), AtomicValue marker trait, 3-level Ordering enum (Relaxed/Synchronized/Strict) mapped vào Trit polarity, full API (load/store/swap/compare_exchange + fetch_add/sub for Tryte/Integer + bitwise ops for Integer), interior mutability via `&+ Atomic<T>` (fixes ADR-0026 v2 §4.3 `&+ mutable` contradiction), conservative E2530 fire conditions. v0.9.x.atomic implementation depends on this lock. *+ 2026-05-29 Addendum: fetch_and/or/xor renamed → fetch_bitwise_and/or/xor (explicit binary-semantic signal)* | Locked |
| [0029](0029-self-host-port-policy.md) | Self-host port policy — codifies v0.8 retrospective lesson (port lag recurring pattern). Locks: 3-layer scope (Layer A language surface MANDATORY lockstep, Layer B internal impl defer-OK, Layer C runtime independent), mandatory same-phase port (no discretion), 3-layer detection (smoke tests + release-check.sh count-based + TODO checklist), ADR template addition (Self-host port plan field), Stage 2/3 byte-identical gate lift chained to JIT (ADR-0030) | Locked |
| [0030](0030-jit-cranelift-integration.md) | JIT integration (Cranelift backend) — refines ROADMAP §v0.9 deliverables. Locks: 3-tier model (Interpreter→VM→JIT, VM persists for cold/warmup/debug), call-count ≥ 100 trigger threshold (Hotspot JVM convention), Cranelift backend với register-SSA 1:1 mapping, BrTrilean → 2 cmp + 2 brnz per ADR-0010 backend, AOT cache `~/.triet/store/jit/{target_triple}/{impl_hash}/`, synchronous JIT v0.9 (background defer v1.0+), Stage 2 ≡ Stage 3 byte-identical lift chained to perf gate. Self-host port plan: Layer C runtime, no same-phase port required. *+ 2026-05-29 Addendum: dev.jit_codegen capability (ambient default usr.*, deny-fallback to VM-only for kernel) + Backend N tier naming realign với VISION §4.2 + `--no-jit` flag + real-time disclaimer* | Locked |
| [0031](0031-borrow-expression-syntax.md) | Borrow expression syntax — closes SPEC §10 v0.7 warning ("runtime chưa expose references") + unblocks ADR-0028 §6 example. Locks: prefix `&FORM operand` syntax (5 forms total `&+`/`&+ mutable`/`&0`/`&0 mutable`/`&-` — no bare `&` form), operand grammar IDENT + field-access only (index/compound expressions defer per §10.3 backlog — `vec[i]` index syntax doesn't exist in Triết yet), prefix unary precedence tier (right-binding, higher than binary ops, lower than postfix `.`/`()`), Type::Reference(form, T) typecheck emission, lowerer passthrough (refs erase at runtime per ADR-0026 v2 §7). **Borrow check enforcement split (Phương án A 2026-05-30 author "chậm mà chắc"):** E2420 UseAfterMove SHIPS v0.9 (.7d sub-task — minimum to prevent demo from teaching wrong semantics); E2440 NLL + E2400 lifetime elision + E2403 `&-` upgrade defer v0.10 per §10.1 backlog. Self-host port plan: Layer A lockstep mandatory per ADR-0029 §3 — `compiler/parser/parser.tri` mirrors Rust impl same-phase. §10 captures full v0.10 backlog (6 items: borrow enforcement, multi-thread atomic completion, operand scope expansion, Pointer E2530, CLAUDE.md drift, self-host typecheck port). Implementation sub-phase plan §9 (.7a ADR / .7b Rust impl / .7c self-host port / .7d E2420 enforcement / .7e demo single-call + e2e). | Locked |

### v0.10 — Full builtin shim + AOT cache + NLL enforcement + multi-thread Atomic

| ADR | Title | Status |
|---|---|---|
| [0032](0032-builtin-shim-abi.md) | Builtin shim ABI — locks 5 design constraints from ADR-0030 §12.2 so v0.10.x.jit.1 (framework) + .2 (43 impls) ship against settled design. §1 Hybrid `RuntimeValue` ABI (primitives unboxed via Cranelift native `i8`/`i16`/`i64`/`i128`; composites Rc-boxed reusing `Rc<RuntimeValue>` shape from ADR-0028 §3). §2 `Rc::into_raw` on box-out + `__triet_drop_arc` shim at SSA last-use (lowerer consults ValueKind per ADR-0023). §3 Capability gate compile-time hoist via frozen `CapabilitySet` snapshot — refuse-to-emit on denied namespace; inherits ADR-0017 program-load resolution invariant. §4 `extern "C-unwind"` ABI + thread-local `CURRENT_VM_ERROR` slot + dispatcher `catch_unwind`. §5 `unsafe_code = "deny"` override scope is `triet-jit` crate ONLY (workspace `forbid` preserved elsewhere; mandatory `// SAFETY:` per block). §6 Static `SHIM_TABLE` registry + `__triet_*` symbol prefix discipline + `JITBuilder::symbol()` wiring. §7 3-layer test gates (framework smoke + 43-builtin parity + ABI proptest). §8 Self-host port: Layer C runtime, no same-phase port. First v0.10 ADR. | Locked |
| [0033](0033-aot-cache-cranelift-object.md) | AOT cache via `cranelift-object` — locks 5 design constraints from ADR-0030 §13.4 + backend-hybrid shape so v0.10.x.jit.3 ships against settled design. §1 Backend hybrid: keep `cranelift-jit` for Path B fresh compile, add `cranelift-object` for Path A persistence emission (one IR translator, two output paths). §2 Version pinning via `AotCacheManifest { cranelift_version, shim_abi_version, target_triple }` — mismatch silent-fallback to Path B + overwrite. §3 Direct `SHIM_TABLE`/`LIBCALL_TABLE` symbol resolution at load (NOT `libloading`/`dlsym`) — reuses ADR-0032 §6 registry. §4 GC integration: `jit/{triple}/{impl_hash}/` swept against `live_mods` set; new `GcReport.swept_jit_dirs`; conservative-on-corruption rule extended. §5 Per-`target_triple` path separation (no cross-arch loading attempted). §6 Determinism preserved — cache state is runtime-state, not IR-contract; bootstrap byte-identical gate uses `.khi` cmp not machine code. §7 Synchronous write on Path B success + atomic-install (ADR-0015 §3 pattern). §8 Silent-fallback recovery on any load failure. §9 4 test categories (round-trip + version mismatch + GC sweep + cross-arch isolation). Unblocks v0.10.x.jit.3 + chained gate-lift v0.10.x.jit.4 (Stage 2 ≡ Stage 3 byte-identical). | Locked |
| [0034](0034-jit-aggregate-coverage.md) | JIT aggregate coverage via delegate-to-VM shims — closes the JIT-coverage debt a v0.11.x.jit.4 audit surfaced (`compiler/main.tri`: only 3.7% of functions JIT; 96.3% tier down on struct/enum/Outcome/Nullable/String). Author "Hướng A: stop deferring." §1 Aggregate IR opcodes (FieldGet/FieldSet/EnumNew/EnumTag/EnumPayload/Outcome*/Null*) lower to `__triet_*` shims delegating to extracted `pub` VM helpers (generalizes ADR-0032 §6 — divergence-free by construction). §2 `StructNew` variadic → array-ptr+len ABI (also unblocks deferred f-string varargs). §3 String/Null constants → data section + `R_X86_64_64` loader relocation (extends ADR-0033 loader + `SUPPORTED_RELOC_TYPES`, same constraint-4 regimen). §4 Phi → Cranelift block params. §5 lift single-block shim restriction (ADR-0032 jit.2b-i). §6 fix 10 translator panics → clean tier-down. §7 delegate-to-VM for *coverage* now, native aggregate codegen deferred post-v0.11. §8 gate lift needs coverage+warm-cache (compile-cost); ≥10× bench targets JIT-friendly workload (execution). §9 iterative re-measured sub-task sequence (audit = burndown metric). Unblocks the bootstrap byte-identical gate lift. | Locked |
| [0035](0035-jit-boxed-refcount-discipline.md) | JIT boxed-value refcount discipline — closes a latent-double-free class surfaced by agg.cross-call (a `Ret` returning a borrowed box handed the borrowed `Rc` ptr to the caller as owned → caller + owner both drop). One rule: *a `Ret` transfers exactly one owned ref; clone any borrowed return to mint it.* §1 clone-on-return both modes (`__triet_clone_arc` +1; boxed DONE `b90dfed`, unboxed TODO — `TypeTag`-guided so only composite returns clone). §2 cross-mode composite result cloned in the boxed caller (local uniform rule, bounded leak vs per-callee ownership tracking). §3 explicit bounded leak tolerance (one box per occurrence, cold/proportional paths; oracle tier, not production — never trade a leak for a double-free; leak sites must be countable). §4 records the `TypeTag::Unit` ambiguity (AST nodes = structs = `Unit`) that caps cross-mode coverage until an IR-shape change (out of scope). Rejects per-callee escape analysis + GC for v0.11. Extends ADR-0032 §2 lifetime rule; governs ADR-0034 Bậc A boxed values. | Locked |

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
