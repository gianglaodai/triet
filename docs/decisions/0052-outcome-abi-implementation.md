# ADR-0052: Outcome ABI Implementation — 2-Slot MIR + Cranelift Multi-Return

## 1. Status
**Approved (O + G, 2026-06-10).** Builds upon **ADR-0020** (Outcome design-locked: syntax `~+`/`~-`/`~0`,
type `T~E`/`T?~E`, type-level semantics). ADR-0020 locked the **surface syntax/semantics**;
ADR-0052 locks the **low-level ABI** — how to lower Outcome to MIR (2-slot) + pass through
Cranelift JIT (multi-value return). This is a Low-level ABI Contract (altering how Triet functions
return data via registers/FFI).

**G Constraints (invariants — do not violate):**
1. **Payload SCALAR ONLY (Tier A): Integer/Trit/Trilean.** Heap payloads (String/Vector inside
   Outcome) = Tier B/C DEFERRED (ownership/drop/borrow across multi-return ABI = separate landmine).
2. **Un-defer C5 ONLY for `BinaryOutcome`/`TernaryOutcome`.** Remove guard `values.len()>1`
   (`jit:1070`) EXCLUSIVELY for Outcome — DO NOT open generic Tuple-returns.
3. **Cranelift native multi-return** — each value is 1 i64, WITHOUT touching the "single i64"
   value model (light premise proven by C5 spike).

## 2. Context
Outcome (error-handling core, ADR-0020) Frontend + Typecheck are ready, but **Lower is degenerate**:
`~+ e` = identity (lowers payload, does not produce 2-value), `~-` = unsupported (`lower:1124`).
`ReturnShape::BinaryOutcome` (arity 2) + `OutcomeDiscriminant`/`OutcomeUnwrap` MIR ops are defined
but have **0 producers**. JIT blocks multi-value (`jit:1070`, = sealed C5). Outcome producer = the
use-case that unlocks C5 (closing the loop: C5 was sealed due to lack of use-cases, Outcome brings the key).

## 3. ABI Decisions

### 3.1. MIR value model: 2-slot `{disc, payload}`
`T~E` value = **2 i64 slots**:
- `disc: i64` — Trit discriminator: `Positive(1)` = success (payload is T), `Negative(-1)` = failure
  (payload is E). `Zero(0)` is INVALID on T~E (E1025 compile-time, ADR-0020 §1.1).
- `payload: i64` — scalar union (Tier A: 1 i64 containing T or E depending on disc).
- `T?~E` (ternary): disc `Zero(0)` is valid = null state (payload ignored).

### 3.2. Constructor lowering
- `~+ value` → alloc 2-slot; `disc = const Positive(1)`; `payload = lower(value)`.
- `~- error` → `disc = const Negative(-1)`; `payload = lower(error)`.
- `~0` (T?~E only) → `disc = const Zero(0)`; payload undefined.

### 3.3. Return: `ReturnShape::BinaryOutcome` (arity 2)
Fn `-> T~E` → `ReturnShape::BinaryOutcome`, `Return { values: [disc_local, payload_local] }`.
JIT: Cranelift `sig.returns` = 2× `AbiParam::new(I64)`, callee `return_(&[disc, payload])`,
caller `inst_results[0]=disc, [1]=payload`.

### 3.4. Destructure: discriminant + unwrap (Abandoning specialized Statement ops)
`match o { ~+ x => .. ~- e => .. }`:
- Read disc: `Assign { dest, source: outcome.project(OutcomeDiscriminant) }` — `stack_load slot@0`.
- Branch on Trit: `If { cond: disc, positive_bb: success, negative_bb: error, zero_bb: None }`.
- Read payload: `Assign { dest, source: outcome.project(OutcomePayload) }` — `stack_load slot@8`.
**Unified projection-based architecture:** reuses the `Projection`/`Assign`/`StackSlot` infrastructure
identical to Struct/Sret. Specialized `Statement` ops `OutcomeDiscriminant`/`OutcomeUnwrap`/`OutcomeUnwrapError`
(legacy definitions `mir:254-280`) were deleted — they assumed Outcome was a single monolithic value
(prior to StackSlot refactor OP.3.5) and did not match the 2-slot representation. The projection-based
design unifies all offset read/write paths.

### 3.5. JIT un-defer C5 (Outcome ONLY)
Remove guard `jit:1070` `if values.len()>1 → Err` **ONLY when** `return_shape ∈ {BinaryOutcome, TernaryOutcome}`.
Generic >1 values (tuples) still Err (no language surface yet). Cranelift native multi-return — lightweight
premise (C5 spike phase11 proven).

## 4. Slicing (OP.1-4, each slice gate green)
- **OP.1 Typecheck:** verify + supplement `check_outcome_constructor_context` — return-type-match
  (`~+ v`:T, `~- e`:E matches `-> T~E`) + **E1025** (`~0` on T~E) + **E1024 exhaustiveness**
  (match T~E covers ~+/~-). Negative fixtures (check-mode).
- **OP.2 Lower:** constructor → 2-slot + `ReturnShape::BinaryOutcome` + `Return[disc, payload]`.
  **CHECK-MODE fixtures** (parse→typecheck→lower→borrowck→MIR verify) — proves producer correctness
  up to MIR, WITHOUT requiring JIT. Isolates producer from backend.
- **OP.3 JIT (un-defer C5-for-Outcome):** remove guard 1070 for Outcome, Cranelift 2-return,
  caller `inst_results[0,1]`. Fixtures RUN end-to-end (T~E returns values).
- **OP.4 Match/unwrap:** OutcomeDiscriminant + branch + Unwrap. Fixtures run match.

## 5. Expected Teeth (O)
- OP.2: poison disc const (Positive→Zero) → MIR verifier / typecheck catches (Zero invalid on T~E).
- OP.3: poison removing guard for generic tuples → tuple-return must STILL Err (un-defer only for Outcome).
- OP.3: poison caller inst_results[1] (omit payload) → fixture run produces wrong value.
- OP.4: poison OutcomeDiscriminant (read wrong slot) → match branches incorrectly.

## 6. Consequences
- **Positive:** Error-handling core comes alive (major language value); unlocks C5-for-Outcome
  (unsealing 1 Group E lock); foundation for C4 Packed Outcome (later optimization).
- **Deferred (Tier B/C):** Heap payload Outcome (String/Vector) — ownership/drop/borrow across
  multi-return. Generic tuple-return (full C5). C4 Packed (bit-packing disc+payload into 1 register)
  — optimization, after 2-slot is operational.
- **ABI Change:** Functions returning `-> T~E` return 2 registers (SysV) instead of 1 —
  FFI/callers must be aware. Explicitly documented here.
