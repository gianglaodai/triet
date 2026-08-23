# ADR 0085 — Exhaustive `builtin_shim_meta` Table + Existence Gate in `Body::verify()`

**Status:** Decided (G ✅ 2026-07-24 · O ✅ 2026-07-24). Applies to Tier C+.
Closes the single point of failure (SPOF) of the shim metadata table: any `CallDispatch` calling a system shim (`__triet_*`) that lacks a corresponding entry in the table is rejected at the MIR well-formedness gate, rather than being silently ignored and causing miscompilations.

**Issue:** `triet_mir::builtin_shim_meta(name) -> Option<BuiltinShimMeta>` is a **static table queried across FIVE sites** in three crates (borrowck ×3, JIT ×1, lowerer ×1 — see §Caller Table). All five sites use `if let Some(meta)` / `is_some_and`, meaning a **missing entry is silently ignored**:

- JIT M3 (`mir_lower.rs:4784`) does not zero-on-consume → stale heap pointer remains live.
- Lowerer (`lib.rs:1517`) treats all arguments as borrowed → `push_owned` schedules a Drop for an argument that the shim consumed → **double-free** when a heap-consuming shim lacks an entry.
- Borrowck (`checker.rs:1288/1319`) skips mark-Moved and skips check mutate-while-borrowed → **silently misses E2420/E2440 errors**.

This is NOT defense-in-depth (five independent locks) but a **single failure point radiating across five locations**: when the table lies by omission, all five sites fail simultaneously. Today this is latent — **eight** shims are currently missing entries (`__triet_string_contains`, `_hash`, `__triet_vector_contains`, `__triet_hashmap_contains`, `__triet_cap_check`, `__triet_pow`, `__triet_string_append`, `_clear`), all of which happen to be **borrow/scalar** operations, making the all-borrow default coincidentally sound (verified: `__triet_string_append(slot, byte-scalar)` — the second parameter is a Copy byte i64, not a heap pointer). Any future **heap-consuming** shim missing an entry will fail silently and destructively.

> **AMENDMENT 2026-07-24 (7 → 8):** The original table listed 7 (O's recon `comm` compared **only** JIT-dispatch names vs meta). D caught the error when cross-checking: `__triet_vector_contains` is emitted from the **lowerer** (`lib.rs:2607`, branch `ty.is_vec()`), absent from O's JIT grep → missed. Currently running live in fixture `86_contains_vector_run.tri` (`contains(v4, 42)`). Without adding entry #8, the verify() gate would fail fixture 86. Correct verification measurement: `comm -23 <(grep lower + jit) <(grep meta)` = 8.

This is **Phase 1** of a two-phase plan (G locked 2026-07-24, Option 4 divide-and-conquer). Phase 1 closes **P-exist** (missing entries). **P-flag** (incorrect boolean flags within existing entries) is addressed in **Phase 2** — behavioral canaries — outside this ADR's scope.

## Decision

### 1. Exhaustive Table
Add explicit entries for the **eight** missing shims. The table must cover **all** `__triet_*` shims that the compiler can emit:

| Shim | `arg_consumes` | `mutates_arg` | Notes |
|---|---|---|---|
| `__triet_string_contains` | all-false | `None` | Pure borrow |
| `__triet_string_hash` | all-false | `None` | Borrow (content hash) |
| `__triet_vector_contains` | `[false, false]` | `None` | Pure borrow (arity 2 = `fn_2_1`; AMEND — caught by D) |
| `__triet_hashmap_contains` | all-false | `None` | Pure borrow |
| `__triet_cap_check` | all-false | `None` | ZST capability token |
| `__triet_pow` | all-false | `None` | Scalar Copy, no heap |
| `__triet_string_append` | `[false, false]` | `None` (Phase 1) | In-place mutation BUT see AMEND-2 |
| `__triet_string_clear` | `[false]` | `None` (Phase 1) | In-place mutation BUT see AMEND-2 |

> **AMEND-2 2026-07-24 (`Some(0)` → `None` for append/clear):** The original draft set `mutates_arg: Some(0)` for append/clear to "simultaneously fix E2440 mutate-while-borrowed" (scope creep by O). When D wired this in → **5 live fixtures failed with E2440 self-conflict** (`93_clear_run`, `96/97_append_*`, `99_append_then_clear`, `100_endgame`). Root cause (independently verified by O, toggling `Some(0) ↔ None`): append/clear uses calling convention `clear(&0 mutable m)` — lowerer passes **the raw Local of `m`** to arg[0], and evaluating `&0 mutable m` **creates an active loan `source=m`**; M3 precheck (`checker.rs:1288`) finds that loan conflicts with the very argument being mutated → E2440 with **its own loan**. `pop`/`remove` do not hit this because they pass bare container Locals without `&0`. While `mutates_arg: Some(0)` is **semantically correct** (append reallocating while `&0` is shared is a real hazard), **the checker must distinguish self-loans from foreign loans** — a behavioral change belonging to **Phase 2**. Setting `None` in Phase 1 **preserves pre-WO behavior** (append/clear had no entries previously = None) → zero regressions, no new holes. `arg_consumes` is preserved (satisfying P-exist against double-frees). E2440 for string mutations + checker self-loan exclusion are deferred to Phase 2.

Arity (`arg_consumes.len()`) is derived directly from `ShimSymbol::fn_N_M` signatures registered in `driver/main.rs`, NEVER guessed visually. `append`/`clear` use `mutates_arg: None` in Phase 1 (see AMEND-2).

### 2. Existence Gate in `Body::verify()` (Discriminator `__triet_`)
`Body::verify()` (`triet-mir/src/lib.rs:1855`) — the MIR well-formedness gate running in **Phase 3.5 of the driver, BEFORE borrowck (P4) and JIT (P5)** (`driver/main.rs:82-90`, comment: *"Run BEFORE borrowck and JIT so they can assume well-formed MIR"*) — introduces a new invariant:

> For every terminator `CallDispatch { callee_name, .. }`: if `callee_name.starts_with("__triet_")` and `builtin_shim_meta(callee_name)` returns `None` → `Err(MirError::UnknownShim { name })`.

The `__triet_` discriminator is a **structural boundary, not a hand-written list**: all system shims carry prefix `__triet_`; user functions (`concrete_fn`, `"fibonacci"`, `"f"`) and synthetic borrowck entries (`"consume"`, `"__test_shim_multiply"`) NEVER carry that prefix — so returning `None` for them remains valid. The gate self-enforces on the prefix: any future `__triet_*` shim missing an entry triggers an immediate error, without requiring synchronization of secondary lists.

The five read-sites (R1–R5) **retain** `if let Some` — now provably `Some` for `__triet_` names after `verify()` validation; their `None` branches become harmless defensive fallbacks. The blast radius is confined to **a single crate** (`triet-mir`: table entries + error variant + verify loop), rather than five sites across three crates.

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|---|---|---|---|
| α | None → Err at **each** read-site (R1–R5) | Defense at every layer | **Five duplicates** of the same predicate `__triet_ && None` across three crates (three error types) → missing a 6th site recreates the SPOF; exactly the duplication being eliminated | **Rejected** (G 2026-07-24) |
| β | **Single gate in `Body::verify()`** + retain R1–R5 | Single source of truth (DRY); runs at P3.5 before borrowck/JIT; teeth enforce compilation aborts; 1-crate radius | Does not gate unit tests constructing `Body` directly bypassing verify | **SELECTED** — Compiler contract protects user input (always passing driver P3.5); tests injecting invalid MIR accept the risks |
| γ | Registry Enum: JIT-dispatch + meta + emit keyed on single enum, non-exhaustive match = compile error | Eliminates SPOF by construction, no runtime gate needed | Major core surgery (25 shims × 3 layers), excessive blast radius for a static table | **Rejected** (G 2026-07-24) |
| δ | Existence Canary: test enumerating all shims, asserting presence in table | Cheap | **List is a 4th copy of the table** — whoever forgets table entries will forget test lists; zero-assertion hazard (circular oracle) | **Rejected** — "superficial" |

## Consequences

### Positive
- Missing entries for `__triet_*` shims no longer fail silently: compilation fails at P3.5 with `MirError::UnknownShim`, before reaching borrowck or JIT.
- `append`/`clear` carry `mutates_arg: None` (Phase 1) — E2440 string mutation checks deferred to Phase 2 alongside checker self-loan exclusions (AMEND-2).
- Validation predicate resides in exactly one location — impossible for copies to drift.

### Negative
- `verify()` does not guard tests constructing `Body` directly without calling `verify()`. Accepted: the compiler's guarantee applies to user inputs through the driver pipeline.

### Risks to Mitigate
- **Teeth:** Temporarily remove `__triet_vector_push` from the table → compile a fixture using `push` → `verify()` MUST return `MirError::UnknownShim`, aborting compilation at P3.5 WITHOUT silent code generation → restore via snapshot. This test guards **existence** — enforcing "guard existence first, flags second".

## Effective Date

- Tier C+ — verify gate + exhaustive table take effect immediately upon Phase 1 landing.
- **Phase 2 (Outside this ADR):** Behavioral roundtrip canaries per consuming shim closing P-flag (incorrect booleans). FREE-count oracle must deduplicate pointers.
- Related: ADR-0040 §3.1/§3.6 (shim registry + M3 consume-tracking), ADR-0079 (`returns_borrow_of`/`mutates_arg`), ADR-0082 §AMEND (M3 zeroing).
