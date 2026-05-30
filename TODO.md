# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.9.x archived

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
| v0.9.x.jit (Cranelift JIT — partial; .4/.6/.7/.8 deferred v0.10) | 0030 | 1536 |
| v0.9.final (version bump + archive) | — | 1536 |

---

## v0.10 — Full builtin shim + AOT cache + NLL enforcement + multi-thread Atomic 🔄 opening

**Timeline 2026-05-30:** Author chose 2-day implementation window with AI as primary code author. Goal: solve the coherent cluster v0.9 deliberately deferred. Per "chậm mà chắc" — don't ship temporary, ship complete.

**Scope (priority order, depends on backlog feasibility):**

1. **Full builtin shim layer** per [ADR-0030 §12](docs/decisions/0030-jit-cranelift-integration.md) — 43 builtins × `RuntimeValue` ABI marshaling. Five design constraints (ABI representation / lifetime mgmt / capability gate / panic propagation / unsafe override) per §12.2. ~1500-2500 LOC.
2. **AOT cache via cranelift-object backend swap** per [ADR-0030 §13](docs/decisions/0030-jit-cranelift-integration.md) — emit ELF/.o with relocations, load via `libloading` at runtime. Filesystem layout per §13.3 (`~/.triet/store/jit/{target_triple}/{impl_hash}/`). ~1000-1500 LOC.
3. **Bootstrap gate lift** per ADR-0030 §14 — once .1 + .2 land, lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required. Validates the full self-host compiles within < 10 min budget per §11.8.
4. **Perf bench ≥10× v0.3 baseline** per ADR-0030 §14 — measure JIT speedup on numeric-heavy programs + bootstrap < 10 min.
5. **NLL borrow checker enforcement** per [ADR-0025](docs/decisions/0025-borrow-checker-rules.md) + [ADR-0031 §10.1](docs/decisions/0031-borrow-expression-syntax.md) — E2440 NLL CFG live-range + E2400 lifetime elision 3 rules + E2403 `&-` upgrade. ~1000+ LOC.
6. **Real `raw_thread.spawn` + multi-thread Atomic** per [ADR-0026 v2 §3](docs/decisions/0026-actor-boundary-send-rules.md) + [ADR-0031 §10.2](docs/decisions/0031-borrow-expression-syntax.md) — OS thread integration + Send-boundary refcount-bump codegen + multi-worker atomic_counter demo.
7. **Interpreter parity for `sys.atomic.*`** per [ADR-0031 §10.7](docs/decisions/0031-borrow-expression-syntax.md) — drop "VM-only" caveat from atomic_counter demo.

**Pre-v0.10 baseline audit:** ✅ `scripts/release-check.sh` clean per ADR-0009 Addendum §C. 1536 tests, all gates green. Safe to open phase.

**Sub-phase planning:** Will draft v0.10.0 design ADR (or amend §12/§13/§14) before any sub-tasks land, per ADR-0009 design-first principle.
