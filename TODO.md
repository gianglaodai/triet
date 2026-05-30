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

### v0.9.x.jit — Cranelift JIT backend (per ADR-0030 §11)

- [x] **v0.9.x.jit.1** — Scaffold `triet-jit` crate. Cargo.toml với cranelift-codegen + cranelift-frontend + cranelift-jit + cranelift-module pinned 0.132 + thiserror + triet-ir. lib.rs skeleton (228 lines): `pub struct JitCompiler` với `HashMap<FuncId, NativeCodePtr>` cache; `pub struct NativeCodePtr { addr: usize }` opaque pointer wrapper; `pub enum JitError` 4 variants (Unimplemented / UnsupportedOpcode / Cranelift / CapabilityDenied per Addendum Gap 1); `compile`/`lookup`/`cached_function_count` stubs. Workspace Cargo.toml +2 lines (member + dependency). 4 scaffold smoke tests. Cold build adds ~10 Cranelift transitive deps per ADR-0030 ~5MB cost budget. No `unsafe` blocks yet — workspace `unsafe_code = "forbid"` honored; override deferred .5 codegen. 1506 → 1510 — `d1fcd55`.
- [x] **v0.9.x.jit.2** — Opcode-by-opcode translation per ADR-0030 §3-§4. New `crates/triet-jit/src/codegen.rs` (~340 lines) + `JitBackend` lazy-init via `cranelift_native` host ISA detection. Supported: `Add`/`Sub`/`Mul`/`Neg` via iadd/isub/imul/ineg; `Eq`/`Ne`/`Lt`/`Le`/`Gt`/`Ge` via icmp + Trilean re-encoding `2*raw - 1` ({-1,+1}); `Br`/`BrIf` (deprecated 2-way, treats Unknown as False via `cond == +1` check); `BrTrilean` per ADR-0010 §4 backend (2 icmp + 2 brif via intermediate fallthrough block — Triết Trit::True=+1, Unknown=0, False=-1); `Ret` with/without value. Type mapping: Trit/Trilean/Unit → i8, Tryte → i16, Integer → i64, Long → UnsupportedOpcode (defer pair-of-i64). `Const` operand + calls + builtins + aggregates + Phi raise `JitError::UnsupportedOpcode` for tier-down per ADR-0030 §2. **No execution tests** — fn-pointer cast requires `unsafe`, lands .5; Cranelift's internal verifier already rejects malformed IR pre-`finalize_definitions`. 8 new tests: identity / arith / comparison → Trilean / multi-block Br / BrIf / BrTrilean 3-way / negative Const + CallLocal tier-down. 1510 → 1518 — `e3e585a`.
- [x] **v0.9.x.jit.3** — Call dispatch + Const wiring per ADR-0030 §3 + ADR-0012 §2. 3 files / +681 lines: new `pub fn compile_program(&IrProgram)` entry; `ProgramContext { func_id_map, path_to_funcid, &ConstantPool }` threaded through codegen; two-pass shape (declare-all → emit-bodies → finalize); per-function tier-down silently skips ClosureCall/aggregate/builtin etc. while the rest of program JITs. `CallLocal` via `declare_func_in_func + builder.ins().call`; `CallCrossModule` via path lookup → same dispatch; `WitnessCall` identical (v0.4 informational tables per ADR-0012 §2). `materialize_constant` covers Integer/Tryte/Trit/Trilean/Unit with ADR-0010 §3 trit encoding. Name mangling `{name}__f{id}` avoids cross-module collision in single JITModule. 5 program-level tests + 2 single-fn negative tests rewritten. 1518 → 1523 — `d3b87eb`.
- [x] **v0.9.x.jit.4** — Structured `CallBuiltin` tier-down diagnostic + ADR-0030 §12 v0.10 backlog (Option A per author "implementer's choice — không ảnh hưởng cú pháp"). Full builtin shim layer defers v0.10 due to `RuntimeValue` ABI marshaling complexity (43 builtins × `Rc<RefCell>`/String/Vector ABI). Ships ~50 LOC: explicit `Instruction::CallBuiltin` arm naming the builtin via `Display` impl; catch-all `other =>` arm switched from Debug to Display formatting (s-expr form). 3 new tests (name diagnostic + arg count + program-level skip). ADR-0030 §12 NEW captures: §12.1 scope reality (43 builtins × marshaling table), §12.2 five design constraints (ABI representation / lifetime mgmt / capability gate / panic propagation / unsafe override), §12.3 v0.9 stop-gap behavior, §12.4 decision rationale citing Phương án A precedent. 1523 → 1526 — `a1ab789`.
- [x] **v0.9.x.jit.5** — VM dispatcher integration. 6 files / +661 lines: `pub trait JitDispatch` in triet-ir (zero deps, breaks cycle); `Vm::jit: Option<Box<dyn JitDispatch>>` + `set_jit_dispatcher`/`disable_jit` setters; `CallLocal` Tier-2 path (record + try_dispatch + skip frame on Some); `JitDispatcher` runtime façade in triet-jit (compiler + counters + one-shot whole-program-compile semantics); `pub fn dispatch_integer` with workspace's **first and only `unsafe` block** (single auditable site, safety contract documented at fn level — Cranelift codegen invariants guarantee i64 signature × arity ≤ 4 × SystemV CC); `JIT_THRESHOLD=100` per ADR-0030 §2 Hotspot convention; `is_jit_integer_dispatchable` signature gate. Lint override: triet-jit only opts `unsafe_code: forbid → deny` (workspace inherit dropped). CLI: `--no-jit` flag + `TRIET_JIT=disabled` env var per Addendum Gap 3; auto-install dispatcher in `run_bytecode` (interpreter path no-op). Capability `dev.jit_codegen` runtime check deferred v0.10 per §12 backlog (env var serves immediate kernel/embedded need). Wider type coverage + `TRIET_JIT_THRESHOLD` env override deferred v0.10. 10 new tests (signature/identity/2-arg add/threshold-at-100/post-compile try_dispatch/Vm-with-and-without dispatcher/disable_jit). **First sub-task that actually EXECUTES JIT-compiled native code end-to-end via Vm.** 1526 → 1536 — `7509c5b`.
- [x] **v0.9.x.jit.6** — AOT cache **deferred v0.10** per ADR-0030 §13 NEW backlog (Option A, mirrors .4 precedent). Architecture finding: `cranelift-jit::JITModule::finalize_definitions` mmaps RX pages in current process address space with absolute addresses + libcall references — NOT position-independent code, can't be naïvely serialized. Real AOT cache needs `cranelift-object` backend swap (emit ELF/.o with relocation records → object load + `libloading` resolve at load time). §13 covers: §13.1 why current backend can't cache, §13.2 cranelift-object swap plan, §13.3 filesystem layout (target_triple + impl_hash from ADR-0014), §13.4 five design constraints (Cranelift version pinning, libcall resolution, GC integration, cross-machine portability, determinism preservation), §13.5 v0.9 stop-gap (full re-JIT per `dao run`), §13.6 rationale (coherent v0.10 phase pairs §12 builtin + §13 cache + §7 gate-lift). lib.rs doc updated: .4/.6 marked deferred, .7 annotated as blocked by .6 (no cache → full re-JIT → bootstrap time prohibitive). No source changes, no test delta. — `5419680`.
- [x] **v0.9.x.jit.7+.8** — **Both deferred v0.10** per ADR-0030 §14 NEW rollup (single combined deferral per 2-day v0.10 timeline 2026-05-30). `.7` bootstrap byte-identical gate lift blocked by `.6` AOT cache absence (3000-fn self-host × cold JIT cost prohibitive without persistent cache per §13.5). `.8` perf bench measuring partial JIT (most builtins tier-down per §12) understates architectural value — defer to alongside `.4` builtin shim completion for honest "what JIT bought us" measurement. v0.9.x.jit phase closes at `.6`. Shipped 6 sub-tasks (`.1`-`.3`+`.5` real impl; `.4`+`.6` deferred with backlog). Net achievement: first Cranelift native execution + Tier-1/2 graduation + workspace single audited unsafe block + partial coverage (numeric arith/cmp/control flow + intra-program calls). v0.10 absorbs `.4` builtin shim + `.6` AOT cache + `.7` gate lift + `.8` bench.

### v0.9.final — release

- [ ] **v0.9.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` clean, Cargo 0.8.0 → 0.9.0, SPEC v0.9 header, README + ARCHITECTURE.md sync, version bump commit độc lập (no bundling per cadence).

### Workflow note

Trước khi bắt đầu sub-task v0.9.x đầu tiên (v0.9.0.1): hooks đã install và baseline clean. Per ADR-0009 Addendum §C, pre-version audit pass đã hoàn thành 2026-05-29.
