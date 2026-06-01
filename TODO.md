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
- [x] **jit.4.agg.1** — per-function boxed codegen mode (Bậc A) + struct ops. **agg.1a `3b48657`**: `pub` VM helpers `exec_struct_new/field_get/field_set` + delegate shims `__triet_struct_new`(array-ptr+len §2)/`__triet_field_get`/`__triet_field_set` + unit tests. **agg.1b `f13b660`**: `is_boxed`/`build_signature_for`/`emit_function_body(boxed)`/`translate_boxed_instruction` (StructNew via Cranelift stack-slot spill, FieldGet/FieldSet, Ret; rest tier down); end-to-end value-parity (make/first/set0 == VM). **Re-audit: JIT-able 146→344 (3.7%→8.7%); "struct ops" category eliminated.** Infra agg.2–4 reuse.
- [x] **jit.4.agg.1c** — **boxed CORE opcodes** (data-driven per §9 re-audit). Complete (i/ii/guard/iii/iv/v):
  - **1c-i `2f764de`**: binary scalar ops (Add..Ge + Ł3 Luk* + K3 Kleene* + Neg) via `JitBinOp` + `exec_jit_binop`/`exec_jit_neg` + `__triet_binop`/`__triet_neg` shims.
  - **1c-ii `91c83ed`**: primitive constants via `JitConstKind` + `exec_box_const` + `__triet_box_const` (Trit/Tryte/Integer/Trilean/Unit/Null; String/Long → agg.3). JIT-able 344→391.
  - **1c-guard `34d21bb`**: cross-mode call ABI guard — unboxed caller of a boxed callee tiers down (`ProgramContext.boxed_funcs`); closes a latent miscompile.
  - **1c-iii `3b4032a`**: boxed same-mode `CallLocal`/`CallCrossModule`/`WitnessCall` (`translate_boxed_call` — ptr args, ptr result; post-call sentinel propagates callee failure; non-boxed callee tiers down, symmetric to the unboxed guard). Cross-mode (boxed↔unboxed) marshaling still deferred. JIT-able 391→418 (9.9%→10.6%).
  - **1c-iv `ced2321`**: boxed branches `Br`/`BrIf`/`BrTrilean` via the total `__triet_trilean_tag` shim (delegates `exec_trilean_tag`→`as_trilean`) + the same icmp/brif dispatch as unboxed. **Multi-block drop safety**: the drop-at-Ret pass is sound only single-block (dominance); a multi-block boxed fn skips drops — bounded dev-tier leak (memory-safe, never a double-free), real drop placement = post-v0.11.
  - **1c-v `c520b6c`**: boxed **Phi** via Cranelift block params (`collect_block_phis` appends one I64 param per φ to non-entry blocks; `boxed_block_args` threads each branch's incoming value by predecessor `BlockId`). Forward if/else-merge JITs; loop-carried / malformed φ tier down. **Completes multi-block boxed scaffolding** (call + branches + Phi). Audit stays 418 — prerequisites, not a coverage gate; every boxed multi-block fn still tiers down on enum/Outcome/String. The 113 `φ` first-blockers are all-scalar **unboxed** fns (unboxed φ = separate out-of-scope path).
- [x] **jit.4.agg.2** — boxed enum + Outcome ops (the real coverage unlock).
  - **2a `69aafb0`**: enum ops `EnumNew`/`EnumTag`/`EnumPayload` (`exec_enum_*` + `__triet_enum_*` shims; payload presence as a separate i8 flag since a payload can be a boxed Null). **JIT-able 418→1112 (10.6%→28.1%, +694)** — enum was the dominant blocker. `assert_rv_eq` extended to Enum/Outcome/Trilean/Trit. Struct-payload round-trip test ([[triet_enum_struct_payload_identity]] as a JIT==VM check).
  - **2b `7c87178`**: Outcome ops `OutcomeNew{Positive,Negative,Null}`/`OutcomeDiscriminant`/`OutcomeUnwrap{Value,Error}` (`exec_outcome_*` + `__triet_outcome_*`; discriminant total w/ cross-tolerance, unwrap* finish_ptr → wrong-arm surfaces via per-call sentinel). **JIT-able 1112→1204 (28.1%→30.5%).** Wrong-arm-unwrap failure-path test (`dispatch_with_shim_errors`→Err).
- [~] **jit.4.agg.3** — Nullable ops + String/Null constants.
  - **3a `a10e673`**: Nullable ops `NullWrap`/`NullUnwrap`/`NullCheck` (`exec_null_*` + `__triet_null_*`; wrap/check total, unwrap finish_ptr → Null-unwrap surfaces via sentinel). **JIT-able 1204→1298 (30.5%→32.8%).** Null-unwrap-on-Null failure-path test.
  - [ ] **3b (NEXT — UNSAFE, plan-before-code)**: String + Null constants + **loader `R_X86_64_64` data relocation** (extends `SUPPORTED_RELOC_TYPES` + the ADR-0033 constraint-4 regimen: value-parity + proptest fuzz + W^X). The ONLY sub-task that re-touches the hand-rolled ELF loader (highest mem-corruption risk in the project). **In-process JIT note:** the audit uses the in-process `JITModule`, so String const support could land via Cranelift `DataDescription` (no manual reloc) for the audit/in-process path FIRST, deferring the AOT `.o` reloc — split 3b-i (safe, in-process) / 3b-ii (unsafe loader reloc) if author prefers. Null const (10) is already handled by `boxed_const_kind_payload`; the audit's Null-const blockers are UNBOXED fns (out of scope).
- [ ] **jit.4.agg.4** — §5 multi-block shim codegen (lift jit.2b-i single-block restriction) + precise multi-block drop placement (SSA liveness/dominance — supersedes the agg.1c-iv leak-skip). §4 Phi DONE in 1c-v. Order may move earlier if it blocks re-measurement.
- [x] **jit.4.agg.cross-call** — cross-mode call marshaling + the refcount discipline ([ADR-0035](docs/decisions/0035-jit-boxed-refcount-discipline.md), Locked).
  - **cross-call.a `0732a35` (boxed→unboxed scalar)**: `boundary_class` + `__triet_unbox_scalar`/`emit_box_scalar`/`emit_unbox_scalar` + `func_sigs` in `ProgramContext`. 1298→1318.
  - **clone-on-return boxed `b90dfed`**: closed the same-mode boxed latent double-free (borrowed-param return). `__triet_clone_arc` + `emit_clone_arc`; boxed Ret clones iff returned `Value(id)` ∉ created_boxed.
  - **ADR-0035 `d57e5b1`→Locked**: unified clone-on-return discipline — *a Ret transfers exactly one owned ref; clone any borrowed return to mint it*. Author sign-off "triển khai Bậc A" (leak-tolerant over no-leak; confirmed non-conflicting with Bậc C).
  - **§1 unboxed + §2 composite `78c5ad4`**: unboxed Ret clones a borrowed COMPOSITE return (TypeTag-guided `is_composite_tag` — scalars never cloned), fixing the unboxed latent double-free (since jit.2b). Cross-mode composite = pure PASS-THROUGH (no caller clone, leak-free): §1-both-modes makes every return owned → boxed caller records cross-mode composite result as owned + composite arg borrowed by callee. **1318→1622 (33.3%→41.0%, +304).** cross-mode blockers 1257→410.
  - **cross-call.b `6987115` (unboxed→boxed scalar)**: symmetric in `translate_call` (box scalar args → call → unbox result + drop temp boxes; `fn_state` threaded). Coverage unchanged (1622) — self-host has ~no unboxed→boxed scalar call sites. **First DeepSeek trial under HANDOFF_PROTOCOL.md — passed clean** (test verbatim, no prohibitions, Opus re-verified). Closes the cross-call matrix.
- **REMAINING BLOCKERS after cross-call (audit @ 41.0%):** `cross-mode` 410 + `call` 192 + `Constant` 131 + `String constant` 127 + `φ` 52.
  - **`TypeTag::Unit` ceiling ([ADR-0035 §4]) = the 410 cross-mode + much of the 192 call.** AST nodes lower to `TypeTag::Unit` (`lowerer.rs` `_ => TypeTag::Unit`), ambiguous with true-Unit → cross-mode Unit boundaries tier down. Lifting needs the lowerer to distinguish user-struct/enum/generic from true-`Unit` — an **IR-shape change** (new `TypeTag` variant(s) + `.triv` bump + self-host lockstep). Biggest remaining lever but a careful, bigger slice — its own ADR.
  - `String`/`Null`/`Long` **`Constant`** (127+131) = **agg.3b** (below).
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
