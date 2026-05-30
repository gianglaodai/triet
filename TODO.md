# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.8.x archived

All shipped phases now live in [`ROADMAP.md`](ROADMAP.md):

| Phase | ADRs | Final test count |
|---|---|---|
| v0.2.x Module system | 0005, 0006 | 700+ |
| v0.3 Bytecode VM + Stable IR | 0007, 0008 | 835 |
| v0.3.x.cleanup | 0009 | 835 |
| v0.3.x.ternary | 0010 | 838 |
| v0.4 Crate-Pack + Stable ABI | 0011, 0012, 0013 | 867 |
| v0.5 CAS Packaging | 0014, 0015 | 918 |
| v0.5.x.review | 0015 Addendum | 924 |
| v0.6 Capability System | 0016, 0017, 0018 | 1079 |
| v0.6.x.review | 0018 Addendum | 1085 |
| v0.7 Self-hosting Compiler | 0019, 0020, 0021, 0024 | 1345 |
| v0.8 Ownership Foundation + BYOS | 0022, 0025, 0026 v2, 0027 | 1425 |
| v0.8.x.review (audit fixes) | — | 1425 |
| v0.8.x.docs-reorg (token + ADR thematic + ROADMAP compress + SPEC expand) | — | 1425 |
| v0.8.x.cadence-fix (process enforcement automation) | 0009 Addendum | 1425 |
| v0.8.x.completion (trục 2 implementation gap closure) | — | 1436 |

---

## v0.9 — Wide-phased (JIT + Borrow + Atomic + Self-host policy) 🔄 in progress

**Scope decision 2026-05-29:** Author chose wide-phased per ADR-0025 + ADR-0026 v2 explicit "defer v0.9" promises. Internal ordering: design ADRs first (v0.9.0), then implementation sub-phases run roughly in parallel after design lands.

**Pre-v0.9 baseline audit:** ✅ `scripts/release-check.sh` clean per ADR-0009 Addendum §C mandatory protocol. 1436 tests passing, all gates green. Safe to open phase.

### v0.9.0 — Design phase ✅ COMPLETE (ADRs first per ADR-0009 + project philosophy)

- [x] **v0.9.0.1** — [ADR-0028](docs/decisions/0028-atomic-primitive.md) Atomic Primitive — refined ADR-0026 v2 §4 placeholder. Builtin shim strategy, AtomicValue marker trait, 3-level Ordering ↔ Trit mapping, full API, `&+ Atomic<T>` interior mutability (fixes v2 §4.3 contradiction), conservative E2530. Drafted `603864c`, locked `06244fe` (+ ADR-0026 v2 Addendum + indexes).
- [x] **v0.9.0.2** — [ADR-0029](docs/decisions/0029-self-host-port-policy.md) Self-host port policy — 3-layer scope (A lockstep mandatory / B defer-OK / C independent), 3-layer detection (smoke + count-based release-check + TODO checklist), ADR template addition. Drafted `260fa9a`, locked `99a089b`. Plus §4 detection implementation + backlog port ADR-0020 §3 ternary postfix tokens `~+>/~->/~0>` (caught by detection on first run — ~6-month silent drift from v0.7.4.3-error.4) — `deb04d1`.
- [x] **v0.9.0.3** — [ADR-0030](docs/decisions/0030-jit-cranelift-integration.md) JIT integration (Cranelift backend) — 3-tier model (Interpreter+VM+JIT), 100-call threshold, register-SSA 1:1 mapping, AOT cache per impl_hash, sync JIT v0.9 (background defer v1.0+), no capability gate. Stage 2/3 byte-identical lift conditions. Drafted `d9b0289`, locked `3bed098`. First ADR using ADR-0029 §5 Self-host port plan template.

### v0.9.x.atomic — Atomic Primitive implementation (ADR-0028 + Addendum)

- [x] **v0.9.x.atomic.1** — `AtomicValue` typecheck enforcement. `Type::is_atomic_value()` method, `TypeError::NonAtomicValueType` E1040, validation at check.rs construction site, 8 lib.rs tests + 2 diagnostics_format tests. +10 net tests (1436→1446). check_resolved.rs path deferred v0.9.x.atomic.5 stdlib work — `6788d1c`.
- [x] **v0.9.x.atomic.2** — IR builtin declaration (10 atomic builtins) + `.triv` v5→v6 + VM placeholders + path lookup. Self-host lockstep port `compiler/pack_writer.tri` TRIV_VERSION (ADR-0029 §2 paid off — bootstrap byte-identical gate caught Stage 2 still emitting v5 immediately, fixed same commit). 1446 → 1447 — `d898760`.
- [x] **v0.9.x.atomic.3** — VM dispatch universal ops (AtomicNew + Load/Store/Swap/CompareExchange) per ADR-0028 §4.1. `RuntimeValue::Atomic(Rc<RefCell>)` + `TypeTag::Atomic` + `VmError::BuiltinUnimplemented` E2211 + `atomic_value_eq` helper. 6 dispatch tests. 1447 → 1453 — `a90caa6`.
- [x] **v0.9.x.atomic.4** — VM dispatch fetch_add/sub + fetch_bitwise_and/or/xor. ArithmeticOp/BitwiseOp helper enums + atomic_fetch_arithmetic/bitwise functions. Reuses existing arithmetic_add/sub helpers (overflow detection inherited). 5 dispatch tests. 1453 → 1458 — `0d3f9fa`.
- [x] **v0.9.x.atomic.5a** — Loader synthetic `sys` root with conditional child declaration (probe `std/sys/<name>.tri` presence). Preserves v0.8.11 ambient behavior when no files exist (current state); readies infrastructure for .5b. Self-host port not needed (Layer C runtime classification per ADR-0029 §3). 1458 unchanged — `8131d1e`.
- [x] **v0.9.x.atomic.5b** — Stdlib `std/sys/atomic.tri` shipped: `enum Ordering { Relaxed, Synchronized, Strict }` + `struct CompareExchangeFailed<T>` + 10 function signatures (load/store/swap/compare_exchange/fetch_add/fetch_sub/fetch_bitwise_and|or|xor) per ADR-0028 §3-§4 + Addendum. Demo updated to `from sys.atomic import Synchronized, fetch_add` + Ordering arg. 4 e2e tests updated (drop `Atomic` paperwork import — built-in type). Loader stdlib count 14 → 16 modules + arenas. Module system demo snapshot updated. Self-host port not needed (Layer C — stdlib data file, no Rust counterpart per ADR-0029 §3). All 1458 tests passing, release-check clean — `6f6b92e`.
- [x] **v0.9.x.atomic.5c** — Stdlib `std/sys/raw_thread.tri` shipped: `struct Handle { thread_id: Integer }` + `spawn(work: Integer) -> Handle` + `join(handle: Handle)` placeholders per ADR-0026 v2 §3 (full closure-typed signature defers v0.10+ when Send-bounded closure types ship). Loader stdlib count 16→17 modules + arenas. Cross-module Atomic typecheck path closed in `check_resolved.rs:297` — non-AtomicValue inner type returns `Type::Unknown` instead of propagating malformed `Type::Atomic(bad)` through cross-module name_table (E1040 attribution stays at `check.rs` declaration site per ADR-0028 §2). 2 new check_resolved tests (valid round-trip + no-cascade). 1460 tests passing, release-check clean — `97e30d6`.
- [x] **v0.9.x.atomic.6** — E2530 `InvalidAtomicOrdering` conservative fire per ADR-0028 §10: `compare_exchange` success-weaker-than-failure pattern. `ConcurrencyError::InvalidAtomicOrdering` variant + `check_atomic_ordering` helper with dual gate (callee name `compare_exchange` AND signature shape `(_, _, _, Ordering, Ordering)`). Strength rank Relaxed=0 < Synchronized=1 < Strict=2 per ADR-0028 §3. ADR-0027 [Fix N] help text (raise success / lower failure / use Synchronized on both). Pointer-Relaxed `fetch_*` deferred until Pointer type lands. Aliased imports + runtime-bound ordering values corpus-deferred per §10. 12 lib tests (3 positive + 6 negative + 3 false-positive guards) + 2 diagnostics_format tests. Self-host port not needed (Layer C runtime classification per ADR-0029 §3). 1460 → 1474 — `0922936`.
- [ ] **v0.9.x.atomic.7** — Demo runtime exercise. Blocked by missing expression-level borrow syntax (SPEC §10 v0.7 warning surface). Split into 4 sub-tasks per author choice 2026-05-30:
  - [ ] **v0.9.x.atomic.7a** — [ADR-0031](docs/decisions/0031-borrow-expression-syntax.md) Borrow Expression Syntax draft + lock. Prefix `&FORM operand` (all 5 forms), operand IDENT/field/index v0.9 scope, prefix unary precedence tier, lowerer passthrough, self-host Layer A lockstep port per ADR-0029 §3. Borrow checker NLL enforcement defers v0.9.x.borrow.*.
  - [ ] **v0.9.x.atomic.7b** — Rust impl: `Expr::Borrow { form, operand }` AST variant + parser prefix rule + typecheck Type::Reference emission + lowerer passthrough. Per-crate tests (parser + typecheck + lowerer).
  - [ ] **v0.9.x.atomic.7c** — Self-host Layer A port: `compiler/parser/parser.tri` AST + prefix rule lockstep. Bootstrap symmetry test extension.
  - [ ] **v0.9.x.atomic.7d** — `atomic_counter` demo upgrade: `let counter = new(0); spawn_worker(&+ counter); load(&+ counter, Synchronized)`. E2E test in `crates/triet-cli/tests/atomic_counter_e2e.rs` — assert `Counter after 3 increments: 3`. Per ADR-0028 §9 single-thread VM no-op atomicity.
- [ ] **v0.9.x.atomic.8** — Phase verify gate: cargo test + clippy + fmt clean, release-check.sh pass, ROADMAP/TODO archive. Final test count target ~1500+.

### v0.9.x.borrow — Borrow checker enforcement (independent of Atomic/JIT)

- [ ] **v0.9.x.borrow.1+** — NLL enforcement (E2440 real fires), lifetime elision 3 rules (E2400 real), `&-` upgrade tracking (E2403 real). Per ADR-0025 explicit defer.

### v0.9.x.borrow — Borrow checker enforcement (independent of Atomic/JIT)

- [ ] **v0.9.x.borrow.1+** — NLL enforcement (E2440 real fires), lifetime elision 3 rules (E2400 real), `&-` upgrade tracking (E2403 real). Per ADR-0025 explicit defer.

### v0.9.x.jit — Cranelift JIT backend (after ADR-0030 lock)

- [ ] **v0.9.x.jit.1+** — Cranelift integration, profile-guided dispatch, AOT cache. Per ROADMAP §v0.9 original target.

### v0.9.final — release

- [ ] **v0.9.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` clean, Cargo 0.8.0 → 0.9.0, SPEC v0.9 header, README + ARCHITECTURE.md sync, version bump commit độc lập (no bundling per cadence).

### Workflow note

Trước khi bắt đầu sub-task v0.9.x đầu tiên (v0.9.0.1): hooks đã install và baseline clean. Per ADR-0009 Addendum §C, pre-version audit pass đã hoàn thành 2026-05-29.
