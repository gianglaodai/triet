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

### v0.9.0 — Design phase (ADRs first per ADR-0009 + project philosophy)

- [ ] **v0.9.0.1** — Draft [ADR-0028](docs/decisions/0028-atomic-primitive.md) Atomic Primitive — refine ADR-0026 v2 §4 placeholder. Lock: implementation pattern (VM opcodes vs builtin shims), Ordering ↔ Trit mapping, full operation set, constructor + drop, E2530 fire conditions.
- [ ] **v0.9.0.2** — Draft [ADR-0029](docs/decisions/0029-self-host-port-policy.md) Self-host port policy — per v0.8.x.completion.4 lessons learned. Lock: lockstep vs freeze, sync triggers, frozen state ground rules.
- [ ] **v0.9.0.3** — Draft [ADR-0030](docs/decisions/0030-jit-cranelift-integration.md) JIT integration — Cranelift backend choices, tier-2 dispatch, AOT cache layout, perf gate criteria.

### v0.9.x.atomic — Atomic Primitive implementation (after ADR-0028 lock)

- [ ] **v0.9.x.atomic.1+** — Sub-tasks defined post-ADR-0028. Scope: type system signatures, VM opcodes/builtins, runtime, stdlib `sys.atomic.*` module, Send rule integration test corpus, E2530 emit.

### v0.9.x.borrow — Borrow checker enforcement (independent of Atomic/JIT)

- [ ] **v0.9.x.borrow.1+** — NLL enforcement (E2440 real fires), lifetime elision 3 rules (E2400 real), `&-` upgrade tracking (E2403 real). Per ADR-0025 explicit defer.

### v0.9.x.jit — Cranelift JIT backend (after ADR-0030 lock)

- [ ] **v0.9.x.jit.1+** — Cranelift integration, profile-guided dispatch, AOT cache. Per ROADMAP §v0.9 original target.

### v0.9.final — release

- [ ] **v0.9.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` clean, Cargo 0.8.0 → 0.9.0, SPEC v0.9 header, README + ARCHITECTURE.md sync, version bump commit độc lập (no bundling per cadence).

### Workflow note

Trước khi bắt đầu sub-task v0.9.x đầu tiên (v0.9.0.1): hooks đã install và baseline clean. Per ADR-0009 Addendum §C, pre-version audit pass đã hoàn thành 2026-05-29.
