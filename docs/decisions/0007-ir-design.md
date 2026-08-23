# ADR 0007 — IR design: register-based SSA, multi-backend substrate

**Status:** Decided. Applicable to v0.3+ and all subsequent backends (v0.9 JIT, v2.0 AOT, v∞ trytecode). Represents the language $\leftrightarrow$ hardware boundary as per [VISION §4](../../VISION.md).

**Issue:** The v0.2 philosophy is a tree-walking interpreter running directly on the AST. This model does not scale to the following pillars:
- **CAS packaging (v0.5)** requires deterministic content hashing — AST node IDs change with every commit, making them unhashable.
- **Stable ABI (v0.4)** requires a stable signature hash on the IR, not the AST.
- **Self-hosting compiler (v0.7)** requires an IR format for the Triet-compiler (written in Triet) to emit.
- **JIT (v0.9, Cranelift)** compiles bytecode $\rightarrow$ machine code; a bytecode representation is required as a starting point.
- **AOT native (v2.0, LLVM)** maps Triet IR $\rightarrow$ LLVM IR; the more direct the mapping, the lower the engineering effort.
- **Trytecode native (v∞, ternary hardware)** maps Triet IR $\rightarrow$ instructions for actual ternary CPUs.

Deciding the IR shape *now* affects the entire pipeline from v0.3 $\rightarrow$ v∞. An error here would necessitate massive rewrites in multiple subsequent phases. This ADR locks the IR shape for all backends.

## Decision

Triet IR is **register-based, in SSA form, with infinite virtual registers and type-tagged per register**. The `.triv` wire format (ADR-0008, to be written when v0.3.8 begins) will serialize the same shape — there will be no separation between wire and in-memory formats in v0.3.

### Specific Form

3-address SSA instructions with a type tag for each register:

```
function @factorial(%n : Integer) -> Integer {
entry:
    %is_zero = eq %n, const Integer 0_integer
    br_if %is_zero, base_case, recursive_case

base_case:
    ret const Integer 1_integer

recursive_case:
    %n_minus_1 = sub %n, const Integer 1_integer
    %recurse   = call @factorial(%n_minus_1)
    %result    = mul %n, %recurse
    ret %result
}
```

**Characteristics:**
- Each virtual register `%name` is assigned **exactly once** (SSA invariant).
- Each register carries a type tag: `Trit`, `Tryte`, `Integer`, `Long`, `Trilean`, `String`, `Unit`, `T?`, or user-defined struct/enum/closure.
- Functions are divided into **basic blocks**, each ending with a terminator (`ret`, `br`, `br_if`, `match`, `unreachable`).
- **Phi nodes** at the entry of a block merge values from multiple predecessors: `%v = phi [%a from L1], [%b from L2]`.
- **Constants** do not consume registers — `const Integer 42_integer` is an inline operand.
- The number of virtual registers is unlimited; the backend (Cranelift, LLVM) handles register allocation.

### Instruction Grouping (high-level, non-exhaustive)

This ADR does not list the full instruction set — details will land in v0.3.1 (within the `triet-ir` crate scaffold). Categorization:

| Group | Example opcodes |
|---|---|
| **Constants** | `const Integer 4le_integer`, `const String "hello"`, `const Trilean unknown` |
| **Arithmetic** | `add`, `sub`, `mul`, `div`, `mod`, `pow`, `neg` (for Tryte/Integer/Long) |
| **Trit/Trilean logic Ł3** | `trit_not`, `trit_and`, `trit_or`, `luk_implies`, `luk_xor`, `luk_iff` |
| **Trit/Trilean logic K3** | `kleene_implies`, `kleene_xor`, `kleene_iff` |
| **Comparison** | `eq`, `ne`, `lt`, `le`, `gt`, `ge` (result is Trilean but never unknown — value equality per SPEC §4.5) |
| **Conversion** | `to_integer`, `to_tryte`, `to_long`, `to_trit`, `to_trilean` (+ saturating/truncating variants) |
| **Control flow** | `br <label>`, `br_if <cond>, <true_label>, <false_label>`, `match <scrutinee>, [arms]`, `ret`, `unreachable` |
| **Function** | `call @func(%args)`, cross-module via `AbsolutePath` from `triet-modules` |
| **Aggregate** | `struct_new`, `field_get`, `field_set`, `enum_new`, `enum_tag`, `enum_payload` |
| **Nullable** | `null_wrap` (T $\rightarrow$ T?), `null_unwrap` (T? $\rightarrow$ T, panic), `null_check` (T? $\rightarrow$ Trit) |
| **Closure** | `closure_new @lambda, [captures]`, `closure_call %c, [args]` |
| **Builtin** | `builtin "<name>", args` (for `println`, `assert`, ...) |
| **Outcome** *(v0.7.4.3-error+, [ADR-0020](0020-outcome-error-handling.md))* | `outcome_new_positive` (Trit::Positive arm), `outcome_new_negative` (Trit::Negative arm), `outcome_new_null` (Trit::Zero arm, T?~E only), `outcome_discriminant` (extract trit), `outcome_unwrap_value` (panic if not Positive), `outcome_unwrap_error` (panic if not Negative). Wire opcodes 0xC1–0xC6. Cross-references nullable `T?` via shared `Constant::Null` for compile-time null literals (per [ADR-0010 Addendum](0010-ternary-native-ir.md#addendum--v0743-error-null-literal-unification)). |

**Capability annotation:** Cross-module calls carry a namespace tag from `AbsolutePath` (e.g., `call @sys.print %s` calls into `sys.*`). The v0.6 capability check will read this tag — no IR shape change is required to enforce capabilities.

### Wire format (defer to ADR-0008)

- v0.3.0–v0.3.7: in-memory IR == wire format. Serialization uses Rust `bincode` or equivalent for speed — NOT an official binary format.
- v0.3.8 (ADR-0008): design the official `.triv` format — magic bytes, version field, sections (header / constant pool / function table / code), varint encoding.
- Stable for v1.0 freeze. Post-v1.0: additive-only; all backends must be able to read legacy IR.

## Rationale

### Mapped to SPEC design principles

| SPEC § | Principle | Application to register SSA |
|---|---|---|
| §0.3.1 | **AI-first** — explicit > implicit, low ambiguity > terseness | SSA register IR is human-readable: `%result = mul %n, %recurse` clearly shows data flow. Stack IR hides data flow via stack position — LLMs must simulate the stack in their "mind" when reading. LLM training data is dense with LLVM IR / Rust MIR (similar architectures) — enabling correct code generation immediately. |
| §0.3.4 | **Stability over speed** — ADR-driven, no "ship now, fix later" | SSA registers are the dominant architecture in modern compilers post-2010 (LLVM, Rust MIR, Swift SIL, Cranelift IR, GCC GIMPLE). A conservative choice with strong prior art. Stack VMs are legacies from the "languages only run on VMs forever" era (JVM 1995, Wasm 2017) — not applicable to Triet AOT-native. |
| §0.3.5 | **Refuse over guess** — clear errors, no silent inference | The SSA invariant ("each register defined exactly once") is a powerful static check with a simple verifier. Compiler bugs are caught early and do not propagate to the backend. |
| §0.3.6 | **Explicit > implicit** — explicit export, capability, and dependency | Register IR uses explicit operand names. Stack IR has implicit data flow via stack position — stack effect bugs are difficult to debug. |
/n| §0.2 | **Ternary first-class** — Trit/Tryte/Integer/Long are primitive fixed-size types | Type tags per register allow `%t1 : Trit`, `%t2 : Trilean`, `%t3 : Long` to carry type info from AST $\rightarrow$ IR $\rightarrow$ backend. Stack IR reduces everything to generic "stack slots" — type info must be stored in auxiliary metadata, which is prone to drift. |

### Multi-backend trajectory (VISION §4)

This is the pivotal reason. Triet is an **AOT-native language with a multi-backend strategy**, not a VM-based language. The IR must be stable and map well to **all backends**, not just the v0.3 VM.

| Backend | Phase | IR target | Mapping from SSA register |
|---|---|---|---|
| Bytecode VM | v0.3 | (Direct Triet IR) | Trivial — VM interprets IR directly |
| JIT Cranelift | v0.9 | Cranelift IR (SSA register) | 1:1 mapping — same paradigm |
| AOT LLVM | v2.0 | LLVM IR (SSA register) | 1:1 mapping — same paradigm |
| Trytecode native | v∞ | Ternary CPU instructions (register-based) | Near 1:1 mapping — physical CPU is also register-based |

If Triet IR were stack-based, **every backend would need to implement its own stack-to-SSA lifting pass** — a 3x engineering effort (Cranelift, LLVM, trytecode) that would persist through v0.9/v2.0/v∞, rather than a one-time investment in v0.3.

### Triet-specific considerations

**Trilean ops must be first-class opcodes**, not function calls. Ł3 and K3 are primitive languages per SPEC §4 — the v0.2 interpreter dispatches them directly without function overhead. The IR must preserve these semantics: `luk_implies` $\neq$ `kleene_implies`, dispatched at the IR level. Capability checks (v0.6) read the opcode to determine if the user intentionally chose Ł3 or K3.

**Long arithmetic is backend-specific.** A `Long` is an 81-trit big-int, which does not fit in standard native CPU registers (>128 bit). Backend lowering:
- VM v0.3: heap-allocated big-int (similar to the `bnum::I256` used in v0.2).
- AOT LLVM v2.0: `i256` or runtime calls to `libgmp`.
- Trytecode v∞: native 8/1-trit register (as ternary hardware supports it).

The IR opcode `add` for `Long` emits the same instruction; the backend handles the lowering. This follows the proven LLVM IR pattern: high-level ops + backend-specific lowering.

**Capability namespaces are preserved.** Cross-module function calls carry the `AbsolutePath` from `triet-modules`. The IR encodes `call @std.io.println %s`, NOT stripped to `call @println %s`. Reasons:
- v0.6 capability checks must know the namespace to enforce rules (`usr.*` cannot call `dev.*` without capability).
- v0.5 CAS hashing must include the namespace to ensure stable identity during intra-module renaming.
- Debug output remains clear for both LLMs and developers.

**Nullable `T?` discriminator is preserved.** Per SPEC §2.5 + ADR-0001: `T?` is a 1-trit discriminator + a `T` payload. IR ops `null_wrap`/`null_unwrap`/`null_check` are explicit, not implicit. Matching a nullable pattern lowers to `null_check` + `br_if` + `null_unwrap`.

**Pattern matching exhaustiveness is enforced at the IR verifier.** SPEC §7.3 requires exhaustive matching. The lowerer checks for exhaustiveness and emits a `match` opcode with a complete list of arms; the verifier re-validates this invariant. Lowering results in a cascade of `br_if` instructions in the backend.

**Memory model is deferred to the v0.3 implementation.** SPEC §10 states the memory model will be finalized in v0.4 (ABI). The v0.3 IR will assume Mojo-style ARC, as used in v0.2. A separate ADR (likely ADR-0009 or equivalent in v0.4) will refine this: ARC opcodes (`retain`, `release`) and borrow checking at the IR level. v0.3 IR can add these opcodes additively later.

### Error code namespace

Namespace expansion per CLAUDE.md:

| Range | Component |
|---|---|
| `E0000` | Lexer |
| `E000X` | Parser |
| `E10XX` | Typecheck |
| `E20XX` | Interpreter (tree-walking, v0.2) |
| `E21XX` | Modules (loader/resolver) |
| **`E22XX`** | **VM runtime (v0.3)** — out of bounds, stack overflow, type tag mismatch, null unwrap, etc. |
| `E23XX` | (reserved for IR verifier once v0.3.1 scaffold is complete) |

## Alternatives Considered

### A1. Stack-based bytecode (JVM, Wasm, CPython 3.x, .NET CIL)

**Reject.**

Pros:
- Simpler implementation for VM v0.3 (~30% less code).
- More compact wire format (opcodes do not require explicit operands).
- Massive prior art (JVM for 30 years, Wasm as a modern standard).

Cons (Fatal):
- **Implicit data flow** violates AI-first §0.3.1 — LLMs/devs must simulate the stack mentally to read the IR.
- **Type info is stripped** down to generic "stack slots" — violates Ternary first-class §0.2.
- **Every AOT/JIT backend must perform stack $\rightarrow$ SSA lifting** — a permanent cost through v0.9/v2.0/v∞.
- **Stack effect verification is weaker than the SSA invariant** — bugs are harder to catch early.
- **Wasm precedent does not apply** — Wasm chose a stack for web sandboxing + small wire size for downloads. Triet has no such constraints.
- **JVM precedent does not apply** — JVM chose a stack because 1995 hardware had few registers; modern hardware is register-rich.

Most importantly: Triet's end-game is AOT native (v2.0 LLVM) + trytecode native (v∞). Optimizing the IR for the VM is a misallocation of resources.

### A2. Tree-IR / direct AST execution

**Reject.** This is the current v0.2. It does not scale to CAS, ABI, JIT, or AOT because the AST is unstable, unhashable, and cannot be lowered to machine code.

### A3. CPS (Continuation-Passing Style) IR

**Defer.** This is appropriate for the concurrency model (v0.8 actor) — first-class continuations map naturally to green threads. However:
- Implementation is complex + difficult for LLMs to learn.
- Triet v0.3 does not have concurrency yet.
- A CPS-conversion pass could be added *on top* of the SSA IR once the v0.8 actor model is defined.

Not rejected permanently — just not the right time.

### A4. MLIR-style multi-level IR (multiple dialects)

**Defer.** This is beneficial for compilers with many domain-specific dialects (Tensor + scalar + GPU). Triet drops SIMD/Tensor (removed in SPEC §10.5) $\rightarrow$ no need for multi-dialects. This could be adopted in v2.0+ if the LLVM backend requires custom optimization passes.

### A5. Direct lowering AST $\rightarrow$ LLVM IR (skipping Triet IR)

**Reject.**

- LLVM lock-in. The Trytecode backend (v∞) would be blocked.
- Self-hosting (v0.7) would be unfeasible — the Triet-compiler (written in Triet) must be able to emit something simpler than LLVM IR.
- JIT (v0.9 Cranelift) would also require duplicate lowering — it wouldn't share a pipeline with AOT.
- LLVM build dependency is massive — violates the "incremental progress, validate IR early" principle of Stability over speed.

Triet IR acts as the **buffer between Triet source and all backends** — this separation of layers is mandatory.

### A6. Continuation-with-block-args (CFG IR without phi nodes, similar to early Cranelift)

**Consider.** Cranelift IR originally used block arguments instead of phi nodes — theoretically equivalent to SSA, sometimes simpler to implement. Triet may adopt this if, during v0.3.1, phi nodes are found to be unnecessarily complex.

**Decision to be made in v0.3.1**, not locked in by ADR-0007. The ADR will be updated if block arguments are chosen.

## Consequences

**Positive:**
- Multi-backend trajectory (VM $\rightarrow$ JIT $\rightarrow$ AOT $\rightarrow$ trytecode) means each backend only needs its own lowerer — no IR redesign.
- IR is human-readable for debugging + LLM-readable for AI assistance (sharing the same training data architecture).
- SSA invariant catches IR bugs early in the verifier.
- 1:1 mapping to LLVM IR (v2.0) — minimal lowering effort.
- Type tag per register preserves Ternary first-class status throughout the pipeline.
- Capability namespace is preserved $\rightarrow$ v0.6 enforcement does not require IR changes.

**Negative:**
- v0.3 implementation effort is ~30% higher than a stack VM. This tradeoff is accepted because all subsequent phases will be significantly more efficient.
- The wire format may be larger than stack bytecode (operands are explicit). Mitigation: ADR-0008 will design varint encoding for compact storage.
- Requires a register allocator in the target backends (v0.9 JIT, v2.0 AOT). Cranelift/LLVM will handle this; Triet does NOT need to implement register allocation. The v0.3 VM does not require register allocation (infinite virtual registers, direct dispatch).

**Migration strategy:**
- v0.2 baseline: tree-walking interpreter. Retained for differential testing in v0.3.7.
- v0.3.1–v0.3.7: IR scaffold + lowerer + VM. The tree-walker acts as the oracle; the VM must match it byte-by-byte.
- v0.3.8+: `.triv` bytecode binary format. CLI adds `dao build` subcommand.
- v0.4+: ABI metadata + CAS hashes read from IR/`.triv`.
- v0.7: Self-hosting compiler emits the same IR shape.
- v0.9: Cranelift backend for JIT — new, but does not change IR.
- v2.0: LLVM backend for AOT — new, but does not change IR.
- v∞: Trytecode backend — new, but does not change IR. Only adds opcode lowering rules for the ternary CPU.

**Breaking changes to the IR require a separate ADR** (per the precedent of ADR-0005). Post-v1.0: changes will be additive only, preserving backward compatibility.

## Implementation roadmap (v0.3.0 $\rightarrow$ v0.3.7)

Detailed sub-tasks are in [TODO.md § v0.3](../../TODO.md). Outline:

1. **v0.3.0 — ADR-0007** (this) ✓ — IR shape decided.
2. **v0.3.1 — Scaffold `triet-ir` crate** — concrete instruction set, constant pool, basic block + function + module types, display formatting, IR verifier.
3. **v0.3.2 — Lowerer AST $\rightarrow$ IR (core)** — literals, arithmetic, Ł3/K3 logic, comparison, control flow.
4. **v0.3.3 — Lowerer items + functions + modules** — function definitions, generics monomorphization, cross-module calls with `AbsolutePath`.
5. **v0.3.4 — Lowerer aggregates + match + closures** — struct/enum/closure/builtins/nullable.
6. **v0.3.5 — VM execution** — interpret IR via opcode dispatch; must produce the same output as the tree-walker.
7. **v0.3.6 — Snapshot tests** — IR output for all `examples/*.tri`.
8. **v0.3.7 — Differential tests** — VM $\equiv$ tree-walker byte-by-byte for all examples.

Thereafter:
- **v0.3.8 — ADR-0008** (to be written) — `.triv` bytecode binary format.
- **v0.3.9 — Serializer/desertializer** — round-trip IR $\leftrightarrow$ `.triv`.
- **v0.3.10 — CLI rewire** — `dao build` + `.triv` execution.
- **v0.3.11 — Benchmark + gate verification** — benchmark $\ge$ 3$\times$ tree-walker speed.

## References

- [LLVM IR Reference](https://llvm.org/docs/LangRef.html) — SSA register design, primary prior art.
- [Rust MIR Documentation](https://rustc-dev-guide.rust-lang.org/mir/index.html) — high-level IR for Rust, the closest prior art to Triet (similar source language complexity).
- [Swift SIL](https://github.com/apple/swift/blob/main/docs/SIL.rst) — generics + ABI considerations at the IR level (prior art for v0.4 ABI).
- [Cranelift IR Reference](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md) — JIT-friendly SSA IR, target backend for v0.9.
- ["A Look at the Lua 5 Implementation"](https://www.lua.org/doc/jucs05.pdf) — register VM with fixed register count, reference for VM dispatch.
- [WebAssembly Specification](https://webassembly.github.io/spec/) — stack-based wire format (the rejected alternative).
- [SSA Book — Static Single Assignment Book](http://ssabook.gforge.inria.fr/latest/book.pdf) — SSA theory, phi nodes, dominance.
- [GHC Cmm](https://gitlab.haskell.org/ghc/ghc/-/wikis/commentary/compiler/cmm-type) — alternative C-- style IR for functional languages (deferred reference).

## Related

- [ADR-0005](0005-module-system.md) — Module system: `AbsolutePath` is the input for IR cross-module calls.
- ADR-0008 (to be written, v0.3.8): Bytecode binary format `.triv`.
- ADR-0009 (to be written, v0.4): ABI metadata format — read from IR.
- ADR-0012 (to be written, v0.5): CAS hashing scheme — `iface_hash` on IR signature, `impl_hash` on IR body.
- ADR-0014 (to be written, v0.6): Capability type system — reads capability tags from IR.
- [VISION §4](../../VISION.md) — Multi-backend execution model.
- [SPEC §0.3](../../SPEC.md) — Design principles (AI-first, Stability over speed, Ternary first-class).
- [ROADMAP § v0.3](../../ROADMAP.md) — Phase deliverables + gates.

---

*This decision freezes the IR shape for Triet. Breaking changes to the IR require a separate ADR (per ADR-0005). Binary wire format details (`.triv`) are deferred to ADR-0008 when v0.3.8 begins.*
