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

- [x] **v0.11.0.1** — [ADR-0033 Addendum] **Loader-approach decision** — locked **Path A (Triết owns its relocating loader)**, author sign-off. Path B (system-linker + `dlopen`) rejected: runtime C-toolchain dependency on the perf path + leans on `dlopen`/ELF dynamic linking, neither exists on balanced-ternary hardware. Path A honors OS-capable/from-scratch identity. 4 normative safety constraints for jit.3: CodeLoader trait, ELF/x86_64 POSIX-first, bounded reloc set + refuse-on-unknown, test regimen (round-trip value parity + proptest fuzz + W^X). Supersedes only ADR-0033 §3 framing; §3 symbol-resolution + §1/§2/§4–§10 unchanged. — `ee624ce`
- [ ] **v0.11.0.2** — [ADR-0033 Addendum] **Cache identity decision** — locked **key = canonical `impl_hash_mod` + per-module objects**, author sign-off. Rejects the content-hash shortcut (parallel identity space disjoint from the [ADR-0014] hash tree → would break `dao store gc` liveness; app-tier reasoning, wrong for OS-capable). GC aligns by construction (jit dir live iff `impl_hash_mod ∈ live_mods`). `triet-jit` stays `triet-pack`-independent: opaque key bytes + injected `trait AotCacheStore`; the caller computes `impl_hash_mod`. Loose runs without the canonical hash → not cached (refuse-over-guess, never fabricate a key). Supersedes Step 1's per-program `emit_object` → per-module (jit.4a).

### v0.11.x.jit — AOT cache implementation (depends on v0.11.0.1 + v0.11.0.2)

- [x] **v0.11.x.jit.3** — AOT cache implementation per ADR-0033. **Mechanism shipped (Steps 0–4b):**
  - Step 0 — generalize IR translator over `Module` trait — `3396f72`
  - Step 1 — object emission + version-pinned manifest (§2) — `cbeb102`
  - Step 2 — `Store::install_aot_cache` + `dao store gc` jit sweep (§4/§5/§7) — `2946ce4`
  - Step 3 — Path-A relocating loader `ElfX86_64Loader` (§3 + Addendum constraints 1–4; loader is unsafe-free) — `c47bd8f`
  - Step 4a — per-**module** object emission + **load-time linker** (v0.11.0.2 Entailment); cross-module 2-module program links + executes → 7 — `52a1cba`
  - Step 4b — wire Path A into `JitDispatcher` via injected `trait AotCacheStore` (opaque key) + §2 version-check + §8 silent fallback + `cache_state()`. §9.1 value-parity + §9.2 version-mismatch refuse, both via mock store. Dead code removed (no `#[allow]`). — `c7abe22`
### v0.11.x.jit.4 — JIT aggregate coverage → bootstrap gate lift (ADR-0034, Hướng A)

**Reframed by the coverage audit (`29aeeaa`):** `compiler/main.tri` is only **3.7% JIT-able** (96.3% tier down on struct/enum/Outcome/Nullable/String). The gate can't lift until the compiler is ~fully JIT-able. Author: "stop deferring — Hướng A." Cover the aggregate data model via delegate-to-VM shims per ADR-0034, re-measuring the audit after each sub-task (the burndown metric).

**Value model (ADR-0034 2026-06-01 Addendum — Bậc A):** **per-function uniform boxing.** All-integer functions keep today's unboxed fast path; any aggregate-touching function compiles fully-boxed (every SSA value an `Rc<RuntimeValue>` ptr, every opcode incl. `Add` a delegate-to-VM shim) → no box/unbox ambiguity → no miscompile, no IR/`.triv`/self-host change. Boxed path = the correctness **oracle** for the later native-codegen phase. Kernel-grade runtime speed comes from **Bậc C native aggregate codegen** (post-v0.11, own ADR), NOT this phase — v0.11 delivers the gate lift (coverage + cache).

- [x] **jit.4.audit** — JIT-coverage measurement tooling (`audit_jit_coverage` + `codegen::collect_tier_downs` + `jit_tier_down_audit.rs`). Finding: 146/3953 JIT-able. — `29aeeaa`
- [x] **jit.4.agg.0** — §6 translator panic → clean skip. Root cause: lowerer emits dead code after an early-`return` terminator within one block; codegen now stops at the terminator (Ret/Br/BrIf/BrTrilean/Unreachable). Re-audit: 0 panics. Regression test added. A real fix (removes a crash the production `compile_program` would hit), not just tier-down. — `89ca3fd`
- [ ] **jit.4.agg.1** — **introduce the per-function boxed codegen mode** (Bậc A): detect aggregate-touching functions, compile them fully-boxed (every value a ptr; boundary box/unbox from known param/return types). First aggregate ops on it: §1 struct (`StructNew`/`FieldGet`/`FieldSet`) via extracted `pub` VM helpers + delegate shims; §2 `StructNew` variadic array-ptr+len ABI (also unblocks deferred f-string varargs). Largest bucket (1314). This sub-task builds the infra agg.2–4 reuse.
- [ ] **jit.4.agg.2** — §1 enum ops (`EnumNew`/`EnumTag`/`EnumPayload`) + Outcome ops (`OutcomeDiscriminant`/wrap/unwrap). ~1489 combined.
- [ ] **jit.4.agg.3** — §1 Nullable ops + §3 String/Null constants + **loader `R_X86_64_64` data relocation** (extends `SUPPORTED_RELOC_TYPES` + the ADR-0033 constraint-4 regimen: value-parity + proptest fuzz + W^X). Only sub-task that re-touches unsafe loader.
- [ ] **jit.4.agg.4** — §4 Phi (Cranelift block params) + §5 multi-block shim codegen (lift jit.2b-i single-block restriction). Order may move earlier if it blocks re-measurement.
- [ ] **jit.4.gate** — once audit shows ~full coverage + warm-cache bootstrap < 10 min: wire CLI `AotCacheStore` adapter over `Store` (`jit/<triple>/<hex(impl_hash_mod)>/`) + compute `impl_hash_mod` on the packaged/bootstrap path + `enable_aot_cache` at `main.rs:824`; lift `stage2_eq_stage3_main_tri_byte_identical` off `#[ignore]`; `criterion` warm-vs-cold bench (≥10× on a JIT-friendly workload per ADR-0034 §8, + warm-cache bootstrap wall-time as gate evidence).
  - **Fold in the jit.3-review perf findings (#4/#5) here** — they're on the bench's cold-cache path:
    - **#4 — 2× codegen on cache miss.** `compile_program_cached` Path B runs `compile_program` (in-process JIT of all modules) AND then a per-module `emit_module_object` loop — every body lowered twice. Share one translation/emit pass.
    - **#5 — O(N²) cross-module declare pre-pass.** `declare_and_define_module` re-declares ALL functions of ALL modules per call → O(N²) for an N-module emit. Build the program-wide declaration table once + reuse.

### v0.11 backlog (trails AOT cache or later phase)

- **JIT shim gaps** — now folded into v0.11.x.jit.4 (ADR-0034): varargs `FStringConcat`/`TextConcat` → agg.1 §2 array-ptr+len ABI; multi-block-shim codegen → agg.4 §5; Ordering-`EnumNew` → agg.2 enum ops.
- **Borrow checker corpus-driven:** field-granular NLL base, inter-procedural borrow, closure captures, E2403 full owner-trail, Rule-2 elision (`self`-param parser), E2410 field-assign enforcement.
- **Concurrency closures:** `spawn(closure)` Send-bound closure types → real Send-boundary refcount-bump codegen (thread.2) + Triết-source multi-worker (thread.3).
- **`std.concurrency.*` stdlib** (Mutex, Channel, M:N green threads) per ADR-0028 §10 — feature-new scope, separate stdlib phase.

### Post-v0.11 — runtime-speed pillar (own phase + ADR)

- **Bậc C — native aggregate codegen** (ADR-0034 Addendum): real data layout (struct = N×i64 in registers/stack, `field_get` = `load`, no `RuntimeValue`/heap/shim) — the kernel-grade runtime-speed tier (VISION §4.3 production tier). Built incrementally on the v0.11 Bậc A boxed path as the **correctness oracle** (each native op verified to match the boxed/VM result on a corpus) + fallback for not-yet-native ops. Needs per-value type info (the cost Bậc B would have paid, now spent where it reaches the destination). Proposed v0.12 / pre-v1.0.

### Workflow note

Per ADR-0009 design-first, **v0.11.0.1 must lock before v0.11.x.jit.3 starts** — the loader approach determines the §3 symbol-resolution mechanism + the unsafe surface. ADR-0032's SHIM_TABLE is the symbol resolution source of truth either way (ADR-0033 §3).
