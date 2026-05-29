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
- [ ] **v0.9.x.atomic.2** — IR builtin declaration (10 atomic builtins: New/Load/Store/Swap/CompareExchange/FetchAdd/FetchSub/FetchBitwiseAnd/FetchBitwiseOr/FetchBitwiseXor). Wire format `.triv` v5 → v6 per ADR-0028 §1. Serde encode/decode + version pin test. VM dispatcher placeholder arms (panic-with-clear-message until v0.9.x.atomic.3-4 lands real dispatch). **NOTE: outline revised v0.9.x.atomic.2 from original "Ordering enum lexer + parser + typecheck" — Ordering ships as regular Triết enum in stdlib file (.5), not separate sub-task.**
- [ ] **v0.9.x.atomic.3** — VM dispatch (single-thread no-op per ADR-0028 §9 dev tier): AtomicNew + AtomicLoad/Store/Swap/CompareExchange. Universal ops cho mọi AtomicValue type (Trit/Tryte/Integer/Trilean).
- [ ] **v0.9.x.atomic.4** — VM dispatch fetch_add/sub (Tryte/Integer per ADR-0028 §4.2) + fetch_bitwise_and/or/xor (Integer only per Addendum). Arithmetic + binary-leak ops.
- [ ] **v0.9.x.atomic.5** — Stdlib `std/sys/atomic.tri` module: `enum Ordering { Relaxed, Synchronized, Strict }` per ADR-0028 §3 + function signatures cho 10 builtins. Loader.rs synthetic `sys` root wire (mirror `std` synthetic root pattern). Capability enforcement: `requires sys.atomic` checked. Also closes check_resolved.rs cross-module Atomic typecheck path (deferred từ .1).
- [ ] **v0.9.x.atomic.6** — E2530 conservative fire conditions per ADR-0028 §10: compare_exchange success<failure ordering, fetch_* Relaxed trên Atomic<Pointer> (when Pointer lands).
- [ ] **v0.9.x.atomic.7** — `atomic_counter` demo upgrade: actually run `spawn_worker(counter)` exercising fetch_add. Verify runtime semantics correct (single-thread VM no-op atomicity per ADR-0028 §9).
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
