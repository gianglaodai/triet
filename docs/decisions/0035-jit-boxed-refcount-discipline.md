# ADR 0035 — JIT boxed-value refcount discipline (clone-on-return, cross-mode marshaling, leak tolerance)

**Trạng thái:** **Proposed** (v0.11.x.jit.4.agg.cross-call, awaiting author sign-off). Builds on [ADR-0032 §2](0032-builtin-shim-abi.md) (the composite `Rc::into_raw` boxed-ptr ABI + `__triet_drop_arc` lifetime shim) and [ADR-0034](0034-jit-aggregate-coverage.md) (Bậc A per-function uniform boxing — the boxed mode whose values this discipline governs). Opened to *close* a latent-double-free class surfaced while implementing cross-mode calls, before it spreads further.

## Issue

[ADR-0034](0034-jit-aggregate-coverage.md) Bậc A compiles each function in one of two modes — **unboxed** (all-integer fast path, every value an `i64` scalar) or **boxed** (aggregate-touching, every value an `Rc<RuntimeValue>` ptr). Composite values cross the shim ABI as `Rc::into_raw` raw `i64` pointers ([ADR-0032 §2](0032-builtin-shim-abi.md)): a fresh box has strong-count 1, and the JIT register that holds it owns that +1, balanced by exactly one `__triet_drop_arc` at last use / `Ret`.

The lifetime rule ADR-0032 §2 wrote down was: *"composite PARAMS are borrowed (caller owns + drops); function-CREATED boxes are owned (dropped at Ret unless returned)."* Implementing cross-mode calls (agg.cross-call) showed this rule is **incomplete for one case it never names: returning a borrowed value.**

When a boxed function returns one of its **parameters** (or any value it borrows rather than creates), the `Ret` hands the borrowed `Rc` pointer to the caller. The caller treats every returned box as **owned** — it records it and drops it at *its* `Ret`. But the original owner (whoever passed the argument) **also** drops it. The same box is freed twice → use-after-free / double-free. This was caught concretely: a composite cross-mode pass-through test aborted with `malloc(): unaligned tcache chunk detected` (glibc's double-free tripwire).

Three findings make this a discipline-level decision rather than a one-line patch:

1. **It is latent in same-mode boxed calls too.** A boxed `keep(p, q) = { _ = p.field(0); ret q }` returns borrowed param `q`. Present since [ADR-0034](0034-jit-aggregate-coverage.md) agg.1c-iii; not yet triggered by the self-host compiler only because such functions hadn't been JIT-exercised. (Fixed in `b90dfed` — see §1 — but recorded here as the motivating case.)

2. **The unboxed mode has the same hole.** The unboxed `Ret` arm (`codegen.rs`) also returns its operand as-is. An unboxed function returning a borrowed **composite** parameter (`id(s: String) = s` — composites are `i64` ptrs even in unboxed mode) double-frees identically. Present since jit.2b; narrow (composite-in-unboxed is a small surface) but real.

3. **Cross-mode composite marshaling needs it on both sides.** A boxed caller calling an unboxed callee that returns a borrowed composite gets an aliased pointer; the unboxed callee performs no refcount bump. Without a discipline, every composite crossing the boxed↔unboxed boundary is a potential double-free — which is why agg.cross-call.a shipped **scalar-only** and tiers composite boundaries down.

The question: **what is the single refcount rule that makes every boxed-value return + cross-mode crossing memory-safe, and what is the explicit cost (leak tolerance) we accept to keep it simple?**

## Quyết định

**Adopt one rule — *a `Ret` transfers exactly one owned reference; clone any borrowed return to mint it* — applied uniformly to both modes, plus a *clone-the-cross-mode-composite-result* rule for the boxed↔unboxed boundary, accepting bounded dev-tier leaks where the alternative is per-callee ownership tracking.** The boxed JIT is the correctness *oracle* ([ADR-0034](0034-jit-aggregate-coverage.md)), so "memory-safe + value-correct, possibly leaky" is the right altitude; precise (leak-free) lifetimes belong to the Bậc C native-codegen phase that replaces boxing entirely.

### §1 — Clone-on-return (both modes) — DONE for boxed (`b90dfed`)

Invariant: **at `Ret`, the returned pointer must carry exactly one owned strong-count that transfers to the caller.**

- Returned value is a **function-created box** (∈ `created_boxed`) or a **fresh inline-`Const` box** → already owns 1 → transfer as-is (skip its `drop_arc` in the at-`Ret` drop loop). No clone.
- Returned value is **borrowed** (a `Value(id)` ∉ `created_boxed`: a parameter, or a φ / pass-through resolving to one) → owns 0 here → emit `__triet_clone_arc(ptr)` (strong-count +1) to mint the owned reference the caller balances. The borrow stays intact for the original owner.

`__triet_clone_arc(ptr) -> ptr` is a framework shim (`Rc::increment_strong_count`, null-safe), the symmetric partner of `__triet_drop_arc`. The clone is correct in both single- and multi-block functions (it fixes the very case the multi-block drop-skip cannot — [ADR-0034](0034-jit-aggregate-coverage.md) agg.1c-iv).

**Boxed mode: implemented in `b90dfed`.** **Unboxed mode: TO DO under this ADR** — the unboxed `Ret` must clone a borrowed return *only when the return type is a composite* (`String`/`Vector`/`HashMap`/`Nullable`/`Atomic`/`Outcome` → an `Rc` ptr); a scalar return (`Integer` i64, `Trilean` i8, …) is value-copy and must **not** be cloned. Cranelift type alone cannot distinguish a composite `i64` ptr from an `Integer` `i64`, so the unboxed clone-on-return consults `func.return_type` (a `TypeTag`, already available — no IR change).

### §2 — Cross-mode composite: clone the result in the boxed caller

When a **boxed** caller calls an **unboxed** callee (or vice-versa) and the crossing value is a composite:

- **Arguments** pass through unmarshaled (same `Rc`-ptr representation in both modes) and are **not** consumed by the callee (an unboxed callee never drops a param; a boxed callee treats params as borrowed) → the caller retains ownership, no action needed.
- **The composite result** is cloned by the **boxed caller** immediately after the call (`__triet_clone_arc`). Rationale: the caller cannot know whether the unboxed callee returned a freshly-created box (owned, +1) or — once §1's unboxed clone-on-return lands — a cloned borrowed box (also owned, +1). Both are owned, so cloning the result yields strong-count 2 where 1 would suffice → a **bounded leak of one box per cross-mode composite-returning call** (the caller drops 1 at its `Ret`, the +1 from the callee's own discipline is never dropped by anyone). Memory-safe (never a double-free); leaks are the accepted dev-tier cost (the same tolerance [ADR-0032 §2](0032-builtin-shim-abi.md) already documents for the error path + inline-const operands).

This keeps the boundary rule **local and uniform** — "clone every cross-mode composite result" — instead of threading per-callee ownership facts through the call site.

### §3 — Leak tolerance is explicit and bounded

Accepted leaks under this discipline, each **one box per occurrence**, all on cold/rare paths or proportional to cross-mode composite calls (not per-iteration in hot loops):

- Cross-mode composite result over-clone (§2).
- Multi-block boxed function created-box drop-skip ([ADR-0034](0034-jit-aggregate-coverage.md) agg.1c-iv — superseded by Bậc C, not this ADR).
- Error-path created boxes + inline-const operand boxes ([ADR-0032 §2](0032-builtin-shim-abi.md)).

**Why acceptable:** Bậc A is the *oracle*, not the production runtime ([VISION §4.3](../../VISION.md) — production speed/footprint is the Bậc C native tier + AOT LLVM v2.0). A bounded leak is memory-*safe* (no UB), and the oracle's job is value-correctness + coverage, not zero-footprint. Precise leak-free lifetimes are a goal of Bậc C, where values aren't heap-boxed at all. **Constraint:** any new leak site must be `log`-able / countable so it can't masquerade as "no leak"; the discipline never trades a leak for a double-free.

### §4 — `TypeTag::Unit` ambiguity bounds cross-mode coverage (recorded, not fixed here)

The lowerer maps every user struct / enum / generic-type-parameter to `TypeTag::Unit` (`lowerer.rs` `_ => TypeTag::Unit`), indistinguishable from a true zero-sized `Unit`. So a cross-mode boundary typed `Unit` **cannot be classified** (box-a-scalar? pass-a-composite-ptr? a real nothing?) and must tier down — even after §1–§3. Since the self-host compiler's dominant aggregate is the AST (structs → `Unit`), this caps cross-mode composite coverage until the lowerer distinguishes user-aggregate from true-`Unit` (a `TypeTag` addition → IR change + `.triv` bump + self-host lockstep). **Out of scope for this ADR** (it is an IR-shape decision, not a refcount one); flagged so the cross-mode coverage ceiling is not mistaken for a refcount bug.

## Không làm

- **Per-callee ownership tracking (return-is-borrowed bit) instead of §2's blanket clone.** Would avoid the §2 leak by recording, per function, whether its return aliases a parameter, and cloning only then. Rejected for v0.11: it is a whole-program escape analysis (a return may alias through nested calls / φ), high-cost + bug-prone, and buys only the elimination of a bounded cold-path leak. The oracle does not need it; Bậc C eliminates boxing entirely. Revisit only if a leak shows up hot in a real profile.
- **A garbage collector / `Arc` cycle collector for boxed values.** `RuntimeValue` is a tree (no cycles by construction — no interior mutability that forms `Rc` cycles in the value model), so refcounting suffices; a GC would be a managed-runtime intrusion exactly the kind [VISION §4.4](../../VISION.md) forbids baking into the execution model.
- **Making unboxed mode refcount-discipline-free by forbidding composite returns from unboxed functions.** Would "solve" §1 finding 2 by tiering down any unboxed function returning a composite. Rejected: it shrinks coverage for no safety gain over the cheap `TypeTag`-guided clone, and pushes more functions onto the slower boxed path.
- **Fixing the `TypeTag::Unit` ambiguity (§4) in this ADR.** Deferred — it is an IR-shape change with self-host lockstep cost, orthogonal to the refcount rule. Tracked in `TODO.md` agg.cross-call.

## Prior art

- **Rust's own `Rc`/`Arc` clone-on-share + drop-on-scope-exit** — this ADR is literally that discipline, hand-emitted in Cranelift IR because the JIT has no Rust ownership tracker over its SSA values. `__triet_clone_arc`/`__triet_drop_arc` = `Rc::clone`/`Drop`.
- **Objective-C / Swift ARC (Automatic Reference Counting)** — compiler-inserted retain/release at ownership transfer points; clone-on-return is ARC's "+1 returned object" convention (the `ns_returns_retained` family). Triết's boxed mode is a minimal hand-rolled ARC for the oracle tier.
- **JVM/CLR escape analysis** — the rejected "per-callee ownership tracking" alternative is essentially escape analysis; deferred for the same reason those VMs gate it behind the optimizing tier, not the baseline.

## Tham chiếu

- [ADR-0032 §2](0032-builtin-shim-abi.md) — composite `Rc::into_raw` boxed-ptr ABI + `__triet_drop_arc` + the leak-tolerance precedent this ADR extends.
- [ADR-0034](0034-jit-aggregate-coverage.md) — Bậc A per-function uniform boxing (the boxed mode governed here) + the oracle/Bậc-C staging that justifies §3's leak tolerance.
- [VISION §4.3 / §4.4](../../VISION.md) — production tier is native (Bậc C / AOT LLVM), not the boxed oracle; no managed runtime baked into the execution model.
- `TODO.md` — v0.11.x.jit.4.agg.cross-call (clone-on-return `b90dfed`, scalar marshaling `0732a35`, the cross-cutting finding, and the §4 `TypeTag::Unit` ceiling).
