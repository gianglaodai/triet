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

### v0.10.x.jit — JIT subsystem completion (jit.2 split → 5 sub-tasks)

- [x] **v0.10.x.jit.1** — Builtin shim infrastructure per ADR-0032. Crate-level `unsafe_code` lint override (`forbid → deny` with documented audit). `extern "C"` shim registry + `JITBuilder::symbol()` wiring. Panic → VmError propagation harness (thread-local context). NO builtin implementations yet — just framework. ~500 LOC + framework tests. — `4a5142b` (shipped registry + drop_arc + capability table + ABI converters + 3 framework tests + 1 new `unsafe` block. **§4 panic→VmError DEFERRED**: `extern "C-unwind"` + `catch_unwind` across JIT frame blocked on cranelift-jit 0.132 — no system unwind-table registration → abort. Framework test #4 caught it. ADR-0032 Addendum records cliff + 3 redesign options. **RESOLVED `712b70c`: option 2 per-call sentinel** — author sign-off 2026-05-31)
- [x] **v0.10.x.jit.2a** — §4 option-2 error mechanism + composite-value JIT flow + 5 representative shims. Per ADR-0032 §4 option-2 resolution: TLS `VmError` slot + `SHIM_FAILED` flag + `__triet_shim_failed` probe + per-call sentinel-check codegen + per-function `error_exit` block + dispatcher TLS check + re-add `VmError::JitShimFault`. Composite-value flow: `map_type` composite→I64 ptr, `Rc::into_raw` box-out / borrow-in, `__triet_drop_arc` emission at SSA last-use (ValueKind tracking). 5 shims (`Assert`/`Println`/`TextLen`/`VectorNew`/`VectorPush`) covering composite arg boxing + primitive↔composite mix + error/sentinel path + drop_arc. Parity tests (VM↔JIT) for the 5. ~800 LOC. **Validates the highest-silent-miscompilation-risk foundation before mass-producing shims.** — `4bb3183` (+16 tests; §4 option-2 TLS+flag+probe+boundary-check; composite ABI box/borrow; 5 shims. **SCOPE NARROWED**: single-shim-call per fn (2nd tier-downs); per-call sentinel codegen + `error_exit` block + `drop_arc` emission DEFERRED jit.2b — single-call scope's boundary TLS check suffices + never creates-and-discards a composite. 6 unsafe blocks all SAFETY-doc'd)
- [x] **v0.10.x.jit.2b-i** — Multi-call codegen + clean fixed-arity shims (collections/text/string/path). Per-call sentinel-check codegen (`__triet_shim_failed` probe after each shim call → branch to per-function `error_exit` block) + `drop_arc` emission at composite SSA last-use (lifts jit.2a's single-shim-call scope). ~18 clean shims: `Print`/`AssertEq`/`TextFromInteger`/`VectorGet`/`VectorLength`/`HashMap*`(5)/`Path*`(3)/`String*`(3)/`ParseInteger`/`TextIntoBytes`/`TextFromBytes`/`Blake3Hash`/`GetEnv`. Several return `T?`/`Outcome` boxed. Parity tests (VM↔JIT). ~800 LOC. — `38cedf7` (+11 tests; multi-call per-call sentinel + drop_arc-at-Ret + lazy error_exit; 21 shims DELEGATE semantics to new pub `triet_ir::dispatch_builtin` = zero VM↔JIT divergence by construction; delegation fixed a jit.2a divergence; single-Triết-block shim scope, multi-block tier-down; 6 unsafe all SAFETY-doc'd)
- [x] **v0.10.x.jit.2b-ii** — Atomic ×10 shims (`AtomicNew`/`Load`/`Store`/`Swap`/`CompareExchange`/`FetchAdd`/`FetchSub`/`FetchBitwise{And,Or,Xor}`). Uses `Arc<Mutex>` repr (thread.2 migration — supersedes ADR-0032 §1 `Rc<RefCell>` text). `compare_exchange` returns Outcome (composite). Parity tests + cross-thread share via the jit dispatch path. ~400 LOC. — `8ea517e` (+8 tests; 10 atomic shims delegate — Arc<Mutex> clone-shares-cell; compare_exchange Outcome +/− marshaling; cross-thread 4-worker share; end-to-end JIT via composite-ptr params. **Scope**: Atomic<Integer>; end-to-end-with-`Synchronized` gated on EnumNew codegen — atomic fns tier-down at enum construction until then. map_type now exhaustive-but-Long)
- [ ] **v0.10.x.jit.2b-iii** — Cliff shims (or defer v0.11): varargs (`FStringConcat`/`TextConcat` — fixed-arity ABI doesn't fit, needs array-ptr+len or boxed-args-Vector ABI; ADR-0032 §1 flagged "Mixed/unresolved") + file I/O ×5 (`ReadFile`/`WriteFile`/`WriteFileBytes`/`FileExists`/`ReadDirRecursive` — side-effects, non-deterministic parity, capability-gated, `ReadDirRecursive` returns `Vector<Tuple>`). Full 43-shim parity matrix (§7.2) + proptest fuzz (§7.3). Evaluate at the time: ship or clean-defer to v0.11 (they tier-down to VM = correct, just not JIT-accelerated).
- [ ] **v0.10.x.jit.3** — AOT cache via `cranelift-object` per ADR-0033. Add `cranelift-object` + `libloading` deps. `JitDispatcher` dual-path: AOT cache hit → object load + symbol resolve; miss → fresh `cranelift-jit` compile + persist to `~/.triet/store/jit/{triple}/{impl_hash}/`. `dao store gc` integration. ~800 LOC.
- [ ] **v0.10.x.jit.4** — Bootstrap gate lift + perf bench per ADR-0030 §14. Lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required (per ADR-0019 §7 Addendum chain). Add `criterion` bench measuring ≥10× v0.3 baseline on numeric programs + bootstrap < 10 min. ~150 LOC + benchmark fixtures.

### v0.10.x.thread — Multi-thread Atomic completion (3 sub-tasks)

- [x] **v0.10.x.thread.1** — `raw_thread.spawn` real OS thread impl per ADR-0026 v2 §3. Replace placeholder `spawn(work: Integer) -> Handle = Handle { thread_id: 0 }` with real OS thread creation. `Handle.join()` blocks until thread terminates. POSIX-first per ADR-0018 precedent (Windows stub OK). ~400 LOC + tests. — `68e8a0e` (+8 tests; `.triv` v6→v7 with self-host lockstep; spawned thread body empty per closure type system deferral; interpreter parity + JIT shim defer; thread bodies via `std::thread::spawn`)
- [x] **v0.10.x.thread.2** — Send-boundary refcount-bump codegen per ADR-0026 v2 §3.2. When `&+ T` crosses spawn boundary, emit refcount-bump on ObjectHeader (`triet-core::memory`). Matching Drop on thread join. User-visible: nothing changes; under the hood: multi-share enabled. ~300 LOC. — `98890a4` (+2 cross-thread tests; Plan B — Atomic `Rc<RefCell>`→`Arc<Mutex>` migration for Send infrastructure; real codegen + ObjectHeader integration defers v0.11+ when closure type system gains Send-bound expressiveness — no syntactic site for codegen at v0.10. Interpreter kept `Rc<RefCell>` — Value enum has Rc<…> children, !Send anyway. ~250 LOC)
- [x] **v0.10.x.thread.3** — `&+ Atomic<T>` multi-thread clone semantics + multi-worker demo per ADR-0028 §5 + ADR-0031 §10.2. Wire clone-on-Send-boundary path for `&+ Atomic<T>`; single-thread `&+` stays linear move per v0.9 .7d E2420. Reactivate 3-worker `atomic_counter` demo with concurrency assertion (counter eventually consistent ≥ 3 after all join). ~200 LOC + e2e test. — `e683e8e` (Plan A — Rust-level e2e proves Arc<Mutex> share works; counter == 3 deterministic. Triết-source multi-worker defers post-v0.10 — `spawn(closure)` + Send-bound closure types not yet available. Demo `.tri` comment updated with cliff explanation. ~300 LOC)

### v0.10.x.borrow — Borrow checker enforcement (3 sub-tasks, Tier 1B)

- [x] **v0.10.x.borrow.1** — E2440 NLL borrow exclusivity (full CFG live-range) per ADR-0025 §2 + ADR-0031 §10.1. Compute borrow-active region from creation to last-use; reject overlapping `&0 mutable` / `&0` / `&+` borrows. Biggest item — ~1000+ LOC. **Risk:** may tier-down further if 2-day budget too tight; .2 + .3 are lower-risk. — `3ea2f81` (+10 tests; 3-pass collect+live-range+conflict; branch isolation via event serialization; loop extension via marker pairs; bootstrap clean — no false positive on 23K LOC compiler self-host; field-granular base + inter-procedural + closures + param-vs-local defer v0.11+)
- [x] **v0.10.x.borrow.2** — E2400 lifetime elision 3 rules per ADR-0025 §3. Quy tắc 1 (single input borrow → output), quy tắc 2 (`self` receiver → output ties self), quy tắc 3 (owned return). E2400 fires when all 3 fail. ~300 LOC. — `1b78f94` (+14 tests; Rule 2 dormant pending `self`-parameter parser syntax; nested-borrow defer v0.11+ corpus-driven)
- [x] **v0.10.x.borrow.3** — E2403 `&-` weak observer upgrade + E2410/E2411 mutability per ADR-0022 §2 row 5 + v0.8.10 skeletons. E2403: deref `&- T → T?` upgrade tracking. E2410/E2411: assign-to-frozen + mutate-via-readonly-borrow full enforcement. ~400 LOC combined. — `b216ad0` (+12 tests; E2411 reroute + E2403 conservative direct-return; v0.8 skeleton message corrected `&0`→`&+`. E2410 dormant pending field-assign parser syntax; E2403 full owner-trail defers v0.11+ per §8.3)

### v0.10.final — release

- [ ] **v0.10.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` ✓✓✓✓ all 4 gates clean, Cargo 0.9.0 → 0.10.0, SPEC v0.9 → v0.10, README + ARCHITECTURE.md + ROADMAP + CLAUDE.md sync, ROADMAP archive sub-phase summary table, version bump commit độc lập (no bundling per cadence).

### v0.11 backlog (deferred from v0.10 Option B)

- `std.concurrency.*` stdlib (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

### Workflow note

Per ADR-0009 design-first principle, **v0.10.0.1 + v0.10.0.2 must lock before any impl sub-task starts**. ADR-0032 unblocks v0.10.x.jit.1; ADR-0033 unblocks v0.10.x.jit.3. The other workstreams (interp / thread / borrow) don't depend on new ADRs (existing ADR-0025 + ADR-0026 v2 + ADR-0031 cover).
