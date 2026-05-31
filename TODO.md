# TODO

Sub-task tracking — short-term work in progress.

- Long-term phasing: [`ROADMAP.md`](ROADMAP.md)
- Architectural decisions: [`docs/decisions/`](docs/decisions/)
- Language semantics: [`SPEC.md`](SPEC.md), [`VISION.md`](VISION.md)

This file tracks the **current phase** only. When a phase finishes, its summary archives to `ROADMAP.md` and detailed checkboxes are deleted from here.

---

## v0.2 — v0.10.x archived

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
| v0.10.0 (Design phase — ADR-0032/0033) | 0032, 0033 | 1536 |
| v0.10.x.interp (Interpreter atomic parity) | 0031 | 1549 |
| v0.10.x.jit (JIT builtin-shim layer — 36/43; .3/.4 deferred v0.11) | 0032 | ~1620 |
| v0.10.x.thread (Multi-thread Atomic — spawn + Arc<Mutex>) | 0026 v2, 0028 | ~1635 |
| v0.10.x.borrow (NLL enforcement — E2440/E2400/E2411/E2403) | 0025, 0031 | 1637 |
| v0.10.final (version bump + archive) | — | 1637 |

---

## v0.11 — JIT AOT cache + bootstrap gate lift + std.concurrency 🔜 next

Deferred from v0.10 (see [`ROADMAP.md`](ROADMAP.md) §v0.10 *Không làm*):

- **JIT AOT cache (jit.3)** via cranelift-object per ADR-0033 — relocating-ELF-loader cliff (no turnkey crate loads `.o` + symbol resolver → executable). Design locked; dedicated phase.
- **Bootstrap gate lift + ≥10× perf bench (jit.4)** per ADR-0030 §14 — chains on jit.3 warm cache (3000-fn cold-JIT self-host prohibitive). Lifts `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]`.
- **JIT shim gaps:** varargs `FStringConcat`/`TextConcat` (array-ptr+len ABI, ADR-0032 jit.2b-iii Addendum); multi-block-shim codegen (jit.2b-i single-block scope); Ordering-`EnumNew` codegen for end-to-end atomic-function JIT.
- **Borrow checker corpus-driven:** field-granular NLL base, inter-procedural borrow, closure captures, E2403 full owner-trail, Rule-2 elision (`self`-param parser), E2410 field-assign enforcement.
- **Concurrency closures:** `spawn(closure)` Send-bound closure types → real Send-boundary refcount-bump codegen (thread.2) + Triết-source multi-worker (thread.3).
- **`std.concurrency.*` stdlib** (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

**Pre-v0.11 baseline:** open with a `scripts/release-check.sh` audit per ADR-0009 Addendum §C before the first impl sub-task.
