# ADR 0030 — JIT Integration (Cranelift backend)

**Status:** **Locked** (v0.9.0.3, author sign-off 2026-05-29). Refines [ROADMAP §v0.9](../../ROADMAP.md) JIT deliverables. The author confirmed 4 architecturally significant decisions: §1 3-tier model (Interpreter + VM + JIT, no JIT-only); §2 100-call threshold (Hotspot JVM convention); §6 no capability gate for default `usr.*` programs; §7 synchronous JIT for v0.9 (background deferral in v1.0+). First ADR using the [ADR-0029 §5](0029-self-host-port-policy.md) Self-host port plan template — see §10.

> **2026-05-29 Addendum (v0.9.0.3.c):** Self-review post-lock identified 3 architectural gaps requiring resolution before implementation phase v0.9.x.jit starts:
>
> **Gap 1 — JIT codegen IS a privileged operation.** §6 original wording "JIT is part of runtime, no more privileged than VM is" was wrong about fundamentals: the VM does not write executable memory; the JIT does (W^X mmap RW$\rightarrow$RX flip). Kernel modules cannot allocate RWX pages; hardened userspace runtimes (SELinux, macOS Hardened Runtime) require entitlements for JIT. Per [VISION §3.5](../../VISION.md) Pillar 5 capability + Pillar 4 OS-capable, JIT must have a capability gate.
>
> **Resolution:** Add capability `dev.jit_codegen` — required for the JIT codegen path in any program. Default ambient for `usr.*` (user-mode programs get JIT for free, matching current §6 intent). Kernel/embedded programs explicitly DENY via `dao.package requires dev.jit_codegen deny` — runtime detects and falls back to VM-only mode automatically (NOT an error; per BYOS philosophy ADR-0026 v2, runtime features are external/optional). Capability ambient resolution per [ADR-0016 §5](0016-capability-type-system.md) — no friction for default `usr.*`.
>
> **Gap 2 — Tier model naming diverged from VISION §4.2.** ADR §1 used "Tier 0/1/2" but VISION §4.2 lists "Backend 1: VM / Backend 2: JIT / Backend 3: AOT / Backend 4: Trytecode" with the interpreter as an auxiliary "Development tier (still runs alongside VM)". Realigned:
>
> - **Backend 0 (auxiliary dev):** tree-walking interpreter — debug fallback, opt-in via `dao run --interpret` flag. NOT a graduation tier; runs alongside Backend 1.
> - **Backend 1: bytecode VM** (existing v0.3 baseline; cold path + warmup + JIT-disabled fallback).
> - **Backend 2: Cranelift JIT** (NEW v0.9; hot path post-graduation per §2 100-call threshold).
> - **Backend 3: AOT native** (v2.0 LLVM future).
> - **Backend 4: Trytecode native** (v∞ future).
>
> Per-function graduation rule (§1 original) unchanged. Naming alignment only; semantics preserved.
>
> **Gap 3 — Missing explicit JIT-off escape hatch + real-time disclaimer.**
>
> - **CLI flag `--no-jit`**: disables Backend 2 graduation entirely, running pure Backend 1 VM. For debugging, reproducibility, sandboxed environments. Persistent via `TRIET_JIT=disabled` env var.
> - **Real-time suitability note** (added to §7 Consequences): "JIT is NOT suitable for hard real-time contexts (~1-3s pause on first trigger of hot function is unpredictable). Real-time kernel code should use Backend 1 VM (deterministic dispatch latency) or wait for Backend 3 AOT (v2.0) / Backend 4 Trytecode (v∞)."
>
> **Addendum scope:** §6 capability gate semantics changed (no-gate $\rightarrow$ ambient-default-with-deny-fallback); §1 tier naming realigned with VISION §4.2; §7 real-time note added. The ADR-0030 body is NOT edited per the project ADR immutability rule; the Addendum is authoritative for these 3 gaps. The v0.9.x.jit implementation phase uses Addendum semantics, not original §1/§6/§7 text.
>
> Cross-references: [ADR-0016 §5](0016-capability-type-system.md) (capability ambient resolution rule); [ADR-0026 v2 §6 BYOS](0026-actor-boundary-send-rules.md) (runtime features external philosophy); [VISION §3.5 + §4.2](../../VISION.md) (Pillar 5 + backend tier naming).

**Issue:** ROADMAP §v0.9 positions JIT as the primary v0.9 deliverable: "Tier 2 Cranelift JIT for frequently executed functions (profile-guided)" + "AOT cache: second execution uses cached JIT-output". Plus carry-forward perf gates: benchmark $\ge$ 10× v0.3 baseline, full 3-stage bootstrap < 10 minutes, Stage 2 ≡ Stage 3 byte-identical gate lifted from `#[ignore]` $\rightarrow$ CI-required.

Open questions ADR-0030 must resolve:

1. **Tier model** — Bytecode VM stays? Or JIT-only post-warm-up?
2. **Trigger heuristics** — call-count threshold? function-size weighting? Tier-down on JIT failure?
3. **IR $\rightarrow$ Cranelift translation** — direct mapping or intermediate canonicalization?
4. **BrTrilean lowering** — per [ADR-0010](0010-ternary-native-ir.md) backend table 2-cmp-2-branch; specifics?
5. **AOT cache layout** — directory structure? invalidation key? determinism guarantee?
6. **Capability gate** — `sys.jit` required? Or ambient like `vm.run`?
7. **Threading** — synchronous JIT (compile-on-trigger) or background JIT thread?
8. **Lowerer determinism preservation** — does JIT codegen require determinism? Same `.triv` $\rightarrow$ same machine code?
9. **Stage 2 ≡ Stage 3 byte-identical gate** — lift conditions?
10. **WitnessCall + cross-package dispatch** — JIT handling?
11. **Self-host port plan** — per ADR-0029 §5 (new): JIT is Layer C (runtime), self-host port not required.

---

## §1 — Tier model: 3-tier (Interpreter $\rightarrow$ VM $\rightarrow$ JIT)

**Author review required.**

**Decision:** Triet runtime has 3 tiers, ordered by call-count graduation:

```
Tier 0 — Tree-walking interpreter  (existing v0.2; dev tier, debug-friendly)
Tier 1 — Bytecode VM register-SSA  (existing v0.3; dev tier, baseline)
Tier 2 — Cranelift JIT             (NEW v0.9; production-feasible warm tier)
```

(Future v2.0 LLVM AOT = Tier 3; future v∞ trytecode native = Tier ∞ per [VISION §4.3](../../VISION.md).)

**Graduation policy:**
- Default entry tier: **Tier 1 (VM)**. Tier 0 (interpreter) only when `dao run --interpret` flag is set (debugging).
- Functions graduate **Tier 1 $\rightarrow$ Tier 2** when call count exceeds the threshold (§2).
- No tier-down: once a function JITs, it stays JIT (no de-optimization in v0.9). A future ADR can add tier-down if profile-guided re-specialization is needed.

**Per-function graduation, not per-program.** Cold functions stay on VM; hot functions graduate to JIT. Mixes seamlessly: a VM caller can call a JIT'd callee and vice versa via a shared calling convention.

**Why keep VM as Tier 1 (not skip to JIT)?**

- **Warmup time**: pure-JIT entry pays compilation cost on first call. VM dispatches instantly.
- **Cold code path**: most programs have a hot/cold distribution (Pareto). Cold code = 90% of code, executed rarely $\rightarrow$ VM execution is cheaper than JIT compilation cost.
- **Bootstrap path**: the self-host compiler compiles itself $\rightarrow$ many cold-path functions (parsers, typecheck rules). VM keeps cold paths lightweight.
- **Debug-friendly fallback**: if JIT produces incorrect code, falling back to VM is a known-good safety net.

**Rejected alternative:** JIT-only (replacing VM). Pros: simpler runtime. Cons: cold-start cost, no fallback, breaks v0.3 contract that the VM is "stable IR's executor".

---

## §2 — Trigger heuristics: call count threshold

**Author review required.**

**Decision v0.9:** Function graduates Tier 1 $\rightarrow$ Tier 2 when **call count $\ge$ 100**. Threshold is encoded in `triet-ir::JitConfig::trigger_threshold`, runtime-configurable via `TRIET_JIT_THRESHOLD` env var (escape hatch for benchmarking and tuning).

**Trigger detection:**
- Each `FuncId` tracks a per-process call count in `Vm::dispatch_counters: HashMap<FuncId, u32>`.
- On every cross-function call (CallCrossModule, WitnessCall, etc.), the counter increments.
- When counter $\ge$ 100, the dispatcher attempts JIT compilation and replaces VM dispatch with a native call thunk for that `FuncId`.

**Why 100?** Industry heuristic from Hotspot JVM (Tier 1 $\rightarrow$ 2 at ~100 calls). Conservative — avoids JIT'ing one-shot functions. Aggressive enough to catch loops (a loop body with 100 iterations = 100 dispatches).

**Rejected alternatives:**

- **Always-JIT (threshold = 1)**: compiles every function. Wastes compilation budget on cold code.
- **Profile-then-JIT** (collect profile for N runs, then JIT on next run): adds complexity for marginal benefit at v0.9 scale.
- **Function-size weighting** (large functions JIT first): premature optimization; size does not always correlate with hotness.
- **Cycle-counting** (instrument loops): adds runtime overhead in the VM.

**Tier-down on failure:** if Cranelift fails to compile (e.g., unsupported opcode), log a warning and continue VM dispatch. No retry; the function stays permanently in the VM tier for this session.

---

## §3 — IR $\rightarrow$ Cranelift IR translation

**Decision:** Direct register-SSA mapping. Triet IR is already register-based SSA (per [ADR-0007](0007-ir-design.md)); Cranelift IR is SSA-based with explicit basic blocks. Translation is 1:1 per opcode.

**New crate:** `triet-jit` — sibling of `triet-ir`, depends on `cranelift-codegen` + `cranelift-jit`. Public API:

```rust
pub struct JitCompiler {
    cranelift_ctx: cranelift_jit::JITModule,
    function_cache: HashMap<FuncId, *const u8>,  // native code pointers
}

impl JitCompiler {
    pub fn compile(&mut self, func: &triet_ir::Function) -> Result<*const u8, JitError>
    pub fn lookup(&self, id: FuncId) -> Option<*const u8>
}
```

**Calling convention:** Cranelift default (System V on Linux, Microsoft x64 on Windows). Triet values map to Cranelift types:

| Triet Type | Cranelift Type (binary CPU) |
|---|---|
| `Trit` | `i8` (uses 3 distinct values `{-1, 0, +1}` packed) |
| `Tryte` | `i16` |
| `Integer` | `i64` |
| `Long` | `i128` (Cranelift extension; or pair-of-i64 if unsupported) |
| `Trilean` | `i8` (same encoding as Trit) |
| `T?` discriminator | `i8` for trit, plus payload registers |
| `&+ T`, `&0 T`, `&-` | `i64` pointer (with ObjectHeader RC tracking via Rust runtime calls) |

**Opcode translation table (selected):**

| Triet IR Opcode | Cranelift IR Pattern |
|---|---|
| `IntegerAdd` | `iadd` |
| `IntegerMul` | `imul` |
| `IntegerCmp::Eq` | `icmp eq` $\rightarrow$ producing `i8` (extend) |
| `BrTrilean { value, neg, zero, pos }` | 2 `icmp` + 2 `brnz` per ADR-0010 backend table |
| `CallLocal { func, args }` | `call $func, $args` (intra-module direct call) |
| `CallCrossModule { path, args }` | indirect call through dispatcher table (resolved at compile-time if possible, else runtime lookup) |
| `WitnessCall { table_id, method_index, args }` | indirect call through witness table per [ADR-0012](0012-witness-table-dispatch.md) |
| `Constant::Null` | `iconst.i8 0` (Trit::Zero per [ADR-0010 Addendum §C](0010-ternary-native-ir.md)) |
| Builtin opcodes 4-26 (Vec/HashMap/IO) + 27-39 (Atomic per ADR-0028) | Rust runtime function call (`extern "C"` shim) |

**Builtin shim integration:** Builtins lower to `call $rust_builtin_<id>` (extern "C" function in `triet-jit` linking Rust runtime). The [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md) Rust-shim approach is maintained.

---

## §4 — `BrTrilean` lowering per ADR-0010

**Decision:** `BrTrilean { value: i8, neg_block, zero_block, pos_block }` lowers to 2 compare + 2 branch instructions on binary CPUs:

```
        ; Cranelift IR
        v100 = icmp eq value, iconst.i8(-1)   ; check Trit::Negative
        brnz v100, neg_block
        v101 = icmp eq value, iconst.i8(0)    ; check Trit::Zero
        brnz v101, zero_block
        jump pos_block                        ; fallthrough = Trit::Positive
```

**Order chosen (Negative $\rightarrow$ Zero $\rightarrow$ Positive):** matches the v0.7 VM dispatcher order. Empirically, Trit::Zero is the most common branch in `T?` null-check patterns; future profile-guided reordering may adjust this. Deferred.

**On hypothetical trytecode CPU (v∞ scope):** `BrTrilean` lowers to a single native instruction (per ADR-0010 backend table). A future v∞ ADR will refine this.

---

## §5 — AOT cache layout + invalidation

**Decision:** Cache native code by **`impl_hash` of the function's owning module** per the [ADR-0014](0014-hash-scheme-refinement.md) hash tree.

```
~/.triet/store/
├── pkg/{impl_hash}/...     (existing — package storage)
├── term/{impl_hash}/...    (existing — term storage)
└── jit/                    (NEW v0.9)
    └── {target_triple}/    (e.g., x86_64-unknown-linux-gnu)
        └── {impl_hash}/    (module-level hash from ADR-0014)
            ├── functions.bin       (serialized JIT'd machine code)
            └── manifest.bin        (FuncId → offset, calling convention)
```

**Per-target-triple separation:** cache is invalid cross-architecture (x86_64 code unusable on ARM64). Triple is obtained from Rust `std::env::consts::ARCH` + OS detection.

**Invalidation:** A module's `impl_hash` changes when any function in the module changes $\rightarrow$ the old cache directory becomes orphaned. Tied to existing `dao store gc` mark-and-sweep (ADR-0015 §6). The JIT cache is **roots-tracked**; deleted alongside `pkg/{hash}/` when the module is GC'd.

**Determinism (cache hit/miss):**

- Cache hit requires: same `{target_triple}/{impl_hash}/` directory exists + Cranelift codegen version matches.
- Cranelift version is pinned in workspace `Cargo.toml`; bumping invalidates all cached entries (full re-JIT).
- Cache hit/miss can vary across runs (e.g., first cold run misses; warm runs hit). **NOT a determinism violation** per [ADR-0007 §IR](0007-ir-design.md) — IR is deterministic; runtime cache state is not required to be.

**First-run cost:** ~1-3 seconds per `main.tri` function JIT (Cranelift O0 codegen). Subsequent runs amortize. Self-host bootstrap: ~3000 functions $\times$ cold JIT $\approx$ 9-30s overhead first time; cached thereafter.

---

## §6 — Capability gate

**Author review required.**

**Decision v0.9:** **No capability gate** for JIT in default `usr.*` programs. JIT is part of the runtime, no more privileged than the VM.

**Exception:** `dev.jit_unsafe` capability for power-user APIs exposing JIT internals (e.g., manual recompilation triggers, JIT codegen options). v0.9 ships **no public API** for these — they remain internal to the `triet-jit` crate. `dev.jit_unsafe` is reserved for future stdlib bindings.

**Sandboxing concern (W^X):** JIT codegen writes code into RWX-mapped pages. On hardened systems (SELinux, macOS Hardened Runtime), this requires entitlements. The Triet runtime handles this via:

- Linux: `mmap(PROT_READ|PROT_WRITE)` $\rightarrow$ write code $\rightarrow$ `mprotect(PROT_READ|PROT_EXEC)` flip.
- Detection of W^X policy mismatch (e.g., grsecurity) $\rightarrow$ fall back to VM-only mode, log warning.

**Rationale for no capability:** matches VM execution which does not require capabilities. JIT is semantically equivalent — same IR, faster execution. The capability boundary exists at IR generation (compile-time, already handled by `sys.*`/`dev.*` namespace checks), not at the execution backend.

**Rejected:** `sys.jit` mandatory capability. Pros: explicit acknowledgment. Cons: every `usr.*` program would need it $\rightarrow$ friction without security benefit (capabilities apply at IR level, not codegen level).

---

## §7 — Threading model

**Author review required.**

**Decision v0.9:** **Synchronous JIT compilation** on the dispatcher thread. When trigger fires (call count $\ge$ 100), the VM dispatcher blocks, invokes Cranelift compilation, replaces the dispatch entry, and continues execution.

**Latency cost:** ~1-3s pause on first trigger of each hot function. Acceptable for v0.9 (dev/CI scenarios). Production interactive applications may notice — addressed in v1.0+ post-ADR work.

**Future v1.0+ ADR:** background JIT compilation thread, lock-free patch-in via atomic pointer swap. More complex; deferred until profiling indicates real interactive stutter.

**Alternative considered:** Async JIT trigger (warm thread runs compilation, VM continues at Tier 1). Cleaner UX but adds threading complexity to v0.9 scope. Rejected.

---

## §8 — Lowerer determinism preservation

**Decision:** JIT does NOT require same-machine-code determinism. Per [ADR-0007](0007-ir-design.md), the determinism contract operates at the IR level (`.triv` is deterministic given `.tri` input). JIT machine code is an implementation detail.

Concretely: `dao build foo.tri -o foo.khi` produces byte-identical `.khi` (passes `bootstrap_determinism` test). `dao run foo.khi` JIT compiles $\rightarrow$ machine code may differ across:
- Cranelift versions (pinned in workspace, but upgradeable).
- Target triple (x86_64 vs. ARM64 = different ISA).
- Optimization passes (v0.9 = O0 only; future tunable).

**Cache hits are deterministic per-target-triple per-Cranelift-version.** Two runs on the same machine with the same Triet toolchain version yield identical cached machine code. Across different machines, no guarantee is made.

**Impact on Stage 2 ≡ Stage 3 byte-identical:** Stage 2/3 compare `.khi` bytes (IR output of self-host), NOT machine code. JIT acceleration enhances execution speed of Stage 2/3 themselves; it does not alter the `.khi` they produce.

---

## §9 — Stage 2 ≡ Stage 3 byte-identical gate lift

**Decision:** Per ROADMAP §v0.9 Functional gate: lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required when:

1. JIT enables Stage 2 (Triet-implemented compiler) to compile `main.tri` in **< 5 min** on dev hardware. (Pre-JIT: ~15+ min per ADR-0019 §7 Addendum measurement.)
2. JIT enables the full 3-stage bootstrap loop in **< 10 min** total per ROADMAP §v0.9 Gate.
3. Stage 2 ≡ Stage 3 `cmp` comparison produces identical bytes (deterministic IR per §8).
4. ADR-0029 §6 cross-reference: self-host port lockstep is maintained throughout v0.9 $\rightarrow$ Stage 2 can read current Triet source.

**Verification mechanism:** New CI test `crates/triet-bootstrap/tests/bootstrap_loop.rs::stage2_eq_stage3_with_jit` (removes `#[ignore]` once perf gate is reached). Test creates `Vm` with JIT enabled, runs Stage 2 `main.tri` compilation, and compares output bytes of Stage 2 vs. Stage 3.

**Carry-forward note:** ADR-0019 §7 Addendum tied this lift to JIT; ADR-0030 §9 confirms the timeline.

---

## §10 — Self-host port plan (per ADR-0029 §5 template)

**Layer A surface changes:** **No.** JIT is an internal runtime layer; no lexer, parser AST, or SPEC grammar changes.

**Layer B internal changes:** **No.** Typecheck, lowerer, and IR shape are unchanged. JIT consumes existing IR.

**Layer C runtime changes:** **Yes.** New `triet-jit` crate; `Vm` integrates JIT dispatch. Self-host (`compiler/`) does not observe this — self-host produces `.khi` (IR bytecode); JIT is a runtime consumer of the bytecode.

**Same-phase port required:** **No.** Per ADR-0029 §3 Layer C independent timeline rule, a self-host port is not required.

**Bootstrap interaction:** Stage 2 (Triet-implemented compiler) benefits from JIT acceleration (per §9 gate lift) but Stage 2 source code does NOT change. It is the same `compiler/*.tri` running on a faster runtime backend.

---

## §11 — Implementation sub-phase plan (v0.9.x.jit)

**Sub-task ordering** (informational — exact breakdown lands in v0.9.x.jit.N sub-tasks):

1. **v0.9.x.jit.1** — Scaffold `triet-jit` crate. `Cargo.toml` + `lib.rs` skeleton + Cranelift dependency pinned.
2. **v0.9.x.jit.2** — Opcode-by-opcode translation: arithmetic + comparisons + control flow (`BrIf`, `BrTrilean`).
3. **v0.9.x.jit.3** — Call dispatch: `CallLocal` + `CallCrossModule` + `WitnessCall`.
4. **v0.9.x.jit.4** — Builtin shim integration (opcodes 4-26 + 27-39 for Atomic).
5. **v0.9.x.jit.5** — VM dispatcher integration: trigger detection + JIT compile path + native call thunk.
6. **v0.9.x.jit.6** — AOT cache filesystem layout + invalidation.
7. **v0.9.x.jit.7** — Stage 2 ≡ Stage 3 gate verification + lift from `#[ignore]`.
8. **v0.9.x.jit.8** — Perf bench: $\ge$ 10× v0.3 baseline on numeric-heavy programs; bootstrap < 10 min.

Each sub-task represents an independent commit per cadence.

---

## §12 — v0.10 backlog: full builtin shim layer (revealed by v0.9.x.jit.4)

**Addendum 2026-05-30:** v0.9.x.jit.4 implementation surfaced that the original §3 "Builtin shim integration" item is **substantially more complex** than other ADR-0030 §3 opcode-translation tasks because it requires cross-ABI marshaling of Triet runtime values. Per author "slow and steady" stance, v0.9 ships ONLY the structured tier-down diagnostic — functions calling stdlib builtins tier-down to VM dispatch, with an error message naming the specific builtin instead of a generic Debug fallback. Full shim layer defers to v0.10.

### 12.1 — Why deferred (scope reality)

43 builtins across categories — virtually all require non-primitive ABI marshaling:

| Category | Builtins | Marshaling Complexity |
|---|---|---|
| I/O | `Println` / `Print` | String args via `*const u8 + len` |
| Assert | `Assert` / `AssertEq` | `Assert` takes (Trilean, String); `AssertEq` takes any two `RuntimeValue` for structural equality |
| Text | `TextLen` / `TextConcat` / `TextFromInteger` / `ParseInteger` / `IntoBytes` / `FromBytes` | String allocation + lifetime ownership |
| Collections | `Vector*` (4 ops) + `HashMap*` (5 ops) | Heap-allocated containers via `Rc::into_raw` + matching `drop_arc` shims |
| File I/O | `ReadFile` / `WriteFile` / `WriteFileBytes` / `FileExists` / `ReadDirRecursive` | String paths + Vec<u8>/Vec<String> returns |
| Path | `PathJoin` / `PathParent` / `PathBasename` | String $\rightarrow$ String |
| String | `StringSubstring` / `StringSplit` / `StringIndexOf` | String slicing + ownership |
| Misc | `Blake3Hash` (String $\rightarrow$ Vec<u8>) / `GetEnv` (String $\rightarrow$ String?) / `FStringConcat` (varargs) | Mixed |
| Atomic (per ADR-0028) | `AtomicNew` / `Load` / `Store` / `Swap` / `CompareExchange` / `FetchAdd` / `FetchSub` / `FetchBitwise{And,Or,Xor}` | `Rc<RefCell<RuntimeValue>>` pointer marshaling; lifetime across JIT boundary |

### 12.2 — Design constraints for v0.10 implementation

When v0.10 addresses this, the design must resolve:

1. **`RuntimeValue` ABI representation.** JIT registers hold raw primitives (`i64`/`i8`); Rust shims must receive and return structured `RuntimeValue`. Choose between:
   - Pass everything as `*const RuntimeValue` (boxed-by-default, slow).
   - Specialize per-builtin per argument type (43 $\times$ N marshaling stubs, verbose).
   - Hybrid: primitives unboxed, composites boxed.
2. **Lifetime management.** `Rc::into_raw` leaks refcounts unless a matching `drop_arc` shim runs. JIT'd code must emit `drop_arc(ptr)` at the proper point — integrating via Cranelift's `cold_block`/`ehpad` for clean Drop semantics, or explicit reference counting in IR.
3. **Capability gate enforcement.** ADR-0028 §8 + ADR-0016 §5 require per-builtin capability checks. Currently VM does this at `path_to_builtin` time; the JIT shim layer needs equivalent runtime checks (or compile-time elision if grants are static).
4. **Panic $\rightarrow$ VM error propagation.** Rust shims panic on `VmError`-class failures (`Assert` fail, `Vector::get` OOB). JIT-side, this requires catching panics and converting them to VM-compatible error paths. Cranelift trap blocks are one approach; `extern "C-unwind"` is another.
5. **`unsafe_code` policy.** The shim layer requires `#[unsafe(no_mangle)]` + raw pointer casts; v0.9 keeps `unsafe_code = "forbid"` honored. v0.10 must override to `deny` with audit comments at each `unsafe` block.

### 12.3 — v0.9 stop-gap behavior (shipped)

The `Instruction::CallBuiltin` opcode raises `JitError::UnsupportedOpcode` with a diagnostic naming the specific builtin. The function tiers down to VM dispatch per ADR-0030 §2; other functions in the program still JIT. Diagnostic format:

```
unsupported IR opcode for JIT backend: CallBuiltin(println) with 1 arg(s) —
full builtin shim layer defers v0.10 per ADR-0030 §12 backlog
(RuntimeValue ABI marshaling complexity)
```

Real-world v0.9 impact: most user code paths (numeric loops, control flow, function calls) still JIT. Functions with `println` / `assert` / collection ops stay on VM. Self-host bootstrap (which uses `HashMap` heavily) sees partial JIT acceleration only.

### 12.4 — Decision rationale

Author chose "implementer's call — does not affect syntax" on 2026-05-30. Per the "slow and steady" precedent (`Option A` from v0.9.x.atomic.7a), the principle is: do not ship temporary code that a v0.10 redesign would invalidate. The full builtin shim layer crosses too many design questions (the 5 items above) to ship safely within v0.9 scope — deferred to a coherent v0.10 phase.

---

## §13 — v0.10 backlog: AOT cache layer (revealed by v0.9.x.jit.6)

**Addendum 2026-05-30:** v0.9.x.jit.6 implementation revealed that ADR-0030 §5 "AOT cache layout" requires a **fundamental backend swap** — `cranelift-jit` (emits in-process mmap RX pages) $\rightarrow$ `cranelift-object` (emits ELF/.o object files suitable for serialization + cross-process loading). This is not merely "adding a cache layer on top of existing JIT"; it is "using a different Cranelift module type". Per "slow and steady" + `.4` precedents, v0.9 defers the full AOT cache to v0.10 with an explicit design backlog.

### 13.1 — Why the cranelift-jit backend cannot be cached as-is

`cranelift-jit::JITModule::finalize_definitions()` mmaps RX pages in the **current process address space** and returns raw pointers via `get_finalized_function`. These pages:

- Contain absolute addresses for cross-function calls (resolved at `define_function` time relative to the module's mmap base).
- Reference Rust runtime symbol addresses (e.g., `cranelift_module::default_libcall_names` entries — `__triet_libcall_X` thunks).
- Are not position-independent code by default.

Dumping to disk and reloading would require:
1. Tracking every relocation Cranelift applied.
2. Re-applying relocations on load against the new process's address space.
3. Re-resolving libcall symbols.
4. Verifying RW page layouts match.

This is precisely what `cranelift-object` provides — emitting ELF objects with relocation records that a separate object-file loader processes. v0.10 should make this switch.

### 13.2 — v0.10 backend swap: `cranelift-object`

Replace (or add alongside) the current `cranelift-jit` dependency:

```toml
# triet-jit/Cargo.toml v0.10:
cranelift-object = "0.132"   # NEW — for AOT path
cranelift-jit    = "0.132"   # KEEP — for hot-path live JIT
```

Two execution paths:
- **AOT cache hit:** load `.o` from `~/.triet/store/jit/{target_triple}/{impl_hash}/` $\rightarrow$ use `object` crate + `libloading` to map and resolve $\rightarrow$ cast fn pointer.
- **Cache miss:** Cranelift-jit fresh compilation (current v0.9 path) $\rightarrow$ emit machine code $\rightarrow$ optionally serialize to AOT cache on graceful shutdown.

### 13.3 — Filesystem layout (per ADR-0030 §5)

Already specified in §5; v0.10 implements:

```
~/.triet/store/
└── jit/
    └── {target_triple}/    e.g. x86_64-unknown-linux-gnu
        └── {impl_hash}/    module-level ADR-0014 hash
            ├── functions.o          (ELF object via cranelift-object)
            └── manifest.bin         (FuncId → symbol-name table, CC)
```

`target_triple` is obtained from Rust `std::env::consts::ARCH` + OS detection. `impl_hash` is computed by `triet-pack`'s ADR-0014 hash tree (`crates/triet-pack/src/lockfile.rs`).

### 13.4 — Five design constraints v0.10 must address

When v0.10 addresses this, the design must resolve:

1. **Cranelift version pinning.** Cache invalidates on Cranelift version bumps — record `cranelift_codegen::VERSION` in the manifest, and refuse on mismatch during load.
2. **Libcall symbol resolution.** Triet does not currently use Cranelift libcalls (we wired `default_libcall_names` but emitted code does not reference them). When the v0.10 builtin shim layer (§12) adds `extern "C"` shims, those symbols become libcalls — the AOT load path must re-resolve them at the new process's `dlsym` time.
3. **`std store gc` integration.** Per ADR-0015 §6 mark-and-sweep, JIT cache directories are GC roots tied to `impl_hash`. When the package's `pkg/{hash}/` is collected, `jit/{triple}/{hash}/` is collected too. Wire into the existing GC walker.
4. **Cross-machine cache portability.** Per §5 "Per-target-triple separation" — refuse loading if the `target_triple` directory does not match the host. Do not attempt cross-architecture loading.
5. **Determinism preservation.** Per ADR-0007, IR is deterministic; the AOT cache's presence is not part of the determinism contract (cache hits/misses can differ across runs). Document explicitly; bootstrap tests continue to rely on byte-identical IR output, not cache state.

### 13.5 — v0.9 stop-gap behavior (shipped)

No persistent cache. `JitDispatcher` compiles fresh on every run when its threshold is crossed (per .5 dispatch model). Per-process amortization applies — once compiled within a session, subsequent calls hit the in-memory cache. A new `dao run` invocation triggers a full re-compile.

For self-host bootstrap (3000 functions $\times$ 1-3s JIT each): full re-compile every run. Per ADR-0030 §11.7 + §9, this is the gate-lift blocker — v0.9.x.jit.7 cannot lift Stage 2 ≡ Stage 3 byte-identical bootstrap from `#[ignore]` while JIT compilation costs are incurred on each run. The v0.10 AOT cache resolves this.

### 13.6 — Decision rationale

Same as §12.4 (builtin shim): defer to a coherent v0.10 phase. Moving from `cranelift-jit` $\rightarrow$ `cranelift-object` is a backend swap, not an additive feature. Implementing a skeleton with current `cranelift-jit` and then reworking for v0.10 is the exact "ship temporary code" anti-pattern rejected by the author.

Furthermore, AOT cache value is tied to §12 builtin shim coverage. With most builtins tiering down to VM in v0.9, the JIT'd subset of any non-trivial program is small — the cache hit benefit is proportionally modest. v0.10 ships both together for full payoff.

---

## §14 — v0.10 backlog rollup: §11.7 + §11.8 also defer (chained from §12 + §13)

**Addendum 2026-05-30 (v0.9 phase close decision):** Author chose 2-day v0.10 implementation window on 2026-05-30, with AI as primary code author. Per "solve the most in v0.10" goal, remaining JIT sub-tasks `.7` (bootstrap byte-identical gate lift) and `.8` (perf bench $\ge$ 10×) also defer to v0.10. The v0.9.x.jit phase closes at .6.

### 14.1 — Why .7 (bootstrap gate lift) defers

Per §9 + §13.5: lifting `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required requires the Stage 2/3 self-host compiler to finish within the bootstrap budget (< 10 min per §11.8). The 3000-function self-host $\times$ cold JIT compilation cost is prohibitive without the §13 AOT cache. Once cached, subsequent runs are fast — making the gate lift feasible in v0.10 when §12 + §13 land.

### 14.2 — Why .8 (perf bench) defers

Per §12.3: most v0.9 user code paths tier down to VM because the builtin shim layer is not yet ready. Benchmarking pure-numeric loops (the JIT'd subset) would understate architectural value, whereas benchmarking the same workload in v0.10 with full builtin coverage provides a more accurate measurement of JIT performance benefits. Defer the perf gate to v0.10 alongside the surface that completes it.

### 14.3 — v0.9 .x.jit phase close summary

Shipped 6 sub-tasks: `.1` scaffold, `.2` opcode translation, `.3` call dispatch + Const, `.4` CallBuiltin structured tier-down, `.5` VM dispatcher integration (first native execution), `.6` AOT cache deferral with §13 backlog. Three deferred to v0.10: `.4` builtin shim (§12), `.6` AOT cache (§13), `.7`+`.8` bootstrap-gate + perf-bench (this §14).

Net v0.9 JIT achievement: **first execution of Cranelift-compiled native code from Triet Vm**, with full Tier-1/Tier-2 graduation model ($\ge$ 100-call threshold per §2), `--no-jit` escape hatch, workspace's single audited `unsafe` block, and partial-program coverage (numeric arithmetic + cmp + control flow + intra-program calls). The foundation is laid for v0.10 to integrate builtin shims, AOT caching, and lifted CI gates.

---

## Consequences

**Positive Outcomes:**

- Bootstrap byte-identical gate becomes CI-required (closes ADR-0019 perf-deferred gate).
- Self-host compiler bootstrap loop $\le$ 2× Rust impl runtime (ROADMAP §v0.9 target).
- Examples + demos run 10×+ faster (numeric-heavy programs achieve near-native execution speed).
- Production-feasible execution tier exists — Triet becomes usable for non-dev workloads prior to v2.0 LLVM AOT.
- Atomic primitive (ADR-0028) builtins receive native dispatch through JIT — critical when the v0.10 stdlib ships real threading.

**Constraints & Costs:**

- New `triet-jit` crate $\approx$ 3000-5000 LOC. Cranelift dependency adds ~5MB to compile-time deps but has no runtime size impact (linked statically).
- `.triv` wire format unchanged — JIT consumes the existing format. No new opcodes.
- First-run latency: ~1-3s per hot function JIT compile. Acceptable for v0.9; reduce in v1.0+ via background thread (deferred).
- W^X mmap path adds OS-specific code (Linux/macOS/Windows divergence). POSIX-first; Windows ConPTY-style stub if not supported.

**Risks & Verification Needs:**

- Cranelift compilation time at scale: ~3000 self-host functions $\times$ 1-3s = 3000-9000s = up to 2.5h cold first run. Caching amortizes this, but cold start remains a real cost. Verify during benchmarking.
- Memory: each JIT'd function holds machine code in RX pages. 3000 functions $\times$ ~1KB avg = ~3MB working set. Manageable.

---

## Rejected Alternatives

- **JIT-only runtime** (removing VM as Tier 1). Cold-start cost, no fallback. Rejected per §1.
- **Profile-guided multi-tier JIT** (Tier 2a baseline JIT, Tier 2b optimized JIT). Premature for v0.9 scale (~3000 self-host functions). Deferred to post-v1.0.
- **Tier-down (de-optimization)** — once JIT'd, functions stay JIT'd. Re-specialization / on-stack replacement / OSR deferred to post-v1.0.
- **Background JIT thread** — synchronous in v0.9; async deferred per §7.
- **LLVM as v0.9 backend** — Cranelift chosen for fast compilation + Rust-native ergonomics. LLVM is planned for v2.0 (per ROADMAP §v2.0).
- **JIT for tree-walking interpreter** — the interpreter is Tier 0 dev-only; no JIT needed.
- **Custom IR optimization passes pre-JIT** — Cranelift's built-in passes (DCE, inlining at codegen) are sufficient. Custom IR optimizations deferred to post-v1.0 if profiling demonstrates clear wins.
- **Inline caches for dispatch** — deferred to v1.0+ profiling work.
- **Trytecode-native JIT (v∞ scope)** — outside v0.9; subject of a future v∞ ADR.

---

## Prior Art

| Source | What We Adopted | What We Changed |
|---|---|---|
| Cranelift (Bytecode Alliance) | Codegen backend; SSA IR mapping | Triet: 3-tier model with VM persisting as Tier 1, rather than JIT-only |
| HotSpot JVM | Call-count threshold ~100; per-method tier-up | Triet: simpler (2-tier, no Tier 3 optimizing compiler) |
| WasmTime + Wasmer | Cranelift-as-WASM-JIT pattern | Triet: SSA IR directly, no WASM intermediate |
| LuaJIT | Trace-based tier-up | Triet: method-based (tracing deferred to post-v1.0) |
| V8 (Ignition + TurboFan) | 2-tier pattern (interpreter + JIT) | Triet: 3-tier with explicit VM tier retained |
| Rust `rustc_codegen_cranelift` | Cranelift-as-Rust-backend pattern | Triet: similar embedding; different IR shape (Triet IR is closer to Cranelift IR than rustc MIR is) |

**Novel Contributions in Triet:**

- **Trit-aware register types** — Trit and Trilean values map to `i8` with `{-1, 0, +1}` encoding (not `{0, 1}` boolean). Cranelift does not natively understand trits; our codegen patterns guarantee semantic correctness.
- **BrTrilean $\rightarrow$ 2 cmp + 2 branch** pattern per [ADR-0010](0010-ternary-native-ir.md) backend table. Standardized for both Cranelift v0.9 and future LLVM v2.0.
- **Per-`impl_hash` AOT cache** tied to the ADR-0014 CAS hash tree. Reuses existing GC infrastructure (ADR-0015 §6).

---

## References

- [ROADMAP §v0.9](../../ROADMAP.md) — JIT deliverables + perf gates (parent target).
- [ADR-0007](0007-ir-design.md) — IR design (register SSA shape Cranelift consumes).
- [ADR-0008](0008-triv-binary-format.md) — `.triv` wire format (unchanged by JIT).
- [ADR-0010](0010-ternary-native-ir.md) — Ternary IR backend mapping (BrTrilean $\rightarrow$ 2 cmp + 2 branch).
- [ADR-0010 Addendum §C](0010-ternary-native-ir.md#addendum-c--v0743-error3c-brtrilean-unknown_block-demoted-to-defense-in-depth) — Constant::Null = Trit::Zero per Trilean refinement.
- [ADR-0011](0011-abi-metadata-format.md) — ABI metadata (cross-module dispatch).
- [ADR-0012](0012-witness-table-dispatch.md) — Witness table cross-package generics.
- [ADR-0014](0014-hash-scheme-refinement.md) — Hash scheme (AOT cache uses `impl_hash`).
- [ADR-0015](0015-package-store-layout.md) — Package store (JIT cache co-located).
- [ADR-0019 §7 Addendum](0019-self-hosting-compiler-bootstrap.md#addendum--v0713-perf-gate--10-ph%C3%BAt-deferral) — Perf gate deferral chained to this lift.
- [ADR-0028](0028-atomic-primitive.md) — Atomic primitive builtins 27-39 (JIT calls Rust shims).
- [ADR-0029 §5 + §6](0029-self-host-port-policy.md) — Self-host port plan template (this ADR §10 first use); Stage 2/3 gate lift cross-reference.
- [VISION §4.3](../../VISION.md) — Multi-backend execution model (VM dev tier, JIT/AOT production).
- Cranelift docs — https://github.com/bytecodealliance/wasmtime/tree/main/cranelift (pinned version in workspace `Cargo.toml`).
- Bytecode Alliance security model — sandboxing patterns for JIT codegen.
