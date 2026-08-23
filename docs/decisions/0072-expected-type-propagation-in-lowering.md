# ADR 0072 — Expected-Type Propagation in AST→MIR Lowering

**Status:** **🔒 SEALED** (Permanently sealed by Mentor G 2026-06-27; blood-verified by O across 3 slices — byte-identical across entire corpus + independent failing poison tests + clean structural grep). Applicable to rewrite-era (Tier C, `triet-lower`). Closes **TODO Gap #2** (expected-type propagation for `~0`/Outcome-constructors nested in block-finals/if-arms/match-arms) + producer-side bug for **functions returning `T?`** discovered in the 2026-06-27 session.

**Implementation (3 slices, local commits awaiting push):** Slice 1 `c9a46e6` (signature plumbing, byte-identical) · Slice 2 `2c900fb` (wired 4 sources + leaf-consumer reading `expected` + eliminated 3 Bug-B redirects + transition fallback §2.5; unlocked scalar `T?`-returns) · Slice 3 (transparent forwarding across if/match/block + completely removed fallback §2.5 + **excised `c.sig.return_type` from constructor inputs** + extracted `emit_outcome_zero` helper; SEALED). Proof of closure: 157 UNTYPED (via old fallback) vs 157 ANNOTATED (via explicit sources) ⟹ **MIR byte-identical** — system detects zero divergence. `c.sig.return_type` now lives solely at the 4 legitimate return-position sources.

**G's Ruling (LOCKED 2026-06-27):** use **explicit parameter `expected: Option<&MirType>`** passed directly through `lower_expr` signature — DO NOT use hidden context (`c.expected_stack`). Rationale from G: *"Hidden state inevitably breeds rotten bugs — `c.sig.return_type` is the textbook example. Refactor it properly once; let the compiler be self-describing."* 3-slice roadmap; prerequisite: **byte-identical gate at Slice 1.**

**Issue:** Syntaxes `~+ v` / `~- e` / `~0` (`Expr::OutcomeConstructor`) and bare `~0` (`Expr::NullLiteral`) **do not carry sufficient intrinsic information** to determine their lower-level representation: a 16-byte Outcome StackSlot (`OutcomeAlloc`), or a nullable representation (PA-3c — plain-payload / NULL_SENTINEL). That decision depends on the **expected type at the call site** (`expected type` / context). The lowerer previously **DID NOT propagate** expected types; it used `c.sig.return_type` (the FUNCTION's return type) as a global proxy, manually patching other local sites with ad-hoc "redirects". Result: a class of `OutcomeAlloc on non-Outcome type` bugs + per-site patchwork debt.

---

## 1. Context — Physical Evidence (Recon Session 2026-06-27)

### 1.1 Case study: a misdiagnosis persisting for a session due to missing expected-type

Handover notes recorded a blocker on `match-arm bind heap payload move-out`: `match get(){~+ s => s}` → `lowerer does not support Identifier`. **Recon proved these were TWO overlapping issues, neither being "match-arm move-out":**

1. **Fog of war — name collision.** The test function was named `get`, colliding with the builtin free-function `get` (Vector/HashMap) at `lib.rs:2220`. Calling `get()` (0 args) → `arguments.len() != 2` → `unsupported_expr(callee)` (`lib.rs:2223-2227`) → printed `Identifier { name: "get" }`. Renaming `get`→`fetch`: error evaporated.
2. **The real culprit — expected-type.** After renaming: `function fetch() -> Integer? = ~+ 5` (producer only, no match, no move-out, no heap) → `MIR verification error: OutcomeAlloc on local _0 with non-Outcome type 'Integer?'`.

Probe matrix (each row modified exactly 1 variable, executed independently by driver):

| Probe | Form | Result |
|---|---|---|
| `match make_greeting() { ~+ x => x }` (`String~Integer`) | Outcome move-out | **OK, runs, exit 0** |
| `fetch() -> Integer? = ~+ 5` + empty main | nullable producer | **FAILS OutcomeAlloc** |
| `-> Integer? = fetch()` | passthrough | FAILS OutcomeAlloc |
| `let r: Integer? = fetch()` | consumer | FAILS OutcomeAlloc |
| `let r: Integer? = ~+ 5; match r {…}` (local) | local nullable match | **OK** |

→ `match`-arm move-out on Outcomes **was already working**. The temporary slot for call-return-aggregates lowered normally (fixtures 113/139/142). "Match-arm move-out" was NOT a new feature — it was a **victim** of missing expected-type.

### 1.2 Defective Mechanism (file:line)

`Expr::OutcomeConstructor` (`lib.rs:1712`) decided Outcome-vs-Nullable by reading **`c.sig.return_type`** (function return type — GLOBAL context):
- `lib.rs:1722` `~0`: if `c.sig.return_type` ≠ Outcome → treated as NullLiteral.
- `lib.rs:1762-1775` payload-type: `if let Outcome = c.sig.return_type` → extracts value/error type; otherwise `Unknown`.
- `lib.rs:1784-1790` **ALWAYS emitted `OutcomeAlloc` with `outcome_ty = c.sig.return_type.clone()`**.

When `c.sig.return_type = Nullable(T)`: the `if let Outcome` branch missed → payload `Unknown`, **YET STILL emitted `OutcomeAlloc` on a slot typed `Nullable(T)`** → verifier caught `OutcomeAlloc on non-Outcome type`. This was **"Bug B"** — named as such in the codebase at `lib.rs:1307-1313`.

### 1.3 Existing Bolt-On "Redirects" — Evidence of Patchwork Debt

Because the constructor read global context, every LOCAL context had to be patched by **stripping `~+` BEFORE reaching the constructor**:
- **let-annotation** `let x: T? = ~+ v` — `lib.rs:1314-1324`: if annotation is nullable + init is `~+ inner`, lowers `inner` as plain value instead of OutcomeConstructor.
- **struct-field** `Struct { f: ~+ v }` (field `f: T?`) — `lib.rs:2986-2998`: same `~+` stripping trick.
- **return-stmt `~0`** — `lib.rs:1446-1451`; **expr-body `~0`** — `lib.rs:884-895` (only `is_null_expr`, NOT handling `~+ v`).

Every new value-context position (block-final, if-arm, match-arm, call-argument, function-return-body for `~+ v`) required another ad-hoc patch. Comment in `lib.rs:882-883` admitted: *"Block-final / if-arm `~0` is a SEPARATE expected-type-propagation gap"*.

### 1.4 Technical Correction to G's Framing (NullableAlloc)

The WO mentioned "calling the right Constructor (`NullableAlloc` vs `OutcomeAlloc`)". **`NullableAlloc` DOES NOT EXIST** (`grep -rn NullableAlloc crates/` = empty). Under PA-3c, constructing a nullable is NOT an allocation:
- **present scalar** `~+ 5` (`Integer?`) = lower `5` plain — **the value IS the representation** (identity, no tag).
- **present aggregate** `~+ Struct{..}` (`Struct?`) = lower payload plain + **widening Assign** (slot = size+8, tag@0, fields@+8 — JIT taxonomy case 2, `lib.rs:1339-1362`).
- **null** `~0` (`T?`) = `Const NULL_SENTINEL` (`lib.rs:892` / struct niche at runtime).

Thus, expected-type's role is NOT to "choose a different allocation", but to **select the LOWERING PATH**: Outcome-StackSlot vs nullable-(identity/widen/sentinel). This ADR preserves existing PA-3c mechanics; it merely shifts the decision authority from `c.sig.return_type` (flawed proxy) to `expected_ty` (accurate local context).

---

## 2. Decision

**The flow `expected_ty: Option<&MirType>` is explicitly threaded down the lowering tree.** `Expr::OutcomeConstructor` and `Expr::NullLiteral` read `expected_ty` (NOT `c.sig.return_type`) to choose the lowering path. All bolt-on "redirects" (§1.3) are deleted, replaced by a single uniform rule: **value-context positions propagate expected_ty down to their sub-expressions.**

### 2.1 Modifying `lower_expr` Signature

```rust
// OLD (lib.rs:1622):
fn lower_expr(expr_id: ExprId, arena: &Arena, c: &mut Ctx) -> Result<Local, LowerError>

// NEW:
fn lower_expr(
    expr_id: ExprId,
    expected: Option<&MirType>,   // expected type at position; None = unconstrained
    arena: &Arena,
    c: &mut Ctx,
) -> Result<Local, LowerError>
```

All 61 `lower_expr(` call sites updated mechanically. **Safe default = `None`** (preserves exact existing behavior for all non-value-context positions). Only a small set of positions pass `Some(_)` (see §2.3). Rule against perpetual churn: across all 61 sites, **no** secondary overload added — one function, one parameter, mostly `None`.

> **G's Ruling (LOCKED):** explicit parameter. The alternative `c.expected_stack` (hidden context) **was rejected** — sharing the exact architectural pathology of `c.sig.return_type`. Churn across 61 sites is a one-time cost for a self-describing compiler.

### 2.2 Propagation Rules — TRANSPARENT vs OPAQUE

Categorization of child positions for each `Expr`:

- **TRANSPARENT** (forward expected_ty down intact — child value IS the parent value):
  - `Block { .., tail }` → tail receives block's expected type.
  - `If { cond, then, else }` → cond receives `Some(Trilean!)`; **then & else receive if's expected type** (both arms share result type).
  - `Match { scrutinee, arms }` → scrutinee receives `None` (independent type); **each arm body receives match's expected type**.
  - `OutcomeConstructor`/`NullLiteral` = **consuming LEAVES** (see §2.4).
- **OPAQUE** (child receives `None` — child type unrelated to parent type):
  - `BinaryOp`/`UnaryOp` operands, comparisons, logical operations.
  - index/receiver/builtin arguments, while condition, match scrutinee.
  - *(call arguments: parameter types can be forwarded in the future; left as `None` in this ADR — out of scope, logged to backlog.)*

### 2.3 Sources of expected_ty (where `Some` originates)

| Position | expected_ty originates from | Current file:line |
|---|---|---|
| Function body tail (block-final / expr-body) | `c.sig.return_type` | `lib.rs:878-898` |
| `Stmt::Return expr` | `c.sig.return_type` | `lib.rs:1446` region |
| `let x: T = init` | annotation `T` (via `lower_type_simple`) | `lib.rs:1307-1366` |
| Struct-field init `Struct{ f: e }` | declared type of field `f` | `lib.rs:2954-2998` |
| Match-arm body | expected type of `match` (transparent) | match branch §2.2 |
| If-arm body | expected type of `if` (transparent) | if branch §2.2 |

Note: `c.sig.return_type` DOES NOT vanish — it becomes the **initial origin** of expected_ty at exactly 2 locations (function-body-tail + return-stmt), rather than being covertly inspected deep in the constructor.

### 2.4 Consuming LEAVES — Restructuring `OutcomeConstructor` & `NullLiteral`

`Expr::OutcomeConstructor { arm, payload }` inspects `expected` (NOT `c.sig.return_type`):

```
match expected {
  Some(Outcome{value_type, error_type, ..}) =>
      // OUTCOME PATH (preserved intact lib.rs:1762-1810): OutcomeAlloc + disc + payload.
      // payload_ty = value_type/error_type per arm.
  Some(Nullable(inner)) =>
      match arm {
        Positive(Some(p)) => lower_expr(p, Some(inner), ..)   // plain payload; widening handled by parent Assign (PA-3c)
        Zero               => Const NULL_SENTINEL              // null representation
        Negative(_)        => Err  // ~- on T? — should already be blocked by typechecker
      }
  Some(other_non_wrapper) | None =>
      Err(null_literal_without_expected_type / outcome_without_expected_type)
}
```

`Expr::NullLiteral` (`lib.rs:1679`) operates analogously: `Some(Nullable(_))` → sentinel; `Some(Outcome{..})` → Outcome-zero (disc=0); otherwise → `Err` (generalizing the `is_null_expr` special case in `lib.rs:884` — eliminating ad-hoc branches).

**Debt payoff:** three bolt-on redirects (`lib.rs:1314-1324`, `2986-2998`, `884-895`) **are deleted**. They previously stripped `~+`/`~0` because the constructor read incorrect context; now that constructor reads `expected` accurately → `~+`/`~0` flows cleanly through `lower_expr(.., Some(field_or_annotation_ty))`. Less code, single unified pathway.

---

### 2.5 Transition Fallback Slice 2→3 (maintaining green gates per slice)

Recon on 2026-06-27 identified **3 fixtures (160/161/187)** placing `~+`/`~-` inside **match-arm-bodies** of functions returning Outcomes (e.g. `function f(c) -> Integer~Integer = match c { Color::Red => ~+ 5, … }`). They functioned because the constructor fell back to `c.sig.return_type`. If Slice 2 forced leaf-consumers to read `expected` STRICTLY (returning Err on `None`) while forwarding through `match`/`if`/`block` remained scheduled for **Slice 3** → those 3 fixtures would receive `None` → break → fail the test gate.

**Slicing strategy (by O, maintaining green gates throughout):**
- **Slice 2** — leaf-consumer reads `expected`, but when `expected == None` **falls back to `c.sig.return_type`** (legacy behavior). Wired sites (function-body/let/return/struct-field) pass real `Some(_)` → unlocking `T?`-returns. Unwired sites (match/if/block arms: 160/161/187) receive `None` → fallback → byte-identical.
- **Slice 3** — wire transparent forwarding (if/match/block) **AND remove fallback entirely** → `c.sig.return_type` ceases to be an input to the constructor (surviving only as an expected-type *source* at function-body/return per §2.3). Completes the elimination of hidden context.

The fallback is **transitional scaffolding scheduled for teardown in Slice 3**, NOT hidden debt. `c.sig.return_type` does not die fully in Slice 2 — it dies in Slice 3. Documented transparently so Slice 2 is not mistaken for the final elimination of hidden context.

## 3. Scope & Blast Radius

- **Signature churn:** 61 `lower_expr(` call sites (mechanical, predominantly adding `None`).
- **Reading `c.sig.return_type` for constructor decisions:** 11 usages (`grep` 2026-06-27) — consolidated to §2.3.
- **Deletions:** 3 bolt-on redirects + ad-hoc `is_null_expr` branch in function body.
- **Untouched:** JIT (`triet-jit`), MIR statement set (`OutcomeAlloc`/`StructAlloc`/`EnumAlloc` retained — NO new `NullableAlloc`), borrowck. Confined strictly to `triet-lower`.

**Preserved invariants (regression gate):** all Outcome fixtures (113/139/142, 107-135) + local nullable fixtures (225-230) + nested-nullable-aggregates (ADR-0065) remain **byte-identical**. Outcome path `Some(Outcome{..})` replicates legacy logic verbatim; local nullable path previously traversing redirects now flows through expected_ty with identical results.

---

## 4. Unlocked Capabilities (features enabled post-ADR landing)

1. `function f() -> T? = ~+ v` / `= ~0` lowers correctly (scalar + aggregate + heap-aware per ADR-0062 if enabled).
2. `match call_returning_T_question() { ~+ s => … ~0 => … }` executes.
3. `if c { ~+ v } else { ~0 }` with result type `T?` executes (Gap #2 if-arms).
4. Block-final `{ …; ~0 }` in positions expecting `T?`/Outcome executes (Gap #2 block-finals).
5. Entire existing test corpus PASSES byte-identical.

---

## 5. Rejected Alternatives

**Option A — per-site patching:** adding a "Bug B" redirect at function-body-tail (and subsequently if-arms, match-arms…). **Rejected (G, 2026-06-27):** *"Patch function body today, patch if-arms tomorrow, patch match-arms the day after? Never."* TODO Gap #2 explicitly warned "DO NOT patch per-site". Option A compounds debt; every new value-context site creates another redirect and new bug surface.

---

## 6. Arising Backlog (NOT forgotten — finalized by G 2026-06-27)

- **BuiltinShadowing UX trap (new error code).** Defining a user function with the same name as a builtin (`get`/`append`/…) currently triggers confusing `unsupported_expr` errors (case study §1.1 burned a full session). Fix: proper error diagnostics `ReservedBuiltinName` / `BuiltinShadowing` (namespace `triet::lower::Exxxx` or typechecker). Priority: consolidate into subsequent cleanup WO, NOT priority #1, CANNOT BE FORGOTTEN.
- **call-argument expected_ty:** propagate parameter types down to arguments (for passing `~+`/`~0` as arguments). Out of scope for this ADR; logged for future implementation.

---

## 7. References

- Closes **TODO Gap #2** (`TODO.md` under Heap-Nullable backlog #68).
- Related: ADR-0020 (Outcome), ADR-0041 (PA-3c nullable sentinel), ADR-0065 (nullable aggregate — source of bolt-on redirects), ADR-0062 (heap-nullable — benefits upon enablement).
- Handover correction: `spec/plans/MENTOR_G_STATE.md:14/49` ("match-arm move-out blocked by does not support Identifier") = misdiagnosis; true blocker = expected-type (this ADR).
