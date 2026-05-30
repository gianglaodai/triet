# ADR 0030 — JIT Integration (Cranelift backend)

**Trạng thái:** **Locked** (v0.9.0.3, author sign-off 2026-05-29). Refines [ROADMAP §v0.9](../../ROADMAP.md) JIT deliverables. Author confirmed 4 architecturally-significant decisions: §1 3-tier model (Interpreter+VM+JIT, no JIT-only); §2 100-call threshold (Hotspot JVM convention); §6 no capability gate for default `usr.*` programs; §7 synchronous JIT for v0.9 (background defer v1.0+). First ADR using [ADR-0029 §5](0029-self-host-port-policy.md) Self-host port plan template — see §10.

> **2026-05-29 Addendum (v0.9.0.3.c):** Self-review post-lock identified 3 architectural gaps requiring resolution before implementation phase v0.9.x.jit starts:
>
> **Gap 1 — JIT codegen IS privileged operation.** §6 original wording "JIT is part of runtime, no more privileged than VM is" was wrong about fundamentals: VM doesn't write executable memory; JIT does (W^X mmap RW→RX flip). Kernel modules cannot allocate RWX pages; hardened userspace runtimes (SELinux, macOS Hardened Runtime) require entitlement for JIT. Per [VISION §3.5](../../VISION.md) Pillar 5 capability + Pillar 4 OS-capable, JIT must have capability gate.
>
> **Resolution:** Add capability `dev.jit_codegen` — required for JIT codegen path in any program. Default ambient for `usr.*` (user-mode programs get JIT for free, matching current §6 intent). Kernel/embedded programs explicitly DENY via `dao.package requires dev.jit_codegen deny` — runtime detects, falls back to VM-only mode automatically (NOT error; per BYOS philosophy ADR-0026 v2, runtime features are external/optional). Capability ambient resolution per [ADR-0016 §5](0016-capability-type-system.md) — no friction for default `usr.*`.
>
> **Gap 2 — Tier model naming diverged từ VISION §4.2.** ADR §1 used "Tier 0/1/2" but VISION §4.2 lists "Backend 1: VM / Backend 2: JIT / Backend 3: AOT / Backend 4: Trytecode" với interpreter as auxiliary "Development tier (still runs alongside VM)". Realign:
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
> - **CLI flag `--no-jit`**: disable Backend 2 graduation entirely, run pure Backend 1 VM. For debugging, reproducibility, sandboxed environments. Persistent via `TRIET_JIT=disabled` env var.
> - **Real-time suitability note** (added to §7 Hệ quả): "JIT NOT suitable for hard real-time contexts (~1-3s pause on first trigger of hot function is unpredictable). Real-time kernel code should use Backend 1 VM (deterministic dispatch latency) or wait for Backend 3 AOT (v2.0) / Backend 4 Trytecode (v∞)."
>
> **Addendum scope:** §6 capability gate semantics changed (no-gate → ambient-default-with-deny-fallback); §1 tier naming realigned with VISION §4.2; §7 real-time note added. ADR-0030 body NOT edited per project ADR immutability rule; Addendum is authoritative for these 3 gaps. v0.9.x.jit implementation phase uses Addendum semantics, not original §1/§6/§7 text.
>
> Cross-references: [ADR-0016 §5](0016-capability-type-system.md) (capability ambient resolution rule); [ADR-0026 v2 §6 BYOS](0026-actor-boundary-send-rules.md) (runtime features external philosophy); [VISION §3.5 + §4.2](../../VISION.md) (Pillar 5 + backend tier naming).

**Issue:** ROADMAP §v0.9 đặt JIT làm primary v0.9 deliverable: "Tier 2 Cranelift JIT cho function chạy thường xuyên (profile-guided)" + "AOT cache: lần chạy thứ 2 dùng JIT-output cached". Plus carry-forward perf gates: bench ≥10× v0.3 baseline, full 3-stage bootstrap < 10 phút, Stage 2 ≡ Stage 3 byte-identical lift từ `#[ignore]` → CI-required.

Open questions ADR-0030 phải lock:

1. **Tier model** — Bytecode VM stays? Or JIT-only post-warm-up?
2. **Trigger heuristics** — call-count threshold? function-size weighting? Tier-down trên JIT failure?
3. **IR → Cranelift translation** — direct mapping or intermediate canonicalization?
4. **BrTrilean lowering** — per [ADR-0010](0010-ternary-native-ir.md) backend table 2-cmp-2-branch; specifics?
5. **AOT cache layout** — directory structure? invalidation key? determinism guarantee?
6. **Capability gate** — `sys.jit` required? Or ambient like `vm.run`?
7. **Threading** — synchronous JIT (compile-on-trigger) or background JIT thread?
8. **Lowerer determinism preservation** — JIT codegen có cần determinism? Same .triv → same machine code?
9. **Stage 2 ≡ Stage 3 byte-identical gate** — lift conditions?
10. **WitnessCall + cross-package dispatch** — JIT handles?
11. **Self-host port plan** — per ADR-0029 §5 (mới): JIT là Layer C (runtime), self-host port not required.

---

## §1 — Tier model: 3-tier (Interpreter → VM → JIT)

**Author review required.**

**Decision:** Triết runtime has 3 tiers, ordered by call-count graduation:

```
Tier 0 — Tree-walking interpreter  (existing v0.2; dev tier, debug-friendly)
Tier 1 — Bytecode VM register-SSA  (existing v0.3; dev tier, baseline)
Tier 2 — Cranelift JIT             (NEW v0.9; production-feasible warm tier)
```

(Future v2.0 LLVM AOT = Tier 3; future v∞ trytecode native = Tier ∞ per [VISION §4.3](../../VISION.md).)

**Graduation policy:**
- Default entry tier: **Tier 1 (VM)**. Tier 0 (interpreter) only when `dao run --interpret` flag set (debugging).
- Functions graduate **Tier 1 → Tier 2** when call count exceeds threshold (§2).
- No tier-down: once a function JITs, it stays JIT (no de-optimization v0.9). Future ADR can add tier-down if profile-guided re-spec needed.

**Per-function graduation, not per-program.** Cold functions stay VM; hot functions go JIT. Mixes seamlessly: VM caller can call JIT'd callee and vice versa via shared calling convention.

**Why keep VM as Tier 1 (not skip to JIT)?**

- **Warmup time**: pure-JIT entry pays compilation cost on first call. VM dispatches instantly.
- **Cold code path**: most programs have hot/cold distribution (Pareto). Cold = 90% of code, run rarely → VM is cheaper than JIT compilation cost.
- **Bootstrap path**: self-host compiler compiles itself → many cold-path functions (parsers, typecheck rules). VM keeps cold path light.
- **Debug-friendly fallback**: if JIT produces wrong code, falling back to VM is a known-good safety net.

**Alternative rejected:** JIT-only (replace VM). Pros: simpler runtime. Cons: cold-start cost, no fallback, breaks v0.3 contract that VM is "stable IR's executor".

---

## §2 — Trigger heuristics: call count threshold

**Author review required.**

**Decision v0.9:** Function graduates Tier 1 → Tier 2 when **call count ≥ 100**. Threshold encoded in `triet-ir::JitConfig::trigger_threshold`, runtime-configurable via `TRIET_JIT_THRESHOLD` env var (escape hatch for benchmarking/tuning).

**Trigger detection:**
- Each `FuncId` tracks a per-process call count in `Vm::dispatch_counters: HashMap<FuncId, u32>`.
- On every cross-function call (CallCrossModule, WitnessCall, etc.), counter increments.
- When counter ≥ 100, dispatcher attempts JIT compile + replace VM dispatch with native call thunk for that FuncId.

**Why 100?** Industry heuristic from Hotspot JVM (Tier 1 → 2 at ~100 calls). Conservative — avoids JIT'ing one-shot functions. Aggressive enough to catch loops (a loop body with 100 iterations = 100 dispatches).

**Rejected alternatives:**

- **Always-JIT (threshold = 1)**: compiles every function. Wastes compilation budget on cold code.
- **Profile-then-JIT** (collect profile for N runs, then JIT next run): adds complexity for marginal benefit at v0.9 scale.
- **Function-size weighting** (large functions JIT first): premature optimization; size doesn't always correlate with hot.
- **Cycle-counting** (instrument loops): adds runtime overhead in VM.

**Tier-down on failure:** if Cranelift fails to compile (e.g., unsupported opcode), log warning + keep VM dispatching. No retry; function stays VM-tier permanently this session.

---

## §3 — IR → Cranelift IR translation

**Decision:** Direct register-SSA mapping. Triết IR is already register-based SSA (per [ADR-0007](0007-ir-design.md)); Cranelift IR is SSA-based with explicit basic blocks. Translation is 1:1 per-opcode.

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

**Calling convention:** Cranelift's default (System V on Linux, Microsoft x64 on Windows). Triết values map to Cranelift types:

| Triết type | Cranelift type (binary CPU) |
|---|---|
| `Trit` | `i8` (use 3 distinct values `{-1, 0, +1}` packed) |
| `Tryte` | `i16` |
| `Integer` | `i64` |
| `Long` | `i128` (Cranelift extension; or pair-of-i64 if unsupported) |
| `Trilean` | `i8` (same encoding as Trit) |
| `T?` discriminator | `i8` for trit, plus payload registers |
| `&+ T`, `&0 T`, `&-` | `i64` pointer (with ObjectHeader RC tracking via Rust runtime calls) |

**Opcode translation table (selected):**

| Triết IR opcode | Cranelift IR pattern |
|---|---|
| `IntegerAdd` | `iadd` |
| `IntegerMul` | `imul` |
| `IntegerCmp::Eq` | `icmp eq` → producing `i8` (extend) |
| `BrTrilean { value, neg, zero, pos }` | 2 `icmp` + 2 `brnz` per ADR-0010 backend table |
| `CallLocal { func, args }` | `call $func, $args` (intra-module direct call) |
| `CallCrossModule { path, args }` | indirect call through dispatcher table (resolve at compile-time if possible, else runtime lookup) |
| `WitnessCall { table_id, method_index, args }` | indirect call through witness table per [ADR-0012](0012-witness-table-dispatch.md) |
| `Constant::Null` | `iconst.i8 0` (Trit::Zero per [ADR-0010 Addendum §C](0010-ternary-native-ir.md)) |
| Builtin opcodes 4-26 (Vec/HashMap/IO) + 27-39 (Atomic per ADR-0028) | Rust runtime function call (`extern "C"` shim) |

**Builtin shim integration:** Builtins lower to `call $rust_builtin_<id>` (extern "C" function in `triet-jit` linking Rust runtime). Per [ADR-0019 §5](0019-self-hosting-compiler-bootstrap.md) Rust-shim approach maintained.

---

## §4 — `BrTrilean` lowering per ADR-0010

**Decision:** `BrTrilean { value: i8, neg_block, zero_block, pos_block }` lowers to 2 compare + 2 branch on binary CPU:

```
        ; Cranelift IR
        v100 = icmp eq value, iconst.i8(-1)   ; check Trit::Negative
        brnz v100, neg_block
        v101 = icmp eq value, iconst.i8(0)    ; check Trit::Zero
        brnz v101, zero_block
        jump pos_block                          ; fallthrough = Trit::Positive
```

**Order chosen (Negative → Zero → Positive):** matches v0.7 VM dispatcher order. Empirically Trit::Zero is the most common branch in `T?` null-check patterns; future profile-guided reordering may swap. Defer.

**On hypothetical trytecode CPU (v∞ scope):** `BrTrilean` lowers to single native instruction (per ADR-0010 backend table). v∞ ADR will refine.

---

## §5 — AOT cache layout + invalidation

**Decision:** Cache native code by **`impl_hash` of the function's owning module** per [ADR-0014](0014-hash-scheme-refinement.md) hash tree.

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

**Per-target-triple separation:** cache invalid cross-architecture (x86_64 code unusable on ARM64). Triple comes from Rust `std::env::consts::ARCH` + OS detection.

**Invalidation:** Module's `impl_hash` changes when any function in module changes → cache directory becomes orphan. Tied to existing `dao store gc` mark-and-sweep (ADR-0015 §6). JIT cache is **roots-tracked**; deleted alongside `pkg/{hash}/` when module is GC'd.

**Determinism (cache hit/miss):**

- Cache hit requires: same `{target_triple}/{impl_hash}/` directory exists + Cranelift codegen version matches.
- Cranelift version pinned in workspace `Cargo.toml`; bump invalidates all cached entries (full re-JIT).
- Cache hit/miss can vary across runs (e.g., first cold run misses; warm runs hit). **NOT a determinism violation** per [ADR-0007 §IR](0007-ir-design.md) — IR is deterministic; runtime behavior (cache state) is not required to be.

**First-run cost:** ~1-3 seconds per main.tri function JIT (Cranelift O0 codegen). Subsequent runs amortize. Self-host bootstrap: ~3000 functions × cold JIT ≈ 9-30s overhead first time; cached after.

---

## §6 — Capability gate

**Author review required.**

**Decision v0.9:** **No capability gate** for JIT in default `usr.*` programs. JIT is part of runtime, no more privileged than the VM is.

**Exception:** `dev.jit_unsafe` capability for power-user APIs that expose JIT internals (e.g., manual recompilation triggers, JIT codegen options). v0.9 ships **no public API** for these — they're internal to `triet-jit` crate. `dev.jit_unsafe` reserved for future stdlib bindings.

**Sandboxing concern (W^X):** JIT codegen writes code into RWX-mapped pages. On hardened systems (SELinux, macOS Hardened Runtime), this requires entitlement. Triết runtime handles via:

- Linux: `mmap(PROT_READ|PROT_WRITE)` → write code → `mprotect(PROT_READ|PROT_EXEC)` flip.
- Detection of W^X policy mismatch (e.g., grsecurity) → fall back to VM-only mode, log warning.

**Rationale for no capability:** matches VM execution which doesn't require capability. JIT is semantically equivalent — same IR, faster execution. Capability boundary is at IR generation (compile-time, already handled by `sys.*`/`dev.*` namespace check), not at execution backend.

**Rejected:** `sys.jit` mandatory capability. Pros: explicit acknowledgment. Cons: every `usr.*` program would need it → friction without security benefit (capability is at IR level, not codegen level).

---

## §7 — Threading model

**Author review required.**

**Decision v0.9:** **Synchronous JIT compilation** on dispatcher thread. When trigger fires (call count ≥ 100), VM dispatcher blocks, calls Cranelift compile, replaces dispatch entry, continues execution.

**Latency cost:** ~1-3s pause on first trigger of each hot function. Acceptable for v0.9 (dev/CI scenarios). Production interactive applications may notice — addressed in v1.0+ post-ADR-tbd.

**Future v1.0+ ADR:** background JIT compilation thread, lock-free patch-in via atomic pointer swap. More complex; defers until profile shows real interactive jank.

**Alternative considered:** Async JIT trigger (warm thread runs compilation, VM continues at Tier 1). Cleaner UX but adds threading complexity to v0.9 scope. Rejected.

---

## §8 — Lowerer determinism preservation

**Decision:** JIT does NOT need same-machine-code determinism. Per [ADR-0007](0007-ir-design.md), determinism contract is at IR level (`.triv` is deterministic given `.tri` input). JIT machine code is implementation detail.

Concretely: `dao build foo.tri -o foo.khi` produces byte-identical `.khi` (passes `bootstrap_determinism` test). `dao run foo.khi` JIT compiles → machine code may differ across:
- Cranelift versions (pinned in workspace, but upgradeable).
- Target triple (x86_64 vs ARM64 = different ISA).
- Optimization passes (v0.9 = O0 only; future tunable).

**Cache hits are deterministic per-target-triple per-Cranelift-version.** Two runs on same machine same Triết toolchain version = same cached machine code. Across machines = no guarantee.

**Impact on Stage 2 ≡ Stage 3 byte-identical:** Stage 2/3 compare `.khi` bytes (IR output of self-host), NOT machine code. JIT acceleration is for execution speed of Stage 2/3 themselves; doesn't affect the `.khi` they produce.

---

## §9 — Stage 2 ≡ Stage 3 byte-identical gate lift

**Decision:** Per ROADMAP §v0.9 Functional gate: lift `bootstrap_loop.rs::stage2_eq_stage3_main_tri_byte_identical` from `#[ignore]` to CI-required when:

1. JIT enables Stage 2 (Triết-impl compiler) to compile main.tri in **< 5 min** on dev hardware. (Pre-JIT: ~15+ min per ADR-0019 §7 Addendum measurement.)
2. JIT enables full 3-stage bootstrap loop in **< 10 min** total per ROADMAP §v0.9 Gate.
3. Stage 2 ≡ Stage 3 `cmp` comparison produces identical bytes (deterministic IR per §8).
4. ADR-0029 §6 cross-reference: self-host port lockstep maintained throughout v0.9 → Stage 2 can read current Triết source.

**Verification mechanism:** New CI test `crates/triet-bootstrap/tests/bootstrap_loop.rs::stage2_eq_stage3_with_jit` (removes `#[ignore]` once perf gate hits). Test creates `Vm` with JIT enabled, runs Stage 2 main.tri compile, compares output bytes Stage 2 vs Stage 3.

**Carry-forward note:** ADR-0019 §7 Addendum chained this lift to JIT; ADR-0030 §9 confirms timeline.

---

## §10 — Self-host port plan (per ADR-0029 §5 template)

**Layer A surface changes:** **No.** JIT is internal runtime layer; no lexer, parser AST, or SPEC grammar changes.

**Layer B internal changes:** **No.** Typecheck, lowerer, IR shape unchanged. JIT consumes existing IR.

**Layer C runtime changes:** **Yes.** New `triet-jit` crate; `Vm` integrates JIT dispatch. Self-host (`compiler/`) doesn't see this — self-host produces `.khi` (IR bytecode); JIT is consumer of the bytecode at runtime.

**Same-phase port required:** **No.** Per ADR-0029 §3 Layer C independent timeline rule, self-host port not needed.

**Bootstrap interaction:** Stage 2 (Triết-impl compiler) benefits from JIT acceleration (per §9 gate lift) but Stage 2 source code does NOT change. It's the same `compiler/*.tri` running on a faster runtime backend.

---

## §11 — Implementation sub-phase plan (v0.9.x.jit)

**Sub-task ordering** (informational — exact split lands in v0.9.x.jit.N sub-tasks):

1. **v0.9.x.jit.1** — Scaffold `triet-jit` crate. Cargo.toml + lib.rs skeleton + Cranelift dependency pinned.
2. **v0.9.x.jit.2** — Opcode-by-opcode translation: arithmetic + comparisons + control flow (BrIf, BrTrilean).
3. **v0.9.x.jit.3** — Call dispatch: CallLocal + CallCrossModule + WitnessCall.
4. **v0.9.x.jit.4** — Builtin shim integration (opcodes 4-26 + 27-39 for Atomic).
5. **v0.9.x.jit.5** — VM dispatcher integration: trigger detection + JIT compile path + native call thunk.
6. **v0.9.x.jit.6** — AOT cache filesystem layout + invalidation.
7. **v0.9.x.jit.7** — Stage 2 ≡ Stage 3 gate verification + lift from `#[ignore]`.
8. **v0.9.x.jit.8** — Perf bench: ≥10× v0.3 baseline on numeric-heavy programs; bootstrap < 10 min.

Each sub-task = independent commit per cadence.

---

## §12 — v0.10 backlog: full builtin shim layer (revealed by v0.9.x.jit.4)

**Addendum 2026-05-30:** v0.9.x.jit.4 implementation surfaced that the original §3 "Builtin shim integration" item is **substantially more complex** than the other ADR-0030 §3 opcode-translation work because it requires cross-ABI marshaling of Triết runtime values. Per author "chậm mà chắc" stance, v0.9 ships ONLY the structured tier-down diagnostic — functions calling stdlib builtins still tier-down to VM dispatch, just with an error message that names the specific builtin instead of a generic Debug fallback. Full shim layer defers v0.10.

### 12.1 — Why deferred (scope reality)

43 builtins across categories — virtually all require non-primitive ABI marshaling:

| Category | Builtins | Marshaling complexity |
|---|---|---|
| I/O | `Println` / `Print` | String args via `*const u8 + len` |
| Assert | `Assert` / `AssertEq` | `Assert` takes (Trilean, String); `AssertEq` takes any two `RuntimeValue` for structural equality |
| Text | `TextLen` / `TextConcat` / `TextFromInteger` / `ParseInteger` / `IntoBytes` / `FromBytes` | String allocation + lifetime ownership |
| Collections | `Vector*` (4 ops) + `HashMap*` (5 ops) | Heap-allocated containers via `Rc::into_raw` + matching `drop_arc` shims |
| File I/O | `ReadFile` / `WriteFile` / `WriteFileBytes` / `FileExists` / `ReadDirRecursive` | String paths + Vec<u8>/Vec<String> returns |
| Path | `PathJoin` / `PathParent` / `PathBasename` | String → String |
| String | `StringSubstring` / `StringSplit` / `StringIndexOf` | String slicing + ownership |
| Misc | `Blake3Hash` (String → Vec<u8>) / `GetEnv` (String → String?) / `FStringConcat` (varargs) | Mixed |
| Atomic (per ADR-0028) | `AtomicNew` / `Load` / `Store` / `Swap` / `CompareExchange` / `FetchAdd` / `FetchSub` / `FetchBitwise{And,Or,Xor}` | `Rc<RefCell<RuntimeValue>>` pointer marshaling; lifetime across JIT boundary |

### 12.2 — Design constraints for v0.10 implementation

When v0.10 picks this up, design must address:

1. **`RuntimeValue` ABI representation.** JIT registers hold raw primitives (`i64`/`i8`); Rust shims need to receive/return structured `RuntimeValue`. Decide between:
   - Pass everything as `*const RuntimeValue` (boxed-by-default, slow).
   - Specialize per-builtin per arg type (43 × N marshaling stubs, verbose).
   - Hybrid: primitives unboxed, composites boxed.
2. **Lifetime management.** `Rc::into_raw` leaks the refcount unless a matching `drop_arc` shim runs. JIT'd code must emit `drop_arc(ptr)` at the right point — could integrate via Cranelift's `cold_block`/`ehpad` for clean Drop semantics, or explicit reference counting in IR.
3. **Capability gate enforcement.** ADR-0028 §8 + ADR-0016 §5 require per-builtin capability checks. Currently VM does this at `path_to_builtin` time; JIT shim layer needs equivalent runtime check (or compile-time elision if grants are static).
4. **Panic → VM error propagation.** Rust shims panic on `VmError`-class failures (`Assert` fail, `Vector::get` OOB). JIT-side, this means catching the panic + converting to VM-compatible error path. Cranelift trap blocks are one approach; `extern "C-unwind"` is another.
5. **`unsafe_code` policy.** Shim layer requires `#[unsafe(no_mangle)]` + raw pointer casts; v0.9 keeps `unsafe_code = "forbid"` honored. v0.10 must override to `deny` with audit comments at each `unsafe` block.

### 12.3 — v0.9 stop-gap behavior (shipped)

`Instruction::CallBuiltin` opcode raises `JitError::UnsupportedOpcode` with a diagnostic naming the specific builtin. The function tier-downs to VM dispatch per ADR-0030 §2; other functions in the program still JIT. Diagnostic format:

```
unsupported IR opcode for JIT backend: CallBuiltin(println) with 1 arg(s) —
full builtin shim layer defers v0.10 per ADR-0030 §12 backlog
(RuntimeValue ABI marshaling complexity)
```

Real-world v0.9 impact: most user code paths (numeric loops, control flow, function calls) still JIT. Functions with `println` / `assert` / collection ops stay on VM. Self-host bootstrap (which uses `HashMap` heavily) sees partial JIT acceleration only.

### 12.4 — Decision rationale

Author chose "implementer's call — không ảnh hưởng cú pháp" 2026-05-30. Per "chậm mà chắc" precedent (`Phương án A` from v0.9.x.atomic.7a), the principle is: don't ship temporary code that v0.10 redesign would invalidate. Full builtin shim layer crosses too many design questions (above 5) to ship safely in v0.9 scope — defer to a coherent v0.10 phase.

---

## §13 — v0.10 backlog: AOT cache layer (revealed by v0.9.x.jit.6)

**Addendum 2026-05-30:** v0.9.x.jit.6 implementation reveals that ADR-0030 §5 "AOT cache layout" requires a **fundamental backend swap** — `cranelift-jit` (emits in-process mmap RX pages) → `cranelift-object` (emits ELF/.o object files suitable for serialization + cross-process loading). This is not "add a cache layer on top of existing JIT"; it's "use a different Cranelift module type". Per "chậm mà chắc" + .4 precedent, v0.9 defers full AOT cache to v0.10 with explicit design backlog.

### 13.1 — Why the cranelift-jit backend can't be cached as-is

`cranelift-jit::JITModule::finalize_definitions()` mmaps RX pages in the **current process address space** and returns raw pointers via `get_finalized_function`. These pages:

- Contain absolute addresses for cross-function calls (resolved at `define_function` time relative to module's mmap base).
- Reference Rust runtime symbol addresses (e.g., `cranelift_module::default_libcall_names` entries — `__triet_libcall_X` thunks).
- Are not position-independent code by default.

Dumping to disk + reloading would require:
1. Tracking every relocation Cranelift applied.
2. Re-applying relocations on load against the new process's address space.
3. Re-resolving libcall symbols.
4. Verifying RW page layout matches.

This is precisely what `cranelift-object` does — it emits ELF objects with relocation records, which a separate object-file loader processes. v0.10 should switch.

### 13.2 — v0.10 backend swap: `cranelift-object`

Replace (or add alongside) the current `cranelift-jit` dep:

```toml
# triet-jit/Cargo.toml v0.10:
cranelift-object = "0.132"   # NEW — for AOT path
cranelift-jit    = "0.132"   # KEEP — for hot-path live JIT
```

Two execution paths:
- **AOT cache hit:** load `.o` from `~/.triet/store/jit/{target_triple}/{impl_hash}/` → use `object` crate + `libloading` to map + resolve → cast fn pointer.
- **Cache miss:** Cranelift-jit fresh compile (current v0.9 path) → emit machine code → optionally serialize to AOT cache on graceful shutdown.

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

`target_triple` from Rust `std::env::consts::ARCH` + OS detection. `impl_hash` already computed by `triet-pack`'s ADR-0014 hash tree (`crates/triet-pack/src/lockfile.rs`).

### 13.4 — Five design constraints v0.10 must address

When v0.10 picks this up, design must address:

1. **Cranelift version pinning.** Cache invalidates on Cranelift bump — record `cranelift_codegen::VERSION` in manifest, refuse-on-mismatch on load.
2. **Libcall symbol resolution.** Triết doesn't currently use Cranelift libcalls (we wired `default_libcall_names` but emitted code doesn't reference them). When v0.10 builtin shim layer (§12) adds `extern "C"` shims, those symbols become libcalls — the AOT load path must re-resolve at new process's `dlsym` time.
3. **`std store gc` integration.** Per ADR-0015 §6 mark-and-sweep, JIT cache directories are GC roots tied to `impl_hash`. When the package's `pkg/{hash}/` is collected, `jit/{triple}/{hash}/` is too. Wire into existing GC walker.
4. **Cross-machine cache portability.** Per §5 "Per-target-triple separation" — refuse load if `target_triple` directory doesn't match host. Don't try cross-arch loading.
5. **Determinism preservation.** Per ADR-0007 IR is deterministic; the AOT cache's existence is not part of the determinism contract (cache hit/miss can differ across runs). Document explicitly; bootstrap tests still rely on byte-identical IR output, not cache state.

### 13.5 — v0.9 stop-gap behavior (shipped)

No persistent cache. `JitDispatcher` compiles fresh on every run when its threshold crosses (per .5 dispatch model). Per-process amortization — once compiled within a session, subsequent calls hit the in-memory cache. New `dao run` invocation = full re-compile.

For self-host bootstrap (3000 functions × 1-3s JIT each): full re-compile every run. Per ADR-0030 §11.7 + §9, this is the gate-lift blocker — v0.9.x.jit.7 cannot lift Stage 2 ≡ Stage 3 byte-identical bootstrap from `#[ignore]` while the JIT compile cost is paid each run. v0.10 AOT cache closes that.

### 13.6 — Decision rationale

Same as §12.4 (builtin shim): defer to coherent v0.10 phase. `cranelift-jit → cranelift-object` is a backend swap, not an additive feature. Implementing skeleton with current `cranelift-jit` then reworking for v0.10 is the exact "ship temporary code" anti-pattern author rejected.

Also: AOT cache value tied to §12 builtin shim coverage. With most builtins tier-downing to VM in v0.9, the JIT'd subset of any non-trivial program is small — cache hit benefit is proportionally small. v0.10 ships both together for full payoff.

---

## Hệ quả

**Possible (positive):**

- Bootstrap byte-identical gate becomes CI-required (closes ADR-0019 perf-deferred gate).
- Self-host compiler bootstrap loop ≤ 2× Rust impl runtime (ROADMAP §v0.9 target).
- Examples + demos run 10×+ faster (numeric-heavy programs feel native-speed).
- Production-feasible execution tier exists — Triết becomes usable for non-dev workloads pre-v2.0 LLVM AOT.
- Atomic primitive (ADR-0028) builtins get native dispatch through JIT — important when v0.10 stdlib ships real threading.

**Constrained (cost):**

- New `triet-jit` crate ≈ 3000-5000 LOC. Cranelift dependency adds ~5MB to compile-time deps but no runtime size impact (linked statically).
- `.triv` wire format unchanged — JIT consumes existing format. No new opcodes.
- First-run latency: ~1-3s per hot function JIT compile. Acceptable for v0.9; reduce v1.0+ via background thread (deferred).
- W^X mmap path adds OS-specific code (Linux/macOS/Windows divergence). POSIX-first; Windows ConPTY-style stub if not supported.

**Costly (need verify):**

- Cranelift compilation time at scale: ~3000 self-host functions × 1-3s = 9000-9000s = 2.5h cold first run. Cache amortizes but cold start is a real cost. Verify in benchmark phase.
- Memory: each JIT'd function holds machine code in RX pages. 3000 functions × ~1KB avg = ~3MB working set. Fine.

---

## Không làm (explicitly rejected)

- **JIT-only runtime** (remove VM as Tier 1). Cold-start cost, no fallback. Rejected per §1.
- **Profile-guided multi-tier JIT** (Tier 2a baseline JIT, Tier 2b optimized JIT). Premature for v0.9 scale (~3000 self-host functions). Defer post-v1.0.
- **Tier-down (de-optimization)** — once JIT, stays JIT. Re-spec / on-stack replacement / OSR defer post-v1.0.
- **Background JIT thread** — synchronous v0.9, async defers per §7.
- **LLVM as v0.9 backend** — Cranelift chosen for fast compile + Rust-native ergonomics. LLVM = v2.0 (per ROADMAP §v2.0).
- **JIT for tree-walking interpreter** — interpreter is Tier 0 dev-only, no JIT needed.
- **Custom IR optimization passes pre-JIT** — Cranelift's built-in passes (DCE, inlining at codegen) sufficient. Custom IR opt = post-v1.0 if profile shows wins.
- **Inline caches for dispatch** — defer v1.0+ profile work.
- **Trytecode-native JIT (v∞ scope)** — outside v0.9; v∞ ADR.

---

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| Cranelift (Bytecode Alliance) | The whole codegen backend; SSA IR mapping | Triết: 3-tier with VM persisting as Tier 1, not JIT-only |
| HotSpot JVM | Call-count threshold ~100; per-method tier-up | Triết: simpler (2-tier, no Tier 3 optimizing compiler) |
| WasmTime + Wasmer | Cranelift-as-WASM-JIT pattern | Triết: SSA IR directly, no WASM intermediate |
| LuaJIT | Trace-based tier-up | Triết: method-based (tracing defer post-v1.0) |
| V8 (Ignition + TurboFan) | 2-tier pattern (interpreter + JIT) | Triết: 3-tier with explicit VM tier kept |
| Rust `rustc_codegen_cranelift` | Cranelift-as-Rust-backend pattern | Triết: similar embedding; different IR shape (Triết IR is closer to Cranelift IR than rustc MIR is) |

**What we invented:**

- **Trit-aware register types** — Trit and Trilean values map to i8 with `{-1, 0, +1}` encoding (not `{0, 1}` boolean). Cranelift doesn't natively know about trit; our codegen patterns ensure semantic correctness.
- **BrTrilean → 2 cmp + 2 branch** pattern per [ADR-0010](0010-ternary-native-ir.md) backend table. Standardized for both Cranelift v0.9 and future LLVM v2.0.
- **Per-`impl_hash` AOT cache** tied to ADR-0014 CAS hash tree. Reuses existing GC infrastructure (ADR-0015 §6).

---

## Tham chiếu

- [ROADMAP §v0.9](../../ROADMAP.md) — JIT deliverables + perf gates (parent target).
- [ADR-0007](0007-ir-design.md) — IR design (register SSA shape Cranelift consumes).
- [ADR-0008](0008-triv-binary-format.md) — `.triv` wire format (unchanged by JIT).
- [ADR-0010](0010-ternary-native-ir.md) — Ternary IR backend mapping (BrTrilean → 2 cmp + 2 branch).
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
