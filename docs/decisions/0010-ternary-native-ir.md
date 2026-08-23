# ADR 0010 — Ternary-native IR: BrTrilean, Eq, NullCheck

**Status:** Decision. Applies to all new IR opcodes + lowerer + VM from the v0.3.x.ternary phase. Modifies `.triv` wire format (v2) — remains backward-compatible with the v1 reader as new opcodes are additive.

**Issue:** Following the v0.3.x cleanup, an audit of 8 cleanup commits revealed that—despite the SPEC + VISION commitment to Trilet being a first-class ternary type—most of the lowerer, IR, and VM **collapse Trilean to Boolean** at branch boundaries. Specifically:

1. **`BrIf` is a 2-way branch.** The condition is Trilean, but the VM uses `is_truthy()` (which only returns `true` for `Trilean::True`). Both `Unknown` and `False` follow the `else` branch. Since Ł3 possesses 3 values, the IR effectively discards 1/3 of the information.

2. **`if?` vs `if` collapse to the same `BrIf`.** SPEC §7.1.1 stipulates:
   - `if cond` requires a definitely-known condition; **panic on Unknown**.
   - `if? cond` treats `Unknown` as `False`.
   
   Currently, both use `BrIf(cond, then, else)`, meaning the differing semantics are hardcoded-collapsed at the lowerer instead of being expressed within the IR.

3. **`EnumTag` returns a Trit but only utilizes 2/3 of the states.** Code comment: `Positive for variant 0, Negative for variant >=1`. An enum with 3 variants (Red/Green/Blue) should dispatch a single instruction based on one trit; currently, it generates N-1 chained binary `BrIf` instructions.

4. **`Constant::Null` is a bolt-on.** In a ternary system, the discriminator `T?` is naturally a single trit:
   - `+1` = Some (definitely present)
   - `0` = Unknown (unverified — useful for async/lazy)
   - `-1` = None (definitely absent)
   
   Separating `Constant::Null` treats "null as a separate entity" from the ternary space.

5. **`Eq` on `Trilean::Unknown` returns `Trilean::False`** instead of `Trilean::Unknown`. Ł3 logic dictates that two unknown values cannot be asserted as equal or unequal $\rightarrow$ equality must result in `Unknown`.

VISION §5 lists three capabilities that make Trilet irreplaceable by "Rust + Mojo + Nix": *Trit-level capability, Ł3 checking, and ternary ABI primitives*. All three are currently undermined by this binary collapse.

This ADR locks the ternary-native design before the v0.4 ABI freeze—because after v0.4, every binary leak becomes a difficult-to-fix ABI commitment.

## Decision

### 1. `BrTrilean` replaces `BrIf` as the primary branch opcode

```
BrTrilean { cond, true_block, unknown_block, false_block }
```

- The condition is an SSA value with Trilean semantics.
- Runtime dispatch occurs directly based on the Trilean value:
  - `Trilean::True`  $\rightarrow$ `true_block`
  - `Trilean::Unknown` $\rightarrow$ `unknown_block`
  - `Trilean::False` $\rightarrow$ `false_block`

**Lowering:**
| Source construct | true_block | unknown_block | false_block |
|---|---|---|---|
| `if cond { … } else { … }` (plain) | then | **`unreachable_block`** (panic) | else |
| `if? cond { … } else { … }` | then | else | else |
| `while cond { … }` (plain) | body | **`unreachable_block`** | exit |
| `while? cond { … }` | body | exit | exit |
| Match arm test (Eq $\rightarrow$ Trilean) | arm_body | next_test | next_test |
| Pattern test (tuple/literal) | enter_body | next_test | next_test |

**`BrIf` is retained** for two specific cases where binary semantics are sufficient:
- Branching on a `Trit` that has been fully verified as 2-state (e.g., current `NullCheck`).
- Backward-compatible decoding of `.triv` v1 files.

The new lowerer (as of v0.3.x.ternary) must emit `BrTriint` for every branch where the condition is Trilean. `BrIf` is kept solely for compatibility.

### 2. `EnumTag` utilizes all 3 trit values

For an enum with N variants:

| N | Tag type | Encoding |
|---|---|---|
| 1 | Unit (no tag) | implicit |
| 2 | Trit (1 trit) | `-1, +1` (zero reserved for future async/lazy variants) |
| 3 | Trit (1 trit) | `-1, 0, +1` — *idiomatic ternary* |
| 4–9 | Tryte (9 trit) | offset from -4 |
| 10+ | Integer | full range |

Match dispatch for a 3-variant enum lowers to **a single `BrTrilean` instruction** on the tag, rather than chained `BrIf` instructions.

### 3. Nullable discriminator uses `Trit::Zero` as null

The discriminator for `T?` is a Trit:
- `+1` = Some(value)
- `0`  = null (canonical)
- `-1` = reserved (definitely-missing, distinct from null — for future "explicit absent" semantics)

**Implementation pragmatism** — the `Constant::Null` variant is retained in the enum for compact wire encoding (1 byte vs. 1 instruction + operand) and to allow `NullCheck` to pattern-match directly without inspecting the payload. However, its **semantics** are explicitly documented as the "Trit::Zero state of the nullable discriminator," rather than "null is a separate entity distinct from the trit space." This is the anchor of its ternary identity.

VM `NullCheck` returns a Trit:
- `RuntimeValue::Null` $\rightarrow$ `Trit::Zero` (matches discriminator)
- Some-wrapped value $\rightarrow$ `Trit::Positive`
- Future "definitely missing" $\rightarrow$ `Trit::Negative` (reserved, not currently emitted)

Branches use `BrTrilean` on the `NullCheck` result instead of `BrIf`.

The complete removal of `Constant::Null` (replacing it with a `Const(Trit::Zero) + NullWrap` pattern) is deferred—breaking the `.triv` wire format for purely aesthetic reasons without changing semantics. This will be revisited in v0.5 (CAS packaging) if hash stability requires consolidation.

### 4. `Eq` / `Ne` are Ł3-aware

When both operands are `Trilean::Unknown`:
- `Eq` returns `Trilean::Unknown` (unverifiable)
- `Ne` returns `Trilean::Unknown`

When one operand is `Trilean::Unknown` and the other is `True`/`False`:
- `Eq` returns `Trilean::Unknown` (cannot assert equality/inequality)
- `Ne` returns `Trilean::Unknown`

When both operands are definite:
- Equal $\rightarrow$ `Trilean::True`, otherwise `Trilean::False`.

For Trit operands: same — `Trit::Zero` $\leftrightarrow$ `Unknown` propagation.

For Integer/Long/Tryte/String operands (lacking an `Unknown` state): 2-valued semantics remain valid — always returns `True` or ` $\text{False}$`.

### 5. `.triv` wire format compatibility

- New Opcode IDs (`BrTrilean`) are appended to the end of the enum encoding — this does not break the v1 decoder.
- The `.triv` version field is bumped from 1 $\rightarrow$ 2 (per ADR-0008) when the format introduces new instructions.
- A v1 reader encountering `BrTrilean` will return `TrivError::UnknownOpcode` — it will not silently misinterpret the instruction.

### 6. Reserved Trit semantics at the IR level

Throughout the IR, a `Trit` must never be allowed to "mean boolean":
- `+1` = positive / yes / present / variant-positive
- `0` = zero / unknown / pending / canonical-null
- `-1` = negative / no / absent / variant-negative

Any code in the lowerer or VM that collapses one of these three states must include a comment explaining **why the binary collapse is correct** at that specific location (e.g., "tag has been verified as 2-state in a previous pass").

## Consequences

### For v0.4 (ABI)

- If a cross-package call result is Trilean $\rightarrow$ the witness table dispatch must be capable of encoding 3-state values.
- Capability checks (v0.6) are planned to use Ł3 `Unknown` to defer to runtime; `BrTrilean` becomes an **identity opcode** rather than just an implementation detail.

### For the backend (v0.9 JIT, v2.0 LLVM, v$\infty$ trytecode)

- **JIT (Cranelift)**: `BrTrilean` lowers to 2 comparisons + 2 branches (binary CPU). There is an encoding overhead, but it remains correct.
- **LLVM AOT**: Same — 2 comparisons + 2 branches.
- **Trytecode**: `BrTrilean` lowers to **a single instruction** — this is the point where Trilet permanently triumphs over ternary hardware.

### For the SPEC

- §7.1.1 is officially implemented (plain `if` panics on `Unknown`). Currently, this is only a TODO comment.
- §1.5.2 (Trilean three-valued logic) is consistent end-to-end — there are no longer any points of silent collapse.

### Pace

- Implementation: 1–2 days (mostly mechanical lowerer migration; a test corpus of 11/11 is already available as a regression net).
- Does not break existing tests if the lowering maintains correct semantics (e.g., `Unknown` $\rightarrow$ `False` for `if?`/`match` defaults).

## Alternatives Considered

- **Complete removal of `BrIf`**: Deferred — it is still required for backward `.triv` decoding and for cases where the binary state is truly verified (Trit verified as 2-state). A subsequent optional phase could audit and remove it.
- **Encoding 4+ variant enums as Trytes**: Deferred — there are no immediate use cases; this ADR only defines the mapping, and the lowerer currently only implements 2–3 variants.
- **Capability Trilean dispatch (v0.6 pillar #5)**: Deferred — this will be built upon the `BrTrilean` infrastructure.
- **Trytecode backend on ternary hardware**: v$\infty$.

## Prior Art

- **CMU CCured / Refinement types**: 3-state qualifier propagation (safe/uncheckable/wild). Shares the philosophy of "do not collapse semantics at the IR level."
- **Setun (Brusentsov 1958)**: Native 3-way branch hardware — `JZ negative, zero, positive` instruction. This is the direction Trilet follows.
- **LLVM `select` vs `br`**: LLVM separates `select` (data) from `br` (control). In Trilet, `BrTrilean` is a `br` with three successors instead of two.
- **Anti-pattern**: JVM `IFEQ`/`IFNE` only checks zero/non-zero — this has been entrenched in binary thinking since 1995 and cannot be changed without breaking the ABI.

## References

- [SPEC §1.5.2 — Trilean](../../SPEC.md)
- [SPEC §7.1.1 — if/if? semantics](../../SPEC.md)
- [VISION §5 — Trilet Identity](../../VISION.md)
- [ADR-0007 — IR design](0007-ir-design.md) (this ADR refines)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (this ADR bumps version)
- [ADR-0009 — Version gate policy](0009-version-gate-policy.md) (this ADR is filed under the v0.3.x.ternary phase)

---

## Addendum — v0.7.4.3-error (null literal unification)

Per [ADR-0020 §10](0020-outcome-error-handling.md) (2026-05-17), the source-level syntax for the `Trit::Zero` discriminator state is unified across the language: `~0` becomes canonical, and `null` is deprecated as a synonym until removal in v1.0.

**No change to IR or wire format.** The `Constant::Null` IR opcode locked in this ADR continues to encode "the canonical `Trit::Zero` state of a nullable discriminator." The only change is the **source-level naming** that the lowerer accepts:

| Source syntax | Lowerer behavior | IR emission |
|---|---|---|
| `null` | Emit W2001 `NullDeprecated` warning, then lower normally | `Constant::Null` (unchanged) |
| `~0` | Lower normally (no warning) | `Constant::Null` (unchanged) |

Both source forms produce **byte-identical** `.triv` output — the wire-format `Constant::Null` encoding (1 byte, `0x00` 0-byte payload per ADR-0008 §"Constant pool") is the canonical `Trit::Zero` on-disk representation. No version bump.

**For `T?~E` outcome types** (introduced in ADR-0020 §1), the same `Constant::Null` IR opcode encodes the null arm — the `Trit::Zero` discriminator state is universal across nullable types and ternary outcome types alike. The `OUTCOME_NEW_NULL` opcode (ADrad-0020 §7.3, opcode `0xC3`) is the dynamic constructor equivalent; `Constant::Null` is the compile-time-constant form.

**No backend change required.** Backends already handle `Constant::Null` (VM: shipped in v0.3; JIT v0.9 / AOT v2.0 / Trytecode v$\infty$: contract pre-existing). The source-level unification is a parser-and-typecheck-only change.

---

## Addendum §C — v0.7.4.3-error.3c (BrTrilean unknown_block demoted to defense-in-depth)

Per [ADR-0021](0021-trilean-refinement.md) (2026-05-18), the safety contract for plain `if cond` shifts from **runtime panic via `BrTrilean` unknown_block** (this ADR §1) to **compile-time error via E1033 `PossiblyUnknownCondition`** (ADR-0021 §3).

**No change to IR, VM, or wire format.** The `BrTrilean { unknown_block }` opcode locked in this ADR continues to exist with identical runtime semantics. The change is purely in the **threat model**:

| Era | Primary safety mechanism for plain `if` on possibly-Unknown |
|---|---|
| Pre-ADR-0021 (v0.7 $\le$ .3b) | Runtime panic — VM dispatches Unknown discriminator to `unknown_block`, which the lowerer emits as Panic |
| Post-ADR-0021 (v0.7.4.3-error.3d+) | Compile-time error — typecheck rejects the program before IR is generated |

The runtime path remains **defense-in-depth** for three legitimate cases:

1. **`if? cond`** — the relaxed form continues to dispatch all three Trilean states correctly via `BrTrilean`. The `unknown_block` for `if?` is the *else* branch, not a panic.
2. **`match`** — a three-arm match on Trilean lowers through `BrTrilean`; all arms remain reachable.
3. **`.triv` consumers that skip typecheck** — backends loading IR from untrusted sources (cross-package CAS imports without manifest verification, or hypothetical future JIT-on-untrusted-bytecode) cannot rely on typecheck having run. The runtime panic stays as a paranoia net.

The **Author 2026-05-18 directive** ("handle immediately" / no warning period) means v0.7.4.3-error.3d ships with compile-time rejection immediately. Programs that relied on the runtime panic as their primary safety mechanism must migrate per ADR-0021 §3 remediations.

**No backend change required.** The `BrTrilean` opcode, its three-successor encoding, and the lowerer's emission strategy for `if` / `if?` / `match` are unchanged.

---

## Addendum §D — v0.7.4.3-error.6a (outcome-null runtime unification)

Closes the runtime-level half of [Addendum §B](#addendum--v074.3-error-null-literal-unification) (null/`~0` source unification, 2026-05-17). Addendum §B promised:

> "Both source forms produce **byte-identical** `.triv` output — the wire-format `Constant::Null` encoding (1 byte, `0x00` 0-byte payload per ADR-0008 §"Constant pool") is the canonical `Trit::Zero` on-disk representation."

The `.3a`/`.3b` implementation broke this promise: source `~0` lowered to the new `OutcomeNewNull` opcode (`0xC3`) producing `RuntimeValue::Outcome { Trit::Zero, None }`, while source `null` lowered to `Constant::Null` producing `RuntimeValue::Null`. This resulted in two different runtime shapes for one canonical state.

A concrete consequence (surfaced during `v0.7.4.3-error.4b` corpus migration): `examples/nullable.tri` uses `~0` inside a `String?` Elvis `?:` fallback. After migrating `null $\rightarrow$ ~0`, the VM-tier Elvis (built on `NullCheck` over `RuntimeValue::Null`) no longer recognized the value as null — it saw `RuntimeValue::Outcome` instead, and the fallback never fired. The Interpreter (which has no `Outcome` value at all) likewise rejected the migrated form.

### Decision

**Lock:** Three changes, all backward-compatible at the wire-format level.

1. **Lowerer.** `Expr::OutcomeConstructor { arm: Zero, payload: None }` now emits `Constant::Null` instead of `Instruction::OutcomeNewNull`. Source `~0` and source `null` (deprecated W2001) produce byte-identical IR — finally honoring the promise in §B. The `OutcomeNewNull` opcode (`0xC3`) is retained for backward `.trit` compatibility and as the dynamic-constructor path for tools that build IR without source (no version bump).

2. **VM cross-tolerance.** The `Trit::Zero` state has a single canonical runtime representation (`RuntimeValue::Null`), but the IR carries two runtime shapes (`RuntimeValue::Null` and `RuntimeValue::Outcome { Trit::Zero, None }`) for legacy reasons. Four opcodes accept both shapes interchangeably:

| Opcode | Pre-§D | Post-§D |
|---|---|---|
| `OutcomeDiscriminant` on `RuntimeValue::Null` | E2201 TypeMismatch | return `Trit::Zero` |
| `NullCheck` on `RuntimeValue::Outcome { Zero, None }` | E2201 TypeMismatch | return `Trit::Zero` |
| `OutcomeUnwrapValue` on `RuntimeValue::Null` | E2201 TypeMismatch | E2210 InvalidOutcomeState (clean message: "unwrap_value on null state") |
| `OutcomeUnwrapError` on `RuntimeValue::Null` | E2201 TypeMismatch | E2210 InvalidOutcomeState ("unwrap_error on null state") |

The asymmetry (panic E2210 instead of E2201) for unwrap-on-null reflects the semantic: the value IS in a valid `Trit::Zero` state, just not the arm being unwrapped — exactly like calling `.unwrap_value()` on a failure outcome.

3. **Interpreter parity.** `Expr::OutcomeConstructor { arm: Zero, payload: None }` evaluates to `Value::Null` directly (matches lowerer + VM). The Interpreter does not carry a separate `Value::Outcome` enum variant, so this is automatic — only the rejection arm needed updating.

### Tests

Round-trip tests (`crates/triet-ir/src/vm.rs#[cfg(test)] mod tests`) cover each of the four cross-tolerant cases. The existing `.3a` test `vm_outcome_discriminant_returns_trit_per_arm` continues to verify the `OutcomeNewNull $\rightarrow$ OutcomeDiscriminant $\rightarrow$ Trit::Zero` path (unchanged — opcode still emits `RuntimeValue::Outcome`). The `.3b` e2e test `outcome_null_constructor_on_ternary_outcome` exercises the new path (`~0` source $\rightarrow$ `Constant::Null` IR $\rightarrow$ `RuntimeValue::Null` runtime $\rightarrow$ cross-tolerant `OutcomeDiscriminant` returns Zero $\rightarrow$ match arm `~0` fires).

`examples/nullable.tri` is migrated back to the `~0` form in `.6b`. A differential test (interpreter vs VM) re-greens the build, closing the `.4b` deferred item.

### Alternatives Considered

- **Drop `OutcomeNewNull` opcode.** Rejected — backward `.triv` compatibility and future dynamic-construction paths (JIT, tool emitters) require the opcode to remain alive even though the lowerer no longer emits it from source.
- **Force `RuntimeValue::Outcome { Zero, None }` $\rightarrow$ `RuntimeValue::Null` at the VM level.** Rejected — this would require the VM to inspect every `Outcome` value at construction time. Cross-tolerance on the consuming opcodes is simpler and equally correct.
- **Add an `is_null()` helper as a method on `RuntimeValue`.** The cross-tolerance logic lives within the opcode dispatch sites — this minimizes the number of places that must be kept in sync.
