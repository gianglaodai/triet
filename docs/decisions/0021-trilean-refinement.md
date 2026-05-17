# ADR 0021 — Compile-time `Trilean!` refinement for strict `if`

**Trạng thái:** Quyết định. Áp dụng cho v0.7.4.3-error.3c onward. Closes long-standing TODO in [`crates/triet-typecheck/src/check.rs:397-412`](../../crates/triet-typecheck/src/check.rs) (comment: *"A future pass could refine this"*). Aligns implementation với [SPEC §7.1.1](../../SPEC.md) (which has always specified compile-time error for plain-`if` on possibly-Unknown conditions). Refines [ADR-0010 §1](0010-ternary-native-ir.md) (which made the runtime panic the primary safety mechanism — now demoted to defense-in-depth).

**Issue:** Triết v0.7.4.3-error.3b ships với three-layer error story:

| Layer | Mechanism | Triggered by |
|---|---|---|
| Recoverable (data) | `T?` / `T~E` / `T?~E` / `Trilean::Unknown` | Expected absence / failure / uncertainty |
| Bug-tier (panic) | `VmError` E22XX | Unsoundness — division-by-zero, force-unwrap-null, unwrap-wrong-arm |
| **Plain `if` on possibly-Unknown** | **Runtime panic via `BrTrilean` unknown_block** | **`if cond` when cond *might* be Trilean::Unknown** |

The third layer is the outlier. SPEC §7.1.1 has always called it a "compile error". ADR-0010 §1 documented runtime-panic as the implementation strategy because the type checker "can't tell statically whether a Trilean is always known". The typecheck source-of-truth file confirms the gap:

```rust
// crates/triet-typecheck/src/check.rs:397-412
fn check_condition_type(&mut self, cond_ty: Type, ...) {
    match cond_ty {
        Type::Trilean | Type::Unknown => {
            // Plain `if` requires a definite Trilean. The checker
            // can't tell statically whether a Trilean is "always
            // known", so we accept any Trilean here and rely on
            // `if?` for explicit unknown handling. A future pass
            // could refine this.
        }
        ...
    }
}
```

Author 2026-05-18 (during v0.7.4.3-error error-handling work): *"Hiện tại chúng ta đang để panic ở runtime. Điều này không tốt, hãy để lỗi ở compile time. `if` chỉ nhận boolean mà thôi."* The Outcome work (ADR-0020) closed the recoverable-error gap; this ADR closes the strict-`if` gap symmetrically.

Two consistency bugs surface during this audit:

1. **SPEC §7.1.1 vs ADR-0010 §4 conflict.** SPEC says `if cond == true` "chỉ true mới chạy, unknown đối xử false". But ADR-0010 §4 explicitly states `Unknown == true → Unknown`, which means `if (cond == true)` with `cond = Unknown` still panics under ADR-0010. SPEC §7.1.1 is incorrect on this point. Fixed in §6 below.

2. **Comparison ops type-erase refinement.** `Integer == Integer` and `Integer < Integer` *cannot* produce Unknown (Integer has no Unknown state), yet currently return generic `Type::Trilean` indistinguishable from `Trilean::Unknown`-bearing comparisons. The checker has no way to prove `if (a == b)` is safe.

This ADR fixes both via a **refinement type** layered on top of the existing `Type::Trilean` — no new primitive type, no new wire format, no new VM opcodes. The refinement lives entirely in the typecheck crate.

## §1 — `Trilean!` refinement type

**Lock:** A new typecheck-only refinement marker `Trilean!` distinguishes Trilean values **statically proven** non-Unknown from generic Trilean (which might be Unknown).

```text
Type lattice (refinement direction):

        Trilean
           ▲
           │ widening (implicit, always allowed)
           │
        Trilean!
           │
           │ narrowing (explicit only — .assume_known() / pattern match / etc.)
           ▼
        Trilean (with proof obligation)
```

- **`Trilean!`** is a subtype of `Trilean`. Any value of type `Trilean!` can be passed wherever `Trilean` is expected — the widening is implicit and always sound.
- **`Trilean` → `Trilean!`** narrowing is never implicit. The author writes `.assume_known()` (runtime check, panic if Unknown), uses pattern matching (`match cond { true => ..., false => ..., unknown => ... }`), or uses `if?` (treats Unknown as one of the three arms).

Surface syntax: the `!` suffix is the **only** distinguishing marker. It appears in error messages, type annotations, and diagnostic output. Authors rarely write `Trilean!` directly — it arises through inference (see §2). Display form per SPEC convention follows `!`-as-strict marker (cf. `expect!` in Rust prelude, `!`-banged methods).

`Trilean!` is **purely type-level**:

- No new AST node — `Type::Trilean` gains a single `bool` field (per Q6-A).
- No new runtime value — `RuntimeValue::Trilean` continues to carry the full Ł3 lattice including Unknown.
- No new IR opcode — branch and comparison opcodes are unchanged.
- No new `.triv` wire format version bump — refinement is erased at IR lowering.

The runtime tier remains free to carry an Unknown value in a register typed `Trilean!`, but **typecheck guarantees no source-level path can construct such a state** without an explicit narrowing call. If the runtime ever sees Unknown in a `Trilean!`-typed slot, that is a typecheck bug, not a user error.

## §2 — Operator and literal refinement rules

The refinement is *propagated* through operations, not declared. The typecheck rules are:

### 2.1 — Literal types

| Source literal | Inferred type |
|---|---|
| `true` | `Trilean!` |
| `false` | `Trilean!` |
| `unknown` | `Trilean` |

Rationale: `true` / `false` literals are statically proven non-Unknown at the source level. The `unknown` literal is the canonical Trilean::Unknown.

### 2.2 — Equality and ordering

Comparisons follow ADR-0010 §4 ("Ł3-aware Eq/Ne") but track refinement:

| LHS type | RHS type | `==` / `!=` result | `<` / `<=` / `>` / `>=` result |
|---|---|---|---|
| Integer | Integer | `Trilean!` | `Trilean!` |
| Tryte | Tryte | `Trilean!` | `Trilean!` |
| Long | Long | `Trilean!` | `Trilean!` |
| Trit | Trit | `Trilean` (Trit::Zero ↔ Unknown propagation per ADR-0010 §3) | `Trilean!` (Trit ordering is total) |
| String | String | `Trilean!` | `Trilean!` (lexicographic, total) |
| `Trilean!` | `Trilean!` | `Trilean!` (both ≠ Unknown ⇒ result ≠ Unknown) | N/A (Trilean has no ordering) |
| `Trilean` | `Trilean!` or `Trilean` | `Trilean` (might be Unknown) | N/A |
| `Trilean!` | `Trilean` | `Trilean` (one side might be Unknown) | N/A |
| `T?` | `T?` / `T` / `null` | `Trilean` (null propagates Unknown per ADR-0001 + ADR-0010 §3) | (same) |
| `T~E` / `T?~E` | (same) | `Trilean` (outcome state introduces uncertainty) | N/A |

The key rule: **non-nullable, non-Trilean primitives compare to `Trilean!`** because their type has no Unknown state. Any operand whose static type is `Trilean` (without `!`) or `T?` or outcome introduces possibly-Unknown into the comparison.

### 2.3 — Logical operators

Łukasiewicz (`&&`, `||`, `^`, `=>`, `<=>`) and Kleene (`~>`, `~^`, `<~>`) operators preserve refinement when both operands are refined:

| LHS type | RHS type | Result |
|---|---|---|
| `Trilean!` | `Trilean!` | `Trilean!` |
| `Trilean` | (anything Trilean-like) | `Trilean` |
| (anything Trilean-like) | `Trilean` | `Trilean` |

Rationale: in Łukasiewicz Ł3, the truth table for `True ∧ True`, `True ∧ False`, `False ∧ False` never produces Unknown — Unknown is only produced when at least one operand is Unknown. Same for `∨`, `→`, `↔`. The refinement is closed under these operations.

The `!` unary-not operator: `!Trilean!` → `Trilean!`, `!Trilean` → `Trilean`. (Negation cannot introduce Unknown.)

### 2.4 — `assume_known()` method

`Trilean.assume_known()` returns `Trilean!`, with runtime semantics: panic (E2209 `AssumeKnownOnUnknown`, new in ADR-0021 — slot reserved in E22XX namespace) if the value is Unknown at runtime. Same shape as `T?.unwrap_value(message)` / `Outcome.unwrap_value(message)` from ADR-0020: explicit verbose method, panic-possible-but-source-visible.

Per `feedback_explicit_strictness.md`, the method **must take a message argument**:

```triet
trilean_val.assume_known("expected known after validation step")
```

This makes the narrowing intent reviewable at the call site, mirroring `.unwrap_value("msg")` / `.unwrap_error("msg")`.

### 2.5 — `NullCheck` result is `Trilean!`

The `NullCheck` IR opcode (ADR-0010 §3) returns a Trit-encoded discriminator: Positive = non-null, Zero = null. When the lowerer materializes this into a Trilean-typed register (for `if`/branch purposes), the static type is `Trilean!` — the check definitively answers Positive or Zero, never Unknown.

Practical effect: `if x.is_null() { … }` (sugar shortly to add) typechecks under the refinement system.

### 2.6 — Match arm narrowing

Inside a `match` arm bound to a specific Trilean variant, the bound variable narrows to `Trilean!`:

```triet
match cond {
    true => { /* cond bound here is Trilean! True */ },
    false => { /* cond bound here is Trilean! False */ },
    unknown => { /* cond bound here is Trilean (carries Unknown) */ }
}
```

This is automatic flow-sensitive narrowing — same mechanism that lets `if (cond == true)` work *if* and only if `cond` is already `Trilean!`. The third arm (`unknown`) does NOT widen back to `Trilean!` because the value at that point IS Unknown.

### 2.7 — Function return types

A function declared `-> Trilean!` must return only `Trilean!` values. Returning `Trilean` raises E1034 `TrileanReturnNotRefined`. Authors who want explicit-refined return semantics annotate at the declaration:

```triet
function definitely_positive(x: Integer) -> Trilean! = x > 0   // OK: Integer > Integer ⇒ Trilean!
function maybe_unknown(x: Trilean) -> Trilean! = x              // E1034: x is Trilean, not Trilean!
```

A function declared `-> Trilean` (without `!`) accepts both — implicit widening from `Trilean!` to `Trilean` is always allowed.

## §3 — Strict `if` accepts only `Trilean!`

**Lock:** Plain `if cond { … } else { … }` requires `cond: Trilean!`. A `Trilean` (without `!`) in this position raises **E1033 `PossiblyUnknownCondition`** at compile time.

```triet
function classify(n: Integer) -> String = {
    if n > 0 { "positive" } else { "non-positive" }   // OK: Trilean!
}

function risky(t: Trilean) -> String = {
    if t { "yes" } else { "no" }                       // E1033 — `t: Trilean`, might be Unknown
}
```

The diagnostic E1033 lists the four remediations from SPEC §7.1.1:

```text
error[E1033]: condition might be Trilean::Unknown — plain `if` requires Trilean!
  ┌─ risky.tri:2:8
  │
2 │     if t { "yes" } else { "no" }
  │        ^ this is `Trilean` (might be Unknown)
  │
  = note: plain `if` panics on Unknown per ADR-0010 — Triết forbids that path
          statically. Choose one of:

          1) Use `if?` to treat Unknown as false:
                if? t { "yes" } else { "no" }

          2) Use `match` for explicit three-arm dispatch:
                match t {
                    true => "yes",
                    false => "no",
                    unknown => "?",
                }

          3) Narrow with `.assume_known("reason")` (panics at runtime if Unknown):
                if t.assume_known("validated upstream") { "yes" } else { "no" }

          4) Compare against `true` — but only works if both sides are Trilean!.
             For `Trilean!` x: `if x == true` works; for `Trilean` x: still E1033.

          See SPEC §7.1.1 and ADR-0021 for the full design.
```

### 3.1 — `if?` accepts both

The relaxed `if? cond` form accepts both `Trilean!` and `Trilean`. The `?` is the author signaling "I have handled the Unknown case (by treating it as false)". This was always the design intent of `if?` — ADR-0021 makes it explicit by giving `if?` a wider input type than `if`.

### 3.2 — While loops

`while cond` and `loop { … }`-with-internal-`if-break` follow the same rule: `cond: Trilean!` required. `while? cond` accepts both, with Unknown treated as false (loop exit).

Same E1033 message, same remediations.

### 3.3 — Match guards

`match X { pattern if guard => body, ... }` — the guard expression must be `Trilean!`. Same E1033 if Trilean. Same remediations.

## §4 — Type system implementation

**Lock:** Single-variant approach (Q6-A).

```rust
// crates/triet-typecheck/src/types.rs
pub enum Type {
    Integer,
    Tryte,
    Long,
    Trit,
    Trilean { refined: bool },   // refined: true ⇒ Trilean!
    // ...
}
```

`Type::Trilean { refined: true }` is `Trilean!`. `Type::Trilean { refined: false }` is plain `Trilean`. Display: `Trilean!` when refined, `Trilean` otherwise. Equality: `matches()` accepts `Trilean!` where `Trilean` is expected (widening) but not the reverse.

Constructor helpers in `types.rs`:

```rust
impl Type {
    pub const TRILEAN: Self = Self::Trilean { refined: false };
    pub const TRILEAN_KNOWN: Self = Self::Trilean { refined: true };
}
```

Existing call sites that wrote `Type::Trilean` (unit variant) become `Type::TRILEAN` (const). Mechanical rename — one commit's worth of churn.

### 4.1 — `matches()` widening

```rust
// crates/triet-typecheck/src/types.rs
fn matches(&self, other: &Self) -> bool {
    match (self, other) {
        // Trilean! widens to Trilean (when other expects Trilean, self can be Trilean!)
        (Self::Trilean { refined: _ }, Self::Trilean { refined: false }) => true,
        // Trilean! to Trilean! requires self to be refined
        (Self::Trilean { refined: true }, Self::Trilean { refined: true }) => true,
        // Plain Trilean cannot satisfy Trilean!
        (Self::Trilean { refined: false }, Self::Trilean { refined: true }) => false,
        // ... other cases unchanged ...
    }
}
```

### 4.2 — Operator dispatch

`crates/triet-typecheck/src/check/exprs.rs` operator dispatch consults a small table per §2.2 / §2.3 to compute the refinement of the result type from operand refinements. Pure data — no flow analysis.

### 4.3 — `check_condition_type` (the actual fix)

```rust
// crates/triet-typecheck/src/check.rs
fn check_condition_type(&mut self, cond_ty: Type, treat_unknown_as_false: bool, span: Span) {
    match cond_ty {
        Type::Trilean { refined: true } => { /* OK */ },
        Type::Trilean { refined: false } | Type::Unknown if !treat_unknown_as_false => {
            self.errors.push(TypeError::PossiblyUnknownCondition { span });
        },
        Type::Trilean { refined: false } | Type::Unknown => { /* if? — OK */ },
        other => self.errors.push(TypeError::NonTrileanCondition { found: other, span }),
    }
}
```

`treat_unknown_as_false` is the existing flag the parser sets for `if?` / `while?` / `match`-arm-guard contexts.

### 4.4 — Error code

E1033 — `PossiblyUnknownCondition` — joins the v0.7.4.3-error.2 batch (E1024 – E1032 from ADR-0020). E1034 — `TrileanReturnNotRefined` — for §2.7. Both follow the existing miette diagnostic + `#[label]` shape.

## §5 — No runtime changes

**Lock:** Zero impact on IR, VM, `.triv` wire format, or any backend.

- `BrTrilean` unknown_block continues to exist. Post-3d, it becomes **defense-in-depth** rather than primary safety: every reachable plain-`if` site has typecheck-proof its cond is `Trilean!`, so `unknown_block` should never fire in well-typed code. The opcode is retained because (a) `if?` paths still legitimately route Unknown through `unknown_block` to the else branch, and (b) the VM is allowed to be paranoid about IR it didn't produce itself (e.g., loaded `.triv` files where typecheck was skipped).
- `Eq` / `Lt` / `Gt` etc. opcodes unchanged — refinement is erased at lowering.
- `Constant::Trilean` unchanged — refinement is not encoded in constants.
- `.triv` wire format stays at v5 (no version bump for type-level-only changes — per ADR-0010 §"wire format compatibility" precedent).

ADR-0010 §1 originally documented `BrTrilean { unknown_block }` as the **primary** safety mechanism for strict `if`. ADR-0021 demotes it to defense-in-depth without breaking the contract — see [ADR-0010 Addendum §C](0010-ternary-native-ir.md) (added 2026-05-18 as part of this work).

## §6 — SPEC §7.1.1 fix

**Lock:** SPEC §7.1.1 line 706 currently says:

> ```triet
> if cond == true { ... }         // chỉ true mới chạy, unknown đối xử false
> ```

This is **incorrect** per ADR-0010 §4 ("Trilean::Unknown == true ⇒ Trilean::Unknown"). With ADR-0021, the line is corrected:

| `cond` static type | `cond == true` | `if (cond == true)` behavior |
|---|---|---|
| `Trilean!` | `Trilean!` (result of `Trilean! == Trilean!`) | OK — plain `if` accepts `Trilean!` |
| `Trilean` (without `!`) | `Trilean` (Unknown propagates) | E1033 at typecheck — plain `if` rejects `Trilean` |

SPEC §7.1.1 is updated alongside this ADR (the `.3c` commit) to:

1. Remove the "chỉ true mới chạy, unknown đối xử false" line for `if cond == true`.
2. Add a note: "If `cond: Trilean!` then `cond == true` is `Trilean!` and safe in plain `if`. If `cond: Trilean` then `cond == true` is `Trilean` and `if` rejects it — use `if?`, `match`, or `.assume_known()`."
3. Cross-reference ADR-0021 from §7.1.1.

## §7 — Migration

**Lock:** No deprecation warning period. v0.7.4.3-error.3d ships E1033 as a hard compile error.

Author 2026-05-18 directive: "xử lý ngay" — no warning-period. Rationale: SPEC §7.1.1 has always specified compile-time error since pre-v0.2. Programs relying on the runtime-panic fallback were depending on undocumented behavior. The migration is mechanical — every offending site has at least one of the four §3 remediations applicable.

Corpus audit (2026-05-18, pre-`.3d`):

| File | Lines | Pattern | Suggested migration |
|---|---|---|---|
| `demos/02-module-system/utils.tri:15,23` | `if (a == unknown)` | `a` is Trilean | `match a { unknown => …, _ => … }` |
| `demos/02-module-system/utils/print.tri:10,20` | `if (t == true)` | `t` is Trilean | `match t { true => …, _ => … }` |
| `demos/02-module-system/alu.tri:25,34,49,68,69,73` | `if (a == false)` etc. | `a` is Trilean (Ł3 gate inputs) | `match` or `if?` per case |
| `demos/02-module-system/memory.tri:34` | `if (enable == true)` | `enable` is Trilean | `match` or `if?` |
| `examples/*.tri` | none flagged | — | — |
| `std/*.tri` | none flagged (stubs only) | — | — |

12 sites total across 5 files in `demos/`. Examples + stdlib are clean. Migration is the `.3e` sub-task — pure source-level edits, no logic change.

## §8 — Coexistence with existing features

- **`assume_known()`** — already exists per `check/methods.rs` (Trilean → Trilean). ADR-0021 changes the *type* of the result to `Trilean!`. The runtime semantics (panic if Unknown) are unchanged. Method signature gains a `message: String` argument per `feedback_explicit_strictness` — same as `unwrap_value(message)` / `unwrap_error(message)` from ADR-0020. Old callers must add a message argument; this is its own small migration in `.3e`.
- **`NullCheck`** — IR opcode result already returns Trit-encoded. The typecheck wrapper exposing it via syntax (e.g. `x.is_null()` sugar) types as `Trilean!`. (No such sugar exists yet — when it ships, it lands with `Trilean!`.)
- **`Trilean::True` / `False` / `Unknown` constructors** — Triết doesn't expose these as user-callable constructors. Literals `true` / `false` / `unknown` are the only source-level way to materialize Trilean values, and §2.1 already specifies their types.
- **Pattern match exhaustiveness** — match on `Trilean!` need only cover `true` and `false`; the `unknown` arm is statically unreachable. The exhaustiveness checker is updated to allow 2-arm `match` on `Trilean!` (E1015 `NonExhaustiveMatch` does not fire). Same shape as exhaustiveness on `Outcome` with `allow_null_state: false` (per ADR-0020 §5).

## §9 — Memory / serialization

No memory or serialization impact. Refinement is type-level only. The `Trilean { refined: bool }` field adds 1 bit (rounded to 1 byte) per Type instance in the typecheck crate's in-memory representation — negligible.

`.triv` wire format unchanged — refinement is erased before lowering. A `.triv` file produced by a Triết v0.7.4.3-error.3d compiler is byte-identical to one produced by v0.7.4.3-error.3b for any program that didn't use the strict-`if`-on-`Trilean` path.

## §10 — Test plan

`.3d` adds the following test coverage (in `crates/triet-typecheck/src/check_resolved.rs#[cfg(test)] mod tests`):

1. `if (a > b)` on Integer operands typechecks (no E1033).
2. `if (trilean_var)` raises E1033 with the four remediations in the diagnostic message.
3. `if (trilean_var == true)` raises E1033 (because `Trilean == Trilean!` widens RHS, result is `Trilean`).
4. `if (Trilean! && Trilean!)` typechecks (refinement preserved through `&&`).
5. `if (Trilean! && Trilean)` raises E1033 (one Trilean operand poisons the result).
6. `if? (trilean_var)` typechecks (relaxed form).
7. `match cond { true => …, false => …, unknown => … }` on Trilean typechecks (exhaustive 3-arm).
8. `match cond { true => …, false => … }` on `Trilean!` typechecks (exhaustive 2-arm — `unknown` unreachable).
9. `match cond { true => …, false => … }` on Trilean raises E1015 NonExhaustiveMatch.
10. `function f(x: Trilean) -> Trilean! = x` raises E1034.
11. `function f(x: Integer) -> Trilean! = x > 0` typechecks.
12. `t.assume_known("msg")` returns `Trilean!`.
13. `t.assume_known()` (no message) raises E1024 from ADR-0020 family (message required) — or new variant if needed.
14. `if (nullable_a == nullable_b)` raises E1033.
15. While-loop strict form: `while (t)` raises E1033 when `t: Trilean`.

End-to-end in `crates/triet-cli/tests/`:

16. Source program with `if (n > 0)` runs cleanly through parse → typecheck → lower → VM.
17. Source program with `if (trilean_var)` fails at typecheck with E1033 (test inspects the diagnostic).
18. Migrated `demos/02-module-system/alu.tri` typechecks cleanly after `.3e`.

## §11 — Không làm

- **Not introduce a separate `Bool` type** (rejected approach). Considered: a 2-state type distinct from Trilean. Rejected because it dilutes the "tam phân first-class" identity (VISION §5) — Triết is built around Trilean as the canonical truth type. A `Bool` would imply that 2-valued is the "default" and 3-valued is "extended", reversing the philosophy.
- **Not flow-sensitive refinement (occurrence typing)**. Rejected scope creep: tracking "after this `if (x == true) { … }` block, x is narrowed to Trilean! True" requires Hindley–Milner-with-refinements machinery far beyond the v0.2 type checker. Authors can use `match` or `.assume_known()` for the rare narrowing case.
- **Not effect tracking for unknown-introduction**. Rejected: would require annotating every function as "might-introduce-unknown" or not. Refinement on types is sufficient — Trilean values traveling through generic-Trilean-typed signatures stay generic-Trilean.
- **Not retroactively make `BrTrilean { unknown_block }` an error at IR-construction time**. Backends that load `.triv` files from less-strict producers must still handle Unknown — the runtime safety net is retained per §5.
- **Not warning-period**. Per author 2026-05-18 directive: "xử lý ngay". 12-site migration corpus is small enough; rip-the-band-aid approach matches "stability over speed" interpretation (one painful migration is better than two years of `if?`-vs-`if`-equivalent confusion).
- **Not enforce `Trilean!` in function parameter positions automatically.** Function parameters declared `Trilean` accept `Trilean!` (widening). Function parameters declared `Trilean!` accept only `Trilean!`. Authors choose the right declaration; no automatic narrowing.

## §12 — Prior art

- **Rust `bool` vs `Option<bool>`**: bare `bool` is 2-state, `Option<bool>` is 3-state (None for unknown). Distinction is via wrapper type. Triết uses refinement on the same Trilean type — closer to subtype than wrapper, lighter weight.
- **Refinement types (Liquid Haskell, F\*)**: arbitrary predicate refinement. Triết uses a single-bit refinement which is much weaker but doesn't require SMT solving — fits the v0.2 type-checker.
- **Kotlin null-safety (`String` vs `String?`)**: 2-state nullability. Triết already has `T?` for this; ADR-0021 applies the same compile-time-refusal pattern to Trilean::Unknown.
- **Java Optional / Stream API**: nominal types, no refinement. Anti-pattern reference — Triết wants the typechecker to do the work, not the author writing `.orElseThrow()` at every use site.
- **CMU CCured (referenced in ADR-0010)**: 3-state qualifier propagation (safe/uncheckable/wild). ADR-0021 follows the same "qualifier on a base type" mental model.
- **Setun JZ instruction**: the hardware 3-way branch motivated ADR-0010's BrTrilean. ADR-0021 layers compile-time discipline on top; the hardware path stays open for v∞ trytecode.

## §13 — Tham chiếu

- [SPEC §7.1.1 — if/if? semantics + unknown handling](../../SPEC.md) (updated alongside this ADR)
- [VISION §5 — Bản sắc Triết: ternary first-class](../../VISION.md)
- [ADR-0001 — Nullable memory layout](0001-nullable-memory-layout.md) (refinement applies symmetrically to NullCheck per §2.5)
- [ADR-0010 — Ternary-native IR](0010-ternary-native-ir.md) (this ADR refines §1 + §4; adds Addendum §C)
- [ADR-0017 — Trilean policy hook](0017-trilean-policy-hook.md) (capability resolver continues to return generic `Trilean` — `Trilean!` is at compile time only)
- [ADR-0020 — Outcome error handling](0020-outcome-error-handling.md) (sibling — closes recoverable-error gap, this ADR closes strict-`if` gap)
- [SPEC §1.5.2 — Trilean three-valued logic](../../SPEC.md)
- [`feedback_explicit_strictness.md`](../../../../home/giang/.claude/projects/-mnt-M2-STORAGE-Work-workspace-gh-rust-triet/memory/feedback_explicit_strictness.md) (panic-possible ops MUST be verbose methods with message argument — applies to `.assume_known(message)`)
