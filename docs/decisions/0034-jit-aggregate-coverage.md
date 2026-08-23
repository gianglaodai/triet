# ADR 0034 — JIT aggregate coverage via delegate-to-VM shims (struct / enum / Outcome / Nullable / String, + Phi, multi-block shims, panic→tier-down)

**Status:** **Locked** (v0.11.x.jit.4, author sign-off — "Direction A: stop deferring, make the compiler fully JIT-able"; value-representation model locked in the 2026-06-01 Addendum below). Builds on [ADR-0032](0032-builtin-shim-abi.md) (the delegate-to-VM shim ABI it generalizes) and unblocks the bootstrap byte-identical gate lift chained from [ADR-0033 §9.5](0033-aot-cache-cranelift-object.md). First ADR opened to *close* deferred JIT-coverage debt rather than defer it.

> **2026-06-01 Addendum — value-representation model LOCKED: per-function uniform boxing (Tier A). Native aggregate codegen (Tier C) is the dedicated post-v0.11 runtime-speed phase. Author sign-off.**
>
> Implementing §1 surfaced a design question §1 had glossed: **how does the JIT represent intermediate SSA values whose static type is unknown?** The IR (`triet_ir::Function`) carries types **only at function boundaries** (params + `return_type`); there is no per-`ValueId` type table, and a struct's field types are **erased at lowering** (`RuntimeValue::Struct` is just `Vec<RuntimeValue>`, unlabelled). So at `%d = field_get %obj, i`, codegen cannot know whether the field is an `Integer` (unboxed `i64`) or a `String` (boxed `i64` ptr) — and a fixed-signature shim that always returned a boxed ptr would be misread by a downstream `Add` expecting an unboxed `i64` → **silent miscompilation**, the exact class the project guards hardest.
>
> **Three tiers were weighed (not two):**
> - **Tier A — per-function uniform boxing (CHOSEN for v0.11).** A function compiles in one of two modes: (1) **all-integer** (no aggregate opcode) → today's fully-**unboxed** fast path, every value an `i64`, unchanged; (2) **aggregate-touching** → fully-**boxed**, *every* SSA value is a `Rc<RuntimeValue>` ptr and *every* opcode (incl. `Add`) is a delegate-to-VM shim. Within a boxed function there is no box/unbox ambiguity (everything is a ptr → `field_get`/`add` all take + return ptrs), so the miscompile risk **vanishes by construction**. Boundary conversions use the **known** param/return types (+ a callee's known `return_type` at cross-fn call sites). **No IR change, no `.triv` bump, no self-host port** — boundary types already exist. Correct (zero VM↔VM-delegation divergence), full coverage, cacheable → lifts the gate. Modest runtime speed (a shim call + heap box per op) — slower than ideal but faster than the VM (no bytecode-dispatch loop).
> - **Tier B — typed boxed + unbox-for-arithmetic (REJECTED).** Keep aggregates boxed but unbox primitives for native CPU arithmetic. Needs a per-`ValueId` type table → IR change + `.triv` bump + **self-host lockstep** (the Stage 2 ≡ Stage 3 byte-identical gate forces `compiler/*.tri` to emit the same type info). Significant cost, yet it is a **middle tier**: still heap-boxes aggregates + shim-accesses fields, so it is **not** the kernel-grade runtime-speed destination. Worst position — big cost, not the endgame.
> - **Tier C — native aggregate codegen (DEFERRED to a dedicated post-v0.11 phase).** Real data layout: a `Point` is two `i64`s in registers/stack, `field_get` is a `load`, no `RuntimeValue`, no shim, no heap. This is the actual kernel-grade runtime-speed tier (the [VISION §4.3](../../VISION.md) production tier).
>
> **Why this staging — grounded in the author's stated priorities (kernel/OS/app at all levels; for kernel: compile-time safety+correctness AND high runtime speed; end of v0 → careful, not rushed):**
> 1. **Safety + correctness first.** Tier A is the only tier that *cannot* diverge from VM semantics (it runs the VM's own logic via shims). Critically, the boxed path becomes the **correctness oracle**: every later native-codegen op (Tier C) is verified to produce the *same value* as the boxed/VM path on a corpus. Jumping straight to B/C without this oracle builds the fast path with no divergence-checking net — against "safety + correctness first."
> 2. **Runtime speed comes from Tier C, not B.** So the large, careful investment is spent **directly on the destination** (native layout), not on an intermediate boxed-with-unboxing tier that never reaches kernel speed. Tier C needs type info anyway — that investment is made where it pays off.
> 3. **A is the foundation for C, not a detour.** Per-function dispatch lets native codegen be introduced **incrementally + verifiably** (one op at a time, boxed path as oracle + as fallback for not-yet-native ops) — the standard "correct slow tier first, then optimize hot paths against it" JIT-construction discipline. This *is* the slow-but-steady path to kernel speed, not a shortcut around it.
>
> **Honest scope statement:** Tier A **alone does not meet the kernel-grade runtime-speed pillar** — it is a stepping stone. The runtime-speed pillar is delivered by the **Tier C native-codegen phase** (proposed v0.12 / pre-v1.0, its own ADR), built on the verified Tier A baseline. v0.11's deliverable is the gate lift (coverage + cache), which Tier A fully achieves.
>
> **Supersedes:** §1's framing ("opcodes lower to shims" with an implicit composite ABI) is now read as Tier A per-function uniform boxing. §7's "delegate-to-VM for coverage now; native deferred" is sharpened into this explicit A→C staging. All other sections unchanged.

## Issue

v0.11.x.jit.3 shipped a complete, reviewed AOT cache (per-module objects + a Triết-owned load-time linker, [ADR-0033](0033-aot-cache-cranelift-object.md) + Addenda). But the cache + the bootstrap gate lift it was meant to unblock both rest on a tacit premise: **that the self-host compiler is JIT-able.** A coverage measurement (`triet-bootstrap::jit_tier_down_audit`, commit `29aeeaa`, a resilient dry-JIT that records every function's tier-down reason) overturned that premise:

```
compiler/main.tri — 3953 functions
  JIT-able : 146  (3.7%)
  tier down: 3807 (96.3%)
  by category:
    1314  struct ops (struct_new / field_get / field_set)
     760  Outcome ops (outcome_discriminant / wrap / unwrap)
     729  enum ops (enum_new / enum_tag
