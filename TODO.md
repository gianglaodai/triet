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

## v0.10 — Full builtin shim + AOT cache + NLL enforcement + multi-thread Atomic 🔄 in progress

**Scope decision 2026-05-30 (Option B):** Author chose 2-day implementation window with AI as primary code author. Tier 1A (JIT completion + multi-thread + interpreter parity) + Tier 1B (NLL borrow enforcement) = 12 items / 14 sub-tasks. `std.concurrency.*` stdlib (Mutex / Channel / M:N green threads per ADR-0028 §10) deferred v0.11 — feature-new scope without existing ADR; v0.10 closes v0.9 ADR promises.

**Pre-v0.10 baseline audit:** ✅ `scripts/release-check.sh` PASSED per ADR-0009 Addendum §C mandatory protocol. 1536 tests passing, all 4 gates green. Safe to open phase.

### v0.10.0 — Design phase (2 NEW ADRs)

- [x] **v0.10.0.1** — [ADR-0032 NEW] Builtin shim ABI design — locks 5 constraints from ADR-0030 §12.2: (1) `RuntimeValue` ABI representation choice (boxed vs specialized vs hybrid), (2) lifetime management (`Rc::into_raw` + `drop_arc` pattern), (3) capability gate enforcement (per-builtin runtime check), (4) panic → `VmError` propagation (Cranelift trap vs `extern "C-unwind"`), (5) `unsafe_code` policy override scope. — `dcd49ae`
- [x] **v0.10.0.2** — [ADR-0033 NEW] AOT cache cranelift-object protocol — locks 5 constraints from ADR-0030 §13.4: (1) Cranelift version pinning + cache invalidation, (2) libcall symbol resolution at load via `libloading`/`dlsym`, (3) `dao store gc` mark-and-sweep root tracking, (4) cross-machine portability (per-`target_triple` separation), (5) determinism preservation (cache hit/miss not part of IR contract). — `7268e26`

### v0.10.x.interp — Interpreter parity (smallest, lowest risk, warm-up)

- [x] **v0.10.x.interp.1** — Atomic builtin interpreter parity per ADR-0031 §10.7. Add `sys.atomic.*` path intercepts to `triet-interpreter` mirroring VM's `path_to_builtin`. `RuntimeValue::Atomic` variant in interpreter Value enum + per-op dispatch. Drops VM-only caveat from `atomic_counter` demo. ~300 LOC + tests. — `be9e535` (+13 tests; compare_exchange returns TypeError pending Outcome parity; `atomic_counter.tri` end-to-end via `dao run` confirmed)

### v0.10.x.jit — JIT subsystem completion (4 sub-tasks)

- [ ] **v0.10.x.jit.1** — Builtin shim infrastructure per ADR-0032. Crate-level `unsafe_code` lint override (`forbid → deny` with documented audit). `extern "C"` shim registry + `JITBuilder::symbol()` wiring. Panic → VmError propagation harness (thread-local context). NO builtin implementations yet — just framework. ~500 LOC + framework tests.
- [ ] **v0.10.x.jit.2** — Builtin shim implementations (all 43 builtins) per ADR-0032 ABI choice. Wire `BuiltinName::*` variants to `extern "C"` shims across I/O / Assert / Text / Vector / HashMap / File I/O / Path / String / Misc / Atomic categories. ~1300-1800 LOC across shim functions + integration tests.
- [ ] **v0.10.x.jit.3** — AOT cache via `cranelift-object` per ADR-0033. Add `cranelift-object` + `libloading` deps. `JitDispatcher` dual-path: AOT cache hit → object load + symbol resolve; miss → fresh `cranelift-jit` compile + persist to `~/.triet/store/jit/{triple}/{impl_hash}/`. `dao store gc` integration. ~800 LOC.
- [ ] **v0.10.x.jit.4** — Bootstrap gate lift + perf bench per ADR-0030 §14. Lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required (per ADR-0019 §7 Addendum chain). Add `criterion` bench measuring ≥10× v0.3 baseline on numeric programs + bootstrap < 10 min. ~150 LOC + benchmark fixtures.

### v0.10.x.thread — Multi-thread Atomic completion (3 sub-tasks)

- [ ] **v0.10.x.thread.1** — `raw_thread.spawn` real OS thread impl per ADR-0026 v2 §3. Replace placeholder `spawn(work: Integer) -> Handle = Handle { thread_id: 0 }` with real OS thread creation. `Handle.join()` blocks until thread terminates. POSIX-first per ADR-0018 precedent (Windows stub OK). ~400 LOC + tests.
- [ ] **v0.10.x.thread.2** — Send-boundary refcount-bump codegen per ADR-0026 v2 §3.2. When `&+ T` crosses spawn boundary, emit refcount-bump on ObjectHeader (`triet-core::memory`). Matching Drop on thread join. User-visible: nothing changes; under the hood: multi-share enabled. ~300 LOC.
- [ ] **v0.10.x.thread.3** — `&+ Atomic<T>` multi-thread clone semantics + multi-worker demo per ADR-0028 §5 + ADR-0031 §10.2. Wire clone-on-Send-boundary path for `&+ Atomic<T>`; single-thread `&+` stays linear move per v0.9 .7d E2420. Reactivate 3-worker `atomic_counter` demo with concurrency assertion (counter eventually consistent ≥ 3 after all join). ~200 LOC + e2e test.

### v0.10.x.borrow — Borrow checker enforcement (3 sub-tasks, Tier 1B)

- [ ] **v0.10.x.borrow.1** — E2440 NLL borrow exclusivity (full CFG live-range) per ADR-0025 §2 + ADR-0031 §10.1. Compute borrow-active region from creation to last-use; reject overlapping `&0 mutable` / `&0` / `&+` borrows. Biggest item — ~1000+ LOC. **Risk:** may tier-down further if 2-day budget too tight; .2 + .3 are lower-risk.
- [x] **v0.10.x.borrow.2** — E2400 lifetime elision 3 rules per ADR-0025 §3. Quy tắc 1 (single input borrow → output), quy tắc 2 (`self` receiver → output ties self), quy tắc 3 (owned return). E2400 fires when all 3 fail. ~300 LOC. — `1b78f94` (+14 tests; Rule 2 dormant pending `self`-parameter parser syntax; nested-borrow defer v0.11+ corpus-driven)
- [ ] **v0.10.x.borrow.3** — E2403 `&-` weak observer upgrade + E2410/E2411 mutability per ADR-0022 §2 row 5 + v0.8.10 skeletons. E2403: deref `&- T → T?` upgrade tracking. E2410/E2411: assign-to-frozen + mutate-via-readonly-borrow full enforcement. ~400 LOC combined.

### v0.10.final — release

- [ ] **v0.10.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` ✓✓✓✓ all 4 gates clean, Cargo 0.9.0 → 0.10.0, SPEC v0.9 → v0.10, README + ARCHITECTURE.md + ROADMAP + CLAUDE.md sync, ROADMAP archive sub-phase summary table, version bump commit độc lập (no bundling per cadence).

### v0.11 backlog (deferred from v0.10 Option B)

- `std.concurrency.*` stdlib (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

### Workflow note

Per ADR-0009 design-first principle, **v0.10.0.1 + v0.10.0.2 must lock before any impl sub-task starts**. ADR-0032 unblocks v0.10.x.jit.1; ADR-0033 unblocks v0.10.x.jit.3. The other workstreams (interp / thread / borrow) don't depend on new ADRs (existing ADR-0025 + ADR-0026 v2 + ADR-0031 cover).
