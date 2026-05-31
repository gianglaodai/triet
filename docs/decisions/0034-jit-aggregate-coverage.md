# ADR 0034 — JIT aggregate coverage via delegate-to-VM shims (struct / enum / Outcome / Nullable / String, + Phi, multi-block shims, panic→tier-down)

**Trạng thái:** **Locked** (v0.11.x.jit.4, author sign-off — "Hướng A: stop deferring, make the compiler fully JIT-able"). Builds on [ADR-0032](0032-builtin-shim-abi.md) (the delegate-to-VM shim ABI it generalizes) and unblocks the bootstrap byte-identical gate lift chained from [ADR-0033 §9.5](0033-aot-cache-cranelift-object.md). First ADR opened to *close* deferred JIT-coverage debt rather than defer it.

## Issue

v0.11.x.jit.3 shipped a complete, reviewed AOT cache (per-module objects + a Triết-owned load-time linker, [ADR-0033](0033-aot-cache-cranelift-object.md) + Addenda). But the cache + the bootstrap gate lift it was meant to unblock both rest on a tacit premise: **that the self-host compiler is JIT-able.** A coverage measurement (`triet-bootstrap::jit_tier_down_audit`, commit `29aeeaa`, a resilient dry-JIT that records every function's tier-down reason) overturned that premise:

```
compiler/main.tri — 3953 functions
  JIT-able : 146  (3.7%)
  tier down: 3807 (96.3%)
  by category:
    1314  struct ops (struct_new / field_get / field_set)
     760  Outcome ops (outcome_discriminant / wrap / unwrap)
     729  enum ops (enum_new / enum_tag / enum_payload)
     515  String constant     +  94  Null constant
     172  Nullable ops (null_wrap / null_unwrap / null_check)
     169  Phi nodes
      10  translator PANIC (Cranelift assertion — a real bug)
       8  Long (i128)         +  20  builtin shim arity mismatch
```

The JIT today handles essentially **only pure-integer-arithmetic leaf functions**. The compiler — like any real program, and emphatically like kernel/OS code — is built on the **aggregate / heap data model**: structs (the AST), enums (variants), `Outcome` (`T~E` error handling), `Nullable`, and strings. All of it tiers down. So:

1. The AOT cache cannot warm-load the compiler (jit.3 fix #3: a program isn't persisted unless *every* function JITs), so the bootstrap gate stays `#[ignore]`'d — exactly what v0.11 exists to lift.
2. A language that can only JIT integer arithmetic, and must fall back to a tree-walking interpreter for all aggregate work, **cannot be an OS-capable production tier** ([VISION §4.3](../../VISION.md)) — kernel code has no interpreter to fall back to.

The earlier guess (varargs f-strings / multi-block shims were the binding constraint) was wrong; the audit shows those aren't even reached. The real question this ADR answers: **how does the JIT cover the aggregate data model, at what altitude, and in what order — without re-litigating ABI shape mid-implementation?**

## Quyết định

**Cover the aggregate data model by extending the [ADR-0032](0032-builtin-shim-abi.md) delegate-to-VM shim pattern to the aggregate IR opcodes, plus three enabling pieces (Phi codegen, multi-block shim codegen, panic→clean-tier-down). Re-measure after each sub-task. Native aggregate codegen is explicitly deferred as a later perf refinement.**

### §1 — Aggregate opcodes as delegate-to-VM shims (the core)

Each aggregate IR opcode lowers to a call to a new `__triet_*` shim whose body **delegates to the same VM logic that already executes it** — generalizing ADR-0032's "all shims delegate semantics to `triet_ir::dispatch_builtin` → VM↔JIT divergence impossible by construction." The VM's per-opcode logic (currently inline in the `vm.rs` instruction loop) is **extracted into `pub` helper functions** that both the VM loop and the new shims call — the exact `dispatch_builtin`-is-a-pub-wrapper-over-`execute_builtin` precedent (ADR-0032 §6).

Opcodes covered (names per `triet_ir::Instruction`):

| Opcode | Shim shape (per ADR-0032 §1 composite ABI: composites are `Rc::into_raw` `i64` ptrs) |
|---|---|
| `FieldGet { object, field_idx }` | `__triet_field_get(obj: i64, idx: i64) -> i64` (fixed arity) |
| `FieldSet { object, field_idx, value }` | `__triet_field_set(obj: i64, idx: i64, val: i64) -> i64` |
| `EnumNew { variant_idx, payload }` | `__triet_enum_new(variant: i64, payload_or_unit: i64) -> i64` |
| `EnumTag { scrutinee }` | `__triet_enum_tag(scr: i64) -> i64` (returns Integer index) |
| `EnumPayload { scrutinee }` | `__triet_enum_payload(scr: i64) -> i64` |
| `OutcomeDiscriminant` / wrap / unwrap | `__triet_outcome_*` fixed-arity |
| `NullWrap` / `NullUnwrap` / `NullCheck` | `__triet_null_*(v: i64) -> i64` |
| `StructNew { fields: Vec<Operand> }` | **variadic** — see §2 |

The hybrid ABID (primitives unboxed `i8/i16/i64`, composites `Rc`-boxed `i64` ptr; box/borrow/`__triet_drop_arc` lifetime) is **reused unchanged** from ADR-0032 §1/§2 — these opcodes' operands are exactly the composite pointers that ABI already defines.

### §2 — `StructNew` is variadic → the genuine array-ptr+len ABI (shared with the deferred f-string varargs)

`StructNew` takes an arbitrary field count, so it needs the **array-ptr + len** calling shape (`__triet_struct_new(fields_ptr: i64, len: i64) -> i64`): codegen spills the N already-resolved field values into a stack slot, passes its address + length. This is the *same* "varargs ABI cliff" that deferred `FStringConcat`/`TextConcat` (ADR-0032 jit.2b-iii) — so this ADR's §2 work **also unblocks those** for free. The shim borrows the slice, clones into `Vec<RuntimeValue>`, boxes the resulting `Struct`. (The stack slot is caller-owned + lives across the single call — no lifetime escape.)

### §3 — String / Null constants → data section + the `R_X86_64_64` relocation (extends the loader)

A `Constant::String` must materialize its bytes. Codegen emits the UTF-8 bytes into the object's **data section** and a `__triet_string_new(ptr: i64, len: i64) -> i64` shim boxes a `RuntimeValue::String`. Referencing a data symbol from `.text` produces an **`R_X86_64_64` (absolute-64) relocation** — which the [ADR-0033](0033-aot-cache-cranelift-object.md) Path-A loader currently **refuses** (it handles only PC-relative `PC32`/`PLT32`/`GOTPCREL`). So §3 **extends the loader's relocation set + `SUPPORTED_RELOC_TYPES`** to handle absolute-64 data relocations (patch the 8-byte field with `base + symbol_offset`), with the **same Addendum-constraint-4 test regimen** (round-trip value parity, proptest fuzz of the new patch arithmetic, W^X) the relocation patcher is already held to. `Constant::Null` boxes `RuntimeValue::Null` (no data section needed). This is the only sub-task that re-touches the unsafe loader surface; it is gated accordingly.

### §4 — Phi codegen (control-flow merge)

`Phi` lowers to a **Cranelift block parameter**: each predecessor block passes its incoming value as a branch argument, and the merge block receives it as a param. Mechanical Cranelift SSA, no shim, no unsafe — but it touches the block-emission core, so it lands as its own sub-task with control-flow tests (if/match/loop value merges).

### §5 — Multi-block shim codegen (lift the single-block restriction)

ADR-0032 jit.2b-i restricted shim calls to **single-Triết-block** functions (the per-call error sentinel assumed linear within-block flow). Aggregate ops appear throughout multi-block functions (every `if`/`match`/`loop` body), so this restriction must lift: the per-call `__triet_shim_failed` sentinel + lazy `error_exit` branch must work across arbitrary block structure. This generalizes the existing sentinel mechanism; no new unsafe.

### §6 — Translator panic → clean tier-down (a real bug, not just coverage)

The audit's `catch_unwind` wrapper revealed **10 functions where the translator *panics*** (a Cranelift "instruction added to a filled block" assertion) rather than returning `UnsupportedOpcode`. A panic mid-`compile_program` would **abort the real JIT** (or, post-§5, corrupt builder state) — strictly worse than a tier-down. §6 finds the IR shapes that trigger these and converts each to a clean `Err(UnsupportedOpcode)` (or fixes the codegen so they translate). This is correctness debt independent of coverage and is done first (it makes the rest of the bring-up safe to iterate on).

### §7 — Altitude: delegate-to-VM for *coverage* now; native aggregate codegen deferred

Delegating aggregates to VM helpers is **not** the eventual production-tier altitude — a true OS-capable backend will lay structs out natively (registers/stack/heap, no `RuntimeValue` boxing). But delegate-to-VM is the right altitude **for this phase** because it is: (a) **divergence-free by construction** (same VM code path — the ADR-0032 guarantee), (b) **low-risk** (no new aggregate-layout codegen, the project's "chậm mà chắc" bar), and (c) **sufficient to lift the gate** — see §8. Native aggregate codegen is recorded as a post-v0.11 perf refinement, not this ADR's scope. Correctness + full coverage first; speed later, each step measured.

### §8 — What "lift the gate" needs vs. what the ≥10× bench needs (they differ)

Two jit.4 goals, distinct mechanisms:

- **Bootstrap gate lift** (`stage2_eq_stage3_main_tri_byte_identical` off `#[ignore]`): the binding cost is **JIT compile time** (cold: ~3000 functions × seconds). The AOT cache eliminates *recompilation* across runs once the compiler is JIT-able — so the gate lift needs **coverage** (this ADR) + warm cache, and benefits even if aggregate *execution* is delegation-heavy. This is the primary v0.11 deliverable.
- **≥10× perf bench** (ADR-0030 §14): measures *execution* speed. Delegate-to-VM aggregates narrow the gap (they remove bytecode-dispatch overhead from the arithmetic/control-flow glue) but a headline ≥10× on aggregate-heavy code likely wants native codegen (§7, deferred). The bench therefore targets a **JIT-friendly workload** (numeric/control-flow-heavy) for the ≥10× claim, and separately reports warm-cache bootstrap wall-time as the gate-lift evidence. Honest measurement over a flattering single number.

### §9 — Iterative, re-measured sub-task sequence

Each sub-task ships independently (its own tests + commit), and **re-runs the audit** to confirm coverage rose + nothing regressed:

1. **jit.4.agg.0** — §6 panic→tier-down (make iteration safe). Re-audit: 0 panics.
2. **jit.4.agg.1** — §1 struct ops (`FieldGet`/`FieldSet`) + §2 `StructNew` variadic ABI. Largest bucket (1314); also unblocks deferred f-string varargs.
3. **jit.4.agg.2** — §1 enum ops + Outcome ops (1489 combined).
4. **jit.4.agg.3** — §1 Nullable ops + §3 String/Null constants + loader `R_X86_64_64` extension (with the constraint-4 test regimen).
5. **jit.4.agg.4** — §4 Phi + §5 multi-block shim codegen (cross-cutting; may be needed earlier if it blocks re-measurement — order adjusts to the data).
6. **jit.4.gate** — once the audit shows the compiler ~fully JIT-able + warm-cache bootstrap < 10 min, wire the CLI `AotCacheStore` key path (deferred from jit.3 Step 4b) and lift `stage2_eq_stage3_main_tri_byte_identical` off `#[ignore]`. `criterion` warm-vs-cold bench per §8.

Re-measurement is the control loop: the audit's category counts are the burndown metric, so the plan self-corrects if interactions (a function blocked by *multiple* gaps) reveal the order is wrong.

## Hệ quả

**Possible (positive):**

- The self-host compiler becomes JIT-able → the AOT cache warm-loads it → the bootstrap byte-identical gate (`#[ignore]`'d since v0.7) lifts at CI-acceptable wall time. The headline v0.11 deliverable.
- The JIT covers the **whole language's data model**, not an integer-only subset — the first concrete step from "dev-tier JIT" toward the OS-capable production tier (VISION §4.3).
- Zero VM↔JIT divergence preserved: aggregate semantics stay single-sourced in the extracted VM helpers (ADR-0032 guarantee generalized).
- The deferred f-string/concat varargs (ADR-0032 jit.2b-iii) fall out of §2's array-ptr+len ABI for free.
- A real correctness bug (10 translator panics) is fixed (§6).

**Constrained (cost):**

- A large bring-up — effectively a second JIT coverage pass. Bounded + sequenced + re-measured (§9), but not small. No deadline pressure (author): correctness over speed.
- §3 re-touches the unsafe loader (one new relocation type) — held to the full ADR-0033 Addendum constraint-4 regimen; the only new unsafe surface.
- Delegate-to-VM aggregates are not maximally fast (a shim call + box per op); native codegen deferred (§7) — the ≥10× bench accounts for this (§8).
- VM per-opcode logic must be extracted into `pub` helpers — a refactor of the `vm.rs` instruction loop, kept behaviour-preserving (the existing VM tests are the guard).

**Costly (verify during implementation):**

- Interaction effects: a function may be blocked by several gaps at once, so coverage may rise non-linearly with sub-tasks. The re-measurement loop (§9) is the mitigation — trust the audit numbers over the plan order.
- `StructNew` stack-slot spill lifetime (§2) + String data-section relocation (§3) are the two novel ABI/loader pieces; both need explicit value-parity + (for §3) fuzz tests before they're trusted.

## Không làm (explicitly rejected)

- **Native aggregate codegen (struct layout in registers/memory) now** — higher risk (new layout codegen), not needed to lift the gate. Deferred to a post-v0.11 perf phase (§7).
- **Partial-program warm cache** (cache the JIT-able subset, VM-dispatch the rest, native code calling back into the VM for un-JIT'd callees) — considered as a shortcut to "warm cache without full coverage." Rejected: it adds a cross-tier call boundary (native→VM trampoline) = new unsafe surface + linker complexity, and it *masks* the coverage gap instead of closing it, leaving the compiler permanently part-interpreted — against the OS-capable trajectory. Full coverage is the honest path.
- **Re-deriving aggregate semantics in codegen** (instead of delegating to VM helpers) — would create a second source of truth for struct/enum/Outcome semantics → VM↔JIT divergence risk, the exact failure mode ADR-0032 §6 designed out. Rejected.
- **Continuing to defer** (a v0.12 "JIT coverage" phase) — the author's explicit instruction: v0.11 is already the trailer for v0.9/v0.10 deferrals; deferring again never closes the debt. Rejected.
- **A flattering ≥10× number on the compiler itself** — the compiler is aggregate-heavy + delegation-bound; quoting ≥10× there would misrepresent. The bench targets a JIT-friendly workload + reports warm-cache bootstrap wall-time separately (§8). Honest over flattering.

## Prior art

| Source | What we copy | What we change |
|---|---|---|
| [ADR-0032](0032-builtin-shim-abi.md) delegate-to-VM shims | The entire mechanism: `pub` VM wrapper + `__triet_*` shim + composite ABI + per-call sentinel | Generalize from `CallBuiltin` to the aggregate IR opcodes |
| HotSpot / V8 tiered JIT | Interpreter fallback for un-compiled constructs; bring coverage up incrementally | Triết: delegate-to-VM (not deopt); divergence-free by shared helper |
| GraalVM partial evaluation | Reuse interpreter semantics as the compilation source of truth | Triết: explicit shim boundary, not automatic PE |

**What we invented:** generalizing the builtin-shim delegate-to-VM pattern to cover the *full aggregate data model* as the bring-up path to production-tier coverage, driven by a re-runnable coverage audit as the burndown control loop.

## Tham chiếu

- [ADR-0030 §2 + §14](0030-jit-cranelift-integration.md) — tier-down policy (the fallback this ADR shrinks); the ≥10× bench chained here (§8).
- [ADR-0032](0032-builtin-shim-abi.md) — the delegate-to-VM shim ABI this ADR generalizes (§1); the deferred varargs cliff §2 unblocks; the single-block restriction §5 lifts.
- [ADR-0033 §9.5 + Addendum constraint 4](0033-aot-cache-cranelift-object.md) — the gate-lift chain this ADR unblocks; the loader-relocation test regimen §3 inherits.
- [VISION §4.3 + §6](../../VISION.md) — multi-tier execution / production tier (§7 altitude); refuse-over-guess (the audit-driven, no-deadline discipline).
- `crates/triet-bootstrap/tests/jit_tier_down_audit.rs` (commit `29aeeaa`) — the coverage measurement + burndown metric (§9).
- `crates/triet-ir/src/instr.rs` — the aggregate opcode definitions (§1).
- `crates/triet-ir/src/vm.rs` — the per-opcode VM logic to extract into `pub` helpers (§1).
