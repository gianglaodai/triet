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
| v0.9.0 (Design phase — ADR-0028/0029/0030) | 0028, 0029, 0030 | 1436 |
| v0.9.x.atomic (Atomic Primitive Implementation) | 0028, 0031 | 1506 |

---

## v0.9 — Wide-phased (JIT + Borrow + Atomic + Self-host policy) 🔄 in progress

**Scope decision 2026-05-29:** Author chose wide-phased per ADR-0025 + ADR-0026 v2 explicit "defer v0.9" promises. Internal ordering: design ADRs first (v0.9.0), then implementation sub-phases run roughly in parallel after design lands.

**Pre-v0.9 baseline audit:** ✅ `scripts/release-check.sh` clean per ADR-0009 Addendum §C mandatory protocol. 1436 tests passing, all gates green. Safe to open phase.

### v0.9.0 — Design phase ✅ ARCHIVED (see ROADMAP §v0.9 sub-phase status table)

### v0.9.x.atomic — Atomic Primitive Implementation ✅ ARCHIVED (see ROADMAP §v0.9.x.atomic)

### v0.9.x.borrow — Folded into v0.9.x.atomic.7d per ADR-0031 §4 Phương án A

NLL (E2440) + lifetime elision (E2400) + `&-` upgrade (E2403) + mutability (E2410/11) defer v0.10 per [ADR-0031 §10.1](docs/decisions/0031-borrow-expression-syntax.md) backlog (corpus-driven per ADR-0025).

### v0.9.x.jit — Cranelift JIT backend (after ADR-0030 lock)

- [ ] **v0.9.x.jit.1+** — Cranelift integration, profile-guided dispatch, AOT cache. Per ROADMAP §v0.9 original target.

### v0.9.final — release

- [ ] **v0.9.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` clean, Cargo 0.8.0 → 0.9.0, SPEC v0.9 header, README + ARCHITECTURE.md sync, version bump commit độc lập (no bundling per cadence).

### Workflow note

Trước khi bắt đầu sub-task v0.9.x đầu tiên (v0.9.0.1): hooks đã install và baseline clean. Per ADR-0009 Addendum §C, pre-version audit pass đã hoàn thành 2026-05-29.
