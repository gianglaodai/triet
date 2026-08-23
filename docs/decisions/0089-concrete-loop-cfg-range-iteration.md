# ADR 0089 — Concrete Loop CFG & Range Iteration (Amending ADR-0003)

> Status: **SIGNED + IMPLEMENTED (2026-07-26) — O ✅ G ✅.** Slice 1 fully landed:
> `loop`/`break`/`continue` (CFG primitives) + `for i in <Range>` desugaring + non-Range
> guard (E1052) + break-value guard (E0009) + break/continue-outside-loop guard
> (E1143). Blood-verified by O: gate `0·clean·0·472·0`; 3 poison prongs fail RED (break-drop permanent
> counting FREE 3→2, guard E1052→E1100, guard E0009→silent-discard); E1143 fresh-binary
> confirmed (stale-binary E1140 caught per rule #12). SPEC §7.2 + ADR-0086 honest synchronization.

> Scope decided by G (2026-07-26): **Scope B — Slice 1**. Pragmatic engineering:
> ship `loop`/`break`/`continue` + `for i in <Range>` via explicit CFG desugaring,
> **WITHOUT touching generic trait dispatch**. Trait-based `Iterator<T>` (ADR-0003) deferred
> indefinitely until generics mature.

## Context — Half-Silent Trap + ADR-0003 Never Landed

Current status (measured 2026-07-26, file:line):

| Construct | Parser | Typecheck | Lower |
|---|---|---|---|
| `while` | ✓ | ✓ `check.rs:709` | ✅ `lib.rs:2100` (3-block: hdr/bdy/ext) |
| `for x in e` | ✓ | ⚠️ `check.rs:692` — only `Type::Range` for real element; other iterables → `Type::Unknown` **SILENTLY** | ❌ E1100 `lib.rs:2144` |
| `loop` | ✓ | ✓ `check.rs:722` | ❌ E1100 |
| `break`/`continue` | ✓ (`break x` outside loop → E0006) | ✓ no-op `check.rs:687` | ❌ E1100 |
| `Expr::Range` (`a..b`) | ✓ `expr.rs:111` | ✓ `Type::Range(inner)` `exprs.rs:224` | ❌ E1100 (no lowering arm) |

Consequence: `for i in 0..10 { ... }` **typechecked as `i: Integer` then DIED with E1100 at lowering** —
a half-silent trap. `for x in vector` was even worse: typechecked silently with `x: Unknown` (no error)
and also hit E1100. Users could not execute basic `for`/`loop` constructs.

**ADR-0003 (LIVE per ARCHIVE:79)** specified `Iterator<T>`/`Iterable<T>` traits with
`next() -> T?` + desugaring `for → loop`, BUT (ADR-0003 line 64) **"NOT LANDED"** —
slipped past all phases v0.2→v0.8, blocked by missing generics and trait dispatch.
This was an overambitious design forbidden at this stage.

## Decision

### §1 — Tombstone/Defer ADR-0003 Trait Protocol + Retire "AI-first"

- **DEFER INDEFINITELY** generic protocols `Iterator<T>`/`Iterable<T>` from ADR-0003
  (`.iter()`/`.next()`/adapters `map`/`filter`/`zip`/`enumerate`). Gated on mature
  generic trait dispatch — out of scope for current rewrite.
- **🪦 RETIRE "AI-first suitability" rationale** (ADR-0003 line 48). VISION tombstoned
  "AI-first" on 2026-06-22 — no metrics, no premature promises. Iteration rationale is re-anchored on
  **coherence + craft**: an explicit, readable CFG loop model digestible by borrowck,
  independent of runtime iterator objects.
- ADR-0003 remains LIVE as a **future design blueprint** for trait protocols; this ADR
  AMENDS the roadmap: concrete Slice 1 precedes trait protocols, which belong to a future era.

### §2 — Typecheck Guard (MANDATORY — ending silent traps)

`Stmt::For` arm (`check.rs:692`) patched: **iterable MUST be an `Expr::Range` node**
(checking expression kind, not just static type — since Range-typed variables cannot yet be lowered):

- Iterable is `Expr::Range { start, end, inclusive }` with `start`/`end` of matching
  scalar-numeric type (Integer/Long/…) → element-type = type of `start`; bind `variable`;
  proceeds to lowering.
- **All other iterables** (Vector, HashMap, String, struct, enum, Range-typed
  non-literals, …) → **REFUSE IMMEDIATELY at typecheck** with new error code
  **`E1052 NonRangeIterationUnsupported`** (namespace `triet::typecheck::E1052`),
  with diagnostic referencing this ADR + ADR-0003 (trait iteration deferral). Return `Type::Unknown` for
  the element (suppressing cascades) AFTER pushing the error — **NEVER silently assign `Unknown` without errors**.
- **FORBID `Type::Unknown` from flowing down to lowering for For loops.** Only inline Ranges may pass.

Diagnostic message for E1052 (formatted per ADR-0027):
```
E1052 NonRangeIterationUnsupported
Iteration over non-Range types is deferred (trait Iterator<T> not yet implemented).

[Fix 1] Use a Range loop over an index:
  Change `for x in <collection>` to `for i in 0..length(<collection>) { ... get(i) ... }`
```

### §2b — FORBID Silent Discard of break-values at PARSER (Found by G, verified by O)

🩸 **SECOND silent trap, confirmed at `stmt.rs:169-184`:** `parse_break` parsed
`value = if <terminator> None else Some(parse_expression(...)?)` — meaning it **DID** read
expressions on `break x`/`break 42` — but then only used `value` for the span, **returning
`Stmt::Break` (unit) and DISCARDING the value**. Result: `break 42;` was **silently swallowed** into `break;`
— the compiler discarded user values without a trace.

**Iron Directive (G, Slice 1):** NO ambiguous deferrals, NO silent discards.
- Patch `parse_break` (`stmt.rs:169`): if `value.is_some()` → **emit `ParseError` code
  `E0009 BreakWithValueNotSupported`** immediately at parser (break-with-value deferred entirely
  in Slice 1, including within `loop{}`). Swallowing values is strictly forbidden.
- Note `E0006 BreakValueOutsideLoop` already existed (`error.rs:86`) with help "only valid
  inside loop{}" — Slice 1 defers break-values entirely, so E0009 broadens/subsumes that
  pathway; D verified E0006 reachability, retaining pre-existing definitions if needed (surgical).

### §3 — Lowering CFG (explicit desugaring, reusing `while` shape)

Add **loop-context stack** to lowerer state (PREVIOUSLY NON-EXISTENT — confirmed
grep for `loop_stack`/`break_target`/`continue_target` = 0 hits):

```rust
struct LoopContext {
    break_bb: BasicBlock,     // target for `break` (= ext)
    continue_bb: BasicBlock,  // target for `continue` (= hdr for loop/while; = step for for)
    drop_snapshot: usize,     // owned_locals.len() AT the moment of entering loop-body scope
}
// Vec<LoopContext> — push on loop entry, pop on exit. break/continue read top.
```

**`loop { body }`** — 2 blocks:
```
cur → Goto hdr
hdr: <body>            // break→ext, continue→hdr
     Goto hdr          // back-edge (if body falls through)
ext:                   // c.cur after loop
```
`continue_bb = hdr`, `break_bb = ext`.

**`for i in start..end`** — desugars into while-style CFG with induction var + step block:
```
i = start              // Assign to induction local (Integer, scalar/Copy — NOT owned)
cur → Goto hdr
hdr: cond = i < end    // exclusive `..`; inclusive `..=` → i <= end. Comparison → Trilean!
     If cond → bdy else ext
bdy: <body>            // break→ext, continue→step
     Goto step
step: i = i + 1        // increment (Add — range-enforced trap per ADR-0044)
     Goto hdr          // back-edge
ext:
```
`continue_bb = step` (continue must execute increment before re-testing — NEVER jumps directly to
hdr, avoiding infinite loops). `break_bb = ext`. `inclusive` is read DIRECTLY from `Expr::Range.inclusive`
(NOT carried in `Type::Range`); because the iterable is an inline `Expr::Range`, start/end are extracted
from AST — **requiring no runtime Range value** (Range remains un-lowered as a standalone Expr).

**`break`** (top loop-context required — **CORRECTION (cleanup pass,
2026-07-26): parser DOES NOT restrict break/continue inside loops.** `E0006
BreakValueOutsideLoop` had 0 construction sites in parser (dead code, never
enforced) and typecheck no-ops `Stmt::Break`/`Stmt::Continue` — so top-level
`break;`/`continue;` OUTSIDE loops actually reached lowerer. Defensive guard here
(Track B rule #1: never panic on user input) refuses via dedicated code
**`E1143 BreakContinueOutsideLoop`** rather than treating else-branches as an ICE/bug):
emits drops for `owned_locals[drop_snapshot..]` in `pop_scope` ORDER
(references first, then LIFO `.rev()`), then `Goto break_bb`; sets `c.cur` = new dead block.

**`continue`**: identical to break but issues `Goto continue_bb`.

**`while`** (already lowered): wires break/continue using the SAME loop-context (`continue_bb=hdr`,
`break_bb=ext`) — near-zero cost, eliminating the paradox of "break forbidden in while". Included in Slice 1.

### §4 — Soundness (drop on unstructured jumps — G warning)

Drop mechanisms currently follow 2 patterns (`lib.rs`): (a) `pop_scope` `:552` drops on static
scope boundaries (draining snapshot.., references first, `.rev()`); (b) `flush_all_for_return` `:659`
drops ALL owned locals for `return`, **emitting without clearing** (Case-D: locals live before a
split must drop on EVERY exit path). NO unstructured jump drop mechanism existed.

break/continue **mimics the exact pattern of `flush_all_for_return`** but bounded to
`owned_locals[drop_snapshot..]` (the exact owned locals instantiated in loop-body up to jump
point, across all nested scopes — since `owned_locals` is flat and monotonic; closed child scopes
have already drained):

- **Emit without clearing:** break/continue DOES NOT drain `owned_locals`. Following jump, `c.cur` =
  dead block → structural `pop_scope` at body end emits drops into dead block (unreachable,
  harmless) AND drains (maintaining consistent owned_locals accounting for outer scope).
- **Fall-through path (no break):** only `pop_scope` drops (once) before back-edge.
- **Break path:** only break drops (once) then terminates. ⇒ every owned local drops **exactly once
  along each execution path**. No leaks, no double-frees.
- **Refactor:** extracted "sort + emit Drop for a slice of locals" into helper
  `emit_scope_drops(&[Local])` called by both `pop_scope` (with drain) and break/continue (without drain)
  — single source of drop-ordering, preventing divergence.

**Borrowck: NO changes required.** `check_body_with` `checker.rs:508` operates on raw `build_cfg()`
(worklist + monotonic fixpoint, converging over back-edges — `checker.rs:552`+`:563` already
propagates partial-moves across back-edges for While). CFG for loop/for uses standard `Goto`/`If`
(identical to While) ⇒ borrowck naturally accepts it. Correctly placed drops in lowerer ⇒ borrowck
AUTOMATICALLY validates E2450/UAF at break exits (catching misplacements).

### §5 — Safeguards (Acceptance Fixture List)

Positive:
- **T-loop-basic** (EXPECT): `loop { ...; if cond { break; } }` counts/sums correctly.
- **T-for-range** (EXPECT): `for i in 0..5 { sum = sum + i }` → 10 (exclusive).
- **T-for-range-inc** (EXPECT): `for i in 0..=5` → 15 (inclusive).
- **T-continue** (EXPECT): `for i in 0..N { if skip { continue; } ... }` — continue executes
  step, does not loop infinitely, does not double-count.
- **T-nested-break** (EXPECT): nested loops, `break` only exits inner loop (validating loop-context stack).

Soundness (counting harness, pointer deduplication):
- **T-break-drop** ⭐ (G mandate a) — **PERMANENT safeguard, CLEANUP pass 2026-07-26**:
  fixture 477 (`// EXPECT: 3`, value-only) is VACUOUS for soundness (leaks do not alter
  exit code); the actual safeguard is `crates/triet-driver/tests/break_drop_counting.rs`
  (`break_path_frees_heap_local_each_iteration`) — `loop { let s = "x"; i+=1; if i==3
  { break } }` across 3 iterations, FREE=3 (2 structural back-edges + 1 break-path). **Poison
  verification (D, before asserting):** removing `emit_scope_drops` from `Stmt::Break` arm →
  FREE=2 (measured empirically) → test fails RED.
- **T-break-borrow** (G dangling pointer warning): breaking out of loop with local borrows → borrowck
  detects drops on exit edge, preventing UAF and false E2450.

Negative (typecheck guard — G mandate b):
- **T-for-vector-refuse** ⭐: `for x in <vector>` → **E1052 at typecheck** (NOT E1100 at
  lowerer). Fixture `// ERROR: E1052`. Poison guard (removing refusal branch) → falls to E1100 lowerer
  = proving guard intercepts at correct stage.
- **T-break-value-refuse** ⭐ (G mandate — ending silent discard §2b): `break 42;` →
  **E0009 at PARSER** (`// ERROR: E0009`), NOT silently swallowed into `break;`. **Poison:**
  reverting `parse_break` to silent-discard (removing E0009 branch) → `break 42;` parses
  as unit `Stmt::Break` = value silently lost ⇒ fixture fails RED (expected E0009, got no-error).
- **T-break-outside-loop-refuse** ⭐ (CLEANUP pass, honesty item b): top-level `break;`
  outside all loops → **E1143 at lowerer** (NOT borrowing E1140 UndefinedLocal). Fixture 478
  `// ERROR: E1143` + `crates/triet-lower/tests/diagnostics.rs::e1143_break_continue_outside_loop_code_via_fixture_478`
  (locking `err.code()`). **Poison verification (D):** modifying `#[diagnostic(code(...))]` to
  dummy code → `diagnostics.rs` fails RED.

### §6 — Out of Scope (deferred, documented explicitly)

- `Iterator<T>`/`Iterable<T>` traits, `.iter()`/`.next()`, adapters — deferred per §1.
- `for x in Vector/HashMap/String` — refused under E1052 (Slice 2+, requires dedicated index-loop or traits).
- `break x` break-with-value — explicitly refused (§5 T-break-value-defer).
- `drain` (Vector/HashMap consume + tombstone per element) — DEDICATED subsequent WO, related
  to ADR-0082 move-out; NOT in Slice 1.
- Range-typed **variables** (`let r = 0..10; for i in r`) — refused under E1052 (inline-Range only).
- Increment overflow on `..=` upper bound: `i = i + 1` following final iteration may trap per
  ADR-0044 — accepted (consistent with range enforcement), documented.

## §AMEND — Slice 2a: `for item in <Vector>` copy/by-value sugar

> Status for this section: **SIGNED + IMPLEMENTED (2026-07-26) — O ✅ G ✅.** Fully landed: for-item
> Vector by-value sugar (scalar + bare copy-Struct), infallible-get raw-shim desugaring, guard
> **E1053** tightened EXACTLY matching lowerer (`is_scalar() || (UserStruct && is_copy_aggregate)` —
> Enum/Nullable/heap refused at typecheck, NEVER reaching E1100 lowerer). Blood-verified by O: gate
> `0·clean·0·479·0`; poison fails RED (broad guard→485 E1100 trap reopened; handle-aliasing→SIGABRT
> D+O; load-bearing E1053 guard); counting container FREE=1 lvalue+rvalue (emit_shim_call
> `lib.rs:1783` handles ownership — redundant `if !is_lvalue push_owned` line REMOVED). §2a.3.1
> container double-free avoided by reusing locals (no new local aliases).

Unlocks `for item in v` with `v : Vector<T>` when **T is Copy** (scalar or copy-aggregate).
Desugars into index-loops reusing FULL Slice 1 CFG — **WITHOUT generic traits, WITHOUT
consumption, WITHOUT tombstones** (elements read via Copy, `v` remains intact after loop).

### §2a.1 — Finding (O probe 2026-07-26)
`for i in 0..len(v) { ... }` **ALREADY WORKED** in Slice 1 (`len(v)` is `end` of inline
`Expr::Range`, probed correctly). The blocker for "reading elements" was `get(v,i)` returning `T?` +
`!!` (ForceUnwrap) **not yet lowered** (E1100) — but Slice 2a **DOES NOT require `!!`**: desugaring
uses an **infallible internal get** (in-bounds ensures OOB-null is impossible), binding `item : T`.
`!!` is independent technical debt (Slice 2c, NOT piggybacked — ordered by G).

### §2a.2 — Typecheck Guard (Allow Vector-Copy, Refuse Vector-heap)
`Stmt::For` arm (`check.rs:704`, post Slice 1): decision cascade:
1. iterable node is `Expr::Range` → Slice 1 path (Range elements).
2. else `iter_ty == Type::Vector(inner)` (`types.rs:40`):
   - `inner.is_scalar()` (`types.rs:147`) **OR** `inner.is_copy_aggregate()` (`types.rs:238`)
     → **ALLOW**; element-type = `(*inner).clone()`; bind `item : inner`.
   - else (`inner.is_heap()` — String/Vector/HashMap — or heap-bearing struct,
     meaning `!is_copy_aggregate()`) → **REFUSE** with new code **`E1053
     HeapVectorByValueIterationUnsupported`** (`triet::typecheck::E1053`; E1052 highest
     currently used). Diagnostic references drain (Slice 2b).
3. else (HashMap, String, standard struct, Range-typed-variable, MethodCall like
   `.drain()`/`.enumerate()`, …) → **E1052** (retained from Slice 1, unchanged).

🚩 **IRON INVARIANT (G):** by-value copy of a heap element = **heap pointer aliasing →
DOUBLE FREE** when both `item` (owned) and `v[i]` drop. Therefore, Vector-heap MUST
refuse at typecheck, NEVER reaching lowerer/JIT. `get`-builtin already refuses heap elements
(E1047 `exprs.rs:1179`) — but `for` is an INDEPENDENT surface requiring dedicated guard E1053 (with
messages tailored to iteration context, pointing to drain).

### §2a.3 — Lowering Desugar (`Stmt::For`, Vector branch)
After matching `Expr::Range` fails, lower iterable into a local; if
`local_decls[iter_local].ty == MirType::Vector(inner)` (`mir:496`) → desugar:
```
iter_local = <base handle of iterable>   // §2a.3.1 — DO NOT alias handle into new owned_local
__len = len(iter_local)         // __triet_vector_len shim (i64) — computed ONCE before loop
__i = 0
cur → Goto hdr
hdr: cond = __i < __len         // Lt → Trilean!
     If cond → bdy else ext
bdy: item = <infallible-get>(iter_local, __i)   // shim per inner kind (below), bind item : inner (NOT T?)
     <body>                     // break→ext, continue→step (loop-context matching Slice 1)
     Goto step
step: __i = __i + 1 ; Goto hdr
ext:
```

#### §2a.3.1 — ⚠️ HANDLE-ALIASING = CONTAINER DOUBLE-FREE (G steel warning)
Vectors are **8-byte handles**. If desugaring creates a NEW `owned_local __vec` and emits
`Assign(__vec = v)` (copying handle), both `__vec` AND `v` exist in `owned_locals`
→ `pop_scope`/scope-exit issues **TWO Drops on the SAME buffer** = **CONTAINER double-free**.
Structural rules:
- **iterable is a named lvalue** (`Expr::Variable` → `for item in my_vec`): `iter_local`
  = **existing local of `my_vec`** (`c.vars[name]`). ABSOLUTELY DO NOT `alloc_local`
  + `push_owned` anew. `len`/`get` merely READ the handle (read-use), without consuming. `my_vec`
  drops exactly once at ITS scope-exit, not the loop's scope-exit.
- **iterable is an rvalue** (`for item in make_vector()`): `iter_local` = temp from
  `lower_expr`; this temp MUST be owned-tracked to drop **exactly once at loop exit** (no
  leaks, no double-frees). D mapped traces (rule 20): verify how `lower_expr` owned-tracks
  temp heap-rvalues; if untracked → `push_owned(iter_local)` once, dropping at `ext`.
- **Differentiate via expr-kind:** `matches!(arena.expression(iterable).node, Expr::Variable(_))`
  → lvalue branch; else → rvalue branch. (Params `&0 Vector`/`&0 mutable Vector` are also
  Variables → lvalues, NO drop — correct since borrows do not own).
**Infallible-get per inner kind** (reusing existing `get` shims, NO new shims):
- `inner` scalar → `__triet_vector_get(__vec, __i)` (`mir_lower.rs:5936`, returns raw i64),
  binds `item : inner` directly (WITHOUT wrapping Nullable — in-bounds guarantees ≠ NULL_SENTINEL).
- `inner` copy-aggregate (Copy Struct) → `__triet_vector_get_copy` (`mir_lower.rs:4007`,
  sret), binds `item : Struct`.
- loop-context: `break_bb=ext`, `continue_bb=step` (matching for-Range).
- **NO move-out, NO tombstone:** `__triet_vector_get`/`get_copy` COPIES bytes; `v`
  retains len/buffer. `item` scalar/copy-agg NOT owned-tracked (no heap) → no
  Drop. Following loop, `v` remains owned by caller, dropping normally once.

### §2a.4 — Soundness
- **Element copy ⇒ no heap element aliasing:** guard §2a.2 excludes all heap elements;
  scalar/copy-aggregates copy pure bytes without shared heap pointers ⇒ no element double-free.
- **No CONTAINER handle aliasing (§2a.3.1):** lvalue → no new owned_local; rvalue →
  owned exactly once. Eliminates CONTAINER double-free (G minefield).
- **`v` unchanged:** no operations mutate `v`'s len/buffer ⇒ `len(v)` unchanged post-loop.
- **Borrowck:** UNTOUCHED (standard Goto/If CFG per §4; `v` is read-only — read-use, not move).

### §2a.5 — Safeguards
- **T2a-scalar** (EXPECT): `for x in v { sum += x }` on `Vector<Integer>` → sum correct.
- **T2a-copy-struct** (EXPECT): `for p in pts { sum += p.x }` on `Vector<CopyStruct>` → correct.
- **T2a-intact** ⭐ (EXPECT): `let v=...; for x in v {}; return len(v)` — (1) len UNCHANGED
  (copy without consume; poison: replace infallible-get with pop → len decreases → fails RED);
  (2) **`v` exiting scope at main end → clean exit 0, NO SIGABRT** (prevents CONTAINER double-free §2a.3.1;
  poison: alias handle into new owned_local → double-free → subprocess fails RED). **Counting safeguard
  (verified by O):** `__triet_vector_free` count = 1 for container, NOT 2.
- **T2a-rvalue** ⭐ (EXPECT + counting): `for x in make_vector() {}` — container rvalue
  drops **exactly once** (FREE=1: not leaking FREE=0, not double FREE=2). Proves rvalue branch
  correctly owned-tracked.
- **T2a-heap-refuse** ⭐ (ERROR E1053): `for s in string_vector { }` → **E1053 at
  typecheck** (NOT E1100 lowerer, NOT reaching JIT). **Poison (verified by O):** removing E1053 refusal
  branch → for-loop lowers→JIT→**double-free SIGABRT** (subprocess safeguard) = proves refusal is load-bearing.
- **T2a-break/continue** (EXPECT): break/continue inside for-Vector works (loop-context).

### §2a.6 — Out of Scope (Slice 2a)
- Vector-heap iteration (String/Struct-heap) → refused E1053, **consuming path = drain (Slice 2b)**.
- HashMap iteration, String iteration → E1052 (not yet opened).
- `for item in v.drain()`/`.enumerate()` (MethodCall) → E1052 (Slice 2b/trait deferral).
- `!!` ForceUnwrap → independent debt Slice 2c (ordered split by G).
- mutable `item` / writing back to `v[i]` (`set`) → NO (set-builtin does not exist).

### §2a.7 — Sites
1. **Typecheck** `check.rs:704` (For arm — add Vector branch) + `error.rs` (add E1053).
2. **Lower** `lib.rs` `Stmt::For` (add MirType::Vector branch after Range-match failure;
   infallible-get per inner kind; loop-context matching for-Range).
3. **Borrowck / Schema / JIT shims** — UNTOUCHED (reuses `__triet_vector_get`/`_copy`/`_len`).

## Open Questions
1. ~~What does `break x` parse to?~~ **CLOSED (O verified `stmt.rs:169-184`):** silent discard →
   unit `Stmt::Break`. Ruled: parser emits E0009 (§2b). Closed.
2. `i` induction variable: Slice 1 treats as standard local (user reassigning `i` affects loop — identical
   to while-desugar). Accepted for Slice 1; fresh-per-iteration binding deferred if needed.

## Sites (When Implementing — WO Finalized)
1. **Parser** `stmt.rs:169` (`parse_break`) + `error.rs` (add `E0009 BreakWithValueNotSupported`
   variant) — §2b forbids silent discard of break-values.
2. **Typecheck** `check.rs:692` (`Stmt::For` arm) + `error.rs` (add E1052 variant).
3. **Lower** `crates/triet-lower/src/lib.rs`: state loop-context stack; arms for `Stmt::Loop`,
   `Stmt::Break`, `Stmt::Continue`, `Stmt::For` (replacing E1100 catch-all `:2144`); wire
   break/continue into `Stmt::While` `:2100`; helper `emit_scope_drops`. `break`/`continue`
   with empty loop-context stack (top-level, outside all loops — see §3 correction) → dedicated code
   **`E1143 BreakContinueOutsideLoop`** (`LowerError::break_continue_outside_loop`, ADR-0086
   amendment), NOT borrowing `E1140 UndefinedLocal`.
4. **Borrowck** — UNTOUCHED (§4).
5. **Schema** — For/Loop/Break/Continue already present (schema:1329-1353); NO schema changes.

## Signatures
- **O: ✅ (2026-07-26)** — drafted + verified parse_break claim (`stmt.rs:169-184` silent discard)
  + borrowck CFG-generics (`checker.rs:552/563`) + While-shape (`lib.rs:2100`) via code. Blood verification
  executed post D implementation (bidirectional poison safeguards §5).
- **G: ✅ (2026-07-26)** — approved architecture §2/§4, discovered + ordered §2b break-value rejection.

## §AMEND — Slice 2b: `for item in <Vector>.drain()` consuming iteration (move-out)

> Status for this section: **SIGNED + IMPLEMENTED (2026-07-26) — O ✅ G ✅.** Finalized scope:
> **Vector<T>.drain() ONLY** (HashMap.drain() REJECTED — "one fortress at a time"). Approved architecture:
> **desugars into `pop_front` loop** (0 new JIT shims, 100% proven components), accepting O(N²) correctness-first.

Unlocks `for item in v.drain()` — **consuming** `v` element by element, moving out **by-value** each
`item : T` for **ALL T** (including heap: `Vector<String>`, `Vector<User{String}>`). This is
the consuming path refused by Slice 2a (E1053 copy=alias): drain **transfers ownership** → eliminating
aliasing → heap elements become valid. Continuation of ADR-0082 §AMEND-2 move-out, **NO new foundational ADR**.

### §2b.1 — Finding (O recon 2026-07-26, file:line)

`drain` = **100% PROVEN components**, requiring no new JIT/borrowck/schema shims:

| Component | Status | Evidence |
|---|---|---|
| loop/break/continue CFG | ✅ Slice 1 | `lib.rs:2325` for-arm, `loop_stack` |
| `pop_front(v)` move-out + **len-- tombstone** | ✅ ADR-0082 §AMEND-2 | shim `mir_lower.rs:4491`; `mutates_arg:Some(0)`, `arg_consumes:[false]` (`triet-mir/lib.rs:1194`) |
| `pop_front → T?` + `match ~+/~0` on **String** | ✅ end-to-end | fixture **347** `vector_string_pop_front_run`; **351** multi-element shift |
| pop on `Vector<UserStruct-heap-bearing>` (internal String) | ✅ REAL allocator | fixture **338** `vector_userstruct_pop_run` |
| `v.drain()` parse | ✅ `Expr::MethodCall{receiver,method,args}` | `expr.rs:965` |

### §2b.2 — `.drain()` is an EXCLUSIVE FOR-GUARD (G steel condition #1)

`drain` **IS NOT registered as a general method** in the symbol table. It is meaningful solely in
for-iterable positions. `Stmt::For` arm (`check.rs:692`) checks **expr-kind BEFORE** generic inference:
- iterable is `Expr::MethodCall { receiver, method == "drain", arguments == [] }` →
  infer **ONLY `receiver`** (bypassing E1041 no-matching-overload), then apply guard §2b.3.
- `v.drain()` standalone (`let x = v.drain();` / `v.drain();`) → routes to standard MethodCall
  → **E1041** (method not found). FORBIDDEN from passing.

### §2b.3 — Typecheck Guards (fail-closed, refuse-over-guess)

Inside drain branch, after inferring `receiver`:
1. `receiver_ty == Type::Vector(inner)` **and `receiver` IS NOT a reference** →
   - **`inner == Type::Nullable(_)`** → **REFUSE E1053** (G steel condition #4): `Vector<T?>`
     drain yields `pop_front : (T?)? = Nullable(Nullable(_))` = **double-nullable** — forbidden territory
     under ADR-0088 (get-family V=Nullable already refused E1051). Message references ADR-0088 deferral.
     **SOUNDNESS-BEFORE-SYNTAX**: unproven safety ⇒ refuse.
   - else → **ALLOW** ALL `inner` (scalar / copy-agg / **heap** String/Vector/HashMap /
     heap-bearing struct/enum); element-type = `(*inner).clone()`; bind `item : inner`.
2. `receiver_ty` is a **reference** (`&0 Vector` / `&0 mutable Vector` / `&mutable Vector`) →
   **REFUSE E1053** (G steel condition #2): drain = consuming mutation, FORBIDDEN across
   shared borrows. Slice 2b accepts only **owned locals or rvalue** Vectors. Borrow-receiver drain
   = clean future extension.
3. receiver is **HashMap / String / other type**, or method **≠ "drain"** (`v.other()`) →
   **E1052** (matching Slice 1/2a — non-Range/non-drain iteration deferred).

### §2b.4 — Lowering Desugar (`Stmt::For`, drain branch — BEFORE Range/Vector branches)

Match `Expr::MethodCall{method=="drain"}` at start of `Stmt::For`. Lower `receiver` into
`iter_local` (lvalue → existing local; rvalue → owned temp — **owned-tracked exactly once**, matching
§2a.3.1 container handle discipline). Emit CFG:
```
cur → Goto hdr
hdr: __opt = pop_front(iter_local)   // Nullable(inner); len-- (tombstone) each iteration
     <present-test>                  // reuse Nullable match tag-test (scalar sentinel PA-3c
     If present → bdy else ext       //   vs tag-prepend struct/String) — D maps trace
bdy: item = <present-unwrap __opt>   // reuse match ~+ present-arm bind (proven 319/347/338)
     <body>                          // break→ext, continue→hdr (NO step block —
     Goto hdr                        //   pop_front AUTO-advances; avoids infinite loops unlike for-Range)
ext:                                 // iter_local (empty, len==0) drops at scope-exit: buffer-only
```
- loop-context: `break_bb = ext`, `continue_bb = hdr`.
- **Avoid "match-arm diverges"**: emit `Terminator::If` directly on present-tag (avoiding match-exprs
  with `~0 => break` arms). Present-test + unwrap = REUSES lowering routines of
  `match nullable { ~+ x => .., ~0 => .. }` — mapped directly by D (rule 20); refuse if unclear
  (rule 4), DO NOT reinvent tag-testing.

### §2b.5 — Soundness (AMEND-2.1 contract satisfied FOR FREE)

- **Per-element tombstone (🔩 DOUBLY LOAD-BEARING — Measured by O 2026-07-26)**: `pop_front` decrements `len`
  each iteration → at ALL break/return/fall-through points, `v.len` = **exact count of UNDRAINED elements**.
  `Drop(v)` frees exactly surviving elements `0..len` + buffer. Drained elements are owned by `item` (dropping
  in body). ⇒ **each leaf frees exactly once** — 0 leaks, 0 double-frees, even on early breaks.
  **CRITICAL FINDING (O independent poison):** removing `len--` from `__triet_vector_pop_front`
  triggers TWO distinct failure modes — (a) **full-drain HANGS INFINITELY** because `pop_front` never
  reports empty (len remains unchanged) → present-test never terminates; (b) **break-mid FAILS** with
  survivor re-free mismatch (Drop re-walks moved-out slots). Thus `len--` bears **DUAL load**:
  it serves as the **TERMINATION condition** for CFG loops, AND the **double-free barrier** for
  survivors. Safeguards in `drain_iter_counting.rs` protect both.
- **Heap elements safely unlocked**: move-out transfers ownership (unlike Slice 2a copy=alias) ⇒ no dual
  ownership of allocations.
- **Rvalue temporaries**: `for x in make_vec().drain()` — empty container post-loop, buffer drops exactly
  **once** (not leaking FREE=0, not double-freeing FREE=2).
- **Borrowck UNTOUCHED**: `pop_front.mutates_arg=Some(0)` → E2440 automatically catches active loans on `v`;
  CFG uses standard Goto/If.
- **O(N²)**: `pop_front` shifts elements → draining N elements = O(N²). **Accepted as correctness-first**
  (reusing 100% proven infrastructure >> new O(N) cursor shims risking off-by-one/dangling pointers).
  O(N) cursor-drain = **technical debt for future performance ADR**.

### §2b.6 — Safeguards (Blood-verified by O — snapshot tests, NO git checkout; 6 G steel conditions)

Positive:
- **T2b-scalar** (EXPECT): `for x in v.drain()` on `Vector<Integer>` → sum correct.
- **T2b-heap-string** ⭐ (G condition #3, EXPECT): `Vector<String>` drain — strings readable
  in body, exits 0 clean (REAL allocator), freed cleanly post-loop.
- **T2b-heap-struct** ⭐ (G condition #3, EXPECT): `Vector<User{name:String}>` drain — String fields
  readable, dropped cleanly.
- **T2b-empty** (EXPECT): `v.drain()` on empty vector → 0 iterations, v drops cleanly.
- **T2b-break/continue** (EXPECT): break/continue inside drain works (loop-context).

Soundness (counting/subprocess — pointer deduplication):
- **T2b-tombstone** ⭐ (G condition #5): drain N heap elements → FREE = N (elements) + 1 (buffer).
  **Poison** (O): break len-- accounting (simulating broken tombstone) → popped cells double-free →
  **SIGABRT tcache**. Measured EMPIRICALLY.
- **T2b-break-mid** ⭐ (G condition #5): drain 5, `break` after 2 → FREE = 2 (items) + 3
  (survivors via `Drop(v)`) + buffer; NO double-frees (134), NO leaks (insufficient FREE).
- **T2b-rvalue** ⭐ (G condition #6): `for x in make_vec().drain()` — buffer FREE=1 (not
  leaking FREE=0, not double-freeing FREE=2).

Negative (fail-closed guards):
- **T2b-standalone-refuse** ⭐ (G condition #1, ERROR E1041): `let x = v.drain();` → **E1041**
  at typecheck (drain IS NOT a general method). Poison: register drain as general method → loses E1041.
- **T2b-borrow-refuse** ⭐ (G condition #2, ERROR E1053): drain on `&0 Vector` param →
  **E1053** at typecheck (DOES NOT compile silently → NO UB/crash). Poison: remove reference guard →
  reaches lowerer/JIT.
- **T2b-nullable-refuse** ⭐ (G condition #4, ERROR E1053): `Vector<String?>` / `Vector<Integer?>`
  drain → **E1053** (double-nullable ADR-0088 deferral). Fail-closed, never guess.
- **T2b-nondrain-method-refuse** (ERROR E1052): `for x in v.enumerate()` → **E1052**.

### §2b.7 — Sites
1. **Typecheck** `check.rs:692` (`Stmt::For` arm — add drain MethodCall branch BEFORE
   inline-Range check; guards §2b.3). `error.rs` — reuses E1041/E1052/E1053 (NO new codes;
   E1053 messages aware of drain context for references vs nullables).
2. **Lower** `lib.rs` `Stmt::For` (add drain MethodCall branch BEFORE Range `:2337` &
   Vector `:2484` branches; desugars pop_front loop §2b.4; reuses Nullable present-test/unwrap; loop-context).
3. **Borrowck / Schema / JIT shims** — UNTOUCHED (reuses `__triet_vector_pop_front`/present-test).

### §2b.8 — Out of Scope (Slice 2b)
- **HashMap.drain()** — REJECTED (G): touches `emit_hashmap_value_free_loop` + separate bucket state-gates →
  standalone campaign. Refused under E1052.
- String iteration, `.enumerate()`/`.iter()` adapters — E1052 (trait deferral §1).
- `Vector<T?>` drain (double-nullable) → E1053, awaits ADR-0088.
- Borrow-receiver drain (`&mutable Vector`) → E1053, clean future extension.
- O(N) cursor-drain shim → performance debt for future ADR.

### §2b — Signatures
- **O: ✅ BLOOD VERIFIED (2026-07-26)** — recon 5 proven pieces (fixtures 347/351/338) + verified
  independently: clean gate `0·clean·0·488·0`; poison tombstone `len--` fails RED bidirectionally (full-drain infinite HANG
  + break-mid FAILED count) → doubly-load-bearing (§2b.5); fat-Nullable present-test correct
  (487/488 total=5 real allocator); guards 491-494 match codes (E1041/E1053/E1053/E1052); Deinit=zero
  without free; sentinel collision impossible (PA-3c outside Integer range). Logic in D's code
  (check.rs/lib.rs/error.rs) INTACT.
- **G: ✅ ISSUED + CO-SIGNED (2026-07-26)** — approved Vector-only scope (REJECTED HashMap.drain() —
  "one fortress at a time") + zero-shim pop_front desugaring architecture + 6 steel conditions. Co-signed post
  verification: accepted Option-1 ("evidence is king, no ceremonial rituals"); ordered "Tombstone DOUBLY LOAD-BEARING"
  carved into §2b.5.
- **Giang: ✅ ISSUED (2026-07-26)** — confirmed Vector-only scope, issued implementation order.

---

## §AMEND — Slice 2d: `for item in <&0 mutable Vector>.drain()` Borrow-Receiver Drain

Unlocks §2b.8 row "Borrow-receiver drain → clean future extension". Slice 2b consumed owners
by-value; **Slice 2d drains VIA exclusive mutable borrow — caller RETAINS container**.

### §2d.1 — Scope (Finalized by G 2026-07-27)
- **ONLY `&0 mutable Vector<T>`** (`ReferenceForm::BorrowExclusiveMutable`). All other forms —
  `&0` read-only (`BorrowReadOnly`), `&+`/`&+ mutable` (`StrongFrozen`/`StrongMutable`),
  `&-` (`WeakObserver`) — **CONTINUE to refuse E1053** (DrainBorrowedReceiverUnsupported).
- **T non-nullable.** `&0 mutable Vector<T?>` → E1051/E1053 (double-nullable, awaits ADR-0088).
- **NO new foundational ADR.** Mirrors Slice 2b desugaring (`pop_front` loop) EXCEPT container buffer-drop at loop exit.

### §2d.2 — Container-Survives Semantics (Distinct from Slice 2b)
Runtime repr of `Vector<T>` = **buffer-pointer handle** single-i64 (`{len@0, cap@8, data@16}`).
`&0 mutable Vector` reference value = **identical buffer-pointer** (measured: `__triet_vector_get(vec)` =
`vec as *const u8`, fixture 168 `&0 xs`→get→15 ✅). Therefore:
- `pop_front(handle)` `len--` mutates **SHARED buffer header** → caller observes drain
  (unlike `String` — where `len` resides in stack fat-slot so `clear` needs slot-ptr; Vector `len` resides in heap buffer).
- Receiver is `MirType::Reference{..}` → `is_reference()==true`/`is_copy()==true` (mir/lib.rs:736)
  → **NO `push_owned`, NO `Statement::Drop`** → buffer **PRESERVED** for caller. Post-loop,
  caller observes a **valid empty Vector** (`len==0`, cap retained) — re-push/len/drop work normally.

### §2d.3 — Break-Mid Caller-Drop Soundness (Verified by O)
Break-mid: `buffer.len` accurately decremented by popped count (tombstone `len--` on each `pop_front`). Survivors
occupy `0..len`. Caller dropping `v` AFTER loop → `emit_vector_element_free_loop` reads `len=load(ptr,0)`
from buffer header + loops `i<len` (mir_lower.rs:1873/1880) → frees **ONLY survivors**, NEVER touching popped
elements (which moved out and were consumed in body). `__triet_vector_free` deallocates buffer block per `cap`
(mir_lower.rs:5828). **Tombstone `len--` in shared buffer = double-free barrier — now protecting
caller-later-drops as well** (Slice 2b protected owner-consumed-drops; identical mechanism, new interaction). Safeguards
for break-mid on `Vector<String>` prove matching FREE counts, with no leaks and no double-frees.

### §2d.4 — Touchpoints (2, contained)
1. **typecheck `check.rs:754`** — blind refusal `matches!(Type::Reference(..))` → **form-aware**:
   `Type::Reference(ReferenceForm::BorrowExclusiveMutable, inner)` where `inner=Type::Vector(T)`,
   `T` is not `Nullable` → ALLOW, element=`T`. All other forms / `T?` → E1053/E1051 (retained).
   (Typecheck `Type::Reference` = **tuple** `(ReferenceForm, Box<Self>)`, types.rs:117.)
2. **lower `lib.rs:2361`** — previously `let MirType::Vector(inner) = ty else Err`. Expanded to accept
   `MirType::Reference { form: BorrowExclusiveMutable, inner }` where `*inner = MirType::Vector(elem)`;
   unwraps elem, iter_local = reference-value (buffer handle); pop_front loop desugaring
   RETAINS Slice 2b logic (is_reference naturally skips drop). (MIR `MirType::Reference` = **struct**
   `{ form, inner }`, mir/lib.rs:507 — NOT a tuple.)
3. **borrowck** — `&0 mutable` exclusive loan spans entire loop (existing NLL E2440, unedited).

### §2d.5 — Out of Scope (Slice 2d)
`&+ mutable`/BYOS drain · HashMap.drain() (standalone fortress) · `Vector<T?>` (ADR-0088) ·
O(N) cursor-drain perf. All retain existing refusals.

### §2d — Signatures
- **O: ✅ BLOOD VERIFIED (2026-07-27)** — recon file:line + verified 7/7 load-bearing truths
  (ReferenceForm variants ✅ · Type::Reference tuple ✅ · MirType::Reference struct — **corrected G's
  tuple reference** ✅ · reference=buffer-handle ✅ · pop_front len-- shared buffer ✅ · element-free-loop
  scans 0..len ✅ · is_reference→no-drop ✅). **3 touchpoints** (D `014442e`+`2dcc9b6`): typecheck
  form-aware `check.rs:759` + lower Reference-unwrap `lib.rs:2373` + **JIT fat-detect Reference-unwrap
  `mir_lower.rs:3909`** (defect caught by D outside phase-1 scope → phase-2 opened touchpoint #3, mirroring
  `_get_copy:3967`). Independent gate `0·clean·0·501·0`. **Poison verification:** (2) removing form-guard → 492/507 lose
  E1053 FAILING RED; (3) removing JIT Reference-unwrap → heap drain 506 yields `unexpected String return` FAILING RED (scalar 505
  passes — radius verified); (1) push_owned DID NOT fail RED → **revealed 2-layer no-drop protection** (lowerer is_copy +
  JIT Drop:3397 both evaluate `is_copy(Reference)==true` mir:736), escalating poison to chokepoint → 506 `Drop
  not supported` fails closed + counting FAILS RED = container-survives verified load-bearing (fail-closed, NO silent
  double-free). Fixtures 505-509 + counting safeguards (full=3, break-mid=5) pass green, md5 across 4 files matches.
- **G: ✅ CAMPAIGN ACCEPTED (2026-07-27)** — independent verification on commit `2dcc9b6`: gate
  `0·clean·0·501·0`, counting safeguards (full=3, break-mid=5) clean, canaries for E1053 / break-mid survivor
  drop accurate, 2-layer no-drop (`is_copy(Reference)` lowerer + JIT Drop:3397) guards borrows safely
  fail-closed.
- **G: ✅ ARCHITECTURE APPROVED (2026-07-27)** — approved `&0 mutable`-only + T-non-nullable scope,
  mandated ADR-first, measured (E) Type::Reference/ReferenceForm for O, carved Container-Survives +
  Break-Mid Caller-Drop soundness.
- **Giang: ✅ DIRECTION FINALIZED (2026-07-27)** — selected candidate #6 of 7.

## §AMEND — HashMap.drain() Deferral (Two Walls, Fail-Closed E1054)

> ⚠️ **SUPERSEDED by §AMEND-2 (2026-07-27, `816a729`)** — `HashMap.drain()`
> HAS NOW LANDED via Option 2 destructuring-only desugaring. The "requires Tuple
> lowering" wall below **was bypassed, not demolished** (`MirType::Tuple`
> remains = 0). E1054 REMAINS but with updated role: solely refusing shapes
> outside the Slice 1 fence (§HM2.5). Read §AMEND-2 for current state; section below
> retained as historical record of deferral rationale at that time.

Formalizes §2d.5 out-of-scope line "HashMap.drain() (standalone fortress)". Campaign opened
for HashMap.drain() (2026-07-27); pre-recon by O **REJECTED backlog label "mirrors
Vector.drain / separate bucket state-gates"** — label overlooked the larger wall (Tuples).
Decision: **DO NOT land feature; refuse fail-closed with DEDICATED error code.**

### §HM-drain.1 — Two Technical Walls (Verified by O at file:line, 2026-07-27)

**🧱 Wall 1 — YIELD SHAPE requires Tuple `(K,V)`, and Tuples ARE NOT lowered.**
Correct semantics for `for (k, v) in m.drain()` yields `(K, V)`. Tuples exist in
AST + typecheck + parser (`Type::Tuple` `types.rs:49`; `Pattern::Tuple`
`parser/pattern.rs:173`; test `parses_for_with_tuple_destructuring`
`parser/stmt.rs:450`) — **but grepping `Tuple` across `triet-lower` / `triet-mir` /
`triet-jit` = 0 hits across ALL THREE CRATES**. Tuples have no MIR representation, no lowering, no JIT
layout. ⇒ yielding `(K,V)` requires **building tuple lowering from scratch** (MIR + JIT) = standalone
prerequisite campaign, heavier than drain itself, unlocking features far beyond drain (multi-value
returns, destructuring). HashMap.drain **is gated BEHIND** that campaign.

**🧱 Wall 2 — No primitive key-less entry enumeration.**
HashMap layout (`mir_lower.rs:6444`): open-addressing, slot = `key_stride +
value_stride + 1 state-byte`, body = `[len@0, cap@8, slots@16…]`, state==0 =
empty (enumerable in principle: walk 0..cap skipping empty). Shim inventory:
`alloc/free/len/insert/get/get_ref/get_ref_agg/get_copy/remove/contains` — grepping
`hashmap_keys/values/iter/next/pop/drain/entries` = **0 hits**. Vector.drain reused
`pop_front` (proven shim); HashMap **has no analog** — `remove(key)` requires
knowing the key upfront. ⇒ desugaring loop in the style of Slice 2b is IMPOSSIBLE; requires **new shims**
(`__triet_hashmap_drain_next` cursor / bucket walker) or exposing bucket internals
to the lowerer.

### §HM-drain.2 — Decision: DEFER, Refuse Fail-Closed (NO lossy semantics)

Option B (values-only) / Option C (keys-only) **WERE FORBIDDEN** (Ruled by Giang 2026-07-27): draining
while silently dropping key/value = lossy, asymmetric, counter-intuitive — semantic poison,
violating Lesson #6 (mentor_o_persona rule 18: "verify whether the shape is PERMITTED
to exist, do not patch mechanisms into gaps"). Without Tuple `(K,V)` lowering,
`HashMap.drain()` **MUST NOT EXIST**. Refuse cleanly, fail-closed, NO
silent errors, NO untracked panics.

### §HM-drain.3 — DEDICATED Error Code: E1054 (DO NOT fall into generic E1052)

Currently `for x in m.drain()` (HashMap receiver) falls into `else` `check.rs:795-803`
→ **E1052** `NonRangeIterationUnsupported` (generic "trait Iterator not implemented").
Obscures the true architectural blockers (2 cliffs). Formalized dedicated code:

- **E1054 `DrainHashMapUnsupported`** (next available — E1050..E1053 in use).
- Header: `E1054: `for` iteration over `HashMap<{key}, {value}>.drain()` is unsupported`.
- Diagnostic body explicitly states the 2 walls: (1) yielding `(K,V)` requires Tuple lowering
  (absent in MIR/JIT) — references prerequisite; (2) missing entry-enumeration shims.
- `[Fix]` suggests: use `remove(m, k)` with known keys, or await Tuple
  lowering + `HashMap.drain()` (deferred, ADR-0089 §AMEND HashMap.drain).
- **Scope bounded: SOLELY `.drain()` with receiver = `Type::HashMap(..)`.** Plain
  `for x in m` (non-drain HashMap iteration) RETAINS E1052 — separate deferral
  (Iterator trait), not drain.

### §HM-drain.4 — Touchpoints (contained, 1 site in typecheck)

`check.rs` drain-branch: BEFORE `else` `:795`, add arm
`if let Type::HashMap(k, v) = &receiver_ty` → push `DrainHashMapUnsupported`.
String/other RETAIN `NonRangeIterationUnsupported`. Untouched in lower/mir/jit
(refuses at typecheck ⇒ never reaches lowerer). Zero new shims.

### §HM-drain.5 — Safeguards (Refusal fixture + provable poison, implemented by D)

- Fixture: `for (k,v) in m.drain()` (or `for x in m.drain()`) on
  `HashMap<Integer,Integer>` → EXPECT-ERROR **E1054** (not E1052, no panics,
  no SIGILL).
- **Poison proving safeguard at harness layer** (rule 15): remove HashMap-detection arm
  in `check.rs` → fixture MUST fail RED (falls back to E1052 `got E1052, expected E1054`).
  Restored byte-identical.

### §HM-drain.6 — Prerequisites / Out of Scope

- **Actual feature prerequisite:** campaign "Tuple lowering (MIR + JIT)" MUST
  land BEFORE HashMap.drain() can exist properly. Logged in backlog as
  standalone campaign, NOT piggybacked on this amendment.
- Retained deferrals: entry-enumeration shims · O(N) cursor-drain · `HashMap<K, V?>`
  (double-nullable values, ADR-0088) · plain `for x in m` HashMap iteration (E1052).

### §HM-drain — Signatures
- **O: ✅ RECON + DESIGN + BLOOD VERIFIED (2026-07-27)** — verified 2 walls
  at file:line (Tuple 0 hits lower/mir/jit; shim inventory lacks key-less enumeration);
  proposed E1054 + drain-only scope; drafted WO for D (pen D → `c001075`). **Independent verification:**
  reviewed diff; independent gate `0·clean·0·502·0`; **manual poison** (disabling HashMap arm `check.rs:795` →
  fixture 510 `FAIL: expected E1054, got E1052`) → restored byte-identical (md5 `bd8c08c4…`);
  scope check 471 plain-iterate retains E1052 under poison.
- **G: ✅ ARCHITECTURE APPROVED (2026-07-27)** — APPROVED Option D (refuse-over-guess,
  no lossy semantics); E1054 `DrainHashMapUnsupported` **strictly scoped to
  `.drain()`** (DO NOT conflate plain iteration — "one error code, one semantic contract");
  mandated poison E1052-vs-E1054 safeguard; gate target `0·clean·0·502·0`.
- **Giang: ✅ SIGNED OPTION D (2026-07-27)** — clean deferral, forbade lossy Options B/C,
  required ADR + dedicated error code + fail-closed safeguards.

---

## §AMEND-2 — HashMap.drain() LANDED (Option 2 Destructuring-Only Desugaring)

**Status:** CLOSED (O✅/G✅/Giang✅ 2026-07-27, `816a729`).
**SUPERSEDES §AMEND HashMap.drain() Deferral** above: the "requires Tuple
lowering" wall **NO LONGER BLOCKS** — it was bypassed, not demolished. E1054
survives with an updated role: from *refusing all* `.drain()` on HashMaps → solely
refusing shapes **outside the Slice 1 fence** (§HM2.5).

### §HM2.1 — Why NOT First-Class Tuples (Option 1 REJECTED)

Old deferral notes identified Wall #1 as *"yielding `(K,V)` requires Tuple lowering when Tuples
have 0 hits across lower/mir/jit"*. Recon re-evaluated the real cost of tearing down that wall:

- `MirType::` is matched at **729 sites** in `triet-lower`/`triet-mir`/
  `triet-jit` (with `mir_lower.rs` alone containing 29 exhaustive matches).
- Adding a variant `MirType::Tuple` = reintroduces the exact class of **"exact match,
  FORGOT variant"** bugs the project just spent an entire campaign eliminating (the "forgot
  `Nullable`" family: 6 occurrences, **2 inside safety nets themselves**).
- Touches **B-γ multi-value returns** (deferred indefinitely) and neighbors **B-β sub-8B
  packing** (demolished).

**Decisive architectural question:** `for (k,v) in m.drain()` requires **two variables
inside the loop body**, NOT a **first-class tuple value**. Option 1 builds a first-class type
only to immediately tear it into two — paying a 729-site tax for an intermediate no one retains.
**G REJECTED Option 1, APPROVED Option 2.**

### §HM2.2 — 🔒 SUPREME INVARIANT: Zero Tuples in MIR

> **Tuples LIVE in front, DIE at lowering.** `MirType` retains its exact 11 variants.

Permanent invariant check (verified by O 2026-07-27): `MirType::Tuple` = **0** across
all of `triet-lower` + `triet-mir` + `triet-jit`.

⚠️ **INCORRECT Verification Method (Avoid):** raw `grep -c Tuple` **IS NOT** the metric
— the lowerer MUST match `triet_syntax::Pattern::Tuple` to destructure
(`triet-lower/src/lib.rs:2036`), which is the intentional Option 2 design rather than a violation.
The sole valid invariant metric is **`MirType::Tuple` = 0**.

### §HM2.3 — Three Touchpoints

1. **typecheck** `check.rs:829-845` — pattern is `Pattern::Tuple` with **exactly 2**
   children ∧ `key_ok` ∧ `value_ok` ⇒ returns `Type::Tuple([K,V])`, binding via
   `bind_pattern` (`check.rs:1097`, existing mechanism). Otherwise ⇒ E1054.
   `Type::Tuple` in typecheck is VALID — it dies in the subsequent step.
2. **lower** `triet-lower/src/lib.rs:2031+` — destructures `Pattern::Tuple(2)`
   into **two separate locals** `_key`/`_val`; CFG mirrors drain-arm in Slice 2b
   (cursor local, `break`→ext, `continue`→hdr). No tuple values generated.
3. **JIT** `mir_lower.rs:7005+` — new shim `__triet_hashmap_drain_next`.

### §HM2.4 — Shim `__triet_hashmap_drain_next`: 4-Step Move-Out Sequence

Body mirrors `__triet_hashmap_remove` (`:6824`) starting from `state == 1` branch. Each
drained entry MUST execute completely, in exact sequence:

1. surface KEY → `key_out_ptr`, VALUE → `val_out_ptr` (`copy_nonoverlapping`)
2. **zero key-cell** (`write_bytes(key_ptr, 0, key_stride)`)
3. **`state → 2`** (tombstone)
4. **`len--`**

then returns `idx + 1` as the next cursor.

**Why this sequence closes all 3 critical memory hazards (G mandate) with ONE flag:**
drop-glue **walks only `state == 1`** (`mir_lower.rs:1940` freeing KEY, `:2038`
freeing VALUE) ⇒ ① move-out sound (tombstones immune to double-free) · ②
break-mid: drained entries have `state 2` (skipped) + remaining entries have `state 1`
(drop-glue cleans) ⇒ no leaks, no double-frees · ③ container-survives:
`len--` on each entry ⇒ full drain leaves `len == 0`, allowing valid re-insertion.

**O(N) Cursor, NOT O(N²) Rescan** (demonstrated by D with empirical data, G concurred):
in case `cap=1000, len=10` → cursor inspects each slot exactly once = **1000**
state reads across full drain; rescan-from-0 = **10 × 1000 = 10,000**. In general,
cursor is `O(cap)` vs rescan `O(len × cap)`.

**Sound termination:** `while idx < cap` validates bounds BEFORE reading any bytes ⇒
`cap == 0` / cursor ≥ `cap` → returns sentinel immediately, without reading past header.
Fixture 525 (empty map) guards this case.

### §HM2.5 — 🔑 SENTINEL CONVENTION: Cursor Uses `-1`, NOT `NULL_SENTINEL`

**G mandated explicit documentation to prevent future confusion.** Two sentinels co-exist
in the codebase, **with distinct domains and concepts**:

| Sentinel | Value | Semantic | Used in |
|---|---|---|---|
| `triet_mir::NULL_SENTINEL` | `i64::MIN` | **absent value** (nullable PA-3c) | `T?`, pop/remove/get |
| cursor-stop (new) | `-1` | **exhausted slots to scan** | `__triet_hashmap_drain_next` |

Valid cursor domain is always `>= 0` ⇒ `-1` never collides with valid ranges. **DO NOT reuse
`NULL_SENTINEL` for cursors** — conflating these concepts invites silent bugs.

### §HM2.6 — Slice 1 Fence + E1054 Boundaries

**OPEN:** `K` ∈ {scalar, String} · `V` ∈ {scalar, String, Vector, HashMap}.
**REFUSED (E1054):** pattern not `Pattern::Tuple` of length 2 · 3-tuples ·
aggregate keys/values · `V = Nullable`.
**`m.drain()` outside `for` guards** → **E1015** (`no field or method named drain`)
— preserves for-guard-ONLY invariant, matching Vector precedent (fixture 491).

### §HM2.7 — Safeguards + 3-Prong Poison Verification Protocol (Verified by O)

**11 fixtures 520-530** · **`hashmap_drain_counting.rs`** (7 tests, **POINTER DEDUPLICATION**:
asserts both `count == N` AND `dup == 0` — simple FREE counts are blind to
double-frees, since 3 frees can represent 3 objects OR 2 objects + 1 duplicate).

| Prong | Poison Modification | Observed Result (Verified by O) |
|---|---|---|
| **P1** | `state → 2` modified to `1u8` | `drain_full` **9 vs 6** · `break_mid` **10 vs 8**, duplicated pointers ⇒ **actual double-free** |
| **P2** | omit `len--` | `drain_full_leaves_len_exactly_zero` **3 vs 0** |
| **P3** | typecheck guard fails **OPEN** (`if true`) | 527·528·529·530 fail RED **+ old fixture 510 fails RED**; 520-525 pass GREEN |

⚔ **Lesson P2 — "Not failing RED" must be investigated via reachable paths:**
P2 DID NOT fail the full corpus, because the drain loop terminates on `state` (via cursor),
NOT on `len`; re-inserting into `cap=4` also did not trigger resizing thresholds. This was
**(b) insufficiently strong test coverage**, not (a) inherently unobservable. **D reported
honestly and added dedicated safeguard** `drain_full_leaves_len_exactly_zero` checking
`len(m)` directly.

⚔ **Lesson P3 — "Disabling guard" means failing OPEN, not fail-closed.**
D initially tested `if false &&` (stricter) = wrong direction; D corrected to
`if true ||` (permissive acceptance) and re-measured. Under correct poison direction, refused
shapes **did not compile successfully** but were blocked by lowerer with different `LowerError`s ⇒
proving **2-layer defense-in-depth** (typecheck = correct code, lower = final fail-closed barrier),
sharing architecture with ADR-0088 Lane A.

### Effective Date §AMEND-2

- Effective from `816a729` (2026-07-27). Gate `0 · clean · 0 · **522** · 0 · CLEAN`
  (511 → 522 fixture files; **522 is TOTAL FILE COUNT**, not highest file index).
- **Open Debt:** aggregate key/value drain (moving out aggregate keys = new ABI pathway) ·
  `V = Nullable` (awaiting measurements; HashMap drain via out-params DOES NOT wrap
  `Nullable`, making it theoretically safer than `Vector<T?>`, but unmeasured ⇒ retained refusal) ·
  splitting 4-meaning E1054 · first-class `Tuple` (Option 1) remains **REJECTED**,
  reopened only upon genuine multi-return use cases.

### Signatures §AMEND-2

- **O: ✅** — recon reframing (Option 1 with 729 sites vs Option 2 with 0 hits); independent verification
  across 3 poison prongs + gates + `MirType::Tuple`=0.
- **G: ✅** — REJECTED Option 1 ("setting fire to our own house"), approved Option 2; identified 3 critical
  memory hazards + mandated heap-key×heap-value pointer deduplication tests; mandated sentinel `-1` convention;
  accepted 4-meaning E1054 temporarily for slice 1.
- **Giang: ✅** — finalized direction on Tuples/HashMap.drain, issued implementation authorization.

## §AMEND-3 — Split 4-meaning E1054 → E1056 (pattern) / E1054 (key) / E1057 (value)

Resolves diagnostic debt noted in §HM2.6/§AMEND-2 ("splitting 4-meaning E1054"). `E1054
DrainHashMapUnsupported` packed 3 independent axes into ONE `if…else` branch — pattern
not `(k,v)`, aggregate key, nullable/aggregate value — and ALWAYS printed
`HashMap<{key}, {value}>` as the cause even when syntax was the culprit (fixtures 527/528:
diagnostic blamed types when pattern structure failed). Violated ADR-0086 "one error code, one contract".

**Split across 3 axes in cascading order: pattern → key → value:**

| Code | Variant | Axis | Fixtures |
|---|---|---|---|
| **E1056** | `DrainHashMapPatternUnsupported` | loop pattern ≠ `(k, v)` 2-tuple — message DOES NOT print `key`/`value` | 527, 528 |
| **E1054** | `DrainHashMapKeyUnsupported` (narrowed) | `K` aggregate — blames `key` | 529 |
| **E1057** | `DrainHashMapValueUnsupported` | `V` nullable/aggregate — blames `value` | 530 |

Cascade evaluates patterns first (independent of K/V), then keys, then values — each
refusal specifies only the failed axis. `526` (drain outside `for` guard) unchanged,
retains E1015. `TypeError::error_span` updated across 3 arms. Acceptance behavior UNCHANGED;
modifies diagnostic surface only.

### Signatures §AMEND-3

- **O: ✅** — drafted 5-touchpoint WO, finalized 3-code table + cascade order.
- **G: ✅** — approved axis split, confirmed ACCEPT scope unexpanded.

## §AMEND-4 — Hardening FIFO Contract + INDEFINITE DEFERRAL of O(N) cursor-drain

### §4.1 — FIFO Contract (Observable, Binding Semantics)

`for x in v.drain()` on `Vector<T>` traverses elements in order of **indices
`0 -> len-1` (FIFO)** — matching insertion order via `push`. This IS NOT an accidental
implementation detail of `pop_front` loop desugaring (Slice 2b §2b.4): it is **observable
semantics** that tests and users rely upon (`break`-mid preserves survivors in order,
§2d.3; `return`-mid likewise, §4.2 below). **Altering traversal order is a breaking change**,
requiring a dedicated ADR, and cannot be changed silently by swapping underlying shims.

Corpus PRIOR to this WO was **blind to order**: O poisoned `__triet_vector_pop_front` ->
`__triet_vector_pop` (flipping FIFO to LIFO) and executed all 522 fixtures at that time
— only **1/522 failed RED** (`490_drain_break_continue.tri`), and that failure was an **accidental
constant collision** (`if x == 100 { break }` happened to match on first element under LIFO),
not because tests were designed to validate order. 6 related fixtures (486/487/488/505/506/509)
passed 100% under reversal — `509` was blind because all Strings had `length == 1`, making order
permutations unobservable. Fixtures 531-534 (§4.3) patch this blind spot using position-weighted
oracles (`acc = acc*10 + x`), AVOIDING simple cumulative sums.

### §4.2 — O(N) cursor-drain: INDEFINITELY DEFERRED (Not "unimplemented", REJECTED)

Optimizing `Vector.drain()` from O(N²) (current `pop_front` loop) to O(N) (cursor pointers +
single-pass buffer epilogue cleanup at loop exit, mirroring `state`-flag cursor idea from
`HashMap.drain()` Option 2 §AMEND-2) was **REJECTED** by G, based on 4 MEASURED reasons:

1. **`return` inside drain loop body is an INDEPENDENT exit edge NOT traversing block `ext`**
   (the normal convergence point at loop exit). Observed directly in `fn drain_it` of
   `534_drain_order_return_mid_survivors.tri` (MIR dump measured 2026-07-27):
   ```
   bb2: { ... Drop(_1) Drop(_0) Drop(_5) Return(_1) }   // return-mid
   bb3: { Drop(_1) Drop(_0)          Return(_1) }        // normal ext exit
   bb4: { If(_4) → +:bb3, -:bb2 }
   ```
   `bb2` (return-mid, `-` branch of `If` null-check on `pop_front`) and
   `bb3` (`+` branch, "buffer empty" — true loop `ext` exit) are **TWO DISTINCT
   BLOCKS with DIFFERENT DROP SETS**: `bb2` contains `Drop(_5)` (dropping the
   `item` variable moved out inside the loop before returning), while `bb3` DOES NOT —
   because at `bb3` no `item` remains live to drop. This is self-verifying proof in the test
   corpus that return-mid and normal-exit are DISTINCT non-converging pathways.
   Any cursor+epilogue design placing cleanup/tombstoning logic at `ext` (i.e. `bb3`) is
   **COMPLETELY BYPASSED** by `return`-mid (`bb2`) — leaving caller's buffer in a corrupt
   state (cursor advanced while `len`/tombstones un-updated), leading to **double-frees**
   when the caller subsequently drops or accesses the buffer.
2. **`Vector<T>` buffers lack per-slot state bytes** (`{len@0, cap@8, data@16}`
   — only a single `len` counter, unlike `HashMap` which maintains per-slot `state` fields, §AMEND-2).
   The cursor mechanism of `HashMap.drain()` (where a single `state` flag closes all 3 critical hazards:
   sound move-out / break-mid / container-survives — see `AMEND-2` Option 2) **has no equivalent
   on `Vector`**: no fields exist to mark "already moved out" independently of `len`.
3. The existing invariant `buffer[0..len)` = **live set at ALL times** (not just at `ext`)
   provides soundness **FOR FREE** across ALL exit edges — including edges not explicitly
   enumerated (break, continue, return, future panics). A cursor-epilogue design must enumerate
   and correctly handle EVERY exit edge — the burden of proof far outweighs the benefit.
4. O(N²) in `pop_front` loop is a **performance tax** (runtime latency), **NOT a soundness flaw**
   — no correctness mandate forces immediate modification; trading soundness for double-free risks
   in item 1 is unacceptable.

**LIFO alternatives** (swapping `pop_front` for `pop`, traversing from buffer end) and
**buffer-reversal alternatives prior to traversal** were both **REJECTED** for the same core reason:
both violate the FIFO contract §4.1 (breaking change without ADR), and buffer-reversal incurs an extra
O(N) data copying pass purely to swap observable order — failing to resolve O(N²) complexity while
altering semantics.

Conclusion: O(N²) `pop_front` loop (Slice 2b §2b.4) is the ONLY approved lowering implementation
for `Vector.drain()` until a future ADR proves soundness across ALL exit edges (including
`return`/`break`/`continue`/future edges) for a cursor-based design.

### §4.3 — Sentinels (WO-Drain-FIFO-Teeth, O✅/G✅/Giang✅ 2026-07-27)

- **531/532/533/534** — guards for FIFO contract §4.1, position-weighted oracles
  (`acc = acc*10 + <value>`, not cumulative sums):
  - `531_drain_order_scalar.tri` — owned `Vector<Integer>`, EXPECT 123.
  - `532_drain_order_string.tri` — owned `Vector<String>` (heap move-out),
    distinct lengths 1/2/4 (509 used length-1 everywhere and was blind), EXPECT 124.
  - `533_drain_order_break_mid_survivors.tri` — `&0 mutable` borrow-receiver
    drain + `break`-mid + reading survivors in order via `pop_front`, EXPECT 1024.
  - `534_drain_order_return_mid_survivors.tri` — identical to 533 but `break` ->
    `return acc;` inside loop body. **First fixture in entire corpus** exercising
    return-mid edge `bb2` vs normal-exit `bb3` in `fn drain_it` (§4.2 item 1) — the exact
    edge that rejected the O(N) design. EXPECT 1024.
- **535/536** — guards for CASCADE ORDER pattern -> key -> value (§AMEND-3), multi-axis
  (single fixture violating ≥2 axes simultaneously, unlike 510/527/528/529/530 where
  each file tests a single axis):
  - `535_hashmap_drain_multiaxis_pattern_wins.tri` — violates all 3 axes simultaneously
    -> locks pattern-wins-first edge (E1056).
  - `536_hashmap_drain_multiaxis_key_wins.tri` — valid pattern, invalid key+value
    -> locks key-wins-before-value edge (E1054).

### Signatures §AMEND-4

- **O: ✅ 2026-07-27** — drafted 6-fixture WO + measured live 4 EXPECT values
  (123/124/1024/1024) + 2 cascade error codes (E1056/E1054), confirmed 4 reasons
  indefinitely deferring O(N) cursor-drain based on MIR `bb2/bb3` probes and FIFO->LIFO poison tests.
- **G: ✅ 2026-07-27** — approved indefinite deferral of O(N) (closed until new ADR),
  approved FIFO contract as binding semantics.
- **Giang: ✅ 2026-07-27** — finalized direction.
