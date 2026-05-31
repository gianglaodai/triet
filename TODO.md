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

## v0.11 — JIT AOT cache + bootstrap gate lift 🔄 in progress

**Scope decision 2026-05-31 (author):** v0.11 prioritizes the **JIT AOT cache** first (over `std.concurrency.*` or low-risk cleanup). Rationale: it lifts the bootstrap byte-identical gate that's been `#[ignore]`'d since v0.7 + delivers the headline ≥10× perf win. The other v0.11 backlog items (varargs shims, borrow corpus, concurrency closures, `std.concurrency.*`) trail the AOT cache or move to a later phase.

**Pre-v0.11 baseline audit:** ✅ `scripts/release-check.sh` PASSED per ADR-0009 Addendum §C. 1637 tests, all 4 gates green (run 2026-05-31). Safe to open phase.

### v0.11.0 — Design phase (loader-approach resolution)

- [ ] **v0.11.0.1** — [ADR-0033 Addendum or ADR-0034 NEW] **Loader-approach decision** — ADR-0033's design is locked but its §3 Path-A loader is the deferred cliff (hand-rolled `R_X86_64_*` relocation patching + `mmap` RW→RX = highest mem-corruption-risk code in project). Before any impl, lock HOW the `.o` becomes executable: evaluate **(A)** hand-rolled relocating loader (ADR-0033 §3 as-written, `object` + `memmap2`) vs **(B)** system-linker + `dlopen` (cranelift-object `.o` → `cc -shared` → `libloading::Library`, host built `-rdynamic` so shim symbols resolve via dynamic table). Decide on the mem-safety / portability / dep-tree tradeoff; supersede ADR-0033 §3 if (B). Author sign-off required (ADR change). DESIGN-FIRST per ADR-0009 — blocks v0.11.x.jit.3.

### v0.11.x.jit — AOT cache implementation (depends on v0.11.0.1)

- [ ] **v0.11.x.jit.3** — AOT cache implementation per ADR-0033 (§1 backend hybrid + §2 version-pinned manifest + §3 symbol resolution per chosen loader + §4 `dao store gc` integration + §5 per-triple separation + §6 determinism doc + §7 synchronous atomic-install + §8 silent fallback + §9.1–9.4 test gates). ~800 LOC + 4 test categories. POSIX/ELF-first per ADR-0018 precedent.
- [ ] **v0.11.x.jit.4** — Bootstrap gate lift + ≥10× perf bench per ADR-0030 §9 + §14. Lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` once warm-cache self-host completes < 10 min (ADR-0033 §9.5 chain). `criterion` warm-vs-cold bench, ≥10× v0.3 baseline target.

### v0.11 backlog (trails AOT cache or later phase)

- **JIT shim gaps:** varargs `FStringConcat`/`TextConcat` (array-ptr+len ABI, ADR-0032 jit.2b-iii Addendum); multi-block-shim codegen (jit.2b-i single-block scope); Ordering-`EnumNew` codegen for end-to-end atomic-function JIT.
- **Borrow checker corpus-driven:** field-granular NLL base, inter-procedural borrow, closure captures, E2403 full owner-trail, Rule-2 elision (`self`-param parser), E2410 field-assign enforcement.
- **Concurrency closures:** `spawn(closure)` Send-bound closure types → real Send-boundary refcount-bump codegen (thread.2) + Triết-source multi-worker (thread.3).
- **`std.concurrency.*` stdlib** (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

### Workflow note

Per ADR-0009 design-first, **v0.11.0.1 must lock before v0.11.x.jit.3 starts** — the loader approach determines the §3 symbol-resolution mechanism + the unsafe surface. ADR-0032's SHIM_TABLE is the symbol resolution source of truth either way (ADR-0033 §3).
