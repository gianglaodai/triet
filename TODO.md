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
- [ ] **v0.9.x.jit.4** — Builtin shim integration: opcodes 4-26 (Vec/HashMap/IO) + 27-39 (Atomic per ADR-0028).
- [ ] **v0.9.x.jit.5** — VM dispatcher integration: call-count threshold trigger (≥100 per ADR-0030 §2) + JIT compile path + native call thunk + `dev.jit_codegen` capability enforcement (per Addendum Gap 1).
- [ ] **v0.9.x.jit.6** — AOT cache filesystem layout per ADR-0030 §5 + invalidation by impl_hash.
- [ ] **v0.9.x.jit.7** — Stage 2 ≡ Stage 3 byte-identical gate verification + lift from `#[ignore]` to CI-required per ADR-0019 §7 Addendum.
- [ ] **v0.9.x.jit.8** — Perf bench: ≥10× v0.3 baseline on numeric-heavy programs; full 3-stage bootstrap < 10 phút.

### v0.9.final — release

- [ ] **v0.9.final** — Per ADR-0009 + Addendum §C: `scripts/release-check.sh` clean, Cargo 0.8.0 → 0.9.0, SPEC v0.9 header, README + ARCHITECTURE.md sync, version bump commit độc lập (no bundling per cadence).

### Workflow note

Trước khi bắt đầu sub-task v0.9.x đầu tiên (v0.9.0.1): hooks đã install và baseline clean. Per ADR-0009 Addendum §C, pre-version audit pass đã hoàn thành 2026-05-29.
